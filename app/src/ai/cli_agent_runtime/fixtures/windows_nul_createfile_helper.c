#define WIN32_LEAN_AND_MEAN
#include <windows.h>

/* 仅在测试私有目录中运行；不使用 CRT、USER32 或外部进程。 */
typedef struct {
    DWORD magic;
    DWORD version;
    DWORD access[2];
    DWORD share_mode;
    DWORD disposition;
    DWORD results[2][3][4];
} CREATEFILE_RECEIPT;

typedef char receipt_size_must_be_120[(sizeof(CREATEFILE_RECEIPT) == 120) ? 1 : -1];

static WCHAR report_path[1024];
static WCHAR ordinary_path[1024];
static CREATEFILE_RECEIPT receipt;

static void probe(const WCHAR *path, DWORD access, DWORD result[4]) {
    HANDLE file = CreateFileW(path, access,
                              FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                              NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) {
        result[0] = 0;
        result[1] = GetLastError();
        return;
    }
    result[0] = 1;
    result[1] = 0;
    result[2] = GetFileType(file);
    result[3] = CloseHandle(file) != 0;
}

void WINAPI ProbeMain(void) {
    DWORD report_length = GetEnvironmentVariableW(
        L"INFINISHELL_WINDOWS_NUL_CREATEFILE_REPORT", report_path, 1024);
    DWORD ordinary_length = GetEnvironmentVariableW(
        L"INFINISHELL_WINDOWS_NUL_CREATEFILE_ORDINARY", ordinary_path, 1024);
    if (report_length == 0 || report_length >= 1024 ||
        ordinary_length == 0 || ordinary_length >= 1024) {
        ExitProcess(2);
    }

    receipt.magic = 0x4e554c31; /* NUL1 */
    receipt.version = 1;
    receipt.access[0] = GENERIC_WRITE;
    receipt.access[1] = FILE_GENERIC_WRITE;
    receipt.share_mode = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    receipt.disposition = OPEN_EXISTING;
    for (DWORD access_index = 0; access_index < 2; ++access_index) {
        DWORD access = receipt.access[access_index];
        probe(ordinary_path, access, receipt.results[access_index][0]);
        probe(L"NUL", access, receipt.results[access_index][1]);
        probe(L"\\\\.\\NUL", access, receipt.results[access_index][2]);
    }

    HANDLE output = CreateFileW(report_path, GENERIC_WRITE, 0, NULL,
                                CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
    if (output == INVALID_HANDLE_VALUE) {
        ExitProcess(3);
    }
    DWORD written = 0;
    BOOL saved = WriteFile(output, &receipt, (DWORD)sizeof(receipt), &written, NULL);
    BOOL closed = CloseHandle(output);
    ExitProcess(saved && written == (DWORD)sizeof(receipt) && closed ? 0 : 4);
}
