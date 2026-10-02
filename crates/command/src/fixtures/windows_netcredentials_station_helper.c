#define WIN32_LEAN_AND_MEAN
#include <windows.h>

/* 仅测试：无 CRT、网络调用、真实凭据、ACL 写入或显式站切换。 */
#define MAGIC 0x4e435331u
#define SD_LIMIT 16384u
#define WAIT_LIMIT 10000u

typedef struct {
    DWORD magic, version, kind;
    BYTE nonce[32];
} HEADER;

typedef struct {
    DWORD pid, created_low, created_high, auth_low, auth_high, session;
    DWORD token_type, elevated, integrity, sid_length;
    BYTE sid[SECURITY_MAX_SID_SIZE];
} PROCESS_SNAPSHOT;

typedef struct {
    DWORD name_bytes;
    WCHAR name[256];
    DWORD inherit, flags, sd_bytes;
    BYTE sd[SD_LIMIT];
} OBJECT_SNAPSHOT;

typedef struct {
    HEADER header;
    PROCESS_SNAPSHOT process;
    DWORD metric;
    OBJECT_SNAPSHOT station, desktop;
} OBSERVATION;

typedef struct {
    HEADER header;
    PROCESS_SNAPSHOT process;
} SUSPENDED;

typedef struct {
    HEADER header;
    DWORD stage, win32_error, hresult_from_win32;
    DWORD child_wait, child_exit, process_closed, thread_closed;
    DWORD cleanup_stage, cleanup_error;
} COMPLETION;

typedef char observation_size[(sizeof(OBSERVATION) == 33980) ? 1 : -1];
typedef char suspended_size[(sizeof(SUSPENDED) == 152) ? 1 : -1];
typedef char completion_size[(sizeof(COMPLETION) == 80) ? 1 : -1];

static WCHAR root[1024], path[1200], image[1024], command[1030];
static WCHAR stage_text[8], nonce_text[40];
static OBSERVATION observation;
static SUSPENDED suspended, authorization;
static COMPLETION completion;
static ULONG_PTR token_buffer[128];
static BYTE io_buffer[sizeof(SUSPENDED)];

/* volatile 阻止编译器将无 CRT 的清零循环替换为 memset。 */
static void clear_bytes(void *value, DWORD length) {
    volatile BYTE *bytes = (volatile BYTE *)value;
    for (DWORD i = 0; i < length; ++i) bytes[i] = 0;
}

static void copy_bytes(void *destination, const void *source, DWORD length) {
    volatile BYTE *output = (volatile BYTE *)destination;
    const volatile BYTE *input = (const volatile BYTE *)source;
    for (DWORD i = 0; i < length; ++i) output[i] = input[i];
}

static BOOL equal_bytes(const void *left, const void *right, DWORD length) {
    const BYTE *a = (const BYTE *)left, *b = (const BYTE *)right;
    for (DWORD i = 0; i < length; ++i) if (a[i] != b[i]) return FALSE;
    return TRUE;
}

static void header(HEADER *value, DWORD kind) {
    value->magic = MAGIC;
    value->version = 1;
    value->kind = kind;
    for (DWORD i = 0; i < 32; ++i) value->nonce[i] = (BYTE)nonce_text[i];
}

static BOOL make_path(const WCHAR *name) {
    DWORD at = 0;
    while (root[at] != 0 && at < 1024) { path[at] = root[at]; ++at; }
    if (at == 1024) { SetLastError(ERROR_INVALID_DATA); return FALSE; }
    path[at++] = L'\\';
    for (DWORD i = 0; name[i] != 0; ++i) {
        if (at >= 1199) { SetLastError(ERROR_BUFFER_OVERFLOW); return FALSE; }
        path[at++] = name[i];
    }
    path[at] = 0;
    return TRUE;
}

