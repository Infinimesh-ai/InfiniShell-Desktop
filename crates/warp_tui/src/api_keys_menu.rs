//! Stateful `/api-keys` inline menu backed by the shared TUI input editor.

use ai::LLMProvider;
use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent};
use ai::grok_subscription::oauth::OauthAttempt;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::UserWorkspaces;
use warp_core::features::FeatureFlag;
use warp_core::settings::ToggleableSetting as _;
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::grok_oauth::{TuiGrokOAuthController, TuiGrokOAuthControllerEvent};
use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuInputOwnership, TuiInlineMenuListState,
    TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuScrollAnchor, TuiInlineMenuSnapshot,
    TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{
    TuiInputSuggestionsMode, TuiInputSuggestionsModeEvent, TuiInputSuggestionsModeModel,
};
use crate::tui_builder::TuiUiBuilder;

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);
const PROVIDER_ROWS: [TuiApiKeysRow; 4] = [
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::Provider(LLMProvider::Anthropic),
    },
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::Provider(LLMProvider::Google),
    },
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::Provider(LLMProvider::OpenAI),
    },
    TuiApiKeysRow {
        kind: TuiApiKeysRowKind::Provider(LLMProvider::Xai),
    },
];
const FALLBACK_ROW: TuiApiKeysRow = TuiApiKeysRow {
    kind: TuiApiKeysRowKind::WarpCreditFallbackSetting,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiApiKeysRowKind {
    Provider(LLMProvider),
    WarpCreditFallbackSetting,
}

#[derive(Debug, Clone, Copy)]
struct TuiApiKeysRow {
    kind: TuiApiKeysRowKind,
}

fn api_key_row_title(kind: TuiApiKeysRowKind) -> String {
    match kind {
        TuiApiKeysRowKind::Provider(LLMProvider::Anthropic) => {
            warp::t!("tui-api-key-anthropic")
        }
        TuiApiKeysRowKind::Provider(LLMProvider::Google) => warp::t!("tui-api-key-google"),
        TuiApiKeysRowKind::Provider(LLMProvider::OpenAI) => warp::t!("tui-api-key-openai"),
        TuiApiKeysRowKind::Provider(LLMProvider::Xai) => warp::t!("tui-api-key-grok"),
        TuiApiKeysRowKind::Provider(provider @ LLMProvider::Unknown) => warp::t!(
            "tui-api-key-provider-title",
            provider = provider.display_name()
        ),
        TuiApiKeysRowKind::WarpCreditFallbackSetting => {
            warp::t!("tui-api-key-warp-credit-fallback")
        }
    }
}

#[derive(Default)]
enum TuiApiKeysMenuState {
    #[default]
    Closed,
    Browsing {
        list: TuiInlineMenuListState<TuiApiKeysRow>,
        error: Option<String>,
    },
    EditingProvider {
        provider: LLMProvider,
        error: Option<String>,
    },
    ConnectingGrok {
        controller: ModelHandle<TuiGrokOAuthController>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiApiKeysFooter {
    ProviderList { can_clear: bool },
    WarpCreditFallback,
    EditingProvider(LLMProvider),
    ConnectingGrok,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiApiKeysMenuEvent;

pub(crate) struct TuiApiKeysMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    state: TuiApiKeysMenuState,
}

impl TuiApiKeysMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                return;
            }
            match &mut model.state {
                TuiApiKeysMenuState::Browsing { error, .. } => {
                    error.take();
                    model.refresh_rows(ctx);
                }
                TuiApiKeysMenuState::EditingProvider { error, .. } => {
                    if error.take().is_some() {
                        ctx.emit(TuiApiKeysMenuEvent);
                    }
                }
                TuiApiKeysMenuState::ConnectingGrok { controller } => {
                    controller.update(ctx, |controller, ctx| {
                        controller.clear_manual_error(ctx);
                    });
                }
                TuiApiKeysMenuState::Closed => {}
            }
        });
        ctx.subscribe_to_model(
            &ApiKeyManager::handle(ctx),
            |model, _, _: &ApiKeyManagerEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            },
        );
        ctx.subscribe_to_model(&AISettings::handle(ctx), |model, _, event, ctx| {
            if model.is_open(ctx)
                && matches!(
                    event,
                    AISettingsChangedEvent::CanUseWarpCreditsForFallback { .. }
                )
            {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(
            &suggestions_mode,
            |model, _, event: &TuiInputSuggestionsModeEvent, ctx| {
                if event.mode != TuiInputSuggestionsMode::ApiKeys {
                    model.deactivate(ctx);
                }
            },
        );
        Self {
            input_editor,
            suggestions_mode,
            state: TuiApiKeysMenuState::Closed,
        }
    }

    fn start_grok_oauth(&mut self, ctx: &mut ModelContext<Self>) {
        let workspaces = UserWorkspaces::as_ref(ctx);
        let policy_error = if !FeatureFlag::SuperGrok.is_enabled() {
            Some(warp::t!("tui-api-key-grok-build-unavailable"))
        } else if !workspaces.is_byo_api_key_enabled(ctx) {
            Some(warp::t!("tui-api-key-grok-byok-required"))
        } else if !workspaces.are_member_byo_keys_allowed() {
            Some(warp::t!("tui-api-key-member-credentials-disallowed"))
        } else {
            None
        };
        if let Some(error) = policy_error {
            self.set_browsing_error(error, ctx);
            return;
        }
        let attempt = match OauthAttempt::start() {
            Ok(attempt) => attempt,
            Err(error) => {
                self.set_browsing_error(error.to_string(), ctx);
                return;
            }
        };
        self.clear_input(ctx);
        let controller = ctx.add_model(move |ctx| TuiGrokOAuthController::new(attempt, ctx));
        ctx.subscribe_to_model(&controller, |menu, _, event, ctx| match event {
            TuiGrokOAuthControllerEvent::Connected => menu.transition_to_browsing(ctx),
            TuiGrokOAuthControllerEvent::Updated => ctx.emit(TuiApiKeysMenuEvent),
        });
        self.state = TuiApiKeysMenuState::ConnectingGrok { controller };
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        !matches!(self.state, TuiApiKeysMenuState::Closed)
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::ApiKeys
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::ApiKeys, ctx)
        });
        if !did_open {
            return;
        }
        self.transition_to_browsing(ctx);
    }

    /// Opens the menu and jumps straight into the Grok connect path, equivalent to selecting the
    /// "X premium or SuperGrok subscription" row. Reuses `edit_provider` so the already-connected
    /// and policy-gated cases surface the same messaging as the provider list.
    pub(crate) fn open_and_connect_grok(&mut self, ctx: &mut ModelContext<Self>) {
        self.open(ctx);
        if self.is_open(ctx) {
            self.edit_provider(LLMProvider::Xai, ctx);
        }
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        match self.state {
            TuiApiKeysMenuState::Closed => {}
            TuiApiKeysMenuState::Browsing { .. } => self.close(ctx),
            TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::ConnectingGrok { .. } => self.transition_to_browsing(ctx),
        }
    }

    /// Returns the shared editor owner for the active API keys state.
    pub(crate) fn input_ownership(&self, ctx: &AppContext) -> TuiInlineMenuInputOwnership {
        if !self.is_open(ctx) {
            return TuiInlineMenuInputOwnership::Composer;
        }
        match self.state {
            TuiApiKeysMenuState::EditingProvider { .. } => {
                TuiInlineMenuInputOwnership::InlineMenuMasked
            }
            TuiApiKeysMenuState::Browsing { .. } | TuiApiKeysMenuState::ConnectingGrok { .. } => {
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            }
            TuiApiKeysMenuState::Closed => TuiInlineMenuInputOwnership::Composer,
        }
    }

    pub(crate) fn uses_credential_border(&self, ctx: &AppContext) -> bool {
        self.is_open(ctx)
            && matches!(
                self.state,
                TuiApiKeysMenuState::EditingProvider { .. }
                    | TuiApiKeysMenuState::ConnectingGrok { .. }
            )
    }

    pub(crate) fn footer(&self, ctx: &AppContext) -> Option<TuiApiKeysFooter> {
        if !self.is_open(ctx) {
            return None;
        }
        match &self.state {
            TuiApiKeysMenuState::Closed => None,
            TuiApiKeysMenuState::Browsing { list, .. } => {
                Some(match list.selected_row().map(|row| row.kind) {
                    Some(TuiApiKeysRowKind::WarpCreditFallbackSetting) => {
                        TuiApiKeysFooter::WarpCreditFallback
                    }
                    Some(TuiApiKeysRowKind::Provider(provider)) => TuiApiKeysFooter::ProviderList {
                        can_clear: provider_connected(provider, ctx),
                    },
                    None => TuiApiKeysFooter::ProviderList { can_clear: false },
                })
            }
            TuiApiKeysMenuState::EditingProvider { provider, .. } => {
                Some(TuiApiKeysFooter::EditingProvider(*provider))
            }
            TuiApiKeysMenuState::ConnectingGrok { .. } => Some(TuiApiKeysFooter::ConnectingGrok),
        }
    }

    pub(crate) fn can_clear_selected(&self, ctx: &AppContext) -> bool {
        match &self.state {
            TuiApiKeysMenuState::Browsing { list, .. } => {
                match list.selected_row().map(|row| row.kind) {
                    Some(TuiApiKeysRowKind::Provider(provider)) => {
                        provider_connected(provider, ctx)
                    }
                    Some(TuiApiKeysRowKind::WarpCreditFallbackSetting) | None => false,
                }
            }
            TuiApiKeysMenuState::Closed
            | TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::ConnectingGrok { .. } => false,
        }
    }

    pub(crate) fn clear_selected(&mut self, ctx: &mut ModelContext<Self>) {
        let provider = match &self.state {
            TuiApiKeysMenuState::Browsing { list, .. } => {
                match list.selected_row().map(|row| row.kind) {
                    Some(TuiApiKeysRowKind::Provider(provider)) => provider,
                    Some(TuiApiKeysRowKind::WarpCreditFallbackSetting) | None => return,
                }
            }
            TuiApiKeysMenuState::Closed
            | TuiApiKeysMenuState::EditingProvider { .. }
            | TuiApiKeysMenuState::ConnectingGrok { .. } => return,
        };
        let result = if provider == LLMProvider::Xai {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.set_grok_tokens(None, ctx);
            });
            Ok(())
        } else {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.persist_provider_key(provider, None, ctx)
            })
        };
        match result {
            Ok(()) => self.refresh_rows(ctx),
            Err(_) => self.set_browsing_error(warp::t!("tui-api-key-clear-failed"), ctx),
        }
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiApiKeysMenuEvent);
        selected
    }

    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            return;
        };
        list.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    pub(crate) fn accept_selected(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.state {
            TuiApiKeysMenuState::Closed => {}
            TuiApiKeysMenuState::Browsing { list, .. } => {
                let Some(kind) = list.selected_row().map(|row| row.kind) else {
                    return;
                };
                match kind {
                    TuiApiKeysRowKind::Provider(provider) => self.edit_provider(provider, ctx),
                    TuiApiKeysRowKind::WarpCreditFallbackSetting => self.toggle_fallback(ctx),
                }
            }
            TuiApiKeysMenuState::EditingProvider { provider, .. } => {
                let provider = *provider;
                self.save_provider(provider, ctx);
            }
            TuiApiKeysMenuState::ConnectingGrok { controller } => {
                let controller = controller.clone();
                let code = input_text(&self.input_editor, ctx);
                if !code.trim().is_empty() {
                    self.clear_input(ctx);
                }
                controller.update(ctx, |controller, ctx| {
                    controller.submit_manual_code(code, ctx);
                });
            }
        }
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        match &self.state {
            TuiApiKeysMenuState::Closed => None,
            TuiApiKeysMenuState::Browsing { list, error } => Some(TuiInlineMenuSnapshot {
                header: Some(TuiInlineMenuHeader {
                    title: Some(error.clone().unwrap_or_else(|| warp::t!("tui-api-keys"))),
                    tabs: Vec::new(),
                }),
                rows: list
                    .rows()
                    .iter()
                    .map(|row| self.snapshot_row(row, ctx, false))
                    .collect(),
                selected_index: list.selected_index(),
                scroll_offset: list.scroll_offset(),
                scroll_anchor: list.scroll_anchor(),
                max_visible_rows: MAX_VISIBLE_ROWS,
                status: None,
            }),
            TuiApiKeysMenuState::EditingProvider { provider, error } => {
                Some(TuiInlineMenuSnapshot {
                    header: Some(TuiInlineMenuHeader {
                        title: Some(warp::t!(
                            "tui-api-key-provider-title",
                            provider = provider.display_name()
                        )),
                        tabs: Vec::new(),
                    }),
                    rows: Vec::new(),
                    selected_index: None,
                    scroll_offset: 0,
                    scroll_anchor: TuiInlineMenuScrollAnchor::Selection,
                    max_visible_rows: MAX_VISIBLE_ROWS,
                    status: error.clone().map(TuiInlineMenuStatus::Empty),
                })
            }
            TuiApiKeysMenuState::ConnectingGrok { controller } => {
                let error = controller.as_ref(ctx).error().map(ToOwned::to_owned);
                let rows = PROVIDER_ROWS
                    .into_iter()
                    .chain(std::iter::once(FALLBACK_ROW))
                    .map(|row| self.snapshot_row(&row, ctx, true))
                    .collect();
                Some(TuiInlineMenuSnapshot {
                    header: Some(TuiInlineMenuHeader {
                        title: Some(error.unwrap_or_else(|| warp::t!("tui-api-keys"))),
                        tabs: Vec::new(),
                    }),
                    rows,
                    selected_index: Some(3),
                    scroll_offset: 0,
                    scroll_anchor: TuiInlineMenuScrollAnchor::Selection,
                    max_visible_rows: MAX_VISIBLE_ROWS,
                    status: None,
                })
            }
        }
    }

    fn snapshot_row(
        &self,
        row: &TuiApiKeysRow,
        ctx: &AppContext,
        connecting_grok: bool,
    ) -> TuiInlineMenuRow {
        let (description, state_suffix, is_selectable) = match row.kind {
            TuiApiKeysRowKind::Provider(provider) => {
                let connected = provider_connected(provider, ctx);
                let suffix = if connecting_grok && provider == LLMProvider::Xai {
                    warp::t!("tui-api-key-connecting")
                } else if connected {
                    warp::t!("tui-api-key-connected")
                } else {
                    warp::t!("tui-api-key-not-connected")
                };
                (Some(String::new()), Some(suffix), !connecting_grok)
            }
            TuiApiKeysRowKind::WarpCreditFallbackSetting => (
                Some(warp::t!("tui-api-key-warp-credit-fallback-description")),
                Some(
                    if *AISettings::as_ref(ctx).can_use_warp_credits_for_fallback {
                        warp::t!("tui-state-on-parenthesized")
                    } else {
                        warp::t!("tui-state-off-parenthesized")
                    },
                ),
                !connecting_grok,
            ),
        };
        let style = if row.kind == TuiApiKeysRowKind::WarpCreditFallbackSetting {
            TuiInlineMenuRowStyle::StateWithDetail
        } else {
            TuiInlineMenuRowStyle::InlineMenuItem
        };
        TuiInlineMenuRow {
            title: api_key_row_title(row.kind),
            prefix: None,
            description,
            state_suffix,
            promotional_suffix: None,
            is_selectable,
            style,
        }
    }

    fn edit_provider(&mut self, provider: LLMProvider, ctx: &mut ModelContext<Self>) {
        if provider == LLMProvider::Xai {
            if ApiKeyManager::as_ref(ctx).has_grok_subscription() {
                self.set_browsing_error(warp::t!("tui-api-key-grok-already-connected"), ctx);
            } else {
                self.start_grok_oauth(ctx);
            }
            return;
        }
        let key = provider
            .api_key(ApiKeyManager::as_ref(ctx).keys())
            .unwrap_or_default()
            .to_owned();
        self.set_input(&key, ctx);
        self.state = TuiApiKeysMenuState::EditingProvider {
            provider,
            error: None,
        };
        ctx.emit(TuiApiKeysMenuEvent);
    }

    fn save_provider(&mut self, provider: LLMProvider, ctx: &mut ModelContext<Self>) {
        let value = input_text(&self.input_editor, ctx);
        let value = (!value.is_empty()).then_some(value);
        match ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.persist_provider_key(provider, value, ctx)
        }) {
            Ok(()) => self.transition_to_browsing(ctx),
            Err(_) => {
                if let TuiApiKeysMenuState::EditingProvider { error, .. } = &mut self.state {
                    *error = Some(warp::t!("tui-api-key-save-failed"));
                }
                ctx.emit(TuiApiKeysMenuEvent);
            }
        }
    }

    fn toggle_fallback(&mut self, ctx: &mut ModelContext<Self>) {
        let result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .can_use_warp_credits_for_fallback
                .toggle_and_save_value(ctx)
        });
        match result {
            Ok(_) => self.refresh_rows(ctx),
            Err(_) => self.set_browsing_error(warp::t!("tui-api-key-fallback-save-failed"), ctx),
        }
    }

    fn transition_to_browsing(&mut self, ctx: &mut ModelContext<Self>) {
        if let TuiApiKeysMenuState::ConnectingGrok { controller } = &self.state
            && controller.as_ref(ctx).is_active()
        {
            controller.update(ctx, |controller, ctx| controller.cancel(ctx));
        }
        self.clear_input(ctx);
        self.state = TuiApiKeysMenuState::Browsing {
            list: TuiInlineMenuListState::default(),
            error: None,
        };
        self.refresh_rows(ctx);
    }

    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        self.deactivate(ctx);
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::ApiKeys, ctx);
        });
    }

    /// Clears API-key-specific state when shared menu arbitration moves elsewhere.
    fn deactivate(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.state, TuiApiKeysMenuState::Closed) {
            return;
        }
        let grok_controller = match &self.state {
            TuiApiKeysMenuState::ConnectingGrok { controller } => Some(controller.clone()),
            TuiApiKeysMenuState::Closed
            | TuiApiKeysMenuState::Browsing { .. }
            | TuiApiKeysMenuState::EditingProvider { .. } => None,
        };
        self.state = TuiApiKeysMenuState::Closed;
        if let Some(controller) = grok_controller
            && controller.as_ref(ctx).is_active()
        {
            controller.update(ctx, |controller, ctx| controller.cancel(ctx));
        }
        self.clear_input(ctx);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiApiKeysMenuState::Browsing { list, .. } = &mut self.state else {
            ctx.emit(TuiApiKeysMenuEvent);
            return;
        };
        let query = input_text(&self.input_editor, ctx).to_ascii_lowercase();
        let rows = PROVIDER_ROWS
            .into_iter()
            .filter(|row| {
                api_key_row_title(row.kind)
                    .to_ascii_lowercase()
                    .contains(&query)
            })
            .chain(std::iter::once(FALLBACK_ROW))
            .collect();
        let previous_kind = list.selected_row().map(|row| row.kind);
        let preferred_index = previous_kind
            .and_then(|kind| {
                let rows: &Vec<TuiApiKeysRow> = &rows;
                rows.iter().position(|row| row.kind == kind)
            })
            .or(Some(0));
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiApiKeysMenuEvent);
    }

    fn set_browsing_error(&mut self, message: String, ctx: &mut ModelContext<Self>) {
        if let TuiApiKeysMenuState::Browsing { error, .. } = &mut self.state {
            *error = Some(message);
            ctx.emit(TuiApiKeysMenuEvent);
        }
    }

    fn clear_input(&self, ctx: &mut ModelContext<Self>) {
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
    }

    fn set_input(&self, text: &str, ctx: &mut ModelContext<Self>) {
        self.input_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
            editor.user_insert(text, ctx);
        });
    }
}

