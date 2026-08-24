use warpui::App;
use warpui::keymap::Trigger;

use super::add_overflow_menu_items_as_editable_binding;
use crate::util::bindings::CustomAction;

#[test]
fn cleanup_storage_binding_uses_menu_custom_action() {
    App::test((), |mut app| async move {
        app.update(add_overflow_menu_items_as_editable_binding);

        app.update(|ctx| {
            let binding = ctx
                .editable_bindings()
                .find(|binding| binding.name == "workspace:cleanup_storage")
                .expect("cleanup storage binding should be registered");

            assert_eq!(
                binding.trigger,
                &Trigger::Custom(CustomAction::CleanupStorage.into())
            );
        });
    });
}
