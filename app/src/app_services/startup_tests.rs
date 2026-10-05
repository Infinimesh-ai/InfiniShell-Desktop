use super::{StartupArgsForwardingError, pass_startup_args_to_existing_instance};

#[test]
fn crash_recovery_watcher_does_not_forward_startup_arguments() {
    let args = warp_cli::AppArgs {
        crash_recovery_mechanism: Some(warp_cli::RecoveryMechanism::DedicatedGpu),
        ..Default::default()
    };

    assert!(matches!(
        pass_startup_args_to_existing_instance(&args),
        Err(StartupArgsForwardingError::IgnoredForCrashRecoveryProcess)
    ));
}
