use std::collections::HashMap;
#[cfg(unix)]
use std::path::PathBuf;

use chrono::Local;
use pathfinder_color::ColorU;
use smol_str::SmolStr;
use warp_editor::render::model::LineCount;
use warp_util::path::EscapeChar;
use warpui::App;

#[cfg(unix)]
use super::cli_agent_search_dirs;
use super::{
    CLIAgent, UBER_TEAM_UID, build_diff_hunk_prompt, build_review_prompt,
    build_selection_line_range_prompt, build_selection_substring_prompt,
};
use crate::ai::agent::{AgentReviewCommentBatch, DiffSetHunk};
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentOrigin, LineDiffContent,
};
use crate::server::ids::ServerId;
use crate::ui_components::icons::Icon;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

#[cfg(unix)]
mod launch_fallback {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::Arc;

    use smol_str::SmolStr;

    use super::super::{CLIAgent, CLIAgentLaunchSnapshot, cli_agent_launch_fallback};
    use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
    use crate::terminal::model::session::{Session, SessionInfo};
    use crate::terminal::shell::ShellType;

    fn snapshot() -> CLIAgentLaunchSnapshot<'static> {
        CLIAgentLaunchSnapshot {
            host_namespace_verified: true,
            command_snapshot_complete: true,
            shell_type: ShellType::Bash,
            path: Some("/shell/bin"),
            cwd: Some(Path::new("/project")),
            command_known: false,
        }
    }

    #[test]
    fn current_path_keeps_bare_command_before_discovered_installation() {
        let discovered = Path::new("/cache/bin/grok");
        assert_eq!(
            cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot(), |path| path
                == discovered
                || path == Path::new("/shell/bin/grok")),
            None
        );
    }

    #[test]
    fn alias_or_function_snapshot_keeps_bare_command() {
        let discovered = Path::new("/cache/bin/grok");
        let mut snapshot = snapshot();
        snapshot.command_known = true;
        assert_eq!(
            cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                == discovered),
            None
        );
    }

    #[test]
    fn session_alias_and_function_names_retain_their_resolution_priority() {
        let alias = SessionInfo::new_for_test().with_aliases(HashMap::from([(
            SmolStr::new("grok"),
            "custom-grok --profile user".into(),
        )]));
        let function =
            SessionInfo::new_for_test().with_function_names(HashSet::from([SmolStr::new("grok")]));
        let discovered = Path::new("/cache/bin/grok");
        for info in [alias, function] {
            let session = Session::new(info, Arc::new(TestCommandExecutor::default()));
            let mut snapshot = snapshot();
            snapshot.command_known = session.top_level_commands().any(|name| name == "grok");
            assert!(snapshot.command_known);
            assert_eq!(
                cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                    == discovered),
                None
            );
        }
    }

    #[test]
    fn complete_local_snapshot_uses_same_cli_and_quotes_unicode_spaces_and_apostrophe() {
        for agent in [CLIAgent::Claude, CLIAgent::Codex, CLIAgent::Grok] {
            let path = format!("/用户/CLI 工具'目录/{}", agent.command_prefix());
            let discovered = Path::new(&path);
            let expected = format!("'/用户/CLI 工具'\"'\"'目录/{}'", agent.command_prefix());
            for shell_type in [ShellType::Bash, ShellType::Zsh] {
                let mut snapshot = snapshot();
                snapshot.shell_type = shell_type;
                assert_eq!(
                    cli_agent_launch_fallback(agent, Some(discovered), snapshot, |path| path
                        == discovered),
                    Some(expected.clone())
                );
            }
        }
    }

    #[test]
    fn relative_path_resolves_against_session_cwd() {
        let mut snapshot = snapshot();
        snapshot.path = Some("bin:/other");
        let discovered = Path::new("/cache/bin/grok");
        assert_eq!(
            cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                == discovered
                || path == Path::new("/project/bin/grok")),
            None
        );
    }

    #[test]
    fn relative_path_without_reliable_cwd_keeps_bare_command() {
        for cwd in [None, Some(Path::new("relative-project"))] {
            let mut snapshot = snapshot();
            snapshot.path = Some("bin:/other");
            snapshot.cwd = cwd;
            assert_eq!(
                cli_agent_launch_fallback(
                    CLIAgent::Grok,
                    Some(Path::new("/cache/bin/grok")),
                    snapshot,
                    |_| true
                ),
                None
            );
        }
    }

    #[test]
    fn empty_path_segment_uses_session_cwd() {
        let mut snapshot = snapshot();
        snapshot.path = Some(":/other");
        let discovered = Path::new("/cache/bin/grok");
        assert_eq!(
            cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                == discovered
                || path == Path::new("/project/grok")),
            None
        );
    }

    #[test]
    fn unknown_path_or_incomplete_commands_do_not_use_cached_installation() {
        let discovered = Path::new("/cache/bin/grok");
        let mut no_path = snapshot();
        no_path.path = None;
        let mut incomplete = snapshot();
        incomplete.command_snapshot_complete = false;
        for snapshot in [no_path, incomplete] {
            assert_eq!(
                cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                    == discovered),
                None
            );
        }
    }

    #[test]
    fn remote_or_unknown_namespace_never_uses_host_installation() {
        let mut snapshot = snapshot();
        snapshot.host_namespace_verified = false;
        let discovered = Path::new("/cache/bin/grok");
        assert_eq!(
            cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                == discovered),
            None
        );
    }

    #[test]
    fn powershell_and_fish_keep_bare_command_with_unknown_resolution() {
        let discovered = Path::new("/cache/bin/grok");
        for shell_type in [ShellType::PowerShell, ShellType::Fish] {
            let mut snapshot = snapshot();
            snapshot.shell_type = shell_type;
            assert_eq!(
                cli_agent_launch_fallback(CLIAgent::Grok, Some(discovered), snapshot, |path| path
                    == discovered),
                None
            );
        }
    }

    #[test]
    fn removed_installation_or_mismatched_cli_cannot_be_used() {
        for discovered in [
            None,
            Some(Path::new("/cache/bin/grok")),
            Some(Path::new("/cache/bin/claude")),
            Some(Path::new("relative/grok")),
        ] {
            assert_eq!(
                cli_agent_launch_fallback(CLIAgent::Grok, discovered, snapshot(), |_| false),
                None
            );
        }
        assert_eq!(
            cli_agent_launch_fallback(
                CLIAgent::Grok,
                Some(Path::new("/cache/bin/claude")),
                snapshot(),
                |path| path == Path::new("/cache/bin/claude")
            ),
            None
        );
    }
}

