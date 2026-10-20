// Windows x64 的独立 libtest 退出诊断；不替代 nextest 的正式门禁。
// 构建：cl /std:c++17 /EHsc /W4 windows_exit_observer.cpp dbghelp.lib psapi.lib
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#define _WIN32_WINNT 0x0A00
#include <windows.h>
#include <tlhelp32.h>
#include <dbghelp.h>
#include <psapi.h>

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cwchar>
#include <cstring>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#pragma comment(lib, "dbghelp.lib")
#pragma comment(lib, "psapi.lib")
#if !defined(_M_X64)
#error 此工具仅支持原生 Windows x64
#endif

namespace {
constexpr wchar_t kTest[] = L"workspace::view::tests::restore_conversation_in_active_pane_enters_existing_live_conversation_without_loading";
constexpr DWORD kLimitMs = 60000, kSampleAtMs = 30000, kSamplerMs = 5000;
constexpr DWORD kEofMs = 200;
constexpr int kObserverError = 126, kTimeout = 124, kEofTimeout = 125;

struct Failure { const char* operation; DWORD code; };
[[noreturn]] void fail(const char* op) { throw Failure{op, GetLastError()}; }
void require(bool value, const char* op) {
    if (!value) throw Failure{op, ERROR_INVALID_DATA};
}

struct Handle {
    HANDLE value = nullptr;
    Handle() = default;
    explicit Handle(HANDLE h) : value(h) {}
    ~Handle() { reset(); }
    Handle(Handle&& other) noexcept : value(other.value) { other.value = nullptr; }
    Handle& operator=(Handle&& other) noexcept {
        if (this != &other) { reset(); value = other.value; other.value = nullptr; }
        return *this;
    }
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    explicit operator bool() const { return value && value != INVALID_HANDLE_VALUE; }
    void reset() { if (*this) CloseHandle(value); value = nullptr; }
};

std::string utf8(const std::wstring& s) {
    if (s.empty()) return {};
    int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, s.data(), static_cast<int>(s.size()), nullptr, 0, nullptr, nullptr);
    if (!n) fail("utf8_size");
    std::string result(n, '\0');
    if (!WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, s.data(), static_cast<int>(s.size()), result.data(), n, nullptr, nullptr)) fail("utf8_encode");
    return result;
}

std::string json(const std::string& s) {
    std::string result = "\"";
    for (unsigned char c : s) {
        if (c == '"' || c == '\\') { result += '\\'; result += c; }
        else if (c < 0x20) {
            char b[7]; std::snprintf(b, sizeof b, "\\u%04x", c); result += b;
        } else result += c;
    }
    return result + '"';
}
std::string number(uint64_t n) { return std::to_string(n); }
std::string boolean(bool b) { return b ? "true" : "false"; }
std::string hex(uint64_t n) {
    char b[32]; std::snprintf(b, sizeof b, "0x%llx", static_cast<unsigned long long>(n)); return b;
}
void write_all(HANDLE h, const void* data, size_t size) {
    auto p = static_cast<const char*>(data);
    while (size) {
        DWORD written = 0;
        if (!WriteFile(h, p, static_cast<DWORD>(std::min<size_t>(size, 65536)), &written, nullptr)) fail("write_file");
        require(written != 0, "write_zero"); p += written; size -= written;
    }
}
Handle new_file(const std::wstring& path) {
    Handle h(CreateFileW(path.c_str(), GENERIC_WRITE, FILE_SHARE_READ, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr));
    if (!h) fail("create_evidence_file");
    return h;
}
void save(const std::wstring& path, const std::string& text) {
    auto h = new_file(path); write_all(h.value, text.data(), text.size());
    if (!FlushFileBuffers(h.value)) fail("flush_evidence_file");
}
std::wstring env(const wchar_t* name) {
    DWORD n = GetEnvironmentVariableW(name, nullptr, 0);
    require(n > 1 && n <= 32768, "required_environment_missing");
    std::wstring result(n, L'\0');
    DWORD used = GetEnvironmentVariableW(name, result.data(), n);
    require(used && used < n, "environment_changed"); result.resize(used); return result;
}
uint64_t filetime(FILETIME f) { return (static_cast<uint64_t>(f.dwHighDateTime) << 32) | f.dwLowDateTime; }
uint64_t creation(HANDLE process) {
    FILETIME c{}, e{}, k{}, u{};
    if (!GetProcessTimes(process, &c, &e, &k, &u)) fail("process_times");
    return filetime(c);
}
std::wstring image_path(HANDLE process) {
    std::wstring result(32768, L'\0'); DWORD n = static_cast<DWORD>(result.size());
    if (!QueryFullProcessImageNameW(process, 0, result.data(), &n)) fail("process_image");
    result.resize(n); return result;
}
std::wstring self_path() { return image_path(GetCurrentProcess()); }

