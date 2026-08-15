use std::collections::{HashMap, HashSet};

use regex::Regex;
use warp_core::features::FeatureFlag;
use warp_core::settings::{ChangeEventReason, Setting};
use warpui::{
    AppContext, Entity, ModelContext, SingletonEntity, Tracked, ViewContext, WeakViewHandle,
    WindowId,
};

use super::team::{MembershipRole, Team};
#[cfg(test)]
use super::workspace::WorkspaceMemberUsageInfo;
use super::workspace::{
    AdminEnablementSetting, BillingMetadata, CustomerType, EnterpriseSecretRegex,
    HostEnablementSetting, UgcCollectionEnablementSetting, Workspace, WorkspaceUid,
};
use crate::ai::llms::LLMModelHost;
use crate::auth::{AuthStateProvider, TEST_USER_UID, UserUid};
use crate::channel::ChannelState;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::{ObjectType, Owner, Space, StoredObjectEventEntrypoint};
use crate::server::ids::ServerId;
use crate::settings::{AISettings, CodeSettings, PrivacySettings};
#[cfg(test)]
use crate::workspaces::workspace::{AIAutonomyPolicy, WorkspaceMember, WorkspaceSettings};
use crate::workspaces::workspace::{
    AiAutonomySettings, PurchaseAddOnCreditsPolicy, SandboxedAgentSettings,
    UsageBasedPricingSettings,
};

/// Zap(本地化):升级/账单链接仍指向 Warp 官网,只用于外部跳转,不发起任何云端请求。
const STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX: &str = "/upgrade";

#[derive(Debug)]
pub enum UserWorkspacesEvent {
    AddDomainRestrictionsSuccess,
    AddDomainRestrictionsRejected(anyhow::Error),
    DeleteDomainRestrictionSuccess,
    DeleteDomainRestrictionRejected(anyhow::Error),
    EmailInviteSent,
    EmailInviteRejected(anyhow::Error),
    ToggleInviteLinksSuccess,
    ToggleInviteLinksRejected(anyhow::Error),
    ResetInviteLinks,
    ResetInviteLinksRejected(anyhow::Error),
    DeleteTeamInvite,
    DeleteTeamInviteRejected(anyhow::Error),
    SetTeamMemberRoleSuccess,
    SetTeamMemberRoleRejected(anyhow::Error),
    UpdateWorkspaceSettingsSuccess,
    UpdateWorkspaceSettingsRejected(anyhow::Error),
    AiOveragesUpdated,
    PurchaseAddonCreditsSuccess,
    /// The purchase requires the user to complete checkout in the browser
    /// (no saved payment method). Credits arrive via webhook + polling after
    /// checkout completes.
    PurchaseAddonCreditsCheckoutRequired {
        checkout_url: String,
    },
    PurchaseAddonCreditsRejected(anyhow::Error),
    /// Fired whenever the set of teams the user is on changes.
    TeamsChanged,
    /// Fired when the selected workspace actually changes to a different one.
    CurrentWorkspaceChanged,
    /// Fired when a single window's team assignment changes. Windows are independent, so
    /// subscribers that hold per-window state must only react to their own window.
    WindowTeamChanged {
        window_id: WindowId,
    },
    CodebaseContextEnablementChanged,
    /// Fired when a service agreement's sunsetted_to_build_ts field is updated.
    SunsettedToBuildDataUpdated,
}

/// UserWorkspaces is a singleton model that holds workspace metadata (name, members, etc).
/// It should be used for getting information about the workspaces, teams, current teams,
/// and all other things related to operating on workspace and team data.
/// TODO: consolidate local SQLite refresh/update paths.
pub struct UserWorkspaces {
    current_workspace_uid: Tracked<Option<WorkspaceUid>>,
    workspaces: Tracked<Vec<Workspace>>,
    /// Per-window team assignment. Windows are independent, so this is the only
    /// place a window's team lives.
    window_team_uids: HashMap<WindowId, Option<ServerId>>,
    /// The user-level add-on credits purchase policy. Teamless users have no
    /// team, so this is the only place their purchase policy survives.
    /// Zap(本地化):没有云端 workspaces-metadata 轮询,这里始终是 `None`。
    user_purchase_policy: Option<PurchaseAddOnCreditsPolicy>,
}

pub struct CreateTeamResponse {
    pub workspace: Workspace,
    pub team: Team,
}

