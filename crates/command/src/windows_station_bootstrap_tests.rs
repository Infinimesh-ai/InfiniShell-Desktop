use super::super::appcontainer::desktop::verify_expected_station_name;
use super::*;

// 只持有空 Job 和普通文件；不运行引导程序，也不连接或修改测试宿主的窗口站。
pub(in crate::windows) fn unstarted_station(root: &Path, job: OwnedHandle) -> PrivateStation {
    let image = root.join("bootstrap.exe");
    fs::write(&image, b"fixed-image").unwrap();
    let digest = format!("{:x}", Sha256::digest(b"fixed-image"));
    PrivateStation {
        root: PathLease::capture(root).unwrap(),
        package_root: PathLease::capture(root).unwrap(),
        image: StationBootstrapImage::capture(&image, 11, &digest).unwrap(),
        job,
        first: None,
        first_identity: None,
        second: None,
        second_identity: None,
        first_pipe: None,
        second_pipe: None,
        debugger: None,
        nonce: Uuid::new_v4().simple().to_string(),
        phase: "prepare",
        startup: Vec::new(),
        device_map: None,
        closed: false,
        reaped: false,
        abort_attempted: false,
        close_attempted: false,
        #[cfg(feature = "native-probe-witness")]
        witness_desktop: None,
        #[cfg(feature = "native-probe-witness")]
        witness_desktop_error: None,
    }
}

pub(in crate::windows) fn query_only_job(job: &OwnedHandle) -> OwnedHandle {
    // Win32 的 JOB_OBJECT_QUERY；测试仅保留查询权限，不扩大产品依赖特性。
    const JOB_OBJECT_QUERY: u32 = 0x0004;
    let mut duplicate = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw(job),
            GetCurrentProcess(),
            &mut duplicate,
            JOB_OBJECT_QUERY,
            false,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    }
    .unwrap();
    owned(duplicate)
}

#[test]
fn failed_abort_cannot_be_retried_after_its_job_becomes_terminable() {
    let temporary = tempfile::tempdir().unwrap();
    let job = owned(unsafe { CreateJobObjectW(None, None) }.unwrap());
    let mut station = unstarted_station(temporary.path(), query_only_job(&job));

    let failure = station.abort_and_reap().unwrap_err();

    assert_eq!(
        failure
            .get_ref()
            .unwrap()
            .downcast_ref::<windows::core::Error>()
            .unwrap()
            .code(),
        HRESULT::from_win32(5)
    );
    assert!(station.abort_attempted);
    assert!(!station.reaped);
    let receipt = temporary.path().join("aborted-cleanup.json");
    let original = fs::read(&receipt).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["hresult"], HRESULT::from_win32(5).0);

    // 换回完整权限后，旧实现会再次终止空 Job 并错误地把本对象标为已回收。
    station.job = job;
    let repeated = station.abort_and_reap().unwrap_err();
    assert_eq!(repeated.kind(), io::ErrorKind::Other);
    assert_eq!(repeated.to_string(), "窗口站异常回收已失败");
    assert!(!station.reaped);
    drop(station);
    assert_eq!(fs::read(receipt).unwrap(), original);
    assert!(!temporary.path().join("cleanup.json").exists());
    assert!(!temporary.path().join("aborted-release-state.json").exists());
}

#[test]
fn successful_abort_is_idempotent_without_writing_a_normal_cleanup_receipt() {
    let temporary = tempfile::tempdir().unwrap();
    let job = owned(unsafe { CreateJobObjectW(None, None) }.unwrap());
    let mut station = unstarted_station(temporary.path(), job);

    station.abort_and_reap().unwrap();

    assert!(station.abort_attempted);
    assert!(station.reaped);
    let receipt = temporary.path().join("aborted-cleanup.json");
    let original = fs::read(&receipt).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(value["ok"], true);
    station.abort_and_reap().unwrap();
    drop(station);
    assert_eq!(fs::read(receipt).unwrap(), original);
    assert!(!temporary.path().join("cleanup.json").exists());
}

#[test]
fn application_path_normalizes_short_unicode_and_spaces_without_releasing_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("中文 程序.exe");
    fs::write(&path, b"original").unwrap();
    let lease = PathLease::capture(&path).unwrap();
    let canonical = lease.path().to_owned();
    let expected = canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
    assert!(wide(canonical.as_os_str()).unwrap().len() <= MAX_PATH as usize);

    let application = lease.application_path().unwrap();

    assert_eq!(application, wide(OsStr::new(expected)).unwrap());
    assert_eq!(lease.path(), canonical);
    assert_eq!(
        file_identity(&open_path_locked(Path::new(expected)).unwrap()).unwrap(),
        lease.entries[0].2
    );
    lease.verify().unwrap();
}

