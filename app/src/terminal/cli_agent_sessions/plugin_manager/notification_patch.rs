//! 只替换已核对的通知脚本；插件注册、禁用和权限设置仍由原生 CLI 管理。

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
use std::io::Read as _;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, io};

#[cfg(all(windows, target_arch = "x86_64"))]
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::{NamedTempFile, TempDir};
#[cfg(not(target_family = "wasm"))]
use {command::r#async::Command, warpui::r#async::FutureExt as _};

use super::PluginInstallError;
#[cfg(not(target_family = "wasm"))]
use crate::util::path::resolve_executable_in_path;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum PatchKind {
    Claude,
    Codex,
}

impl PatchKind {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn marketplace(self) -> &'static str {
        match self {
            Self::Claude => "claude-code-warp",
            Self::Codex => "codex-warp",
        }
    }

    fn home_variable(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
        }
    }

    fn version(self) -> &'static str {
        match self {
            Self::Claude => "2.2.0",
            Self::Codex => "0.4.0",
        }
    }

    fn manifest(self) -> &'static str {
        match self {
            Self::Claude => ".claude-plugin/plugin.json",
            Self::Codex => ".codex-plugin/plugin.json",
        }
    }

    fn metadata(self) -> Metadata {
        let source = match self {
            Self::Claude => include_str!(
                "../../../../assets/bundled/cli-agent-plugins/claude/PATCH_METADATA.json"
            ),
            Self::Codex => include_str!(
                "../../../../assets/bundled/cli-agent-plugins/codex/PATCH_METADATA.json"
            ),
        };
        serde_json::from_str(source).expect("随附通知修补元数据必须有效")
    }

    fn files(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Claude => &[
                (
                    "scripts/build-payload.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/claude/scripts/build-payload.sh"
                    ),
                ),
                (
                    "scripts/on-stop.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/claude/scripts/on-stop.sh"
                    ),
                ),
                (
                    "scripts/should-use-structured.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/claude/scripts/should-use-structured.sh"
                    ),
                ),
                (
                    "hooks/hooks.json",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/claude/hooks/hooks.json"
                    ),
                ),
                (
                    "scripts/warp-notify.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/claude/scripts/warp-notify.sh"
                    ),
                ),
            ],
            Self::Codex => &[
                (
                    "scripts/on-prompt-submit.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/codex/scripts/on-prompt-submit.sh"
                    ),
                ),
                (
                    "scripts/build-payload.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/codex/scripts/build-payload.sh"
                    ),
                ),
                (
                    "scripts/on-stop.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/codex/scripts/on-stop.sh"
                    ),
                ),
                (
                    "hooks/hooks.json",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/codex/hooks/hooks.json"
                    ),
                ),
                (
                    "scripts/warp-notify.sh",
                    include_str!(
                        "../../../../assets/bundled/cli-agent-plugins/codex/scripts/warp-notify.sh"
                    ),
                ),
            ],
        }
    }
}

#[derive(Deserialize)]
struct Metadata {
    compatible_bases: Vec<CompatibleBase>,
    cli_contract_version: String,
    files: BTreeMap<String, FileHashes>,
}

#[derive(Deserialize)]
struct CompatibleBase {
    version: String,
    tree_sha256: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct FileHashes {
    upstream_sha256: String,
    replacement_sha256: String,
    #[serde(default)]
    previous_replacement_sha256: Vec<String>,
}

#[derive(Debug)]
struct Installation {
    path: PathBuf,
    version: String,
}

fn invalid_state() -> io::Error {
    io::Error::other("插件来源、版本、路径或摘要不符合已验证的通知修补契约")
}

pub(super) fn unsupported() -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-patch-incompatible"),
        log: String::new(),
    }
}

pub(super) fn modified() -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-patch-modified"),
        log: String::new(),
    }
}

fn operation_error(error: impl std::fmt::Display, log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-patch-failed"),
        log: format!("{log}\n{error}"),
    }
}

pub(super) fn auto_install_supported() -> bool {
    // Claude 的 Windows 原生通知链尚未验证，继续保留原有 Unix 门槛。
    cfg!(unix)
}

