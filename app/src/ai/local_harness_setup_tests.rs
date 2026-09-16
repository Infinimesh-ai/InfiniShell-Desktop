use super::*;
use crate::features::FeatureFlag;

#[test]
fn claude_is_product_enabled_when_cli_is_installed() {
    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Claude, |_| true),
        LocalHarnessSetupState::Ready
    );
}

#[test]
fn claude_is_disabled_for_missing_cli() {
    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Claude, |_| false),
        LocalHarnessSetupState::MissingHarness {
            tooltip: LOCAL_HARNESS_INSTALLATION_REQUIRED_TOOLTIP,
        }
    );
}

#[test]
fn codex_is_enabled_when_flag_is_on() {
    let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);

    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Codex, |_| true),
        LocalHarnessSetupState::Ready
    );
}

#[test]
fn codex_requires_cli_when_flag_is_on() {
    let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);

    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Codex, |_| false),
        LocalHarnessSetupState::MissingHarness {
            tooltip: LOCAL_CODEX_HARNESS_INSTALLATION_REQUIRED_TOOLTIP,
        }
    );
}

#[test]
fn codex_remains_product_disabled() {
    assert_eq!(
        local_harness_setup_state_with_cli_resolver(Harness::Codex, |_| true),
        LocalHarnessSetupState::ProductDisabled {
            message: LOCAL_CODEX_HARNESS_DISABLED_MESSAGE,
        }
    );
}

#[test]
fn readiness_asks_for_the_same_agent_identity_as_the_installation_scan() {
    let _codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    for (harness, installed) in [
        (Harness::Claude, CLIAgent::Claude),
        (Harness::Codex, CLIAgent::Codex),
    ] {
        assert_eq!(
            local_harness_setup_state_with_cli_resolver(harness, |agent| {
                assert_eq!(agent, installed);
                true
            }),
            LocalHarnessSetupState::Ready
        );
    }
}
