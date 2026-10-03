use super::*;
use command::blocking::Command;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Stdio};

struct WaitingChild(Child);
impl WaitingChild {
    fn new() -> Self {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf x; IFS= read -r line || :"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = [0];
        child
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(ready, [b'x']);
        Self(child)
    }
}
impl Drop for WaitingChild {
    fn drop(&mut self) {
        self.0.stdin.take();
        let _ = self.0.wait();
    }
}

#[test]
fn retained_lifetime_proves_only_the_original_child_exit() {
    let mut child = WaitingChild::new();
    let lifetime = NativeLifetime::capture(child.0.id() as i32).unwrap();
    assert!(!lifetime.exited().unwrap());
    let encoded = serde_json::to_vec(&lifetime).unwrap();
    child.0.stdin.take();
    assert!(child.0.wait().unwrap().success());

    let restored: NativeLifetime = serde_json::from_slice(&encoded).unwrap();
    assert!(restored.exited().unwrap());
    assert!(
        !NativeLifetime::capture(std::process::id() as i32)
            .unwrap()
            .exited()
            .unwrap()
    );
}

#[test]
fn exec_does_not_retire_the_still_running_consumer() {
    let mut child = WaitingChild(
        Command::new("/bin/sh")
            .args(["-c", "printf x; IFS= read -r line; exec /bin/cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut ready = [0];
    child
        .0
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .unwrap();
    let lifetime = NativeLifetime::capture(child.0.id() as i32).unwrap();
    child
        .0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"exec\nready\n")
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.0.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    assert!(!lifetime.exited().unwrap());
    assert!(
        lifetime
            .process
            .same_lifetime(&Token::capture(child.0.id() as i32).unwrap())
    );
}

#[test]
fn old_owned_token_keeps_its_original_serialized_fields() {
    let token = Token::capture(std::process::id() as i32).unwrap();
    let encoded = serde_json::to_value(&token).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), 3);
    assert!(encoded.get("pid").is_some());
    assert!(encoded.get("boot").is_some());
    assert!(encoded.get("values").is_some());
    let restored: Token = serde_json::from_value(encoded).unwrap();
    assert!(token.same_lifetime(&restored));
}

#[cfg(target_os = "linux")]
#[test]
fn changed_proc_observer_cannot_prove_a_consumer_exit() {
    let child = WaitingChild::new();
    let mut lifetime = NativeLifetime::capture(child.0.id() as i32).unwrap();
    lifetime.linux_observer.as_mut().unwrap().proc_root.1 = 0;
    assert!(lifetime.exited().is_err());
    let mut lifetime = NativeLifetime::capture(child.0.id() as i32).unwrap();
    lifetime.linux_observer.as_mut().unwrap().pid_namespace.1 = 0;
    assert!(lifetime.exited().is_err());
    lifetime.linux_observer = None;
    assert!(lifetime.exited().is_err());
}