/// Helper to build an alias map from pairs.
fn aliases(pairs: &[(&str, &str)]) -> HashMap<SmolStr, String> {
    pairs
        .iter()
        .map(|(k, v)| (SmolStr::new(k), v.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers for prompt-building tests
// ---------------------------------------------------------------------------

fn make_comment(
    content: &str,
    target: AttachedReviewCommentTarget,
    outdated: bool,
) -> AttachedReviewComment {
    AttachedReviewComment {
        id: Default::default(),
        content: content.to_string(),
        target,
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated,
        origin: CommentOrigin::Native,
    }
}

fn batch(comments: Vec<AttachedReviewComment>) -> AgentReviewCommentBatch {
    AgentReviewCommentBatch {
        comments,
        diff_set: HashMap::new(),
    }
}

fn local_path(path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Local(path.into())
}

// ---------------------------------------------------------------------------
// build_review_prompt tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_review_prompt_current_line_is_1_indexed() {
    // LineCount 0 (0-indexed) should appear as L1 in the prompt.
    let comment = make_comment(
        "fix this",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: local_path("/repo/src/main.rs"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(0),
                line_range: LineCount::from(0)..LineCount::from(1),
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(
        prompt.contains("/repo/src/main.rs L1"),
        "expected 1-indexed L1, got: {prompt}",
    );
    assert!(prompt.contains("fix this"));
}

#[test]
fn test_build_review_prompt_removed_line_is_1_indexed() {
    let comment = make_comment(
        "why was this deleted?",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: local_path("/repo/old.rs"),
            line: EditorLineLocation::Removed {
                line_number: LineCount::from(9),
                line_range: LineCount::from(9)..LineCount::from(10),
                index: 0,
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(
        prompt.contains("(deleted, was L10"),
        "expected 1-indexed L10, got: {prompt}",
    );
}

#[test]
fn test_build_review_prompt_collapsed_range_is_1_indexed_start() {
    let comment = make_comment(
        "check this hunk",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: local_path("/repo/lib.rs"),
            line: EditorLineLocation::Collapsed {
                line_range: LineCount::from(4)..LineCount::from(10),
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    // line_range is [4, 10) 0-indexed -> L5-L10 (1-indexed, both ends inclusive)
    assert!(prompt.contains("L5-L10"), "expected L5-L10, got: {prompt}",);
}

#[test]
fn test_build_review_prompt_file_level_comment() {
    let comment = make_comment(
        "needs refactoring",
        AttachedReviewCommentTarget::File {
            absolute_file_path: local_path("/repo/src/utils.rs"),
        },
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(prompt.contains("/repo/src/utils.rs: needs refactoring"));
    // Not a deleted file (empty diff_set), so no "deleted file" text.
    assert!(!prompt.contains("deleted file"));
}

#[test]
fn test_build_review_prompt_deleted_file_comment() {
    let comment = make_comment(
        "why remove this?",
        AttachedReviewCommentTarget::File {
            absolute_file_path: local_path("/repo/src/old.rs"),
        },
        false,
    );
    let mut review = batch(vec![comment]);
    review.diff_set.insert(
        "src/old.rs".to_string(),
        vec![DiffSetHunk {
            line_range: LineCount::from(0)..LineCount::from(5),
            diff_content: String::new(),
            lines_added: 0,
            lines_removed: 5,
        }],
    );
    let prompt = build_review_prompt(&review);
    assert!(
        prompt.contains("(deleted file"),
        "expected deleted file annotation, got: {prompt}",
    );
}

#[test]
fn test_build_review_prompt_general_comment() {
    let comment = make_comment(
        "overall looks good",
        AttachedReviewCommentTarget::General,
        false,
    );
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(prompt.contains("General: overall looks good"));
}

#[test]
fn test_build_review_prompt_skips_outdated_comments() {
    let active = make_comment("keep me", AttachedReviewCommentTarget::General, false);
    let outdated = make_comment("skip me", AttachedReviewCommentTarget::General, true);
    let prompt = build_review_prompt(&batch(vec![active, outdated]));
    assert!(prompt.contains("keep me"));
    assert!(!prompt.contains("skip me"));
}

#[test]
fn test_build_review_prompt_multiple_comments() {
    let c1 = make_comment(
        "first",
        AttachedReviewCommentTarget::Line {
            absolute_file_path: local_path("/repo/a.rs"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(4),
                line_range: LineCount::from(4)..LineCount::from(5),
            },
            content: LineDiffContent::default(),
        },
        false,
    );
    let c2 = make_comment("second", AttachedReviewCommentTarget::General, false);
    let prompt = build_review_prompt(&batch(vec![c1, c2]));
    assert!(prompt.contains("/repo/a.rs L5: first"));
    assert!(prompt.contains("General: second"));
}

#[test]
fn test_build_review_prompt_exports_internal_markdown_without_punctuation_escapes() {
    let comment = make_comment("Fix this\\.", AttachedReviewCommentTarget::General, false);
    let prompt = build_review_prompt(&batch(vec![comment]));
    assert!(prompt.contains("General: Fix this."));
    assert!(!prompt.contains("Fix this\\."));
}

// ---------------------------------------------------------------------------
// build_diff_hunk_prompt tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_diff_hunk_prompt_format() {
    let prompt = build_diff_hunk_prompt("/repo/src/main.rs", 10, 20, 3, 2);
    assert_eq!(
        prompt,
        "/repo/src/main.rs L10-L20 (+3 -2) -- run `git diff` to see the full context.",
    );
}

// ---------------------------------------------------------------------------
// build_selection_line_range_prompt tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_selection_line_range_prompt_format() {
    let result = build_selection_line_range_prompt("src/foo.rs", 5, 10);
    assert_eq!(result, "src/foo.rs L5-L10");
}

#[test]
fn test_build_selection_substring_prompt_format() {
    let result = build_selection_substring_prompt("src/foo.rs", 5, "let x = 42;");
    assert_eq!(result, "src/foo.rs L5: let x = 42;");
}

#[test]
fn test_detect_known_agents() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            for (command, expected) in [
                ("claude", CLIAgent::Claude),
                ("gemini", CLIAgent::Gemini),
                ("codex", CLIAgent::Codex),
                ("grok", CLIAgent::Grok),
                ("deepseek", CLIAgent::DeepSeek),
                ("deepseek-tui", CLIAgent::DeepSeek),
                ("agy", CLIAgent::Antigravity),
                ("amp", CLIAgent::Amp),
                ("droid", CLIAgent::Droid),
                ("opencode", CLIAgent::OpenCode),
                ("copilot", CLIAgent::Copilot),
                ("agent", CLIAgent::CursorCli),
                ("goose", CLIAgent::Goose),
                ("vibe", CLIAgent::Vibe),
                ("omp", CLIAgent::OhMyPi),
                ("infinishell-tui", CLIAgent::WarpTui),
                ("infinishell-tui-dev", CLIAgent::WarpTui),
                ("warp", CLIAgent::WarpTui),
                ("warp-dev", CLIAgent::WarpTui),
                ("./script/run-tui", CLIAgent::WarpTui),
            ] {
                assert_eq!(
                    CLIAgent::detect(command, None, None, ctx),
                    Some(expected),
                    "failed to detect {command}",
                );
            }
        });
    });
}

#[test]
fn test_detect_with_arguments() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("claude --model opus", None, None, ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(
                CLIAgent::detect("gemini chat", None, None, ctx),
                Some(CLIAgent::Gemini),
            );
        });
    });
}

