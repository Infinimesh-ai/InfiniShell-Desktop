use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::io::Read as _;

use uuid::Uuid;
use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, GetHandleInformation, HANDLE_FLAG_INHERIT};
use windows::Win32::System::Pipes::PeekNamedPipe;

use super::super::station_bootstrap::tests::{query_only_job, unstarted_station};
use super::*;

#[test]
fn failed_station_abort_preserves_the_profile_when_the_probe_is_dropped() {
    let directory = tempfile::tempdir().unwrap();
    let station_job = owned(unsafe { CreateJobObjectW(None, None) }.unwrap());
    let station = unstarted_station(directory.path(), query_only_job(&station_job));
    let name = format!("InfiniShell.Version.{}", Uuid::new_v4());
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
    let probe = AppContainerProbe {
        profile_name: profile_name.clone(),
        sid,
        grants: Vec::new(),
        job: owned(unsafe { CreateJobObjectW(None, None) }.unwrap()),
        process: None,
        confirmed_exit_code: None,
        startup_failure: None,
        thread: None,
        process_id: 0,
        cleaned: false,
        private_station: Some(station),
        profile_directories: None,
        captured_output: None,
        private_desktop: None,
    };

    drop(probe);

    // 同名创建必须仍返回已存在；若外层把失败站当作 None，这里会错误地创建新 profile。
    let existing = unsafe {
        CreateAppContainerProfile(
            PCWSTR(profile_name.as_ptr()),
            PCWSTR(profile_name.as_ptr()),
            PCWSTR(profile_name.as_ptr()),
            None,
        )
    };
    if let Ok(unexpected) = &existing {
        unsafe { FreeSid(*unexpected) };
    }
    // 空 Job 从未启动进程；只清理本测试创建的 profile，断言前也不遗留失败现场。
    unsafe { DeleteAppContainerProfile(PCWSTR(profile_name.as_ptr())) }.unwrap();
    assert_eq!(existing.unwrap_err().code(), HRESULT::from_win32(183));
    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("aborted-cleanup.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["ok"], false);
    assert_eq!(receipt["hresult"], HRESULT::from_win32(5).0);
    assert!(!directory.path().join("cleanup.json").exists());
}

fn stream_test_pipe() -> (File, File) {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }.unwrap();
    (File::from(owned(read)), File::from(owned(write)))
}

fn stream_test_file(value: HANDLE) -> File {
    File::from(
        unsafe { BorrowedHandle::borrow_raw(value.0) }
            .try_clone_to_owned()
            .unwrap(),
    )
}

fn stream_flags(value: HANDLE) -> u32 {
    let mut flags = 0;
    unsafe { GetHandleInformation(value, &mut flags) }.unwrap();
    flags
}

fn stream_bytes_available(value: HANDLE) -> u32 {
    let mut available = 0;
    unsafe { PeekNamedPipe(value, None, 0, None, Some(&mut available), None) }.unwrap();
    available
}

#[test]
fn private_version_stdin_reaches_eof_without_consuming_control_input() {
    let (mut control_read, mut control_write) = stream_test_pipe();
    control_write.write_all(b"control").unwrap();
    let current = unsafe { GetCurrentProcess() };
    let streams = InheritedStreams::from_handles(
        Some(unsafe { BorrowedHandle::borrow_raw(current.0) }),
        [
            HANDLE(control_read.as_raw_handle()),
            HANDLE(control_write.as_raw_handle()),
            HANDLE(control_write.as_raw_handle()),
        ],
    )
    .unwrap();

    // 先用非阻塞查询确认所有写端已释放，再实际读取，避免泄漏回归令测试挂住。
    let failure =
        unsafe { PeekNamedPipe(streams.handles[0], None, 0, None, None, None) }.unwrap_err();
    assert_eq!(failure.code(), HRESULT::from_win32(ERROR_BROKEN_PIPE.0));
    assert_eq!(
        stream_test_file(streams.handles[0]).read(&mut [0]).unwrap(),
        0
    );
    drop(streams);

    assert_eq!(
        stream_bytes_available(HANDLE(control_read.as_raw_handle())),
        7
    );
    let mut bytes = [0; 7];
    control_read.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"control");
}

#[test]
fn private_version_streams_inherit_only_eof_and_output_copies() {
    let (input_read, input_write) = stream_test_pipe();
    let (mut output_read, output_write) = stream_test_pipe();
    let (mut error_read, error_write) = stream_test_pipe();
    let current = unsafe { GetCurrentProcess() };
    let streams = InheritedStreams::from_handles(
        Some(unsafe { BorrowedHandle::borrow_raw(current.0) }),
        [
            HANDLE(input_read.as_raw_handle()),
            HANDLE(output_write.as_raw_handle()),
            HANDLE(error_write.as_raw_handle()),
        ],
    )
    .unwrap();

    assert_eq!(streams.handles.len(), 3);
    assert_eq!(stream_flags(streams.handles[0]), HANDLE_FLAG_INHERIT.0);
    assert_eq!(stream_flags(streams.handles[1]), HANDLE_FLAG_INHERIT.0);
    assert_eq!(stream_flags(streams.handles[2]), HANDLE_FLAG_INHERIT.0);
    assert_eq!(stream_flags(HANDLE(input_read.as_raw_handle())), 0);
    assert_eq!(stream_flags(HANDLE(input_write.as_raw_handle())), 0);
    assert_eq!(stream_flags(HANDLE(output_write.as_raw_handle())), 0);
    assert_eq!(stream_flags(HANDLE(error_write.as_raw_handle())), 0);
    stream_test_file(streams.handles[1])
        .write_all(b"o")
        .unwrap();
    stream_test_file(streams.handles[2])
        .write_all(b"e")
        .unwrap();
    assert_eq!(
        stream_bytes_available(HANDLE(output_read.as_raw_handle())),
        1
    );
    assert_eq!(
        stream_bytes_available(HANDLE(error_read.as_raw_handle())),
        1
    );
    let mut output = [0];
    let mut error = [0];
    output_read.read_exact(&mut output).unwrap();
    error_read.read_exact(&mut error).unwrap();
    assert_eq!(&output, b"o");
    assert_eq!(&error, b"e");
}

#[test]
fn inherited_probe_stdin_remains_connected_to_its_original_pipe() {
    let (input_read, mut input_write) = stream_test_pipe();
    let (output_read, output_write) = stream_test_pipe();
    input_write.write_all(b"input").unwrap();
    let streams = InheritedStreams::from_handles(
        None,
        [
            HANDLE(input_read.as_raw_handle()),
            HANDLE(output_write.as_raw_handle()),
            HANDLE(output_write.as_raw_handle()),
        ],
    )
    .unwrap();

    assert_eq!(stream_bytes_available(streams.handles[0]), 5);
    let mut candidate_input = stream_test_file(streams.handles[0]);
    let mut bytes = [0; 5];
    candidate_input.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"input");
    drop(input_write);
    let failure =
        unsafe { PeekNamedPipe(streams.handles[0], None, 0, None, None, None) }.unwrap_err();
    assert_eq!(failure.code(), HRESULT::from_win32(ERROR_BROKEN_PIPE.0));
    assert_eq!(candidate_input.read(&mut [0]).unwrap(), 0);
    drop(output_read);
}

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