// 拒绝网络路径和各级重解析点；不修改 ACL，也不尝试绕开拒绝。
void plain_path(const std::wstring& path) {
    require(path.size() > 3 && path[1] == L':' && path[2] == L'\\' && path.find(L'/') == std::wstring::npos, "absolute_drive_path_required");
    size_t component_start = 3;
    for (size_t i = 3; i <= path.size(); ++i) {
        if (i != path.size() && path[i] != L'\\') continue;
        auto component = path.substr(component_start, i - component_start);
        require(!component.empty() && component != L"." && component != L"..", "dot_component_rejected");
        component_start = i + 1;
        DWORD a = GetFileAttributesW(path.substr(0, i).c_str());
        if (a == INVALID_FILE_ATTRIBUTES) fail("path_attributes");
        require(!(a & FILE_ATTRIBUTE_REPARSE_POINT), "reparse_point_rejected");
    }
}
std::wstring final_path(HANDLE h) {
    std::wstring s(32768, L'\0');
    DWORD n = GetFinalPathNameByHandleW(h, s.data(), static_cast<DWORD>(s.size()), FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
    require(n && n < s.size(), "final_path_failed"); s.resize(n);
    if (s.rfind(L"\\\\?\\", 0) == 0) s.erase(0, 4);
    return s;
}
FILE_ID_INFO identity(HANDLE h) {
    FILE_ID_INFO id{};
    if (!GetFileInformationByHandleEx(h, FileIdInfo, &id, sizeof id)) fail("file_id");
    return id;
}
bool same_id(const FILE_ID_INFO& a, const FILE_ID_INFO& b) {
    return a.VolumeSerialNumber == b.VolumeSerialNumber && std::memcmp(a.FileId.Identifier, b.FileId.Identifier, 16) == 0;
}
std::string identity_json(const FILE_ID_INFO& id) {
    std::string bytes;
    for (BYTE b : id.FileId.Identifier) { char x[3]; std::snprintf(x, sizeof x, "%02x", b); bytes += x; }
    return "{\"volume_serial\":" + number(id.VolumeSerialNumber) + ",\"file_id\":" + json(bytes) + "}";
}
Handle binary_file(const std::wstring& path) {
    plain_path(path);
    Handle h(CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr));
    if (!h) fail("open_bound_binary");
    BY_HANDLE_FILE_INFORMATION info{};
    if (!GetFileInformationByHandle(h.value, &info)) fail("binary_attributes");
    require(!(info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "binary_not_regular");
    return h;
}

struct Log {
    Handle file;
    ULONGLONG start = GetTickCount64();
    std::mutex mutex;
    explicit Log(const std::wstring& path) : file(new_file(path)) {}
    void event(const char* type, const std::string& fields = {}) {
        std::lock_guard<std::mutex> lock(mutex);
        ULONGLONG now = GetTickCount64();
        std::string line = "{\"mono_ms\":" + number(now) + ",\"elapsed_ms\":" + number(now - start) + ",\"type\":" + json(type) + fields + "}\n";
        write_all(file.value, line.data(), line.size());
    }
    void flush() { if (!FlushFileBuffers(file.value)) fail("flush_log"); }
};

std::wstring quote(const std::wstring& s) {
    std::wstring r = L"\""; size_t slashes = 0;
    for (wchar_t c : s) {
        if (c == L'\\') { ++slashes; continue; }
        r.append(slashes * (c == L'"' ? 2 : 1), L'\\'); slashes = 0;
        if (c == L'"') r += L'\\'; r += c;
    }
    r.append(slashes * 2, L'\\'); return r + L'"';
}
Handle duplicate(HANDLE h, DWORD access, bool inherit) {
    HANDLE out = nullptr;
    if (!DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &out, access, inherit, access ? 0 : DUPLICATE_SAME_ACCESS)) fail("duplicate_handle");
    return Handle(out);
}
struct Process { Handle process, thread; DWORD pid = 0; };
Process spawn(const std::wstring& exe, const std::vector<std::wstring>& args,
              const std::vector<HANDLE>& inherited, HANDLE input, HANDLE output, HANDLE error) {
    std::wstring cmd = quote(exe);
    for (const auto& a : args) cmd += L" " + quote(a);
    SIZE_T bytes = 0;
    InitializeProcThreadAttributeList(nullptr, 1, 0, &bytes);
    std::vector<unsigned char> storage(bytes);
    auto attrs = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
    if (!InitializeProcThreadAttributeList(attrs, 1, 0, &bytes)) fail("attribute_list");
    struct Attributes { LPPROC_THREAD_ATTRIBUTE_LIST p; ~Attributes() { DeleteProcThreadAttributeList(p); } } cleanup{attrs};
    if (!UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        const_cast<HANDLE*>(inherited.data()), inherited.size() * sizeof(HANDLE), nullptr, nullptr)) fail("handle_list");
    STARTUPINFOEXW si{}; si.StartupInfo.cb = sizeof si; si.lpAttributeList = attrs;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = input; si.StartupInfo.hStdOutput = output; si.StartupInfo.hStdError = error;
    PROCESS_INFORMATION pi{};
    // 不使用 BREAKAWAY；创建后先绑定自有 Job 和身份，再执行任何 child 指令。
    if (!CreateProcessW(exe.c_str(), cmd.data(), nullptr, nullptr, TRUE,
        CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
        nullptr, nullptr, &si.StartupInfo, &pi)) fail("create_child");
    return {Handle(pi.hProcess), Handle(pi.hThread), pi.dwProcessId};
}
Handle private_job() {
    Handle job(CreateJobObjectW(nullptr, nullptr)); if (!job) fail("create_job");
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if (!SetInformationJobObject(job.value, JobObjectExtendedLimitInformation, &limits, sizeof limits)) fail("job_limits");
    return job;
}
void assign_job(HANDLE job, Process& p) {
    if (!AssignProcessToJobObject(job, p.process.value)) {
        DWORD code = GetLastError(); TerminateProcess(p.process.value, kObserverError);
        WaitForSingleObject(p.process.value, 1000); throw Failure{"assign_own_job", code};
    }
    BOOL member = FALSE;
    if (!IsProcessInJob(p.process.value, job, &member) || !member) fail("child_job_membership");
}
void resume_primary(Process& p) {
    DWORD previous = ResumeThread(p.thread.value);
    require(previous == 1, "primary_resume_count");
    p.thread.reset();
}