#[test]
fn application_path_does_not_select_another_file_by_trimming_a_verbatim_name() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let verbatim = root.join("program.exe.");
    let ordinary = root.join("program.exe");
    fs::write(&verbatim, b"verbatim").unwrap();
    fs::write(&ordinary, b"ordinary").unwrap();
    let lease = PathLease::capture(&verbatim).unwrap();
    assert_ne!(
        file_identity(&open_path_locked(&ordinary).unwrap()).unwrap(),
        lease.entries[0].2
    );

    let application = lease.application_path();

    // 系统可以保留 verbatim 表示或拒绝转换，但不得选中去尾点后的另一文件。
    assert!(
        application.is_err() || application.unwrap() == wide(lease.path().as_os_str()).unwrap()
    );
    lease.verify().unwrap();
}

#[test]
fn application_path_preserves_long_verbatim_paths() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join("长目录甲".repeat(30))
        .join("长目录乙".repeat(30));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("program.exe");
    fs::write(&path, b"original").unwrap();
    let lease = PathLease::capture(&path).unwrap();
    let expected = wide(lease.path().as_os_str()).unwrap();
    assert!(expected.len() > MAX_PATH as usize);

    assert_eq!(lease.application_path().unwrap(), expected);
    lease.verify().unwrap();
}

#[test]
fn application_path_keeps_the_original_file_locked_against_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("program.exe");
    let replacement = temporary.path().join("replacement.exe");
    fs::write(&path, b"original").unwrap();
    fs::write(&replacement, b"replacement").unwrap();
    let lease = PathLease::capture(&path).unwrap();
    let application = lease.application_path().unwrap();

    assert!(fs::rename(&replacement, &path).is_err());

    assert_eq!(lease.application_path().unwrap(), application);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    lease.verify().unwrap();
}

#[test]
fn release_observation_distinguishes_remaining_station_from_remaining_logon() {
    let station_remaining = release_observation(&Ok(false), &Ok(true));
    assert_eq!(station_remaining["station_absent"], false);
    assert_eq!(station_remaining["logon_absent"], true);
    assert!(station_remaining["station_query_error"].is_null());
    assert!(station_remaining["logon_query_error"].is_null());

    let logon_remaining = release_observation(&Ok(true), &Ok(false));
    assert_eq!(logon_remaining["station_absent"], true);
    assert_eq!(logon_remaining["logon_absent"], false);
}

#[test]
fn release_observation_keeps_query_failure_unknown_without_error_text() {
    let station = Err(io::Error::other(windows::core::Error::from_hresult(
        HRESULT::from_win32(5),
    )));
    let logon = Err(io::Error::other("PRIVATE_ERROR_TEXT"));
    let receipt = release_observation(&station, &logon);
    assert!(receipt["station_absent"].is_null());
    assert!(receipt["logon_absent"].is_null());
    assert_eq!(receipt["station_query_error"]["hresult"], -2147024891i32);
    assert_eq!(receipt["logon_query_error"]["kind"], "Other");
    assert!(receipt["logon_query_error"]["os_error"].is_null());
    assert!(receipt["logon_query_error"]["hresult"].is_null());
    assert!(!receipt.to_string().contains("PRIVATE_ERROR_TEXT"));
}

fn identity(pid: u32, auth_low: u32) -> Snapshot {
    Snapshot {
        pid,
        created: u64::from(pid) * 100,
        auth_low,
        auth_high: 0,
        session: 1,
        user: "S-1-5-21-1-2-3-1001".to_owned(),
        integrity: "S-1-16-8192".to_owned(),
        elevated: 0,
        token_type: 1,
    }
}

fn device_target() -> device_map::Target {
    serde_json::from_value(serde_json::json!({
        "path": r"\\?\C:\private\candidate",
        "identity": {"volume": 1, "high": 0, "low": 2},
        "nt_path": r"\Device\HarddiskVolume1\private\candidate"
    }))
    .unwrap()
}

