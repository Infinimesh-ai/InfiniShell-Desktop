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
    apply_files(
        directory.path(),
        directory.path().parent().unwrap(),
        SYNTHETIC_FILES,
        &hashes,
        |_, _| Ok(()),
    )
    .unwrap();
    apply_files(
        directory.path(),
        directory.path().parent().unwrap(),
        SYNTHETIC_FILES,
        &hashes,
        |_, _| Ok(()),
    )
    .unwrap();
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
    assert!(
        apply_files(
            directory.path(),
            directory.path().parent().unwrap(),
            SYNTHETIC_FILES,
            &hashes,
            |_, _| Ok(())
        )
        .is_err()
    );
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
    let result = apply_files(
        directory.path(),
        directory.path().parent().unwrap(),
        SYNTHETIC_FILES,
        &hashes,
        |index, _| {
            if index == 1 {
                Err(io::Error::other("注入第二次替换失败"))
            } else {
                Ok(())
            }
        },
    );
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
            let result = apply_files(&root, directory.path(), kind.files(), &hashes, |_, path| {
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
            assert!(
                apply_files(
                    &root,
                    directory.path(),
                    kind.files(),
                    &hashes,
                    |_, _| Ok(())
                )
                .is_err()
            );
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
        apply_files(
            &root,
            directory.path(),
            kind.files(),
            &hashes,
            |_, _| Ok(()),
        )
        .unwrap();
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
    let result = apply_files(
        directory.path(),
        directory.path().parent().unwrap(),
        SYNTHETIC_FILES,
        &hashes,
        |index, _| {
            if index == 1 {
                fs::write(directory.path().join("scripts/a"), "concurrent edit")?;
                return Err(io::Error::other("注入并发编辑"));
            }
            Ok(())
        },
    );
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
    apply_files(
        directory.path(),
        directory.path().parent().unwrap(),
        SYNTHETIC_FILES,
        &hashes,
        |_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/b")).unwrap(),
        "replacement-b"
    );
}

#[test]
fn abandoned_forward_and_rollback_staging_does_not_pollute_active_tree() {
    for interrupted_rollback in [false, true] {
        let (directory, hashes) = synthetic_bundle();
        let staging_parent = tempfile::tempdir().unwrap();
        let staging = patch_staging(directory.path(), staging_parent.path()).unwrap();
        let target = directory.path().join("scripts/a");
        let contents = if interrupted_rollback {
            fs::write(&target, "replacement-a").unwrap();
            b"original-a".as_slice()
        } else {
            b"replacement-a".as_slice()
        };
        let permissions = fs::metadata(&target).unwrap().permissions();
        let staged = stage_replacement(staging.path(), contents, permissions.clone()).unwrap();
        let (handle, orphan) = staged.keep().unwrap();
        drop(handle);
        let abandoned = staging.keep();
        assert!(orphan.starts_with(&abandoned));
        assert!(!abandoned.starts_with(directory.path()));
        assert_eq!(fs::read(&orphan).unwrap(), contents);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(
            fs::read_dir(directory.path().join("scripts"))
                .unwrap()
                .count(),
            2
        );
        apply_files(
            directory.path(),
            staging_parent.path(),
            SYNTHETIC_FILES,
            &hashes,
            |_, _| Ok(()),
        )
        .unwrap();
        for (relative, replacement) in SYNTHETIC_FILES {
            assert_eq!(
                fs::read(directory.path().join(relative)).unwrap(),
                replacement.as_bytes()
            );
        }
        assert_eq!(fs::metadata(&target).unwrap().permissions(), permissions);
        // 重试不接管或清除旧暂存；未知内容不能被猜测成事务文件。
        assert_eq!(fs::read(orphan).unwrap(), contents);
        assert_eq!(fs::read_dir(staging_parent.path()).unwrap().count(), 1);
    }
}

#[test]
fn rejects_staging_inside_active_tree_before_replacing_files() {
    let (directory, hashes) = synthetic_bundle();
    assert!(
        apply_files(
            directory.path(),
            directory.path(),
            SYNTHETIC_FILES,
            &hashes,
            |_, _| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("scripts/a")).unwrap(),
        "original-a"
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn rejects_staging_alias_into_active_tree() {
    let (directory, _) = synthetic_bundle();
    let outside = tempfile::tempdir().unwrap();
    let alias = outside.path().join("alias");
    std::os::unix::fs::symlink(directory.path(), &alias).unwrap();
    assert!(patch_staging(directory.path(), &alias).is_err());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn rejects_corrupted_bundled_replacement() {
    let (directory, hashes) = synthetic_bundle();
    assert!(
        apply_files(
            directory.path(),
            directory.path().parent().unwrap(),
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
    assert!(
        apply_files(
            directory.path(),
            directory.path().parent().unwrap(),
            SYNTHETIC_FILES,
            &hashes,
            |_, _| Ok(())
        )
        .is_err()
    );
    fs::remove_file(&target).unwrap();
    fs::hard_link(&external, &target).unwrap();
    assert!(
        apply_files(
            directory.path(),
            directory.path().parent().unwrap(),
            SYNTHETIC_FILES,
            &hashes,
            |_, _| Ok(())
        )
        .is_err()
    );
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

#[test]
fn codex_windows_install_entry_does_not_enable_claude_or_other_architectures() {
    assert_eq!(auto_install_supported_for(PatchKind::Claude), cfg!(unix));
    assert_eq!(
        auto_install_supported_for(PatchKind::Codex),
        cfg!(unix) || cfg!(all(windows, target_arch = "x86_64"))
    );
}

#[test]
fn windows_native_binary_preflight_rejects_shims_and_unreviewed_executables() {
    let directory = tempfile::tempdir().unwrap();
    let shim = directory.path().join("codex.cmd");
    let executable = directory.path().join("codex.exe");
    let body = b"codex-cli 0.147.0";
    fs::write(&shim, body).unwrap();
    fs::write(&executable, body).unwrap();
    assert!(verify_windows_codex_binary(&shim).is_err());
    assert!(verify_windows_codex_binary(&executable).is_err());
    assert!(verify_windows_codex_binary(directory.path()).is_err());
    assert_eq!(fs::read(&shim).unwrap(), body);
    assert_eq!(fs::read(&executable).unwrap(), body);
}

#[test]
fn windows_dependency_receipt_requires_exact_output_and_empty_error_stream() {
    assert!(windows_dependency_receipt_matches(
        b"infinishell-codex-windows-dependencies-v1\r\n",
        b""
    ));
    assert!(!windows_dependency_receipt_matches(b"", b""));
    assert!(!windows_dependency_receipt_matches(
        b"infinishell-codex-windows-dependencies-v1\n",
        b""
    ));
    assert!(!windows_dependency_receipt_matches(
        b"infinishell-codex-windows-dependencies-v1\r\n",
        b"dependency failed"
    ));
    assert!(!windows_dependency_receipt_matches(
        b"infinishell-codex-windows-dependencies-v1\r\nextra",
        b""
    ));
}

#[test]
fn windows_dependency_preflight_reuses_reviewed_launcher_argument_encoding() {
    let original =
        include_str!("../../../../../script/cli-agent-parity/codex_windows_hook_command.ps1")
            .replace("\r\n", "\n");
    let preamble = original
        .split("# BEGIN_NOTIFICATION_LAUNCH")
        .next()
        .unwrap();
    let dependency = WINDOWS_DEPENDENCY_PROBE.replace("\r\n", "\n").replacen(
        "$ProgressPreference = 'SilentlyContinue'\n",
        "",
        1,
    );
    assert!(
        dependency.starts_with(preamble),
        "依赖预检除进度流抑制外必须复用已审阅的原生 argv 编码"
    );
}

#[test]
fn windows_dependency_preflight_accepts_reviewed_git_bash_runtime_names() {
    assert!(
        WINDOWS_DEPENDENCY_PROBE
            .contains(r#"case "$OSTYPE" in msys*|cygwin) ;; *) exit 1 ;; esac"#)
    );
    assert!(WINDOWS_DEPENDENCY_PROBE.contains("$ProgressPreference = 'SilentlyContinue'"));
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[tokio::test]
async fn windows_runtime_rejects_unreviewed_binary_before_any_cli_command_or_home_write() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex.exe");
    fs::write(&executable, b"unreviewed binary").unwrap();
    let home = directory.path().join("not-created");
    let mut log = String::new();
    let result =
        VerifiedRuntime::probe(PatchKind::Codex, &home, directory.path().to_str(), &mut log).await;
    assert_eq!(result.err().unwrap().message, unsupported().message);
    assert!(!home.exists());
    assert!(log.is_empty());
    assert_eq!(fs::read(executable).unwrap(), b"unreviewed binary");
}

#[test]
fn windows_installer_binary_is_bound_to_the_formal_native_evidence() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-native-hook-trust-rev4-windows.json"
    )).unwrap();
    assert_eq!(evidence["fixed_cli"]["bytes"], WINDOWS_CODEX_BYTES);
    assert_eq!(evidence["fixed_cli"]["sha256"], WINDOWS_CODEX_SHA256);
    assert_eq!(evidence["verified_architecture"], "x86_64");
    assert_eq!(evidence["cli"], "codex-cli 0.147.0");
}