struct Pipe { Handle read, write; };
Pipe pipe_pair() {
    SECURITY_ATTRIBUTES sa{sizeof sa, nullptr, TRUE}; HANDLE r = nullptr, w = nullptr;
    if (!CreatePipe(&r, &w, &sa, 0)) fail("create_pipe");
    Pipe p{Handle(r), Handle(w)};
    if (!SetHandleInformation(r, HANDLE_FLAG_INHERIT, 0)) fail("pipe_read_noinherit");
    return p;
}
struct Reader {
    Handle read, archive, thread;
    HANDLE forward = nullptr;
    Log* log = nullptr;
    const char* stream = nullptr;
    std::atomic<bool> eof{false}, error{false}, test_ok{false}, summary_ok{false};
    std::string tail;
};
DWORD WINAPI read_stream(void* opaque) {
    auto& s = *static_cast<Reader*>(opaque);
    try {
        char buffer[8192];
        for (;;) {
            DWORD used = 0;
            if (!ReadFile(s.read.value, buffer, sizeof buffer, &used, nullptr)) {
                DWORD e = GetLastError();
                if (e == ERROR_BROKEN_PIPE) {
                    s.eof = true;
                    s.log->event("pipe_eof", ",\"stream\":" + json(s.stream)); break;
                }
                throw Failure{"pipe_read", e};
            }
            if (!used) continue;
            write_all(s.archive.value, buffer, used);
            if (std::strcmp(s.stream, "stdout") == 0) {
                s.tail.append(buffer, used);
                const std::string test_line = "test " + utf8(kTest) + " ... ok";
                if (!s.test_ok && s.tail.find(test_line) != std::string::npos) {
                    s.test_ok = true; s.log->event("libtest_test_ok");
                }
                if (!s.summary_ok && s.tail.find("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;") != std::string::npos) {
                    s.summary_ok = true; s.log->event("libtest_summary_one_passed");
                }
                if (s.tail.size() > 16384) s.tail.erase(0, s.tail.size() - 8192);
            }
            // 转发可能阻塞，因此 IO 在独立线程，不占用 60 秒/采样 watchdog。
            write_all(s.forward, buffer, used);
        }
        if (!FlushFileBuffers(s.archive.value)) fail("flush_child_output");
    } catch (const Failure& e) {
        s.error = true;
        try { s.log->event("io_error", ",\"stream\":" + json(s.stream) + ",\"operation\":" + json(e.operation) + ",\"win32\":" + number(e.code)); } catch (...) {}
    } catch (...) { s.error = true; }
    return 0;
}

// sampler 只接受父进程显式继承的句柄，不 OpenProcess、不接收任意 PID。
uintptr_t parse_handle(const wchar_t* text) {
    wchar_t* end = nullptr; unsigned long long n = wcstoull(text, &end, 10);
    require(n && end && *end == 0, "inherited_handle_argument"); return static_cast<uintptr_t>(n);
}
int sample_mode(int argc, wchar_t** argv) {
    require(argc >= 5, "sampler_arguments");
    HANDLE process = reinterpret_cast<HANDLE>(parse_handle(argv[2]));
    HANDLE output = reinterpret_cast<HANDLE>(parse_handle(argv[3]));
    DWORD target = GetProcessId(process); require(target && target != GetCurrentProcessId(), "sampler_process_handle");
    auto emit = [&](const std::string& line) { auto text = line + "\n"; write_all(output, text.data(), text.size()); };
    const auto path = image_path(process);
    const auto slash = path.find_last_of(L'\\'); require(slash != std::wstring::npos, "sampler_image_directory");
    DWORD options = SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS | SYMOPT_EXACT_SYMBOLS
        | SYMOPT_FAIL_CRITICAL_ERRORS | SYMOPT_NO_PROMPTS | SYMOPT_IGNORE_NT_SYMPATH
        | SYMOPT_IGNORE_CVREC | SYMOPT_IGNORE_IMAGEDIR | SYMOPT_NO_IMAGE_SEARCH;
#ifdef SYMOPT_DISABLE_SYMSRV_AUTODETECT
    options |= SYMOPT_DISABLE_SYMSRV_AUTODETECT;
#endif
    SymSetOptions(options);
    if (!SymInitializeW(process, path.substr(0, slash).c_str(), TRUE)) fail("symbol_initialize");
    struct Symbols { HANDLE p; ~Symbols() { SymCleanup(p); } } cleanup{process};
    emit("{\"type\":\"sample_begin\",\"mono_ms\":" + number(GetTickCount64()) + ",\"pid\":" + number(target) + "}");
    HMODULE modules[2048]{}; DWORD needed = 0;
    if (!EnumProcessModulesEx(process, modules, sizeof modules, &needed, LIST_MODULES_64BIT)) fail("enumerate_modules");
    require(needed <= sizeof modules, "module_limit");
    for (size_t i = 0; i < needed / sizeof(HMODULE); ++i) {
        wchar_t name[1024]{}; MODULEINFO info{};
        if (!GetModuleBaseNameW(process, modules[i], name, 1024) || !GetModuleInformation(process, modules[i], &info, sizeof info)) fail("module_identity");
        IMAGEHLP_MODULE64 symbol_module{}; symbol_module.SizeOfStruct = sizeof symbol_module;
        bool known = SymGetModuleInfo64(process, reinterpret_cast<DWORD64>(modules[i]), &symbol_module) != FALSE;
        const GUID& guid = symbol_module.PdbSig70;
        char guid_text[40]{};
        std::snprintf(guid_text, sizeof guid_text, "%08lx-%04x-%04x-%02x%02x-%02x%02x%02x%02x%02x%02x",
            static_cast<unsigned long>(guid.Data1), guid.Data2, guid.Data3, guid.Data4[0], guid.Data4[1],
            guid.Data4[2], guid.Data4[3], guid.Data4[4], guid.Data4[5], guid.Data4[6], guid.Data4[7]);
        GUID zero{}; bool pdb_known = known && std::memcmp(&guid, &zero, sizeof guid) != 0;
        emit("{\"type\":\"module\",\"name\":" + json(utf8(name)) + ",\"base\":" + json(hex(reinterpret_cast<uintptr_t>(info.lpBaseOfDll)))
            + ",\"image_size\":" + number(info.SizeOfImage)
            + ",\"pe_timestamp\":" + (known ? number(symbol_module.TimeDateStamp) : "null")
            + ",\"pe_checksum\":" + (known ? number(symbol_module.CheckSum) : "null")
            + ",\"pdb_guid\":" + (pdb_known ? json(guid_text) : "null")
            + ",\"pdb_age\":" + (pdb_known ? number(symbol_module.PdbAge) : "null") + "}");
    }
    for (int i = 4; i < argc; ++i) {
        HANDLE thread = reinterpret_cast<HANDLE>(parse_handle(argv[i]));
        require(GetProcessIdOfThread(thread) == target, "sample_thread_owner");
        DWORD tid = GetThreadId(thread); require(tid != 0, "sample_thread_id");
        CONTEXT context{}; context.ContextFlags = CONTEXT_FULL;
        if (!GetThreadContext(thread, &context)) fail("thread_context");
        STACKFRAME64 frame{};
        frame.AddrPC = {context.Rip, 0, AddrModeFlat};
        frame.AddrFrame = {context.Rbp, 0, AddrModeFlat};
        frame.AddrStack = {context.Rsp, 0, AddrModeFlat};
        DWORD64 previous = 0;
        for (unsigned depth = 0; depth < 96 && frame.AddrPC.Offset; ++depth) {
            DWORD64 pc = frame.AddrPC.Offset;
            if (depth && pc == previous) break;
            previous = pc;
            DWORD64 base = SymGetModuleBase64(process, pc);
            alignas(SYMBOL_INFO) unsigned char symbol_bytes[sizeof(SYMBOL_INFO) + MAX_SYM_NAME]{};
            auto symbol = reinterpret_cast<SYMBOL_INFO*>(symbol_bytes);
            symbol->SizeOfStruct = sizeof(SYMBOL_INFO); symbol->MaxNameLen = MAX_SYM_NAME;
            DWORD64 displacement = 0;
            bool resolved = SymFromAddr(process, pc, &displacement, symbol) != FALSE;
            // 仅记录代码地址与符号，不记录 CONTEXT 寄存器值、栈字节、参数或局部变量。
            emit("{\"type\":\"frame\",\"tid\":" + number(tid) + ",\"depth\":" + number(depth) + ",\"pc\":" + json(hex(pc)) + ",\"module_base\":" + json(hex(base)) + ",\"module_offset\":" + json(hex(base ? pc - base : pc)) + ",\"symbol\":" + (resolved ? json(std::string(symbol->Name, symbol->NameLen)) : "null") + "}");
            if (!StackWalk64(IMAGE_FILE_MACHINE_AMD64, process, thread, &frame, &context, nullptr, SymFunctionTableAccess64, SymGetModuleBase64, nullptr)) break;
        }
    }
    emit("{\"type\":\"sample_end\",\"mono_ms\":" + number(GetTickCount64()) + "}");
    if (!FlushFileBuffers(output)) fail("sample_flush");
    return 0;
}

