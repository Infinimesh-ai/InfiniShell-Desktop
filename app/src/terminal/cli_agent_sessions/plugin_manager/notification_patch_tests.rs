use super::*;

fn synthetic_bundle() -> (tempfile::TempDir, BTreeMap<String, FileHashes>) {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("scripts")).unwrap();
    let mut hashes = BTreeMap::new();
    for (name, original, replacement) in [
        ("a", "original-a", "replacement-a"),
        ("b", "original-b", "replacement-b"),
    ] {
        let relative = format!("scripts/{name}");
        fs::write(directory.path().join(&relative), original).unwrap();
        hashes.insert(
            relative,
            FileHashes {
                upstream_sha256: sha256(original.as_bytes()),
                replacement_sha256: sha256(replacement.as_bytes()),
            },
        );
    }
    (directory, hashes)
}

const SYNTHETIC_FILES: &[(&str, &str)] = &[
    ("scripts/a", "replacement-a"),
    ("scripts/b", "replacement-b"),
];

#[test]
fn bundle_hashes_and_reviewed_versions_are_consistent() {
    for kind in [PatchKind::Claude, PatchKind::Codex] {
        let metadata = kind.metadata();
        assert!(
            metadata
                .compatible_bases
                .iter()
                .any(|base| base.version == kind.version())
        );
        for (relative, contents) in kind.files() {
            let hashes = &metadata.files[*relative];
            assert_eq!(sha256(contents.as_bytes()), hashes.replacement_sha256);
            // 旧版本仅用于原生升级预检；替换件的原始摘要必须对应最终受测版本。
            let target = metadata
                .compatible_bases
                .iter()
                .find(|base| base.version == kind.version())
                .unwrap();
            assert_eq!(target.tree_sha256[*relative], hashes.upstream_sha256);
            assert!(
                metadata
                    .compatible_bases
                    .iter()
                    .all(|base| base.tree_sha256.contains_key(*relative))
            );
        }
    }
    assert_eq!(PatchKind::Claude.version(), "2.2.0");
}

#[test]
fn runtime_version_requires_exact_probed_identity() {
    assert!(version_matches(PatchKind::Codex, "codex-cli 0.147.0\n"));
    assert!(version_matches(
        PatchKind::Claude,
        "2.1.273 (Claude Code)\n"
    ));
    assert!(!version_matches(PatchKind::Codex, "codex-cli 0.148.0"));
    assert!(!version_matches(PatchKind::Codex, "codex-cli 0.147.0-beta"));
    assert!(!version_matches(PatchKind::Claude, "2.1.273 (Grok Build)"));
}

#[test]
fn applies_exact_files_idempotently_without_changing_permissions() {
    let (directory, hashes) = synthetic_bundle();
    let original_permissions = fs::metadata(directory.path().join("scripts/a"))
        .unwrap()
        .permissions();
    apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |_, _| Ok(())).unwrap();
    apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |_, _| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/a")).unwrap(),
        "replacement-a"
    );
    assert_eq!(
        fs::metadata(directory.path().join("scripts/a"))
            .unwrap()
            .permissions(),
        original_permissions
    );
    assert_eq!(
        fs::read_dir(directory.path().join("scripts"))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn rejects_custom_file_before_any_write() {
    let (directory, hashes) = synthetic_bundle();
    fs::write(directory.path().join("scripts/b"), "user customization").unwrap();
    assert!(apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |_, _| Ok(())).is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/a")).unwrap(),
        "original-a"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/b")).unwrap(),
        "user customization"
    );
}

#[test]
fn second_write_failure_restores_first_file() {
    let (directory, hashes) = synthetic_bundle();
    let result = apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |index, _| {
        if index == 1 {
            Err(io::Error::other("注入第二次替换失败"))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/a")).unwrap(),
        "original-a"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/b")).unwrap(),
        "original-b"
    );
}

#[test]
fn hook_manifest_and_notify_failures_restore_files_in_spaced_chinese_path() {
    for kind in [PatchKind::Claude, PatchKind::Codex] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("应用 数据 ' $() ` 配置");
        let mut hashes = BTreeMap::new();
        let mut originals = BTreeMap::new();
        for (relative, replacement) in kind.files() {
            let original = format!("original {relative}");
            fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
            fs::write(root.join(relative), &original).unwrap();
            hashes.insert(
                (*relative).to_owned(),
                FileHashes {
                    upstream_sha256: sha256(original.as_bytes()),
                    replacement_sha256: sha256(replacement.as_bytes()),
                },
            );
            originals.insert(*relative, original);
        }
        for failed_file in ["hooks/hooks.json", "scripts/warp-notify.sh"] {
            let result = apply_files(&root, kind.files(), &hashes, |_, path| {
                if path == root.join(failed_file) {
                    Err(io::Error::other("注入清单或 tmux 通知替换失败"))
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            for (relative, original) in &originals {
                assert_eq!(fs::read_to_string(root.join(relative)).unwrap(), *original);
            }
            // 清单与通知脚本都是受控文件，自定义内容在任何替换前拒绝。
            fs::write(root.join(failed_file), "custom content").unwrap();
            assert!(apply_files(&root, kind.files(), &hashes, |_, _| Ok(())).is_err());
            assert_eq!(
                fs::read_to_string(root.join("scripts/build-payload.sh")).unwrap(),
                originals["scripts/build-payload.sh"]
            );
            assert_eq!(
                fs::read_to_string(root.join(failed_file)).unwrap(),
                "custom content"
            );
            fs::write(root.join(failed_file), &originals[failed_file]).unwrap();
        }
        apply_files(&root, kind.files(), &hashes, |_, _| Ok(())).unwrap();
        for (relative, replacement) in kind.files() {
            assert_eq!(
                fs::read_to_string(root.join(relative)).unwrap(),
                *replacement
            );
        }
    }
}

#[test]
fn rollback_preserves_a_concurrent_user_edit() {
    let (directory, hashes) = synthetic_bundle();
    let result = apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |index, _| {
        if index == 1 {
            fs::write(directory.path().join("scripts/a"), "concurrent edit")?;
            return Err(io::Error::other("注入并发编辑"));
        }
        Ok(())
    });
    assert!(result.unwrap_err().to_string().contains("恢复失败"));
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/a")).unwrap(),
        "concurrent edit"
    );
}

#[test]
fn interrupted_mixed_patch_can_be_completed() {
    let (directory, hashes) = synthetic_bundle();
    fs::write(directory.path().join("scripts/a"), "replacement-a").unwrap();
    apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |_, _| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/b")).unwrap(),
        "replacement-b"
    );
}

