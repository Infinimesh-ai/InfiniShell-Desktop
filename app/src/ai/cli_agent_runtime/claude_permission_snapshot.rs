//! 固定 Claude 的安全权限观察；资料一致也不等于原子父权限上限。

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Rejection {
    InvalidShape,
    UnknownFields,
    NativeErrors,
    IdentityChanged,
    ModeChanged,
    WorkingDirectoryChanged,
    ManagedOnlyUnverified,
    SandboxNotExplicitlyDisabled,
    SettingsLiveRulesMismatch,
    NativeRequestFailed,
    TimedOut,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Observation {
    connection_generation: Uuid,
    native_pid: Option<u64>,
    mode_before: Option<String>,
    mode_after: Option<String>,
    settings: Option<Value>,
    rules: Option<Value>,
    rejections: Vec<Rejection>,
    atomic_permission_ceiling_proven: bool,
    dispatch_authorized: bool,
}

impl Observation {
    pub(super) fn new(generation: Uuid, initialize: &Value) -> Self {
        let mut observation = Self {
            connection_generation: generation,
            native_pid: initialize
                .get("pid")
                .and_then(Value::as_u64)
                .filter(|pid| *pid > 0),
            mode_before: mode(initialize.get("current_permission_mode")).map(str::to_owned),
            mode_after: None,
            settings: None,
            rules: None,
            rejections: Vec::new(),
            atomic_permission_ceiling_proven: false,
            dispatch_authorized: false,
        };
        if observation.native_pid.is_none() || observation.mode_before.is_none() {
            observation.reject(Rejection::InvalidShape);
        }
        observation
    }

    pub(super) fn reject(&mut self, reason: Rejection) {
        if !self.rejections.contains(&reason) {
            self.rejections.push(reason);
        }
    }

    pub(super) fn settings(&mut self, value: &Value) {
        match project_settings(value) {
            Ok((settings, reasons)) => {
                self.settings = Some(settings);
                for reason in reasons {
                    self.reject(reason);
                }
            }
            Err(reason) => self.reject(reason),
        }
    }

    pub(super) fn rules(&mut self, value: &Value) {
        match project_rules(value) {
            Ok((rules, reasons)) => {
                self.rules = Some(rules);
                for reason in reasons {
                    self.reject(reason);
                }
            }
            Err(reason) => self.reject(reason),
        }
    }

    pub(super) fn finish(&mut self, initialize: &Value, cwd: &Path) {
        if self.native_pid.is_none()
            || initialize.get("pid").and_then(Value::as_u64) != self.native_pid
            || initialize.get("session_state").and_then(Value::as_str) != Some("idle")
        {
            self.reject(Rejection::IdentityChanged);
        }
        self.mode_after = mode(initialize.get("current_permission_mode")).map(str::to_owned);
        if self.mode_before.is_none() || self.mode_before != self.mode_after {
            self.reject(Rejection::ModeChanged);
        }
        let (Some(settings), Some(rules)) = (&self.settings, &self.rules) else {
            self.reject(Rejection::InvalidShape);
            return;
        };
        let mut reasons = Vec::new();
        if rules["originalCwd"].as_str().map(Path::new) != Some(cwd) {
            reasons.push(Rejection::WorkingDirectoryChanged);
        }
        if rules["managedOnly"] != false {
            reasons.push(Rejection::ManagedOnlyUnverified);
        }
        if settings
            .pointer("/effective/sandbox/enabled")
            .and_then(Value::as_bool)
            != Some(false)
        {
            reasons.push(Rejection::SandboxNotExplicitlyDisabled);
        }
        let mut expected = BTreeSet::new();
        for source in settings["sources"].as_array().expect("projected sources") {
            for behavior in ["allow", "deny", "ask"] {
                if let Some(values) = source["settings"]["permissions"][behavior].as_array() {
                    for rule in values {
                        expected.insert((source["source"].as_str(), behavior, rule.as_str()));
                    }
                }
            }
        }
        let mut actual = BTreeSet::new();
        let mut inactive = false;
        for rule in rules["rules"].as_array().expect("projected rules") {
            inactive |= rule["notInEffect"] == true;
            if settings_source(rule["source"].as_str().expect("projected source")) {
                actual.insert((
                    rule["source"].as_str(),
                    rule["behavior"].as_str().expect("projected behavior"),
                    rule["rule"].as_str(),
                ));
            }
        }
        let sourced_rules = expected
            .iter()
            .map(|(_, behavior, rule)| (*behavior, *rule))
            .collect::<BTreeSet<_>>();
        let mut effective_rules = BTreeSet::new();
        for behavior in ["allow", "deny", "ask"] {
            if let Some(values) = settings["effective"]["permissions"][behavior].as_array() {
                for rule in values {
                    effective_rules.insert((behavior, rule.as_str()));
                }
            }
        }
        if actual != expected || effective_rules != sourced_rules || inactive {
            reasons.push(Rejection::SettingsLiveRulesMismatch);
        }
        for reason in reasons {
            self.reject(reason);
        }
    }

    pub(super) fn invalidate_mode(&mut self, current: &Value) {
        if mode(Some(current)) != self.mode_after.as_deref().or(self.mode_before.as_deref()) {
            self.reject(Rejection::ModeChanged);
        }
    }

    pub(super) fn value(&self) -> Value {
        serde_json::to_value(self).expect("permission observation is serializable")
    }
}

pub(super) fn mode(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|mode| {
        matches!(
            *mode,
            "default" | "dontAsk" | "plan" | "acceptEdits" | "auto" | "bypassPermissions"
        )
    })
}