#[test]
fn test_detect_vibe_acp_binary() {
    // The mistral-vibe package ships a `vibe-acp` ACP-mode binary alongside
    // the user-facing `vibe` TUI. Both must be detected as the same agent.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("vibe-acp", None, None, ctx),
                Some(CLIAgent::Vibe),
            );
            assert_eq!(
                CLIAgent::detect("vibe-acp --some-flag", None, None, ctx),
                Some(CLIAgent::Vibe),
            );
            // Distinct binary names should not bleed into Vibe.
            assert_eq!(CLIAgent::detect("vibe-other", None, None, ctx), None);
        });
    });
}

#[test]
fn test_detect_with_leading_whitespace() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("  claude", None, None, ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(CLIAgent::detect("\tclaude --help", None, None, ctx), None,);
        });
    });
}

#[test]
fn test_detect_no_match() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(CLIAgent::detect("ls -la", None, None, ctx), None);
            assert_eq!(CLIAgent::detect("vim", None, None, ctx), None);
            assert_eq!(CLIAgent::detect("claude_wrapper", None, None, ctx), None);
        });
    });
}

#[test]
fn test_detect_with_alias() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("c", "claude")]);
            assert_eq!(
                CLIAgent::detect("c", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(CLIAgent::detect("c --help", None, Some(&map), ctx), None,);

            let map = aliases(&[("o", "omp")]);
            assert_eq!(
                CLIAgent::detect("o", None, Some(&map), ctx),
                Some(CLIAgent::OhMyPi),
            );
        });
    });
}