static BOOL save(const WCHAR *name, const void *value, DWORD length) {
    if (!make_path(name)) return FALSE;
    HANDLE file = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_NEW,
                              FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT, NULL);
    if (file == INVALID_HANDLE_VALUE) return FALSE;
    DWORD written = 0;
    BOOL ok = WriteFile(file, value, length, &written, NULL);
    DWORD error = ok ? ERROR_SUCCESS : GetLastError();
    if (ok && written != length) { ok = FALSE; error = ERROR_WRITE_FAULT; }
    if (ok && !FlushFileBuffers(file)) { ok = FALSE; error = GetLastError(); }
    if (!CloseHandle(file) && ok) { ok = FALSE; error = GetLastError(); }
    SetLastError(error);
    return ok;
}

static BOOL token_field(HANDLE token, TOKEN_INFORMATION_CLASS kind, void *value,
                       DWORD capacity, DWORD expected) {
    DWORD used = 0;
    if (!GetTokenInformation(token, kind, value, capacity, &used)) return FALSE;
    if (expected != 0 && used != expected) {
        SetLastError(ERROR_INVALID_DATA); return FALSE;
    }
    return TRUE;
}

static BOOL snapshot(HANDLE process, PROCESS_SNAPSHOT *value) {
    FILETIME created, exited, kernel, user;
    TOKEN_STATISTICS statistics;
    TOKEN_ELEVATION elevation;
    HANDLE token = NULL;
    value->pid = GetProcessId(process);
    if (value->pid == 0 || !GetProcessTimes(process, &created, &exited, &kernel, &user)) return FALSE;
    value->created_low = created.dwLowDateTime;
    value->created_high = created.dwHighDateTime;
    if (!OpenProcessToken(process, TOKEN_QUERY, &token)) return FALSE;
    BOOL ok = token_field(token, TokenStatistics, &statistics, sizeof(statistics), sizeof(statistics));
    if (ok) {
        value->auth_low = statistics.AuthenticationId.LowPart;
        value->auth_high = (DWORD)statistics.AuthenticationId.HighPart;
        value->token_type = (DWORD)statistics.TokenType;
        ok = token_field(token, TokenSessionId, &value->session, sizeof(DWORD), sizeof(DWORD));
    }
    if (ok) ok = token_field(token, TokenElevation, &elevation, sizeof(elevation), sizeof(elevation));
    if (ok) {
        value->elevated = elevation.TokenIsElevated;
        ok = token_field(token, TokenUser, token_buffer, sizeof(token_buffer), 0);
    }
    if (ok) {
        PSID sid = ((TOKEN_USER *)token_buffer)->User.Sid;
        if (!IsValidSid(sid) || GetLengthSid(sid) > SECURITY_MAX_SID_SIZE) {
            SetLastError(ERROR_INVALID_SID); ok = FALSE;
        } else {
            value->sid_length = GetLengthSid(sid);
            ok = CopySid(SECURITY_MAX_SID_SIZE, value->sid, sid);
        }
    }
    if (ok) ok = token_field(token, TokenIntegrityLevel, token_buffer, sizeof(token_buffer), 0);
    if (ok) {
        PSID sid = ((TOKEN_MANDATORY_LABEL *)token_buffer)->Label.Sid;
        if (!IsValidSid(sid) || *GetSidSubAuthorityCount(sid) == 0) {
            SetLastError(ERROR_INVALID_SID); ok = FALSE;
        } else value->integrity = *GetSidSubAuthority(sid, *GetSidSubAuthorityCount(sid) - 1);
    }
    DWORD error = ok ? ERROR_SUCCESS : GetLastError();
    if (!CloseHandle(token) && ok) { ok = FALSE; error = GetLastError(); }
    SetLastError(error);
    return ok;
}