fn device_binding(owner: &Snapshot) -> device_map::Binding {
    serde_json::from_value(serde_json::json!({
        "target": device_target(),
        "mapped_root": r"D:\",
        "auth_low": owner.auth_low,
        "auth_high": owner.auth_high,
        "user": owner.user,
        "session": owner.session
    }))
    .unwrap()
}

#[test]
fn ready_requires_exact_process_generation_profile_and_new_station() {
    let caller = identity(10, 11);
    let first = identity(20, 21);
    let second = identity(30, 21);
    let request = Request {
        schema: 1,
        nonce: "a".repeat(32),
        profile: format!("InfiniShell.Version.{}", Uuid::new_v4()),
        container_sid: "S-1-15-2-1".to_owned(),
        caller,
        caller_station: "WinSta0".to_owned(),
        image: PathBuf::from(r"C:\bootstrap.exe"),
        image_size: 5,
        image_sha256: "b".repeat(64),
        device_target: device_target(),
        #[cfg(feature = "native-probe-witness")]
        witness_desktop: false,
    };
    let ready = Ready {
        nonce: request.nonce.clone(),
        process: second.clone(),
        station: first.station_name(),
        station_created_explicitly: false,
        desktop: request.profile.clone(),
        device_map: device_binding(&second),
        device_map_verified_and_created: true,
        #[cfg(feature = "native-probe-witness")]
        witness_desktop_handle: None,
    };
    #[cfg(feature = "native-probe-witness")]
    {
        // 取证构建未显式启用时，正常引导报文不携带取证专属字段。
        let request_wire = serde_json::to_value(&request).unwrap();
        let ready_wire = serde_json::to_value(&ready).unwrap();
        assert!(request_wire.get("witness_desktop").is_none());
        assert!(ready_wire.get("witness_desktop_handle").is_none());
        assert!(
            !serde_json::from_value::<Request>(request_wire)
                .unwrap()
                .witness_desktop
        );
        assert!(
            serde_json::from_value::<Ready>(ready_wire)
                .unwrap()
                .witness_desktop_handle
                .is_none()
        );
    }
    validate_ready(&request, &first, &second, &ready).unwrap();
    assert_eq!(
        serde_json::to_value(&ready).unwrap()["station_created_explicitly"],
        false
    );
    let explicitly_created = Ready {
        station_created_explicitly: true,
        ..ready.clone()
    };
    validate_ready(&request, &first, &second, &explicitly_created).unwrap();
    assert_eq!(
        serde_json::to_value(&explicitly_created).unwrap()["station_created_explicitly"],
        true
    );
    for changed in [
        Ready {
            nonce: "c".repeat(32),
            ..ready.clone()
        },
        Ready {
            process: Snapshot {
                created: second.created + 1,
                ..second.clone()
            },
            ..ready.clone()
        },
        Ready {
            station: request.caller_station.clone(),
            ..ready.clone()
        },
        Ready {
            station: request.caller_station.clone(),
            ..explicitly_created
        },
        Ready {
            desktop: "different".to_owned(),
            ..ready.clone()
        },
        Ready {
            device_map: device_binding(&request.caller),
            ..ready.clone()
        },
        Ready {
            device_map_verified_and_created: false,
            ..ready.clone()
        },
    ] {
        assert!(validate_ready(&request, &first, &second, &changed).is_err());
    }
}

#[test]
fn package_logon_requires_same_authentication_user_and_session_but_allows_lower_integrity() {
    let helper = identity(20, 21);
    let package = Snapshot {
        pid: 40,
        created: 4000,
        integrity: "S-1-16-4096".to_owned(),
        elevated: 0,
        ..helper.clone()
    };
    assert!(package.same_package_logon(&helper));
    assert!(!package.same_local(&helper));
    for changed in [
        Snapshot {
            auth_low: 22,
            ..package.clone()
        },
        Snapshot {
            auth_high: 1,
            ..package.clone()
        },
        Snapshot {
            session: 2,
            ..package.clone()
        },
        Snapshot {
            user: "S-1-5-18".to_owned(),
            ..package.clone()
        },
    ] {
        assert!(!changed.same_package_logon(&helper));
    }
}

#[test]
fn new_authentication_id_does_not_relax_local_token_identity() {
    let before = identity(10, 11);
    let new_logon = identity(20, 21);
    assert!(before.same_local(&new_logon));
    assert!(!before.same_logon(&new_logon));
    for changed in [
        Snapshot {
            session: 2,
            ..new_logon.clone()
        },
        Snapshot {
            elevated: 1,
            ..new_logon.clone()
        },
        Snapshot {
            integrity: "S-1-16-12288".to_owned(),
            ..new_logon.clone()
        },
        Snapshot {
            user: "S-1-5-18".to_owned(),
            ..new_logon.clone()
        },
    ] {
        assert!(!before.same_local(&changed));
    }
}