impl UserWorkspaces {
    #[cfg(any(test, feature = "test-util"))]
    pub fn mock(cached_workspaces: Vec<Workspace>, _ctx: &mut ModelContext<Self>) -> Self {
        Self {
            current_workspace_uid: cached_workspaces.first().map(|w| w.uid).into(),
            workspaces: cached_workspaces.into(),
            window_team_uids: Default::default(),
            user_purchase_policy: None,
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn default_mock(ctx: &mut ModelContext<Self>) -> Self {
        Self::mock(vec![], ctx)
    }

    pub fn new(
        cached_workspaces: Vec<Workspace>,
        current_workspace_uid: Option<WorkspaceUid>,
    ) -> Self {
        Self {
            current_workspace_uid: current_workspace_uid.into(),
            workspaces: cached_workspaces.into(),
            window_team_uids: Default::default(),
            user_purchase_policy: None,
        }
    }

    pub fn upgrade_link(user_id: UserUid) -> String {
        format!(
            "{}{}/{}/{}",
            ChannelState::server_root_url(),
            STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX,
            "user",
            user_id.as_str()
        )
    }

    pub fn upgrade_link_for_team(team_uid: ServerId) -> String {
        format!(
            "{}{}/{}",
            ChannelState::server_root_url(),
            STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX,
            team_uid
        )
    }

    pub fn warp_agent_cli_upgrade_link(user_id: Option<UserUid>) -> String {
        let upgrade_link = user_id.map_or_else(
            || {
                format!(
                    "{}{}",
                    ChannelState::server_root_url().trim_end_matches('/'),
                    STRIPE_SUBSCRIPTION_INTERVAL_PAGE_PREFIX
                )
            },
            Self::upgrade_link,
        );
        format!("{upgrade_link}?source=warp-agent-cli")
    }
    pub fn admin_billing_link_for_team(team_uid: ServerId) -> String {
        format!(
            "{}/admin/{team_uid}/billing",
            ChannelState::server_root_url().trim_end_matches('/')
        )
    }

    pub fn admin_billing_link_for_default_team(&self, user_email: &str) -> Option<String> {
        let team_uid = self.inherited_or_default_team_uid(None)?;
        self.team_from_uid(team_uid)
            .filter(|team| team.has_admin_permissions(user_email))
            .map(|_| Self::admin_billing_link_for_team(team_uid))
    }

    pub fn team_from_uid(&self, team_uid: ServerId) -> Option<&Team> {
        self.current_workspace()
            .and_then(|workspace| workspace.teams.iter().find(|team| team.uid == team_uid))
    }

    pub fn register_window(
        &mut self,
        window_id: WindowId,
        team_uid: Option<ServerId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_team_uid = self.team_uid_for_window(window_id);
        self.window_team_uids.entry(window_id).or_insert(team_uid);
        if self.team_uid_for_window(window_id) != previous_team_uid {
            ctx.emit(UserWorkspacesEvent::WindowTeamChanged { window_id });
        }
        ctx.notify();
    }
    pub fn inherited_or_default_team_uid(
        &self,
        source_window_id: Option<WindowId>,
    ) -> Option<ServerId> {
        source_window_id
            .and_then(|source_window_id| self.team_uid_for_window(source_window_id))
            .or_else(|| {
                self.current_workspace()
                    .and_then(|workspace| workspace.teams.first())
                    .map(|team| team.uid)
            })
    }

    pub fn set_team_for_window(
        &mut self,
        window_id: WindowId,
        team_uid: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let window_team_uid = self.window_team_uids.entry(window_id).or_default();
        if window_team_uid.is_none() {
            *window_team_uid = Some(team_uid);
            ctx.emit(UserWorkspacesEvent::WindowTeamChanged { window_id });
            ctx.notify();
        }
    }

    pub fn team_uid_for_window(&self, window_id: WindowId) -> Option<ServerId> {
        self.window_team_uids.get(&window_id).copied().flatten()
    }

    /// Returns `true` when the user belongs to more than one team in the current
    /// workspace, meaning the team-switcher pill and dropdown should be shown.
    /// Single-team and no-workspace users return `false` so their UI is unchanged.
    pub fn can_switch_teams(&self) -> bool {
        self.current_workspace()
            .map(|ws| ws.teams.len() > 1)
            .unwrap_or(false)
    }
    pub fn team_for_window(&self, window_id: WindowId) -> Option<&Team> {
        self.team_uid_for_window(window_id)
            .and_then(|team_uid| self.team_from_uid(team_uid))
    }
    pub fn team_for_view<T: Entity>(&self, ctx: &ViewContext<T>) -> Option<&Team> {
        self.team_for_window(ctx.window_id())
    }

    pub fn team_for_view_handle<T: Entity>(
        &self,
        view_handle: &WeakViewHandle<T>,
        ctx: &AppContext,
    ) -> Option<&Team> {
        view_handle
            .window_id(ctx)
            .and_then(|window_id| self.team_for_window(window_id))
    }

    /// Returns the windows whose team assignment changed.
    #[must_use]
    fn reconcile_window_team_assignments(&mut self) -> Vec<WindowId> {
        let team_uids = self
            .current_workspace()
            .map(|workspace| {
                workspace
                    .teams
                    .iter()
                    .map(|team| team.uid)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let fallback_team_uid = team_uids.first().copied();

        let mut reassigned_windows = Vec::new();
        for (window_id, window_team_uid) in self.window_team_uids.iter_mut() {
            if window_team_uid.is_none_or(|team_uid| !team_uids.contains(&team_uid))
                && *window_team_uid != fallback_team_uid
            {
                *window_team_uid = fallback_team_uid;
                reassigned_windows.push(*window_id);
            }
        }
        reassigned_windows
    }

    fn emit_window_team_changed(windows: Vec<WindowId>, ctx: &mut ModelContext<Self>) {
        for window_id in windows {
            ctx.emit(UserWorkspacesEvent::WindowTeamChanged { window_id });
        }
    }

    pub fn team_from_uid_across_all_workspaces(&self, team_uid: ServerId) -> Option<&Team> {
        let _ = team_uid;
        None
    }

    /// The teams [`Self::owner_to_space`] recognizes. An owner naming a team outside this set
    /// resolves to the shared space instead of that team's space, so a change here remaps
    /// objects between spaces without any of them changing.
    pub fn team_uids_across_all_workspaces(&self) -> HashSet<ServerId> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.teams.iter())
            .map(|team| team.uid)
            .collect()
    }

    pub fn workspace_from_uid(&self, workspace_uid: WorkspaceUid) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.uid == workspace_uid)
    }

