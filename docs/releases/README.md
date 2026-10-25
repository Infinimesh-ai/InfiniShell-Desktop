# Current release and acceptance scope

[简体中文](README.zh-CN.md) · [Documentation index](../README.md)

As of October 25, 2026, the stable Latest release is [v0.2026.10.21.15.36.oss_00](https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/tag/v0.2026.10.21.15.36.oss_00), published at 22:05:02 UTC on October 21 (06:05:02 Beijing time on October 22). Product source is pinned to `255849d04ffe6a8645f7ec31f3cd6fd58004c4be`. Subsequent documentation maintenance does not move the product tag or constitute new product acceptance.

| Purpose | Assets |
| --- | --- |
| Desktop installation | macOS Apple Silicon DMG, Windows x64 Setup, and Linux x86_64 AppImage/DEB/RPM/Arch: 6 assets |
| SSH extensions | x86_64 and aarch64 for each of macOS/Linux/Windows: 6 assets |

Intel macOS has an SSH extension only, not a desktop DMG. Windows Setup is `NotSigned`; do not describe it as signed.

## Acceptance boundaries

- The approved CLI merge scope, G01–G10 and V03 (11 items), is closed and merged. Deferred and limited V01/V02/V05 coverage remains; this is not complete acceptance on every physical platform.
- Full preflight on the same product source: Linux 12,212 passed / 123 skipped; Windows 11,932 passed / 160 skipped. Skips are not passes.
- The final macOS DMG has verified signing, notarization, stapling, and scoped English/Chinese GUI results. Later records cover discovery from the previous stable release, actual upgrade, and persistence preservation. Controlled-provider scenarios do not establish real-model capabilities; upgrade does not prove uninterrupted shell/PTY/background-process survival.
- Windows installation/PATH/non-BMP/uninstall/GUI and Ubuntu GUI were not repeated for this release by user decision and remain `user_skipped/not_tested`. The released macOS build discovering a later future release is deferred and untested. Input C while approval is still pending remains `not_executed_not_waived`.
- Ten final SSH scenarios per Linux/Windows platform were accepted within their recorded scope. The original Windows 20-second shutdown failure remains. Later natural exit or absent endpoints do not prove all descendants, handles, and temporary directories were cleaned up.
- Full Draft downloads and post-publication anonymous downloads of all 12 assets were verified. Documentation maintenance does not change existing `cleanup_ready=false`, failure, or untested records.

## Records

- [Machine-readable current summary](../../specs/cli-agent-parity/CURRENT_STATUS.json)
- [CLI capabilities and scope](../../specs/cli-agent-parity/README.md)
- [Public documentation and private evidence policy](../evidence/README.md)

Update this page, its Chinese counterpart, and the machine summary together. Retired G09, Draft, and one-off platform acceptance logs are no longer tracked in public Git; originals remain in local backups. Untested scope and cleanup gates on this page remain effective.
