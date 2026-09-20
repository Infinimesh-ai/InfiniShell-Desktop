"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const tty = require("node:tty");
const { execFileSync } = require("node:child_process");

const PLUGIN_VERSION = "0.1.2";
const MAX_INPUT_BYTES = 1024 * 1024;
const MAX_TEXT_CHARS = 16000;

function eventName(value) {
  if (typeof value !== "string") return undefined;
  return value.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase();
}

function text(value) {
  return typeof value === "string" ? value.slice(0, MAX_TEXT_CHARS) : undefined;
}

function alias(payload, camel, snake) {
  const first = payload[camel];
  const second = payload[snake];
  if (first !== undefined && second !== undefined && first !== second) {
    throw new Error("conflicting_alias");
  }
  return first === undefined ? second : first;
}

function normalize(payload, environment) {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return null;
  if (!/^\d+$/.test(environment.WARP_CLI_AGENT_PROTOCOL_VERSION || "")
      || Number(environment.WARP_CLI_AGENT_PROTOCOL_VERSION) < 1) return null;

  const event = eventName(payload.hookEventName ?? payload.hook_event_name);
  const alternate = eventName(payload.hook_event_name);
  const sessionId = alias(payload, "sessionId", "session_id");
  // Claude 兼容目录里的脚本也可能被 Grok 加载，目录名不能证明运行身份。
  if (!event || (alternate && alternate !== event)
      || eventName(environment.GROK_HOOK_EVENT) !== event
      || typeof sessionId !== "string" || sessionId.length === 0
      || environment.GROK_SESSION_ID !== sessionId) return null;

  return {
    event,
    sessionId,
    promptId: text(alias(payload, "promptId", "prompt_id")),
    timestamp: text(payload.timestamp),
    cwd: text(payload.cwd),
    project: text(payload.workspaceRoot),
    query: text(payload.prompt),
    response: text(alias(payload, "lastAssistantMessage", "last_assistant_message")),
    transcript_path: text(alias(payload, "transcriptPath", "transcript_path")),
    tool_name: text(alias(payload, "toolName", "tool_name")),
    toolId: text(alias(payload, "toolUseId", "tool_use_id")),
    summary: text(payload.message ?? payload.reason ?? payload.errorDetails),
    reason: text(payload.reason ?? payload.stopReason),
    error_type: text(payload.error),
    stopHookActive: alias(payload, "stopHookActive", "stop_hook_active"),
  };
}

function makeNotification(input, state) {
  const stamp = Date.parse(input.timestamp);
  const validStamp = Number.isFinite(stamp);
  const id = crypto.createHash("sha256").update(JSON.stringify(input)).digest("hex");
  if (state.seen?.includes(id)) return null;
  if (validStamp && state.promptTimestamp && stamp < state.promptTimestamp) return null;

  if (input.event === "session_start") {
    if (state.latestTimestamp && (!validStamp || stamp < state.latestTimestamp)) return null;
    state.failed = false;
    state.closed = false;
    state.promptId = undefined;
  } else if (input.promptId && input.promptId !== state.promptId) {
    if (state.latestTimestamp && (!validStamp || stamp < state.latestTimestamp)) return null;
    state.promptId = input.promptId;
    state.promptTimestamp = validStamp ? stamp : undefined;
    state.failed = false;
    state.closed = false;
  }

  state.seen = [...(state.seen || []).slice(-255), id];
  if (validStamp) state.latestTimestamp = Math.max(state.latestTimestamp || 0, stamp);

  let event;
  switch (input.event) {
    case "session_start": event = "session_start"; break;
    case "user_prompt_submit": event = "prompt_submit"; break;
    case "post_tool_use":
    case "post_tool_use_failure": event = "tool_complete"; break;
    case "stop_failure":
      state.failed = true;
      event = "stop_failure";
      break;
    case "session_end":
      state.closed = true;
      return null;
    case "stop":
      // 真实错误退出仍会触发 shutdown Stop；阻塞型 Stop 也可能继续下一轮。
      if (state.failed || state.closed || input.reason === "shutdown") return null;
      event = input.reason === "end_turn" && input.response && !input.stopHookActive
        ? "stop" : "notification";
      break;
    case "permission_denied":
    case "notification": event = "notification"; break;
    default: return null;
  }
  if (state.closed && event !== "session_start") return null;

  return {
    v: 1,
    agent: "grok",
    event,
    session_id: input.sessionId,
    cwd: input.cwd,
    project: input.project,
    query: input.query,
    response: input.response,
    transcript_path: input.transcript_path,
    summary: input.summary,
    tool_name: input.tool_name,
    plugin_version: PLUGIN_VERSION,
    error_type: input.event === "permission_denied" ? "permission_denied" : input.error_type,
    event_id: `grok:${id}`,
  };
}

function encode(notification, environment) {
  // JSON 转义 C0；另外转义 C1，避免模型文本变成终端控制序列。
  const body = JSON.stringify(notification).replace(/[\u007f-\u009f]/g,
    character => `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`);
  const sequence = `\x1b]777;notify;warp://cli-agent;${body}\x07`;
  return environment.TMUX
    ? `\x1bPtmux;${sequence.replace(/\x1b/g, "\x1b\x1b")}\x1b\\`
    : sequence;
}