#[test]
fn test_detect_alias_not_matching() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("c", "cat")]);
            assert_eq!(CLIAgent::detect("c", None, Some(&map), ctx), None);
        });
    });
}

#[test]
fn test_detect_alias_multi_word_value() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // Alias whose value starts with "gemini" but has extra words
            let map = aliases(&[("g", "gemini chat --verbose")]);
            assert_eq!(
                CLIAgent::detect("g", None, Some(&map), ctx),
                Some(CLIAgent::Gemini),
            );
        });
    });
}

#[test]
fn test_detect_with_env_var_prefix() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect(
                    "EXAMPLE=true opencode",
                    Some(EscapeChar::Backslash),
                    None,
                    ctx,
                ),
                Some(CLIAgent::OpenCode),
            );
            assert_eq!(
                CLIAgent::detect("FOO=1 omp", Some(EscapeChar::Backslash), None, ctx,),
                Some(CLIAgent::OhMyPi),
            );
        });
    });
}

#[test]
fn test_detect_with_multiple_env_vars() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect(
                    "FOO=1 BAR=2 opencode --flag",
                    Some(EscapeChar::Backslash),
                    None,
                    ctx,
                ),
                Some(CLIAgent::OpenCode),
            );
        });
    });
}

#[test]
fn test_detect_with_alias_and_env_var() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("oc", "EXAMPLE=1 opencode")]);
            assert_eq!(
                CLIAgent::detect("oc --flag", Some(EscapeChar::Backslash), Some(&map), ctx,),
                Some(CLIAgent::OpenCode),
            );
        });
    });
}

/// Creates a workspace containing a team with the given UID.
fn workspace_with_team_uid(uid: &str) -> Workspace {
    Workspace::from_local_cache(
        ServerId::from_string_lossy("test-workspace-uid-001").into(),
        "Test Workspace".to_string(),
        Some(vec![Team::from_local_cache(
            ServerId::from_string_lossy(uid),
            "Test Team".to_string(),
            None,
            None,
            None,
        )]),
    )
}

#[test]
fn test_detect_aifx_agent_run_claude_on_uber_team() {
    App::test((), |mut app| async move {
        let uber_workspace = workspace_with_team_uid(UBER_TEAM_UID);
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![uber_workspace], ctx));

        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("aifx agent run claude", None, None, ctx),
                Some(CLIAgent::Claude),
            );
            // With extra args
            assert_eq!(
                CLIAgent::detect("aifx agent run claude --verbose", None, None, ctx),
                Some(CLIAgent::Claude),
            );
        });
    });
}

#[test]
fn test_detect_aifx_agent_run_claude_via_alias_on_uber_team() {
    App::test((), |mut app| async move {
        let uber_workspace = workspace_with_team_uid(UBER_TEAM_UID);
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![uber_workspace], ctx));

        app.update(|ctx| {
            let map = aliases(&[("ai", "aifx agent run claude")]);
            assert_eq!(
                CLIAgent::detect("ai", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
            assert_eq!(
                CLIAgent::detect("ai --flag", None, Some(&map), ctx),
                Some(CLIAgent::Claude),
            );
        });
    });
}

#[test]
fn test_detect_aifx_agent_run_claude_not_on_uber_team() {
    App::test((), |mut app| async move {
        // Register UserWorkspaces with no Uber team membership
        app.add_singleton_model(UserWorkspaces::default_mock);

        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("aifx agent run claude", None, None, ctx),
                None,
            );
        });
    });
}

#[test]
fn test_serialized_name_round_trips_known_agents() {
    for agent in enum_iterator::all::<CLIAgent>() {
        let name = agent.to_serialized_name();
        if agent == CLIAgent::Unknown {
            assert_eq!(name, "Unknown");
        } else {
            assert!(!name.is_empty(), "empty serialized name for {agent:?}");
        }
        assert_eq!(
            CLIAgent::from_serialized_name(&name),
            agent,
            "round-trip failed for {agent:?} with serialized name {name:?}",
        );
    }
}

