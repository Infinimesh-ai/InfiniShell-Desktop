use super::Icon;

#[test]
fn infinishell_brand_icons_use_canonical_assets() {
    assert_eq!(
        <&'static str>::from(Icon::InfiniShell),
        "bundled/svg/infinishell-mark.svg"
    );
    assert_eq!(
        <&'static str>::from(Icon::Agent),
        "bundled/svg/infinishell-mark.svg"
    );
    assert_eq!(
        <&'static str>::from(Icon::InfiniShellDrive),
        "bundled/svg/infinishell-drive.svg"
    );
}
