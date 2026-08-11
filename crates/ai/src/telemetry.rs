// openWarp 已移除代码库 embedding 索引,原索引同步 telemetry 不再注册。
// Zap:遥测上报层已删除,这里仅保留事件类型壳与分类元数据枚举,
// 供 api_keys 的既有调用点继续类型检查;`send_telemetry_from_ctx!` 在 Zap 里是
// 无副作用的编译期 shim。事件本身不含任何 UGC。
#![allow(dead_code)]

use serde::Serialize;

#[derive(Clone)]
pub enum AITelemetryEvent {
    ProviderCredentialChanged {
        provider: ProviderCredentialTelemetryProvider,
        credential_kind: ProviderCredentialTelemetryKind,
        action: ProviderCredentialTelemetryAction,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderCredentialTelemetryProvider {
    OpenAi,
    Anthropic,
    Google,
    Xai,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderCredentialTelemetryKind {
    PastedKey,
    Oauth,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderCredentialTelemetryAction {
    Added,
    Removed,
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