#[test]
fn pipe_peer_is_bound_to_retained_process_identity() {
    let nonce = Uuid::new_v4().simple().to_string();
    let actual = Snapshot::capture(unsafe { GetCurrentProcess() }).unwrap();
    let server = Pipe::server(&nonce, "test", &actual.user).unwrap();
    let (_client, _process, peer) = Pipe::connect(&nonce, "test").unwrap();
    assert_eq!(actual, peer);
    let wrong = Snapshot {
        created: actual.created + 1,
        ..actual
    };
    assert!(
        server
            .accept(unsafe { GetCurrentProcess() }, &wrong)
            .is_err()
    );
}

#[test]
fn authenticated_pipe_rejects_unknown_authorization_fields() {
    let nonce = Uuid::new_v4().simple().to_string();
    let actual = Snapshot::capture(unsafe { GetCurrentProcess() }).unwrap();
    let server = Pipe::server(&nonce, "test", &actual.user).unwrap();
    let (client, _process, _) = Pipe::connect(&nonce, "test").unwrap();
    server
        .accept(unsafe { GetCurrentProcess() }, &actual)
        .unwrap();
    client.send(&serde_json::json!({"nonce":nonce,"process":actual,"desktop_verified_and_closed":true,"device_map":device_binding(&actual),"device_map_verified_and_removed":true,"unexpected":true})).unwrap();
    assert!(
        server
            .receive::<Closed>(
                unsafe { GetCurrentProcess() },
                Instant::now() + START_TIMEOUT
            )
            .is_err()
    );
}

#[test]
fn bound_image_lease_rejects_write_and_wrong_digest() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bootstrap.exe");
    fs::write(&path, b"fixed-image").unwrap();
    let digest = format!("{:x}", Sha256::digest(b"fixed-image"));
    let image = StationBootstrapImage::capture(&path, 11, &digest).unwrap();
    assert!(OpenOptions::new().write(true).open(&path).is_err());
    assert!(StationBootstrapImage::capture(&path, 11, &"0".repeat(64)).is_err());
    image.lease.verify().unwrap();
    drop(image);
    OpenOptions::new().write(true).open(&path).unwrap();
}

#[test]
fn failure_receipt_keeps_native_code_without_error_body() {
    let temp = tempfile::tempdir().unwrap();
    let result: io::Result<()> = Err(io::Error::other(windows::core::Error::from_hresult(
        HRESULT::from_win32(5),
    )));
    diagnostic(
        temp.path(),
        "failure.json",
        "create_second_suspended",
        &result,
        Some(1),
        Some(2),
    );
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("failure.json")).unwrap()).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["hresult"], HRESULT::from_win32(5).0);
    assert_eq!(value["error_kind"], "Other");
    assert!(value["check_stage"].is_null());
    assert!(value["caller_station_matched"].is_null());
    assert!(value["native_phase"].is_null());
    assert_eq!(value["first_exit_code"], 1);
    assert_eq!(value["second_exit_code"], 2);
    assert!(value.get("message").is_none());
}

#[test]
fn failure_receipt_keeps_profile_check_without_station_access_or_private_values() {
    let temp = tempfile::tempdir().unwrap();
    let failure = NewLogonDesktop::create(
        "PRIVATE_EXPECTED_STATION",
        "InfiniShell.Version.fixed-diagnostic-test",
        "PRIVATE_CONTAINER_SID",
        "PRIVATE_CALLER_STATION",
    )
    .err()
    .unwrap();
    assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    assert_eq!(failure.to_string(), "新站授权 SID 与本轮 profile 不匹配");
    let result: io::Result<()> = Err(failure);
    diagnostic(
        temp.path(),
        "failure.json",
        "station_acl_and_desktop",
        &result,
        None,
        None,
    );

    let bytes = fs::read(temp.path().join("failure.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["phase"], "station_acl_and_desktop");
    assert_eq!(value["error_kind"], "InvalidData");
    assert_eq!(value["check_stage"], "new_logon_profile_sid_mismatch");
    assert!(value["caller_station_matched"].is_null());
    assert!(value["os_error"].is_null());
    assert!(value["hresult"].is_null());
    assert!(value["native_phase"].is_null());
    assert!(!String::from_utf8(bytes).unwrap().contains("PRIVATE_"));
    assert!(value.get("message").is_none());
}

#[test]
fn station_name_failure_receipt_keeps_only_the_true_caller_comparison() {
    let temp = tempfile::tempdir().unwrap();
    let result =
        verify_expected_station_name("PRIVATE_CALLER", "PRIVATE_EXPECTED", "private_caller");
    diagnostic(
        temp.path(),
        "failure.json",
        "station_acl_and_desktop",
        &result,
        None,
        None,
    );

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("failure.json")).unwrap()).unwrap();
    assert_eq!(value["error_kind"], "InvalidData");
    assert_eq!(value["check_stage"], "new_logon_station_name_mismatch");
    assert_eq!(value["caller_station_matched"], true);
    assert!(value["os_error"].is_null());
    assert!(value["hresult"].is_null());
    assert!(value["native_phase"].is_null());
    assert!(!value.to_string().contains("PRIVATE_"));
    assert!(!value.to_string().contains("private_caller"));
}

