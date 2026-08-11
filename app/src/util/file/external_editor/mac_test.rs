use super::is_infinishell_bundle;

#[test]
fn is_infinishell_bundle_recognises_infinishell_channels() {
    // OSS (InfiniShell) 自身。
    assert!(is_infinishell_bundle("dev.infinishell.InfiniShell"));
    // 上游 Warp 各 channel —— 同样视为本应用家族,允许 default-app 重定向。
    assert!(is_infinishell_bundle("dev.warp.WarpDev"));
    assert!(is_infinishell_bundle("dev.warp.WarpPreview"));
    assert!(is_infinishell_bundle("dev.warp.WarpOss"));
}

#[test]
fn is_infinishell_bundle_rejects_other_apps() {
    assert!(!is_infinishell_bundle("com.microsoft.VSCode"));
    assert!(!is_infinishell_bundle("com.apple.TextEdit"));
    assert!(!is_infinishell_bundle("dev.zed.Zed"));
    assert!(!is_infinishell_bundle("invalid"));
    assert!(!is_infinishell_bundle(""));
}
