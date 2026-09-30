use super::*;
use serde_json::{Value, json};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _, symlink};
use std::os::unix::net::UnixListener;

fn instance() -> Uuid {
    Uuid::parse_str("72912484-76c6-46df-931c-38db2f552c4e").unwrap()
}

#[test]
fn complete_response_remains_readable_after_peer_closes() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    write_frame(&mut server, b"{}", Instant::now() + IO_TIMEOUT).unwrap();
    drop(server);

    assert_eq!(
        read_frame(&mut client, Instant::now() + IO_TIMEOUT).unwrap(),
        b"{}"
    );
}

#[test]
fn response_read_keeps_one_deadline_for_buffered_header_and_missing_body() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    write_frame(&mut server, b"{}", Instant::now() + IO_TIMEOUT).unwrap();
    assert_eq!(
        read_frame(&mut client, Instant::now() - Duration::from_secs(1))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );

    let (mut client, mut server) = UnixStream::pair().unwrap();
    server.write_all(&10u32.to_be_bytes()).unwrap();
    assert_eq!(
        read_frame(&mut client, Instant::now() + Duration::from_millis(30))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn frames_are_bounded_and_truncated_response_is_not_acknowledged() {
    for length in [0u32, MAX_FRAME_BYTES as u32 + 1] {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        server.write_all(&length.to_be_bytes()).unwrap();
        assert!(read_frame(&mut client, Instant::now() + IO_TIMEOUT).is_err());
    }
    let (mut client, mut server) = UnixStream::pair().unwrap();
    server.write_all(&10u32.to_be_bytes()).unwrap();
    server.write_all(b"{}").unwrap();
    server.shutdown(Shutdown::Write).unwrap();
    assert_eq!(
        read_frame(&mut client, Instant::now() + IO_TIMEOUT)
            .unwrap_err()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
    let (mut client, mut server) = UnixStream::pair().unwrap();
    write_frame(&mut server, b"{}", Instant::now() + IO_TIMEOUT).unwrap();
    assert_eq!(
        read_frame(&mut client, Instant::now() + IO_TIMEOUT).unwrap(),
        b"{}"
    );
    assert_eq!(
        time_left(Instant::now() - Duration::from_secs(1))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn revoked_or_expired_admission_writes_no_request_bytes() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    server.set_nonblocking(true).unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(
        write_admitted_frame(&mut client, b"{}", None, || {
            calls.set(calls.get() + 1);
            false
        })
        .is_err()
    );
    assert_eq!(calls.get(), 1);
    assert!(
        write_admitted_frame(
            &mut client,
            b"{}",
            Some(Instant::now() - Duration::from_secs(1)),
            || {
                calls.set(calls.get() + 1);
                true
            }
        )
        .is_err()
    );
    assert_eq!(calls.get(), 1);
    let mut bytes = [0u8; 1];
    assert_eq!(
        server.read(&mut bytes).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    write_admitted_frame(&mut client, b"{}", None, || {
        calls.set(calls.get() + 1);
        true
    })
    .unwrap();
    assert_eq!(calls.get(), 2);
    server.set_nonblocking(false).unwrap();
    assert_eq!(
        read_frame(&mut server, Instant::now() + IO_TIMEOUT).unwrap(),
        b"{}"
    );
}

fn fixture() -> (tempfile::TempDir, PathBuf, UnixListener) {
    let root = tempfile::Builder::new().prefix("nb-").tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let directory = canonical.join(format!("t-{}", instance().simple()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let socket = directory.join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let manifest = json!({"protocol_version":1,"native_pid":std::process::id(),
        "instance_id":instance(),"socket":socket,"token":"3".repeat(64)});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("manifest.json"))
        .unwrap();
    file.write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    (root, directory, listener)
}

fn locator(root: &Path, directory: PathBuf) -> io::Result<Locator> {
    let root = root.canonicalize()?;
    Locator::read(
        &root,
        NodeStamp::of(&fs::symlink_metadata(&root)?),
        directory,
    )?
    .ok_or_else(invalid)
}

#[test]
fn manifest_and_socket_identity_changes_invalidate_the_locator() {
    let (root, directory, _listener) = fixture();
    let found = locator(root.path(), directory.clone()).unwrap();
    found.validate().unwrap();
    let path = directory.join("manifest.json");
    let bytes = fs::read(&path).unwrap();
    fs::rename(&path, directory.join("old.json")).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    assert!(found.validate().is_err());
    let found = locator(root.path(), directory.clone()).unwrap();
    let socket = directory.join("control.sock");
    fs::rename(&socket, directory.join("old.sock")).unwrap();
    let _replacement = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(found.validate().is_err());
}

#[test]
fn manifest_cannot_escape_private_instance_or_follow_symlinks() {
    let (root, directory, _listener) = fixture();
    let path = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["socket"] = directory
        .join("../control.sock")
        .to_string_lossy()
        .into_owned()
        .into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(locator(root.path(), directory.clone()).is_err());
    fs::rename(&path, directory.join("real.json")).unwrap();
    symlink(directory.join("real.json"), &path).unwrap();
    assert!(locator(root.path(), directory.clone()).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(locator(root.path(), directory).is_err());
}