#[test]
fn test_from_serialized_name_falls_back_to_unknown() {
    assert_eq!(CLIAgent::from_serialized_name(""), CLIAgent::Unknown);
    assert_eq!(
        CLIAgent::from_serialized_name("nonexistent"),
        CLIAgent::Unknown
    );
}

#[test]
fn test_detect_aifx_agent_run_claude_wrong_team() {
    App::test((), |mut app| async move {
        let other_workspace = workspace_with_team_uid("some-other-team-uid-01");
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![other_workspace], ctx));

        app.update(|ctx| {
            assert_eq!(
                CLIAgent::detect("aifx agent run claude", None, None, ctx),
                None,
            );
        });
    });
}

#[cfg(unix)]
#[test]
fn test_cli_agent_search_dirs_include_common_gui_app_paths() {
    let dirs: Vec<PathBuf> = cli_agent_search_dirs().collect();

    assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
}

#[cfg(unix)]
#[test]
fn test_cli_agent_search_dirs_include_home_managed_bins() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let dirs: Vec<PathBuf> = cli_agent_search_dirs().collect();

    assert!(dirs.contains(&home.join(".cargo/bin")));
    assert!(dirs.contains(&home.join(".bun/bin")));
    assert!(dirs.contains(&home.join(".local/bin")));
    assert!(dirs.contains(&home.join(".grok/bin")));
}

#[test]
fn test_oh_my_pi_supports_bash_mode() {
    assert!(CLIAgent::OhMyPi.supports_bash_mode());
}

#[test]
fn test_warp_tui_matches_binaries_and_launchers() {
    // Direct binary names.
    assert!(CLIAgent::WarpTui.matches_command("warp", None));
    assert!(CLIAgent::WarpTui.matches_command("warp-preview", None));
    assert!(CLIAgent::WarpTui.matches_command("warp-dev", None));
    assert!(CLIAgent::WarpTui.matches_command("infinishell-tui", None));
    assert!(CLIAgent::WarpTui.matches_command("infinishell-tui-local", None));
    assert!(CLIAgent::WarpTui.matches_command("infinishell-tui-dev", None));
    assert!(CLIAgent::WarpTui.matches_command("infinishell-tui-preview", None));
    assert!(CLIAgent::WarpTui.matches_command("infinishell-tui-stable", None));
    assert!(CLIAgent::WarpTui.matches_command("warp-tui", None));
    assert!(CLIAgent::WarpTui.matches_command("warp-tui-oss", None));
    // The dev launcher script.
    assert!(CLIAgent::WarpTui.matches_command("./script/run-tui", None));
    assert!(CLIAgent::WarpTui.matches_command("script/run-tui", None));
    // Absolute / relative paths to the binary.
    assert!(
        CLIAgent::WarpTui
            .matches_command("/workspace/infinishell/target/debug/infinishell-tui", None,)
    );
    assert!(CLIAgent::WarpTui.matches_command("./target/debug/infinishell-tui", None));
    assert!(CLIAgent::WarpTui.matches_command(
        "/Applications/WarpPreview.app/Contents/MacOS/warp-preview --resume abc",
        None,
    ));
    // With arguments and leading whitespace.
    assert!(CLIAgent::WarpTui.matches_command("  infinishell-tui --resume abc", None));
}

#[test]
fn test_warp_tui_matches_with_env_var_prefix() {
    // Env-var assignments before the command are skipped when an escape char is
    // provided (mirrors `CLIAgent::detect`).
    assert!(CLIAgent::WarpTui.matches_command(
        "WARP_API_KEY=secret infinishell-tui",
        Some(EscapeChar::Backslash),
    ));
}

#[test]
fn test_warp_tui_does_not_match_other_commands() {
    assert!(!CLIAgent::WarpTui.matches_command("vim", None));
    assert!(!CLIAgent::WarpTui.matches_command("htop", None));
    assert!(!CLIAgent::WarpTui.matches_command("claude", None));
    // Lookalikes / substrings should not match.
    assert!(!CLIAgent::WarpTui.matches_command("warp-preview-wrapper", None));
    assert!(!CLIAgent::WarpTui.matches_command("mywarp-dev", None));
    assert!(!CLIAgent::WarpTui.matches_command("infinishell-tui-wrapper", None));
    assert!(!CLIAgent::WarpTui.matches_command("myinfinishell-tui", None));
    assert!(!CLIAgent::WarpTui.matches_command("warp-tui-wrapper", None));
    assert!(!CLIAgent::WarpTui.matches_command("mywarp-tui", None));
    assert!(!CLIAgent::WarpTui.matches_command("", None));
    // `cargo run` is a known non-match (the first token is `cargo`).
    assert!(!CLIAgent::WarpTui.matches_command("cargo run -p warp_tui", None));
}