pub(super) fn auto_install_supported_for(kind: PatchKind) -> bool {
    match kind {
        PatchKind::Claude => auto_install_supported(),
        // 此处仅提供入口；写入前仍须验证固定 CLI 以及当前 PATH 的实际依赖。
        PatchKind::Codex => cfg!(unix) || cfg!(all(windows, target_arch = "x86_64")),
    }
}

#[cfg(any(test, all(windows, target_arch = "x86_64")))]
const WINDOWS_CODEX_BYTES: u64 = 298_668_336;
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
const WINDOWS_CODEX_SHA256: &str =
    "935a1911ed2556e4ffcec995f4886ac2ac425863ba26fed264df62e30272ad9d";
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
const WINDOWS_DEPENDENCY_PROBE: &str = include_str!(
    "../../../../assets/bundled/cli-agent-plugins/codex/windows-install-preflight.ps1"
);
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
fn windows_dependency_receipt_matches(stdout: &[u8], stderr: &[u8]) -> bool {
    stdout == b"infinishell-codex-windows-dependencies-v1\r\n" && stderr.is_empty()
}

#[cfg(any(test, all(windows, target_arch = "x86_64")))]
fn verify_windows_codex_binary(path: &Path) -> io::Result<()> {
    // 当前原生证据仅覆盖官方 x64 exe；不把同版本 shim 或其他构建当成已验证入口。
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(invalid_state());
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() != WINDOWS_CODEX_BYTES {
        return Err(invalid_state());
    }
    let mut file = file.take(WINDOWS_CODEX_BYTES + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != WINDOWS_CODEX_SHA256 {
        return Err(invalid_state());
    }
    Ok(())
}

pub(super) struct VerifiedRuntime {
    executable: PathBuf,
    kind: PatchKind,
    home: PathBuf,
    search_path: OsString,
    isolated_home: bool,
}

impl VerifiedRuntime {
    /// 暂存安装只改变显式 CLI HOME；原生输出须再校验，不能直接当真实配置已安装。
    pub(super) fn with_home(&self, home: &Path) -> Self {
        Self {
            executable: self.executable.clone(),
            kind: self.kind,
            home: home.to_owned(),
            search_path: self.search_path.clone(),
            isolated_home: true,
        }
    }

    pub(super) async fn probe(
        kind: PatchKind,
        home: &Path,
        path_env: Option<&str>,
        log: &mut String,
    ) -> Result<Self, PluginInstallError> {
        #[cfg(not(target_family = "wasm"))]
        {
            if !auto_install_supported_for(kind) {
                return Err(unsupported());
            }
            let search_path = path_env
                .map(OsString::from)
                .unwrap_or_else(|| env::var_os("PATH").unwrap_or_default());
            let executable = resolve_executable_in_path(kind.name(), &search_path)
                .ok_or_else(unsupported)?
                .into_owned();
            let runtime = Self {
                executable,
                kind,
                home: home.to_owned(),
                search_path,
                isolated_home: false,
            };
            #[cfg(all(windows, target_arch = "x86_64"))]
            verify_windows_codex_binary(&runtime.executable).map_err(|_| unsupported())?;
            let version = runtime
                .run_with_timeout(&["--version"], Duration::from_secs(5), log)
                .await?;
            if !version_matches(kind, &version) {
                return Err(unsupported());
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            {
                runtime.probe_windows_dependencies(log).await?;
                Ok(runtime)
            }
            #[cfg(not(all(windows, target_arch = "x86_64")))]
            {
                // 与后续插件命令使用完全相同的 PATH，不借 shell 启动文件重定向用户配置目录。
                let mut runtime = runtime;
                let cli = runtime.executable.clone();
                for dependency in ["bash", "jq"] {
                    runtime.executable =
                        resolve_executable_in_path(dependency, &runtime.search_path)
                            .ok_or_else(unsupported)?
                            .into_owned();
                    runtime
                        .run_with_timeout(&["--version"], Duration::from_secs(5), log)
                        .await?;
                }
                runtime.executable = cli;
                Ok(runtime)
            }
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (kind, home, path_env, log);
            Err(unsupported())
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    async fn probe_windows_dependencies(&self, log: &mut String) -> Result<(), PluginInstallError> {
        let failed = || PluginInstallError {
            message: crate::t!("cli-agent-plugin-codex-windows-dependencies"),
            // 只记录固定诊断，不把 PowerShell 返回的环境值或路径写入日志。
            log: format!("{log}\nWindows Codex 通知依赖预检未通过。"),
        };
        let system_root = env::var_os("SystemRoot").ok_or_else(|| failed())?;
        let powershell =
            PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        if !powershell.is_absolute() || !powershell.is_file() {
            return Err(failed());
        }
        let source = WINDOWS_DEPENDENCY_PROBE.replace("\r\n", "\n");
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            source
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let output = Command::new(powershell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encoded,
            ])
            .env("PATH", &self.search_path)
            .kill_on_drop(true)
            .output()
            .with_timeout(Duration::from_secs(15))
            .await
            .map_err(|_| failed())?
            .map_err(|_| failed())?;
        if !output.status.success()
            || !windows_dependency_receipt_matches(&output.stdout, &output.stderr)
        {
            return Err(failed());
        }
        log.push_str("Windows Codex 通知依赖预检通过。\n");
        Ok(())
    }

    pub(super) async fn run(
        &self,
        args: &[&str],
        log: &mut String,
    ) -> Result<(), PluginInstallError> {
        self.run_with_timeout(args, Duration::from_secs(60), log)
            .await
            .map(|_| ())
    }

    async fn run_with_timeout(
        &self,
        args: &[&str],
        timeout: Duration,
        log: &mut String,
    ) -> Result<String, PluginInstallError> {
        #[cfg(not(target_family = "wasm"))]
        {
            let mut command = Command::new(&self.executable);
            command
                .args(args)
                .env("PATH", &self.search_path)
                .env(self.kind.home_variable(), &self.home)
                .kill_on_drop(true);
            if self.kind == PatchKind::Claude && self.isolated_home {
                // 暂存原生安装不能通过 HOME 回退读取或改写用户真实配置。
                command
                    .env("HOME", &self.home)
                    .env("USERPROFILE", &self.home)
                    .current_dir(&self.home);
            }
            let output = command
                .output()
                .with_timeout(timeout)
                .await
                .map_err(|error| operation_error(error, log))?
                .map_err(|error| operation_error(error, log))?;
            log.push_str(&format!("$ {} {}\n", self.kind.name(), args.join(" ")));
            for stream in [&output.stdout, &output.stderr] {
                log.push_str(&String::from_utf8_lossy(&stream[..stream.len().min(65536)]));
                log.push('\n');
            }
            if !output.status.success() {
                return Err(operation_error(output.status, log));
            }
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (args, timeout, log);
            Err(unsupported())
        }
    }
}

fn version_matches(kind: PatchKind, output: &str) -> bool {
    let tokens = output.split_whitespace().collect::<Vec<_>>();
    let version = match (kind, tokens.as_slice()) {
        (PatchKind::Claude, [version, "(Claude", "Code)"])
        | (PatchKind::Codex, ["codex-cli", version]) => *version,
        // 其他输出形状不能证明实际 CLI 身份。
        _ => return false,
    };
    version == kind.metadata().cli_contract_version
        // 新版仅补 macOS 原生插件安装及 hook 合同，不扩大其他平台的验证范围。
        || (cfg!(target_os = "macos")
            && matches!(
                (kind, version),
                (PatchKind::Claude, "2.1.280") | (PatchKind::Codex, "0.156.1")
            ))
}

fn installed(home: &Path, kind: PatchKind) -> io::Result<Option<Installation>> {
    let base = home
        .join("plugins/cache")
        .join(kind.marketplace())
        .join("warp");
    let candidate = match kind {
        PatchKind::Claude => {
            let registry = match fs::read(home.join("plugins/installed_plugins.json")) {
                Ok(contents) => serde_json::from_slice::<Value>(&contents)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            let Some(value) = registry
                .get("plugins")
                .and_then(|plugins| plugins.get("warp@claude-code-warp"))
            else {
                return Ok(None);
            };
            let entries = value.as_array().ok_or_else(invalid_state)?;
            if entries.is_empty() {
                return Ok(None);
            }
            if entries.len() != 1 || entries[0].get("scope").and_then(Value::as_str) != Some("user")
            {
                return Err(invalid_state());
            }
            let entry = &entries[0];
            Installation {
                path: PathBuf::from(
                    entry
                        .get("installPath")
                        .and_then(Value::as_str)
                        .ok_or_else(invalid_state)?,
                ),
                version: entry
                    .get("version")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid_state)?
                    .to_owned(),
            }
        }
        PatchKind::Codex => {
            let entries = match fs::read_dir(&base) {
                Ok(entries) => entries.collect::<io::Result<Vec<_>>>()?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            // 无可靠的活跃缓存指针时拒绝多个版本，不能猜最新目录就是当前运行版本。
            if entries.is_empty() {
                return Ok(None);
            }
            if entries.len() != 1 {
                return Err(invalid_state());
            }
            let path = entries[0].path();
            let value: Value = serde_json::from_slice(&fs::read(path.join(kind.manifest()))?)?;
            Installation {
                path,
                version: value
                    .get("version")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid_state)?
                    .to_owned(),
            }
        }
    };
    if fs::symlink_metadata(&candidate.path)?
        .file_type()
        .is_symlink()
        || candidate
            .path
            .parent()
            .ok_or_else(invalid_state)?
            .canonicalize()?
            != base.canonicalize()?
        || !candidate
            .path
            .canonicalize()?
            .starts_with(home.canonicalize()?)
        || candidate.path.file_name().and_then(|name| name.to_str())
            != Some(candidate.version.as_str())
    {
        return Err(invalid_state());
    }
    let manifest = checked_file(&candidate.path, kind.manifest())?;
    let value: Value = serde_json::from_slice(&fs::read(manifest)?)?;
    if value.get("name").and_then(Value::as_str) != Some("warp")
        || value.get("version").and_then(Value::as_str) != Some(candidate.version.as_str())
    {
        return Err(invalid_state());
    }
    Ok(Some(candidate))
}

fn checked_file(root: &Path, relative: &str) -> io::Result<PathBuf> {
    let mut path = root.to_owned();
    for part in Path::new(relative).components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(invalid_state());
        }
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(invalid_state());
        }
    }
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(invalid_state());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(invalid_state());
        }
    }
    Ok(path)
}

