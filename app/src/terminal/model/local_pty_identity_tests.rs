use std::os::unix::io::FromRawFd;

use super::*;

fn pty() -> (File, File) {
    let pair = nix::pty::openpty(None, None).unwrap();
    unsafe {
        (
            File::from_raw_fd(pair.master),
            File::from_raw_fd(pair.slave),
        )
    }
}

#[test]
fn current_process_requires_the_original_lifetime() {
    let (master, slave) = pty();
    let mut identity = LocalPtyIdentity::capture(std::process::id(), &master).unwrap();
    assert_eq!(identity.slave_device(), slave.metadata().unwrap().rdev());
    assert_eq!(identity.current_shell().unwrap(), identity.shell);
    identity.shell.unique_id += 1;
    assert!(identity.current_shell().is_err());
}

#[test]
fn missing_pty_identity_is_not_inferred() {
    let (master, _slave) = pty();
    assert!(LocalPtyIdentity::capture(0, &master).is_err());
    assert!(LocalPtyIdentity::capture(u32::MAX, &master).is_err());
    let regular = tempfile::tempfile().unwrap();
    assert!(LocalPtyIdentity::capture(std::process::id(), &regular).is_err());
}
