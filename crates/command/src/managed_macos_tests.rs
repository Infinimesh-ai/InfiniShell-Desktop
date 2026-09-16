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
