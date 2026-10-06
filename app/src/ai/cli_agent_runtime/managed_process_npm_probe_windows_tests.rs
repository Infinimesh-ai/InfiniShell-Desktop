use std::os::windows::ffi::OsStrExt as _;

use super::super::AtomicDirectoryIdentity;
use super::*;

#[test]
fn mapped_cmd_keeps_the_fixed_shim_and_private_paths_on_the_bound_drive() {
    let root = Path::new(r"Z:\");
    let (arguments, environment) = mapped_command("cmd", root, false).unwrap();
    assert_eq!(
        arguments,
        r#"/d /v:off /s /c ""Z:\install\codex.cmd" --version""#
    );
    for (name, relative) in [
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("APPDATA", "config"),
        ("CODEX_HOME", "config"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
    ] {
        let value = &environment.iter().find(|(key, _)| key == name).unwrap().1;
        assert_eq!(value, root.join(relative).as_os_str());
    }
    let path = &environment.iter().find(|(key, _)| key == "PATH").unwrap().1;
    let paths = std::env::split_paths(path).collect::<Vec<_>>();
    assert_eq!(paths[0], root.join("runtime"));
    let baseline = candidate_environment("cmd", root).unwrap();
    let baseline_path = &baseline.iter().find(|(key, _)| key == "PATH").unwrap().1;
    assert_eq!(
        paths[1..],
        std::env::split_paths(baseline_path).collect::<Vec<_>>()
    );
    assert!(environment.iter().all(|(key, _)| key != "NODE_OPTIONS"));
}

#[test]
fn mapped_powershell_keeps_alongside_node_selection_without_a_path_override() {
    let root = Path::new(r"D:\");
    let (arguments, environment) = mapped_command("powershell", root, true).unwrap();
    assert_eq!(
        arguments,
        r#"-NoLogo -NoProfile -NonInteractive -Command "$global:LASTEXITCODE=1; Set-Location -LiteralPath 'D:\' -ErrorAction Stop; @() | & 'D:\install\codex.ps1' --version | & { process { $_ } }; exit $global:LASTEXITCODE""#
    );
    let baseline = candidate_environment("powershell", root).unwrap();
    assert_eq!(
        environment.iter().find(|(key, _)| key == "PATH"),
        baseline.iter().find(|(key, _)| key == "PATH")
    );
}

#[test]
fn mapped_commands_reject_non_root_and_unbound_drive_representations() {
    for root in [
        r"C:\",
        r"Z:",
        r"Z:\child",
        r"\\?\Z:\",
        r"z:\",
        r"\\server\share",
        r"Z:\&",
        r"Z:\'; exit 0; #",
        r#"Z:\"; exit 0; #"#,
        r"Z:\$(exit 0)",
    ] {
        assert!(mapped_command("cmd", Path::new(root), false).is_err());
        assert!(mapped_command("powershell", Path::new(root), false).is_err());
    }
    assert!(mapped_command("other", Path::new(r"Z:\"), false).is_err());
}

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
fn powershell_candidate_classifies_the_bound_node_as_an_executable() {
    let root = Path::new("private-candidate");
    let baseline = super::super::version_probe::resolved_environment(root).unwrap();
    let candidate = candidate_environment("powershell", root).unwrap();
    let extensions = candidate
        .iter()
        .filter(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATHEXT"))
        .collect::<Vec<_>>();
    assert_eq!(extensions.len(), 1);
    assert_eq!(extensions[0].1, ".EXE");
    assert_eq!(
        candidate
            .iter()
            .filter(|(name, _)| !name.to_string_lossy().eq_ignore_ascii_case("PATHEXT"))
            .cloned()
            .collect::<Vec<_>>(),
        baseline
    );
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