#[test]
fn test_warp_tui_variant_properties() {
    assert!(CLIAgent::Claude.supports_cli_agent_footer());
    assert_eq!(CLIAgent::WarpTui.command_prefix(), "infinishell-tui");
    assert_eq!(
        CLIAgent::WarpTui.command_prefixes(),
        &[
            "infinishell-tui",
            "infinishell-tui-local",
            "infinishell-tui-dev",
            "infinishell-tui-preview",
            "infinishell-tui-stable",
            "warp",
            "warp-preview",
            "warp-dev",
            "warp-tui",
            "warp-tui-oss",
            "run-tui",
        ]
    );
    assert_eq!(CLIAgent::WarpTui.display_name(), "InfiniShell TUI");
    assert_eq!(CLIAgent::WarpTui.brand_color(), Some(ColorU::black()));
    // InfiniShell TUI 使用 InfiniShell 品牌图标。
    assert_eq!(CLIAgent::WarpTui.icon(), Some(Icon::InfiniShell));
    assert_eq!(CLIAgent::WarpTui.brand_icon_color(), ColorU::white());
    assert!(CLIAgent::WarpTui.supported_skill_providers().is_empty());
    assert!(!CLIAgent::WarpTui.supports_bash_mode());
    assert!(!CLIAgent::WarpTui.supports_cli_agent_footer());
    assert!(matches!(
        crate::server::telemetry::CLIAgentType::from(CLIAgent::WarpTui),
        crate::server::telemetry::CLIAgentType::WarpTui
    ));
    // Serialized name round-trips (also covered by
    // `test_serialized_name_round_trips_known_agents`, asserted explicitly here).
    assert_eq!(
        CLIAgent::from_serialized_name(&CLIAgent::WarpTui.to_serialized_name()),
        CLIAgent::WarpTui
    );
}

#[test]
fn management_and_structured_commands_do_not_enable_rich_input() {
    assert!(!CLIAgent::Grok.matches_command("grok plugin install hooks", None));
    assert!(!CLIAgent::Grok.matches_command("grok --cwd '/tmp/my repo' inspect", None));
    assert!(!CLIAgent::Grok.matches_command("grok agent stdio", None));
    assert!(!CLIAgent::Grok.matches_command("grok -p 'fix the tests'", None));
    assert!(!CLIAgent::Grok.matches_command("grok --version", None));
    assert!(!CLIAgent::Codex.matches_command("codex --profile default plugin list", None));
    assert!(!CLIAgent::Codex.matches_command("codex exec 'fix the tests'", None));
    assert!(!CLIAgent::Codex.matches_command("codex app-server", None));
    assert!(!CLIAgent::Codex.matches_command("codex resume --help", None));
    assert!(!CLIAgent::Claude.matches_command("claude mcp list", None));
    assert!(!CLIAgent::Claude.matches_command("claude --print 'fix the tests'", None));
    assert!(!CLIAgent::Claude.matches_command("claude --version", None));
}

#[test]
fn interactive_resume_and_prompt_values_keep_rich_input() {
    assert!(CLIAgent::Grok.matches_command("grok --resume", None));
    assert!(CLIAgent::Grok.matches_command("grok --worktree", None));
    assert!(CLIAgent::Grok.matches_command("grok --resume plugin", None));
    assert!(CLIAgent::Grok.matches_command("grok --model plugin '修复错误'", None));
    assert!(CLIAgent::Grok.matches_command("grok dashboard", None));
    assert!(CLIAgent::Codex.matches_command("codex resume --last", None));
    assert!(CLIAgent::Codex.matches_command("codex fork --last", None));
    assert!(CLIAgent::Codex.matches_command("codex -- 'plugin'", None));
    assert!(CLIAgent::Claude.matches_command("claude --continue", None));
    assert!(CLIAgent::Claude.matches_command("claude --resume", None));
}

#[test]
fn parity_detection_handles_quoted_paths_and_windows_launchers() {
    assert!(CLIAgent::Grok.matches_command(
        "MODE=test '/opt/my tools/grok' '修复错误'",
        Some(EscapeChar::Backslash)
    ));
    assert!(CLIAgent::Codex.matches_command(
        r#""C:\Program Files\Codex\codex.exe" resume --last"#,
        Some(EscapeChar::Backtick)
    ));
    assert!(!CLIAgent::Claude.matches_command(
        r#""C:\Program Files\Claude\claude.cmd" --version"#,
        Some(EscapeChar::Backtick)
    ));
    assert!(!CLIAgent::Grok.matches_command("grok-wrapper", None));
}

#[test]
fn management_aliases_do_not_enable_rich_input() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let map = aliases(&[("gx", "grok plugin"), ("cx", "codex --profile default")]);
            assert_eq!(CLIAgent::detect("gx list", None, Some(&map), ctx), None);
            assert_eq!(
                CLIAgent::detect("cx --version", None, Some(&map), ctx),
                None
            );
            assert_eq!(
                CLIAgent::detect("cx resume --last", None, Some(&map), ctx),
                Some(CLIAgent::Codex)
            );
        });
    });
}