struct Suspended {
    Handle thread;
    DWORD id = 0, previous = 0;
    bool added = false;
};
struct State {
    std::wstring root, directory;
    std::string mode, stop_reason, observer_error;
    Log* log = nullptr;
    DWORD child_pid = 0, child_exit = 0;
    bool has_exit = false, timed_out = false, eof_timeout = false, job_terminated = false;
    bool eof_deadline_recorded = false, stdout_eof_at_deadline = false, stderr_eof_at_deadline = false;
    bool job_empty = false;
    DWORD active_processes_final = MAXDWORD;
    unsigned samples = 0, resumed = 0, sampler_failures = 0;
    void stop(const char* reason) {
        if (stop_reason.empty()) stop_reason = reason;
        Handle h(CreateFileW((root + L"\\STOP.json").c_str(), GENERIC_WRITE, FILE_SHARE_READ, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr));
        if (!h) { if (GetLastError() == ERROR_FILE_EXISTS) return; fail("create_stop"); }
        std::string text = "{\"reason\":" + json(stop_reason) + ",\"mode\":" + json(mode) + ",\"observer_pid\":" + number(GetCurrentProcessId()) + ",\"child_pid\":" + number(child_pid) + "}\n";
        write_all(h.value, text.data(), text.size()); if (!FlushFileBuffers(h.value)) fail("flush_stop");
    }
};

