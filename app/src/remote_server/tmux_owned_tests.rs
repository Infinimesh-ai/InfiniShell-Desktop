use super::*;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::sync::Barrier;

// 仅用于私有记录的序列化测试；这些字段不能代替真实内核导出或授予输入权限。
fn target() -> TmuxTarget {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "server": {"token":{"pid":101,"boot":"fixture","values":[1,2,3]},"uid":501,"executable":"/fixture/tmux","executable_file":[1,2]},
        "pane": {"token":{"pid":102,"boot":"fixture","values":[4,5,6]},"uid":501,"executable":"/fixture/shell","executable_file":[1,3]},
        "session_id":"$1","window_id":"@1","pane_id":"%1","pane_session":102,
        "tty":"/fixture/tty","tty_file":[1,4,5]
    })).unwrap()
}

fn fixture() -> (tempfile::TempDir, Store, Claim, Uuid) {
    let directory = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let store = Store::new(directory.path()).unwrap();
    let key = Uuid::new_v4();
    let claim = Claim {
        version: 1,
        id: Uuid::new_v4(),
        key_sha256: digest(key.as_bytes()),
        agent: TerminalBindingOwnedAgent::Codex as i32,
        host: "daemon".into(),
        terminal_session: 42,
        terminal_epoch: Uuid::new_v4(),
        source: target(),
    };
    (directory, store, claim, key)
}

#[test]
fn launch_number_remains_claimed_after_store_reopen() {
    let (directory, store, claim, key) = fixture();
    assert!(store.claim(&claim).unwrap());
    let reopened = Store::new(directory.path()).unwrap();
    assert!(!reopened.claim(&claim).unwrap());
    let loaded = reopened
        .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
        .unwrap();
    assert_eq!(loaded.terminal_session, 42);
    assert!(reopened.outcome(&loaded).is_err());
}

#[test]
fn simultaneous_claims_allow_only_one_dispatch_owner() {
    let (_directory, store, claim, _) = fixture();
    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(2));
    let other_store = store.clone();
    let other_claim = claim.clone();
    let other_barrier = barrier.clone();
    let other = std::thread::spawn(move || {
        other_barrier.wait();
        other_store.claim(&other_claim).unwrap()
    });
    barrier.wait();
    let first = store.claim(&claim).unwrap();
    assert_ne!(first, other.join().unwrap());
}

