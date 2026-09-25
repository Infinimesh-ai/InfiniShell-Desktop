use crate::ai::blocklist::PendingFile;

/// 普通终端交付本地路径引用，由 CLI 自己读取并执行原生权限检查。
/// 整批检查完成前不向 PTY 写入；二进制内容不会被冒充为图片或文本发送。
pub(super) async fn prepare_file_attachments(
    text: String,
    files: Vec<PendingFile>,
) -> Result<String, String> {
    let mut paths = Vec::with_capacity(files.len());
    for file in files {
        let invalid_file = || {
            crate::t!(
                "cli-agent-input-file-unreadable",
                filename = file.file_name.clone()
            )
        };
        if !file.file_path.is_absolute() {
            return Err(invalid_file());
        }
        let metadata = async_fs::metadata(&file.file_path)
            .await
            .map_err(|_| invalid_file())?;
        if !metadata.is_file() {
            return Err(invalid_file());
        }
        // 打开只读句柄验证当前账号的权限，不加载文件内容或改变 CLI 的权限策略。
        let opened = async_fs::File::open(&file.file_path)
            .await
            .map_err(|_| invalid_file())?;
        if !opened
            .metadata()
            .await
            .map_err(|_| invalid_file())?
            .is_file()
        {
            return Err(invalid_file());
        }
        let path = file.file_path.to_str().ok_or_else(invalid_file)?;
        paths.push(path.to_owned());
    }
    // JSON 保留中文、空格、引号与反斜杠边界；不把文件名当作 shell 命令拼接。
    let paths = serde_json::to_string(&paths).expect("文件路径字符串可以编码为 JSON");
    let references = crate::t!("cli-agent-input-local-file-references", paths = paths);
    if text.is_empty() {
        Ok(references)
    } else {
        Ok(format!("{text}\n\n{references}"))
    }
}

#[cfg(test)]
#[path = "file_attachments_tests.rs"]
mod tests;