/// Returns whether the provider has a configured API key or connected subscription.
fn provider_connected(provider: LLMProvider, ctx: &AppContext) -> bool {
    if provider == LLMProvider::Xai {
        ApiKeyManager::as_ref(ctx).has_grok_subscription()
    } else {
        provider
            .api_key(ApiKeyManager::as_ref(ctx).keys())
            .is_some_and(|key| !key.is_empty())
    }
}

/// Returns the shared input editor's current text.
fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

/// Renders the footer actions for the current API keys menu state.
pub(crate) fn render_api_keys_footer(
    footer: TuiApiKeysFooter,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let key = builder.link_text_style();
    let muted = builder.muted_text_style();
    let accent = builder.credential_entry_accent_style();
    let spans = match footer {
        TuiApiKeysFooter::ProviderList { can_clear } => {
            let mut spans = vec![
                ("enter".to_owned(), key),
                (format!(" {} | ", warp::t!("tui-hint-set-api-key")), muted),
            ];
            if can_clear {
                spans.extend([
                    ("ctrl + x".to_owned(), key),
                    (format!(" {} | ", warp::t!("tui-hint-clear-api-key")), muted),
                ]);
            }
            spans.extend([
                ("esc".to_owned(), key),
                (format!(" {}", warp::t!("tui-hint-close-menu")), muted),
            ]);
            spans
        }
        TuiApiKeysFooter::WarpCreditFallback => vec![
            ("enter".to_owned(), key),
            (
                format!(" {} | ", warp::t!("tui-hint-toggle-warp-credit-fallback")),
                muted,
            ),
            ("esc".to_owned(), key),
            (format!(" {}", warp::t!("tui-hint-close-menu")), muted),
        ],
        TuiApiKeysFooter::EditingProvider(provider) => vec![
            (
                warp::t!(
                    "tui-api-key-connect-provider",
                    provider = provider.display_name()
                ),
                accent,
            ),
            (" | ".to_owned(), muted),
            ("enter".to_owned(), key),
            (format!(" {} | ", warp::t!("tui-hint-save-key")), muted),
            ("esc".to_owned(), key),
            (format!(" {}", warp::t!("tui-hint-cancel")), muted),
        ],
        TuiApiKeysFooter::ConnectingGrok => vec![
            (warp::t!("tui-api-key-connect-grok"), accent),
            (" | ".to_owned(), muted),
            ("enter".to_owned(), key),
            (format!(" {} | ", warp::t!("tui-hint-confirm")), muted),
            ("esc".to_owned(), key),
            (format!(" {}", warp::t!("tui-hint-cancel")), muted),
        ],
    };
    TuiText::from_spans(spans).truncate().finish()
}
impl Entity for TuiApiKeysMenuModel {
    type Event = TuiApiKeysMenuEvent;
}

#[cfg(test)]
#[path = "api_keys_menu_tests.rs"]
mod tests;
