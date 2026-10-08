use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

use super::*;

#[test]
fn version_invocation_presents_native_paths_bound_to_the_official_shim() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("中文 安装目录");
    fs::create_dir(&root).unwrap();
    let entry = root.join("codex.ps1");
    fs::write(&entry, contract::powershell_shim()).unwrap();
    let before = stamp(&entry).unwrap();
    let identity = tree::path_identity(&entry).unwrap();

    // 输入模拟已绑定的 canonical 入口；呈现给 PowerShell 的普通路径必须仍是同一文件。
    let invocation = version_invocation(&before.canonical).unwrap().unwrap();
    assert_eq!(invocation.args.len(), 6);
    assert_eq!(
        invocation.args[..4],
        [
            OsString::from("-NoLogo"),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
        ]
    );
    assert_eq!(invocation.args[5], "--version");
    let presented_entry = Path::new(&invocation.args[4]);
    assert!(presented_entry.is_absolute());
    assert!(
        !presented_entry
            .as_os_str()
            .as_encoded_bytes()
            .starts_with(br"\\?\")
    );
    assert_eq!(stamp(presented_entry).unwrap(), before);
    assert_eq!(tree::path_identity(presented_entry).unwrap(), identity);

    let mut system_directory = [0u16; 32768];
    let count = unsafe { GetSystemDirectoryW(Some(&mut system_directory)) } as usize;
    assert!(count > 0 && count < system_directory.len());
    let expected_program = PathBuf::from(OsString::from_wide(&system_directory[..count]))
        .join("WindowsPowerShell/v1.0/powershell.exe");
    assert!(invocation.program.is_absolute());
    assert!(
        !invocation
            .program
            .as_os_str()
            .as_encoded_bytes()
            .starts_with(br"\\?\")
    );
    assert_eq!(
        invocation.program.canonicalize().unwrap(),
        expected_program.canonicalize().unwrap()
    );
    assert!(invocation.env.is_empty());
    assert!(invocation.env_remove.is_empty());
}

#[test]
fn version_invocation_preserves_verbatim_components_that_change_dos_meaning() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    for name in ["安装目录.", "安装目录 ", "CON"] {
        let directory = root.join(name);
        fs::create_dir(&directory).unwrap();
        let entry = directory.join("codex.ps1");
        fs::write(&entry, contract::powershell_shim()).unwrap();
        let before = stamp(&entry).unwrap();
        let identity = tree::path_identity(&entry).unwrap();

        // 移除这些路径的 verbatim 前缀会改变所指对象，调用参数必须保留完整路径。
        let invocation = version_invocation(&before.canonical).unwrap().unwrap();
        let presented_entry = Path::new(&invocation.args[4]);
        assert_eq!(presented_entry, before.canonical, "{name}");
        assert_eq!(stamp(presented_entry).unwrap(), before, "{name}");
        assert_eq!(
            tree::path_identity(presented_entry).unwrap(),
            identity,
            "{name}"
        );
    }
}

#[test]
fn version_invocation_preserves_verbatim_paths_beyond_the_dos_limit() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join("长".repeat(130))
        .join("目录".repeat(70));
    fs::create_dir_all(&root).unwrap();
    let entry = root.join("codex.ps1");
    fs::write(&entry, contract::powershell_shim()).unwrap();
    let before = stamp(&entry).unwrap();
    let identity = tree::path_identity(&entry).unwrap();
    assert!(before.canonical.as_os_str().encode_wide().count() > 260);

    let invocation = version_invocation(&before.canonical).unwrap().unwrap();
    let presented_entry = Path::new(&invocation.args[4]);
    assert_eq!(presented_entry, before.canonical);
    assert_eq!(stamp(presented_entry).unwrap(), before);
    assert_eq!(tree::path_identity(presented_entry).unwrap(), identity);
}

#[test]
fn version_invocation_rejects_unofficial_shims_before_presenting_paths() {
    let temporary = tempfile::tempdir().unwrap();
    let entry = temporary.path().join("codex.ps1");
    fs::write(&entry, b"Write-Output 'not the official shim'").unwrap();

    assert_eq!(
        version_invocation(&entry.canonicalize().unwrap()).unwrap_err(),
        Error::UnsupportedSource
    );
}

#[test]
fn nonfile_pending_journal_requires_recovery_without_removing_evidence() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let pending = journal_path(&root);
    fs::create_dir(&pending).unwrap();
    let retained = pending.join("retained-evidence");
    fs::write(&retained, b"pending transaction evidence").unwrap();
    // 真实目录触发底层来源错误；恢复入口不能因此解除尚未收敛事务的启动保护。
    assert_eq!(
        read_limited(&pending, MAX_CONFIG),
        Err(Error::SourceChanged)
    );

    assert_eq!(
        recover(CLIAgent::Codex, &root.join("codex.ps1"), &root),
        Err(Error::RecoveryRequired)
    );

    assert!(pending.is_dir());
    assert_eq!(
        fs::read(&retained).unwrap(),
        b"pending transaction evidence"
    );
}