static BOOL object_snapshot(HANDLE object, OBJECT_SNAPSHOT *value, DWORD stage) {
    USEROBJECTFLAGS flags;
    DWORD used = 0;
    completion.stage = stage;
    if (!GetUserObjectInformationW(object, UOI_NAME, value->name, sizeof(value->name), &used)) return FALSE;
    if (used < 4 || used > sizeof(value->name) || used % 2 != 0 || value->name[used / 2 - 1] != 0) {
        SetLastError(ERROR_INVALID_DATA); return FALSE;
    }
    value->name_bytes = used;
    completion.stage = stage + 1;
    if (!GetUserObjectInformationW(object, UOI_FLAGS, &flags, sizeof(flags), &used)) return FALSE;
    if (used != sizeof(flags)) { SetLastError(ERROR_INVALID_DATA); return FALSE; }
    value->inherit = (DWORD)flags.fInherit;
    value->flags = flags.dwFlags;
    completion.stage = stage + 2;
    SECURITY_INFORMATION requested = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    if (!GetUserObjectSecurity(object, &requested, value->sd, sizeof(value->sd), &used)) return FALSE;
    if (used == 0 || used > SD_LIMIT || !IsValidSecurityDescriptor(value->sd)) {
        SetLastError(ERROR_INVALID_SECURITY_DESCR); return FALSE;
    }
    value->sd_bytes = used;
    return TRUE;
}

static BOOL observe(DWORD kind) {
    header(&observation.header, kind);
    completion.stage = 30;
    if (!snapshot(GetCurrentProcess(), &observation.process)) return FALSE;
    HANDLE thread_token = NULL;
    completion.stage = 31;
    if (OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &thread_token)) {
        CloseHandle(thread_token); SetLastError(ERROR_BAD_IMPERSONATION_LEVEL); return FALSE;
    }
    if (GetLastError() != ERROR_NO_TOKEN) return FALSE;
    completion.stage = 32;
    /* 首次固定非站/桌面 USER32 调用；不创建窗口或切换调用方对象。 */
    observation.metric = (DWORD)GetSystemMetrics(SM_CXSCREEN);
    HWINSTA station = GetProcessWindowStation();
    if (station == NULL) return FALSE;
    HDESK desktop = GetThreadDesktop(GetCurrentThreadId());
    if (desktop == NULL) return FALSE;
    return object_snapshot((HANDLE)station, &observation.station, 40) &&
           object_snapshot((HANDLE)desktop, &observation.desktop, 50);
}

static BOOL wait_authorization(void) {
    ULONGLONG deadline = GetTickCount64() + WAIT_LIMIT;
    if (!make_path(L"resume2.bin")) return FALSE;
    while (GetTickCount64() < deadline) {
        HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING,
                                  FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT, NULL);
        if (file != INVALID_HANDLE_VALUE) {
            DWORD used = 0;
            LARGE_INTEGER size;
            BOOL ok = GetFileSizeEx(file, &size);
            if (ok && (size.QuadPart < 0 || (ULONGLONG)size.QuadPart != (ULONGLONG)sizeof(authorization))) {
                ok = FALSE; SetLastError(ERROR_INVALID_DATA);
            }
            if (ok) ok = ReadFile(file, io_buffer, sizeof(io_buffer), &used, NULL);
            if (ok && used != sizeof(io_buffer)) { ok = FALSE; SetLastError(ERROR_INVALID_DATA); }
            DWORD error = ok ? ERROR_SUCCESS : GetLastError();
            if (!CloseHandle(file) && ok) { ok = FALSE; error = GetLastError(); }
            header(&authorization.header, 4);
            copy_bytes(&authorization.process, &suspended.process, sizeof(suspended.process));
            if (!ok || !equal_bytes(io_buffer, &authorization, sizeof(authorization))) {
                SetLastError(error == ERROR_SUCCESS ? ERROR_INVALID_DATA : error); return FALSE;
            }
            return TRUE;
        }
        DWORD error = GetLastError();
        if (error != ERROR_FILE_NOT_FOUND && error != ERROR_SHARING_VIOLATION) return FALSE;
        Sleep(10);
    }
    SetLastError(ERROR_TIMEOUT); return FALSE;
}