#[test]
fn rejects_corrupted_bundled_replacement() {
    let (directory, hashes) = synthetic_bundle();
    assert!(
        apply_files(
            directory.path(),
            &[("scripts/a", "tampered")],
            &hashes,
            |_, _| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/a")).unwrap(),
        "original-a"
    );
}

#[cfg(unix)]
#[test]
fn rejects_symbolic_and_hard_links() {
    let (directory, hashes) = synthetic_bundle();
    let external = directory.path().join("external");
    fs::write(&external, "original-a").unwrap();
    let target = directory.path().join("scripts/a");
    fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(&external, &target).unwrap();
    assert!(apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |_, _| Ok(())).is_err());
    fs::remove_file(&target).unwrap();
    fs::hard_link(&external, &target).unwrap();
    assert!(apply_files(directory.path(), SYNTHETIC_FILES, &hashes, |_, _| Ok(())).is_err());
    assert_eq!(fs::read_to_string(external).unwrap(), "original-a");
}

fn installed_fixture(home: &Path, kind: PatchKind) -> PathBuf {
    let root = home
        .join("plugins/cache")
        .join(kind.marketplace())
        .join("warp")
        .join(kind.version());
    fs::create_dir_all(root.join(Path::new(kind.manifest()).parent().unwrap())).unwrap();
    fs::write(
        root.join(kind.manifest()),
        serde_json::to_vec(&serde_json::json!({"name":"warp", "version":kind.version()})).unwrap(),
    )
    .unwrap();
    for (relative, contents) in kind.files() {
        fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
        fs::write(root.join(relative), contents).unwrap();
    }
    if kind == PatchKind::Claude {
        fs::write(home.join("plugins/installed_plugins.json"), serde_json::to_vec(&serde_json::json!({"version":2,"plugins":{"warp@claude-code-warp":[{"scope":"user","installPath":root,"version":kind.version()}]}})).unwrap()).unwrap();
    }
    root
}

#[test]
fn duplicate_claude_registry_entries_are_not_guessed() {
    let directory = tempfile::tempdir().unwrap();
    installed_fixture(directory.path(), PatchKind::Claude);
    let path = directory.path().join("plugins/installed_plugins.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let entries = registry["plugins"]["warp@claude-code-warp"]
        .as_array_mut()
        .unwrap();
    entries.push(entries[0].clone());
    fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert!(installed(directory.path(), PatchKind::Claude).is_err());
}

#[test]
fn multiple_codex_cached_versions_are_not_guessed() {
    let directory = tempfile::tempdir().unwrap();
    let root = installed_fixture(directory.path(), PatchKind::Codex);
    fs::create_dir(root.parent().unwrap().join("0.3.0")).unwrap();
    assert!(installed(directory.path(), PatchKind::Codex).is_err());
}

#[test]
fn explicit_invalidation_rechecks_a_changed_file_immediately() {
    let directory = tempfile::tempdir().unwrap();
    let root = installed_fixture(directory.path(), PatchKind::Codex);
    assert!(is_applied(directory.path(), PatchKind::Codex));
    fs::write(root.join("scripts/on-stop.sh"), "custom").unwrap();
    invalidate(directory.path(), PatchKind::Codex);
    assert!(!is_applied(directory.path(), PatchKind::Codex));
}

#[test]
fn minimum_claude_version_does_not_accept_old_stop_failure_gap() {
    let directory = tempfile::tempdir().unwrap();
    let installation = Installation {
        path: directory.path().to_owned(),
        version: "2.1.0".to_owned(),
    };
    assert!(!ready(&installation, PatchKind::Claude).unwrap());
}