    pub fn workspace_from_uid_mut(
        &mut self,
        workspace_uid: WorkspaceUid,
    ) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.uid == workspace_uid)
    }

    pub fn is_at_tier_limit_for_object_type(
        team_uid: ServerId,
        object_type: ObjectType,
        ctx: &AppContext,
    ) -> bool {
        match object_type {
            ObjectType::Notebook => {
                !UserWorkspaces::has_capacity_for_shared_notebooks(team_uid, ctx, 1)
            }
            ObjectType::Workflow => {
                !UserWorkspaces::has_capacity_for_shared_workflows(team_uid, ctx, 1)
            }
            ObjectType::Folder => false,
            ObjectType::GenericStringObject(_) => false,
        }
    }

    pub fn is_at_tier_limit_for_some_warp_drive_objects(
        team_uid: ServerId,
        ctx: &AppContext,
    ) -> bool {
        UserWorkspaces::is_at_tier_limit_for_object_type(team_uid, ObjectType::Notebook, ctx)
            || UserWorkspaces::is_at_tier_limit_for_object_type(team_uid, ObjectType::Workflow, ctx)
    }

    // Checks if the team has capacity for another shared notebook for their current
    // billing tier, given their current notebook count and delinquency status.
    pub fn has_capacity_for_shared_notebooks(
        team_uid: ServerId,
        ctx: &AppContext,
        new_shared_notebooks: usize,
    ) -> bool {
        let current_shared_notebooks = ObjectStoreModel::as_ref(ctx)
            .active_notebooks_in_space(Space::Team { team_uid }, ctx)
            .count();

        let team = UserWorkspaces::as_ref(ctx).team_from_uid(team_uid);
        if let Some(team) = team {
            // If the team is past due or unpaid, then don't allow new notebooks.
            if team.billing_metadata.is_delinquent_due_to_payment_issue() {
                return false;
            }

            if let Some(policy) = team.billing_metadata.tier.shared_notebooks_policy {
                // Allow new notebooks if policy is unlimited or if the number of notebooks
                // is less than the limit.
                policy.is_unlimited
                    || current_shared_notebooks + new_shared_notebooks
                        <= policy
                            .limit
                            .try_into()
                            .expect("shared notebooks limit should be within max i64 range")
            } else {
                // If no policy is set, then allow it to go through by default (should still be enforced server-side)
                true
            }
        } else {
            // If the team is not found, then allow it to go through by default (should still be enforced server-side)
            true
        }
    }

    // Checks if the team has capacity for another shared workflow for their current
    // billing tier, given their current workflow count and delinquency status.
    pub fn has_capacity_for_shared_workflows(
        team_uid: ServerId,
        ctx: &AppContext,
        new_shared_workflows: usize,
    ) -> bool {
        let current_shared_workflows = ObjectStoreModel::as_ref(ctx)
            .active_workflows_in_space(Space::Team { team_uid }, ctx)
            .count();

        let team = UserWorkspaces::as_ref(ctx).team_from_uid(team_uid);
        if let Some(team) = team {
            // If the team is past due or unpaid, then don't allow new workflows.
            if team.billing_metadata.is_delinquent_due_to_payment_issue() {
                return false;
            }

            if let Some(policy) = team.billing_metadata.tier.shared_workflows_policy {
                // Allow new workflows if policy is unlimited or if the number of workflows
                // is less than the limit.
                policy.is_unlimited
                    || current_shared_workflows + new_shared_workflows
                        <= policy
                            .limit
                            .try_into()
                            .expect("shared workflows limit should be within max i64 range")
            } else {
                // If no policy is set, then allow it to go through by default (should still be enforced server-side)
                true
            }
        } else {
            // If the team is not found, then allow it to go through by default (should still be enforced server-side)
            true
        }
    }

    /// Return the uid of user's current team (if any) without refreshing.
    /// Zap(本地化):没有账号/团队,始终 `None`。
    pub fn current_team_uid(&self) -> Option<ServerId> {
        None
    }

    pub fn current_team_mut(&mut self) -> Option<&mut Team> {
        None
    }

    /// Zap(本地化):没有账号/团队,始终 `None`。
    pub fn current_team(&self) -> Option<&Team> {
        None
    }

    pub fn sole_team(&self) -> Option<&Team> {
        let [team] = self.current_workspace()?.teams.as_slice() else {
            return None;
        };
        Some(team)
    }

    pub fn sole_team_uid(&self) -> Option<ServerId> {
        self.sole_team().map(|team| team.uid)
    }

    /// Note that the workspace is populated with dummy data until the initial fetch
    /// completes (only workspace name/ID and workspace team's name/ID are cached in
    /// sqlite locally).
    /// Consider whether you need to wait for the results of the fetch before checking the
    /// values of other fields.
    pub fn current_workspace(&self) -> Option<&Workspace> {
        self.current_workspace_uid
            .and_then(|workspace_uid| self.workspace_from_uid(workspace_uid))
    }
    pub fn current_workspace_billing_metadata(&self) -> Option<&BillingMetadata> {
        self.current_workspace()
            .map(|workspace| &workspace.billing_metadata)
    }

    /// The given team's billing metadata when the team is known, otherwise
    /// the current workspace's. For purchase surfaces that need
    /// team/workspace-scoped state (e.g. delinquency); for the purchase
    /// policy itself use [`Self::purchase_policy_for_team`], which adds the
    /// user-level fallback for teamless users.
    pub fn team_billing_metadata<'a>(
        &'a self,
        team: Option<&'a Team>,
    ) -> Option<&'a BillingMetadata> {
        team.map(|team| &team.billing_metadata)
            .or_else(|| self.current_workspace_billing_metadata())
    }

    pub fn is_custom_llm_enabled_for_team(&self, team: Option<&Team>) -> bool {
        team.map(Team::is_custom_llm_enabled)
            .or_else(|| {
                self.current_workspace()
                    .map(Workspace::is_custom_llm_enabled)
            })
            .unwrap_or(false)
    }

    /// The add-on credits purchase policy for the current viewer context: the
    /// current workspace's policy when one exists, else the user-level policy
    /// from the workspaces-metadata response (how teamless users get one).
    ///
    /// Callers bound to a view/window should use
    /// [`Self::purchase_policy_for_team`] instead, since their team can
    /// differ from the current workspace's in multi-team situations.
    pub fn purchase_policy(&self) -> Option<PurchaseAddOnCreditsPolicy> {
        self.current_workspace_billing_metadata()
            .and_then(|billing| billing.tier.purchase_add_on_credits_policy)
            .or(self.user_purchase_policy)
    }

    /// [`Self::purchase_policy`], preferring the given team's policy when the
    /// team is known (e.g. resolved from a view or window).
    pub fn purchase_policy_for_team(
        &self,
        team: Option<&Team>,
    ) -> Option<PurchaseAddOnCreditsPolicy> {
        team.and_then(|team| team.billing_metadata.tier.purchase_add_on_credits_policy)
            .or_else(|| self.purchase_policy())
    }

    /// Updates the user-level add-on credits purchase policy captured from a
    /// workspaces-metadata response. Must be called on every path that
    /// applies such a response so the teamless fallback can't go stale.
    pub fn set_user_purchase_policy(&mut self, policy: Option<PurchaseAddOnCreditsPolicy>) {
        self.user_purchase_policy = policy;
    }

    pub fn current_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.current_workspace_uid
            .and_then(|workspace_uid| self.workspace_from_uid_mut(workspace_uid))
    }

    pub fn workspaces(&self) -> &Vec<Workspace> {
        &self.workspaces
    }

    pub fn set_current_workspace_uid(
        &mut self,
        workspace_uid: WorkspaceUid,
        ctx: &mut ModelContext<Self>,
    ) {
        let changed = *self.current_workspace_uid != Some(workspace_uid);
        *self.current_workspace_uid = Some(workspace_uid);
        let reassigned_windows = self.reconcile_window_team_assignments();
        self.notify_and_emit_teams_changed(ctx);
        Self::emit_window_team_changed(reassigned_windows, ctx);
        if changed {
            ctx.emit(UserWorkspacesEvent::CurrentWorkspaceChanged);
        }
    }

    /// Returns `true` if active AI is allowed for the current workspace, based on billing config.
    ///
    /// In the future, we should store active AI enablement on the policy directly. For now, we
    /// proxy whether active AI by checking whether any active AI feature is enabled.
    pub fn is_active_ai_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            workspace
                .billing_metadata
                .tier
                .warp_ai_policy
                .is_none_or(|policy| {
                    policy.is_prompt_suggestions_toggleable
                        || policy.is_next_command_enabled
                        || policy.is_code_suggestions_toggleable
                        || policy.is_git_operations_ai_enabled
                })
        })
    }

    /// Returns `true` if the given team's enterprise status allows AI features that have an
    /// enterprise gate. Non-enterprise teams always pass; enterprise teams pass only if they
    /// are on the Zap Plan or the build is dogfood (both our internal Zap team and dogfood
    /// team are billed as enterprise).
    pub fn ai_allowed_for_team(team: Option<&Team>) -> bool {
        !team.is_some_and(|team| team.billing_metadata.customer_type == CustomerType::Enterprise)
            || team.is_some_and(|team| team.billing_metadata.is_warp_plan())
            || ChannelState::channel().is_dogfood()
    }

    /// Whether Prompt Suggestions should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_prompt_suggestions_toggleable(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle prompt suggestions (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_prompt_suggestions_toggleable)
            })
    }

    /// Whether Code Suggestions should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_code_suggestions_toggleable(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle code suggestions (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_code_suggestions_toggleable)
            })
    }

    /// Whether Next Command should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_next_command_enabled(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle Next Command (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_next_command_enabled)
            })
    }

    /// Whether Git Operations AI is enabled for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    pub fn is_git_operations_ai_enabled(&self) -> bool {
        self.current_workspace()
            // If the user has no team, they can toggle Git Operations AI (no restrictions).
            .is_none_or(|workspace| {
                workspace
                    .billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|policy| policy.is_git_operations_ai_enabled)
            })
    }

    /// Whether voice input should be toggleable for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    /// If voice input support is not compiled into this build, always returns `false`.
    pub fn is_voice_enabled(&self) -> bool {
        cfg!(feature = "voice_input")
            && self
                .current_workspace()
                // If the user has no team, they can toggle Voice (no restrictions).
                .is_none_or(|workspace| {
                    workspace
                        .billing_metadata
                        .tier
                        .warp_ai_policy
                        .is_some_and(|policy| policy.is_voice_enabled)
                })
    }

    /// Whether BYO API key is enabled for the current user, based on the active policies.
    /// Note that the value may be incorrect if called before the team's billing metadata has been fetched.
    ///
    /// Zap:BYOP(自带 provider / API key)是核心能力,所以在没有 workspace 策略时
    /// 一律允许,不走上游的 `SoloUserByok` 灰度门控。仅当某个残留的 workspace
    /// 策略显式关闭时才禁用。
    pub fn is_byo_api_key_enabled(&self, app: &AppContext) -> bool {
        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }
        self.current_workspace()
            .map(|workspace| workspace.is_byo_api_key_enabled())
            .unwrap_or(true)
    }

    /// Whether the current workspace's managed BYOK/BYOE policy allows members
    /// to use their own provider API keys. Users with no workspace, or
    /// workspaces without the managed BYOK/BYOE policy, have no team-level
    /// restriction, so this returns true and the normal BYO entitlement applies.
    pub fn are_member_byo_keys_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            !workspace.billing_metadata.is_managed_byok_byoe_enabled()
                || workspace
                    .settings
                    .team_byo
                    .as_ref()
                    .is_some_and(|team_byo| {
                        team_byo.first_party_enabled && team_byo.allow_user_keys
                    })
        })
    }
    /// Whether custom inference endpoints are enabled for the current user.
    /// Anonymous or logged-out users are not allowed to use custom inference.
    /// Controlled by the BYO_ENDPOINT billing policy.
    pub fn is_custom_inference_enabled(&self, app: &AppContext) -> bool {
        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }

        self.current_workspace()
            .map(|workspace| workspace.billing_metadata.is_byo_endpoint_enabled())
            .unwrap_or(true)
    }

    /// Whether the current workspace's managed BYOK/BYOE policy allows members
    /// to use their own custom endpoints. Users with no workspace, or
    /// workspaces without the managed BYOK/BYOE policy, have no team-level
    /// restriction, so this returns true and the normal BYO entitlement applies.
    pub fn are_member_byo_endpoints_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            !workspace.billing_metadata.is_managed_byok_byoe_enabled()
                || workspace
                    .settings
                    .team_byo
                    .as_ref()
                    .is_some_and(|team_byo| {
                        team_byo.endpoints_enabled && team_byo.allow_user_endpoints
                    })
        })
    }

    pub fn aws_bedrock_host_settings(&self) -> Option<&super::workspace::LlmHostSettings> {
        self.current_workspace().and_then(|workspace| {
            workspace
                .settings
                .llm_settings
                .host_configs
                .get(&LLMModelHost::AwsBedrock)
        })
    }

    /// Did the admin enable AWS Bedrock for the current workspace?
    pub fn is_aws_bedrock_available_from_workspace(&self) -> bool {
        self.current_workspace().is_some_and(|workspace| {
            workspace.settings.llm_settings.enabled
                && self
                    .aws_bedrock_host_settings()
                    .is_some_and(|settings| settings.enabled)
        })
    }
    pub fn aws_bedrock_host_enablement_setting(&self) -> HostEnablementSetting {
        self.aws_bedrock_host_settings()
            .map(|settings| settings.enablement_setting.clone())
            .unwrap_or_default()
    }

    pub fn is_aws_bedrock_credentials_toggleable(&self) -> bool {
        matches!(
            self.aws_bedrock_host_enablement_setting(),
            HostEnablementSetting::RespectUserSetting
        )
    }

    pub fn is_aws_bedrock_credentials_enabled(&self, app: &AppContext) -> bool {
        // i.e. did the admin go and toggle on aws bedrock in the admin panel?
        if !self.is_aws_bedrock_available_from_workspace() {
            return false;
        }

        match self.aws_bedrock_host_enablement_setting() {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => *AISettings::as_ref(app)
                .aws_bedrock_credentials_enabled
                .value(),
        }
    }

    pub fn gemini_enterprise_host_settings(&self) -> Option<&super::workspace::LlmHostSettings> {
        self.current_workspace().and_then(|workspace| {
            workspace
                .settings
                .llm_settings
                .host_configs
                .get(&LLMModelHost::GeminiEnterprise)
        })
    }

    /// Did the admin enable Gemini Enterprise (GEAP) for the current workspace?
    pub fn is_gemini_enterprise_available_from_workspace(&self) -> bool {
        self.current_workspace().is_some_and(|workspace| {
            workspace.settings.llm_settings.enabled
                && self
                    .gemini_enterprise_host_settings()
                    .is_some_and(|settings| settings.enabled)
        })
    }

    pub fn gemini_enterprise_host_enablement_setting(&self) -> HostEnablementSetting {
        self.gemini_enterprise_host_settings()
            .map(|settings| settings.enablement_setting.clone())
            .unwrap_or_default()
    }

    pub fn is_gemini_enterprise_credentials_toggleable(&self) -> bool {
        matches!(
            self.gemini_enterprise_host_enablement_setting(),
            HostEnablementSetting::RespectUserSetting
        )
    }

    /// Whether Gemini Enterprise (GEAP) credentials should be minted and attached for the
    /// current user. Anonymous/logged-out guard from [`Self::is_byo_api_key_enabled`]:
    /// a GEAP credential mint is rooted in the user's Warp session, so without one
    /// there is nothing to mint from.
    pub fn is_gemini_enterprise_credentials_enabled(&self, app: &AppContext) -> bool {
        if !FeatureFlag::GeminiEnterprise.is_enabled() {
            return false;
        }
        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }
        // i.e. did the admin toggle on Gemini Enterprise in the admin panel?
        if !self.is_gemini_enterprise_available_from_workspace() {
            return false;
        }

        match self.gemini_enterprise_host_enablement_setting() {
            HostEnablementSetting::Enforce => true,
            HostEnablementSetting::RespectUserSetting => *AISettings::as_ref(app)
                .gemini_enterprise_credentials_enabled
                .value(),
        }
    }

    /// Returns the AI autonomy settings that are enforced by the workspace for all its members.
    /// If a setting is `None`, the workspace doesn't enforce a particular setting.
    pub fn ai_autonomy_settings(&self) -> AiAutonomySettings {
        self.current_workspace()
            .map(|workspace| workspace.settings.ai_autonomy_settings.clone())
            .unwrap_or_default()
    }

    /// Returns the sandboxed agent settings enforced by the workspace, if any.
    pub fn sandboxed_agent_settings(&self) -> Option<SandboxedAgentSettings> {
        self.current_workspace()
            .and_then(|workspace| workspace.settings.sandboxed_agent_settings.clone())
    }

    /// Returns true iff AI autonomy features are allowed for this client.
    /// TODO: This should be deleted soon. AI autonomy settings have been moved into organization
    /// settings (see `ai_autonomy_settings` above), but there could be an interim time where we
    /// have not set up the org settings yet for an enterprise that previously had the entire
    /// feature set disabled. To capture that case, we'll see if all the settings are `None`;
    /// if so, we'll fall back to their billing metadata's value. Once we've migrated everyone
    /// into org settings, we should remove `is_enabled` from the policy and delete this function.
    pub fn is_ai_autonomy_allowed(&self) -> bool {
        self.current_workspace().is_none_or(|workspace| {
            let settings = &workspace.settings.ai_autonomy_settings;
            let all_settings_none = settings.apply_code_diffs_setting.is_none()
                && settings.read_files_setting.is_none()
                && settings.read_files_allowlist.is_none()
                && settings.execute_commands_setting.is_none()
                && settings.execute_commands_allowlist.is_none()
                && settings.execute_commands_denylist.is_none();

            if all_settings_none {
                workspace
                    .billing_metadata
                    .tier
                    .ai_autonomy_policy
                    .is_some_and(|policy| policy.is_enabled)
            } else {
                true
            }
        })
    }

    // Zap:团队空间是云端协作入口,本地版不暴露任何 Team space。
    pub fn team_spaces(&self) -> Vec<Space> {
        vec![]
    }

    // Zap:Drive 只保留本地 Personal space。Team / Shared 都是云端协作面,
    // 即使旧缓存里还有 workspace metadata,也不能重新进入 Drive 或 Workflow UI。
    pub fn all_user_spaces(&self, ctx: &AppContext) -> Vec<Space> {
        let _ = ctx;
        vec![Space::Personal]
    }

    /// The spaces visible from a given window. Zap has no teams (see
    /// [`Self::team_from_uid`]), so in practice this collapses to the personal
    /// space; the upstream shape is kept so the Drive/palette data sources stay
    /// window-scoped.
    pub fn spaces_for_window(&self, window_id: WindowId, ctx: &AppContext) -> Vec<Space> {
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_user_web_anonymous_user()
            .unwrap_or_default()
        {
            return vec![Space::Shared];
        }
        let mut spaces = vec![];
        if let Some(team) = self.team_for_window(window_id) {
            spaces.push(Space::Team { team_uid: team.uid });
        }

        if FeatureFlag::SharedWithMe.is_enabled()
            && ObjectStoreModel::as_ref(ctx).has_directly_shared_objects(self, ctx)
        {
            spaces.push(Space::Shared);
        }
        spaces.push(Space::Personal);

        spaces
    }

    // Zap(本地化分支)个人空间 owner 固定绑到本地占位用户。
    // 必须保持稳定,否则重启后旧对象 owner 字段对不上,Personal Space 列表里"看不见"旧数据。
    fn effective_personal_user_uid() -> UserUid {
        UserUid::new(TEST_USER_UID)
    }

    // Returns the [`Owner`] for the user's personal drive.
    // Zap:Drive Personal 空间下的 Workflow / EnvVar / Folder / Notebook / Import
    // 等 Create 动作统一归属本地占位用户(只本地 sqlite 持久化)。
    pub fn personal_drive(&self, ctx: &AppContext) -> Option<Owner> {
        let _ = ctx;
        Some(Owner::User {
            user_uid: Self::effective_personal_user_uid(),
        })
    }

    // Maps a [`Space`] into an [`Owner`], based on the user's team memberships. If the space
    // does not directly identify an owner (it's the space for shared objects), returns `None`.
    pub fn space_to_owner(&self, space: Space, ctx: &AppContext) -> Option<Owner> {
        match space {
            Space::Team { .. } => None,
            Space::Personal => self.personal_drive(ctx),
            Space::Shared => None,
        }
    }

    // Maps an [`Owner`] into a [`Space`], based on the user's team memberships.
    // This is always possible, as unknown owners imply the shared space.
    pub fn owner_to_space(&self, owner: Owner, ctx: &AppContext) -> Space {
        let _ = ctx;
        match owner {
            Owner::User { user_uid } => {
                if !FeatureFlag::SharedWithMe.is_enabled() {
                    return Space::Personal;
                }

                // Zap:用 effective_personal_user_uid 比较,确保无 auth 下
                // 本地 Owner(user_uid="zap")也归到 Personal 而非 Shared。
                if user_uid == Self::effective_personal_user_uid() {
                    Space::Personal
                } else {
                    Space::Shared
                }
            }
            Owner::Team { .. } => Space::Shared,
        }
    }

    pub fn has_teams(&self) -> bool {
        false
    }

    pub fn has_workspaces(&self) -> bool {
        !self.workspaces.is_empty()
    }

    pub fn update_workspaces(&mut self, workspaces: Vec<Workspace>, ctx: &mut ModelContext<Self>) {
        // Check if sunsetted_to_build_ts changed for any workspace
        let sunsetted_to_build_changed = self.has_sunsetted_to_build_data_changed(&workspaces);

        *self.workspaces = workspaces;
        let reassigned_windows = self.reconcile_window_team_assignments();
        self.notify_and_emit_teams_changed(ctx);
        Self::emit_window_team_changed(reassigned_windows, ctx);

        if sunsetted_to_build_changed {
            ctx.emit(UserWorkspacesEvent::SunsettedToBuildDataUpdated);
        }
    }

    /// Checks if any workspace's service agreement sunsetted_to_build_ts field has changed.
    fn has_sunsetted_to_build_data_changed(&self, new_workspaces: &[Workspace]) -> bool {
        for new_workspace in new_workspaces {
            // Find the corresponding old workspace
            let old_workspace = self.workspaces.iter().find(|w| w.uid == new_workspace.uid);

            if let Some(old_workspace) = old_workspace {
                // Check if any team's service agreement sunsetted_to_build_ts changed
                for new_team in &new_workspace.teams {
                    let old_team = old_workspace.teams.iter().find(|t| t.uid == new_team.uid);

                    if let Some(old_team) = old_team {
                        let old_sunsetted = old_team
                            .billing_metadata
                            .service_agreements
                            .first()
                            .and_then(|sa| sa.sunsetted_to_build_ts);

                        let new_sunsetted = new_team
                            .billing_metadata
                            .service_agreements
                            .first()
                            .and_then(|sa| sa.sunsetted_to_build_ts);

                        // Detect if it changed from None to Some or changed value
                        if old_sunsetted != new_sunsetted {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn notify_and_emit_teams_changed(&self, ctx: &mut ModelContext<Self>) {
        // PrivacySettings can't observe UserWorkspaces for updates, as it's initialized too early in
        // the app initialization flow. So, we update it manually whenever teams data changes.
        PrivacySettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.set_is_telemetry_force_enabled(false);
            settings.set_enterprise_secret_redaction_settings(
                self.is_enterprise_secret_redaction_enabled(),
                self.get_enterprise_secret_redaction_regex_list(),
                ChangeEventReason::CloudSync,
                ctx,
            );
        });

        ctx.emit(UserWorkspacesEvent::TeamsChanged);
        ctx.notify();
    }

    pub fn team_created(
        &mut self,
        create_team_response: &CreateTeamResponse,
        ctx: &mut ModelContext<Self>,
    ) {
        self.workspaces.push(create_team_response.workspace.clone());
        self.set_current_workspace_uid(create_team_response.workspace.uid, ctx);
        self.notify_and_emit_teams_changed(ctx);
    }

    pub fn remove_user_from_team(
        &mut self,
        user_uid: UserUid,
        team_uid: ServerId,
        entrypoint: StoredObjectEventEntrypoint,
        _ctx: &mut ModelContext<Self>,
    ) {
        // Zap(本地化):移除成员路径在本地无远端 team 写入目标 → no-op。
        let _ = (user_uid, team_uid, entrypoint);
    }

    pub fn add_invite_link_domain_restrictions(
        &mut self,
        team_uid: ServerId,
        domains: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Zap(本地化):域限制路径在本地无远端 team/invite 写入目标 → 发 Success 事件使 UI 不卡住。
        let _ = (team_uid, domains);
        ctx.emit(UserWorkspacesEvent::AddDomainRestrictionsSuccess);
        ctx.notify();
    }

    pub fn delete_invite_link_domain_restriction(
        &mut self,
        team_uid: ServerId,
        domain_uid: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (team_uid, domain_uid);
        ctx.emit(UserWorkspacesEvent::DeleteDomainRestrictionSuccess);
        ctx.notify();
    }

    pub fn send_email_invites(
        &mut self,
        team_uid: ServerId,
        emails: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (team_uid, emails);
        ctx.emit(UserWorkspacesEvent::EmailInviteSent);
        ctx.notify();
    }

    pub fn set_is_invite_link_enabled(
        &mut self,
        team_uid: ServerId,
        new_value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (team_uid, new_value);
        ctx.emit(UserWorkspacesEvent::ToggleInviteLinksSuccess);
        ctx.notify();
    }

    pub fn reset_invite_links(&mut self, team_uid: ServerId, ctx: &mut ModelContext<Self>) {
        let _ = team_uid;
        ctx.emit(UserWorkspacesEvent::ResetInviteLinks);
        ctx.notify();
    }

    pub fn set_team_member_role(
        &mut self,
        user_uid: UserUid,
        team_uid: ServerId,
        role: MembershipRole,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (user_uid, team_uid, role);
        ctx.emit(UserWorkspacesEvent::SetTeamMemberRoleSuccess);
        ctx.notify();
    }

    pub fn delete_team_invite(
        &mut self,
        team_uid: ServerId,
        invitee_email: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (team_uid, invitee_email);
        ctx.emit(UserWorkspacesEvent::DeleteTeamInvite);
        ctx.notify();
    }

    /// Zap(本地化):没有云端 Stripe 账单门户 → no-op(调用点只是打开设置页按钮)。
    pub fn generate_stripe_billing_portal_link(
        &mut self,
        team_uid: ServerId,
        _ctx: &mut ModelContext<Self>,
    ) {
        let _ = team_uid;
    }

    /// Zap(本地化):没有云端计费通道,直接回拒,避免购买按钮永远停在 loading 态。
    pub fn purchase_addon_credits(
        &mut self,
        team_uid: Option<ServerId>,
        credits: i32,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (team_uid, credits);
        ctx.emit(UserWorkspacesEvent::PurchaseAddonCreditsRejected(
            anyhow::anyhow!("InfiniShell 本地版没有云端计费通道,无法购买 add-on credits"),
        ));
        ctx.notify();
    }

    /// Zap(本地化):无远端 workspace 设置写入目标 → 发 Success 事件使 UI 不卡住。
    pub fn update_addon_credits_settings(
        &mut self,
        team_uid: ServerId,
        auto_reload_enabled: Option<bool>,
        max_monthly_spend_cents: Option<i32>,
        selected_auto_reload_credit_denomination: Option<i32>,
        ctx: &mut ModelContext<Self>,
    ) {
        let _ = (
            team_uid,
            auto_reload_enabled,
            max_monthly_spend_cents,
            selected_auto_reload_credit_denomination,
        );
        ctx.emit(UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess);
        ctx.notify();
    }

    pub fn usage_based_pricing_settings(&self) -> UsageBasedPricingSettings {
        self.current_workspace()
            .map(|workspace| workspace.settings.usage_based_pricing_settings.clone())
            .unwrap_or_default()
    }

    pub fn is_telemetry_force_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.telemetry_settings.force_enabled)
            .unwrap_or(false)
    }

    pub fn refresh_ai_overages(&mut self, _ctx: &mut ModelContext<Self>) {
        // Zap(本地化,Phase 5):本地无云端 AI overages 查询,no-op。
        // 调用点 (`blocklist/controller.rs::maybe_refresh_ai_overages`) UI 不发起有意义的更新。
    }

    pub fn is_enterprise_secret_redaction_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.secret_redaction_settings.enabled)
            .unwrap_or(false)
    }

    pub fn get_enterprise_secret_redaction_regex_list(&self) -> Vec<EnterpriseSecretRegex> {
        self.current_workspace()
            .map(|workspace| workspace.settings.secret_redaction_settings.regexes.clone())
            .unwrap_or_default()
    }

    pub fn get_ugc_collection_enablement_setting(&self) -> UgcCollectionEnablementSetting {
        self.current_workspace()
            .map(|workspace| workspace.settings.ugc_collection_settings.setting.clone())
            .unwrap_or_default()
    }

    /// Zap 没有托管组织策略,云端会话存储完全由用户自己的隐私开关决定,
    /// 所以这里恒为 `RespectUserSetting`(调用点据此判断开关是否可编辑)。
    pub fn get_cloud_conversation_storage_enablement_setting(&self) -> AdminEnablementSetting {
        AdminEnablementSetting::RespectUserSetting
    }

    pub fn is_ai_allowed_in_remote_sessions(&self) -> bool {
        // Zap 没有托管组织策略，远程 SSH 会话始终允许使用本地 Agent 能力。
        true
    }

    pub fn get_remote_session_regex_list(&self) -> Vec<Regex> {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .ai_permissions_settings
                    .remote_session_regex_list
                    .clone()
            })
            .unwrap_or_default()
    }

    pub fn is_anyone_with_link_sharing_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .link_sharing_settings
                    .anyone_with_link_sharing_enabled
            })
            .unwrap_or(false)
    }

    pub fn is_direct_link_sharing_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| {
                workspace
                    .settings
                    .link_sharing_settings
                    .direct_link_sharing_enabled
            })
            .unwrap_or(true)
    }

    /// Whether invite links are enabled for the current workspace. This is a
    /// workspace-level setting; the teams-settings page reads it from here rather
    /// than from the `Team` struct.
    pub fn is_invite_link_enabled(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.is_invite_link_enabled)
            .unwrap_or(false)
    }

    /// Whether the current workspace's team is discoverable. This is a
    /// workspace-level setting; the teams-settings page reads it from here rather
    /// than from the `Team` struct.
    pub fn is_discoverable(&self) -> bool {
        self.current_workspace()
            .map(|workspace| workspace.settings.is_discoverable)
            .unwrap_or(false)
    }

    /// Returns the codebase context settings, taking into account the organization,
    /// global AI settings, and codebase-specific settings.
    /// Prefer this function to determine whether to show indexing-related functionality.
    pub fn is_codebase_context_enabled(&self, app: &AppContext) -> bool {
        // If the organization has an explicit setting, respect it and make user toggle irrelevant.
        // - Enable: forced ON by org, regardless of user preference.
        // - Disable: forced OFF by org.
        // - RespectUserSetting: respect the user setting.
        let org_setting = self.team_allows_codebase_context();
        let ai_globally_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        match org_setting {
            AdminEnablementSetting::Enable => ai_globally_enabled,
            AdminEnablementSetting::Disable => false,
            AdminEnablementSetting::RespectUserSetting => {
                ai_globally_enabled && *CodeSettings::as_ref(app).codebase_context_enabled.value()
            }
        }
    }

    pub fn default_host_slug(&self) -> Option<&str> {
        self.current_workspace()
            .and_then(|workspace| workspace.settings.default_host_slug.as_deref())
    }

    /// Returns the team-level agent attribution setting.
    ///
    /// Use this to decide whether the user's attribution toggle should be locked
    /// (`Enable`/`Disable`) or editable (`RespectUserSetting`).
    pub fn get_agent_attribution_setting(&self) -> AdminEnablementSetting {
        self.current_workspace()
            .map(|workspace| workspace.settings.enable_warp_attribution.clone())
            .unwrap_or_default()
    }

    /// Returns only the organization-specific codebase context enablement setting.
    /// Do not use this function to determine whether codebase context is generally enabled --
    /// use `is_codebase_context_enabled` instead.
    ///
    /// Zap 没有托管组织策略,所以恒为 `RespectUserSetting`:codebase context 完全由
    /// 用户自己的 `CodeSettings` 开关决定。
    pub fn team_allows_codebase_context(&self) -> AdminEnablementSetting {
        AdminEnablementSetting::RespectUserSetting
    }
}