static BOOL stage_one(PROCESS_INFORMATION *child) {
    completion.stage = 10;
    if (!observe(1) || !save(L"stage1.bin", &observation, sizeof(observation))) return FALSE;
    DWORD length = GetModuleFileNameW(NULL, image, 1024);
    if (length == 0) return FALSE;
    if (length >= 1024) { SetLastError(ERROR_BUFFER_OVERFLOW); return FALSE; }
    command[0] = L'"';
    for (DWORD i = 0; i < length; ++i) command[i + 1] = image[i];
    command[length + 1] = L'"'; command[length + 2] = 0;
    if (!SetEnvironmentVariableW(L"INFINISHELL_WINDOWS_NETCREDENTIALS_STAGE", L"2")) return FALSE;
    STARTUPINFOW startup;
    clear_bytes(&startup, sizeof(startup));
    startup.cb = sizeof(startup);
    WCHAR desktop[1] = {0};
    startup.lpDesktop = desktop;
    completion.stage = 11;
    if (!CreateProcessW(image, command, NULL, NULL, FALSE,
                        CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                        NULL, root, &startup, child)) return FALSE;
    completion.stage = 12;
    header(&suspended.header, 3);
    if (!snapshot(child->hProcess, &suspended.process)) return FALSE;
    PROCESS_SNAPSHOT parent, candidate;
    copy_bytes(&parent, &observation.process, sizeof(parent));
    copy_bytes(&candidate, &suspended.process, sizeof(candidate));
    /* PID/创建时间不同，令牌的本地身份和新登录身份必须相同。 */
    parent.pid = candidate.pid; parent.created_low = candidate.created_low; parent.created_high = candidate.created_high;
    if (!equal_bytes(&parent, &candidate, sizeof(parent))) { SetLastError(ERROR_INVALID_OWNER); return FALSE; }
    WCHAR actual_image[1024];
    DWORD actual_length = 1024;
    if (!QueryFullProcessImageNameW(child->hProcess, 0, actual_image, &actual_length)) return FALSE;
    /* 两个 Win32 API 可返回不同的本地卷前缀；只消除标准 verbatim 前缀，不重新解析路径。 */
    const WCHAR *expected_name = image, *actual_name = actual_image;
    DWORD expected_length = length;
    if (expected_length >= 4 && expected_name[0] == L'\\' && expected_name[1] == L'\\' && expected_name[2] == L'?' && expected_name[3] == L'\\') {
        expected_name += 4; expected_length -= 4;
    }
    if (actual_length >= 4 && actual_name[0] == L'\\' && actual_name[1] == L'\\' && actual_name[2] == L'?' && actual_name[3] == L'\\') {
        actual_name += 4; actual_length -= 4;
    }
    if (CompareStringOrdinal(expected_name, (int)expected_length, actual_name, (int)actual_length, TRUE) != CSTR_EQUAL) {
        SetLastError(ERROR_INVALID_NAME); return FALSE;
    }
    if (!save(L"suspended2.bin", &suspended, sizeof(suspended))) return FALSE;
    completion.stage = 13;
    if (!wait_authorization()) return FALSE;
    PROCESS_SNAPSHOT before_resume;
    clear_bytes(&before_resume, sizeof(before_resume));
    if (!snapshot(child->hProcess, &before_resume)) return FALSE;
    if (!equal_bytes(&before_resume, &suspended.process, sizeof(before_resume))) {
        SetLastError(ERROR_INVALID_OWNER); return FALSE;
    }
    DWORD resumed = ResumeThread(child->hThread);
    if (resumed != 1) {
        if (resumed != (DWORD)-1) SetLastError(ERROR_INVALID_STATE);
        return FALSE;
    }
    completion.stage = 14;
    completion.child_wait = WaitForSingleObject(child->hProcess, WAIT_LIMIT);
    if (completion.child_wait != WAIT_OBJECT_0) {
        if (completion.child_wait != WAIT_FAILED) SetLastError(ERROR_TIMEOUT);
        return FALSE;
    }
    return GetExitCodeProcess(child->hProcess, &completion.child_exit);
}

