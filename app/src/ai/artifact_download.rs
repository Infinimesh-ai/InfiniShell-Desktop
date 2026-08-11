use std::path::Path;

pub(crate) fn sanitized_basename(path_or_filename: &str) -> Option<String> {
    let file_name = Path::new(path_or_filename).file_name()?.to_str()?;
    if file_name.is_empty() {
        return None;
    }
    Some(file_name.to_string())
}

// Zap 无云端 artifact 存储:上游的签名 URL 下载工具(extension_for_content_type /
// default_download_filename / download_destination / download_artifact_bytes)依赖已剥离的
// `crate::server::server_api::ai::ArtifactDownloadResponse`,因此不移植。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_basename_accepts_plain_filename() {
        assert_eq!(
            sanitized_basename("report.txt"),
            Some("report.txt".to_string())
        );
    }

    #[test]
    fn sanitized_basename_extracts_from_path() {
        assert_eq!(
            sanitized_basename("outputs/report.txt"),
            Some("report.txt".to_string())
        );
    }
}