fn sha256(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn validate_tree(installation: &Installation, kind: PatchKind) -> io::Result<()> {
    let metadata = kind.metadata();
    let base = metadata
        .compatible_bases
        .iter()
        .find(|base| base.version == installation.version)
        .ok_or_else(invalid_state)?;
    let mut pending = vec![installation.path.clone()];
    let mut observed = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(invalid_state());
            }
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(&installation.path)
                .map_err(|_| invalid_state())?
                .components()
                .map(|component| match component {
                    Component::Normal(name) => name.to_str().ok_or_else(invalid_state),
                    Component::Prefix(_)
                    | Component::RootDir
                    | Component::CurDir
                    | Component::ParentDir => Err(invalid_state()),
                })
                .collect::<io::Result<Vec<_>>>()?
                .join("/");
            // 清单使用正斜杠；按平台路径组件转换，保留 Unix 文件名中的字面反斜杠。
            let original = base.tree_sha256.get(&relative).ok_or_else(invalid_state)?;
            let digest = sha256(&fs::read(checked_file(&installation.path, &relative)?)?);
            let replacement = metadata
                .files
                .get(&relative)
                .map(|file| &file.replacement_sha256);
            let prior = metadata
                .files
                .get(&relative)
                .is_some_and(|file| file.previous_replacement_sha256.contains(&digest));
            if &digest != original && replacement != Some(&digest) && !prior {
                return Err(invalid_state());
            }
            observed += 1;
        }
    }
    if observed != base.tree_sha256.len() {
        return Err(invalid_state());
    }
    Ok(())
}

