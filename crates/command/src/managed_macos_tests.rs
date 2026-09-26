use super::*;

#[test]
fn shared_caller_coalition_cannot_be_claimed() {
    let api = Api::load().unwrap();
    let identity = api.identity(std::process::id() as i32).unwrap();
    assert!(MacosCoalition::claim(identity).is_err());
}

#[test]
fn old_boot_and_zero_cid_cannot_be_exit_proofs() {
    let boot = macos_boot_session().unwrap();
    assert!(MacosCoalition::is_destroyed(0, &boot).is_err());
    assert!(MacosCoalition::is_destroyed(1, "another-boot").is_err());
}

#[test]
fn old_boot_rejects_member_queries_and_signal_before_touching_a_pid() {
    let api = Api::load().unwrap();
    let identity = api.identity(std::process::id() as i32).unwrap();
    let stale = MacosCoalition {
        api,
        cid: identity.resource_cid,
        boot_session: "another-boot".to_owned(),
    };
    assert!(stale.identity(identity.pid).is_err());
    // 只使用 0 号无副作用信号；旧启动身份必须先被拒绝。
    assert!(stale.signal_member(identity, 0).is_err());
}

#[test]
fn live_current_coalition_is_not_destroyed() {
    let identity = Api::load()
        .unwrap()
        .identity(std::process::id() as i32)
        .unwrap();
    assert!(
        !MacosCoalition::is_destroyed(identity.resource_cid, &macos_boot_session().unwrap())
            .unwrap()
    );
}

#[test]
fn owned_process_signal_rejects_old_boot_and_reused_identity() {
    let identity = macos_process_identity(std::process::id() as i32).unwrap();
    let boot = macos_boot_session().unwrap();
    // 使用无副作用的 0 号信号：身份改变必须在调用内核 signal 之前被拒绝。
    assert!(macos_signal_owned_process(identity, "another-boot", 0).is_err());
    assert!(
        macos_signal_owned_process(
            MacosProcessIdentity {
                unique_id: identity.unique_id + 1,
                ..identity
            },
            &boot,
            0,
        )
        .is_err()
    );
    assert!(
        macos_signal_owned_process(
            MacosProcessIdentity {
                pid_version: identity.pid_version.wrapping_add(1),
                ..identity
            },
            &boot,
            0,
        )
        .is_err()
    );
    assert!(
        macos_signal_owned_process(
            MacosProcessIdentity {
                resource_cid: identity.resource_cid + 1,
                ..identity
            },
            &boot,
            0,
        )
        .is_err()
    );
}
