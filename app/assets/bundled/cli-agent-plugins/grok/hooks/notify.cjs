"use strict";

const crypto = require("node:crypto");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { TextDecoder } = require("node:util");

const PLUGIN_VERSION = "0.1.4";
const MAX_INPUT_BYTES = 1024 * 1024;
const MAX_FRAME_BYTES = 4096;
const TOTAL_TIMEOUT_MS = 4000;
const OPTIONAL_TEXT_FIELDS = [
  "cwd", "project", "query", "response", "transcript_path", "summary", "tool_name", "error_type",
];

function eventName(value) {
  if (typeof value !== "string") return undefined;
  return value.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase();
}

function text(value) {
  return typeof value === "string" ? value : undefined;
}

function identifier(value) {
  return typeof value === "string" && /^[A-Za-z0-9._:-]{1,256}$/.test(value);
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
  // 子代理字段只要出现就不能当成主会话，空值和异常类型也不例外。
  if (Object.hasOwn(payload, "subagentType") || Object.hasOwn(payload, "subagent_type")) return null;
  if (environment.WARP_CLI_AGENT_PROTOCOL_VERSION !== "1") return null;
  const event = eventName(payload.hookEventName ?? payload.hook_event_name);
  const alternate = eventName(payload.hook_event_name);
  const sessionId = alias(payload, "sessionId", "session_id");
  const promptId = alias(payload, "promptId", "prompt_id");
  // Claude 兼容目录也可能被 Grok 加载，必须以原生环境和载荷共同确认身份。
  if (!event || (alternate && alternate !== event)
      || eventName(environment.GROK_HOOK_EVENT) !== event
      || !identifier(sessionId) || environment.GROK_SESSION_ID !== sessionId
      || (promptId !== undefined && !identifier(promptId))) return null;
  return {
    event,
    sessionId,
    promptId,
    timestamp: text(payload.timestamp),
    notificationType: text(alias(payload, "notificationType", "notification_type")),
    cwd: text(payload.cwd),
    project: text(alias(payload, "workspaceRoot", "workspace_root")),
    query: text(payload.prompt),
    response: text(alias(payload, "lastAssistantMessage", "last_assistant_message")),
    transcript_path: text(alias(payload, "transcriptPath", "transcript_path")),
    tool_name: text(alias(payload, "toolName", "tool_name")),
    toolId: text(alias(payload, "toolUseId", "tool_use_id")),
    summary: text(payload.message),
    reason: text(payload.reason ?? payload.stopReason),
    error_type: typeof payload.error === "string" && /^[a-z][a-z0-9_]{0,63}$/.test(payload.error)
      ? payload.error : undefined,
    stopHookActive: alias(payload, "stopHookActive", "stop_hook_active"),
  };
}

function makeNotification(input) {
  if (!input) return null;
  let event;
  let terminalUnverified;
  let errorType = input.error_type;
  switch (input.event) {
    case "session_start": event = "session_start"; break;
    case "user_prompt_submit": event = "prompt_submit"; break;
    case "post_tool_use":
    case "post_tool_use_failure": event = "tool_complete"; break;
    case "stop_failure": event = "stop_failure"; break;
    case "stop_cancelled":
      if (input.promptId && ["user_interrupt", "permission_rejected", "permission_cancelled"].includes(input.reason)) {
        event = "cancelled";
        errorType = input.reason;
      } else {
        event = "notification";
        terminalUnverified = true;
        errorType = "terminal_unverified";
      }
      break;
    case "session_end":
      event = "notification";
      terminalUnverified = true;
      break;
    case "stop":
      // Stop 只是候选响应；后续 hook 仍可能继续、拒绝或失败，应用不能据此确认成功。
      if (input.reason === "end_turn" && input.response && input.stopHookActive === false) {
        event = "stop";
      } else {
        event = "notification";
        terminalUnverified = true;
      }
      break;
    case "permission_denied":
      event = "notification";
      errorType = "permission_denied";
      break;
    case "notification":
      // 固定原生类别明确表示权限 UI 正在等待；标题和正文仅用于展示，不能推断审批。
      event = input.notificationType === "permission_prompt" ? "permission_request" : "notification";
      if (input.notificationType === "idle_prompt") terminalUnverified = true;
      break;
    default: return null;
  }
  // 不维护跨 hook 状态、不按时间推进回合；应用按原生 prompt_id 处理重复和迟到事件。
  const id = crypto.createHash("sha256").update(JSON.stringify(input)).digest("hex");
  return {
    v: 1,
    agent: "grok",
    event,
    session_id: input.sessionId,
    prompt_id: input.promptId,
    event_id: `grok:${id}`,
    plugin_version: PLUGIN_VERSION,
    terminal_unverified: terminalUnverified,
    cwd: input.cwd,
    project: input.project,
    query: input.query,
    response: input.response,
    transcript_path: input.transcript_path,
    summary: input.summary,
    tool_name: input.tool_name,
    error_type: errorType,
  };
}

