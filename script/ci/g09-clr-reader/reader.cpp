#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
// bcrypt.h 依赖 Windows 基础类型，固定先包含 windows.h。
// clang-format off
#include <windows.h>
#include <bcrypt.h>
// clang-format on

#include "wire.h"
#include <algorithm>
#include <array>
#include <cstdio>
#include <cstring>
#include <cwchar>
#include <limits>
#include <sstream>
#include <string>
#include <vector>

// 与固定官方 IDL 中的定义相同；不拉入完整元数据写接口。
using mdToken = UINT32;
using mdTypeDef = mdToken;
using mdMethodDef = mdToken;
using mdFieldDef = mdToken;
using CorElementType = ULONG;
#include "vendor/xclrdata.h"

static_assert(sizeof(void *) == 8, "仅支持 x64");
static_assert(sizeof(wchar_t) == 2, "仅支持 Windows UTF-16");

namespace {
constexpr std::uint8_t magic[8] = {'G', '0', '9', 'C', 'L', 'R', '1', 0};
constexpr ULONG field_flags = CLRDATA_FIELD_ALL_KINDS | CLRDATA_FIELD_FROM_INSTANCE;

struct Handle {
  HANDLE value = nullptr;
  explicit Handle(HANDLE h = nullptr) : value(h) {}
  ~Handle() {
    if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value);
  }
  Handle(const Handle &) = delete;
  Handle &operator=(const Handle &) = delete;
};
template <class T> struct Com {
  T *value = nullptr;
  ~Com() {
    if (value) value->Release();
  }
  T **put() {
    if (value) value->Release();
    value = nullptr;
    return &value;
  }
  T *operator->() const { return value; }
  Com() = default;
  Com(const Com &) = delete;
  Com &operator=(const Com &) = delete;
};
struct Library {
  HMODULE value = nullptr;
  ~Library() {
    if (value) FreeLibrary(value);
  }
};

std::uint64_t ticks(const FILETIME &t) {
  return (std::uint64_t(t.dwHighDateTime) << 32) | t.dwLowDateTime;
}
HRESULT win_error() {
  return HRESULT_FROM_WIN32(GetLastError());
}
bool nonzero(const std::uint8_t *bytes, std::size_t size) {
  for (std::size_t i = 0; i < size; ++i)
    if (bytes[i]) return true;
  return false;
}
std::string hex(const std::uint8_t *bytes, std::size_t size) {
  constexpr char digits[] = "0123456789abcdef";
  std::string result;
  result.reserve(size * 2);
  for (std::size_t i = 0; i < size; ++i) {
    result.push_back(digits[bytes[i] >> 4]);
    result.push_back(digits[bytes[i] & 15]);
  }
  return result;
}
std::string guid(const GUID &value) {
  char text[37]{};
  std::snprintf(text, sizeof(text), "%08lx-%04x-%04x-%02x%02x-%02x%02x%02x%02x%02x%02x",
                static_cast<unsigned long>(value.Data1), unsigned(value.Data2), unsigned(value.Data3),
                unsigned(value.Data4[0]), unsigned(value.Data4[1]), unsigned(value.Data4[2]),
                unsigned(value.Data4[3]), unsigned(value.Data4[4]), unsigned(value.Data4[5]),
                unsigned(value.Data4[6]), unsigned(value.Data4[7]));
  return text;
}
bool read_exact(HANDLE file, void *output, DWORD size) {
  auto *bytes = static_cast<std::uint8_t *>(output);
  while (size) {
    DWORD got = 0;
    if (!ReadFile(file, bytes, size, &got, nullptr) || got == 0) return false;
    bytes += got;
    size -= got;
  }
  return true;
}

struct Result {
  const char *status = "unavailable";
  const char *stage = "request";
  HRESULT error = S_OK;
  bool identity = false;
  bool dac_hash = false;
  bool dac_loaded = false;
  bool exhausted = false;
  ULONG state_flags = 0;
  HRESULT exception_error = E_PENDING;
  HRESULT stack_error = E_PENDING;
  std::uint64_t read_bytes = 0;
  std::uint32_t read_calls = 0;
  std::vector<std::string> chain;
  std::vector<std::string> frames;
};

