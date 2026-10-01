use super::*;
use std::os::unix::fs::symlink;

#[test]
fn restored_process_requires_original_boot_and_executable_identity() {
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let target = ProcessTarget::capture(&process).unwrap();
    let encoded = serde_json::to_vec(&target).unwrap();
    let restored: ProcessTarget = serde_json::from_slice(&encoded).unwrap();
    assert!(process.same_identity(&restored.restore().unwrap()));

    let mut old_boot = restored.clone();
    old_boot.token.boot = "another-boot".into();
    assert!(old_boot.restore().is_err());
    let mut wrong_image = restored;
    wrong_image.executable_file.1 = 0;
    assert!(wrong_image.restore().is_err());
}

#[test]
fn another_boot_is_a_different_lifetime_but_an_image_change_is_not() {
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let target = ProcessTarget::capture(&process).unwrap();
    let mut other_boot = target.clone();
    other_boot.token.boot = "another-boot".into();
    assert!(!target.same_birth(&other_boot).unwrap());
    let mut changed_image = target.clone();
    changed_image.executable_file.1 = 0;
    assert!(target.same_birth(&changed_image).unwrap());
    assert!(changed_image.restore().is_err());
}

#[test]
fn durable_target_decoder_rejects_unknown_identity_fields() {
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let mut value = serde_json::to_value(ProcessTarget::capture(&process).unwrap()).unwrap();
    value["client_authorized"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ProcessTarget>(value).is_err());
}

#[test]
fn terminal_identity_refuses_regular_files_and_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let regular = directory.path().join("file");
    fs::write(&regular, b"").unwrap();
    assert!(terminal_identity(&regular).is_err());
    let alias = directory.path().join("alias");
    symlink(&regular, &alias).unwrap();
    assert!(terminal_identity(&alias).is_err());
}
