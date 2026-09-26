//! 固定 Codex npm 公共入口探针；Node、wrapper、平台包先冻结为独立执行闭包。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::{ExpectedFileIdentity, Manifest};

#[cfg(target_os = "macos")]
#[path = "managed_process_npm_probe_macos.rs"]
mod macos;

const PACKAGE_MANIFEST: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0156_package_manifest.json");

fn invalid() -> io::Error {
    io::Error::other("Codex npm 的 Node、launcher 或平台闭包不匹配")
}

fn platform() -> io::Result<(&'static str, &'static str, &'static str)> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok(("linux-x64", "linux-x64", "x86_64-unknown-linux-musl"))
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok(("darwin-arm64", "macos-arm64", "aarch64-apple-darwin"))
    } else {
        Err(invalid())
    }
}

fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        && path
            .to_str()
            .is_some_and(|path| !path.chars().any(char::is_control))
}

pub(super) fn entry_root(node: &Path, arguments: &[OsString]) -> io::Result<PathBuf> {
    platform()?;
    if !plain_absolute(node)
        || node.file_name() != Some(OsStr::new("node"))
        || arguments.len() != 2
        || arguments[1] != "--version"
    {
        return Err(invalid());
    }
    let entry = Path::new(&arguments[0]);
    let root = entry.parent().and_then(Path::parent).ok_or_else(invalid)?;
    if !plain_absolute(entry)
        || !entry.ends_with("bin/codex.js")
        || root
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|name| name.strip_prefix(".infinishell-npm-"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_none_or(|id| id.is_nil())
    {
        return Err(invalid());
    }
    Ok(root.to_owned())
}

pub(super) fn capture_node(node: &ExpectedFileIdentity) -> io::Result<Vec<ExpectedFileIdentity>> {
    platform()?;
    if node.path != node.canonical_path || node.path.file_name() != Some(OsStr::new("node")) {
        return Err(invalid());
    }
    #[cfg(target_os = "macos")]
    {
        macos::capture(node)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(vec![node.clone()])
    }
}

pub(super) fn validate_contract(
    node: &Path,
    arguments: &[OsString],
    files: &[ExpectedFileIdentity],
) -> io::Result<()> {
    let root = entry_root(node, arguments)?;
    super::validate_expected_files_contract(node, files)?;
    if files.first().is_none_or(|file| file.path != node) || files.len() > 192 {
        return Err(invalid());
    }
    let (target, package, triple) = platform()?;
    let metadata: serde_json::Value =
        serde_json::from_slice(PACKAGE_MANIFEST).map_err(io::Error::other)?;
    let native_files: BTreeMap<PathBuf, (u64, String, u32)> =
        serde_json::from_value(metadata["packages"][package]["files"].clone())
            .map_err(io::Error::other)?;
    let dependency = PathBuf::from(format!("node_modules/@openai/codex-{target}"));
    let vendor = dependency.join("vendor").join(triple);
    let mut required = BTreeMap::from([
        (
            PathBuf::from("bin/codex.js"),
            (
                8790,
                "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70".to_owned(),
            ),
        ),
        (
            PathBuf::from("package.json"),
            (
                1082,
                "c3f16464dca0fe1269b17d02fe0997d1ca3a241c3a61da3d89ec13def0a66c6e".to_owned(),
            ),
        ),
        (
            PathBuf::from("README.md"),
            (
                3334,
                "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7".to_owned(),
            ),
        ),
    ]);
    required.extend(
        native_files
            .into_iter()
            .map(|(path, (length, digest, _mode))| (vendor.join(path), (length, digest))),
    );
    let mut platform_metadata = BTreeSet::from([
        dependency.join("package.json"),
        dependency.join("README.md"),
    ]);
    for file in files.iter().skip(1) {
        if file.path != file.canonical_path || file.file_id.is_none() || !plain_absolute(&file.path)
        {
            return Err(invalid());
        }
        match file.path.strip_prefix(&root) {
            Ok(path) => {
                if let Some((length, digest)) = required.remove(path) {
                    if file.size != length || file.sha256 != digest {
                        return Err(invalid());
                    }
                } else if !platform_metadata.remove(path) || file.size == 0 || file.size > 64 * 1024
                {
                    return Err(invalid());
                }
            }
            Err(_) => {
                #[cfg(target_os = "macos")]
                if !macos::homebrew_library_path(&file.path) {
                    return Err(invalid());
                }
                #[cfg(not(target_os = "macos"))]
                return Err(invalid());
            }
        }
    }
    if !required.is_empty() || !platform_metadata.is_empty() {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn verify_node_dependencies(
    node: &Path,
    arguments: &[OsString],
    files: &[ExpectedFileIdentity],
) -> io::Result<()> {
    validate_contract(node, arguments, files)?;
    let root = entry_root(node, arguments)?;
    let dependencies: Vec<_> = files
        .iter()
        .filter(|file| !file.path.starts_with(&root))
        .cloned()
        .collect();
    super::verify_expected_files(&dependencies)?;
    if capture_node(files.first().ok_or_else(invalid)?)? != dependencies {
        return Err(invalid());
    }
    Ok(())
}

fn create_directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)
}

fn copy_bound(
    expected: &ExpectedFileIdentity,
    destination: &Path,
    executable: bool,
) -> io::Result<()> {
    let mut source = super::open_expected_file(&expected.path)?;
    let before = source.metadata()?;
    if expected.path != expected.canonical_path
        || expected.file_id.is_none()
        || expected
            .file_id
            .is_some_and(|id| id.volume != before.dev() || id.index != before.ino())
        || !before.is_file()
        || before.nlink() != 1
        || before.mode() & 0o7022 != 0
        || (before.uid() != 0 && before.uid() != unsafe { libc::geteuid() })
        || before.len() != expected.size
        || expected.size > 512 * 1024 * 1024
    {
        return Err(invalid());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(destination)?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > expected.size {
            return Err(invalid());
        }
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    let after = source.metadata()?;
    if bytes != expected.size
        || format!("{:x}", digest.finalize()) != expected.sha256
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mode() != after.mode()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
    {
        return Err(invalid());
    }
    output.set_permissions(fs::Permissions::from_mode(if executable {
        0o500
    } else {
        0o400
    }))?;
    output.sync_all()
}

// 只读 mode 不能约束同 UID 的解释器；Linux 还需内核拒绝改写、改名和截断候选。
// ABI 3 才具备 TRUNCATE；缺失时拒绝执行，不能降级到可改写的候选目录。
#[cfg(target_os = "linux")]
pub(super) fn protect_linux_candidate(root: &Path) -> io::Result<()> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};

    #[repr(C)]
    struct Ruleset {
        handled_access_fs: u64,
    }
    #[repr(C, packed)]
    struct PathBeneath {
        allowed_access: u64,
        parent_fd: i32,
    }
    const WRITE_FILE: u64 = 1 << 1;
    const TRUNCATE: u64 = 1 << 14;
    const MUTATIONS: u64 = WRITE_FILE | (((1 << 15) - 1) & !((1 << 4) - 1));
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0usize,
            1u32,
        )
    };
    if abi < 3 {
        return Err(io::Error::other(
            "Codex npm 探针需要 Landlock ABI 3 的只读候选边界",
        ));
    }
    let attribute = Ruleset {
        handled_access_fs: MUTATIONS,
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attribute,
            std::mem::size_of::<Ruleset>(),
            0u32,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let ruleset = unsafe { File::from_raw_fd(fd as i32) };
    for (path, rights) in ["home", "config", "cache", "data", "tmp"]
        .into_iter()
        .map(|name| (root.join(name), MUTATIONS))
        .chain(std::iter::once((
            PathBuf::from("/dev/null"),
            WRITE_FILE | TRUNCATE,
        )))
    {
        let path = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        let attribute = PathBeneath {
            allowed_access: rights,
            parent_fd: path.as_raw_fd(),
        };
        if unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset.as_raw_fd(),
                1u32,
                &attribute,
                0u32,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0u32) }
            != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn execute(manifest: &Manifest) -> io::Result<()> {
    validate_contract(
        &manifest.executable,
        &manifest.arguments,
        &manifest.expected_files,
    )?;
    super::verify_expected_files(&manifest.expected_files)?;
    let original_root = entry_root(&manifest.executable, &manifest.arguments)?;
    let node_files: Vec<_> = manifest
        .expected_files
        .iter()
        .filter(|file| !file.path.starts_with(&original_root))
        .cloned()
        .collect();
    if capture_node(&manifest.expected_files[0])? != node_files {
        return Err(invalid());
    }
    let package = manifest.cwd.join("package");
    create_directory(&package)?;
    let mut directories = BTreeSet::from([package.clone()]);
    for expected in manifest
        .expected_files
        .iter()
        .filter(|file| file.path.starts_with(&original_root))
    {
        let relative = expected
            .path
            .strip_prefix(&original_root)
            .map_err(|_| invalid())?;
        let destination = package.join(relative);
        let mut parent = package.clone();
        for component in relative.parent().ok_or_else(invalid)?.components() {
            let Component::Normal(name) = component else {
                return Err(invalid());
            };
            parent.push(name);
            if directories.insert(parent.clone()) {
                create_directory(&parent)?;
            }
        }
        let executable = fs::symlink_metadata(&expected.path)?.mode() & 0o111 != 0;
        copy_bound(expected, &destination, executable)?;
    }
    for directory in directories.iter().rev() {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o500))?;
        File::open(directory)?.sync_all()?;
    }
    let (target, _, triple) = platform()?;
    let native = package.join(format!(
        "node_modules/@openai/codex-{target}/vendor/{triple}/bin/codex"
    ));
    let expected_native = ExpectedFileIdentity::capture(&native)?;
    let arguments = [
        package.join("bin/codex.js").into_os_string(),
        "--version".into(),
    ];
    let mut environment = super::version_probe::resolved_environment(&manifest.cwd)?;
    environment.push((
        "CODEX_HOME".into(),
        manifest.cwd.join("config").into_os_string(),
    ));
    environment.push(("NODE_DISABLE_COMPILE_CACHE".into(), "1".into()));
    #[cfg(target_os = "linux")]
    {
        // 原生依赖也要接受固定 ELF/系统加载器校验，不能借 Node 跳过镜像检查。
        let _native = super::atomic_linux::prepare(&expected_native)?;
        let node = super::atomic_linux::prepare(&manifest.expected_files[0])?;
        super::enter_atomic_cwd(manifest)?;
        protect_linux_candidate(&manifest.cwd)?;
        super::version_probe::deny_network()?;
        return Err(super::atomic_linux::execute(
            &node,
            &manifest.executable,
            &arguments,
            &environment,
        ));
    }
    #[cfg(target_os = "macos")]
    {
        // 这里不调用原单文件接口以放行 Homebrew dylib；独立闭包负责复制和只读隔离。
        macos::verify_signature(&expected_native.path)?;
        let node = macos::snapshot(&manifest.cwd, &node_files)?;
        environment.push((
            "DYLD_LIBRARY_PATH".into(),
            manifest.cwd.join("node-runtime/lib").into_os_string(),
        ));
        environment.push(("DYLD_FALLBACK_LIBRARY_PATH".into(), "/usr/lib".into()));
        super::enter_atomic_cwd(manifest)?;
        macos::isolate(&manifest.cwd, &node, &native)?;
        use command::unix::CommandExt as _;
        let mut command = command::blocking::Command::new(node);
        command.args(arguments).env_clear().envs(environment);
        return Err(command.exec());
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Err(invalid())
}
