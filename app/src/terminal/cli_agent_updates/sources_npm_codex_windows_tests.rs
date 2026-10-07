use super::*;

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