/// 原生更新前核对整个受测插件目录，避免更新命令覆盖自定义脚本或其他新增文件。
pub(super) fn preflight(home: &Path, kind: PatchKind) -> Result<bool, PluginInstallError> {
    invalidate(home, kind);
    let installation = installed(home, kind).map_err(|_| modified())?;
    if let Some(installation) = installation {
        if validate_tree(&installation, kind).is_err()
            && !(kind == PatchKind::Codex
                && installation.version == kind.version()
                && super::codex_source::is_previous_notification_cache(&installation.path))
        {
            return Err(modified());
        }
        return Ok(installation.version == kind.version());
    }
    Ok(false)
}

/// 发布注册指针前核对尚未激活的 Claude 缓存；不依赖真实配置中的安装记录。
pub(super) fn verify_staged_claude_cache(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(invalid_state());
    }
    let installation = Installation {
        path: path.to_owned(),
        version: PatchKind::Claude.version().to_owned(),
    };
    let manifest: Value = serde_json::from_slice(&fs::read(checked_file(
        path,
        PatchKind::Claude.manifest(),
    )?)?)?;
    if manifest.get("name").and_then(Value::as_str) != Some("warp")
        || manifest.get("version").and_then(Value::as_str) != Some(PatchKind::Claude.version())
        || !ready(&installation, PatchKind::Claude)?
    {
        return Err(invalid_state());
    }
    validate_tree(&installation, PatchKind::Claude)
}

