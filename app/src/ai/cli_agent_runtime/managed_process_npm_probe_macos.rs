//! Homebrew Node 的实际 Mach-O 依赖图；只复制绑定字节，不修改 install name 或重签名。

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};

use command::blocking::Command;

use super::{ExpectedFileIdentity, copy_bound, create_directory, invalid};

fn system_library(value: &str) -> bool {
    (value.starts_with("/usr/lib/") || value.starts_with("/System/Library/"))
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

pub(super) fn homebrew_library_path(path: &Path) -> bool {
    (path.starts_with("/opt/homebrew/Cellar") || path.starts_with("/usr/local/Cellar"))
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.ends_with(".dylib"))
}

fn report(path: &Path, argument: &str) -> io::Result<String> {
    let output = Command::new("/usr/bin/otool")
        .arg(argument)
        .arg(path)
        .env_clear()
        .output()?;
    if !output.status.success()
        || !output.stderr.is_empty()
        || output.stdout.len() > 2 * 1024 * 1024
    {
        return Err(invalid());
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

pub(super) fn verify_signature(path: &Path) -> io::Result<()> {
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--all-architectures", "--"])
        .arg(path)
        .env_clear()
        .output()?;
    if !output.status.success() {
        return Err(invalid());
    }
    Ok(())
}

fn expand(value: &str, loader: &Path, executable: &Path) -> io::Result<PathBuf> {
    if value == "@loader_path" {
        Ok(loader.parent().ok_or_else(invalid)?.to_owned())
    } else if value == "@executable_path" {
        Ok(executable.parent().ok_or_else(invalid)?.to_owned())
    } else if let Some(relative) = value.strip_prefix("@loader_path/") {
        Ok(loader.parent().ok_or_else(invalid)?.join(relative))
    } else if let Some(relative) = value.strip_prefix("@executable_path/") {
        Ok(executable.parent().ok_or_else(invalid)?.join(relative))
    } else if value.starts_with('/') {
        Ok(PathBuf::from(value))
    } else {
        Err(invalid())
    }
}

fn rpaths(path: &Path, executable: &Path) -> io::Result<Vec<PathBuf>> {
    let output = report(path, "-l")?;
    let mut paths = Vec::new();
    let mut active = false;
    for line in output.lines().map(str::trim) {
        if line.starts_with("cmd ") {
            active = line == "cmd LC_RPATH";
        }
        if active && let Some(value) = line.strip_prefix("path ") {
            let (value, _) = value.rsplit_once(" (offset ").ok_or_else(invalid)?;
            paths.push(expand(value, path, executable)?);
            active = false;
        }
    }
    Ok(paths)
}

pub(super) fn capture(node: &ExpectedFileIdentity) -> io::Result<Vec<ExpectedFileIdentity>> {
    graph(node).map(|(files, _aliases)| files)
}

fn graph(
    node: &ExpectedFileIdentity,
) -> io::Result<(Vec<ExpectedFileIdentity>, BTreeMap<OsString, PathBuf>)> {
    let executable = &node.path;
    let mut pending = VecDeque::from([(executable.clone(), Vec::<PathBuf>::new())]);
    let mut files = BTreeMap::new();
    let mut names = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    while let Some((path, inherited)) = pending.pop_front() {
        if files.contains_key(&path) {
            continue;
        }
        if files.len() >= 128 {
            return Err(invalid());
        }
        let identity = ExpectedFileIdentity::capture(&path)?;
        if identity.path != identity.canonical_path {
            return Err(invalid());
        }
        if path == *executable {
            if identity != *node {
                return Err(invalid());
            }
        } else if !homebrew_library_path(&path) {
            return Err(invalid());
        }
        let name = path.file_name().ok_or_else(invalid)?.to_owned();
        if names
            .insert(name, path.clone())
            .is_some_and(|previous| previous != path)
        {
            return Err(invalid());
        }
        verify_signature(&path)?;
        let mut search = rpaths(&path, executable)?;
        search.extend(inherited);
        let output = report(&path, "-L")?;
        for line in output.lines().skip(1).map(str::trim) {
            let (dependency, _) = line
                .split_once(" (compatibility version ")
                .ok_or_else(invalid)?;
            if system_library(dependency) {
                continue;
            }
            let candidates: Vec<_> = if let Some(relative) = dependency.strip_prefix("@rpath/") {
                search.iter().map(|root| root.join(relative)).collect()
            } else {
                vec![expand(dependency, &path, executable)?]
            };
            let candidates = candidates
                .into_iter()
                .filter_map(|path| path.canonicalize().ok())
                .collect::<BTreeSet<_>>();
            // 多个真实解析目标不能由宿主猜测 dyld 的运行时顺序。
            if candidates.len() != 1 {
                return Err(invalid());
            }
            let candidate = candidates.into_iter().next().ok_or_else(invalid)?;
            let alias = Path::new(dependency)
                .file_name()
                .ok_or_else(invalid)?
                .to_owned();
            if !alias.to_str().is_some_and(|name| name.ends_with(".dylib"))
                || aliases
                    .insert(alias, candidate.clone())
                    .is_some_and(|previous| previous != candidate)
                || aliases.len() > 256
            {
                return Err(invalid());
            }
            if candidate != path {
                pending.push_back((candidate, search.clone()));
            }
        }
        if ExpectedFileIdentity::capture(&path)? != identity {
            return Err(invalid());
        }
        files.insert(path, identity);
    }
    let main = files.remove(executable).ok_or_else(invalid)?;
    Ok((
        std::iter::once(main).chain(files.into_values()).collect(),
        aliases,
    ))
}

pub(super) fn snapshot(root: &Path, files: &[ExpectedFileIdentity]) -> io::Result<PathBuf> {
    let runtime = root.join("node-runtime");
    create_directory(&runtime)?;
    create_directory(&runtime.join("bin"))?;
    create_directory(&runtime.join("lib"))?;
    let main = files.first().ok_or_else(invalid)?;
    let (current, aliases) = graph(main)?;
    if current.as_slice() != files {
        return Err(invalid());
    }
    copy_bound(main, &runtime.join("bin/node"), true)?;
    verify_signature(&runtime.join("bin/node"))?;
    let mut bytes = main.size;
    for (alias, source) in aliases {
        let file = files
            .iter()
            .find(|file| file.path == source)
            .ok_or_else(invalid)?;
        bytes = bytes.checked_add(file.size).ok_or_else(invalid)?;
        if bytes > 2 * 1024 * 1024 * 1024 {
            return Err(invalid());
        }
        let target = runtime.join("lib").join(alias);
        copy_bound(file, &target, false)?;
        verify_signature(&target)?;
    }
    for path in [&runtime.join("bin"), &runtime.join("lib"), &runtime] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o500))?;
    }
    Ok(runtime.join("bin/node"))
}

