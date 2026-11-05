use warpui::keymap::Keystroke;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView, ViewHandle};

use super::{FeaturesPageAction, FeaturesPageView};
use crate::GlobalResourceHandlesProvider;
use crate::default_terminal::DefaultTerminal;
use crate::gpu_state::GPUState;
use crate::settings::TabBehavior;
use crate::settings_view::DisplayCount;
use crate::settings_view::keybindings::{KeybindingsView, KeybindingsViewAction};
use crate::settings_view::settings_page::SettingsPageMeta;
use crate::terminal::input::OPEN_COMPLETIONS_KEYBINDING_NAME;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn add_features_page(app: &mut App) -> ViewHandle<FeaturesPageView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| DisplayCount::mock());
    app.add_singleton_model(DefaultTerminal::new);
    app.add_singleton_model(|_| GPUState::new());
    app.update(crate::editor::init);
    app.update(crate::terminal::input::init);

    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let resources = GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
        FeaturesPageView::new(resources, ctx)
    })
    .1
}

#[test]
fn default_tab_binding_selects_completions() {
    App::test((), |mut app| async move {
        let features = add_features_page(&mut app);

        features.update(&mut app, |view, ctx| {
            assert_eq!(*view.completions_keystroke, "Tab");
            assert_eq!(*view.tab_behavior, TabBehavior::Completions);
            assert_eq!(
                view.tab_behavior_dropdown.as_ref(ctx).selected_action(),
                Some(FeaturesPageAction::SetTabBehavior(TabBehavior::Completions)),
            );
        });
    });
}

#[test]
fn selecting_autosuggestions_keeps_tab_behavior_after_page_refresh() {
    App::test((), |mut app| async move {
        let features = add_features_page(&mut app);

        features.update(&mut app, |view, ctx| {
            view.handle_action(
                &FeaturesPageAction::SetTabBehavior(TabBehavior::Autosuggestions),
                ctx,
            );
            view.on_page_selected(false, ctx);

            assert_eq!(*view.autosuggestions_keystroke, "Tab");
            assert_ne!(*view.completions_keystroke, "Tab");
            assert_eq!(*view.tab_behavior, TabBehavior::Autosuggestions);
            assert_eq!(
                view.tab_behavior_dropdown.as_ref(ctx).selected_action(),
                Some(FeaturesPageAction::SetTabBehavior(
                    TabBehavior::Autosuggestions
                )),
            );
        });
    });
}

#[test]
fn custom_non_tab_binding_refreshes_to_user_defined() {
    App::test((), |mut app| async move {
        let features = add_features_page(&mut app);
        let keybindings = features.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(KeybindingsView::new)
        });

        keybindings.update(&mut app, |view, ctx| {
            view.on_page_selected(false, ctx);
            let index = view
                .rows
                .as_ref()
                .unwrap()
                .iter()
                .position(|row| row.binding.name == OPEN_COMPLETIONS_KEYBINDING_NAME)
                .expect("默认补全快捷键应在快捷键页中显示");
            let custom_key = Keystroke::parse("ctrl-shift-space").unwrap();
            view.handle_action(&KeybindingsViewAction::KeybindingRowClicked(index), ctx);
            view.handle_action(
                &KeybindingsViewAction::KeystrokeDefined(index, custom_key.clone()),
                ctx,
            );
            view.handle_action(&KeybindingsViewAction::ConfirmKeyStroke(index), ctx);
            assert_eq!(
                view.rows.as_ref().unwrap()[index].binding.trigger,
                Some(custom_key),
            );
        });

        features.update(&mut app, |view, ctx| {
            view.on_page_selected(false, ctx);

            assert_ne!(*view.completions_keystroke, "Tab");
            assert_ne!(*view.autosuggestions_keystroke, "Tab");
            assert_eq!(*view.tab_behavior, TabBehavior::UserDefined);
            assert_eq!(
                view.tab_behavior_dropdown.as_ref(ctx).selected_action(),
                Some(FeaturesPageAction::SetTabBehavior(TabBehavior::UserDefined)),
            );
        });
    });
}