pub(super) fn apply(home: &Path, kind: PatchKind, log: &str) -> Result<(), PluginInstallError> {
    invalidate(home, kind);
    // Windows Codex 只经运行时预检后的完整来源事务，不开放仅修缓存的旁路。
    if !auto_install_supported() {
        return Err(unsupported());
    }
    let installation = installed(home, kind)
        .map_err(|_| modified())?
        .ok_or_else(modified)?;
    if installation.version != kind.version() {
        return Err(unsupported());
    }
    validate_tree(&installation, kind).map_err(|_| modified())?;
    apply_files(
        &installation.path,
        &home.join("plugins"),
        kind.files(),
        &kind.metadata().files,
        |_, _| Ok(()),
    )
    .map_err(|error| operation_error(error, log))?;
    if !ready(&installation, kind).map_err(|error| operation_error(error, log))? {
        return Err(operation_error("替换后摘要验证失败", log));
    }
    Ok(())
}

fn patch_staging(root: &Path, parent: &Path) -> io::Result<TempDir> {
    let parent = parent.canonicalize()?;
    if parent.starts_with(root.canonicalize()?) {
        return Err(invalid_state());
    }
    // 唯一私有目录位于活动树外；进程中断留下的暂存不能污染严格插件清单。
    tempfile::Builder::new()
        .prefix(".infinishell-notification-patch-")
        .tempdir_in(parent)
}

fn stage_replacement(
    staging: &Path,
    contents: &[u8],
    permissions: fs::Permissions,
) -> io::Result<NamedTempFile> {
    let mut temporary = NamedTempFile::new_in(staging)?;
    temporary.as_file().set_permissions(permissions)?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    Ok(temporary)
}

fn replace(
    staging: &Path,
    path: &Path,
    contents: &[u8],
    permissions: fs::Permissions,
) -> io::Result<()> {
    stage_replacement(staging, contents, permissions)?
        .persist(path)
        .map_err(|error| error.error)?;
    Ok(())
}

