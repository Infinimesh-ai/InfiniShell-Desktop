"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

function configAllowsPlugin(config, requireEnabled = false) {
  // inspect 在固定 41 中会把已禁用插件误报为 enabled；再核对原生写出的名单。
  // 不实现完整 TOML：复杂或含糊写法关闭增强，绝不重写用户配置来取得授权。
  if (typeof config !== "string" || config.length > 1024 * 1024
      || config.includes("\0") || config.includes('"""') || config.includes("'''")) return false;
  try {
    const strings = [];
    const text = config.replace(/"(?:\\.|[^"\\\r\n])*"|'[^'\r\n]*'|#[^\r\n]*/g, value => {
      if (value.startsWith("#")) return "";
      strings.push(value.startsWith('"') ? JSON.parse(value) : value.slice(1, -1));
      return `\0${strings.length - 1}\0`;
    });
    if (/["']/.test(text) || /^[ \t]*(?:plugins|\0\d+\0)\s*[.=]/m.test(text)) return false;
    const headers = [...text.matchAll(/^[ \t]*\[\[?[^\]\r\n]+\]\]?[ \t]*$/gm)];
    if (headers.some(header => header[0].includes("\0")
        || (header[0].includes("plugins") && header[0].trim() !== "[plugins]"))) return false;
    const sections = headers.filter(header => header[0].trim() === "[plugins]");
    if (sections.length === 0) return !requireEnabled;
    if (sections.length !== 1) return false;
    const header = sections[0];
    const next = headers.find(candidate => candidate.index > header.index);
    const body = text.slice(header.index + header[0].length, next?.index);
    if (/^[ \t]*\0\d+\0\s*=/m.test(body)) return false;
    const lists = {};
    const remainder = body.replace(/^[ \t]*(enabled|disabled)\s*=\s*\[([^\]]*)\][ \t]*(?=\r?$)/gm,
      (entry, key, contents) => {
        if (Object.hasOwn(lists, key)) throw new Error("duplicate_list");
        const items = contents.trim() ? contents.trim().replace(/,\s*$/, "").split(",") : [];
        lists[key] = items.map(item => {
          const match = item.trim().match(/^\0(\d+)\0$/);
          if (!match) throw new Error("unknown_list");
          return strings[Number(match[1])];
        });
        return "";
      });
    if (/\b(enabled|disabled)\b/.test(remainder)) return false;
    const owned = name => name === "infinishell-grok" || name.endsWith("/infinishell-grok");
    return (!requireEnabled && !Object.hasOwn(lists, "enabled") || !!lists.enabled?.some(owned))
      && !lists.disabled?.some(owned);
  } catch (_) {
    return false;
  }
}

function configEnablesPlugin(config) {
  return configAllowsPlugin(config, true);
}

function configLayersAllowPlugin(state, grokHome, read = filename => fs.readFileSync(filename, "utf8")) {
  // 原生列出真正参与当前目录的配置层；任何一层禁用或无法确认都关闭增强。
  const layers = state?.configSources?.layers;
  const userConfig = path.join(grokHome, "config.toml");
  if (!Array.isArray(layers) || layers.length === 0 || layers.length > 32
      || layers.filter(layer => layer.role === "user" && layer.path === userConfig).length !== 1) return false;
  try {
    return layers.every(layer => typeof layer.path === "string" && path.isAbsolute(layer.path)
      && layer.path.endsWith(".toml") && !layer.note
      && configAllowsPlugin(read(layer.path), layer.path === userConfig));
  } catch (_) {
    return false;
  }
}

function hookIsEnabled(disabledHooks, hookName) {
  // 全局补桥与原插件使用同一事件；原生 /hooks 禁用原插件事件时，补桥也必须静默。
  const match = typeof hookName === "string"
    && hookName.match(/^global\/infinishell-1\.0\.41:([a-z_]+\[0\]\.hooks\[0\])$/);
  if (!match || typeof disabledHooks !== "string" || disabledHooks.length > 1024 * 1024
      || disabledHooks.includes("\0")) return false;
  const original = `plugin/infinishell-grok/hooks:${match[1]}`;
  return !disabledHooks.split(/\r?\n/).some(name => name.trim() === hookName || name.trim() === original);
}

function permitsNotification(state, pluginRoot) {
  if (!state || state.grokVersion !== "1.0.41" || state.projectTrusted !== true
      || !Array.isArray(state.plugins) || !state.permissions
      || (state.permissions.enforced !== undefined && (!Array.isArray(state.permissions.enforced)
        || state.permissions.enforced.some(pin => pin.setting === "nonManagedHooks" && pin.enabled !== true)))) return false;
  const plugins = state.plugins.filter(plugin => plugin.name === "infinishell-grok");
  return plugins.length === 1 && plugins[0].enabled === true
    && plugins[0].path === pluginRoot && plugins[0].provides?.hooks === true;
}

function main(executable, pluginRoot) {
  try {
    // 全局源只补固定桌面版本的加载缺口，不能借它绕过原生插件禁用和项目信任。
    if (!["darwin", "linux", "win32"].includes(process.platform) || process.env.GROK_CONFIG_PATH || process.env.GROK_CONFIG) return;
    const grokHome = path.dirname(path.dirname(pluginRoot));
    if (fs.readFileSync(path.join(grokHome, ".metadata_version"), "utf8").trim() !== "1.0.41") return;
    const state = JSON.parse(execFileSync(executable, ["inspect", "--json"], {
      encoding: "utf8", timeout: 500, maxBuffer: 1024 * 1024, windowsHide: true,
      stdio: ["ignore", "pipe", "ignore"],
    }));
    if (!permitsNotification(state, pluginRoot) || !configLayersAllowPlugin(state, grokHome)) return;
    const disabledPath = path.join(grokHome, "disabled-hooks");
    let disabledHooks = "";
    try {
      disabledHooks = fs.readFileSync(disabledPath, "utf8");
    } catch (error) {
      if (error.code !== "ENOENT") return;
    }
    if (!hookIsEnabled(disabledHooks, process.env.GROK_HOOK_NAME)) return;
    // 沿用原 mapper 的事件身份，应用按相同 ID 去重；stdout 永远不写审批决定。
    require(path.join(pluginRoot, "hooks/notify.cjs")).main();
  } catch (_) {
    // 检查失败仅关闭通知增强，不记录原生配置、输入或认证信息。
  }
}

module.exports = { configEnablesPlugin, configLayersAllowPlugin, hookIsEnabled, permitsNotification, main };
if (require.main === module) main(process.argv[2], process.argv[3]);