void WINAPI ProbeMain(void) {
    DWORD length = GetEnvironmentVariableW(L"INFINISHELL_WINDOWS_NETCREDENTIALS_RUN_DIR", root, 1024);
    DWORD nonce_length = GetEnvironmentVariableW(L"INFINISHELL_WINDOWS_NETCREDENTIALS_NONCE", nonce_text, 40);
    DWORD stage_length = GetEnvironmentVariableW(L"INFINISHELL_WINDOWS_NETCREDENTIALS_STAGE", stage_text, 8);
    if (length == 0 || length >= 1024 || nonce_length != 32 || stage_length != 1 ||
        (stage_text[0] != L'1' && stage_text[0] != L'2')) ExitProcess(2);
    for (DWORD i = 0; i < 32; ++i) {
        if (!((nonce_text[i] >= L'0' && nonce_text[i] <= L'9') ||
              (nonce_text[i] >= L'a' && nonce_text[i] <= L'f'))) ExitProcess(2);
    }
    if (stage_text[0] == L'2') {
        BOOL ok = observe(2);
        DWORD error = ok ? ERROR_SUCCESS : GetLastError();
        /* 即使观察不完整也保留原始字段，由控制器验证长度与阶段。 */
        BOOL saved = save(L"stage2.bin", &observation, sizeof(observation));
        if (!saved && ok) { ok = FALSE; error = GetLastError(); }
        header(&completion.header, 6);
        completion.win32_error = error;
        completion.hresult_from_win32 = (DWORD)HRESULT_FROM_WIN32(error);
        BOOL final_saved = save(L"stage2-complete.bin", &completion, sizeof(completion));
        ExitProcess(ok && final_saved ? 0 : 3);
    }
    PROCESS_INFORMATION child;
    clear_bytes(&child, sizeof(child));
    BOOL ok = stage_one(&child);
    DWORD error = ok ? ERROR_SUCCESS : GetLastError();
    if (!ok && child.hProcess != NULL && WaitForSingleObject(child.hProcess, 0) != WAIT_OBJECT_0) {
        /* 仅用本段 CreateProcess 返回的原句柄回收，绝不按 PID 终止。 */
        if (!TerminateProcess(child.hProcess, 1)) {
            completion.cleanup_stage = 60; completion.cleanup_error = GetLastError();
        }
        completion.child_wait = WaitForSingleObject(child.hProcess, 5000);
        if (completion.child_wait != WAIT_OBJECT_0 && completion.cleanup_error == 0) {
            completion.cleanup_stage = 61;
            completion.cleanup_error = completion.child_wait == WAIT_FAILED ? GetLastError() : ERROR_TIMEOUT;
        }
    }
    if (child.hProcess != NULL) {
        if (!GetExitCodeProcess(child.hProcess, &completion.child_exit) && completion.cleanup_error == 0) {
            completion.cleanup_stage = 62; completion.cleanup_error = GetLastError();
        }
        completion.process_closed = CloseHandle(child.hProcess) != 0;
        if (!completion.process_closed && completion.cleanup_error == 0) {
            completion.cleanup_stage = 63; completion.cleanup_error = GetLastError();
        }
    }
    if (child.hThread != NULL) {
        completion.thread_closed = CloseHandle(child.hThread) != 0;
        if (!completion.thread_closed && completion.cleanup_error == 0) {
            completion.cleanup_stage = 64; completion.cleanup_error = GetLastError();
        }
    }
    header(&completion.header, 5);
    completion.win32_error = error;
    completion.hresult_from_win32 = (DWORD)HRESULT_FROM_WIN32(error);
    BOOL saved = save(L"stage1-complete.bin", &completion, sizeof(completion));
    ExitProcess(ok && saved && completion.child_exit == 0 && completion.process_closed && completion.thread_closed && completion.cleanup_error == 0 ? 0 : 4);
}
