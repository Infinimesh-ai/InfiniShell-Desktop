pub mod external_editor;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};

#[cfg(windows)]
use warp_util::path::is_network_resource;
use warp_util::path::{CleanPathResult, LineAndColumnArg};

pub use self::external_editor::{open_file_path_in_external_editor, open_file_path_with_editor};
use crate::terminal::ShellLaunchData;
use crate::terminal::model::grid::grid_handler::{ContainsPoint, Link};
use crate::terminal::model::index::Point;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilePathType {
    Absolute,
    /// Contains the working directory PathBuf.
    Relative(PathBuf),
}

#[derive(Debug)]
pub enum ShellPathType {
    /// The path comes from the shell and may need to be converted in a shell-aware way.
    ShellNative(String),
    /// The path has already been converted to a OS-native path.
    PlatformNative(PathBuf),
}

/// Zap:某个远端目录(cwd)下真实子项的快照。
///
/// 由 daemon 的 `ListDirectory` RPC 返回的结果填充。终端链接检测器
/// 在远端会话里用它做精确校验:把 `ls -l` 整行候选子串里真正的文件名
/// 切出来 —— 这正是本地会话里 `fs::metadata` 存在性校验所起的作用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDirListing {
    /// 该目录的绝对路径(远端 cwd)。
    pub dir: PathBuf,
    /// 目录下的直接子项:文件名 -> 是否为目录。
    pub entries: HashMap<String, bool>,
}

impl RemoteDirListing {
    pub fn new(dir: PathBuf, entries: HashMap<String, bool>) -> Self {
        Self { dir, entries }
    }
}

/// Zap:终端文件链接的校验来源。
///
/// 本地会话用本地文件系统 `fs::metadata` 判断路径是否存在;远端 SSH
/// (remote-server)会话的文件不在本地磁盘上,本地校验必然失败,因此远端
/// 会话改用 daemon `ListDirectory` RPC 缓存下来的真实目录列表做精确校验。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LinkValidationContext {
    /// 本地会话:用本地文件系统校验路径是否真实存在。
    #[default]
    Local,
    /// 远端 SSH 会话:用缓存下来的远端 cwd 目录列表精确校验。
    ///
    /// `None` 表示该 cwd 的目录列表尚未缓存(异步拉取中或拉取失败),
    /// 此时本轮校验一律视为"无效"(不高亮),等列表到达后 re-render 再点亮。
    Remote(Option<Arc<RemoteDirListing>>),
}

/// Checks if a file path exists and is valid for a file link.
pub fn absolute_path_if_valid(
    clean_path_result: &CleanPathResult,
    working_directory: ShellPathType,
    shell_launch_data: Option<&ShellLaunchData>,
    validation_ctx: &LinkValidationContext,
) -> Option<PathBuf> {
    let (maybe_absolute_path, relative_path) = match shell_launch_data {
        Some(shell_launch_data) => {
            // Attempt to parse the clean path result as an absolute path.
            let maybe_absolute_path =
                shell_launch_data.maybe_convert_absolute_path(&clean_path_result.path);
            let relative_path = match working_directory {
                ShellPathType::ShellNative(base_path_str) => shell_launch_data
                    .maybe_convert_relative_path(&base_path_str, &clean_path_result.path),
                ShellPathType::PlatformNative(base_path) => {
                    shell_launch_data.join_to_native_path(&base_path, &clean_path_result.path)
                }
            };
            (maybe_absolute_path, relative_path)
        }
        None => {
            // We naively attempt to treat the given paths as platform-native.
            let maybe_absolute_path = PathBuf::from(&clean_path_result.path);
            let relative_path = match working_directory {
                ShellPathType::ShellNative(path_str) => {
                    let mut path_buf = PathBuf::from(path_str);
                    path_buf.push(&clean_path_result.path);
                    path_buf
                }
                ShellPathType::PlatformNative(path_buf) => path_buf.join(&clean_path_result.path),
            };
            (Some(maybe_absolute_path), Some(relative_path))
        }
    };

    if relative_path
        .as_ref()
        .is_some_and(|path| is_path_valid(path, clean_path_result, validation_ctx))
    {
        return relative_path;
    } else if maybe_absolute_path
        .as_ref()
        .is_some_and(|path| is_path_valid(path, clean_path_result, validation_ctx))
    {
        return maybe_absolute_path;
    }

    None
}

