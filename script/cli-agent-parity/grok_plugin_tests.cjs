"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");
const crypto = require("node:crypto");
const plugin = require("../../app/assets/bundled/cli-agent-plugins/grok/hooks/notify.cjs");

const fixtures = path.join(__dirname, "../../specs/cli-agent-parity/fixtures");
const captured = fs.readFileSync(path.join(fixtures, "grok-1.0.30-hooks.ndjson"), "utf8")
  .trim().split("\n").map(line => JSON.parse(line));

function environment(event, session = "session-test") {
  return {
    WARP_CLI_AGENT_PROTOCOL_VERSION: "1",
    GROK_HOOK_EVENT: event,
    GROK_SESSION_ID: session,
  };
}

function normalized(event, fields = {}) {
  return plugin.normalize({
    hookEventName: event,
    sessionId: "session-test",
    timestamp: "2026-09-16T01:00:00Z",
    ...fields,
  }, environment(event));
}

test("真实 session_start 上报版本匹配随附 manifest 与 manager 最低版本", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(__dirname,
    "../../app/assets/bundled/cli-agent-plugins/grok/.grok-plugin/plugin.json"), "utf8"));
  const manager = fs.readFileSync(path.join(__dirname,
    "../../app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs"), "utf8");
  // 解析实际 minimum 方法返回的标识符，再读取其声明，不能在测试中抄版本常量。
  const method = manager.match(/fn minimum_plugin_version\(&self\) -> &'static str\s*\{\s*([A-Z_]+)\s*\}/);
  assert.ok(method, "manager 最低版本方法形状必须明确");
  const declaration = manager.match(new RegExp(`const ${method[1]}: &str = "([^"]+)";`));
  assert.ok(declaration, "manager 最低版本声明必须明确");
  const notification = plugin.makeNotification(normalized("session_start"), {});
  assert.equal(notification.event, "session_start");
  assert.equal(notification.plugin_version, manifest.version);
  assert.equal(notification.plugin_version, declaration[1]);
});

test("固定 0.1.0 通知脚本保留父提交原字节和旧版本上报", () => {
  const file = path.join(fixtures, "grok-plugin-0.1.0-notify.cjs");
  assert.equal(crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex"),
    "134fd490e7396157c80c2a33bd9d8a88397881e6f926743319fafe6c0df5ccd8");
  const legacy = require(file);
  const input = legacy.normalize({ hookEventName: "session_start", sessionId: "session-test" },
    environment("session_start"));
  assert.equal(legacy.makeNotification(input, {}).plugin_version, "0.1.0");
});

test("真实额度错误和关闭回调不会变成完成", () => {
  const state = {};
  const notifications = captured.filter(item => item.source === "grok").map(item => {
    const input = plugin.normalize(item.payload, {
      ...item.environment, WARP_CLI_AGENT_PROTOCOL_VERSION: "1",
    });
    return plugin.makeNotification(input, state);
  }).filter(Boolean);
  assert.deepEqual(notifications.map(item => item.event), [
    "session_start", "prompt_submit", "stop_failure",
  ]);
  assert.equal(state.failed, true);
  assert.equal(state.closed, true);
});

test("Claude 兼容 hook 与原生 hook 共享稳定事件标识", () => {
  const original = captured.find(item => item.source === "grok" && item.payload.hookEventName === "stop_failure");
  const inherited = captured.find(item => item.source === "claude" && item.payload.hookEventName === "stop_failure");
  const state = {};
  const first = plugin.normalize(original.payload, { ...original.environment, WARP_CLI_AGENT_PROTOCOL_VERSION: "1" });
  const second = plugin.normalize(inherited.payload, { ...inherited.environment, WARP_CLI_AGENT_PROTOCOL_VERSION: "1" });
  assert.equal(plugin.makeNotification(first, state).event, "stop_failure");
  assert.equal(plugin.makeNotification(second, state), null);
});

test("缺失或冲突的真实 CLI 身份不发通知", () => {
  assert.equal(plugin.normalize({ hookEventName: "stop", sessionId: "one" }, {}), null);
  assert.equal(plugin.normalize({ hookEventName: "stop", sessionId: "one" }, environment("stop", "two")), null);
  assert.equal(plugin.normalize({ hookEventName: "stop", hook_event_name: "StopFailure", sessionId: "one" }, environment("stop", "one")), null);
  assert.throws(() => plugin.normalize({ hookEventName: "stop", sessionId: "one", session_id: "two" }, environment("stop", "one")), /conflicting_alias/);
});

