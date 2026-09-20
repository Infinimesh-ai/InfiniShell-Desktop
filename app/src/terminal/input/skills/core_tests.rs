use std::path::PathBuf;

use ai::skills::parse_skill_content_at_location;
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(feature = "local_fs")]
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
#[cfg(feature = "local_fs")]
use warpui::App;
#[cfg(feature = "local_fs")]
use watcher::HomeDirectoryWatcher;

#[cfg(feature = "local_fs")]
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

use super::*;

fn descriptor(metadata: &str, provider: SkillProvider) -> SkillDescriptor {
    parse_skill_content_at_location(
        LocalOrRemotePath::Local(PathBuf::from("skills/example/SKILL.md")),
        &format!("---\nname: example\n{metadata}\n---\n技能正文\n"),
        provider,
        SkillScope::Project,
    )
    .unwrap()
    .into()
}

#[test]
fn user_invocable_menu_policy_preserves_cli_differences() {
    for (metadata, claude, grok) in [
        ("description: default", true, true),
        ("user-invocable: true", true, true),
        ("user-invocable: false", false, false),
        ("user-invocable: \"true\"", true, true),
        ("user-invocable: \"false\"", true, false),
        ("user-invocable: yes", true, false),
        ("user-invocable: \"TRUE\"", true, false),
        ("user-invocable: \" true \"", true, false),
        ("user-invocable: 1", true, false),
        ("user-invocable: null", true, false),
        ("disable-model-invocation: true", true, true),
    ] {
        let skill = descriptor(metadata, SkillProvider::Claude);
        assert_eq!(
            is_user_invocable(&skill.user_invocable, CLIAgent::Claude),
            claude,
            "{metadata}"
        );
        assert_eq!(
            is_user_invocable(&skill.user_invocable, CLIAgent::Grok),
            grok,
            "{metadata}"
        );
        assert!(
            is_user_invocable(&skill.user_invocable, CLIAgent::Codex),
            "{metadata}"
        );
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn shared_cli_skill_filter_hides_native_false_and_preserves_other_consumers() {
    App::test((), |app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let manager = app.add_singleton_model(SkillManager::new);
        manager.read(&app, |manager, _| {
            let hidden = descriptor("user-invocable: false", SkillProvider::Claude);
            assert!(
                selectable_cli_skill(hidden.clone(), Some(CLIAgent::Claude), manager).is_none()
            );
            assert!(selectable_cli_skill(hidden.clone(), Some(CLIAgent::Grok), manager).is_none());
            assert!(selectable_cli_skill(hidden.clone(), Some(CLIAgent::Codex), manager).is_some());
            assert!(selectable_cli_skill(hidden, None, manager).is_some());
            let visible = descriptor("description: default", SkillProvider::Claude);
            assert!(
                selectable_cli_skill(visible.clone(), Some(CLIAgent::Claude), manager).is_some()
            );
            assert!(selectable_cli_skill(visible, Some(CLIAgent::Grok), manager).is_some());
            let unsupported = descriptor("user-invocable: true", SkillProvider::Grok);
            assert!(selectable_cli_skill(unsupported, Some(CLIAgent::Claude), manager).is_none());
        });
    });
}
