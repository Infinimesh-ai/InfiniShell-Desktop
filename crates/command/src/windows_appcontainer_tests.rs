use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use super::*;

#[test]
fn profile_environment_replaces_one_case_insensitive_value_without_mutating_input() {
    let environment = vec![
        ("TMP".into(), r"C:\private\tmp".into()),
        ("LocalAppData".into(), r"C:\private\data".into()),
        ("HOME".into(), r"C:\private\home".into()),
        ("USERPROFILE".into(), r"C:\private\home".into()),
        ("CODEX_HOME".into(), r"C:\private\config".into()),
    ];

    let result =
        profile_environment(&environment, r"C:\Users\owner\AppData\Local".as_ref()).unwrap();

    assert_eq!(
        result,
        vec![
            ("TMP".into(), r"C:\private\tmp".into()),
            (
                "LocalAppData".into(),
                r"C:\Users\owner\AppData\Local".into()
            ),
            ("HOME".into(), r"C:\private\home".into()),
            ("USERPROFILE".into(), r"C:\private\home".into()),
            ("CODEX_HOME".into(), r"C:\private\config".into()),
        ]
    );
    assert_eq!(environment[1].1, OsStr::new(r"C:\private\data"));
}

#[test]
fn profile_environment_rejects_case_ambiguous_values() {
    let environment = vec![
        ("LOCALAPPDATA".into(), r"C:\private\data".into()),
        ("localappdata".into(), r"C:\other\data".into()),
    ];

    let failure = profile_environment(&environment, r"C:\native".as_ref()).unwrap_err();

    assert_eq!(failure.to_string(), "版本探针环境 LOCALAPPDATA 重复");
    assert_eq!(environment[1].1, OsStr::new(r"C:\other\data"));
}

#[test]
fn profile_environment_rejects_missing_key_without_inserting_it() {
    let environment = vec![("HOME".into(), r"C:\private\home".into())];

    let failure = profile_environment(&environment, r"C:\native".as_ref()).unwrap_err();

    assert_eq!(failure.to_string(), "版本探针环境缺少 LOCALAPPDATA");
    assert_eq!(
        environment,
        vec![("HOME".into(), r"C:\private\home".into())]
    );
}

#[test]
fn profile_environment_keeps_long_unicode_private_paths_independent_of_native_base() {
    let private = OsString::from(concat!(
        r"\\?\C:\私有验收\长期保留的真实工作目录\",
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\\配置",
    ));
    assert!(private.encode_wide().count() > 260);
    let environment = vec![
        ("LOCALAPPDATA".into(), private.clone()),
        ("TMP".into(), private.clone()),
        ("TEMP".into(), private.clone()),
        ("TMPDIR".into(), private.clone()),
        ("APPDATA".into(), private.clone()),
        ("CODEX_HOME".into(), private.clone()),
    ];

    let result =
        profile_environment(&environment, r"C:\Users\用户\AppData\Local".as_ref()).unwrap();

    assert_eq!(result[0].1, OsStr::new(r"C:\Users\用户\AppData\Local"));
    assert_eq!(&result[1..], &environment[1..]);
    assert_eq!(environment[0].1, private);
}

#[test]
fn profile_environment_preserves_unrelated_non_unicode_values() {
    let environment = vec![
        ("LOCALAPPDATA".into(), r"C:\private\data".into()),
        ("UNCHANGED".into(), OsString::from_wide(&[0xd800, 0x003d])),
    ];

    let result = profile_environment(&environment, r"C:\native".as_ref()).unwrap();

    assert_eq!(result[1], environment[1]);
}

#[test]
fn profile_directory_paths_accept_unicode_local_dos_and_verbatim_forms() {
    assert!(local_profile_path(Path::new(
        r"C:\Users\用户\AppData\Local"
    )));
    assert!(local_profile_path(Path::new(
        r"\\?\C:\Users\用户\AppData\Local"
    )));
}

#[test]
fn profile_directory_paths_reject_remote_relative_and_device_namespaces() {
    assert!(!local_profile_path(Path::new(r"\\server\share\profile")));
    assert!(!local_profile_path(Path::new(
        r"\\?\UNC\server\share\profile"
    )));
    assert!(!local_profile_path(Path::new(r"C:profile")));
    assert!(!local_profile_path(Path::new(r"profile")));
    assert!(!local_profile_path(Path::new(r"\\.\C:\profile")));
    assert!(!local_profile_path(Path::new(r"C:\base\..\profile")));
}

#[test]
fn profile_binding_requires_strict_ancestor_identities_without_fixed_directory_depth() {
    let base = ProfileDirectoryIdentity {
        volume: 1,
        high: 2,
        low: 3,
    };
    let profile = ProfileDirectoryIdentity {
        volume: 1,
        high: 2,
        low: 4,
    };
    let nested = ProfileDirectoryIdentity {
        volume: 1,
        high: 2,
        low: 5,
    };

    assert!(profile_is_below_base(&[base], &[base, profile]));
    assert!(profile_is_below_base(&[base], &[base, profile, nested]));
    assert!(!profile_is_below_base(&[], &[profile]));
    assert!(!profile_is_below_base(&[base], &[base]));
    assert!(!profile_is_below_base(&[base], &[profile, nested]));
}

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
        confirmed_exit_code: None,
        startup_failure: None,
        thread: None,
        process_id: 0,
        cleaned: false,
        private_station: None,
        profile_directories: None,
        captured_output: None,
        private_desktop: None,
    };
    // profile 查询失败时 probe 已拥有 SID/Job；展开栈仍能走原 Drop 清理。
    probe.profile_directories = Some(NativeProfileDirectories::capture(probe.sid).unwrap());
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
    assert!(probe.profile_directories.is_some());
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
    assert!(probe.profile_directories.is_none());
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
