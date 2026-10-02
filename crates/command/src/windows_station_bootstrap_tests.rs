use super::*;

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
    };
    let ready = Ready {
        nonce: request.nonce.clone(),
        process: second.clone(),
        station: first.station_name(),
        desktop: request.profile.clone(),
    };
    validate_ready(&request, &first, &second, &ready).unwrap();
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
            desktop: "different".to_owned(),
            ..ready.clone()
        },
    ] {
        assert!(validate_ready(&request, &first, &second, &changed).is_err());
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
    client.send(&serde_json::json!({"nonce":nonce,"process":actual,"desktop_verified_and_closed":true,"unexpected":true})).unwrap();
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
    assert_eq!(value["first_exit_code"], 1);
    assert_eq!(value["second_exit_code"], 2);
    assert!(value.get("message").is_none());
}

#[test]
fn command_quoting_preserves_embedded_quote_and_trailing_slashes() {
    assert_eq!(
        String::from_utf16(&quoted(OsStr::new("x\"y\\")).unwrap()).unwrap(),
        "\"x\\\"y\\\\\""
    );
    assert!(quoted(OsStr::new("invalid\0value")).is_err());
}