test("只含 Claude 拼写的 Grok 载荷仍正确识别", () => {
  const input = plugin.normalize({
    hook_event_name: "UserPromptSubmit", session_id: "session-test", prompt: "中文\nsecond line",
  }, environment("user_prompt_submit"));
  assert.equal(plugin.makeNotification(input, {}).query, "中文\nsecond line");
});

test("PermissionDenied 不进入等待审批", () => {
  const notification = plugin.makeNotification(normalized("permission_denied"), {});
  assert.equal(notification.event, "notification");
  assert.equal(notification.error_type, "permission_denied");
});

test("单个工具失败后仍能接收完成", () => {
  const state = {};
  const tool = plugin.makeNotification(normalized("post_tool_use_failure"), state);
  const stop = plugin.makeNotification(normalized("stop", { reason: "end_turn", lastAssistantMessage: "done" }), state);
  assert.equal(tool.event, "tool_complete");
  assert.equal(stop.event, "stop");
});

test("不明确的 Stop 保持降级通知", () => {
  assert.equal(plugin.makeNotification(normalized("stop", { lastAssistantMessage: "text" }), {}).event, "notification");
  assert.equal(plugin.makeNotification(normalized("stop", { reason: "shutdown", lastAssistantMessage: "text" }), {}), null);
  assert.equal(plugin.makeNotification(normalized("stop", { reason: "end_turn", lastAssistantMessage: "text", stopHookActive: true }), {}).event, "notification");
});

test("旧回合失败不会污染新回合，失败的同一回合也不会被 Stop 覆盖", () => {
  const state = {};
  plugin.makeNotification(normalized("user_prompt_submit", { promptId: "new", timestamp: "2026-09-16T01:00:02Z" }), state);
  assert.equal(plugin.makeNotification(normalized("stop_failure", { promptId: "old", timestamp: "2026-09-16T01:00:01Z" }), state), null);
  assert.equal(plugin.makeNotification(normalized("stop_failure", { promptId: "new", timestamp: "2026-09-16T01:00:03Z" }), state).event, "stop_failure");
  assert.equal(plugin.makeNotification(normalized("stop", { promptId: "new", timestamp: "2026-09-16T01:00:04Z", reason: "end_turn", lastAssistantMessage: "done" }), state), null);
});

test("终端控制字节不会从文本逃逸，Windows 路径保持字面值", () => {
  const notification = { event: "notification", summary: "a\x1b]0;injected\x07\x9c", cwd: 'C:\\用户\\a "quote"\\file.txt' };
  const encoded = plugin.encode(notification, {});
  assert.equal(encoded, '\x1b]777;notify;warp://cli-agent;{"event":"notification","summary":"a\\u001b]0;injected\\u0007\\u009c","cwd":"C:\\\\用户\\\\a \\"quote\\"\\\\file.txt"}\x07');
});

test("tmux DCS 转义恰好包裹一次通知", () => {
  assert.equal(plugin.encode({ v: 1 }, { TMUX: "/tmp/tmux-1000/default,1,0" }),
    '\x1bPtmux;\x1b\x1b]777;notify;warp://cli-agent;{"v":1}\x07\x1b\\');
});

test("跨进程状态文件保留失败并且不保存提示词", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "grok-state-test-"));
  try {
    const messages = [];
    const failure = normalized("stop_failure", { promptId: "one", prompt: "private draft" });
    plugin.withSessionState(failure, directory, message => messages.push(message));
    plugin.withSessionState(failure, directory, message => messages.push(message));
    plugin.withSessionState(normalized("stop", { promptId: "one", reason: "end_turn", lastAssistantMessage: "done" }), directory, message => messages.push(message));
    assert.deepEqual(messages.map(message => message.event), ["stop_failure"]);
    const files = fs.readdirSync(path.join(directory, "infinishell-status-v1"));
    assert.equal(files.length, 1);
    assert.equal(fs.readFileSync(path.join(directory, "infinishell-status-v1", files[0]), "utf8").includes("private draft"), false);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
