"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
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
  const notifications = captured.filter(item => item.source === "grok").map(item => {
    const input = plugin.normalize(item.payload, {
      ...item.environment, WARP_CLI_AGENT_PROTOCOL_VERSION: "1",
    });
    return plugin.makeNotification(input);
  }).filter(Boolean);
  assert.deepEqual(notifications.map(item => item.event), [
    "session_start", "prompt_submit", "stop_failure", "notification", "notification",
  ]);
  assert.equal(notifications[2].error_type, "invalid_request");
  assert.equal(notifications[3].terminal_unverified, true);
  assert.equal(notifications[4].terminal_unverified, true);
});

test("Claude 兼容 hook 与原生 hook 共享稳定事件标识", () => {
  const original = captured.find(item => item.source === "grok" && item.payload.hookEventName === "stop_failure");
  const inherited = captured.find(item => item.source === "claude" && item.payload.hookEventName === "stop_failure");
  const first = plugin.normalize(original.payload, { ...original.environment, WARP_CLI_AGENT_PROTOCOL_VERSION: "1" });
  const second = plugin.normalize(inherited.payload, { ...inherited.environment, WARP_CLI_AGENT_PROTOCOL_VERSION: "1" });
  const nativeNotification = plugin.makeNotification(first);
  const inheritedNotification = plugin.makeNotification(second);
  assert.equal(nativeNotification.event, "stop_failure");
  assert.equal(inheritedNotification.event, "stop_failure");
  assert.equal(nativeNotification.event_id, inheritedNotification.event_id);
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
  const tool = plugin.makeNotification(normalized("post_tool_use_failure"));
  const stop = plugin.makeNotification(normalized("stop", {
    reason: "end_turn", lastAssistantMessage: "done", stopHookActive: false,
  }));
  assert.equal(tool.event, "tool_complete");
  assert.equal(stop.event, "stop");
});

test("不明确的 Stop 保持降级通知", () => {
  for (const fields of [
    { lastAssistantMessage: "text" },
    { reason: "shutdown", lastAssistantMessage: "text", stopHookActive: false },
    { reason: "end_turn", lastAssistantMessage: "text", stopHookActive: true },
  ]) {
    const notification = plugin.makeNotification(normalized("stop", fields));
    assert.equal(notification.event, "notification");
    assert.equal(notification.terminal_unverified, true);
  }
});

test("原生 prompt_id 保留给应用关联旧回合和新回合", () => {
  const prompt = plugin.makeNotification(normalized("user_prompt_submit", {
    promptId: "new", timestamp: "2026-09-16T01:00:02Z",
  }));
  const oldFailure = plugin.makeNotification(normalized("stop_failure", {
    promptId: "old", timestamp: "2026-09-16T01:00:01Z",
  }));
  const newFailure = plugin.makeNotification(normalized("stop_failure", {
    promptId: "new", timestamp: "2026-09-16T01:00:03Z",
  }));
  const stop = plugin.makeNotification(normalized("stop", {
    promptId: "new", timestamp: "2026-09-16T01:00:04Z", reason: "end_turn",
    lastAssistantMessage: "done", stopHookActive: false,
  }));
  assert.equal(prompt.prompt_id, "new");
  assert.equal(oldFailure.prompt_id, "old");
  assert.equal(oldFailure.event, "stop_failure");
  assert.equal(newFailure.prompt_id, "new");
  assert.equal(stop.prompt_id, "new");
  assert.equal(stop.event, "stop");
  assert.notEqual(oldFailure.event_id, newFailure.event_id);
});