#[cfg(test)]
impl UserWorkspaces {
    /// Creates a test workspace with a team and sets it as the current workspace.
    /// Returns the workspace UID and admin UID for use in tests.
    pub fn setup_test_workspace(&mut self, ctx: &mut ModelContext<Self>) {
        let workspace_uid = WorkspaceUid::from(ServerId::from(1));
        let owner_uid = UserUid::new("test_owner");

        let workspace_settings = WorkspaceSettings::default();

        let workspace = Workspace {
            uid: workspace_uid,
            name: "Test Workspace".to_string(),
            stripe_customer_id: None,
            teams: vec![Team {
                uid: ServerId::from(2),
                name: "Test Team".to_string(),
                settings: Default::default(),
                color: None,
                billing_metadata: BillingMetadata::default(),
                members: vec![],
                invite_code: None,
                pending_email_invites: vec![],
                invite_link_domain_restrictions: vec![],
                stripe_customer_id: None,
                is_eligible_for_discovery: false,
                has_billing_history: false,
            }],
            members: vec![WorkspaceMember {
                uid: owner_uid,
                email: "test@example.com".to_string(),
                role: MembershipRole::Owner,
                usage_info: WorkspaceMemberUsageInfo {
                    requests_used_since_last_refresh: 0,
                    request_limit: 1000,
                    is_unlimited: false,
                    is_request_limit_prorated: false,
                },
            }],
            billing_metadata: BillingMetadata::default(),
            bonus_grants_purchased_this_month: Default::default(),
            billing_cycle_usage: None,
            has_billing_history: false,
            settings: workspace_settings,
            invite_code: None,
            invite_link_domain_restrictions: vec![],
            pending_email_invites: vec![],
            is_eligible_for_discovery: false,
            total_requests_used_since_last_refresh: 0,
        };

        self.update_workspaces(vec![workspace], ctx);
        self.set_current_workspace_uid(workspace_uid, ctx);
    }