void take_sample(State& state, HANDLE child, HANDLE job, HANDLE null_handle) {
    ++state.samples;
    std::vector<Suspended> threads;
    Process sampler;
    bool error = false;
    try {
        Handle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)); if (!snapshot) fail("thread_snapshot");
        THREADENTRY32 entry{}; entry.dwSize = sizeof entry;
        if (!Thread32First(snapshot.value, &entry)) fail("thread_first");
        do {
            if (entry.th32OwnerProcessID != state.child_pid) continue;
            require(threads.size() < 256, "thread_limit");
            Handle thread(OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, FALSE, entry.th32ThreadID));
            if (!thread) fail("open_child_thread");
            require(GetProcessIdOfThread(thread.value) == state.child_pid, "thread_identity_changed");
            threads.push_back({std::move(thread), entry.th32ThreadID, 0, false});
        } while (Thread32Next(snapshot.value, &entry));
        require(GetLastError() == ERROR_NO_MORE_FILES && !threads.empty(), "thread_enumeration_incomplete");
        // 先准备 sampler 和全部继承句柄，最后才挂起 child，缩短冻结窗口。
        const auto name = state.directory + (state.samples == 1 ? L"\\sample-01.jsonl" : L"\\sample-02.jsonl");
        auto output = new_file(name);
        auto process_copy = duplicate(child, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | SYNCHRONIZE, true);
        auto output_copy = duplicate(output.value, GENERIC_WRITE, true);
        auto null_copy = duplicate(null_handle, 0, true);
        std::vector<Handle> copies;
        std::vector<HANDLE> inherit{process_copy.value, output_copy.value, null_copy.value};
        std::vector<std::wstring> args{L"--sample", std::to_wstring(reinterpret_cast<uintptr_t>(process_copy.value)), std::to_wstring(reinterpret_cast<uintptr_t>(output_copy.value))};
        for (const auto& t : threads) {
            copies.push_back(duplicate(t.thread.value, THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, true));
            inherit.push_back(copies.back().value); args.push_back(std::to_wstring(reinterpret_cast<uintptr_t>(copies.back().value)));
        }
        sampler = spawn(self_path(), args, inherit, null_copy.value, null_copy.value, null_copy.value);
        assign_job(job, sampler);
        for (auto& t : threads) {
            t.previous = SuspendThread(t.thread.value);
            if (t.previous == static_cast<DWORD>(-1)) fail("suspend_child_thread");
            t.added = true;
            state.log->event("thread_suspended", ",\"sample\":" + number(state.samples) + ",\"tid\":" + number(t.id) + ",\"previous_count\":" + number(t.previous));
        }
        resume_primary(sampler);
        DWORD wait = WaitForSingleObject(sampler.process.value, kSamplerMs);
        if (wait != WAIT_OBJECT_0) {
            if (!TerminateProcess(sampler.process.value, kTimeout)) fail("terminate_sampler");
            WaitForSingleObject(sampler.process.value, 1000);
            throw Failure{wait == WAIT_TIMEOUT ? "sampler_watchdog" : "sampler_wait", wait};
        }
        DWORD code = 0; if (!GetExitCodeProcess(sampler.process.value, &code)) fail("sampler_exit_code");
        require(code == 0, "sampler_nonzero");
        state.log->event("sample_complete", ",\"sample\":" + number(state.samples) + ",\"sampler_pid\":" + number(sampler.pid));
    } catch (const Failure& e) {
        error = true;
        try { state.log->event("sample_error", ",\"operation\":" + json(e.operation) + ",\"win32\":" + number(e.code)); } catch (...) {}
    } catch (...) { error = true; }
    // DbgHelp 失败、超时或子进程崩溃都走此处；只撤销本工具添加的一次挂起。
    if (sampler.process && WaitForSingleObject(sampler.process.value, 0) == WAIT_TIMEOUT) {
        if (!TerminateProcess(sampler.process.value, kObserverError)) error = true;
        if (WaitForSingleObject(sampler.process.value, 1000) != WAIT_OBJECT_0) error = true;
    }
    for (auto& t : threads) {
        if (!t.added) continue;
        DWORD previous = ResumeThread(t.thread.value);
        if (previous == static_cast<DWORD>(-1) || previous != t.previous + 1) error = true;
        else ++state.resumed;
        try {
            state.log->event("thread_resumed", ",\"sample\":" + number(state.samples) + ",\"tid\":" + number(t.id) + ",\"returned_count\":" + number(previous) + ",\"expected_count\":" + number(t.previous + 1));
        } catch (...) { error = true; }
        t.added = false;
    }
    if (error) { ++state.sampler_failures; state.observer_error = "sampling_failed"; state.stop("observer_error"); }
}

int fixture(const std::wstring& mode) {
    // 两次 sampler 各最多 5 秒，加间隔后仍留足退出余量；不改变产品的 30/60 秒期限。
    if (mode == L"sample") Sleep(45000);
    require(mode == L"normal" || mode == L"nonzero" || mode == L"sample", "fixture_mode");
    if (mode == L"nonzero") return 7;
    const std::string text = "running 1 test\ntest " + utf8(kTest) + " ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
    write_all(GetStdHandle(STD_OUTPUT_HANDLE), text.data(), text.size()); return 0;
}