#[test]
fn version_probe_rejects_unrelated_or_malformed_output() {
    assert_eq!(
        super::parse_cli_agent_version(CLIAgent::Grok, "grok 1.0.30 (04b7ffed98c6)\n"),
        Some("1.0.30".to_string())
    );
    assert_eq!(
        super::parse_cli_agent_version(CLIAgent::Codex, "codex-cli 0.147.0\n"),
        Some("0.147.0".to_string())
    );
    assert_eq!(
        super::parse_cli_agent_version(CLIAgent::Claude, "2.1.0 (Claude Code)\n"),
        Some("2.1.0".to_string())
    );
    assert_eq!(
        super::parse_cli_agent_version(CLIAgent::Grok, "claude 1.0.30"),
        None
    );
    assert_eq!(
        super::parse_cli_agent_version(CLIAgent::Codex, "codex-cli 0.147"),
        None
    );
    assert_eq!(
        super::parse_cli_agent_version(CLIAgent::Claude, "login required"),
        None
    );
}

#[cfg(unix)]
#[test]
fn installation_requires_executable_permission() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("grok");
    std::fs::write(&executable, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        super::find_cli_agent_executable(CLIAgent::Grok, &[directory.path().to_path_buf()]),
        None
    );
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        super::find_cli_agent_executable(CLIAgent::Grok, &[directory.path().to_path_buf()]),
        Some(executable)
    );
}

#[test]
fn grok_agents_skill_compatibility_is_limited_to_home_scope() {
    use ai::skills::{SkillProvider, SkillScope};

    assert!(CLIAgent::Grok.supports_skill(SkillProvider::Grok, SkillScope::Project));
    assert!(CLIAgent::Grok.supports_skill(SkillProvider::Claude, SkillScope::Project));
    assert!(CLIAgent::Grok.supports_skill(SkillProvider::Agents, SkillScope::Home));
    assert!(!CLIAgent::Grok.supports_skill(SkillProvider::Agents, SkillScope::Project));
    assert!(!CLIAgent::Claude.supports_skill(SkillProvider::Grok, SkillScope::Home));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn cached_dispatch_path_is_the_same_binary_as_the_completed_installation_scan() {
    let directory = tempfile::tempdir().unwrap();
    let user_bin = directory.path().join("user installation 中文");
    std::fs::create_dir(&user_bin).unwrap();
    for agent in [CLIAgent::Claude, CLIAgent::Codex, CLIAgent::Grok] {
        let command = agent.command_prefixes()[0];
        let executable = user_bin.join(if cfg!(windows) {
            format!("{command}.EXE")
        } else {
            command.to_owned()
        });
        std::fs::write(&executable, b"test binary fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = super::find_cli_agent_executable(agent, &[user_bin.clone()]).unwrap();
        assert_eq!(found, executable);
        assert!(found.is_absolute());
        let installation = super::CLIAgentInstallation {
            executable: Some(found),
            version: super::CLIAgentVersionStatus::Detected("fixture-version".to_owned()),
        };
        let model = super::CLIAgentInstallModel {
            cache: Some(HashMap::from([(agent, installation.clone())])),
            scan_generation: 7,
        };
        assert_eq!(model.executable(agent), Some(executable.as_path()));
        assert_eq!(model.installation(agent), Some(&installation));
        assert_eq!(model.scan_generation, 7);
    }
}

#[test]
fn parity_cd_launch_recognizes_only_three_interactive_agents() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            for agent in [CLIAgent::Claude, CLIAgent::Codex, CLIAgent::Grok] {
                let executable = agent.command_prefix();
                for directory in [
                    "/tmp/project",
                    "./project",
                    "~/project",
                    "'/tmp/中文 空格'",
                    "\"/tmp/a && b\"",
                ] {
                    let command = format!("cd {directory} && {executable} --resume");
                    assert_eq!(
                        CLIAgent::detect(&command, None, None, ctx),
                        Some(agent),
                        "{command}"
                    );
                }
                let command =
                    format!("  cd '/tmp/project'&&'/opt/CLI 工具/{executable}' \"解释 a && b\"  ");
                assert_eq!(
                    CLIAgent::detect(&command, Some(EscapeChar::Backslash), None, ctx),
                    Some(agent)
                );
                let command =
                    format!(r#"cd "C:\工作区 空格" && "C:\CLI 工具\{executable}.exe" --resume"#);
                assert_eq!(
                    CLIAgent::detect(&command, Some(EscapeChar::Backtick), None, ctx),
                    Some(agent)
                );
            }
            assert_eq!(
                CLIAgent::detect("cd /tmp/project && gemini", None, None, ctx),
                None
            );
            assert_eq!(
                CLIAgent::detect("cd /tmp/project && warp", None, None, ctx),
                None
            );
        });
    });
}