test("Node 只把纯 JSON 交给绝对路径 worker", () => {
  const notification = plugin.makeNotification(normalized("notification", {
    message: "a\x1b]0;injected\x07\x9c", cwd: 'C:\\用户\\a "quote"\\file.txt',
  }));
  const original = structuredClone(notification);
  const executable = path.resolve("infinishell-notify-worker");
  const calls = [];
  const execute = (file, args, options) => {
    calls.push({ file, args, options });
    return calls.length === 1 ? '{"protocol":1,"maxFrameBytes":4096}\n' : "";
  };
  assert.equal(plugin.sendNotification(notification, {
    ...environment("notification"), WARP_CLI_AGENT_NOTIFY_EXECUTABLE: executable,
  }, 5000, execute, () => 1000), true);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].file, executable);
  assert.deepEqual(calls[0].args, ["cli-agent-notify", "--protocol-version"]);
  assert.deepEqual(calls[1].args, ["cli-agent-notify"]);
  assert.deepEqual(JSON.parse(calls[1].options.input), JSON.parse(JSON.stringify(notification)));
  assert.equal(calls[1].options.shell, undefined);
  assert.deepEqual(notification, original);
});

test("4096 字节预算按 tmux 最坏帧计算且只移除可选文本", () => {
  const notification = plugin.makeNotification(normalized("notification", {
    promptId: "prompt-one", message: `semi;control\x9c${"中".repeat(4096)}`,
  }));
  const fitted = plugin.fitNotification(notification);
  assert.ok(fitted);
  assert.ok(plugin.frameBytes(fitted) <= 4096);
  assert.equal(fitted.session_id, notification.session_id);
  assert.equal(fitted.prompt_id, notification.prompt_id);
  assert.equal(fitted.event_id, notification.event_id);
  assert.equal(Object.hasOwn(fitted, "summary"), false);
  assert.equal(plugin.fitNotification({ ...notification, session_id: "s".repeat(5000) }), null);
});

test("worker 协议必须精确匹配且空 stdout 才表示完整写入", () => {
  const notification = plugin.makeNotification(normalized("session_start"));
  const executable = path.resolve("infinishell-notify-worker");
  const env = { ...environment("session_start"), WARP_CLI_AGENT_NOTIFY_EXECUTABLE: executable };
  for (const reply of [
    "not-json", "{}", '{"protocol":2,"maxFrameBytes":4096}',
    '{"protocol":1,"maxFrameBytes":4095}',
    '{"protocol":1,"maxFrameBytes":4096,"extra":true}',
  ]) {
    let calls = 0;
    assert.equal(plugin.sendNotification(notification, env, 5000, () => {
      calls += 1;
      return reply;
    }, () => 1000), false);
    assert.equal(calls, 1);
  }
  let calls = 0;
  assert.equal(plugin.sendNotification(notification, env, 5000, () => {
    calls += 1;
    return calls === 1 ? '{"protocol":1,"maxFrameBytes":4096}' : "unexpected";
  }, () => 1000), false);
  assert.equal(calls, 2);
});

test("一次 worker 失败不在 Node 留状态，显式重投仍调用相同事件", () => {
  const notification = plugin.makeNotification(normalized("session_start"));
  const executable = path.resolve("infinishell-notify-worker");
  const env = { ...environment("session_start"), WARP_CLI_AGENT_NOTIFY_EXECUTABLE: executable };
  const calls = [];
  let failSend = true;
  const execute = (file, args, options) => {
    calls.push({ file, args, options });
    if (args.includes("--protocol-version")) return '{"protocol":1,"maxFrameBytes":4096}\n';
    if (failSend) {
      failSend = false;
      throw new Error("terminal unavailable");
    }
    return "";
  };
  assert.equal(plugin.sendNotification(notification, env, 5000, execute, () => 1000), false);
  assert.equal(plugin.sendNotification(notification, env, 5000, execute, () => 1000), true);
  assert.equal(calls.length, 4);
  assert.equal(calls[1].options.input, calls[3].options.input);
  assert.equal(plugin.sendNotification(notification, {
    ...env, WARP_CLI_AGENT_NOTIFY_EXECUTABLE: "relative-worker",
  }, 5000, () => assert.fail("相对路径不能启动"), () => 1000), false);
});