function withSessionState(input, dataDirectory, callback) {
  if (!path.isAbsolute(dataDirectory)) return;
  const directory = path.join(dataDirectory, "infinishell-status-v1");
  fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
  const key = crypto.createHash("sha256").update(input.sessionId).digest("hex");
  const statePath = path.join(directory, `${key}.json`);
  const lockPath = `${statePath}.lock`;
  let lock;
  const deadline = Date.now() + 500;
  while (lock === undefined && Date.now() < deadline) {
    try {
      lock = fs.openSync(lockPath, "wx", 0o600);
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      // 已退出进程留下的旧锁可恢复，不能抢占仍在处理的回调。
      try {
        if (Date.now() - fs.statSync(lockPath).mtimeMs > 300000) fs.unlinkSync(lockPath);
      } catch (inspectionError) {
        if (inspectionError.code !== "ENOENT") throw inspectionError;
      }
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5);
    }
  }
  if (lock === undefined) return;
  const temporaryPath = `${statePath}.${process.pid}.tmp`;
  try {
    let state = {};
    try {
      state = JSON.parse(fs.readFileSync(statePath, "utf8"));
      if (!state || typeof state !== "object" || Array.isArray(state)) return;
    } catch (error) {
      // 已损坏状态无法证明此前是否失败，等待新会话开始而非推断成功。
      if (error.code !== "ENOENT") {
        if (input.event !== "session_start") return;
      }
    }
    const notification = makeNotification(input, state);
    // 只有写入成功才记录去重状态；暂时不可写的终端不能永久吞掉重投事件。
    if (notification) callback(notification);
    fs.writeFileSync(temporaryPath, JSON.stringify(state), { mode: 0o600 });
    fs.renameSync(temporaryPath, statePath);
  } finally {
    try { fs.unlinkSync(temporaryPath); } catch (_) { /* 临时文件通常已原子替换。 */ }
    fs.closeSync(lock);
    fs.unlinkSync(lockPath);
  }
}

function openUnixTerminal(location) {
  if (typeof location !== "string" || !path.isAbsolute(location)
      || !location.startsWith("/dev/") || !fs.lstatSync(location).isCharacterDevice()) {
    throw new Error("invalid_terminal");
  }
  // 不创建、不截断、不跟随符号链接，也不让脱离会话的 hook 取得控制终端。
  const descriptor = fs.openSync(location, fs.constants.O_WRONLY | fs.constants.O_NOCTTY
    | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK);
  if (fs.fstatSync(descriptor).isCharacterDevice() && tty.isatty(descriptor)) return descriptor;
  fs.closeSync(descriptor);
  throw new Error("invalid_terminal");
}

function openTerminal(environment = process.env) {
  // Windows 控制台与 Unix PTY 是不同通道，不把 Unix 路径回退套用于 CONOUT$。
  if (process.platform === "win32") return fs.openSync("CONOUT$", "w");
  try {
    return openUnixTerminal("/dev/tty");
  } catch (error) {
    if (!["ENXIO", "ENODEV", "ENOENT"].includes(error.code)) throw error;
  }
  if (environment.TMUX) {
    // tmux 中的继承环境可能指向外层终端，只允许当前 pane 的真实 PTY。
    if (!/^%\d+$/.test(environment.TMUX_PANE || "")) throw new Error("invalid_tmux_pane");
    const location = execFileSync("tmux", ["display-message", "-p", "-t",
      environment.TMUX_PANE, "#{pane_tty}"], {
      encoding: "utf8", timeout: 500, maxBuffer: 4096,
      stdio: ["ignore", "pipe", "ignore"], env: environment,
    }).trim();
    return openUnixTerminal(location);
  }
  // bootstrap 会在 SSH 及新 shell 内刷新专用值；SSH_TTY 仅供未引导的远端 shell。
  return openUnixTerminal(environment.WARP_CLI_AGENT_TTY || environment.SSH_TTY);
}

function writeTerminal(descriptor, notification, environment) {
  const bytes = Buffer.from(encode(notification, environment));
  const deadline = Date.now() + 500;
  let offset = 0;
  while (offset < bytes.length) {
    if (Date.now() >= deadline) throw new Error("terminal_write_timeout");
    try {
      offset += fs.writeSync(descriptor, bytes, offset, bytes.length - offset);
    } catch (error) {
      if (!["EAGAIN", "EWOULDBLOCK"].includes(error.code)) throw error;
      // 非阻塞打开防止失效路径挂住；短写及暂时背压不能截断 OSC 帧。
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5);
    }
  }
}

function main() {
  let chunks = [];
  let size = 0;
  process.stdin.on("data", chunk => {
    size += chunk.length;
    if (size <= MAX_INPUT_BYTES) chunks.push(chunk);
    else chunks = [];
  });
  process.stdin.on("end", () => {
    try {
      if (size > MAX_INPUT_BYTES) return;
      const input = normalize(JSON.parse(Buffer.concat(chunks).toString("utf8")), process.env);
      if (!input || !process.env.GROK_PLUGIN_DATA) return;
      // hook stdout 属于 Grok 决策协议，只向所在终端写状态，不返回任何权限决定。
      const descriptor = openTerminal();
      try {
        withSessionState(input, process.env.GROK_PLUGIN_DATA,
          notification => writeTerminal(descriptor, notification, process.env));
      } finally {
        fs.closeSync(descriptor);
      }
    } catch (_) {
      // 状态增强失败不能批准、拒绝或阻塞 CLI 工具，也不能泄露原始提示词。
    }
  });
}

module.exports = { normalize, makeNotification, encode, withSessionState, openTerminal, main, PLUGIN_VERSION };
if (require.main === module) main();
