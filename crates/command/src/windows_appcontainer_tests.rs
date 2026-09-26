use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use super::*;

#[test]
fn cleanup_receipt_rejects_external_acl_change_without_overwriting_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private-candidate.txt");
    std::fs::write(&path, b"private fixture").unwrap();
    let path = path.canonicalize().unwrap();
    let mut hash = DefaultHasher::new();
    path.hash(&mut hash);
    let name = format!(
        "InfiniShell.Version.00000000-0000-4000-8000-{:012x}",
        hash.finish() & 0xffff_ffff_ffff
    );
    let profile_name = wide(name.as_ref()).unwrap();
    let sid = unsafe {
        CreateAppContainerProfile(
            PCWSTR(profile_name.as_ptr()),
            PCWSTR(profile_name.as_ptr()),
            PCWSTR(profile_name.as_ptr()),
            None,
        )
    }
    .unwrap();
    let job = owned(unsafe { CreateJobObjectW(None, None) }.unwrap());
    // 空 Job 是真实内核对象；本例专门覆盖进程已空但授权恢复未获证明的分支。
    let mut probe = AppContainerProbe {
        profile_name,
        sid,
        grants: vec![Grant::new(&path, sid, false).unwrap()],
        job,
        process: None,
        thread: None,
        process_id: 0,
        cleaned: false,
    };
    let receipt = directory.path().join("appcontainer-cleanup-v1");
    let grant = &probe.grants[0];
    let status = unsafe {
        SetSecurityInfo(
            HANDLE(grant.file.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(grant.original),
            None,
        )
    };
    assert_eq!(status.0, 0);
    let externally_changed = current_acl(&grant.file).unwrap();
    assert_ne!(externally_changed, grant.applied);

    let failure = probe.write_cleanup_receipt(&receipt).unwrap_err();

    assert_eq!(failure.to_string(), "版本探针 ACL 已被外部修改");
    assert!(!receipt.exists());
    assert!(!probe.cleaned);
    assert_eq!(
        current_acl(&probe.grants[0].file).unwrap(),
        externally_changed
    );
    // 只为回收本测试重新放回自己保存的授权态；生产路径不会覆盖外部 ACL。
    let grant = &probe.grants[0];
    let status = unsafe {
        SetSecurityInfo(
            HANDLE(grant.file.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(grant.granted),
            None,
        )
    };
    assert_eq!(status.0, 0);
    probe.write_cleanup_receipt(&receipt).unwrap();
    assert!(probe.cleaned);
    assert!(probe.grants.is_empty());
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL.0)
        .open(&path)
        .unwrap();
    assert_eq!(current_acl(&file).unwrap(), externally_changed);
    assert_eq!(
        std::fs::read(&receipt).unwrap(),
        b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n"
    );
}