int observe(const std::wstring& protocol) {
    State state; state.mode = protocol.empty() ? "observe" : "protocol-" + utf8(protocol);
    state.root = env(L"INFINISHELL_EXIT_OBSERVER_ROOT"); plain_path(state.root);
    Handle root_handle(CreateFileW(state.root.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr));
    if (!root_handle) fail("open_evidence_root");
    require(GetFileAttributesW(state.root.c_str()) & FILE_ATTRIBUTE_DIRECTORY, "root_not_directory");
    require(_wcsicmp(final_path(root_handle.value).c_str(), state.root.c_str()) == 0, "root_not_canonical");
    require(GetFileAttributesW((state.root + L"\\STOP.json").c_str()) == INVALID_FILE_ATTRIBUTES && GetLastError() == ERROR_FILE_NOT_FOUND, "root_already_stopped");
    state.directory = state.root + L"\\observation-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(creation(GetCurrentProcess()));
    if (!CreateDirectoryW(state.directory.c_str(), nullptr)) fail("create_observation_directory");
    // IO 线程的状态保留到进程结束，避免 CancelSynchronousIo 后尚在退出的线程访问已析构对象。
    auto log = new Log(state.directory + L"\\events.jsonl"); state.log = log;
    auto out = new Reader(); auto err = new Reader();
    Handle job; Process child; int result = kObserverError;
    try {
        const std::wstring exe = protocol.empty() ? env(L"INFINISHELL_EXIT_OBSERVER_BINARY") : self_path();
        if (protocol.empty()) require(env(L"INFINISHELL_EXIT_OBSERVER_TEST") == kTest, "fixed_test_required");
        auto held_binary = binary_file(exe); const auto expected_id = identity(held_binary.value);
        const auto canonical = final_path(held_binary.value);
        BOOL outer_job = FALSE;
        if (!IsProcessInJob(GetCurrentProcess(), nullptr, &outer_job)) fail("observer_outer_job");
        log->event("observer_start", ",\"mode\":" + json(state.mode) + ",\"observer_pid\":" + number(GetCurrentProcessId()) + ",\"outer_job\":" + boolean(outer_job != FALSE) + ",\"root_identity\":" + identity_json(identity(root_handle.value)) + ",\"binary_identity\":" + identity_json(expected_id));
        auto stdout_pipe = pipe_pair(), stderr_pipe = pipe_pair();
        SECURITY_ATTRIBUTES sa{sizeof sa, nullptr, TRUE};
        Handle null_handle(CreateFileW(L"NUL", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0, nullptr));
        if (!null_handle) fail("open_null");
        job = private_job();
        std::vector<std::wstring> args = protocol.empty() ? std::vector<std::wstring>{L"--exact", kTest, L"--nocapture"} : std::vector<std::wstring>{L"--fixture", protocol};
        child = spawn(canonical, args, {null_handle.value, stdout_pipe.write.value, stderr_pipe.write.value}, null_handle.value, stdout_pipe.write.value, stderr_pipe.write.value);
        state.child_pid = child.pid; assign_job(job.value, child);
        const auto actual_path = image_path(child.process.value);
        auto actual_file = binary_file(actual_path);
        require(same_id(expected_id, identity(actual_file.value)) && _wcsicmp(final_path(actual_file.value).c_str(), canonical.c_str()) == 0, "child_image_mismatch");
        BOOL wow64 = TRUE;
        if (!IsWow64Process(child.process.value, &wow64) || wow64) fail("child_not_native_x64");
        log->event("child_bound", ",\"pid\":" + number(child.pid) + ",\"creation_filetime\":" + number(creation(child.process.value)) + ",\"image_path\":" + json(utf8(actual_path)) + ",\"image_identity\":" + identity_json(expected_id) + ",\"own_job\":true");
        out->read = std::move(stdout_pipe.read); out->archive = new_file(state.directory + L"\\child.stdout.log");
        out->forward = GetStdHandle(STD_OUTPUT_HANDLE); out->stream = "stdout"; out->log = log;
        err->read = std::move(stderr_pipe.read); err->archive = new_file(state.directory + L"\\child.stderr.log");
        err->forward = GetStdHandle(STD_ERROR_HANDLE); err->stream = "stderr"; err->log = log;
        out->thread = Handle(CreateThread(nullptr, 0, read_stream, out, 0, nullptr)); if (!out->thread) fail("stdout_reader_thread");
        err->thread = Handle(CreateThread(nullptr, 0, read_stream, err, 0, nullptr)); if (!err->thread) fail("stderr_reader_thread");
        stdout_pipe.write.reset(); stderr_pipe.write.reset();
        const auto started = GetTickCount64(); resume_primary(child);
        log->event("child_resumed");
        while (true) {
            DWORD wait = WaitForSingleObject(child.process.value, 10);
            if (wait == WAIT_OBJECT_0) break;
            if (wait != WAIT_TIMEOUT) fail("child_wait");
            if (out->error || err->error) fail("output_reader_failed");
            ULONGLONG elapsed = GetTickCount64() - started;
            if (elapsed >= kLimitMs) {
                state.timed_out = true; state.stop("timeout_60s");
                if (!TerminateJobObject(job.value, kTimeout)) fail("terminate_timeout_job");
                state.job_terminated = true;
                require(WaitForSingleObject(child.process.value, 1000) == WAIT_OBJECT_0, "timeout_child_not_exited"); break;
            }
            if (elapsed >= kSampleAtMs && state.samples == 0) {
                state.stop("alive_at_30s"); log->event("alive_at_30s");
                take_sample(state, child.process.value, job.value, null_handle.value);
                if (!state.observer_error.empty()) break;
                if (WaitForSingleObject(child.process.value, 1000) == WAIT_TIMEOUT) take_sample(state, child.process.value, job.value, null_handle.value);
                if (!state.observer_error.empty()) break;
            }
        }
        if (!state.observer_error.empty()) throw Failure{"sampling_failed", ERROR_INVALID_DATA};
        if (!GetExitCodeProcess(child.process.value, &state.child_exit)) fail("child_exit_code");
        state.has_exit = true;
        log->event("child_exit", ",\"exit_code\":" + number(state.child_exit) + ",\"job_terminated\":" + boolean(state.job_terminated));
        const auto exited_at = GetTickCount64();
        while ((!out->eof || !err->eof) && GetTickCount64() - exited_at < kEofMs) Sleep(1);
        state.stdout_eof_at_deadline = out->eof; state.stderr_eof_at_deadline = err->eof;
        state.eof_deadline_recorded = true;
        state.eof_timeout = !state.stdout_eof_at_deadline || !state.stderr_eof_at_deadline;
        log->event("eof_deadline", ",\"stdout_eof\":" + boolean(state.stdout_eof_at_deadline)
            + ",\"stderr_eof\":" + boolean(state.stderr_eof_at_deadline) + ",\"observation_limit_ms\":200");
        if (state.eof_timeout) state.stop("pipe_eof_timeout");
        if (state.child_exit != 0) state.stop("child_nonzero");
        if (out->error || err->error) fail("output_reader_failed");
        result = state.timed_out ? kTimeout : state.eof_timeout ? kEofTimeout : static_cast<int>(state.child_exit);
        if (!state.child_exit && (!out->test_ok || !out->summary_ok)) fail("libtest_success_marker_missing");
        if (protocol == L"sample" && (state.samples != 2 || !state.resumed)) fail("protocol_sample_incomplete");
    } catch (const Failure& e) {
        if (state.observer_error.empty()) state.observer_error = e.operation;
        result = kObserverError;
        try { log->event("observer_error", ",\"operation\":" + json(e.operation) + ",\"win32\":" + number(e.code)); state.stop("observer_error"); } catch (...) {}
    } catch (...) {
        state.observer_error = "cpp_exception"; result = kObserverError;
        try { state.stop("observer_error"); } catch (...) {}
    }
    // Job 计数包含尚未释放引用的已退出进程；先保存退出/EOF，再释放自己的句柄。
    if (job && child.process && state.has_exit && !state.job_terminated && state.observer_error.empty()) {
        try {
            require(WaitForSingleObject(child.process.value, 0) == WAIT_OBJECT_0, "recorded_child_not_signaled");
            auto observe_members = [&](const char* phase) {
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting{};
                if (!QueryInformationJobObject(job.value, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, nullptr)) fail("job_natural_accounting");
                // 固定容量、失败关闭；不把未识别的后代等到退出后再记为通过。
                std::vector<ULONG_PTR> storage(257);
                auto members = reinterpret_cast<JOBOBJECT_BASIC_PROCESS_ID_LIST*>(storage.data());
                if (!QueryInformationJobObject(job.value, JobObjectBasicProcessIdList, members,
                    static_cast<DWORD>(storage.size() * sizeof(ULONG_PTR)), nullptr)) fail("job_natural_members");
                require(members->NumberOfAssignedProcesses == members->NumberOfProcessIdsInList
                    && members->NumberOfProcessIdsInList <= 256, "job_member_list_incomplete");
                std::string ids = "["; bool only_child = true;
                for (DWORD i = 0; i < members->NumberOfProcessIdsInList; ++i) {
                    if (i) ids += ',';
                    ids += number(members->ProcessIdList[i]);
                    if (members->ProcessIdList[i] != child.pid) only_child = false;
                }
                ids += ']';
                log->event("job_natural_accounting", ",\"phase\":" + json(phase)
                    + ",\"active_processes\":" + number(accounting.ActiveProcesses)
                    + ",\"assigned_processes\":" + number(members->NumberOfAssignedProcesses)
                    + ",\"member_pids\":" + ids);
                // 仅为首次未知成员补现场身份；不因此接纳该成员，也不按 PID 终止进程。
                if (!only_child) {
                    for (DWORD i = 0; i < members->NumberOfProcessIdsInList; ++i) {
                        const auto pid = members->ProcessIdList[i];
                        if (pid == child.pid) continue;
                        try {
                            require(pid <= MAXDWORD, "job_member_pid_range");
                            Handle member(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
                                FALSE, static_cast<DWORD>(pid)));
                            if (!member) fail("open_job_member_identity");
                            BOOL in_job = FALSE;
                            if (!IsProcessInJob(member.value, job.value, &in_job)) fail("query_job_member_identity");
                            const DWORD actual_pid = GetProcessId(member.value);
                            if (!actual_pid) fail("query_job_member_pid");
                            require(in_job && actual_pid == pid, "job_member_identity_changed");
                            const auto created = creation(member.value);
                            const auto path = image_path(member.value);
                            const DWORD wait = WaitForSingleObject(member.value, 0);
                            if (wait == WAIT_FAILED) fail("wait_job_member_identity");
                            DWORD exit_code = 0;
                            if (!GetExitCodeProcess(member.value, &exit_code)) fail("exit_job_member_identity");
                            log->event("job_other_member_identity", ",\"pid\":" + number(pid)
                                + ",\"creation_filetime\":" + number(created) + ",\"image_path\":" + json(utf8(path))
                                + ",\"in_owned_job\":true,\"wait_result\":" + number(wait)
                                + ",\"exit_code\":" + number(exit_code));
                        } catch (const Failure& e) {
                            log->event("job_other_member_identity_error", ",\"pid\":" + number(pid)
                                + ",\"operation\":" + json(e.operation) + ",\"win32\":" + number(e.code));
                        }
                        break;
                    }
                }
                require(only_child, "job_has_other_processes");
                return accounting.ActiveProcesses == 0 && members->NumberOfProcessIdsInList == 0;
            };
            observe_members("before_child_handle_release");
            child.thread.reset(); child.process.reset();
            log->event("owned_child_handles_released", ",\"pid\":" + number(child.pid));
            const auto settle_started = GetTickCount64();
            bool empty = false;
            do {
                empty = observe_members("after_child_handle_release");
                if (empty) break;
                Sleep(10);
            } while (GetTickCount64() - settle_started < 1000);
            log->event("job_natural_settle", ",\"empty\":" + boolean(empty)
                + ",\"elapsed_ms\":" + number(GetTickCount64() - settle_started));
            require(empty, "job_natural_accounting_not_settled");
        } catch (const Failure& e) {
            state.observer_error = e.operation; result = kObserverError;
            log->event("observer_error", ",\"operation\":" + json(e.operation) + ",\"win32\":" + number(e.code));
        }
    }
    // 仅终止并关闭自己创建的 Job；记录清理行为，不能把 EOF 超时改写成成功。
    if (job) {
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting{};
        if (!QueryInformationJobObject(job.value, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, nullptr)) {
            state.observer_error = "job_accounting_failed"; result = kObserverError;
        } else if (accounting.ActiveProcesses) {
            if (state.has_exit && !state.timed_out && state.observer_error.empty() && !state.eof_timeout) {
                state.observer_error = "job_has_live_descendants"; result = kObserverError;
            }
            if (!TerminateJobObject(job.value, kObserverError)) { state.observer_error = "job_cleanup_failed"; result = kObserverError; }
            else state.job_terminated = true;
        }
        const auto cleanup_started = GetTickCount64();
        do {
            if (!QueryInformationJobObject(job.value, JobObjectBasicAccountingInformation, &accounting, sizeof accounting, nullptr)) {
                state.observer_error = "job_final_accounting_failed"; result = kObserverError; break;
            }
            state.active_processes_final = accounting.ActiveProcesses;
            if (!accounting.ActiveProcesses) { state.job_empty = true; break; }
            Sleep(10);
        } while (GetTickCount64() - cleanup_started < 1000);
        if (!state.job_empty) { state.observer_error = "job_not_empty"; result = kObserverError; }
        log->event("job_final", ",\"job_empty\":" + boolean(state.job_empty)
            + ",\"active_processes\":" + (state.active_processes_final == MAXDWORD ? "null" : number(state.active_processes_final)));
        job.reset();
    }
    // AssignProcessToJobObject 失败时 child 尚未执行；仍只使用自己创建的精确句柄收回。
    if (child.process && WaitForSingleObject(child.process.value, 0) == WAIT_TIMEOUT) {
        if (!TerminateProcess(child.process.value, kObserverError)) state.observer_error = "owned_child_cleanup_failed";
        result = kObserverError;
        log->event("owned_child_fallback_terminate");
    }
    if (child.process && !state.has_exit && WaitForSingleObject(child.process.value, 1000) == WAIT_OBJECT_0) {
        if (GetExitCodeProcess(child.process.value, &state.child_exit)) state.has_exit = true;
    }
    // EOF 判定在上述 200ms 截止时已固定；此处只使本工具自己的 IO 线程退出。
    const bool stdout_eof = state.eof_deadline_recorded ? state.stdout_eof_at_deadline : false;
    const bool stderr_eof = state.eof_deadline_recorded ? state.stderr_eof_at_deadline : false;
    for (Reader* reader : {out, err}) {
        bool exited = false, cancelled = false, flushed = false;
        DWORD cancel_error = ERROR_SUCCESS;
        if (reader->thread) {
            DWORD wait = WaitForSingleObject(reader->thread.value, 1000);
            if (wait == WAIT_TIMEOUT) {
                cancelled = true;
                if (!CancelSynchronousIo(reader->thread.value)) cancel_error = GetLastError();
                // ERROR_NOT_FOUND 只说明此刻没有待取消 IO，仍必须独立确认线程退出。
                if (cancel_error != ERROR_SUCCESS && cancel_error != ERROR_NOT_FOUND) {
                    state.observer_error = "reader_cancel_failed"; result = kObserverError;
                }
                wait = WaitForSingleObject(reader->thread.value, 1000);
                if (state.observer_error.empty()) state.observer_error = "reader_required_cancellation";
                result = kObserverError;
            }
            exited = wait == WAIT_OBJECT_0;
            if (!exited) { state.observer_error = "reader_not_exited"; result = kObserverError; }
            if (reader->error) { state.observer_error = "reader_final_error"; result = kObserverError; }
        }
        // 只有写线程确实退出才做最后 flush；EOF 已出现不等于归档落盘成功。
        if (reader->archive && exited) {
            flushed = FlushFileBuffers(reader->archive.value) != FALSE;
            if (!flushed) { state.observer_error = "child_output_final_flush_failed"; result = kObserverError; }
        }
        log->event("reader_final", ",\"stream\":" + json(reader == out ? "stdout" : "stderr")
            + ",\"thread_exited\":" + boolean(exited) + ",\"cancel_attempted\":" + boolean(cancelled)
            + ",\"cancel_win32\":" + number(cancel_error) + ",\"archive_flushed\":" + boolean(flushed));
    }
    if (!state.observer_error.empty()) { try { state.stop("observer_error"); } catch (...) {} }
    const std::string summary = "{\"mode\":" + json(state.mode) + ",\"child_pid\":" + number(state.child_pid)
        + ",\"child_exit_code\":" + (state.has_exit ? number(state.child_exit) : "null")
        + ",\"stdout_eof\":" + boolean(stdout_eof) + ",\"stderr_eof\":" + boolean(stderr_eof)
        + ",\"eof_deadline_recorded\":" + boolean(state.eof_deadline_recorded)
        + ",\"libtest_success_marker\":" + boolean(out->test_ok && out->summary_ok)
        + ",\"libtest_test_ok\":" + boolean(out->test_ok) + ",\"libtest_summary_one_passed\":" + boolean(out->summary_ok)
        + ",\"sample_count\":" + number(state.samples) + ",\"resumed_threads\":" + number(state.resumed)
        + ",\"sampler_failures\":" + number(state.sampler_failures)
        + ",\"observer_error\":" + (state.observer_error.empty() ? "null" : json(state.observer_error))
        + ",\"timed_out\":" + boolean(state.timed_out) + ",\"eof_timeout\":" + boolean(state.eof_timeout)
        + ",\"stop_reason\":" + (state.stop_reason.empty() ? "null" : json(state.stop_reason))
        + ",\"job_terminated\":" + boolean(state.job_terminated) + ",\"job_empty\":" + boolean(state.job_empty)
        + ",\"active_processes_final\":" + (state.active_processes_final == MAXDWORD ? "null" : number(state.active_processes_final))
        + ",\"observer_exit_code\":" + number(static_cast<DWORD>(result)) + "}\n";
    log->event("observation_complete"); log->flush(); save(state.directory + L"\\summary.json", summary);
    return result;
}
} // namespace

