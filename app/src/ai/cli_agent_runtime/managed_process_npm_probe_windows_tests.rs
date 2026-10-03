use std::os::windows::ffi::OsStrExt as _;

use super::super::AtomicDirectoryIdentity;
use super::*;

#[test]
fn cmd_candidate_uses_standard_heap_without_changing_shared_environment() {
    let root = Path::new("private-candidate");
    let baseline = super::super::version_probe::resolved_environment(root).unwrap();
    let candidate = candidate_environment("cmd", root).unwrap();
    let settings = candidate
        .iter()
        .filter(|(name, _)| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("_NO_DEBUG_HEAP")
        })
        .collect::<Vec<_>>();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].1, "1");
    assert_eq!(
        candidate
            .into_iter()
            .filter(|(name, _)| !name
                .to_string_lossy()
                .eq_ignore_ascii_case("_NO_DEBUG_HEAP"))
            .collect::<Vec<_>>(),
        baseline
    );
}

#[test]
fn powershell_candidate_preserves_shared_environment_without_heap_override() {
    let root = Path::new("private-candidate");
    let baseline = super::super::version_probe::resolved_environment(root).unwrap();
    let candidate = candidate_environment("powershell", root).unwrap();
    assert_eq!(candidate, baseline);
    assert!(candidate.iter().all(|(name, _)| {
        !name
            .to_string_lossy()
            .eq_ignore_ascii_case("_NO_DEBUG_HEAP")
    }));
}

#[test]
fn dos_directory_path_keeps_long_unicode_cwd_bound() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join("中文 npm 工作目录")
        .join("中文 长路径甲".repeat(24))
        .join("中文 长路径乙".repeat(24));
    fs::create_dir_all(&root).unwrap();
    let expected = AtomicDirectoryIdentity::capture(&root).unwrap();
    let cwd = prepare_directory(&expected).unwrap();

    let execution_cwd = dos_directory_path(&cwd).unwrap();

    assert!(execution_cwd.as_os_str().encode_wide().count() > 260);
    assert!(!execution_cwd.to_str().unwrap().starts_with(r"\\?\"));
    assert_eq!(
        execution_cwd.canonicalize().unwrap(),
        expected.canonical_path
    );
    assert_eq!(
        capture_directory_id(&execution_cwd).unwrap(),
        expected.file_id
    );
    assert_eq!(cwd.execution_path(), expected.canonical_path);
    cwd.verify_for_spawn().unwrap();
}

#[test]
fn dos_bound_paths_reject_verbatim_names_with_different_win32_meaning() {
    assert_eq!(
        dos_bound_path(Path::new(r"\\?\C:\probe.\child"))
            .unwrap_err()
            .to_string(),
        "Codex Windows npm 探针闭包不匹配"
    );
    assert_eq!(
        dos_bound_path(Path::new(r"\\?\C:\probe \child"))
            .unwrap_err()
            .to_string(),
        "Codex Windows npm 探针闭包不匹配"
    );
    assert_eq!(
        dos_bound_path(Path::new(r"\\?\C:\probe\NUL.txt"))
            .unwrap_err()
            .to_string(),
        "Codex Windows npm 探针闭包不匹配"
    );
    assert_eq!(
        dos_bound_path(Path::new(r"\\?\C:\probe\COM¹"))
            .unwrap_err()
            .to_string(),
        "Codex Windows npm 探针闭包不匹配"
    );
}

#[test]
fn dos_paths_keep_existing_network_relative_and_command_character_rejections() {
    assert!(dos_path(Path::new(r"\\?\UNC\server\share\probe")).is_err());
    assert!(dos_path(Path::new(r"\\server\share\probe")).is_err());
    assert!(dos_path(Path::new(r"C:probe")).is_err());
    assert!(dos_path(Path::new(r"C:\probe&command")).is_err());
}