function frameBytes(notification) {
  // 始终按 tmux 包装后的最大帧计算；这里只预算字节，终端写入完全交给原生 worker。
  const body = JSON.stringify(notification).replace(/[;\u007f-\u009f]/g,
    character => `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`);
  const osc = `\x1b]777;notify;warp://cli-agent;${body}\x07`;
  return Buffer.byteLength(`\x1bPtmux;${osc.replace(/\x1b/g, "\x1b\x1b")}\x1b\\`, "utf8");
}

function fitNotification(notification) {
  const fitted = { ...notification };
  // 可选文本超限时整字段省略，不截断或重造会话、回合及去重身份。
  for (const field of OPTIONAL_TEXT_FIELDS) {
    if (frameBytes(fitted) <= MAX_FRAME_BYTES) break;
    delete fitted[field];
  }
  return frameBytes(fitted) <= MAX_FRAME_BYTES ? fitted : null;
}

function sendNotification(notification, environment, deadline = Date.now() + TOTAL_TIMEOUT_MS,
  execute = execFileSync, now = Date.now) {
  const executable = environment.WARP_CLI_AGENT_NOTIFY_EXECUTABLE;
  if (typeof executable !== "string" || !path.isAbsolute(executable) || executable.includes("\0")) return false;
  const fitted = fitNotification(notification);
  if (!fitted) return false;
  try {
    let remaining = deadline - now();
    if (remaining <= 0) return false;
    const capabilities = execute(executable, ["cli-agent-notify", "--protocol-version"], {
      encoding: "utf8", timeout: Math.min(500, remaining), maxBuffer: 256, killSignal: "SIGKILL",
      stdio: ["ignore", "pipe", "ignore"], windowsHide: true, env: environment,
    });
    const parsed = JSON.parse(capabilities);
    if (!parsed || Array.isArray(parsed) || parsed.protocol !== 1 || parsed.maxFrameBytes !== MAX_FRAME_BYTES
        || Object.keys(parsed).sort().join(",") !== "maxFrameBytes,protocol") return false;
    remaining = deadline - now();
    if (remaining <= 0) return false;
    const result = execute(executable, ["cli-agent-notify"], {
      input: JSON.stringify(fitted), encoding: "utf8", timeout: Math.min(2600, remaining),
      maxBuffer: 256, killSignal: "SIGKILL", stdio: ["pipe", "pipe", "ignore"],
      windowsHide: true, env: environment,
    });
    // exit 0 加空 stdout 仅确认 worker 写完，不能冒充应用接收确认。
    return result === "";
  } catch (_) {
    // 缺失、忙碌、超时或版本不兼容都不回退旧 TTY 写法，也不自动重投。
    return false;
  }
}

function main() {
  const deadline = Date.now() + TOTAL_TIMEOUT_MS;
  let chunks = [];
  let size = 0;
  let finished = false;
  const timer = setTimeout(() => {
    finished = true;
    chunks = [];
    process.stdin.destroy();
  }, 750);
  process.stdin.on("error", () => { clearTimeout(timer); });
  process.stdin.on("data", chunk => {
    size += chunk.length;
    if (size <= MAX_INPUT_BYTES) chunks.push(chunk);
    else chunks = [];
  });
  process.stdin.on("end", () => {
    clearTimeout(timer);
    if (finished || size > MAX_INPUT_BYTES) return;
    finished = true;
    try {
      const raw = new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks));
      const notification = makeNotification(normalize(JSON.parse(raw), process.env));
      // stdout 属于 Grok hook 决策协议；这里永远不输出任何审批决定或模型正文。
      if (notification) sendNotification(notification, process.env, deadline);
    } catch (_) {
      // 状态增强失败安静降级，不把原始载荷或凭据写入诊断。
    }
  });
}

module.exports = { normalize, makeNotification, frameBytes, fitNotification, sendNotification, main, PLUGIN_VERSION };
if (require.main === module) main();
