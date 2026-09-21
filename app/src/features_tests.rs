use super::*;

#[test]
fn infinishell_desktop_entry_enables_ime_without_promoting_managed_tasks() {
    let app_manifest: toml::Value =
        toml::from_str(include_str!("../Cargo.toml")).expect("app/Cargo.toml 应为有效 TOML");
    assert_eq!(
        app_manifest["package"]["default-run"].as_str(),
        Some("infinishell"),
        "默认桌面运行入口应为 infinishell"
    );
    let infinishell_bin = app_manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .and_then(|bins| {
            bins.iter()
                .find(|bin| bin.get("name").and_then(toml::Value::as_str) == Some("infinishell"))
        })
        .expect("app/Cargo.toml 应声明 infinishell 二进制入口");
    assert_eq!(
        infinishell_bin.get("path").and_then(toml::Value::as_str),
        Some("src/bin/infinishell.rs"),
        "IME 门禁测试读取的源码必须是实际桌面入口"
    );

    let source = include_str!("bin/infinishell.rs");
    let enable_call = "state.with_additional_features(&[FeatureFlag::ImeMarkedText])";
    let enable_offset = source
        .find(enable_call)
        .expect("InfiniShell 桌面入口应显式启用 IME marked-text");
    let cfg_offset = source[..enable_offset]
        .rfind("#[cfg(any(")
        .expect("IME marked-text 应由桌面平台 cfg 约束");
    let ime_enablement = &source[cfg_offset..enable_offset + enable_call.len()];

    for target in ["linux", "macos", "windows"] {
        assert!(
            ime_enablement.contains(&format!("target_os = \"{target}\"")),
            "InfiniShell 桌面入口应在 {target} 启用 IME marked-text"
        );
    }
    assert!(
        !source.contains("FeatureFlag::LocalCLIManagedTasks"),
        "完整验收前，InfiniShell 正式 GUI 入口不应启用本地 CLI 托管任务"
    );
}

#[test]
fn desktop_source_keeps_all_three_cli_update_consumer_entries() {
    let settings_source = include_str!("settings_view/ai_page.rs");
    for agent in ["Codex", "Claude", "Grok"] {
        assert!(
            settings_source.contains(&format!(
                "Box::new(CLIAgentUpdateWidget::new(CLIAgent::{agent}))"
            )),
            "桌面设置应保留 {agent} 的升级与渠道入口"
        );
    }

    let app_source = include_str!("lib.rs");
    assert!(
        app_source.contains("terminal::cli_agent::init_cli_agent_updates(ctx)"),
        "桌面应用应初始化三款 CLI 共用的升级模型"
    );
}

#[test]
fn shared_non_gui_defaults_do_not_enable_local_cli_managed_tasks() {
    let app_manifest: toml::Value =
        toml::from_str(include_str!("../Cargo.toml")).expect("app/Cargo.toml 应为有效 TOML");
    let default_features = app_manifest
        .get("features")
        .and_then(|features| features.get("default"))
        .and_then(toml::Value::as_array)
        .expect("app/Cargo.toml 应声明默认 features");

    assert!(
        default_features
            .iter()
            .all(|feature| { feature.as_str() != Some("local_cli_managed_tasks") })
    );

    let tui_manifest: toml::Value =
        toml::from_str(include_str!("../../crates/warp_tui/Cargo.toml"))
            .expect("crates/warp_tui/Cargo.toml 应为有效 TOML");
    for dependency_kind in ["dependencies", "dev-dependencies"] {
        let tui_warp_features = tui_manifest
            .get(dependency_kind)
            .and_then(|dependencies| dependencies.get("warp"))
            .and_then(|warp| warp.get("features"))
            .and_then(toml::Value::as_array)
            .unwrap_or_else(|| panic!("warp_tui 应显式声明 warp {dependency_kind} features"));
        assert!(
            tui_warp_features
                .iter()
                .all(|feature| { feature.as_str() != Some("local_cli_managed_tasks") }),
            "warp_tui {dependency_kind} 不应启用 local_cli_managed_tasks"
        );
    }

    for (name, source) in [
        (
            "dev.rs",
            include_str!("../../crates/warp_tui/src/bin/dev.rs"),
        ),
        (
            "local.rs",
            include_str!("../../crates/warp_tui/src/bin/local.rs"),
        ),
        (
            "oss.rs",
            include_str!("../../crates/warp_tui/src/bin/oss.rs"),
        ),
        (
            "preview.rs",
            include_str!("../../crates/warp_tui/src/bin/preview.rs"),
        ),
        (
            "stable.rs",
            include_str!("../../crates/warp_tui/src/bin/stable.rs"),
        ),
    ] {
        assert!(
            !source.contains("LocalCLIManagedTasks"),
            "warp_tui {name} 不应直接启用 LocalCLIManagedTasks"
        );
    }

    for (name, flags) in [
        ("DEBUG_FLAGS", DEBUG_FLAGS),
        ("LOCAL_FLAGS", LOCAL_FLAGS),
        ("DOGFOOD_FLAGS", DOGFOOD_FLAGS),
        ("PREVIEW_FLAGS", PREVIEW_FLAGS),
        ("RELEASE_FLAGS", RELEASE_FLAGS),
    ] {
        assert!(
            !flags.contains(&FeatureFlag::LocalCLIManagedTasks),
            "{name} 不应为 TUI 或其他非 GUI 入口默认启用本地 CLI 托管任务"
        );
    }
}