// 回调只读取当前原停点；不提供写入、TLS 修改、目标方法求值或目标执行控制。
class Target final : public ICLRDataTarget {
  const G09ClrRequest &request;
  Result &result;
  ULONG references = 1;

public:
  Target(const G09ClrRequest &r, Result &o) : request(r), result(o) {}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **output) override {
    if (!output) return E_POINTER;
    *output = nullptr;
    if (id == __uuidof(IUnknown) || id == __uuidof(ICLRDataTarget)) {
      *output = static_cast<ICLRDataTarget *>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  ULONG STDMETHODCALLTYPE AddRef() override { return ++references; }
  ULONG STDMETHODCALLTYPE Release() override { return --references; }
  bool expired() const { return GetTickCount64() >= request.deadline_tick_ms; }
  HRESULT STDMETHODCALLTYPE GetMachineType(ULONG32 *output) override {
    if (!output) return E_POINTER;
    *output = IMAGE_FILE_MACHINE_AMD64;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetPointerSize(ULONG32 *output) override {
    if (!output) return E_POINTER;
    *output = 8;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetImageBase(LPCWSTR path, CLRDATA_ADDRESS *output) override {
    if (!path || !output) return E_POINTER;
    *output = 0;
    // DAC 不得以目标内存中的任意路径打开文件或搜索磁盘。
    if (_wcsicmp(path, L"clr.dll") != 0) return E_NOTIMPL;
    *output = request.clr_base;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE ReadVirtual(CLRDATA_ADDRESS address, BYTE *output, ULONG32 requested,
                                        ULONG32 *read) override {
    if (!output || !read) return E_POINTER;
    *read = 0;
    if (expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    if (requested == 0) return S_OK;
    if (result.read_calls >= request.read_budget_calls ||
        requested > request.read_budget_bytes - result.read_bytes) {
      result.exhausted = true;
      return HRESULT_FROM_WIN32(ERROR_NOT_ENOUGH_QUOTA);
    }
    ++result.read_calls;
    result.read_bytes += requested;
    if (address == 0 || address > std::numeric_limits<std::uintptr_t>::max() - requested) return E_INVALIDARG;
    SIZE_T actual = 0;
    const BOOL ok = ReadProcessMemory(reinterpret_cast<HANDLE>(request.process_handle),
                                      reinterpret_cast<const void *>(address), output, requested, &actual);
    const DWORD error = ok ? ERROR_SUCCESS : GetLastError();
    if (actual > requested) return E_UNEXPECTED;
    *read = static_cast<ULONG32>(actual);
    return ok && actual == requested ? S_OK : HRESULT_FROM_WIN32(error ? error : ERROR_PARTIAL_COPY);
  }
  HRESULT STDMETHODCALLTYPE WriteVirtual(CLRDATA_ADDRESS, BYTE *, ULONG32, ULONG32 *written) override {
    if (written) *written = 0;
    return E_ACCESSDENIED;
  }
  HRESULT STDMETHODCALLTYPE GetTLSValue(ULONG32, ULONG32, CLRDATA_ADDRESS *) override { return E_NOTIMPL; }
  HRESULT STDMETHODCALLTYPE SetTLSValue(ULONG32, ULONG32, CLRDATA_ADDRESS) override { return E_ACCESSDENIED; }
  HRESULT STDMETHODCALLTYPE GetCurrentThreadID(ULONG32 *output) override {
    if (!output) return E_POINTER;
    *output = request.thread_id;
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetThreadContext(ULONG32 tid, ULONG32 flags, ULONG32 size,
                                             BYTE *output) override {
    if (expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    if (!output || tid != request.thread_id || size < sizeof(CONTEXT)) return E_INVALIDARG;
    // 仅允许当前异常线程的固定 CONTEXT；不枚举、不按 TID 重开、不写任何寄存器。
    if ((flags & CONTEXT_AMD64) == 0 || (flags & ~CONTEXT_ALL) != 0) return E_INVALIDARG;
    alignas(16) CONTEXT context{};
    context.ContextFlags = flags;
    if (!::GetThreadContext(reinterpret_cast<HANDLE>(request.thread_handle), &context)) return win_error();
    std::memcpy(output, &context, sizeof(context));
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE SetThreadContext(ULONG32, ULONG32, BYTE *) override { return E_ACCESSDENIED; }
  HRESULT STDMETHODCALLTYPE Request(ULONG32, ULONG32, BYTE *, ULONG32, BYTE *) override { return E_NOTIMPL; }
  HRESULT read(CLRDATA_ADDRESS address, void *output, ULONG32 size) {
    ULONG32 actual = 0;
    return ReadVirtual(address, static_cast<BYTE *>(output), size, &actual);
  }
};

HRESULT verify_target(const G09ClrRequest &r, Target &target) {
  const HANDLE process = reinterpret_cast<HANDLE>(r.process_handle);
  const HANDLE thread = reinterpret_cast<HANDLE>(r.thread_handle);
  const DWORD pid = GetProcessId(process);
  if (!pid) return win_error();
  const DWORD tid = GetThreadId(thread);
  if (!tid) return win_error();
  const DWORD thread_pid = GetProcessIdOfThread(thread);
  if (!thread_pid) return win_error();
  if (pid != r.process_id || tid != r.thread_id || thread_pid != r.process_id) return E_ACCESSDENIED;
  FILETIME birth{}, exit{}, kernel{}, user{};
  if (!GetProcessTimes(process, &birth, &exit, &kernel, &user)) return win_error();
  if (ticks(birth) != r.process_birth) return E_ACCESSDENIED;
  if (!GetThreadTimes(thread, &birth, &exit, &kernel, &user)) return win_error();
  if (ticks(birth) != r.thread_birth) return E_ACCESSDENIED;
  // 未退出时 ExitTime 未定义；不据其零值声称进程或线程存活。
  IMAGE_DOS_HEADER dos{};
  HRESULT hr = target.read(r.clr_base, &dos, sizeof(dos));
  if (FAILED(hr)) return hr;
  if (dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 ||
      static_cast<ULONG32>(dos.e_lfanew) > r.clr_image_size - sizeof(IMAGE_NT_HEADERS64))
    return E_INVALIDARG;
  IMAGE_NT_HEADERS64 pe{};
  hr = target.read(r.clr_base + static_cast<ULONG32>(dos.e_lfanew), &pe, sizeof(pe));
  if (FAILED(hr)) return hr;
  return pe.Signature == IMAGE_NT_SIGNATURE && pe.FileHeader.Machine == IMAGE_FILE_MACHINE_AMD64 &&
                 pe.OptionalHeader.Magic == IMAGE_NT_OPTIONAL_HDR64_MAGIC &&
                 pe.OptionalHeader.SizeOfImage == r.clr_image_size &&
                 pe.FileHeader.TimeDateStamp == r.clr_timestamp
             ? S_OK
             : E_ACCESSDENIED;
}

struct DacFile {
  std::vector<HANDLE> parents;
  Handle file;
  BY_HANDLE_FILE_INFORMATION identity{};
  ~DacFile() {
    for (HANDLE h : parents)
      CloseHandle(h);
  }
};
bool same_file(const BY_HANDLE_FILE_INFORMATION &a, const BY_HANDLE_FILE_INFORMATION &b) {
  return a.dwVolumeSerialNumber == b.dwVolumeSerialNumber && a.nFileIndexHigh == b.nFileIndexHigh &&
         a.nFileIndexLow == b.nFileIndexLow && a.nFileSizeHigh == b.nFileSizeHigh &&
         a.nFileSizeLow == b.nFileSizeLow && ticks(a.ftLastWriteTime) == ticks(b.ftLastWriteTime) &&
         a.dwFileAttributes == b.dwFileAttributes && a.nNumberOfLinks == b.nNumberOfLinks;
}
HRESULT bind_dac(const G09ClrRequest &r, const std::wstring &supplied, DacFile &bound, std::wstring &path) {
  wchar_t windows[G09_CLR_MAX_PATH_UNITS]{};
  const UINT length = GetWindowsDirectoryW(windows, G09_CLR_MAX_PATH_UNITS);
  if (!length) return win_error();
  if (length >= G09_CLR_MAX_PATH_UNITS) return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
  path = std::wstring(windows) + L"\\Microsoft.NET\\Framework64\\v4.0.30319\\mscordacwks.dll";
  const std::wstring plain = supplied.rfind(L"\\\\?\\", 0) == 0 ? supplied.substr(4) : supplied;
  if (path.size() > G09_CLR_MAX_PATH_UNITS || _wcsicmp(plain.c_str(), path.c_str()) != 0 || path.size() < 4 ||
      path[1] != L':' || path[2] != L'\\')
    return E_ACCESSDENIED;
  // 逐级拒绝重解析点并禁止父目录重命名；不改变目录权限。
  for (std::size_t end = 3; end < path.size(); end = path.find(L'\\', end + 1)) {
    if (end == std::wstring::npos) break;
    const std::wstring parent = path.substr(0, end);
    HANDLE h = CreateFileW(parent.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                           OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    if (h == INVALID_HANDLE_VALUE) return win_error();
    bound.parents.push_back(h);
    BY_HANDLE_FILE_INFORMATION info{};
    if (!GetFileInformationByHandle(h, &info)) return win_error();
    if (!(info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) ||
        (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT))
      return E_ACCESSDENIED;
  }
  bound.file.value = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING,
                                 FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN, nullptr);
  if (bound.file.value == INVALID_HANDLE_VALUE) return win_error();
  if (!GetFileInformationByHandle(bound.file.value, &bound.identity)) return win_error();
  const auto &id = bound.identity;
  if (id.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) ||
      ((std::uint64_t(id.nFileSizeHigh) << 32) | id.nFileSizeLow) != r.dac_file_size)
    return E_ACCESSDENIED;
  BCRYPT_ALG_HANDLE algorithm = nullptr;
  BCRYPT_HASH_HANDLE hash = nullptr;
  NTSTATUS status = BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0);
  if (status < 0) return HRESULT_FROM_NT(status);
  DWORD object_size = 0, actual = 0;
  status = BCryptGetProperty(algorithm, BCRYPT_OBJECT_LENGTH, reinterpret_cast<BYTE *>(&object_size),
                             sizeof(object_size), &actual, 0);
  if (status < 0 || actual != sizeof(object_size) || object_size > 65536) {
    BCryptCloseAlgorithmProvider(algorithm, 0);
    return E_FAIL;
  }
  std::vector<BYTE> object(object_size);
  status = BCryptCreateHash(algorithm, &hash, object.data(), object_size, nullptr, 0, 0);
  HRESULT hr = status < 0 ? HRESULT_FROM_NT(status) : S_OK;
  std::array<BYTE, 65536> buffer{};
  std::uint64_t offset = 0;
  while (SUCCEEDED(hr) && offset < r.dac_file_size) {
    if (GetTickCount64() >= r.deadline_tick_ms) {
      hr = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
      break;
    }
    const DWORD count =
        static_cast<DWORD>((std::min)(std::uint64_t(buffer.size()), r.dac_file_size - offset));
    DWORD got = 0;
    if (!ReadFile(bound.file.value, buffer.data(), count, &got, nullptr)) {
      hr = win_error();
      break;
    }
    if (got != count) {
      hr = E_FAIL;
      break;
    }
    status = BCryptHashData(hash, buffer.data(), got, 0);
    if (status < 0) {
      hr = HRESULT_FROM_NT(status);
      break;
    }
    offset += got;
  }
  std::array<BYTE, 32> digest{};
  if (SUCCEEDED(hr)) {
    status = BCryptFinishHash(hash, digest.data(), static_cast<ULONG>(digest.size()), 0);
    hr = status < 0 ? HRESULT_FROM_NT(status) : S_OK;
    if (SUCCEEDED(hr) && std::memcmp(digest.data(), r.dac_sha256, digest.size()) != 0) hr = E_ACCESSDENIED;
  }
  if (hash) BCryptDestroyHash(hash);
  BCryptCloseAlgorithmProvider(algorithm, 0);
  BY_HANDLE_FILE_INFORMATION after{};
  if (SUCCEEDED(hr)) {
    if (!GetFileInformationByHandle(bound.file.value, &after)) hr = win_error();
    else if (!same_file(id, after)) hr = E_ACCESSDENIED;
  }
  return hr;
}

HRESULT field(IXCLRDataValue *object, IXCLRDataTypeInstance *declaring, const wchar_t *name,
              Com<IXCLRDataValue> &output) {
  CLRDATA_ENUM iterator = 0;
  HRESULT hr =
      object->StartEnumFieldsByName(name, CLRDATA_BYNAME_CASE_SENSITIVE, field_flags, declaring, &iterator);
  if (hr != S_OK) return hr == S_FALSE ? E_NOINTERFACE : hr;
  mdFieldDef token = 0;
  hr = object->EnumFieldByName(&iterator, output.put(), &token);
  if (hr == S_OK) {
    Com<IXCLRDataValue> duplicate;
    const HRESULT next = object->EnumFieldByName(&iterator, duplicate.put(), &token);
    if (next != S_FALSE) hr = FAILED(next) ? next : E_UNEXPECTED;
  }
  const HRESULT ended = object->EndEnumFieldsByName(iterator);
  if (SUCCEEDED(hr) && FAILED(ended)) hr = ended;
  return hr == S_FALSE ? E_NOINTERFACE : hr;
}
HRESULT declaring_type(IXCLRDataValue *object, const wchar_t *wanted, Com<IXCLRDataTypeInstance> &output) {
  HRESULT hr = object->GetType(output.put());
  for (unsigned i = 0; hr == S_OK && output.value && i < 16; ++i) {
    wchar_t name[256]{};
    ULONG32 needed = 0;
    hr = output->GetName(0, 256, &needed, name);
    if (hr != S_OK || needed == 0 || needed > 256 || name[needed - 1] != 0)
      return FAILED(hr) ? hr : E_UNEXPECTED;
    if (std::wcscmp(name, wanted) == 0) return S_OK;
    Com<IXCLRDataTypeInstance> base;
    hr = output->GetBase(base.put());
    output.put();
    output.value = base.value;
    base.value = nullptr;
  }
  return FAILED(hr) ? hr : E_NOINTERFACE;
}
HRESULT scalar(IXCLRDataValue *value, std::int32_t &output) {
  ULONG64 size = 0;
  ULONG32 actual = 0;
  HRESULT hr = value->GetSize(&size);
  if (hr != S_OK || size != sizeof(output)) return FAILED(hr) ? hr : E_UNEXPECTED;
  hr = value->GetBytes(sizeof(output), &actual, reinterpret_cast<BYTE *>(&output));
  return hr == S_OK && actual == sizeof(output) ? S_OK : FAILED(hr) ? hr : E_UNEXPECTED;
}
const char *known_type(const wchar_t *name) {
  struct Known {
    const wchar_t *wide;
    const char *narrow;
  };
  static constexpr Known values[] = {
      {L"System.Exception", "System.Exception"},
      {L"System.ComponentModel.Win32Exception", "System.ComponentModel.Win32Exception"},
      {L"System.InvalidOperationException", "System.InvalidOperationException"},
      {L"System.ApplicationException", "System.ApplicationException"},
      {L"System.Management.Automation.ApplicationFailedException",
       "System.Management.Automation.ApplicationFailedException"},
      {L"System.UnauthorizedAccessException", "System.UnauthorizedAccessException"},
      {L"System.IO.IOException", "System.IO.IOException"},
      {L"System.ArgumentException", "System.ArgumentException"},
      {L"System.Security.SecurityException", "System.Security.SecurityException"},
      {L"System.TypeInitializationException", "System.TypeInitializationException"},
      {L"System.NullReferenceException", "System.NullReferenceException"}};
  for (const auto &value : values)
    if (std::wcscmp(name, value.wide) == 0) return value.narrow;
  return "unknown_type";
}
HRESULT collect_chain(IXCLRDataTask *task, Target &target, Result &result) {
  Com<IXCLRDataExceptionState> state;
  HRESULT hr = task->GetCurrentExceptionState(state.put());
  if (hr != S_OK || !state.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  hr = state->GetFlags(&result.state_flags);
  if (hr != S_OK) return hr;
  Com<IXCLRDataValue> current;
  hr = state->GetManagedObject(current.put());
  if (hr != S_OK || !current.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  std::array<CLRDATA_ADDRESS, G09_CLR_MAX_CHAIN> seen{};
  for (unsigned depth = 0; depth < G09_CLR_MAX_CHAIN; ++depth) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    CLRDATA_ADDRESS address = 0;
    hr = current->GetAddress(&address);
    if (hr != S_OK) return hr;
    for (unsigned i = 0; i < depth; ++i)
      if (seen[i] == address) return E_UNEXPECTED;
    seen[depth] = address;
    Com<IXCLRDataTypeInstance> type;
    Com<IXCLRDataTypeDefinition> definition;
    Com<IXCLRDataModule> module;
    GUID mvid{};
    mdTypeDef token = 0;
    wchar_t name[256]{};
    ULONG32 name_size = 0;
    hr = current->GetType(type.put());
    if (hr == S_OK) hr = type->GetName(0, 256, &name_size, name);
    if (hr == S_OK && name_size > 0 && name_size <= 256 && name[name_size - 1] == 0)
      hr = type->GetDefinition(definition.put());
    else if (SUCCEEDED(hr)) hr = E_UNEXPECTED;
    if (hr == S_OK) hr = definition->GetTokenAndScope(&token, module.put());
    if (hr == S_OK) hr = module->GetVersionId(&mvid);
    if (hr != S_OK) return hr;
    Com<IXCLRDataTypeInstance> exception_type;
    Com<IXCLRDataValue> hresult_value, inner_value, native_value;
    std::int32_t hresult = 0, native_error = 0;
    const HRESULT exception_type_hr = declaring_type(current.value, L"System.Exception", exception_type);
    HRESULT hresult_hr = exception_type_hr;
    if (hresult_hr == S_OK)
      hresult_hr = field(current.value, exception_type.value, L"_HResult", hresult_value);
    if (hresult_hr == S_OK) hresult_hr = scalar(hresult_value.value, hresult);
    HRESULT inner_hr = exception_type_hr == S_OK
                           ? field(current.value, exception_type.value, L"_innerException", inner_value)
                           : exception_type_hr;
    Com<IXCLRDataTypeInstance> win32_type;
    const HRESULT win32_type_hr =
        declaring_type(current.value, L"System.ComponentModel.Win32Exception", win32_type);
    HRESULT native_hr = win32_type_hr;
    if (native_hr == S_OK)
      native_hr = field(current.value, win32_type.value, L"nativeErrorCode", native_value);
    if (native_hr == S_OK) native_hr = scalar(native_value.value, native_error);
    Com<IXCLRDataValue> next;
    const char *inner_status = "unavailable";
    if (inner_hr == S_OK) {
      ULONG32 flags = 0, got = 0;
      ULONG64 size = 0;
      std::uint64_t reference = 0;
      inner_hr = inner_value->GetFlags(&flags);
      if (inner_hr == S_OK) inner_hr = inner_value->GetSize(&size);
      if (inner_hr == S_OK && size == 8 && (flags & CLRDATA_VALUE_IS_REFERENCE))
        inner_hr = inner_value->GetBytes(8, &got, reinterpret_cast<BYTE *>(&reference));
      else if (SUCCEEDED(inner_hr)) inner_hr = E_UNEXPECTED;
      if (inner_hr == S_OK && got == 8) {
        if (reference == 0) inner_status = "null";
        else if (depth + 1 == G09_CLR_MAX_CHAIN) inner_status = "truncated";
        else {
          inner_hr = inner_value->GetAssociatedValue(next.put());
          if (inner_hr == S_OK && next.value) inner_status = "object";
        }
      } else if (SUCCEEDED(inner_hr)) inner_hr = E_UNEXPECTED;
    }
    std::ostringstream item;
    item << "{\"type\":\"" << known_type(name) << "\",\"module_mvid\":\"" << guid(mvid)
         << "\",\"type_token\":" << token << ",\"hresult\":";
    if (hresult_hr == S_OK) item << static_cast<std::uint32_t>(hresult);
    else item << "null";
    item << ",\"hresult_read_hr\":" << static_cast<std::uint32_t>(hresult_hr) << ",\"native_error_code\":";
    if (native_hr == S_OK) item << native_error;
    else item << "null";
    item << ",\"native_error_read_hr\":" << static_cast<std::uint32_t>(native_hr) << ",\"inner_status\":\""
         << inner_status << "\",\"inner_read_hr\":" << static_cast<std::uint32_t>(inner_hr)
         << ",\"current_state_flags\":" << result.state_flags << "}";
    result.chain.push_back(item.str());
    if (hresult_hr != S_OK) return hresult_hr;
    if (win32_type_hr != S_OK && win32_type_hr != E_NOINTERFACE) return win32_type_hr;
    if (win32_type_hr == S_OK && native_hr != S_OK) return native_hr;
    if (std::strcmp(inner_status, "null") == 0) return S_OK;
    if (std::strcmp(inner_status, "object") != 0) return FAILED(inner_hr) ? inner_hr : S_FALSE;
    current.put();
    current.value = next.value;
    next.value = nullptr;
  }
  return S_FALSE;
}
HRESULT collect_stack(IXCLRDataTask *task, Target &target, Result &result) {
  Com<IXCLRDataStackWalk> walk;
  HRESULT hr =
      task->CreateStackWalk(CLRDATA_SIMPFRAME_MANAGED_METHOD | CLRDATA_SIMPFRAME_RUNTIME_MANAGED_CODE |
                                CLRDATA_SIMPFRAME_RUNTIME_UNMANAGED_CODE,
                            walk.put());
  if (hr != S_OK || !walk.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  bool partial = false;
  for (unsigned n = 0; n < G09_CLR_MAX_FRAMES; ++n) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    hr = walk->Next();
    if (hr == S_FALSE) return partial || result.frames.empty() ? S_FALSE : S_OK;
    if (hr != S_OK) return hr;
    Com<IXCLRDataFrame> frame;
    Com<IXCLRDataMethodInstance> method;
    Com<IXCLRDataModule> module;
    hr = walk->GetFrame(frame.put());
    if (hr != S_OK || !frame.value) return FAILED(hr) ? hr : E_NOINTERFACE;
    hr = frame->GetMethodInstance(method.put());
    // 非托管帧没有 MethodDef；不伪造最近符号，也不追加其它解栈器。
    if (hr == S_FALSE || hr == E_NOINTERFACE) continue;
    if (hr != S_OK || !method.value) return FAILED(hr) ? hr : E_NOINTERFACE;
    mdMethodDef token = 0;
    GUID mvid{};
    hr = method->GetTokenAndScope(&token, module.put());
    if (hr == S_OK) hr = module->GetVersionId(&mvid);
    if (hr != S_OK) return hr;
    alignas(16) CONTEXT context{};
    ULONG32 context_size = 0;
    HRESULT mapping =
        walk->GetContext(CONTEXT_CONTROL, sizeof(context), &context_size, reinterpret_cast<BYTE *>(&context));
    std::array<ULONG32, 8> offsets{};
    ULONG32 needed = 0;
    if (mapping == S_OK && context_size >= offsetof(CONTEXT, Rip) + sizeof(context.Rip) &&
        context_size <= sizeof(context))
      mapping = method->GetILOffsetsByAddress(context.Rip, static_cast<ULONG32>(offsets.size()), &needed,
                                              offsets.data());
    else if (SUCCEEDED(mapping)) mapping = E_UNEXPECTED;
    if (mapping == S_OK && (needed == 0 || needed > offsets.size())) mapping = S_FALSE;
    if (mapping != S_OK) partial = true;
    std::ostringstream item;
    item << "{\"module_mvid\":\"" << guid(mvid) << "\",\"method_token\":" << token
         << ",\"il_status\":" << static_cast<std::uint32_t>(mapping) << ",\"il_offsets\":[";
    if (mapping == S_OK && needed <= offsets.size()) {
      for (ULONG32 i = 0; i < needed; ++i) {
        if (i) item << ',';
        item << offsets[i];
      }
    }
    item << "],\"il_offsets_needed\":" << needed << "}";
    result.frames.push_back(item.str());
  }
  return S_FALSE;
}

void observe(const G09ClrRequest &r, const std::wstring &supplied, Result &result) {
  Handle process(reinterpret_cast<HANDLE>(r.process_handle));
  Handle thread(reinterpret_cast<HANDLE>(r.thread_handle));
  Target target(r, result);
  result.stage = "target_identity";
  result.error = verify_target(r, target);
  if (result.error != S_OK) return;
  result.identity = true;
  DacFile file;
  std::wstring path;
  result.stage = "dac_file";
  result.error = bind_dac(r, supplied, file, path);
  if (result.error != S_OK) return;
  result.dac_hash = true;
  if (target.expired()) {
    result.error = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    return;
  }
  Library library;
  result.stage = "dac_load";
  library.value =
      LoadLibraryExW(path.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
  if (!library.value) {
    result.error = win_error();
    return;
  }
  // 核对实际 HMODULE 的文件，不能仅再次打开请求路径就声称已加载该原件。
  wchar_t module_path[G09_CLR_MAX_PATH_UNITS]{};
  const DWORD module_length = GetModuleFileNameW(library.value, module_path, G09_CLR_MAX_PATH_UNITS);
  if (!module_length) {
    result.error = win_error();
    return;
  }
  if (module_length >= G09_CLR_MAX_PATH_UNITS) {
    result.error = HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
    return;
  }
  std::wstring actual_path(module_path, module_length);
  if (actual_path.rfind(L"\\\\?\\", 0) == 0) actual_path.erase(0, 4);
  if (_wcsicmp(actual_path.c_str(), path.c_str()) != 0) {
    result.error = E_ACCESSDENIED;
    return;
  }
  Handle loaded(CreateFileW(actual_path.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ, nullptr,
                            OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr));
  BY_HANDLE_FILE_INFORMATION after{};
  if (loaded.value == INVALID_HANDLE_VALUE) {
    result.error = win_error();
    return;
  }
  if (!GetFileInformationByHandle(loaded.value, &after)) {
    result.error = win_error();
    return;
  }
  if (!same_file(file.identity, after)) {
    result.error = E_ACCESSDENIED;
    return;
  }
  result.dac_loaded = true;
  auto create =
      reinterpret_cast<PFN_CLRDataCreateInstance>(GetProcAddress(library.value, "CLRDataCreateInstance"));
  if (!create) {
    result.error = win_error();
    return;
  }
  Com<IXCLRDataProcess> clr;
  result.stage = "dac_create";
  result.error = create(__uuidof(IXCLRDataProcess), &target, reinterpret_cast<void **>(clr.put()));
  if (result.error != S_OK || !clr.value) {
    if (result.error == S_OK) result.error = E_NOINTERFACE;
    return;
  }
  Com<IXCLRDataTask> task;
  result.stage = "current_task";
  result.error = clr->GetTaskByOSThreadID(r.thread_id, task.put());
  if (result.error != S_OK || !task.value) {
    if (result.error == S_OK) result.error = E_NOINTERFACE;
    return;
  }
  ULONG32 tid = 0;
  result.error = task->GetOSThreadID(&tid);
  if (result.error != S_OK) return;
  if (tid != r.thread_id) {
    result.error = E_ACCESSDENIED;
    return;
  }
  // 同一原停点读取完成前不 Continue；不以 LastException 或 GetPrevious
  // 替代当前异常/InnerException。
  result.stage = "observation";
  result.exception_error = collect_chain(task.value, target, result);
  result.stack_error = target.expired()   ? HRESULT_FROM_WIN32(ERROR_TIMEOUT)
                       : result.exhausted ? HRESULT_FROM_WIN32(ERROR_NOT_ENOUGH_QUOTA)
                                          : collect_stack(task.value, target, result);
  result.status = result.exception_error == S_OK && result.stack_error == S_OK &&
                          !(result.state_flags & CLRDATA_EXCEPTION_PARTIAL)
                      ? "observed"
                      : "partial";
  result.error = S_OK;
}

bool valid_request(const G09ClrRequest &r) {
  if (std::memcmp(r.magic, magic, 8) != 0 || r.version != 1 || !nonzero(r.nonce, 16) ||
      r.deadline_tick_ms <= GetTickCount64())
    return false;
  if (r.operation == 2) {
    G09ClrRequest allowed{};
    std::memcpy(allowed.magic, r.magic, 8);
    allowed.version = 1;
    allowed.operation = 2;
    std::memcpy(allowed.nonce, r.nonce, 16);
    allowed.deadline_tick_ms = r.deadline_tick_ms;
    return std::memcmp(&allowed, &r, sizeof(r)) == 0;
  }
  return r.operation == 1 && r.event_sequence && r.process_id && r.thread_id && r.process_birth &&
         r.thread_birth && r.process_handle && r.thread_handle && r.process_handle != r.thread_handle &&
         r.process_handle < (std::uint64_t(1) << 63) && r.thread_handle < (std::uint64_t(1) << 63) &&
         r.clr_base && r.clr_image_size >= sizeof(IMAGE_NT_HEADERS64) &&
         r.clr_base <= std::numeric_limits<std::uint64_t>::max() - r.clr_image_size && r.read_budget_bytes &&
         r.read_budget_bytes <= G09_CLR_MAX_READ_BYTES && r.read_budget_calls &&
         r.read_budget_calls <= G09_CLR_MAX_READ_CALLS && r.dac_file_size &&
         r.dac_file_size <= 64 * 1024 * 1024 && nonzero(r.dac_sha256, 32) && r.dac_path_units &&
         r.dac_path_units <= G09_CLR_MAX_PATH_UNITS && r.first_chance == 1 && r.exception_code == 0xe0434352;
}
std::string output(const G09ClrRequest &r, const Result &result) {
  std::ostringstream text;
  text << "{\"schema\":1,\"operation\":" << r.operation << ",\"nonce\":\"" << hex(r.nonce, 16)
       << "\",\"event_sequence\":" << r.event_sequence << ",\"pid\":" << r.process_id
       << ",\"tid\":" << r.thread_id << ",\"process_birth\":" << r.process_birth
       << ",\"thread_birth\":" << r.thread_birth << ",\"status\":\"" << result.status << "\",\"stage\":\""
       << result.stage << "\",\"api_hresult\":" << static_cast<std::uint32_t>(result.error)
       << ",\"target_identity_verified\":" << (result.identity ? "true" : "false")
       << ",\"dac_sha256_verified\":" << (result.dac_hash ? "true" : "false")
       << ",\"dac_loaded\":" << (result.dac_loaded ? "true" : "false")
       << ",\"event_hresult\":" << r.exception_hresult
       << ",\"exception_api_hresult\":" << static_cast<std::uint32_t>(result.exception_error)
       << ",\"stack_api_hresult\":" << static_cast<std::uint32_t>(result.stack_error)
       << ",\"current_state_flags\":" << result.state_flags << ",\"read_bytes\":" << result.read_bytes
       << ",\"read_calls\":" << result.read_calls
       << ",\"budget_exhausted\":" << (result.exhausted ? "true" : "false") << ",\"chain\":[";
  for (std::size_t i = 0; i < result.chain.size(); ++i) {
    if (i) text << ',';
    text << result.chain[i];
  }
  text << "],\"frames\":[";
  for (std::size_t i = 0; i < result.frames.size(); ++i) {
    if (i) text << ',';
    text << result.frames[i];
  }
  text << "]}\n";
  return text.str();
}
} // namespace

int main(int argc, char **) {
  G09ClrRequest request{};
  Result result;
  int exit_code = 2;
  try {
    const HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
    if (argc == 1 && read_exact(input, &request, sizeof(request)) && valid_request(request)) {
      std::wstring path(request.dac_path_units, L'\0');
      if ((path.empty() || read_exact(input, path.data(), request.dac_path_units * 2)) &&
          path.find(L'\0') == std::wstring::npos) {
        BYTE extra = 0;
        DWORD got = 0;
        const BOOL ok = ReadFile(input, &extra, 1, &got, nullptr);
        const DWORD error = ok ? ERROR_SUCCESS : GetLastError();
        if (got == 0 && (ok || error == ERROR_BROKEN_PIPE) && valid_request(request)) {
          if (request.operation == 2) Sleep(INFINITE);
          observe(request, path, result);
          exit_code = result.identity && result.dac_loaded ? 0 : 3;
        }
      }
    }
    if (exit_code == 2) result.error = E_INVALIDARG;
  } catch (...) {
    result.status = "unavailable";
    result.stage = "reader_exception";
    result.error = E_FAIL;
    exit_code = 3;
  }
  const std::string bytes = output(request, result);
  if (bytes.size() > G09_CLR_MAX_OUTPUT_BYTES) return 4;
  DWORD written = 0;
  if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), bytes.data(), static_cast<DWORD>(bytes.size()), &written,
                 nullptr) ||
      written != bytes.size())
    return 4;
  return exit_code;
}