#[test]
fn interrupted_claim_never_reopens_the_dispatch_slot() {
    let (_directory, store, claim, key) = fixture();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(store.directory(claim.id).unwrap())
        .unwrap();
    assert!(!store.claim(&claim).unwrap());
    assert!(store.owns(claim.id).unwrap());
    assert_eq!(
        store
            .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn wrong_key_or_agent_cannot_read_a_claim() {
    let (_directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    assert!(
        store
            .load(claim.id, Uuid::new_v4(), TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    assert!(
        store
            .load(claim.id, key, TerminalBindingOwnedAgent::Grok)
            .is_err()
    );
}

#[test]
fn receipt_is_immutable_and_bound_to_exact_claim_bytes() {
    let (_directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    store
        .finish(&claim, Phase::Started, Some(target()))
        .unwrap();
    assert!(store.finish(&claim, Phase::Failed, None).is_err());
    assert!(store.outcome(&claim).unwrap().target.is_some());
    let path = store.directory(claim.id).unwrap().join("claim.json");
    let mut bytes = read_private(&path).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
    assert!(
        store
            .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
            .is_ok()
    );
    assert!(store.outcome(&claim).is_err());
}

#[test]
fn unknown_outcome_cannot_carry_a_target_capability() {
    let (_directory, store, claim, _) = fixture();
    store.claim(&claim).unwrap();
    store
        .finish(&claim, Phase::Unknown, Some(target()))
        .unwrap();
    assert!(store.outcome(&claim).is_err());
}

#[test]
fn partial_claim_bytes_do_not_authorize_or_allow_replacement() {
    let (_directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    fs::write(store.directory(claim.id).unwrap().join("claim.json"), b"{").unwrap();
    assert!(
        store
            .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    assert!(!store.claim(&claim).unwrap());
}

#[test]
fn root_replacement_invalidates_the_open_store() {
    let (directory, store, claim, _) = fixture();
    fs::rename(&store.root, directory.path().join("original-root")).unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&store.root)
        .unwrap();
    assert!(store.claim(&claim).is_err());
    assert!(store.owns(claim.id).is_err());
}

#[test]
fn symlink_claim_is_not_read_and_does_not_reopen_dispatch() {
    let (directory, store, claim, key) = fixture();
    let other = directory.path().join("unrelated");
    fs::DirBuilder::new().mode(0o700).create(&other).unwrap();
    write_new(&other.join("claim.json"), &claim).unwrap();
    symlink(&other, store.directory(claim.id).unwrap()).unwrap();
    assert!(!store.claim(&claim).unwrap());
    assert!(
        store
            .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
}

#[test]
fn hardlinked_claim_is_not_a_private_ticket() {
    let (directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    fs::hard_link(
        store.directory(claim.id).unwrap().join("claim.json"),
        directory.path().join("alias"),
    )
    .unwrap();
    assert!(
        store
            .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
}

#[test]
fn ownership_query_never_reserves_a_missing_launch() {
    let (_directory, store, claim, _) = fixture();
    assert!(!store.owns(claim.id).unwrap());
    assert!(!store.directory(claim.id).unwrap().exists());
    assert!(store.claim(&claim).unwrap());
}

#[test]
fn claim_persists_only_key_digest() {
    let (_directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    let bytes = read_private(&store.directory(claim.id).unwrap().join("claim.json")).unwrap();
    assert!(!String::from_utf8(bytes).unwrap().contains(&key.to_string()));
}

#[test]
fn unpublished_claim_does_not_block_unrelated_ordinary_consumers() {
    let (_directory, store, claim, _) = fixture();
    let path = store.directory(claim.id).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    write_new(&path.join("claim.prepare.json"), &claim).unwrap();
    // 最终 claim 不存在，从未许可 reserve/split；这里不会查询任何真实 PID。
    assert!(!store.owns_consumer(102, 5).unwrap());
    assert!(!store.claim(&claim).unwrap());
}

#[test]
fn atomic_publication_retains_original_host_without_granting_a_new_host() {
    let (directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    assert!(
        !store
            .directory(claim.id)
            .unwrap()
            .join("claim.prepare.json")
            .exists()
    );
    let reopened = Store::new(directory.path()).unwrap();
    let loaded = reopened
        .load(claim.id, key, TerminalBindingOwnedAgent::Codex)
        .unwrap();
    let original = loaded.original_scope();
    assert_eq!(original.host(), "daemon");
    assert_eq!(original.terminal_session(), 42);
    assert!(!original.matches("new-daemon", 42));
    assert!(!original.matches("daemon", 43));
    // Store 只读出私有身份；新 daemon 必须另经 fresh Bound 才能发布 ImageGuard。
    assert!(!reopened.claim(&claim).unwrap());
}

#[test]
fn pre_dispatch_failure_does_not_block_ordinary_consumers() {
    let (_directory, store, claim, _) = fixture();
    store.claim(&claim).unwrap();
    store
        .finish_launch(
            &claim,
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        )
        .unwrap();
    assert!(store.outcome(&claim).unwrap().phase == Phase::Failed);
    // Failed 没有任何新 pane；不调用真实内核候选，也不限制普通输入。
    assert!(!store.owns_consumer(102, 5).unwrap());
    assert!(!store.claim(&claim).unwrap());
}

#[test]
fn possible_dispatch_remains_unknown() {
    let (_directory, store, claim, _) = fixture();
    store.claim(&claim).unwrap();
    store
        .finish_launch(&claim, Ok(SplitOutcome::OutcomeUnknown))
        .unwrap();
    let outcome = store.outcome(&claim).unwrap();
    assert!(outcome.phase == Phase::Unknown);
    assert!(outcome.target.is_none());
    assert!(!store.claim(&claim).unwrap());
}

fn restarted_service(parent: &Path) -> (Service, TerminalBindingScope) {
    let host = "restarted-daemon".to_owned();
    let codex = Arc::new(CodexService::without_reaper_for_test(host.clone(), parent).unwrap());
    let grok = Arc::new(GrokService::without_reaper_for_test(host.clone(), parent).unwrap());
    let service = Service::new(host.clone(), parent, codex, grok);
    let scope = TerminalBindingScope {
        host_id: host,
        terminal_session_id: 99,
        terminal_epoch: Uuid::new_v4().to_string(),
    };
    (service, scope)
}

#[test]
fn restarted_service_recovers_original_ticket_without_restoring_input() {
    let (directory, store, claim, key) = fixture();
    store.claim(&claim).unwrap();
    let (service, scope) = restarted_service(directory.path());
    let original = service
        .ticket_terminal(&scope, claim.id, key, TerminalBindingOwnedAgent::Codex)
        .unwrap();
    assert!(original.matches("daemon", 42));
    assert!(service.authorizations.lock().unwrap().is_empty());
    // 夹具没有真实存活目标；只读恢复既不查询 PID，也不生成新输入 guard。
    assert!(
        service
            .image_guard(&scope, claim.id, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    assert!(
        service
            .ticket_terminal(
                &scope,
                claim.id,
                Uuid::new_v4(),
                TerminalBindingOwnedAgent::Codex
            )
            .is_err()
    );
    assert!(
        service
            .ticket_terminal(&scope, claim.id, key, TerminalBindingOwnedAgent::Grok)
            .is_err()
    );
}

#[test]
fn image_recovery_keeps_original_scope_and_never_grants_a_live_guard() {
    let (directory, store, claim, _) = fixture();
    store.claim(&claim).unwrap();
    let (service, scope) = restarted_service(directory.path());
    let recovery = claim.recovery_identity();
    service
        .validate_image_recovery(&scope, &recovery, TerminalBindingOwnedAgent::Codex)
        .unwrap();
    assert_eq!(recovery.original_host, "daemon");
    assert_eq!(recovery.original_terminal_session, 42);
    assert!(service.authorizations.lock().unwrap().is_empty());
    assert!(
        service
            .image_guard(&scope, claim.id, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    let mut old_scope = scope.clone();
    old_scope.host_id = claim.host.clone();
    assert!(
        service
            .validate_image_recovery(&old_scope, &recovery, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    old_scope = scope;
    old_scope.terminal_epoch = Uuid::nil().to_string();
    assert!(
        service
            .validate_image_recovery(&old_scope, &recovery, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
}

#[test]
fn image_recovery_rejects_every_changed_claim_identity_field() {
    let (_directory, store, claim, _) = fixture();
    store.claim(&claim).unwrap();
    let recovery = claim.recovery_identity();
    let mut changed = recovery.clone();
    changed.launch_key_sha256 = digest(Uuid::new_v4().as_bytes());
    assert!(
        store
            .validate_image_recovery(&changed, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    changed = recovery.clone();
    changed.original_host = "new-daemon".into();
    assert!(
        store
            .validate_image_recovery(&changed, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    changed = recovery.clone();
    changed.original_terminal_session += 1;
    assert!(
        store
            .validate_image_recovery(&changed, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    changed = recovery.clone();
    changed.agent = TerminalBindingOwnedAgent::Grok as i32;
    assert!(
        store
            .validate_image_recovery(&changed, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    changed = recovery.clone();
    changed.version += 1;
    assert!(
        store
            .validate_image_recovery(&changed, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    changed = recovery.clone();
    changed.launch_id = Uuid::new_v4();
    assert!(
        store
            .validate_image_recovery(&changed, TerminalBindingOwnedAgent::Codex)
            .is_err()
    );
    assert!(
        store
            .validate_image_recovery(&recovery, TerminalBindingOwnedAgent::Grok)
            .is_err()
    );
    assert!(
        store
            .validate_image_recovery(&recovery, TerminalBindingOwnedAgent::Unspecified)
            .is_err()
    );
}
