//! 原生搜索审批只接受当前项目范围；不把路径核对描述为操作系统沙箱。

use std::path::{Component, Path, PathBuf};

pub(super) fn resolve_search_path(root: &Path, requested: Option<&str>) -> Option<PathBuf> {
    // 授权根在创建时已经规范化；不能在审批时跟随后来替换的目录别名。
    if root.canonicalize().ok().as_deref() != Some(root) || !root.is_dir() {
        return None;
    }
    let requested = requested.unwrap_or(".");
    if requested.is_empty() || requested.chars().any(char::is_control) {
        return None;
    }
    let path = Path::new(requested);
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    // Windows 原生工具通常返回普通盘符路径，持久权限根则带扩展前缀。
    let relative = dunce::simplified(&path)
        .strip_prefix(dunce::simplified(root))
        .ok()?;
    let mut current = root.to_owned();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => {
                let name = name.to_str()?;
                if name.ends_with(['.', ' '])
                    || name.contains(':')
                    || [".git", ".claude", ".grok", ".mcp.json"]
                        .iter()
                        .any(|protected| name.eq_ignore_ascii_case(protected))
                {
                    return None;
                }
                current.push(name);
                let metadata = std::fs::symlink_metadata(&current).ok()?;
                if metadata.file_type().is_symlink() {
                    return None;
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt as _;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return None;
                    }
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
        }
    }
    let canonical = current.canonicalize().ok()?;
    (canonical.starts_with(root) && (canonical.is_file() || canonical.is_dir()))
        .then_some(canonical)
}

pub(super) fn bounded_pattern(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16 * 1024 && !value.contains('\0')
}

pub(super) fn relative_glob(value: &str) -> bool {
    // 禁止绝对路径、路径向上跳转与 Windows 分隔符；原生 glob 仍可表达项目内子目录。
    bounded_pattern(value)
        && !value.starts_with('/')
        && !value.contains(['\\', ':'])
        && !value.split('/').any(|part| part == "..")
        && !value.contains(['{', '}'])
        && !value.chars().any(char::is_control)
}