int wmain(int argc, wchar_t** argv) {
    try {
        if (argc >= 2 && std::wcscmp(argv[1], L"--sample") == 0) {
            try { return sample_mode(argc, argv); }
            catch (const Failure& e) {
                try {
                    require(argc >= 4, "sample_error_output");
                    HANDLE output = reinterpret_cast<HANDLE>(parse_handle(argv[3]));
                    std::string line = "{\"type\":\"sample_error\",\"operation\":" + json(e.operation)
                        + ",\"win32\":" + number(e.code) + "}\n";
                    write_all(output, line.data(), line.size()); FlushFileBuffers(output);
                } catch (...) {}
                return kObserverError;
            }
        }
        if (argc == 3 && std::wcscmp(argv[1], L"--fixture") == 0) return fixture(argv[2]);
        if (argc == 2 && std::wcscmp(argv[1], L"--observe") == 0) return observe(L"");
        if (argc == 3 && std::wcscmp(argv[1], L"--protocol-test") == 0) {
            std::wstring mode = argv[2];
            require(mode == L"normal" || mode == L"nonzero" || mode == L"sample", "protocol_mode");
            return observe(mode);
        }
        throw Failure{"arguments_rejected", ERROR_INVALID_PARAMETER};
    } catch (const Failure& e) {
        // 错误仅输出固定操作枚举和 Win32 编号，不打印参数或环境值。
        std::fprintf(stderr, "windows_exit_observer: %s win32=%lu\n", e.operation, static_cast<unsigned long>(e.code));
    } catch (...) { std::fputs("windows_exit_observer: cpp_exception\n", stderr); }
    return kObserverError;
}