pub(super) fn isolate(root: &Path, node: &Path, native: &Path) -> io::Result<()> {
    unsafe extern "C" {
        fn sandbox_init(
            profile: *const libc::c_char,
            flags: u64,
            error: *mut *mut libc::c_char,
        ) -> libc::c_int;
        fn sandbox_free_error(error: *mut libc::c_char);
    }
    let literal = |path: &Path| {
        serde_json::to_string(path.to_str().ok_or_else(invalid)?).map_err(io::Error::other)
    };
    let root = literal(root)?;
    let node = literal(node)?;
    let native = literal(native)?;
    // 只给本次私有 Node 闭包设置 DYLD_LIBRARY_PATH；原始 Cellar/opt 路径没有读取回退。
    // 包与解释器目录对被测进程只读，认证/配置仅能写入本次空 profile。
    // 新版 macOS 将系统 dyld 缓存放在 Cryptex 中；只开放该系统目录的读取。
    // libignition 先打开根目录句柄；literal 仅允许该目录本身，不开放子目录或文件。
    let profile = format!(
        r#"(version 1)
(allow default)
(deny network*)
(deny file-read-data (require-all (regex #"^/")
    (require-not (literal "/"))
    (require-not (subpath {root})) (require-not (subpath "/usr/lib"))
    (require-not (subpath "/System/Library")) (require-not (subpath "/private/var/db/dyld"))
    (require-not (subpath "/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld"))
    (require-not (literal "/dev/null")) (require-not (literal "/dev/urandom")) (require-not (literal "/dev/random"))))
(deny file-write* (require-all (regex #"^/") (require-not (subpath {root})) (require-not (literal "/dev/null"))))
(deny file-write* (subpath (string-append {root} "/package")))
(deny file-write* (subpath (string-append {root} "/node-runtime")))
(deny process-exec (require-all (require-not (literal {node})) (require-not (literal {native}))))
"#
    );
    let profile = CString::new(profile).map_err(io::Error::other)?;
    let mut message = std::ptr::null_mut();
    let status = unsafe { sandbox_init(profile.as_ptr(), 0, &mut message) };
    if !message.is_null() {
        unsafe { sandbox_free_error(message) };
    }
    if status != 0 {
        return Err(io::Error::other("Node 版本探针无法建立只读闭包沙箱"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "managed_process_npm_probe_macos_tests.rs"]
mod tests;