fn is_path_valid(
    path: &Path,
    clean_path_result: &CleanPathResult,
    validation_ctx: &LinkValidationContext,
) -> bool {
    // Checking for the existence of a network resource takes a long time (~15s),
    // and hangs the UI, so we skip validating it.
    #[cfg(windows)]
    if is_network_resource(path) {
        return false;
    }

    // Zap:远端 SSH 会话的文件不在本地磁盘上,`fs::metadata` 必然失败。
    // 改用 daemon `ListDirectory` 缓存下来的真实目录列表精确校验:候选解析
    // 路径有效 ⇔ 其父目录恰好等于缓存的 cwd 且其文件名是该目录下的已知子项。
    // 这给链接检测器的子串搜索提供了和本地 `fs::metadata` 等价的消歧依据,
    // 能从 `ls -l` 整行里准确切出真正的文件名。
    if let LinkValidationContext::Remote(listing) = validation_ctx {
        // cwd 列表尚未缓存(异步拉取中/失败):本轮视为无效,等列表到达后再点亮。
        let Some(listing) = listing else {
            return false;
        };
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        // 父目录必须正好是缓存的那个 cwd。
        if path.parent() != Some(listing.dir.as_path()) {
            return false;
        }
        let Some(&is_dir) = listing.entries.get(file_name) else {
            return false;
        };
        // 与本地一致:带行列号时不能是目录。
        return !is_dir || clean_path_result.line_and_column_num.is_none();
    }

    // It should only be a valid path if the path links to a file or a folder without
    // line and column number attached.
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() || (metadata.is_dir() && clean_path_result.line_and_column_num.is_none())
}

/// Zap:判断一个已解析的远端路径是否指向目录。
///
/// 仅在远端会话点击链接、需要决定"打开文件 vs `cd` 进目录"时调用;
/// 依据是缓存下来的远端 cwd 目录列表。列表未缓存或路径不在其中时返回
/// `false`(按文件处理,与不缓存时的保守行为一致)。
pub fn remote_path_is_dir(path: &Path, listing: &RemoteDirListing) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if path.parent() != Some(listing.dir.as_path()) {
        return false;
    }
    listing.entries.get(file_name).copied().unwrap_or(false)
}

impl FilePathType {
    /// Given a path that we've identified the FilePathType of,
    /// returns the absolute path.
    pub fn absolute_path(&self, path: PathBuf) -> PathBuf {
        match self {
            FilePathType::Absolute => path,
            FilePathType::Relative(directory) => directory.join(&path),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileLink {
    pub link: Link,
    /// This path has been converted (if needed) into a native path from the shell.
    pub absolute_path: PathBuf,
    pub line_and_column_num: Option<LineAndColumnArg>,
}

impl FileLink {
    pub fn absolute_path(&self) -> Option<PathBuf> {
        Some(self.absolute_path.clone())
    }
}

impl ContainsPoint for FileLink {
    fn contains(&self, point: Point) -> bool {
        self.link.contains(point)
    }
}

/// Creates the file at the given path if it doesn't already exist, opening it
/// in write mode. If any directories in the path are missing, those are created
/// as well.
///
/// This always returns an error for unit tests, as they should not directly
/// interact with the filesystem.
pub fn create_file<P: AsRef<Path>>(_path: P) -> io::Result<fs::File> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            Err(io::Error::from_raw_os_error(1))
        } else {
            let path = _path.as_ref();
            fs::create_dir_all(path.parent().ok_or_else(|| {
                io::Error::other(
                    "full_path should never be root directory.",
                )
            })?)?;
            fs::File::create(path)
        }
    }
}
