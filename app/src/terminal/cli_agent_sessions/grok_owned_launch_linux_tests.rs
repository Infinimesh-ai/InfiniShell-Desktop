use super::*;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;

fn exited_identity(pid: i32) -> LinuxProcessIdentity {
    let identity = LinuxProcessIdentity {
        pid,
        start_time_ticks: 1,
        proc_inode: 1,
        pid_namespace_device: 1,
        pid_namespace_inode: 1,
        uid: unsafe { libc::geteuid() },
        executable_device: 1,
        executable_inode: 1,
    };
    // 由真实 pidfd 接口确认高 PID 夹具不存在，不把接口错误当作已退出。
    assert!(identity_exited(identity));
    identity
}

fn durable_manifest(state: &Path) -> (tempfile::TempDir, tempfile::TempDir, LaunchManifest) {
    let (directory, socket) = create_launch_directories(state).unwrap();
    let socket_root = socket.path().canonicalize().unwrap();
    let metadata = fs::symlink_metadata(&socket_root).unwrap();
    let manifest = LaunchManifest {
        version: 3,
        launch_id: Uuid::from_u128(1),
        session_id: Uuid::from_u128(2),
        history: None,
        notifications: None,
        executable: state.join("grok"),
        app_executable: state.join("infinishell"),
        cwd: state.to_owned(),
        socket_path: socket_root.join("leader.sock"),
        socket_directory: Some(SocketDirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        }),
        boot_session: linux_boot_session().unwrap(),
        shell: exited_identity(i32::MAX - 2).into(),
        tty_device: 1,
    };
    (directory, socket, manifest)
}

#[test]
fn remote_release_retires_native_leader_lock_with_readable_mode() {
    for mode in [0o600, 0o644] {
        let state = tempfile::tempdir().unwrap();
        let root = state.path().canonicalize().unwrap();
        let (directory, socket, manifest) = durable_manifest(&root);
        let path = directory.path().join("launch.json");
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let checksum = digest(&bytes);
        let before = exited_identity(i32::MAX - 1);
        let receipt = ExecReceipt {
            version: 1,
            launch_id: manifest.launch_id,
            manifest_sha256: checksum.clone(),
            process: before.into(),
            process_group: before.pid,
            tty_device: manifest.tty_device,
        };
        let bound = BoundProcesses {
            version: 1,
            launch_id: manifest.launch_id,
            manifest_sha256: checksum.clone(),
            tui: LinuxProcessIdentity {
                executable_inode: 2,
                ..before
            }
            .into(),
            leader: exited_identity(i32::MAX).into(),
        };
        write_new(&path, &bytes).unwrap();
        write_new(&directory.path().join("dispatched"), checksum.as_bytes()).unwrap();
        write_new(
            &directory.path().join("exec.json"),
            &serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        write_new(
            &directory.path().join("bound.json"),
            &serde_json::to_vec(&bound).unwrap(),
        )
        .unwrap();
        let lock = socket.path().join("leader.lock");
        fs::write(&lock, b"2147483647").unwrap();
        fs::set_permissions(&lock, fs::Permissions::from_mode(mode)).unwrap();
        drop(UnixListener::bind(&manifest.socket_path).unwrap());
        let mut restored = GrokOwnedLaunch::restore(&path, &checksum).unwrap();

        restored.release_remote_after_exit().unwrap();

        assert!(restored.is_retired());
        assert!(directory.path().join("retired.json").exists());
        assert!(!socket.path().exists());
        assert!(path.exists());
        assert!(GrokOwnedLaunch::restore(&path, &checksum).unwrap().is_retired());
    }
}

#[test]
fn native_leader_lock_rejects_shared_write_and_nonprivate_parent() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let exited = exited_identity(i32::MAX);
    let lock = socket.path().join("leader.lock");
    fs::write(&lock, b"2147483647").unwrap();
    for mode in [0o620, 0o602, 0o664, 0o666] {
        fs::set_permissions(&lock, fs::Permissions::from_mode(mode)).unwrap();
        assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
        assert_eq!(fs::read(&lock).unwrap(), b"2147483647");
    }
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(socket.path(), fs::Permissions::from_mode(0o750)).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
    assert_eq!(fs::read(&lock).unwrap(), b"2147483647");
    fs::set_permissions(socket.path(), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn native_leader_lock_requires_bound_exited_lifetime_and_exact_bytes() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let lock = socket.path().join("leader.lock");
    fs::write(&lock, b"2147483647").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(cleanup_socket_directory(&manifest, None).is_err());
    let wrong = exited_identity(i32::MAX - 1);
    assert!(cleanup_socket_directory(&manifest, Some(wrong)).is_err());
    assert_eq!(fs::read(&lock).unwrap(), b"2147483647");
    cleanup_socket_directory(&manifest, Some(exited_identity(i32::MAX))).unwrap();
    assert!(!socket.path().exists());
}

#[test]
fn live_leader_lock_is_preserved_even_when_pid_text_matches() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let live = linux_process_identity(std::process::id() as i32).unwrap();
    let lock = socket.path().join("leader.lock");
    fs::write(&lock, live.pid.to_string()).unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(live)).is_err());
    assert!(lock.exists());
}

#[test]
fn leader_lock_alias_and_extra_bytes_are_preserved() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().canonicalize().unwrap();
    let (_directory, socket, manifest) = durable_manifest(&root);
    let exited = exited_identity(i32::MAX);
    let lock = socket.path().join("leader.lock");
    fs::write(&lock, b"2147483647\n").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
    fs::remove_file(&lock).unwrap();
    let other = root.join("other.lock");
    fs::write(&other, b"2147483647").unwrap();
    fs::set_permissions(&other, fs::Permissions::from_mode(0o644)).unwrap();
    fs::hard_link(&other, &lock).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
    assert_eq!(fs::read(&other).unwrap(), b"2147483647");
    assert!(lock.exists());
    fs::remove_file(&lock).unwrap();
    symlink(&other, &lock).unwrap();
    assert!(cleanup_socket_directory(&manifest, Some(exited)).is_err());
    assert_eq!(fs::read(other).unwrap(), b"2147483647");
    assert!(fs::symlink_metadata(&lock).unwrap().file_type().is_symlink());
}

#[test]
fn private_receipt_still_rejects_readable_mode() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = directory.path().canonicalize().unwrap();
    let path = root.join("exec.json");
    write_new(&path, b"{}").unwrap();
    assert_eq!(read_private(&path).unwrap(), b"{}");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_private(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{}");
}