fn apply_files(
    root: &Path,
    staging_parent: &Path,
    files: &[(&str, &str)],
    hashes: &BTreeMap<String, FileHashes>,
    mut before_replace: impl FnMut(usize, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let mut originals = Vec::new();
    // 先读完全部文件并核对原始与随附摘要，任何自定义文件都在写入前拒绝。
    for (relative, replacement) in files {
        let path = checked_file(root, relative)?;
        let original = fs::read(&path)?;
        let expected = hashes.get(*relative).ok_or_else(invalid_state)?;
        let digest = sha256(&original);
        if (digest != expected.upstream_sha256
            && digest != expected.replacement_sha256
            && !expected.previous_replacement_sha256.contains(&digest))
            || sha256(replacement.as_bytes()) != expected.replacement_sha256
        {
            return Err(invalid_state());
        }
        originals.push((path.clone(), original, fs::metadata(path)?.permissions()));
    }
    let staging = patch_staging(root, staging_parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let device = fs::metadata(staging.path())?.dev();
        // 在任何替换前拒绝跨挂载点，不能退化成非原子的复制。
        for (path, _, _) in &originals {
            if fs::metadata(path)?.dev() != device {
                return Err(invalid_state());
            }
        }
    }
    let mut replaced = 0;
    let outcome = (|| {
        for (index, ((_, replacement), (path, original, permissions))) in
            files.iter().zip(&originals).enumerate()
        {
            before_replace(index, path)?;
            // 检测预检后的并发编辑，不用旧备份覆盖新内容。
            if fs::read(checked_file(root, files[index].0)?)? != *original {
                return Err(invalid_state());
            }
            replace(
                staging.path(),
                path,
                replacement.as_bytes(),
                permissions.clone(),
            )?;
            replaced += 1;
        }
        for ((relative, replacement), _) in files.iter().zip(&originals) {
            if fs::read(checked_file(root, relative)?)? != replacement.as_bytes() {
                return Err(invalid_state());
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        let mut rollback_failed = false;
        for (index, (path, original, permissions)) in
            originals.iter().take(replaced).enumerate().rev()
        {
            if checked_file(root, files[index].0)
                .and_then(fs::read)
                .ok()
                .as_deref()
                != Some(files[index].1.as_bytes())
                || replace(staging.path(), path, original, permissions.clone()).is_err()
            {
                rollback_failed = true;
            }
        }
        return if rollback_failed {
            Err(io::Error::other(format!(
                "{error}; 部分受控文件恢复失败，请重新校验插件，未覆盖并发编辑"
            )))
        } else {
            Err(error)
        };
    }
    Ok(())
}

fn ready(installation: &Installation, kind: PatchKind) -> io::Result<bool> {
    if installation.version != kind.version() {
        return Ok(false);
    }
    let metadata = kind.metadata();
    for (relative, _) in kind.files() {
        if sha256(&fs::read(checked_file(&installation.path, relative)?)?)
            != metadata.files[*relative].replacement_sha256
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 信任显示需要整个固定版本目录受控，不能仅凭被替换的几个文件推断可信来源。
pub(super) fn full_tree_is_applied(home: &Path, kind: PatchKind) -> bool {
    installed(home, kind)
        .ok()
        .flatten()
        .is_some_and(|installation| {
            validate_tree(&installation, kind).is_ok()
                && ready(&installation, kind).unwrap_or(false)
        })
}

type FileStamp = (PathBuf, u64, Option<SystemTime>);
struct CacheEntry {
    checked_at: Instant,
    stamps: Vec<FileStamp>,
    ready: bool,
}
static CACHE: LazyLock<Mutex<HashMap<(PathBuf, PatchKind), CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn invalidate(home: &Path, kind: PatchKind) {
    CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&(home.to_owned(), kind));
}

/// UI 查询只复用短期缓存；到期先比较文件元数据，内容变化时才重新计算摘要。
pub(super) fn is_applied(home: &Path, kind: PatchKind) -> bool {
    let key = (home.to_owned(), kind);
    let previous = {
        let cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = cache.get(&key) {
            if entry.checked_at.elapsed() < Duration::from_secs(1) {
                return entry.ready;
            }
            Some((entry.stamps.clone(), entry.ready))
        } else {
            None
        }
    };
    let installation = installed(home, kind).ok().flatten();
    let stamps = installation
        .as_ref()
        .and_then(|installation| {
            kind.files()
                .iter()
                .map(|(relative, _)| *relative)
                .chain([kind.manifest()])
                .map(|relative| {
                    let path = checked_file(&installation.path, relative).ok()?;
                    let metadata = fs::metadata(&path).ok()?;
                    Some((path, metadata.len(), metadata.modified().ok()))
                })
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default();
    let applied = if !stamps.is_empty() && previous.as_ref().is_some_and(|(old, _)| old == &stamps)
    {
        previous.is_some_and(|(_, applied)| applied)
    } else {
        installation.is_some_and(|installation| ready(&installation, kind).unwrap_or(false))
    };
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    if cache.len() > 32 {
        cache.clear();
    }
    cache.insert(
        key,
        CacheEntry {
            checked_at: Instant::now(),
            stamps,
            ready: applied,
        },
    );
    applied
}

#[cfg(test)]
#[path = "notification_patch_tests.rs"]
mod tests;