#[test]
fn parity_cd_launch_preserves_exact_tail_and_noninteractive_filters() {
    let source = "cd '/tmp/中文 空格' && grok --model plugin '检查文件'  ";
    assert_eq!(
        CLIAgent::command_after_directory_change(source, EscapeChar::Backslash),
        Some("grok --model plugin '检查文件'  ")
    );
    App::test((), |mut app| async move {
        app.update(|ctx| {
            for command in [
                "grok --version",
                "grok --help",
                "grok plugin install hooks",
                "grok agent stdio",
                "grok --cwd '/tmp/my repo' inspect",
                "grok -p '检查文件'",
                "grok --prompt-file input.txt",
                "codex --version",
                "codex exec '检查文件'",
                "codex app-server",
                "codex resume --help",
                "codex --profile default plugin list",
                "claude --version",
                "claude mcp list",
                "claude --print '检查文件'",
                "claude -p '检查文件'",
            ] {
                let compound = format!("cd /tmp/project && {command}");
                assert_eq!(
                    CLIAgent::detect(&compound, None, None, ctx),
                    None,
                    "{compound}"
                );
            }
            for (command, agent) in [
                ("grok --model plugin '检查文件'", CLIAgent::Grok),
                ("grok --resume plugin", CLIAgent::Grok),
                ("codex --profile plugin resume --last", CLIAgent::Codex),
                ("claude --model plugin --continue", CLIAgent::Claude),
            ] {
                let compound = format!("cd /tmp/project && {command}");
                assert_eq!(
                    CLIAgent::detect(&compound, None, None, ctx),
                    Some(agent),
                    "{compound}"
                );
            }
        });
    });
}

#[test]
fn parity_cd_launch_rejects_other_shell_execution_and_incomplete_syntax() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            for command in [
                "sleep 30 && grok",
                "echo cd /tmp/project && grok",
                "cd && grok",
                "cd '' && grok",
                "cd - && grok",
                "cd -- /tmp/project && grok",
                "cd /tmp/project extra && grok",
                "cd /tmp/project || grok",
                "cd /tmp/project | grok",
                "cd /tmp/project & grok",
                "cd /tmp/project; grok",
                "cd /tmp/project\ngrok",
                "cd /tmp/project &&\ngrok",
                "cd /tmp/project && grok && sleep 30",
                "cd /tmp/project && grok | cat",
                "cd /tmp/project && grok &",
                "cd /tmp/project && grok;",
                "cd /tmp/project && grok &&",
                "cd /tmp/project && grok > output.txt",
                "cd /tmp/project > output.txt && grok",
                "cd /tmp/project && grok < input.txt",
                "cd $(pwd) && grok",
                "cd `pwd` && grok",
                "cd \"$(pwd)\" && grok",
                "cd /tmp/project && grok 'safe'$(sleep 30)",
                "cd /tmp/project && grok $()",
                "cd /tmp/project && grok `sleep 30`",
                "cd /tmp/project && grok <(sleep 30)",
                "cd /tmp/project && (grok)",
                "(cd /tmp/project) && grok",
                "{ cd /tmp/project; } && grok",
                "cd $HOME/project && grok",
                "cd /tmp/project && grok $MODE",
                "cd /tmp/* && grok",
                "cd /tmp/project && grok *.txt",
                "cd /tmp/project && grok --model 'unfinished",
                "cd /tmp/project && grok # comment",
                "cd /tmp/project && grok --model a'b'",
                "MODE=test cd /tmp/project && grok",
                "cd /tmp/project && MODE=test grok",
                "cd \"/tmp && grok\"",
                "cd '/tmp/project && grok",
            ] {
                assert_eq!(
                    CLIAgent::detect(command, None, None, ctx),
                    None,
                    "{command}"
                );
            }
        });
    });
}

#[test]
fn parity_cd_launch_revalidates_aliases_without_changing_direct_alias_detection() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            for (value, expected) in [
                ("grok --model plugin", Some(CLIAgent::Grok)),
                ("codex --profile plugin resume", Some(CLIAgent::Codex)),
                ("claude --continue", Some(CLIAgent::Claude)),
                ("grok plugin", None),
                ("claude --print", None),
                ("codex --version", None),
                ("sleep 30 && grok", None),
                ("grok && sleep 30", None),
                ("grok | cat", None),
                ("grok &", None),
                ("grok $(sleep 30)", None),
                ("grok > output.txt", None),
            ] {
                let map = aliases(&[("a", value)]);
                assert_eq!(
                    CLIAgent::detect("cd /tmp/project && a", None, Some(&map), ctx),
                    expected,
                    "{value}"
                );
            }
            let map = aliases(&[("cd", "grok")]);
            assert_eq!(
                CLIAgent::detect("cd /tmp/project && grok", None, Some(&map), ctx),
                None
            );
            assert_eq!(
                CLIAgent::detect("cd", None, Some(&map), ctx),
                Some(CLIAgent::Grok)
            );
        });
    });
}
