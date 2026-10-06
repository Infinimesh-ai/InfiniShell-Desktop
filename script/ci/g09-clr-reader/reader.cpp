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
using T_CONTEXT = CONTEXT;
#include "sos_layout.h"
#include "vendor/sospriv.h"

static_assert(sizeof(void *) == 8, "仅支持 x64");
static_assert(sizeof(wchar_t) == 2, "仅支持 Windows UTF-16");

namespace {
constexpr std::uint8_t magic[8] = {'G', '0', '9', 'C', 'L', 'R', '1', 0};
constexpr CorElementType element_i4 = 0x08;
constexpr CorElementType element_class = 0x12;
constexpr CorElementType element_boolean = 0x02;

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
  const char *exception_source = "none";
  HRESULT error = S_OK;
  bool identity = false;
  bool dac_hash = false;
  bool dac_loaded = false;
  bool exhausted = false;
  bool object_chain_complete = false;
  ULONG32 state_flags = 0;
  HRESULT exception_error = E_PENDING;
  HRESULT stack_error = E_PENDING;
  std::uint64_t read_bytes = 0;
  std::uint32_t read_calls = 0;
  std::vector<std::string> chain;
  std::vector<std::string> frames;
  std::string managed_mapping;
  std::string managed_continuation;
  std::string private_target;
  std::string managed_map_notes = "null";
  ULONG32 managed_matches = 0;
  HRESULT managed_error = E_PENDING;
  bool managed_bound = false;
  bool managed_values = false;
  ULONG32 scanned_frames = 0;
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

struct BoundType {
  CLRDATA_ADDRESS method_table = 0;
  DacpMethodTableData data{};
  Com<IXCLRDataModule> module;
  Com<IXCLRDataTypeDefinition> definition;
  GUID mvid{};
  wchar_t name[256]{};
};

HRESULT bind_type(ISOSDacInterface *sos, CLRDATA_ADDRESS method_table, BoundType &output) {
  if (!method_table) return E_INVALIDARG;
  output.method_table = method_table;
  HRESULT hr = sos->GetMethodTableData(method_table, &output.data);
  if (hr != S_OK) return hr;
  const auto &data = output.data;
  if (data.bIsFree || data.bIsDynamic || !data.Module || data.ComponentSize || data.BaseSize < 16 ||
      (data.cl & 0xff000000) != 0x02000000 || (data.cl & 0x00ffffff) == 0)
    return E_UNEXPECTED;
  hr = sos->GetModule(data.Module, output.module.put());
  if (hr != S_OK || !output.module.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  hr = output.module->GetTypeDefinitionByToken(data.cl, output.definition.put());
  if (hr != S_OK || !output.definition.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  Com<IXCLRDataModule> scope;
  mdTypeDef token = 0;
  hr = output.definition->GetTokenAndScope(&token, scope.put());
  if (hr != S_OK || !scope.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  if (token != data.cl) return E_UNEXPECTED;
  hr = scope->IsSameObject(output.module.value);
  if (hr != S_OK) return FAILED(hr) ? hr : E_UNEXPECTED;
  hr = output.module->GetVersionId(&output.mvid);
  if (hr != S_OK) return hr;
  if (!nonzero(reinterpret_cast<const std::uint8_t *>(&output.mvid), sizeof(output.mvid)))
    return E_UNEXPECTED;
  ULONG32 needed = 0;
  hr = output.definition->GetName(0, 256, &needed, output.name);
  if (hr != S_OK || needed == 0 || needed > 256 || output.name[needed - 1] != 0)
    return FAILED(hr) ? hr : E_UNEXPECTED;
  return S_OK;
}

HRESULT declaring_type(ISOSDacInterface *sos, CLRDATA_ADDRESS method_table,
                       const DacpUsefulGlobalsData &globals, const wchar_t *wanted,
                       CLRDATA_ADDRESS required_table, Target &target, BoundType &output) {
  std::array<CLRDATA_ADDRESS, 16> seen{};
  for (unsigned i = 0; i < seen.size(); ++i) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    for (unsigned j = 0; j < i; ++j)
      if (seen[j] == method_table) return E_UNEXPECTED;
    seen[i] = method_table;
    HRESULT hr = bind_type(sos, method_table, output);
    if (hr != S_OK) return hr;
    if (std::wcscmp(output.name, wanted) == 0)
      return !required_table || method_table == required_table ? S_OK : E_UNEXPECTED;
    // 只在实际全局 Object MT 和元数据名称同时吻合时确认可选基类不存在。
    if (method_table == globals.ObjectMethodTable)
      return std::wcscmp(output.name, L"System.Object") == 0 ? E_NOINTERFACE : E_UNEXPECTED;
    method_table = output.data.ParentMethodTable;
  }
  return E_UNEXPECTED;
}

HRESULT field(ISOSDacInterface *sos, BoundType &declaring, const wchar_t *wanted, Target &target,
              DacpFieldDescData &output) {
  DacpMethodTableFieldData fields{}, parent{};
  HRESULT hr = sos->GetMethodTableFieldData(declaring.method_table, &fields);
  if (hr != S_OK) return hr;
  if (declaring.data.ParentMethodTable) {
    hr = sos->GetMethodTableFieldData(declaring.data.ParentMethodTable, &parent);
    if (hr != S_OK) return hr;
  }
  if (fields.wNumInstanceFields < parent.wNumInstanceFields) return E_UNEXPECTED;
  // 实例计数包含继承字段；FirstField 仅指本类字段，不能把父类再枚举一次。
  const unsigned count =
      unsigned(fields.wNumInstanceFields) - parent.wNumInstanceFields + fields.wNumStaticFields;
  CLRDATA_ADDRESS address = fields.FirstField;
  bool found = false;
  for (unsigned i = 0; i < count; ++i) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    if (!address) return E_UNEXPECTED;
    DacpFieldDescData value{};
    hr = sos->GetFieldDescData(address, &value);
    if (hr != S_OK) return hr;
    if (value.MTOfEnclosingClass != declaring.method_table || value.ModuleOfType != declaring.data.Module ||
        (value.mb & 0xff000000) != 0x04000000 || (value.mb & 0x00ffffff) == 0)
      return E_UNEXPECTED;
    wchar_t name[256]{};
    ULONG32 needed = 0;
    // type/flags 留空：只核同模块 FieldDef 名称，不经 Value 的声明类型构造对象。
    hr = declaring.definition->GetFieldByToken2(declaring.module.value, value.mb, 256, &needed, name, nullptr,
                                                nullptr);
    if (hr != S_OK || needed == 0 || needed > 256 || name[needed - 1] != 0)
      return FAILED(hr) ? hr : E_UNEXPECTED;
    if (std::wcscmp(name, wanted) == 0) {
      if (found || value.bIsStatic || value.bIsThreadLocal || value.bIsContextLocal) return E_UNEXPECTED;
      output = value;
      found = true;
    }
    if (i + 1 < count && value.NextField <= address) return E_UNEXPECTED;
    address = value.NextField;
  }
  return found ? S_OK : E_NOINTERFACE;
}

HRESULT read_field(Target &target, CLRDATA_ADDRESS object, const DacpObjectData &data,
                   const DacpFieldDescData &field_data, CorElementType expected, void *output, ULONG32 size) {
  if (field_data.Type != expected || field_data.sigType != expected) return E_UNEXPECTED;
  // 普通对象地址指向 MT；FieldDesc offset 从其后开始，Size 还包含前置对象头。
  const std::uint64_t offset = sizeof(void *) + std::uint64_t(field_data.dwOffset);
  if (data.Size < 2 * sizeof(void *) || offset > data.Size - sizeof(void *) ||
      size > data.Size - sizeof(void *) - offset ||
      object > std::numeric_limits<std::uint64_t>::max() - offset)
    return E_UNEXPECTED;
  return target.read(object + offset, output, size);
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
HRESULT collect_chain(IXCLRDataProcess *clr, IXCLRDataTask *task, Target &target, Result &result) {
  Com<ISOSDacInterface> sos;
  HRESULT hr = clr->QueryInterface(__uuidof(ISOSDacInterface), reinterpret_cast<void **>(sos.put()));
  if (hr != S_OK || !sos.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  DacpUsefulGlobalsData globals{};
  hr = sos->GetUsefulGlobals(&globals);
  if (hr != S_OK) return hr;
  if (!globals.ObjectMethodTable || !globals.ExceptionMethodTable ||
      globals.ObjectMethodTable == globals.ExceptionMethodTable)
    return E_UNEXPECTED;
  Com<IXCLRDataExceptionState> state;
  // first-chance 先于 CLR tracker 更新；保留 last-thrown 候选来源及原 PARTIAL 标志。
  result.exception_source = "last_thrown_object_candidate";
  hr = task->GetLastExceptionState(state.put());
  if (hr != S_OK || !state.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  hr = state->GetFlags(&result.state_flags);
  if (hr != S_OK) return hr;
  Com<IXCLRDataValue> root;
  hr = state->GetManagedObject(root.put());
  if (hr != S_OK || !root.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  CLRDATA_ADDRESS address = 0;
  hr = root->GetAddress(&address);
  if (hr != S_OK) return hr;
  std::array<CLRDATA_ADDRESS, G09_CLR_MAX_CHAIN> seen{};
  for (unsigned depth = 0; depth < G09_CLR_MAX_CHAIN; ++depth) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    if (!address) return E_UNEXPECTED;
    for (unsigned i = 0; i < depth; ++i)
      if (seen[i] == address) return E_UNEXPECTED;
    seen[depth] = address;
    // 每一层重新由对象实际 MT 取类型；引用字段的声明类型不能代表 inner 的动态类型。
    DacpObjectData object{};
    hr = sos->GetObjectData(address, &object);
    if (hr != S_OK) return hr;
    // 官方分类的 OBJ_OBJECT 仅指 System.Object；异常等普通派生类归为 OBJ_OTHER。
    if (object.ObjectType != OBJ_OTHER || !object.MethodTable || object.Size < 16) return E_UNEXPECTED;
    BoundType type, exception_type, win32_type;
    hr = bind_type(sos.value, object.MethodTable, type);
    if (hr != S_OK) return hr;
    if (object.Size != type.data.BaseSize) return E_UNEXPECTED;
    DacpFieldDescData hresult_field{}, inner_field{}, native_field{};
    std::int32_t hresult = 0, native_error = 0;
    const HRESULT exception_type_hr =
        declaring_type(sos.value, object.MethodTable, globals, L"System.Exception",
                       globals.ExceptionMethodTable, target, exception_type);
    HRESULT hresult_hr = exception_type_hr;
    if (hresult_hr == S_OK) hresult_hr = field(sos.value, exception_type, L"_HResult", target, hresult_field);
    if (hresult_hr == S_OK)
      hresult_hr = read_field(target, address, object, hresult_field, element_i4, &hresult, sizeof(hresult));
    HRESULT inner_hr = exception_type_hr == S_OK
                           ? field(sos.value, exception_type, L"_innerException", target, inner_field)
                           : exception_type_hr;
    const HRESULT win32_type_hr =
        exception_type_hr == S_OK
            ? declaring_type(sos.value, object.MethodTable, globals, L"System.ComponentModel.Win32Exception",
                             0, target, win32_type)
            : exception_type_hr;
    HRESULT native_hr = win32_type_hr;
    if (native_hr == S_OK) native_hr = field(sos.value, win32_type, L"nativeErrorCode", target, native_field);
    if (native_hr == S_OK)
      native_hr =
          read_field(target, address, object, native_field, element_i4, &native_error, sizeof(native_error));
    CLRDATA_ADDRESS reference = 0;
    const char *inner_status = "unavailable";
    if (inner_hr == S_OK) {
      inner_hr =
          read_field(target, address, object, inner_field, element_class, &reference, sizeof(reference));
      if (inner_hr == S_OK) {
        if (reference == 0) inner_status = "null";
        else if (depth + 1 == G09_CLR_MAX_CHAIN) inner_status = "truncated";
        else inner_status = "object";
      }
    }
    std::ostringstream item;
    item << "{\"type\":\"" << known_type(type.name) << "\",\"module_mvid\":\"" << guid(type.mvid)
         << "\",\"type_token\":" << type.data.cl << ",\"hresult\":";
    if (hresult_hr == S_OK) item << static_cast<std::uint32_t>(hresult);
    else item << "null";
    item << ",\"hresult_read_hr\":" << static_cast<std::uint32_t>(hresult_hr) << ",\"native_error_code\":";
    if (native_hr == S_OK) item << native_error;
    else item << "null";
    item << ",\"native_error_read_hr\":" << static_cast<std::uint32_t>(native_hr) << ",\"inner_status\":\""
         << inner_status << "\",\"inner_read_hr\":" << static_cast<std::uint32_t>(inner_hr)
         << ",\"exception_state_flags\":" << result.state_flags << "}";
    result.chain.push_back(item.str());
    if (hresult_hr != S_OK) return hresult_hr;
    if (win32_type_hr != S_OK && win32_type_hr != E_NOINTERFACE) return win32_type_hr;
    if (win32_type_hr == S_OK && native_hr != S_OK) return native_hr;
    if (std::strcmp(inner_status, "null") == 0) return S_OK;
    if (std::strcmp(inner_status, "object") != 0) return FAILED(inner_hr) ? inner_hr : S_FALSE;
    address = reference;
  }
  return S_FALSE;
}
struct TerminalMapEvidence {
  const char *stage = "entry";
  HRESULT extent_end_hr = E_PENDING;
  ULONG32 map_count = 0;
  ULONG32 entry_index = 0;
  ULONG32 match_count = 0;
  ULONG64 extent_length = 0;
  ULONG64 ip_offset = 0;
  ULONG64 entry_start_offset = 0;
  bool extent_bound = false;
  bool ip_after_entry = false;
  bool entry_bound = false;
};

HRESULT terminal_il_mapping(IXCLRDataMethodInstance *method, CLRDATA_ADDRESS ip, Target &target,
                            ULONG32 &offset, TerminalMapEvidence &evidence) {
  // 旧 DAC 可能漏掉末条非 EPILOG、nativeEndOffset 为零的有效 IL 映射。
  // 只补同一方法的已知结束标记；不修改 IP，也不选择最近项或默认 IL。
  constexpr ULONG32 max_maps = 256;
  if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
  CLRDATA_ADDRESS entry = 0;
  HRESULT hr = method->GetRepresentativeEntryAddress(&entry);
  if (hr != S_OK) return hr;
  if (!entry) return E_UNEXPECTED;
  evidence.stage = "extent_start";
  if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
  CLRDATA_ENUM handle = 0;
  hr = method->StartEnumExtents(&handle);
  if (hr != S_OK) return hr;
  CLRDATA_ADDRESS_RANGE extent{}, extra{};
  // 这里只支持一个完整代码范围；第二次枚举必须明确结束，且每条路径都释放枚举器。
  const HRESULT extent_hr =
      target.expired() ? HRESULT_FROM_WIN32(ERROR_TIMEOUT) : method->EnumExtent(&handle, &extent);
  const HRESULT extra_hr = extent_hr != S_OK  ? E_PENDING
                           : target.expired() ? HRESULT_FROM_WIN32(ERROR_TIMEOUT)
                                              : method->EnumExtent(&handle, &extra);
  const HRESULT end_hr = method->EndEnumExtents(handle);
  evidence.extent_end_hr = end_hr;
  evidence.stage = "extent_read";
  if (extent_hr != S_OK) return extent_hr == S_FALSE ? E_NOINTERFACE : extent_hr;
  evidence.stage = "extent_limit";
  if (extra_hr != S_FALSE) return extra_hr == S_OK ? S_FALSE : extra_hr;
  evidence.stage = "extent_end";
  if (end_hr != S_OK) return end_hr;
  evidence.stage = "extent_binding";
  if (extent.startAddress != entry || extent.endAddress <= entry) return E_UNEXPECTED;
  evidence.extent_bound = true;
  evidence.extent_length = extent.endAddress - entry;
  evidence.ip_after_entry = ip >= entry;
  if (evidence.ip_after_entry) evidence.ip_offset = ip - entry;
  evidence.stage = "ip_outside_extent";
  // 保留等于 end 的相对偏移，但不把返回地址端点自动当作范围内指令。
  if (ip < entry || ip >= extent.endAddress) return E_NOINTERFACE;
  evidence.stage = "map_read";
  if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
  std::array<CLRDATA_IL_ADDRESS_MAP, max_maps> maps{};
  hr = method->GetILAddressMap(max_maps, &evidence.map_count, maps.data());
  if (hr != S_OK) return hr;
  evidence.stage = "map_limit";
  if (evidence.map_count == 0 || evidence.map_count > max_maps) return S_FALSE;
  evidence.entry_index = evidence.map_count - 1;
  const auto &last = maps[evidence.entry_index];
  evidence.stage = "terminal_marker";
  if (last.ilOffset >= 0xfffffffdU || last.endAddress != entry || last.startAddress < entry ||
      last.startAddress >= extent.endAddress)
    return E_NOINTERFACE;
  evidence.entry_bound = true;
  evidence.entry_start_offset = last.startAddress - entry;
  evidence.stage = "map_ranges";
  for (ULONG32 i = 0; i < evidence.map_count; ++i) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    const auto &map = maps[i];
    const bool terminal = i == evidence.entry_index;
    const CLRDATA_ADDRESS end = terminal ? extent.endAddress : map.endAddress;
    if (map.startAddress < entry || map.startAddress >= extent.endAddress || end < map.startAddress ||
        end > extent.endAddress || (!terminal && end != maps[i + 1].startAddress))
      return E_UNEXPECTED;
    if (map.startAddress <= ip && ip < end) {
      ++evidence.match_count;
      if (!terminal || map.ilOffset >= 0xfffffffdU) return E_NOINTERFACE;
    }
  }
  evidence.stage = "unique_terminal_match";
  if (evidence.match_count != 1) return E_NOINTERFACE;
  offset = last.ilOffset;
  evidence.stage = "matched";
  return S_OK;
}

HRESULT hash_bytes(const BYTE *bytes, ULONG length, std::uint8_t digest[32]) {
  BCRYPT_ALG_HANDLE algorithm = nullptr;
  BCRYPT_HASH_HANDLE hash = nullptr;
  NTSTATUS status = BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0);
  if (status < 0) return HRESULT_FROM_NT(status);
  DWORD size = 0, actual = 0;
  status = BCryptGetProperty(algorithm, BCRYPT_OBJECT_LENGTH, reinterpret_cast<BYTE *>(&size), sizeof(size), &actual, 0);
  HRESULT hr = S_OK;
  if (status < 0 || actual != sizeof(size) || size > 65536) hr = status < 0 ? HRESULT_FROM_NT(status) : E_UNEXPECTED;
  std::vector<BYTE> object(size <= 65536 ? size : 0);
  if (hr == S_OK) {
    status = BCryptCreateHash(algorithm, &hash, object.data(), size, nullptr, 0, 0);
    if (status >= 0) status = BCryptHashData(hash, const_cast<BYTE *>(bytes), length, 0);
    if (status >= 0) status = BCryptFinishHash(hash, digest, 32, 0);
    if (status < 0) hr = HRESULT_FROM_NT(status);
  }
  if (hash) BCryptDestroyHash(hash);
  BCryptCloseAlgorithmProvider(algorithm, 0);
  return hr;
}

std::string managed_safe(const G09ManagedRequest &m) {
  std::ostringstream out;
  out << "{\"module_mvid\":\"" << std::string(m.module_mvid, 36) << "\",\"method_token\":" << m.method_token
      << ",\"il_offset\":" << m.il_offset << ",\"enc_version\":" << m.enc_version << ",\"map_count\":" << m.map_count
      << ",\"map_sha256\":\"" << hex(m.map_sha256,32) << "\",\"code_sha256\":\"" << hex(m.code_sha256,32)
      << "\",\"extent_length\":" << m.extent_end-m.extent_start << ",\"pre_sequence\":" << m.pre_sequence << "}";
  return out.str();
}

HRESULT managed_map(IXCLRDataMethodInstance *method, const CONTEXT &context, const G09ClrRequest &r,
                    const G09ManagedRequest &spec, Target &target, G09ManagedRequest &bound, std::string &notes) {
  bound = spec;
  bound.map_count = 0;
  CLRDATA_ADDRESS entry = 0;
  CLRDATA_ADDRESS_RANGE extent{}, extra{};
  std::array<CLRDATA_IL_ADDRESS_MAP,256> maps{};
  std::array<bool,256> closed{}, checked{};
  std::array<HRESULT,4> reverse_hr{};
  std::array<bool,4> reversed{};
  std::array<ULONG32,4> reverse_count{}, reverse_il{};
  bool map_returned = false;
  CLRDATA_ADDRESS excluded = 0;
  std::vector<ULONG32> open_indices;
  std::vector<ULONG32> zero_indices;
  ULONG32 failed_index = UINT32_MAX;
  ULONG32 zero_sentinels = 0;
  const auto finish = [&](const char *stage, HRESULT status) {
    std::ostringstream note;
    const auto relative = [&](CLRDATA_ADDRESS address) {
      if(address>=entry) note << address-entry;
      else note << '-' << entry-address;
    };
    note << "{\"stage\":\"" << stage << "\",\"map_count\":" << bound.map_count
         << ",\"map_complete\":" << (map_returned ? "true" : "false") << ",\"extent_length\":";
    if(extent.endAddress>entry) note << extent.endAddress-entry; else note << "null";
    note << ",\"failed_map_index\":";
    if(failed_index!=UINT32_MAX) note << failed_index; else note << "null";
    note << ",\"raw_il\":";
    if(failed_index!=UINT32_MAX) note << maps[failed_index].ilOffset; else note << "null";
    note << ",\"start_offset\":";
    if(failed_index!=UINT32_MAX) relative(maps[failed_index].startAddress); else note << "null";
    note << ",\"end_offset\":";
    if(failed_index!=UINT32_MAX) relative(maps[failed_index].endAddress); else note << "null";
    note << ",\"returned_il_offsets\":[";
    if(map_returned) for(ULONG32 i=0;i<bound.map_count;++i) {
      if(i) note << ',';
      note << maps[i].ilOffset;
    }
    note << "],\"approved_points\":[";
    for(ULONG32 approved=0;approved<spec.approved_count;++approved) {
      if(approved) note << ',';
      ULONG32 count=0,index=0;
      if(map_returned) for(ULONG32 i=0;i<bound.map_count;++i)
        if(maps[i].ilOffset==spec.approved_il[approved]) { ++count; index=i; }
      note << "{\"il_offset\":" << spec.approved_il[approved] << ",\"match_count\":";
      if(map_returned) note << count; else note << "null";
      note << ",\"closed_range\":";
      if(count==1 && checked[index]) note << (closed[index] ? "true" : "false"); else note << "null";
      note << ",\"reverse_hresult\":";
      if(reversed[approved]) note << static_cast<std::uint32_t>(reverse_hr[approved]); else note << "null";
      note << ",\"reverse_count\":";
      if(reversed[approved]) note << reverse_count[approved]; else note << "null";
      note << ",\"reverse_il_offset\":";
      if(reversed[approved] && reverse_hr[approved]==S_OK && reverse_count[approved]==1) note << reverse_il[approved];
      else note << "null";
      note << '}';
    }
    note << "],\"open_exclusions\":[";
    for(std::size_t n=0;n<open_indices.size();++n) {
      if(n) note << ',';
      const auto i=open_indices[n];
      note << "{\"index\":" << i << ",\"raw_il\":" << maps[i].ilOffset << ",\"start_offset\":";
      relative(maps[i].startAddress);
      note << ",\"end_offset\":";
      relative(maps[i].endAddress);
      note << '}';
    }
    note << "],\"zero_length_entries\":[";
    for(std::size_t n=0;n<zero_indices.size();++n) {
      if(n) note << ',';
      const auto i=zero_indices[n];
      note << "{\"index\":" << i << ",\"raw_il\":" << maps[i].ilOffset << ",\"offset\":";
      relative(maps[i].startAddress);
      note << '}';
    }
    const bool terminal_open=!open_indices.empty() && open_indices.back()+1==bound.map_count;
    note << "],\"zero_length_entries_selectable\":false,\"zero_length_entries_occupy_range\":false"
         << ",\"zero_length_sentinel_count\":" << zero_sentinels << ",\"terminal_exclusion\":{\"present\":"
         << (terminal_open ? "true" : "false") << ",\"raw_il\":";
    if(terminal_open) note << maps[bound.map_count-1].ilOffset; else note << "null";
    note << ",\"start_offset\":";
    if(terminal_open) relative(maps[bound.map_count-1].startAddress); else note << "null";
    note << ",\"raw_end_offset\":" << (terminal_open ? "0" : "null") << ",\"excluded_to_extent_end\":"
         << (terminal_open ? "true" : "false") << "}}";
    notes=note.str();
    return status;
  };
  HRESULT hr = method->GetRepresentativeEntryAddress(&entry);
  if (hr != S_OK) return finish("entry_api",hr);
  if (target.expired()) return finish("enc_deadline",HRESULT_FROM_WIN32(ERROR_TIMEOUT));
  hr = method->GetEnCVersion(&bound.enc_version);
  if (hr != S_OK) return finish("enc_api",hr);
  CLRDATA_ENUM iterator = 0;
  hr = method->StartEnumExtents(&iterator);
  if (hr != S_OK) return finish("extent_start_api",hr);
  const HRESULT first = target.expired() ? HRESULT_FROM_WIN32(ERROR_TIMEOUT) : method->EnumExtent(&iterator,&extent);
  const HRESULT second = first == S_OK && !target.expired() ? method->EnumExtent(&iterator,&extra) : E_PENDING;
  const HRESULT ended = method->EndEnumExtents(iterator);
  if (first != S_OK) return finish("extent_first_api",first);
  if (second != S_FALSE) return finish("extent_unique",second == S_OK ? E_UNEXPECTED : second);
  if (ended != S_OK) return finish("extent_end_api",ended);
  if (entry < 0x10000 || extent.startAddress != entry || extent.endAddress <= entry ||
      extent.endAddress-entry > 65536 || extent.endAddress > 0x0000800000000000ULL ||
      context.Rip < entry || context.Rip >= extent.endAddress || context.Rsp < 0x10000 || context.Rsp % 8)
    return finish("extent_context_bounds",E_UNEXPECTED);
  if (target.expired()) return finish("map_deadline",HRESULT_FROM_WIN32(ERROR_TIMEOUT));
  hr = method->GetILAddressMap(static_cast<ULONG32>(maps.size()),&bound.map_count,maps.data());
  if (hr != S_OK) return finish("map_api",hr);
  if (!bound.map_count || bound.map_count > maps.size()) return finish("map_completeness",E_UNEXPECTED);
  map_returned = true;
  std::vector<BYTE> canonical;
  for (ULONG32 i=0;i<bound.map_count;++i) {
    const auto &map = maps[i];
    failed_index=i;
    if(map.startAddress<entry || map.startAddress>extent.endAddress) return finish("map_start_bounds",E_UNEXPECTED);
    checked[i]=true;
    // 旧 DAC 任意 EPILOG 都可开放；完整保全，只排除其后代码，不推算到下一 funclet。
    if((map.ilOffset==0xfffffffdU || (i+1==bound.map_count && map.ilOffset<0xfffffffdU)) &&
       map.endAddress==entry && map.startAddress>entry && map.startAddress<extent.endAddress) {
      open_indices.push_back(i);
      excluded=excluded ? (std::min)(excluded,map.startAddress) : map.startAddress;
    }
    // GetILAddressMap 保留普通 IL 的零长记录；半开空集无覆盖，也不能选作续点。
    else if(map.startAddress==map.endAddress) {
      zero_indices.push_back(i);
      if(map.ilOffset>=0xfffffffdU) ++zero_sentinels;
    }
    else {
      if(map.endAddress<=map.startAddress || map.endAddress>extent.endAddress) return finish("map_range_bounds",E_UNEXPECTED);
      closed[i]=true;
      for(ULONG32 j=0;j<i;++j)
        if(closed[j] && map.startAddress<maps[j].endAddress && maps[j].startAddress<map.endAddress)
          return finish("map_overlap",E_UNEXPECTED);
    }
    const std::uint64_t fields[] = {map.ilOffset,map.startAddress-entry,map.endAddress-entry,static_cast<std::uint64_t>(map.type)};
    for (const auto number:fields) for(unsigned b=0;b<8;++b) canonical.push_back(static_cast<BYTE>(number>>(8*b)));
  }
  failed_index=UINT32_MAX;
  bool selected = false;
  for (ULONG32 approved=0;approved<spec.approved_count && !selected;++approved) {
    ULONG32 count=0;
    ULONG32 index=0;
    CLRDATA_ADDRESS address=0;
    bool closed_candidate=false;
    for (ULONG32 i=0;i<bound.map_count;++i) if(maps[i].ilOffset==spec.approved_il[approved]) {
      ++count; index=i; address=maps[i].startAddress; closed_candidate=closed[i];
    }
    if (!count) continue;
    if (count != 1 || !closed_candidate) return finish("approved_not_unique_closed",E_UNEXPECTED);
    if(excluded && maps[index].endAddress>excluded) {
      failed_index=index;
      return finish("approved_open_exclusion_overlap",E_UNEXPECTED);
    }
    ULONG32 needed=0;
    std::array<ULONG32,8> offsets{};
    hr=method->GetILOffsetsByAddress(address,static_cast<ULONG32>(offsets.size()),&needed,offsets.data());
    reversed[approved]=true; reverse_hr[approved]=hr; reverse_count[approved]=needed; reverse_il[approved]=offsets[0];
    if(hr!=S_OK) return finish("reverse_api",hr);
    if(needed!=1 || offsets[0]!=spec.approved_il[approved]) return finish("reverse_not_unique_exact",E_UNEXPECTED);
    bound.il_offset=offsets[0]; bound.address=address; selected=true;
  }
  if(!selected) return finish("approved_missing",E_NOINTERFACE);
  bound.extent_start=entry; bound.extent_end=extent.endAddress; bound.frame_rsp=context.Rsp;
  if(r.operation==3) { bound.pre_sequence=r.event_sequence; std::memcpy(bound.pre_nonce,r.nonce,16); }
  hr=hash_bytes(canonical.data(),static_cast<ULONG>(canonical.size()),bound.map_sha256);
  if(hr!=S_OK) return finish("map_hash",hr);
  std::vector<BYTE> code(static_cast<std::size_t>(extent.endAddress-entry));
  if(target.expired()) return finish("code_deadline",HRESULT_FROM_WIN32(ERROR_TIMEOUT));
  hr=target.read(entry,code.data(),static_cast<ULONG32>(code.size()));
  if(hr!=S_OK) return finish("code_read",hr);
  hr=hash_bytes(code.data(),static_cast<ULONG>(code.size()),bound.code_sha256);
  return finish(hr==S_OK ? "bound" : "code_hash",hr);
}

std::string private_managed(const G09ClrRequest &r,const G09ManagedRequest &m) {
  std::string safe=managed_safe(m);
  safe.pop_back();
  std::ostringstream out;
  out << safe << ",\"process_id\":" << r.process_id << ",\"thread_id\":" << r.thread_id
      << ",\"process_birth\":" << r.process_birth << ",\"thread_birth\":" << r.thread_birth
      << ",\"pre_nonce\":\"" << hex(m.pre_nonce,16) << "\",\"frame_rsp\":" << m.frame_rsp
      << ",\"extent_start\":" << m.extent_start << ",\"extent_end\":" << m.extent_end << ",\"address\":" << m.address << "}";
  return out.str();
}

struct LocalRead {
  HRESULT get_local_hr=E_PENDING;
  HRESULT api=E_PENDING,locations_hr=E_PENDING,type_hr=E_PENDING,name_hr=E_PENDING,flags_hr=E_PENDING,size_hr=E_PENDING,bytes_hr=E_PENDING;
  ULONG32 locations=0,flags=0,bytes_read=0;
  ULONG64 size=0;
  bool type_matched=false;
  std::string fields(const char *prefix) const {
    std::ostringstream out;
    out << "\"" << prefix << "api_hresult\":" << static_cast<std::uint32_t>(api)
        << ",\"" << prefix << "get_local_hresult\":" << static_cast<std::uint32_t>(get_local_hr)
        << ",\"" << prefix << "locations_hresult\":" << static_cast<std::uint32_t>(locations_hr)
        << ",\"" << prefix << "locations\":" << locations
        << ",\"" << prefix << "type_hresult\":" << static_cast<std::uint32_t>(type_hr)
        << ",\"" << prefix << "type_name_hresult\":" << static_cast<std::uint32_t>(name_hr)
        << ",\"" << prefix << "flags_hresult\":" << static_cast<std::uint32_t>(flags_hr)
        << ",\"" << prefix << "flags\":" << flags
        << ",\"" << prefix << "size_hresult\":" << static_cast<std::uint32_t>(size_hr)
        << ",\"" << prefix << "size\":" << size
        << ",\"" << prefix << "bytes_hresult\":" << static_cast<std::uint32_t>(bytes_hr)
        << ",\"" << prefix << "bytes_read\":" << bytes_read;
    return out.str();
  }
};

HRESULT local_bytes(IXCLRDataFrame *frame,ULONG32 index,const wchar_t *expected,ULONG32 flags,ULONG32 size,
                    BYTE *bytes,Target &target,LocalRead &read) {
  if(target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
  Com<IXCLRDataValue> value;
  HRESULT hr=frame->GetLocalVariableByIndex(index,value.put(),0,nullptr,nullptr);
  read.get_local_hr=hr;
  if(hr!=S_OK || !value.value) return hr==S_OK ? E_NOINTERFACE : hr;
  read.locations_hr=value->GetNumLocations(&read.locations);
  if(read.locations_hr!=S_OK) return read.locations_hr;
  if(!read.locations || read.locations>2) return E_NOINTERFACE;
  read.flags_hr=value->GetFlags(&read.flags);
  if(read.flags_hr!=S_OK) return read.flags_hr;
  if((read.flags & CLRDATA_VALUE_ALL_KINDS)!=flags) return E_UNEXPECTED;
  // 引用 Value 的 GetType 按 DAC 合同返回 S_FALSE；读取引用后由实际对象 MT 核对类型。
  if(flags!=CLRDATA_VALUE_IS_REFERENCE) {
    Com<IXCLRDataTypeInstance> type;
    read.type_hr=value->GetType(type.put());
    if(read.type_hr!=S_OK || !type.value) return read.type_hr==S_OK ? E_NOINTERFACE : read.type_hr;
    wchar_t name[256]{}; ULONG32 needed=0;
    read.name_hr=type->GetName(0,256,&needed,name);
    if(read.name_hr!=S_OK) return read.name_hr;
    if(!needed || needed>256 || name[needed-1] || std::wcscmp(name,expected)!=0) return E_UNEXPECTED;
    read.type_matched=true;
  }
  read.size_hr=value->GetSize(&read.size);
  if(read.size_hr!=S_OK) return read.size_hr;
  if(read.size!=size) return E_UNEXPECTED;
  if(target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
  read.bytes_hr=value->GetBytes(size,&read.bytes_read,bytes);
  if(read.bytes_hr!=S_OK) return read.bytes_hr;
  return read.bytes_read==size ? S_OK : E_UNEXPECTED;
}

std::string managed_values(IXCLRDataProcess *clr,IXCLRDataFrame *frame,const G09ManagedRequest &m,
                           Target &target,bool &complete) {
  BYTE boolean=0;
  LocalRead local;
  local.api=local_bytes(frame,m.bool_local_index,L"System.Boolean",CLRDATA_VALUE_IS_PRIMITIVE,1,&boolean,target,local);
  if(local.api==S_OK && boolean>1) local.api=E_UNEXPECTED;
  complete=local.api==S_OK;
  std::ostringstream out;
  out << "\"boolean_local\":{\"index\":" << m.bool_local_index << ',' << local.fields("")
      << ",\"type_name\":\"" << (local.type_matched ? "System.Boolean" : "unknown_type") << "\",\"value\":";
  if(local.api==S_OK) out << (boolean ? "true" : "false"); else out << "null";
  const bool requested=m.start_info_local_index!=UINT32_MAX;
  LocalRead reference;
  HRESULT field_hr=E_PENDING,field_read_hr=E_PENDING;
  std::string module;
  mdTypeDef type_token=0; DacpFieldDescData descriptor{};
  BYTE field_value=0;
  if(requested) {
    CLRDATA_ADDRESS address=0;
    reference.api=local_bytes(frame,m.start_info_local_index,L"System.Diagnostics.ProcessStartInfo",CLRDATA_VALUE_IS_REFERENCE,
                             sizeof(address),reinterpret_cast<BYTE *>(&address),target,reference);
    field_hr=reference.api;
    if(field_hr==S_OK && (address<0x10000 || address>0x00007fffffffffffULL || address%8)) field_hr=E_UNEXPECTED;
    Com<ISOSDacInterface> sos;
    DacpObjectData object{}; BoundType type;
    if(field_hr==S_OK) field_hr=clr->QueryInterface(__uuidof(ISOSDacInterface),reinterpret_cast<void **>(sos.put()));
    if(field_hr==S_OK && !sos.value) field_hr=E_NOINTERFACE;
    if(field_hr==S_OK) field_hr=sos->GetObjectData(address,&object);
    if(field_hr==S_OK && (object.ObjectType!=OBJ_OTHER || !object.MethodTable || object.Size<16)) field_hr=E_UNEXPECTED;
    if(field_hr==S_OK) field_hr=bind_type(sos.value,object.MethodTable,type);
    if(field_hr==S_OK && (std::wcscmp(type.name,L"System.Diagnostics.ProcessStartInfo") || object.Size!=type.data.BaseSize)) field_hr=E_UNEXPECTED;
    if(field_hr==S_OK) { module=guid(type.mvid); type_token=type.data.cl; field_hr=field(sos.value,type,L"useShellExecute",target,descriptor); }
    if(field_hr==S_OK) { field_read_hr=read_field(target,address,object,descriptor,element_boolean,&field_value,1); field_hr=field_read_hr; }
    if(field_hr==S_OK && field_value>1) field_hr=E_UNEXPECTED;
    complete=complete && field_hr==S_OK;
  }
  out << "},\"start_info_field\":{\"requested\":" << (requested ? "true" : "false") << ",\"index\":";
  if(requested) out << m.start_info_local_index; else out << "null";
  out << ",\"api_hresult\":" << static_cast<std::uint32_t>(field_hr) << ',' << reference.fields("local_")
      << ",\"module_mvid\":\"" << module << "\",\"type_token\":" << type_token << ",\"field_token\":" << descriptor.mb
      << ",\"field_name\":\"useShellExecute\",\"field_type\":" << descriptor.Type << ",\"field_sig_type\":" << descriptor.sigType
      << ",\"field_read_hresult\":" << static_cast<std::uint32_t>(field_read_hr) << ",\"value\":";
  if(field_hr==S_OK) out << (field_value ? "true" : "false"); else out << "null";
  out << "}";
  return out.str();
}

HRESULT collect_stack(IXCLRDataTask *task, ULONG32 thread_id, bool native_return, Target &target, Result &result,
                      const G09ClrRequest &request, const G09ManagedRequest *managed, IXCLRDataProcess *clr) {
  Com<IXCLRDataStackWalk> walk;
  HRESULT hr =
      task->CreateStackWalk(CLRDATA_SIMPFRAME_MANAGED_METHOD | CLRDATA_SIMPFRAME_RUNTIME_MANAGED_CODE |
                                CLRDATA_SIMPFRAME_RUNTIME_UNMANAGED_CODE,
                            walk.put());
  if (hr != S_OK || !walk.value) return FAILED(hr) ? hr : E_NOINTERFACE;
  // DAC 默认可能取旧 EH filter context；这里只更新读取器内的游标，不写目标线程。
  alignas(16) CONTEXT stopped_context{};
  hr = target.GetThreadContext(thread_id, CONTEXT_FULL, sizeof(stopped_context),
                               reinterpret_cast<BYTE *>(&stopped_context));
  if (hr != S_OK) return hr;
  hr = walk->SetContext2(CLRDATA_STACK_SET_CURRENT_CONTEXT, sizeof(stopped_context),
                         reinterpret_cast<BYTE *>(&stopped_context));
  if (hr != S_OK) return hr;
  bool partial = false;
  bool metadata_frame = false;
  for (unsigned n = 0; n < G09_CLR_MAX_FRAMES; ++n) {
    if (target.expired()) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    // Init/SetContext2 已定位首帧；每次后续迭代才推进，非托管帧也只推进一次。
    if (n != 0) {
      hr = walk->Next();
      if (hr == S_FALSE) return partial || !metadata_frame ? S_FALSE : S_OK;
      if (hr != S_OK) return hr;
    }
    result.scanned_frames=n+1;
    CLRDataSimpleFrameType simple_type;
    CLRDataDetailedFrameType detailed_type;
    hr = walk->GetFrameType(&simple_type, &detailed_type);
    if (hr == S_FALSE) return partial || !metadata_frame ? S_FALSE : S_OK;
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
    const HRESULT context_hr =
        walk->GetContext(CONTEXT_CONTROL, sizeof(context), &context_size, reinterpret_cast<BYTE *>(&context));
    if(managed && token==managed->method_token && guid(mvid)==std::string(managed->module_mvid,36)) {
      ++result.managed_matches;
      G09ManagedRequest bound{};
      HRESULT managed_hr=context_hr;
      if(result.managed_matches!=1) managed_hr=E_UNEXPECTED;
      else if(context_hr==S_OK && context_size<=sizeof(context) && context_size>=offsetof(CONTEXT,Rip)+sizeof(context.Rip))
        managed_hr=managed_map(method.value,context,request,*managed,target,bound,result.managed_map_notes);
      else if(context_hr==S_OK) managed_hr=E_UNEXPECTED;
      if(request.operation==4 && managed_hr==S_OK) {
        if(!result.frames.empty() || context.Rip!=managed->address || stopped_context.Rip!=managed->address ||
            std::memcmp(reinterpret_cast<const BYTE *>(&bound)+68,reinterpret_cast<const BYTE *>(managed)+68,sizeof(bound)-68)!=0)
          managed_hr=E_UNEXPECTED;
      }
      if(result.managed_matches!=1) managed_hr=E_UNEXPECTED;
      result.managed_error=managed_hr;
      if(managed_hr==S_OK) {
        result.managed_mapping=managed_safe(bound);
        if(request.operation==3) result.private_target=private_managed(request,bound);
        else {
          result.managed_bound=true;
          result.managed_continuation=managed_values(clr,frame.value,bound,target,result.managed_values);
        }
      } else { result.private_target.clear(); result.managed_bound=false; result.managed_mapping.clear(); }
    }
    HRESULT mapping = context_hr;
    HRESULT mapping_hr = E_PENDING;
    HRESULT terminal_hr = E_PENDING;
    TerminalMapEvidence terminal;
    const char *mapping_source = "direct";
    std::array<ULONG32, 8> offsets{};
    ULONG32 needed = 0;
    if (mapping == S_OK && context_size >= offsetof(CONTEXT, Rip) + sizeof(context.Rip) &&
        context_size <= sizeof(context)) {
      mapping_hr = method->GetILOffsetsByAddress(context.Rip, static_cast<ULONG32>(offsets.size()), &needed,
                                                 offsets.data());
      mapping = mapping_hr;
      if (mapping_hr == E_NOINTERFACE) {
        terminal_hr = terminal_il_mapping(method.value, context.Rip, target, offsets[0], terminal);
        mapping = terminal_hr;
        needed = terminal_hr == S_OK ? 1 : 0;
        if (terminal_hr == S_OK) mapping_source = "terminal_end_marker";
      }
    } else if (SUCCEEDED(mapping)) mapping = E_UNEXPECTED;
    if (mapping == S_OK && (needed == 0 || needed > offsets.size())) mapping = S_FALSE;
    // 同一 MethodInstance 的完整名称区分 Framework 生成的互操作桩；不输出名称/签名。
    HRESULT name_hr = E_PENDING;
    ULONG32 name_needed = 0;
    bool name_complete = false;
    bool pinvoke_name = false;
    const char *runtime_name = "not_requested";
    if (token == 0x06000000) {
      std::array<WCHAR, 512> name{};
      name_hr = method->GetName(0, static_cast<ULONG32>(name.size()), &name_needed, name.data());
      runtime_name = "unavailable";
      if (name_hr == S_OK && name_needed > 0 && name_needed <= name.size() &&
          name[name_needed - 1] == L'\0' && wcsnlen(name.data(), name.size()) + 1 == name_needed) {
        name_complete = true;
        const std::wstring value(name.data(), name_needed - 1);
        runtime_name = value.rfind(L"DomainBoundILStubClass.IL_STUB_PInvoke(", 0) == 0
                           ? "domain_bound_pinvoke_stub"
                       : value.rfind(L"DomainNeutralILStubClass.IL_STUB_PInvoke(", 0) == 0
                           ? "domain_neutral_pinvoke_stub"
                           : "unknown";
        pinvoke_name = std::strcmp(runtime_name, "unknown") != 0;
      }
    }
    // 原生返回的首帧可为没有元数据 MethodDef 的已确认 P/Invoke 桩。
    // 保留原 E_FAIL 和空 IL，不把该桩冒充托管调用者；其它映射失败仍是不完整栈。
    const bool runtime_pinvoke = native_return && n == 0 && token == 0x06000000 &&
                                 simple_type == CLRDATA_SIMPFRAME_MANAGED_METHOD && detailed_type == 0 &&
                                 context_hr == S_OK && mapping == E_FAIL && mapping_hr == E_FAIL &&
                                 needed == 0 && pinvoke_name;
    const bool metadata_method = (token & 0xff000000) == 0x06000000 && (token & 0x00ffffff) != 0;
    if (!runtime_pinvoke && (!metadata_method || mapping != S_OK)) partial = true;
    if (metadata_method && mapping == S_OK) metadata_frame = true;
    std::ostringstream item;
    item << "{\"module_mvid\":\"" << guid(mvid) << "\",\"method_token\":" << token
         << ",\"frame_kind\":\"" << (runtime_pinvoke ? "runtime_pinvoke_stub" : metadata_method ? "metadata_method" : "unknown_no_metadata") << "\""
         << ",\"simple_frame_type\":" << static_cast<unsigned>(simple_type)
         << ",\"detailed_frame_type\":" << static_cast<unsigned>(detailed_type)
         << ",\"name_hresult\":" << static_cast<std::uint32_t>(name_hr)
         << ",\"name_complete\":" << (name_complete ? "true" : "false")
         << ",\"name_units_needed\":" << name_needed << ",\"runtime_name\":\"" << runtime_name << "\""
         << ",\"context_hresult\":" << static_cast<std::uint32_t>(context_hr)
         << ",\"mapping_hresult\":" << static_cast<std::uint32_t>(mapping_hr) << ",\"mapping_source\":\""
         << mapping_source << "\""
         << ",\"il_status\":" << static_cast<std::uint32_t>(mapping) << ",\"il_offsets\":[";
    if (mapping == S_OK && needed <= offsets.size()) {
      for (ULONG32 i = 0; i < needed; ++i) {
        if (i) item << ',';
        item << offsets[i];
      }
    }
    item << "],\"il_offsets_needed\":" << needed;
    if (mapping_hr == E_NOINTERFACE) {
      item << ",\"terminal_map\":{\"stage\":\"" << terminal.stage
           << "\",\"hresult\":" << static_cast<std::uint32_t>(terminal_hr)
           << ",\"extent_end_hresult\":" << static_cast<std::uint32_t>(terminal.extent_end_hr)
           << ",\"map_count\":" << terminal.map_count << ",\"match_count\":" << terminal.match_count
           << ",\"extent_length\":";
      if (terminal.extent_bound) item << terminal.extent_length;
      else item << "null";
      item << ",\"ip_offset\":";
      if (terminal.extent_bound && terminal.ip_after_entry) item << terminal.ip_offset;
      else item << "null";
      item << ",\"entry_index\":";
      if (terminal.entry_bound) item << terminal.entry_index;
      else item << "null";
      item << ",\"entry_start_offset\":";
      if (terminal.entry_bound) item << terminal.entry_start_offset;
      else item << "null";
      item << ",\"entry_end_offset\":" << (terminal.entry_bound ? "0" : "null") << "}";
    }
    item << "}";
    result.frames.push_back(item.str());
  }
  return S_FALSE;
}

void observe(const G09ClrRequest &r, const std::wstring &supplied, const G09ManagedRequest *managed, Result &result) {
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
  // 同一原停点读取完成前不 Continue；候选新鲜性由固定夹具逐事件核验，
  // 不凭 HRESULT 相同宣称当前对象，也不把 GetPrevious 当作 InnerException。
  result.stage = "observation";
  if (r.operation == 1) {
    result.exception_error = collect_chain(clr.value, task.value, target, result);
    result.object_chain_complete = result.exception_error == S_OK && !result.chain.empty();
  }
  // operation 3 是调用方自有的原生返回单步，只读当前 CLR 栈，不查询异常对象。
  result.stack_error = target.expired()   ? HRESULT_FROM_WIN32(ERROR_TIMEOUT)
                       : result.exhausted ? HRESULT_FROM_WIN32(ERROR_NOT_ENOUGH_QUOTA)
                                          : collect_stack(task.value, r.thread_id, r.operation == 3, target, result,r,managed,clr.value);
  if(managed && !result.managed_matches) result.managed_error=E_NOINTERFACE;
  result.status = (r.operation != 1 ? !result.frames.empty() : result.object_chain_complete) &&
                           result.stack_error == S_OK && !result.exhausted && !target.expired()
                           && (r.operation!=4 || (result.managed_bound && result.managed_values))
                      ? "observed"
                      : "partial";
  result.error = S_OK;
}

bool valid_request(const G09ClrRequest &r) {
  if (std::memcmp(r.magic, magic, 8) != 0 || (r.version != 1 && r.version != 2) ||
      (r.version==2 && r.operation!=3 && r.operation!=4) || (r.version==1 && r.operation==4) || !nonzero(r.nonce, 16) ||
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
  const bool stop_kind = (r.operation == 1 && r.exception_code == 0xe0434352) ||
                         ((r.operation == 3 || r.operation == 4) && r.exception_code == 0x80000004 && r.exception_hresult == 0);
  return stop_kind && r.event_sequence && r.process_id && r.thread_id && r.process_birth && r.thread_birth &&
         r.process_handle && r.thread_handle && r.process_handle != r.thread_handle &&
         r.process_handle < (std::uint64_t(1) << 63) && r.thread_handle < (std::uint64_t(1) << 63) &&
         r.clr_base && r.clr_image_size >= sizeof(IMAGE_NT_HEADERS64) &&
         r.clr_base <= std::numeric_limits<std::uint64_t>::max() - r.clr_image_size &&
         (r.read_budget_bytes == G09_CLR_FIXTURE_READ_BYTES ||
          r.read_budget_bytes == G09_CLR_POWERSHELL_READ_BYTES) &&
         r.read_budget_calls && r.read_budget_calls <= G09_CLR_MAX_READ_CALLS && r.dac_file_size &&
         r.dac_file_size <= 64 * 1024 * 1024 && nonzero(r.dac_sha256, 32) && r.dac_path_units &&
         r.dac_path_units <= G09_CLR_MAX_PATH_UNITS && r.first_chance == 1;
}
bool valid_managed_request(const G09ClrRequest &r,const G09ManagedRequest &m) {
  if(r.version!=2 || !m.approved_count || m.approved_count>4 ||
      (m.method_token & 0xff000000)!=0x06000000 || !(m.method_token & 0x00ffffff) ||
      m.bool_local_index>=256 || (m.start_info_local_index!=UINT32_MAX &&
          (m.start_info_local_index>=256 || m.start_info_local_index==m.bool_local_index))) return false;
  bool nonzero_mvid=false;
  for(unsigned i=0;i<36;++i) {
    const char c=m.module_mvid[i];
    if(i==8 || i==13 || i==18 || i==23) { if(c!='-') return false; }
    else { if(!((c>='0' && c<='9') || (c>='a' && c<='f'))) return false; if(c!='0') nonzero_mvid=true; }
  }
  if(!nonzero_mvid) return false;
  bool selected=false;
  for(unsigned i=0;i<4;++i) {
    if(i>=m.approved_count) { if(m.approved_il[i]) return false; continue; }
    if(m.approved_il[i]>=65536) return false;
    for(unsigned j=0;j<i;++j) if(m.approved_il[j]==m.approved_il[i]) return false;
    if(m.approved_il[i]==m.il_offset) selected=true;
  }
  if(r.operation==3) {
    const auto *bytes=reinterpret_cast<const std::uint8_t *>(&m);
    return !nonzero(bytes+68,sizeof(m)-68);
  }
  return r.operation==4 && m.pre_sequence && m.pre_sequence<r.event_sequence && nonzero(m.pre_nonce,16) &&
      std::memcmp(m.pre_nonce,r.nonce,16)!=0 && selected && m.frame_rsp>=0x10000 && m.frame_rsp<=0x00007fffffffffffULL && !(m.frame_rsp%8) &&
      m.extent_start>=0x10000 && m.extent_end>m.extent_start && m.extent_end-m.extent_start<=65536 &&
      m.extent_end<=0x0000800000000000ULL && m.address>=m.extent_start && m.address<m.extent_end &&
      m.map_count && m.map_count<=256 && nonzero(m.map_sha256,32) && nonzero(m.code_sha256,32);
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
       << ",\"exception_source\":\"" << result.exception_source << "\",\"tracker_complete\":false"
       << ",\"object_chain_complete\":" << (result.object_chain_complete ? "true" : "false")
       << ",\"exception_state_flags\":" << result.state_flags << ",\"read_bytes\":" << result.read_bytes
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
  text << "]";
  if(r.version==2 && r.operation==3) {
    const bool bound=result.managed_error==S_OK && result.managed_matches==1 && !result.private_target.empty() && !result.exhausted;
    text << ",\"managed_mapping\":{\"status\":\"" << (bound ? "bound" : "unknown") << "\",\"api_hresult\":"
         << static_cast<std::uint32_t>(result.managed_error) << ",\"matched_frames\":" << result.managed_matches
         << ",\"scan_frame_count\":" << result.scanned_frames << ",\"stack_api_hresult\":" << static_cast<std::uint32_t>(result.stack_error)
         << ",\"scan_complete\":" << (result.stack_error==S_OK ? "true" : "false") << ",\"mapping\":"
         << (bound ? result.managed_mapping : "null") << ",\"map_notes\":" << result.managed_map_notes << "}";
    if(bound) text << ",\"private_target\":" << result.private_target;
  }
  if(r.version==2 && r.operation==4) {
    text << ",\"managed_continuation\":{\"bound\":" << (result.managed_bound ? "true" : "false")
         << ",\"api_hresult\":" << static_cast<std::uint32_t>(result.managed_error) << ",\"mapping\":"
         << (result.managed_bound ? result.managed_mapping : "null") << ',';
    if(result.managed_bound) text << result.managed_continuation;
    else text << "\"boolean_local\":null,\"start_info_field\":null";
    text << ",\"map_notes\":" << result.managed_map_notes << "}";
  }
  text << "}\n";
  return text.str();
}
} // namespace

int main(int argc, char **) {
  G09ClrRequest request{};
  G09ManagedRequest managed{};
  Result result;
  int exit_code = 2;
  try {
    const HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
    if (argc == 1 && read_exact(input, &request, sizeof(request)) && valid_request(request) &&
        (request.version==1 || (read_exact(input,&managed,sizeof(managed)) && valid_managed_request(request,managed)))) {
      std::wstring path(request.dac_path_units, L'\0');
      if ((path.empty() || read_exact(input, path.data(), request.dac_path_units * 2)) &&
          path.find(L'\0') == std::wstring::npos) {
        BYTE extra = 0;
        DWORD got = 0;
        const BOOL ok = ReadFile(input, &extra, 1, &got, nullptr);
        const DWORD error = ok ? ERROR_SUCCESS : GetLastError();
        if (got == 0 && (ok || error == ERROR_BROKEN_PIPE) && valid_request(request)) {
          if (request.operation == 2) Sleep(INFINITE);
          observe(request, path, request.version==2 ? &managed : nullptr, result);
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