fn object(value: &Value) -> Result<&Map<String, Value>, Rejection> {
    value.as_object().ok_or(Rejection::InvalidShape)
}

fn mark_unknown(value: &Map<String, Value>, known: &[&str], reasons: &mut Vec<Rejection>) {
    // 不写未知键名和值；它们可能来自 env、认证 helper 或未来私有配置。
    if value.keys().any(|key| !known.contains(&key.as_str())) {
        reasons.push(Rejection::UnknownFields);
    }
}

fn settings_source(value: &str) -> bool {
    matches!(
        value,
        "userSettings" | "projectSettings" | "localSettings" | "flagSettings" | "policySettings"
    )
}

fn rule_source(value: &str) -> bool {
    settings_source(value)
        || matches!(
            value,
            "cliArg"
                | "command"
                | "session"
                | "toolsNarrowing"
                | "mcpServerPolicy"
                | "hostCredential"
        )
}

fn setting_fields(value: &Value, reasons: &mut Vec<Rejection>) -> Result<Value, Rejection> {
    let value = object(value)?;
    mark_unknown(value, &["permissions", "sandbox"], reasons);
    let mut projected = Map::new();
    if let Some(permissions) = value.get("permissions") {
        let permissions = object(permissions)?;
        let lists = ["allow", "deny", "ask", "additionalDirectories"];
        let strings = ["defaultMode", "disableBypassPermissionsMode"];
        mark_unknown(
            permissions,
            &[
                "allow",
                "deny",
                "ask",
                "additionalDirectories",
                "defaultMode",
                "disableBypassPermissionsMode",
            ],
            reasons,
        );
        let mut fields = Map::new();
        for key in lists {
            if let Some(value) = permissions.get(key) {
                if !value
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string))
                {
                    return Err(Rejection::InvalidShape);
                }
                fields.insert(key.to_string(), value.clone());
            }
        }
        for key in strings {
            if let Some(value) = permissions.get(key) {
                if !value.is_string() {
                    return Err(Rejection::InvalidShape);
                }
                fields.insert(key.to_string(), value.clone());
            }
        }
        projected.insert("permissions".into(), Value::Object(fields));
    }
    if let Some(sandbox) = value.get("sandbox") {
        let sandbox = object(sandbox)?;
        let keys = [
            "enabled",
            "failIfUnavailable",
            "autoAllowBashIfSandboxed",
            "allowUnsandboxedCommands",
        ];
        mark_unknown(sandbox, &keys, reasons);
        let mut fields = Map::new();
        for key in keys {
            if let Some(value) = sandbox.get(key) {
                if !value.is_boolean() {
                    return Err(Rejection::InvalidShape);
                }
                fields.insert(key.to_string(), value.clone());
            }
        }
        projected.insert("sandbox".into(), Value::Object(fields));
    }
    Ok(Value::Object(projected))
}