    /// Updates the current workspace by applying a mutation function.
    pub fn update_current_workspace<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut Workspace),
    {
        if let Some(workspace) = self.current_workspace() {
            if workspace.teams.is_empty() {
                panic!("No team found in current workspace. Did you call setup_test_workspace()?");
            }

            let mut new_workspace = workspace.clone();
            f(&mut new_workspace);

            self.update_workspaces(vec![new_workspace], ctx);
        } else {
            panic!("No workspace found. Did you call setup_test_workspace()?");
        }
    }

    pub fn update_sandboxed_agent_settings<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut Option<SandboxedAgentSettings>),
    {
        self.update_current_workspace(
            |workspace| {
                f(&mut workspace.settings.sandboxed_agent_settings);
            },
            ctx,
        );
    }

    pub fn update_ai_autonomy_settings<F>(&mut self, f: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(&mut AiAutonomySettings),
    {
        self.update_current_workspace(
            |workspace| {
                f(&mut workspace.settings.ai_autonomy_settings);
            },
            ctx,
        );
    }

    pub fn update_ai_autonomy_policy_flag(&mut self, enabled: bool, ctx: &mut ModelContext<Self>) {
        self.update_current_workspace(
            |workspace| {
                if let Some(team) = workspace.teams.first_mut() {
                    team.billing_metadata.tier.ai_autonomy_policy = Some(AIAutonomyPolicy {
                        is_enabled: enabled,
                        toggleable: true,
                    });
                } else {
                    panic!(
                        "No team found in current workspace. Did you call setup_test_workspace()?"
                    );
                }
            },
            ctx,
        );
    }
}

impl Entity for UserWorkspaces {
    type Event = UserWorkspacesEvent;
}

/// Mark UserWorkspaces as global application state.
impl SingletonEntity for UserWorkspaces {}

// Zap(本地化,Phase 5):`user_workspaces_tests.rs` 全部针对 team RPC 路径(`MockTeamClient` / `mockall::Sequence`),
// 本地化后这些路径不可达，整文件物理删除。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_team_allows_ai_in_remote_sessions() {
        let workspaces = UserWorkspaces::new(vec![], None);

        assert!(workspaces.is_ai_allowed_in_remote_sessions());
    }
}
