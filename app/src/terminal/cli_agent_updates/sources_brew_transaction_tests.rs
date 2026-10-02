use std::os::unix::fs::PermissionsExt as _;

use super::*;

struct Fixture {
    _temporary: tempfile::TempDir,
    parent: Directory,
    journal: Journal,
}

fn fixture() -> Fixture {
    publication_fixture(false)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn private_caskroom_fixture() -> Fixture {
    let mut fixture = fixture();
    let prefix = &fixture.journal.prefix;
    fs::set_permissions(prefix, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(prefix.join("Caskroom"), fs::Permissions::from_mode(0o775)).unwrap();
    let directory = Directory::open(prefix).unwrap();
    fixture.journal.prefix_identity = directory.identity().unwrap();
    fixture.journal.parent_identity = directory.homebrew_caskroom().unwrap().1;
    fixture.journal.verify_external().unwrap();
    fixture
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn private_caskroom_recovery_rejects_changed_prefix_permissions() {
    let fixture = private_caskroom_fixture();
    fs::set_permissions(&fixture.journal.prefix, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(fixture.journal.verify_external().is_err());
    assert_eq!(
        fs::read(&fixture.journal.entry).unwrap(),
        b"new public binary"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn private_caskroom_recovery_rejects_replaced_prefix_or_parent() {
    for replace_prefix in [false, true] {
        let fixture = private_caskroom_fixture();
        let path = if replace_prefix {
            fixture.journal.prefix.clone()
        } else {
            fixture.journal.parent()
        };
        let saved = path.with_extension("saved");
        fs::rename(&path, &saved).unwrap();
        fs::create_dir(&path).unwrap();
        fs::set_permissions(
            &path,
            fs::Permissions::from_mode(if replace_prefix { 0o700 } else { 0o775 }),
        )
        .unwrap();
        assert!(fixture.journal.verify_external().is_err());
        assert!(saved.exists());
    }
}

#[test]
fn replaced_public_manager_link_preserves_the_original_manager_and_package() {
    let mut fixture = fixture();
    let prefix = &fixture.journal.prefix;
    let manager = prefix.join("Homebrew/bin/brew");
    fs::create_dir_all(manager.parent().unwrap()).unwrap();
    fs::rename(prefix.join("bin/brew"), &manager).unwrap();
    symlink(&manager, prefix.join("bin/brew")).unwrap();
    fixture.journal.manager = stamp(&prefix.join("bin/brew")).unwrap();
    fixture.journal.verify_external().unwrap();
    let other = prefix.join("Homebrew/bin/other-brew");
    fs::write(&other, b"another manager").unwrap();
    fs::remove_file(prefix.join("bin/brew")).unwrap();
    symlink(other, prefix.join("bin/brew")).unwrap();

    assert!(matches!(
        fixture.journal.verify_external(),
        Err(Error::SourceChanged)
    ));
    assert_eq!(fs::read(manager).unwrap(), b"unchanged manager");
    assert_eq!(
        fs::read(&fixture.journal.entry).unwrap(),
        b"new public binary"
    );
}

fn publication_fixture(with_alias: bool) -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let prefix = temporary.path().canonicalize().unwrap().join("prefix");
    let (agent, token, old_version, target_version) = if with_alias {
        ("grok", "grok-build", "1.0.40", "1.0.41")
    } else {
        ("codex", "codex", "0.155.1", "0.156.1")
    };
    let platform = if cfg!(target_os = "linux") {
        "linux-x86_64"
    } else {
        "macos-aarch64"
    };
    let old_relative = if with_alias {
        PathBuf::from(format!("grok-{old_version}-{platform}"))
    } else {
        PathBuf::from("bin/codex")
    };
    let new_relative = if with_alias {
        PathBuf::from(format!("grok-{target_version}-{platform}"))
    } else {
        PathBuf::from("bin/codex")
    };
    let original_root = prefix.join("Caskroom").join(token);
    let old_program = original_root.join(old_version).join(&old_relative);
    let new_program = original_root.join(target_version).join(&new_relative);
    fs::create_dir_all(old_program.parent().unwrap()).unwrap();
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(&old_program, b"old public binary").unwrap();
    fs::write(prefix.join("bin/brew"), b"unchanged manager").unwrap();
    let entry = prefix.join("bin").join(agent);
    symlink(
        Path::new("../Caskroom")
            .join(token)
            .join(old_version)
            .join(&old_relative),
        &entry,
    )
    .unwrap();
    let parent = Directory::open(&prefix.join("Caskroom")).unwrap();
    let original = parent.child(token.as_ref()).unwrap().snapshot().unwrap();
    let id = Uuid::new_v4();
    let stage_name = OsString::from(format!(".infinishell-brew-{id}"));
    let stage = parent.create(&stage_name).unwrap();
    let stage_root = prefix.join("Caskroom").join(&stage_name);
    let staged_program = stage_root.join(target_version).join(&new_relative);
    fs::create_dir_all(staged_program.parent().unwrap()).unwrap();
    fs::write(staged_program, b"new public binary").unwrap();
    let link_stage = prefix
        .join("bin")
        .join(format!(".infinishell-brew-link-{id}"));
    symlink(&new_program, &link_stage).unwrap();
    let aliases = if with_alias {
        symlink(&old_program, prefix.join("bin/agent")).unwrap();
        let mut alias = Alias::capture(&prefix, id, &old_program).unwrap();
        alias.prepare(&new_program).unwrap();
        vec![alias]
    } else {
        Vec::new()
    };
    let journal = Journal {
        schema: 1,
        id,
        agent: agent.into(),
        prefix_identity: Directory::open(&prefix).unwrap().identity().unwrap(),
        parent_identity: parent.identity().unwrap(),
        bin_identity: Directory::open(&prefix.join("bin"))
            .unwrap()
            .identity()
            .unwrap(),
        token: token.into(),
        entry: entry.clone(),
        manager: stamp(&prefix.join("bin/brew")).unwrap(),
        old_version: old_version.into(),
        target_version: target_version.into(),
        intent: "fixed-test-intent".into(),
        original,
        prepared: Some(stage.snapshot().unwrap()),
        original_link: link(&entry).unwrap(),
        prepared_link: Some(link(&link_stage).unwrap()),
        phase: Phase::ExchangeIntent,
        probe: None,
        extra_probes: Vec::new(),
        completions: Vec::new(),
        aliases,
        entry_probes: Vec::new(),
        prefix,
    };
    parent.exchange(token.as_ref(), &stage_name).unwrap();
    exchange_links(&journal).unwrap();
    for alias in &journal.aliases {
        alias.publish().unwrap();
    }
    Fixture {
        _temporary: temporary,
        parent,
        journal,
    }
}

#[test]
fn changed_published_tree_preserves_public_link_and_backup() {
    let fixture = fixture();
    let journal = &fixture.journal;
    let current_root = journal.parent().join("codex");
    fs::write(current_root.join("external-marker"), b"external change").unwrap();
    let changed = fixture
        .parent
        .child("codex".as_ref())
        .unwrap()
        .snapshot()
        .unwrap();
    let published_link = link(&journal.entry).unwrap();
    let backup_link = link(&journal.link_stage()).unwrap();

    assert_eq!(
        rollback_publication(journal, &fixture.parent),
        Err(Error::RecoveryRequired)
    );

    assert_eq!(link(&journal.entry).unwrap(), published_link);
    assert_eq!(link(&journal.link_stage()).unwrap(), backup_link);
    assert_eq!(fs::read(&journal.entry).unwrap(), b"new public binary");
    assert_eq!(
        fs::read(current_root.join("external-marker")).unwrap(),
        b"external change"
    );
    assert_eq!(
        fixture
            .parent
            .child("codex".as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        changed
    );
    assert_eq!(
        fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        journal.original
    );
}

#[test]
fn changed_backup_tree_preserves_public_link_and_current_tree() {
    let fixture = fixture();
    let journal = &fixture.journal;
    let backup_root = journal.parent().join(journal.stage_name());
    fs::write(
        backup_root.join("external-marker"),
        b"external backup change",
    )
    .unwrap();
    let changed = fixture
        .parent
        .child(&journal.stage_name())
        .unwrap()
        .snapshot()
        .unwrap();
    let published_link = link(&journal.entry).unwrap();
    let backup_link = link(&journal.link_stage()).unwrap();

    assert_eq!(
        rollback_publication(journal, &fixture.parent),
        Err(Error::RecoveryRequired)
    );

    assert_eq!(link(&journal.entry).unwrap(), published_link);
    assert_eq!(link(&journal.link_stage()).unwrap(), backup_link);
    assert_eq!(fs::read(&journal.entry).unwrap(), b"new public binary");
    assert_eq!(
        fs::read(backup_root.join("external-marker")).unwrap(),
        b"external backup change"
    );
    assert_eq!(
        fixture
            .parent
            .child("codex".as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        *journal.prepared.as_ref().unwrap()
    );
    assert_eq!(
        fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        changed
    );
}

#[test]
fn unchanged_published_trees_restore_original_publication() {
    let fixture = fixture();
    let journal = &fixture.journal;

    rollback_publication(journal, &fixture.parent).unwrap();

    assert_eq!(fs::read(&journal.entry).unwrap(), b"old public binary");
    assert_eq!(link(&journal.entry).unwrap(), journal.original_link);
    assert_eq!(
        link(&journal.link_stage()).unwrap(),
        *journal.prepared_link.as_ref().unwrap()
    );
    assert_eq!(
        fixture
            .parent
            .child("codex".as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        journal.original
    );
    assert_eq!(
        fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        *journal.prepared.as_ref().unwrap()
    );
}

fn artifact_fixture() -> Fixture {
    let mut fixture = publication_fixture(true);
    for (shell, path) in completion_paths(CLIAgent::Grok, &fixture.journal.prefix) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // fish 原本不存在，回滚必须保持这个合法状态。
        if shell != "fish" {
            fs::write(&path, b"old completion").unwrap();
        }
        let completion =
            Completion::prepare(path, b"old completion", b"new completion".to_vec()).unwrap();
        completion.publish().unwrap();
        fixture.journal.completions.push(completion);
    }
    fixture
}

#[derive(Debug, Eq, PartialEq)]
struct ArtifactState {
    current: Snapshot,
    backup: Snapshot,
    public_link: Link,
    public_stage: Link,
    alias: Link,
    alias_stage: Option<Link>,
    completion_directories: Vec<Snapshot>,
}

fn artifact_state(fixture: &Fixture) -> ArtifactState {
    let journal = &fixture.journal;
    let alias_stage = journal
        .prefix
        .join("bin")
        .join(format!(".infinishell-brew-agent-{}", journal.id));
    let alias_stage = match fs::symlink_metadata(&alias_stage) {
        Ok(_) => Some(link(&alias_stage).unwrap()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("别名暂存读取失败：{error}"),
    };
    ArtifactState {
        current: fixture
            .parent
            .child(journal.token.as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        backup: fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        public_link: link(&journal.entry).unwrap(),
        public_stage: link(&journal.link_stage()).unwrap(),
        alias: link(&journal.prefix.join("bin/agent")).unwrap(),
        alias_stage,
        completion_directories: completion_paths(CLIAgent::Grok, &journal.prefix)
            .into_iter()
            .map(|(_, path)| {
                Directory::open(path.parent().unwrap())
                    .unwrap()
                    .snapshot()
                    .unwrap()
            })
            .collect(),
    }
}

#[test]
fn rollback_preflight_changed_completion_preserves_all_artifacts() {
    let fixture = artifact_fixture();
    let [_, (_, zsh), _] = completion_paths(CLIAgent::Grok, &fixture.journal.prefix);
    fs::write(zsh, b"external completion").unwrap();
    let before = artifact_state(&fixture);

    assert_eq!(
        rollback_publication(&fixture.journal, &fixture.parent),
        Err(Error::SourceChanged)
    );

    assert_eq!(artifact_state(&fixture), before);
}

#[test]
fn rollback_preflight_changed_alias_preserves_all_artifacts() {
    let fixture = artifact_fixture();
    let alias_path = fixture.journal.prefix.join("bin/agent");
    fs::remove_file(&alias_path).unwrap();
    symlink(fixture.journal.prefix.join("external-agent"), &alias_path).unwrap();
    let before = artifact_state(&fixture);

    assert_eq!(
        rollback_publication(&fixture.journal, &fixture.parent),
        Err(Error::SourceChanged)
    );

    assert_eq!(artifact_state(&fixture), before);
}

#[test]
fn rollback_preflight_changed_restored_alias_stage_preserves_all_artifacts() {
    let fixture = artifact_fixture();
    fixture.journal.aliases[0].rollback().unwrap();
    let alias_stage = fixture
        .journal
        .prefix
        .join("bin")
        .join(format!(".infinishell-brew-agent-{}", fixture.journal.id));
    fs::remove_file(&alias_stage).unwrap();
    symlink(fixture.journal.prefix.join("external-stage"), &alias_stage).unwrap();
    let before = artifact_state(&fixture);

    assert_eq!(
        rollback_publication(&fixture.journal, &fixture.parent),
        Err(Error::SourceChanged)
    );

    assert_eq!(artifact_state(&fixture), before);
}

#[test]
fn rollback_preflight_changed_restored_public_link_stage_preserves_all_artifacts() {
    let fixture = artifact_fixture();
    exchange_links(&fixture.journal).unwrap();
    let link_stage = fixture.journal.link_stage();
    fs::remove_file(&link_stage).unwrap();
    symlink(
        fixture.journal.prefix.join("external-public-stage"),
        &link_stage,
    )
    .unwrap();
    let before = artifact_state(&fixture);

    assert_eq!(
        rollback_publication(&fixture.journal, &fixture.parent),
        Err(Error::RecoveryRequired)
    );

    assert_eq!(artifact_state(&fixture), before);
}

#[test]
fn rollback_preflight_changed_restored_tree_stage_preserves_all_artifacts() {
    let fixture = artifact_fixture();
    fixture
        .parent
        .exchange(
            fixture.journal.token.as_ref(),
            &fixture.journal.stage_name(),
        )
        .unwrap();
    let stage = fixture.journal.parent().join(fixture.journal.stage_name());
    fs::write(
        stage.join("external-marker"),
        b"external prepared tree change",
    )
    .unwrap();
    let before = artifact_state(&fixture);

    assert_eq!(
        rollback_publication(&fixture.journal, &fixture.parent),
        Err(Error::RecoveryRequired)
    );

    assert_eq!(artifact_state(&fixture), before);
}

#[test]
fn rollback_preflight_resumes_claimed_and_missing_artifacts_idempotently() {
    let fixture = artifact_fixture();
    let [(_, bash), (_, zsh), (_, fish)] =
        completion_paths(CLIAgent::Grok, &fixture.journal.prefix);
    // bash 停在正向认领后，zsh 停在逆向认领后；两者原路径均暂时缺失。
    fs::remove_file(&bash).unwrap();
    let record = serde_json::to_value(&fixture.journal.completions[1]).unwrap();
    let reverse_stage = PathBuf::from(record["reverse_stage"].as_str().unwrap());
    fs::create_dir(&reverse_stage).unwrap();
    fs::set_permissions(&reverse_stage, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(&zsh, reverse_stage.join("claimed")).unwrap();
    fixture.journal.completions[2].rollback().unwrap();
    fixture.journal.aliases[0].rollback().unwrap();
    fixture.journal.aliases[0].cleanup(false).unwrap();
    let restored_alias = link(&fixture.journal.prefix.join("bin/agent")).unwrap();

    rollback_publication(&fixture.journal, &fixture.parent).unwrap();

    assert_eq!(fs::read(&bash).unwrap(), b"old completion");
    assert_eq!(fs::read(&zsh).unwrap(), b"old completion");
    assert!(!fish.exists());
    assert_eq!(
        link(&fixture.journal.entry).unwrap(),
        fixture.journal.original_link
    );
    assert_eq!(
        link(&fixture.journal.prefix.join("bin/agent")).unwrap(),
        restored_alias
    );
    assert_eq!(
        fs::read(&fixture.journal.entry).unwrap(),
        b"old public binary"
    );
    assert_eq!(
        fixture
            .parent
            .child(fixture.journal.token.as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        fixture.journal.original
    );
    let restored = artifact_state(&fixture);

    rollback_publication(&fixture.journal, &fixture.parent).unwrap();

    assert_eq!(artifact_state(&fixture), restored);

    // 合法清理已移走主树和公共链接暂存项，重复恢复也不能重新创建它们。
    fixture
        .parent
        .remove_matching(
            &fixture.journal.stage_name(),
            fixture.journal.prepared.as_ref().unwrap(),
        )
        .unwrap();
    fs::remove_file(fixture.journal.link_stage()).unwrap();

    rollback_publication(&fixture.journal, &fixture.parent).unwrap();

    assert!(
        !fixture
            .parent
            .has_child(&fixture.journal.stage_name())
            .unwrap()
    );
    assert_eq!(
        fs::symlink_metadata(fixture.journal.link_stage())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        fs::read(&fixture.journal.entry).unwrap(),
        b"old public binary"
    );
}
