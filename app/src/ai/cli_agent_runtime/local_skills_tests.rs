use std::fs;

use ai::skills::{SkillScope, parse_skill};
use tempfile::TempDir;

use super::*;

fn skill() -> (TempDir, ParsedSkill) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("审查 技能").join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "---\nname: review-local\ndescription: Review local changes\n---\n检查修改及测试。\n",
    )
    .unwrap();
    let parsed = parse_skill(&path).unwrap();
    (directory, parsed)
}

#[test]
fn local_codex_skill_uses_the_real_name_and_absolute_file_path() {
    let (_directory, skill) = skill();
    let inputs = prepare_local_cli_skill_inputs(vec![skill.clone()], Harness::Codex, true).unwrap();
    let LocalOrRemotePath::Local(path) = skill.path else {
        panic!("夹具路径类型错误");
    };
    assert_eq!(
        inputs,
        vec![InputContent::Skill {
            name: "review-local".to_owned(),
            path
        }]
    );
}

#[test]
fn local_skill_missing_after_snapshot_is_rejected_instead_of_silently_dropped() {
    let (_directory, skill) = skill();
    let LocalOrRemotePath::Local(path) = &skill.path else {
        panic!("夹具路径类型错误");
    };
    fs::remove_file(path).unwrap();
    assert!(prepare_local_cli_skill_inputs(vec![skill], Harness::Codex, true).is_err());
}

#[test]
fn local_skill_unverified_harness_and_unmapped_bundled_files_are_rejected() {
    let (_directory, skill) = skill();
    assert!(prepare_local_cli_skill_inputs(vec![skill.clone()], Harness::Grok, true).is_err());
    assert!(prepare_local_cli_skill_inputs(vec![skill.clone()], Harness::Codex, false).is_err());
    let mut bundled = skill;
    bundled.scope = SkillScope::Bundled;
    assert!(prepare_local_cli_skill_inputs(vec![bundled], Harness::Codex, true).is_err());
    assert!(
        prepare_local_cli_skill_inputs(Vec::new(), Harness::Claude, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn claude_skill_plugin_copies_relative_resources_and_cleans_up_after_each_connection() {
    let (_directory, skill) = skill();
    let LocalOrRemotePath::Local(path) = &skill.path else {
        panic!("夹具路径类型错误");
    };
    let source = path.parent().unwrap();
    fs::create_dir(source.join("references")).unwrap();
    fs::write(source.join("references/context.md"), "完整中文资源\n").unwrap();
    let selected = vec![SelectedLocalSkill {
        name: skill.name.clone(),
        path: path.clone(),
    }];
    let first = prepare_claude_skill_plugin(&selected).unwrap().unwrap();
    let first_directory = first.plugin_directory().to_path_buf();
    assert_eq!(
        fs::read_to_string(first_directory.join("skills/review-local/references/context.md"))
            .unwrap(),
        "完整中文资源\n"
    );
    assert_eq!(
        fs::read(first_directory.join("skills/review-local/SKILL.md")).unwrap(),
        fs::read(path).unwrap()
    );
    assert_eq!(
        first.command_names(),
        vec!["infinishell-local-skills:review-local"]
    );
    assert_eq!(
        first.command_for("review-local", path),
        Some("infinishell-local-skills:review-local".to_owned())
    );
    assert!(
        first
            .command_for("review-local", Path::new("/unrelated/SKILL.md"))
            .is_none()
    );
    assert!(!first_directory.join("settings.json").exists());
    drop(first);
    assert!(!first_directory.exists());
    fs::write(source.join("references/context.md"), "恢复时的新资源").unwrap();
    let resumed = prepare_claude_skill_plugin(&selected).unwrap().unwrap();
    assert_ne!(resumed.plugin_directory(), first_directory);
    assert_eq!(
        fs::read_to_string(
            resumed
                .plugin_directory()
                .join("skills/review-local/references/context.md")
        )
        .unwrap(),
        "恢复时的新资源"
    );
}

#[test]
fn claude_skill_plugin_rejects_duplicate_names_and_multiple_native_commands_in_one_turn() {
    let (_directory, skill) = skill();
    let LocalOrRemotePath::Local(path) = &skill.path else {
        panic!("夹具路径类型错误");
    };
    let selected = SelectedLocalSkill {
        name: skill.name.clone(),
        path: path.clone(),
    };
    assert!(prepare_claude_skill_plugin(&[selected.clone(), selected]).is_err());
    assert!(
        prepare_local_cli_skill_inputs(vec![skill.clone(), skill], Harness::Claude, true).is_err()
    );
}

#[cfg(unix)]
#[test]
fn claude_skill_plugin_rejects_symlink_resources_without_changing_the_source() {
    let (_directory, skill) = skill();
    let LocalOrRemotePath::Local(path) = skill.path else {
        panic!("夹具路径类型错误");
    };
    let before = fs::read(&path).unwrap();
    std::os::unix::fs::symlink(&path, path.parent().unwrap().join("linked.md")).unwrap();
    assert!(
        prepare_claude_skill_plugin(&[SelectedLocalSkill {
            name: skill.name,
            path: path.clone()
        }])
        .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn local_skill_read_is_bounded_and_rejects_invalid_utf8() {
    let (_directory, skill) = skill();
    let LocalOrRemotePath::Local(path) = &skill.path else {
        panic!("夹具路径类型错误");
    };
    fs::write(path, vec![b'a'; 1024 * 1024 + 1]).unwrap();
    assert!(prepare_local_cli_skill_inputs(vec![skill.clone()], Harness::Codex, true).is_err());
    fs::write(path, [0xff, 0xfe]).unwrap();
    assert!(prepare_local_cli_skill_inputs(vec![skill], Harness::Codex, true).is_err());
}
