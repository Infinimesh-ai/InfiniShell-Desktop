use std::borrow::Cow;
use std::path::Path;

use command::blocking::Command;
use serde_json::{Value, json};
use warp_core::session_id::SessionId;
use warpui::AssetProvider;

use super::{ShellType, init_shell_script_for_shell};

struct InitAssets;

impl AssetProvider for InitAssets {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        assert_eq!(path, "bundled/bootstrap/pwsh_init_shell.ps1");
        Ok(Cow::Borrowed(include_bytes!(
            "../../assets/bundled/bootstrap/pwsh_init_shell.ps1"
        )))
    }
}

fn run_case(case: &str) -> (i32, Value) {
    let fixture = tempfile::Builder::new()
        .prefix("warp-private-pwsh-中文 '")
        .tempdir()
        .unwrap();
    let init_path = fixture.path().join("init.ps1");
    std::fs::write(
        &init_path,
        init_shell_script_for_shell(ShellType::PowerShell, &InitAssets, SessionId::from(42)),
    )
    .unwrap();
    // 使用 Windows 自带的固定入口；不跳过失败，也不执行任何真实 profile。
    let powershell = Path::new(&std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = Command::new(&powershell)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/terminal/bootstrap_powershell_private_tests.ps1"),
        )
        .arg(case)
        .env("WARP_TEST_FIXTURE", fixture.path())
        .env("WARP_TEST_INIT", &init_path)
        .env(
            "WARP_TEST_ASSETS",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"),
        )
        .env(
            "PSModulePath",
            std::env::join_paths([
                powershell.parent().unwrap().join("Modules"),
                Path::new(&std::env::var_os("ProgramFiles").unwrap())
                    .join("WindowsPowerShell/Modules"),
            ])
            .unwrap(),
        )
        .output()
        .unwrap();
    let receipt_path = fixture.path().join("result.json");
    let receipt = std::fs::read(&receipt_path)
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or(Value::Null);
    // 错误必须在共享 RC 和 Bootstrap 回报之前结束，不能只检查非零退出码。
    if !output.status.success() {
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "WARP_POWERSHELL_PRIVATE_STARTUP_FAILED"
        );
        assert!(!fixture.path().join("rc-executed.txt").exists());
        assert!(!receipt_path.exists());
        if case != "missing_init" && case != "bootstrap_options_failure" {
            assert!(!fixture.path().join("after-init.txt").exists());
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains("426F6F747374726170706564"));
    } else {
        assert!(
            output.stderr.is_empty(),
            "非预期 PowerShell 错误: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    (output.status.code().unwrap(), receipt)
}

#[test]
fn private_startup_preserves_history_policy_across_module_reload() {
    let (exit, result) = run_case("private");
    assert_eq!(exit, 0);
    assert_eq!(result["rc_executed"], json!(false));
    assert_eq!(result["unloaded_after_init"], json!(true));
    assert_eq!(result["style"], json!("SaveNothing"));
    assert_eq!(result["history_private"], json!(true));
    assert_eq!(result["history_empty"], json!(true));
    assert_eq!(result["real_bootstrapped_path_matches"], json!(true));
    assert_eq!(result["init_shell_emitted"], json!(true));
}

#[test]
fn private_startup_uses_distinct_history_for_each_session() {
    let (exit, result) = run_case("two_sessions");
    assert_eq!(exit, 0);
    assert_eq!(result["distinct_history"], json!(true));
}

#[test]
fn absent_opt_in_preserves_profiles_and_history() {
    let (exit, result) = run_case("default");
    assert_eq!(exit, 0);
    assert_eq!(result["rc_sequence"], json!("1234"));
    assert_eq!(result["style"], json!("SaveIncrementally"));
    assert_eq!(result["history_private"], json!(false));
    assert_eq!(result["real_bootstrapped_path_matches"], json!(true));
}

#[test]
fn private_startup_rejects_relative_root_before_bootstrap() {
    assert_eq!(run_case("relative").0, 1);
}

#[test]
fn private_startup_rejects_missing_root_before_bootstrap() {
    assert_eq!(run_case("missing").0, 1);
}

#[test]
fn private_startup_rejects_file_root_before_bootstrap() {
    assert_eq!(run_case("file").0, 1);
}

#[test]
fn private_startup_rejects_reparse_ancestor_before_bootstrap() {
    assert_eq!(run_case("junction").0, 1);
}

#[test]
fn private_startup_rejects_bootstrap_without_private_init() {
    assert_eq!(run_case("missing_init").0, 1);
}

#[test]
fn private_startup_exits_host_when_init_cannot_set_history_policy() {
    assert_eq!(run_case("init_options_failure").0, 1);
}

#[test]
fn private_startup_exits_host_when_bootstrap_cannot_set_history_policy() {
    assert_eq!(run_case("bootstrap_options_failure").0, 1);
}

#[test]
fn remote_session_does_not_enable_windows_local_opt_in() {
    let (exit, result) = run_case("remote");
    assert_eq!(exit, 0);
    assert_eq!(result["rc_sequence"], json!("1234"));
    assert_eq!(result["history_private"], json!(false));
}

#[test]
fn local_session_takes_priority_over_inherited_ssh_flag() {
    let (exit, result) = run_case("local_inherited_ssh");
    assert_eq!(exit, 0);
    assert_eq!(result["style"], json!("SaveNothing"));
    assert_eq!(result["history_private"], json!(true));
    assert_eq!(result["rc_executed"], json!(false));
    assert_eq!(result["real_bootstrapped_path_matches"], json!(true));
}
