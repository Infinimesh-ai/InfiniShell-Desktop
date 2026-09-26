//! 技能来源按完整资源树固定；受限策略不接受可自行授权、执行命令或派生代理的扩展。

use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::reject;
use crate::ai::cli_agent_runtime::RuntimeError;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SkillSource {
    pub(super) selected: SelectedLocalSkill,
    canonical_directory: PathBuf,
    tree_sha256: String,
}

impl SkillSource {
    pub(super) fn capture(selected: &SelectedLocalSkill) -> Result<Self, RuntimeError> {
        if !selected.path.is_absolute()
            || selected
                .path
                .file_name()
                .is_none_or(|name| name != "SKILL.md")
            || selected.name.is_empty()
            || selected.name.len() > 64
            || selected
                .name
                .bytes()
                .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && byte != b'-')
        {
            return Err(reject("claude_skills_source_identity"));
        }
        let source = selected
            .path
            .parent()
            .ok_or_else(|| reject("claude_skills_source_directory"))?;
        ordinary_directory(source)?;
        let canonical_directory =
            std::fs::canonicalize(source).map_err(|_| reject("claude_skills_source_directory"))?;
        let tree_sha256 = tree_digest(&canonical_directory, &selected.name)?;
        Ok(Self {
            selected: selected.clone(),
            canonical_directory,
            tree_sha256,
        })
    }

    pub(super) fn verify_source(&self) -> Result<(), RuntimeError> {
        if *self != Self::capture(&self.selected)? {
            return Err(reject("claude_skills_source_changed"));
        }
        Ok(())
    }

    pub(super) fn verify_copy(&self, directory: &Path) -> Result<(), RuntimeError> {
        if tree_digest(directory, &self.selected.name)? != self.tree_sha256 {
            return Err(reject("claude_skills_copy_changed"));
        }
        Ok(())
    }

    pub(super) fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.canonical_directory)
            || self
                .selected
                .path
                .parent()
                .is_some_and(|root| path.starts_with(root))
    }
}

// 独立解析白名单，不依赖原生对未知字段或损坏 YAML 的忽略行为。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "argument-hint")]
    argument_hint: Option<String>,
    license: Option<String>,
    compatibility: Option<String>,
    metadata: Option<BTreeMap<String, String>>,
    #[serde(rename = "disable-model-invocation")]
    disable_model_invocation: Option<bool>,
    #[serde(rename = "user-invocable")]
    user_invocable: Option<bool>,
}

fn validate_skill(bytes: &[u8], name: &str) -> Result<(), RuntimeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| reject("claude_skills_source_encoding"))?;
    if text.len() > 1024 * 1024 || text.contains('\0') {
        return Err(reject("claude_skills_source_size"));
    }
    // 原生会在加载技能时展开命令和文件引用；这里拒绝这些旁路而不篡改技能正文。
    if text
        .match_indices('!')
        .any(|(index, _)| text[index + 1..].trim_start().starts_with('`'))
        || text.lines().any(|line| {
            line.trim_start()
                .strip_prefix("```")
                .is_some_and(|suffix| suffix.trim_start().starts_with('!'))
        })
        || text.contains('@')
    {
        return Err(reject("claude_skills_implicit_execution_or_read"));
    }
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        // 不接受不同解析器可能作前导空白/BOM 处理的元数据边界。
        if text
            .trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n'])
            .starts_with("---")
        {
            return Err(reject("claude_skills_frontmatter_boundary"));
        }
        return Ok(());
    }
    let mut frontmatter = String::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    if !closed {
        return Err(reject("claude_skills_frontmatter_boundary"));
    }
    let fields: SkillFrontmatter = serde_yaml::from_str(&frontmatter)
        .map_err(|_| reject("claude_skills_frontmatter_unsupported"))?;
    if fields.name.as_deref().is_some_and(|actual| actual != name)
        || fields.disable_model_invocation == Some(true)
    {
        return Err(reject("claude_skills_native_invocation_unavailable"));
    }
    // 字段只用于原生说明，不携带权限；保持完整读取以避免反序列化后遗漏审计。
    let _ = (
        fields.description,
        fields.argument_hint,
        fields.license,
        fields.compatibility,
        fields.metadata,
        fields.user_invocable,
    );
    Ok(())
}

fn ordinary_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| reject("claude_skills_source_unavailable"))?;
    if !metadata.is_dir() || linked(&metadata) {
        return Err(reject("claude_skills_source_link"));
    }
    Ok(())
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn tree_digest(root: &Path, name: &str) -> Result<String, RuntimeError> {
    ordinary_directory(root)?;
    let mut pending = vec![(root.to_owned(), 0usize)];
    let mut records = BTreeMap::new();
    let mut bytes_left = 32 * 1024 * 1024u64;
    let mut entries_left = 1024usize;
    let mut skill_found = false;
    while let Some((directory, depth)) = pending.pop() {
        ordinary_directory(&directory)?;
        if depth > 16 {
            return Err(reject("claude_skills_resource_limit"));
        }
        for entry in
            std::fs::read_dir(directory).map_err(|_| reject("claude_skills_source_unavailable"))?
        {
            entries_left = entries_left
                .checked_sub(1)
                .ok_or_else(|| reject("claude_skills_resource_limit"))?;
            let path = entry
                .map_err(|_| reject("claude_skills_source_unavailable"))?
                .path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| reject("claude_skills_source_path"))?;
            let mut segments = Vec::new();
            for component in relative.components() {
                let Component::Normal(component) = component else {
                    return Err(reject("claude_skills_source_path"));
                };
                let component = component
                    .to_str()
                    .ok_or_else(|| reject("claude_skills_source_path"))?;
                if component.is_empty()
                    || component.ends_with(['.', ' '])
                    || component.contains([':', '\\'])
                    || component.chars().any(char::is_control)
                    || matches!(
                        component.to_ascii_lowercase().as_str(),
                        ".claude-plugin" | ".mcp.json"
                    )
                {
                    return Err(reject("claude_skills_source_path"));
                }
                segments.push(component);
            }
            let key = segments.join("/");
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| reject("claude_skills_source_unavailable"))?;
            if linked(&metadata) {
                return Err(reject("claude_skills_source_link"));
            }
            if metadata.is_dir() {
                records.insert(key, "directory".to_owned());
                pending.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() {
                return Err(reject("claude_skills_source_special_file"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt as _;
                use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
                options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
            }
            let file = options
                .open(&path)
                .map_err(|_| reject("claude_skills_source_unavailable"))?;
            verify_regular_file(&file)?;
            let mut bytes = Vec::new();
            file.take(bytes_left + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| reject("claude_skills_source_unavailable"))?;
            bytes_left = bytes_left
                .checked_sub(bytes.len() as u64)
                .ok_or_else(|| reject("claude_skills_resource_limit"))?;
            if key == "SKILL.md" {
                validate_skill(&bytes, name)?;
                skill_found = true;
            }
            records.insert(key, format!("file:{:x}", Sha256::digest(&bytes)));
        }
    }
    if !skill_found {
        return Err(reject("claude_skills_source_missing"));
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&records).expect("技能树摘要可以序列化"))
    ))
}

fn verify_regular_file(file: &File) -> Result<(), RuntimeError> {
    let metadata = file
        .metadata()
        .map_err(|_| reject("claude_skills_source_unavailable"))?;
    if !metadata.is_file() || linked(&metadata) {
        return Err(reject("claude_skills_source_link"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(reject("claude_skills_source_link"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| reject("claude_skills_source_unavailable"))?;
        if info.nNumberOfLinks != 1 {
            return Err(reject("claude_skills_source_link"));
        }
    }
    Ok(())
}