#[test]
fn station_name_failure_receipt_preserves_false_without_claiming_a_station_category() {
    let temp = tempfile::tempdir().unwrap();
    let result =
        verify_expected_station_name("PRIVATE_OTHER", "PRIVATE_EXPECTED", "PRIVATE_CALLER");
    diagnostic(
        temp.path(),
        "failure.json",
        "station_acl_and_desktop",
        &result,
        None,
        None,
    );

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("failure.json")).unwrap()).unwrap();
    assert_eq!(value["caller_station_matched"], false);
    assert_eq!(value["check_stage"], "new_logon_station_name_mismatch");
    assert!(value["os_error"].is_null());
    assert!(value["hresult"].is_null());
    assert!(value["native_phase"].is_null());
    assert!(!value.to_string().contains("PRIVATE_"));
    assert!(value.get("station_category").is_none());
}

#[test]
fn ready_send_receipt_keeps_unclassified_failure_unknown_without_its_body() {
    let temp = tempfile::tempdir().unwrap();
    let result: io::Result<()> = Err(io::Error::other("PRIVATE_ERROR_BODY"));
    diagnostic(
        temp.path(),
        "failure.json",
        "station_ready_send",
        &result,
        None,
        None,
    );

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("failure.json")).unwrap()).unwrap();
    assert_eq!(value["phase"], "station_ready_send");
    assert_eq!(value["error_kind"], "Other");
    assert!(value["check_stage"].is_null());
    assert!(value["os_error"].is_null());
    assert!(value["hresult"].is_null());
    assert!(value["native_phase"].is_null());
    assert!(!value.to_string().contains("PRIVATE_ERROR_BODY"));
}

#[test]
fn failure_receipt_keeps_original_os_error_without_inventing_hresult() {
    let temp = tempfile::tempdir().unwrap();
    let result: io::Result<()> = Err(io::Error::from_raw_os_error(5));
    diagnostic(
        temp.path(),
        "failure.json",
        "station_ready_send",
        &result,
        None,
        None,
    );

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("failure.json")).unwrap()).unwrap();
    assert_eq!(value["error_kind"], "PermissionDenied");
    assert_eq!(value["os_error"], 5);
    assert!(value["hresult"].is_null());
    assert!(value["check_stage"].is_null());
    assert!(value["native_phase"].is_null());
}

#[test]
fn successful_receipt_does_not_invent_failure_fields() {
    let temp = tempfile::tempdir().unwrap();
    diagnostic(
        temp.path(),
        "success.json",
        "desktop_lifetime",
        &Ok(()),
        Some(0),
        Some(0),
    );

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("success.json")).unwrap()).unwrap();
    assert_eq!(value["ok"], true);
    assert!(value["caller_station_matched"].is_null());
    assert!(value["error_kind"].is_null());
    assert!(value["check_stage"].is_null());
    assert!(value["os_error"].is_null());
    assert!(value["hresult"].is_null());
    assert!(value["native_phase"].is_null());
}

#[test]
fn command_quoting_preserves_embedded_quote_and_trailing_slashes() {
    assert_eq!(
        String::from_utf16(&quoted(OsStr::new("x\"y\\")).unwrap()).unwrap(),
        "\"x\\\"y\\\\\""
    );
    assert!(quoted(OsStr::new("invalid\0value")).is_err());
}
