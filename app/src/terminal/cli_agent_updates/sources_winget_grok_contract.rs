//! 官方 WinGet 固定 portable 发行合同；官网与 npm 的 Windows 映像不能混用。

pub(super) const VERSION: &str = "1.0.41";
pub(super) const OLD_VERSION: &str = "1.0.40";
pub(super) const PACKAGE: &str = "xAI.GrokBuild";
pub(super) const SOURCE: &str = "Microsoft.Winget.Source_8wekyb3d8bbwe";
pub(super) const PRODUCT: &str = "xAI.GrokBuild_Microsoft.Winget.Source_8wekyb3d8bbwe";
pub(super) const MANIFEST: &str = "https://raw.githubusercontent.com/microsoft/winget-pkgs/6056b5e9a577db0f8d503323ae8b05a585e6c2be/manifests/x/xAI/GrokBuild/1.0.41/xAI.GrokBuild.installer.yaml";
pub(super) const MANIFEST_SHA: &str =
    "cc2011562e6505dfa4e4f9eed955e93368eee2b33b5faf008733ffc313fcc252";
pub(super) const IMAGE_URL: &str = "https://x.ai/cli/grok-1.0.41-windows-x86_64.exe";
pub(super) const ALIAS: &str = "grok.exe";

pub(super) fn native(version: &str) -> Option<&'static str> {
    match version {
        OLD_VERSION => Some("grok-1.0.40-windows-x86_64.exe"),
        VERSION => Some("grok-1.0.41-windows-x86_64.exe"),
        _ => None,
    }
}

pub(super) fn image(version: &str) -> Option<(u64, &'static str)> {
    match version {
        OLD_VERSION => Some((
            153_730_376,
            "034c883fa3962ab6ca409c2d3c7501c642166535dd39fa936ecffe1ac2cad92e",
        )),
        VERSION => Some((
            154_082_120,
            "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        )),
        _ => None,
    }
}