fn project_settings(value: &Value) -> Result<(Value, Vec<Rejection>), Rejection> {
    let value_object = object(value)?;
    let mut reasons = Vec::new();
    mark_unknown(
        value_object,
        &["effective", "sources", "applied", "errors"],
        &mut reasons,
    );
    if value
        .get("errors")
        .is_some_and(|errors| !errors.is_null() && errors != &json!([]))
    {
        reasons.push(Rejection::NativeErrors);
    }
    let effective = setting_fields(&value["effective"], &mut reasons)?;
    let sources = value["sources"].as_array().ok_or(Rejection::InvalidShape)?;
    let mut projected = Vec::new();
    let mut seen = BTreeSet::new();
    for source in sources {
        let source_object = object(source)?;
        mark_unknown(source_object, &["source", "settings"], &mut reasons);
        let name = source["source"].as_str().ok_or(Rejection::InvalidShape)?;
        if !settings_source(name) || !seen.insert(name) {
            return Err(Rejection::InvalidShape);
        }
        projected.push(
            json!({"source":name, "settings":setting_fields(&source["settings"], &mut reasons)?}),
        );
    }
    Ok((json!({"effective":effective, "sources":projected}), reasons))
}

fn project_rules(value: &Value) -> Result<(Value, Vec<Rejection>), Rejection> {
    let mut reasons = Vec::new();
    mark_unknown(object(value)?, &["state"], &mut reasons);
    let state = &value["state"];
    mark_unknown(
        object(state)?,
        &[
            "rules",
            "workspaceDirectories",
            "originalCwd",
            "managedOnly",
            "errors",
        ],
        &mut reasons,
    );
    if state
        .get("errors")
        .is_some_and(|errors| !errors.is_null() && errors != &json!([]))
    {
        reasons.push(Rejection::NativeErrors);
    }
    let cwd = state["originalCwd"]
        .as_str()
        .ok_or(Rejection::InvalidShape)?;
    let managed = state["managedOnly"]
        .as_bool()
        .ok_or(Rejection::InvalidShape)?;
    let mut rules = Vec::new();
    for rule in state["rules"].as_array().ok_or(Rejection::InvalidShape)? {
        mark_unknown(
            object(rule)?,
            &[
                "behavior",
                "source",
                "rule",
                "editability",
                "notInEffect",
                "description",
            ],
            &mut reasons,
        );
        let behavior = rule["behavior"].as_str().ok_or(Rejection::InvalidShape)?;
        let source = rule["source"].as_str().ok_or(Rejection::InvalidShape)?;
        let raw = rule["rule"].as_str().ok_or(Rejection::InvalidShape)?;
        let editability = rule["editability"]
            .as_str()
            .ok_or(Rejection::InvalidShape)?;
        if !matches!(behavior, "allow" | "deny" | "ask")
            || !rule_source(source)
            || !matches!(editability, "readonly" | "persistent" | "session")
        {
            return Err(Rejection::InvalidShape);
        }
        let mut projected =
            json!({"behavior":behavior, "source":source, "rule":raw, "editability":editability});
        if let Some(inactive) = rule.get("notInEffect") {
            if !inactive.is_boolean() {
                return Err(Rejection::InvalidShape);
            }
            projected["notInEffect"] = inactive.clone();
        }
        rules.push(projected);
    }
    let mut directories = Vec::new();
    for directory in state["workspaceDirectories"]
        .as_array()
        .ok_or(Rejection::InvalidShape)?
    {
        mark_unknown(object(directory)?, &["path", "source"], &mut reasons);
        let path = directory["path"].as_str().ok_or(Rejection::InvalidShape)?;
        let source = directory["source"]
            .as_str()
            .ok_or(Rejection::InvalidShape)?;
        if !rule_source(source) {
            return Err(Rejection::InvalidShape);
        }
        directories.push(json!({"path":path, "source":source}));
    }
    Ok((
        json!({"rules":rules, "workspaceDirectories":directories, "originalCwd":cwd, "managedOnly":managed}),
        reasons,
    ))
}

#[cfg(test)]
#[path = "claude_permission_snapshot_tests.rs"]
mod tests;
