#pragma once
#include <cstddef>
#include <cstdint>

// 仅由本轮控制器经私有 stdin 发送；所有整数为小端，尾部为无 NUL 的 UTF-16LE DAC
// 路径。 目标句柄必须由控制器从原 CREATE 事件句柄 DuplicateHandle 到
// reader，不能是 PID 重开。
#pragma pack(push, 1)
struct G09ClrRequest {
  std::uint8_t magic[8];           // 0: G09CLR1\0
  std::uint32_t version;           // 8: 1
  std::uint32_t operation;         // 12: 1 CLR异常；2 无目标清理夹具；3 自有原生返回栈
  std::uint8_t nonce[16];          // 16: 本轮非零随机值
  std::uint64_t event_sequence;    // 32: 控制器精确 pending event 序号
  std::uint32_t process_id;        // 40
  std::uint32_t thread_id;         // 44
  std::uint64_t process_birth;     // 48: 原 GetProcessTimes 创建 FILETIME
  std::uint64_t thread_birth;      // 56: 原 GetThreadTimes 创建 FILETIME
  std::uint64_t process_handle;    // 64: reader 内的只读句柄数值
  std::uint64_t thread_handle;     // 72: reader 内的只读句柄数值
  std::uint64_t clr_base;          // 80: 已授权 CLR LOAD_DLL 基址
  std::uint32_t clr_image_size;    // 88: PE SizeOfImage
  std::uint32_t clr_timestamp;     // 92: PE TimeDateStamp
  std::uint64_t deadline_tick_ms;  // 96: 同机 GetTickCount64 绝对期限
  std::uint32_t read_budget_bytes; // 104: 夹具 16 MiB 或 PS 24 MiB，失败读也扣预算
  std::uint32_t read_budget_calls; // 108: 非零且 <= 8192
  std::uint64_t dac_file_size;     // 112: 非零且 <= 64 MiB
  std::uint8_t dac_sha256[32];     // 120: 原生 CLR 配套 DAC 的原文件摘要
  std::uint32_t dac_path_units;    // 152: 非零且 <= 1024
  std::uint32_t exception_hresult; // 156: op1原事件HRESULT，仅作对照；op3必须为0
  std::uint32_t first_chance;      // 160: 必须为1
  std::uint32_t exception_code;    // 164: op1为0xe0434352；op3为0x80000004
};
#pragma pack(pop)
static_assert(sizeof(G09ClrRequest) == 168);
static_assert(offsetof(G09ClrRequest, process_handle) == 64);
static_assert(offsetof(G09ClrRequest, dac_path_units) == 152);

// v2 仅用于显式方法规格的 op3 与独立 op4；位于原头之后、DAC 路径之前。
// MVID 为固定 36 字节小写 UUID 文本；地址仅通过私有管道，公开收据不包含地址。
#pragma pack(push, 1)
struct G09ManagedRequest {
  char module_mvid[36];
  std::uint32_t method_token;
  std::uint32_t approved_count;
  std::uint32_t approved_il[4];
  std::uint32_t bool_local_index;
  std::uint32_t start_info_local_index; // UINT32_MAX 表示不请求
  std::uint64_t pre_sequence;
  std::uint8_t pre_nonce[16];
  std::uint32_t il_offset;
  std::uint32_t enc_version;
  std::uint64_t frame_rsp;
  std::uint64_t extent_start;
  std::uint64_t extent_end;
  std::uint64_t address;
  std::uint32_t map_count;
  std::uint8_t map_sha256[32];
  std::uint8_t code_sha256[32];
};
#pragma pack(pop)
static_assert(sizeof(G09ManagedRequest) == 200);
static_assert(offsetof(G09ManagedRequest, pre_sequence) == 68);
static_assert(offsetof(G09ManagedRequest, map_sha256) == 136);

constexpr std::uint32_t G09_CLR_FIXTURE_READ_BYTES = 16 * 1024 * 1024;
constexpr std::uint32_t G09_CLR_POWERSHELL_READ_BYTES = 24 * 1024 * 1024;
constexpr std::uint32_t G09_CLR_MAX_READ_CALLS = 8192;
constexpr std::uint32_t G09_CLR_MAX_PATH_UNITS = 1024;
constexpr std::uint32_t G09_CLR_MAX_OUTPUT_BYTES = 32768;
constexpr std::uint32_t G09_CLR_MAX_FRAMES = 32;
constexpr std::uint32_t G09_CLR_MAX_CHAIN = 4;
