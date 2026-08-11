use serde::{Deserialize, Serialize};

/// Telemetry events for the onboarding flow.
/// Zap: 遥测发送已移除,这里仅保留事件类型作为调用点的类型检查外壳
/// (`send_telemetry_from_ctx!` 等宏为 no-op)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OnboardingEvent {
    /// The onboarding flow was started.
    OnboardingStarted,
    /// A specific slide was viewed.
    SlideViewed {
        slide_name: String,
        /// The REV-1939 offer arm, set only for the "choose how to start" offer.
        experiment_arm: Option<String>,
    },
    /// A setting was changed during onboarding.
    SettingChanged { setting: String, value: String },
    /// The onboarding slides were completed.
    OnboardingSlidesCompleted {
        intention: String,
        model: Option<String>,
        autonomy: Option<String>,
        has_project_path: bool,
        /// How the user is accessing AI when intention is agent_driven:
        /// "warp_agent" or "third_party". None when intention is not agent_driven.
        ai_access: Option<String>,
    },
    /// The user clicked the "Get Started" button.
    GetStartedClicked,
    /// The user started folder selection.
    FolderSelectionStarted,
    /// The user selected a folder.
    FolderSelected,
    /// A callout was displayed.
    CalloutDisplayed { callout: String },
    /// The user clicked next on a callout.
    CalloutNext,
    /// The user completed the callout flow.
    CalloutCompleted { completion_type: String },
    /// The user navigated to the next slide.
    SlideNavigatedNext,
    /// The user navigated to the previous slide.
    SlideNavigatedBack,
    /// The user was shown the "Are you sure you don't want AI?" confirmation modal.
    NoAiConfirmationShown,
    /// The user confirmed they don't want AI in the confirmation modal.
    NoAiConfirmed,
    /// The user chose to keep AI ("Give me AI features") in the confirmation modal.
    NoAiConfirmationCancelled,
    /// The user clicked the "Upgrade" button on the "Customize your agent" slide.
    AgentSlideUpgradeClicked,
    /// The user clicked the "Log in" link on the welcome/intro slide.
    WelcomeLoginClicked,
    /// A canonical user action within the onboarding flow.
    OnboardingAction {
        slide_name: String,
        action: String,
        account_class: Option<String>,
        /// The REV-1939 offer arm, set only for "choose how to start" actions.
        experiment_arm: Option<String>,
    },
    OnboardingAuthCompleted {
        account_class: String,
        has_team: bool,
        is_paid: bool,
        team_discovery_outcome: String,
    },
    OnboardingUpgradeStarted {
        source_slide: String,
        account_class: String,
        /// The REV-1939 offer arm, set only for the "choose how to start" offer.
        experiment_arm: Option<String>,
    },
    OnboardingUpgradeCompleted {
        source_slide: String,
        account_class: String,
        /// The REV-1939 offer arm, set only for the "choose how to start" offer.
        experiment_arm: Option<String>,
    },
    OnboardingCompleted {
        completion_type: String,
        /// The REV-1939 offer arm, set only for the "choose how to start" offer.
        experiment_arm: Option<String>,
    },
}
