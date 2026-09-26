//! Linux Claude 的旧入口、候选与新入口分别保留监督代；实际执行使用密封 ELF 快照。

use std::io::Write as _;
use std::os::unix::fs::DirBuilderExt as _;

use futures::AsyncReadExt as _;
use sha2::Digest as _;
use warpui::r#async::FutureExt as _;

use super::*;

pub(super) async fn entries(
    root: &Path,
    path: &Path,
    journal: &mut Journal,
    published: bool,
) -> Result<(), Error> {
    let offset = if published { 1 } else { 0 };
    if journal.cli()? != CLIAgent::Claude || journal.entry_probes.len() != offset {
        return Err(Error::RecoveryRequired);
    }
    let version = if published {
        journal.target_version.clone()
    } else {
        journal.old_version.clone()
    };
    let program = journal
        .parent()
        .join(&journal.token)
        .join(&version)
        .join("claude");
    let (length, sha256) = super::super::brew_claude_linux::native(&version)?;
    verify_entries(journal, published)?;
    let public = journal.prefix.join("bin").join("claude");
    let observed = stamp(&public)?;
    if observed.canonical != program || observed.digest != sha256 {
        return Err(Error::SourceChanged);
    }
    let expected =
        managed_process::ExpectedFileIdentity::capture_release_image(&program, length, sha256)
            .map_err(|_| Error::SourceChanged)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                journal.id,
                &public,
                &program,
                sha256,
                published,
                &journal.original_link,
                &journal.prepared_link,
                &journal.aliases,
            ))
            .map_err(|_| Error::PersistenceFailed)?
        )
    );
    let binding = probe_binding(CLIAgent::Claude, digest.clone())?;
    let generation = Uuid::new_v4();
    journal.entry_probes.push(Probe {
        generation,
        program: program.clone(),
        digest,
        version: None,
        arguments: vec!["--version".into()],
        completed: false,
        output_sha256: None,
    });
    save(path, journal)?;
    let mut child =
        managed_process::spawn_bound_version_probe(root, generation, &program, expected, &binding)
            .await
            .map_err(|_| Error::RecoveryRequired)?;
    let Some(mut stdout) = child.stdout.take() else {
        child.finish().await.map_err(|_| Error::RecoveryRequired)?;
        return Err(Error::RecoveryRequired);
    };
    let mut bytes = Vec::new();
    let output = (&mut stdout)
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .with_timeout(UPDATE_TIMEOUT)
        .await;
    drop(stdout);
    let receipt = if output.is_ok() {
        child.finish_after_stdin_close().await
    } else {
        child.finish().await
    }
    .map_err(|_| Error::RecoveryRequired)?;
    if !receipt.cleanup_confirmed {
        return Err(Error::RecoveryRequired);
    }
    match output {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return Err(Error::ProbeFailed),
        Err(_) => return Err(Error::TimedOut),
    }
    if receipt.exit_code != Some(0) || bytes.len() as u64 > MAX_OUTPUT {
        return Err(Error::ProbeFailed);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?;
    if super::super::parse_cli_agent_version(CLIAgent::Claude, text).as_deref()
        != Some(version.as_str())
    {
        return Err(Error::VersionMismatch);
    }
    verify_entries(journal, published)?;
    if stamp(&public)? != observed {
        return Err(Error::SourceChanged);
    }
    let saved = journal
        .entry_probes
        .last_mut()
        .ok_or(Error::RecoveryRequired)?;
    saved.version = Some(version.clone());
    saved.completed = true;
    saved.output_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
    save(path, journal)?;
    Ok(())
}

fn verify_entries(journal: &Journal, published: bool) -> Result<(), Error> {
    journal.verify_external()?;
    let expected = if published {
        journal
            .prepared_link
            .as_ref()
            .ok_or(Error::RecoveryRequired)?
    } else {
        &journal.original_link
    };
    if link(&journal.entry)? != *expected || !journal.aliases.is_empty() {
        return Err(Error::SourceChanged);
    }
    let version = if published {
        &journal.target_version
    } else {
        &journal.old_version
    };
    super::super::brew_claude_linux::verify_tree(&journal.parent().join(&journal.token), version)
}

/// 在删除可恢复账本前留下不可覆盖收据，保留入口归属和每个真实监督代的清理结果。
pub(super) fn receipt(root: &Path, journal: &Journal, committed: bool) -> Result<(), Error> {
    let mut processes = Vec::new();
    for (kind, records) in [
        ("candidate", journal.probe.iter().collect::<Vec<_>>()),
        ("entry", journal.entry_probes.iter().collect()),
    ] {
        for (index, probe) in records.into_iter().enumerate() {
            let binding = probe_binding(CLIAgent::Claude, probe.digest.clone())?;
            let exit =
                managed_process::confirmed_exit_with_binding(root, probe.generation, &binding)
                    .map_err(|_| Error::RecoveryRequired)?
                    .ok_or(Error::RecoveryRequired)?;
            if !exit.cleanup_confirmed
                || committed
                    && (!probe.completed
                        || exit.exit_code != Some(0)
                        || probe.output_sha256.is_none())
            {
                return Err(Error::RecoveryRequired);
            }
            let entry = (kind == "entry").then(|| journal.prefix.join("bin").join("claude"));
            processes.push(
                json!({"kind":kind,"index":index,"public_entry":entry,"probe":probe,"exit":exit}),
            );
        }
    }
    let bytes = serde_json::to_vec_pretty(&json!({"schema":1,"state":if committed {"committed"} else {"rolled_back"},
        "execution":"resolved-native-sealed-elf-snapshot", "journal":journal, "processes":processes})).map_err(|_| Error::PersistenceFailed)?;
    if bytes.len() as u64 > 24 * MAX_CONFIG {
        return Err(Error::PersistenceFailed);
    }
    let directory = root.join("claude-linux-homebrew-receipts");
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => super::super::sync_config_directory(root)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(Error::PersistenceFailed),
    }
    Directory::open(&directory)?.identity()?;
    let path = directory.join(format!("{}.json", journal.id));
    super::super::plain_ancestors(&path)?;
    if fs::symlink_metadata(&path).is_ok() {
        if read_limited(&path, 24 * MAX_CONFIG)? != bytes {
            return Err(Error::SourceChanged);
        }
        return Ok(());
    }
    let mut file = NamedTempFile::new_in(&directory).map_err(|_| Error::PersistenceFailed)?;
    file.write_all(&bytes)
        .map_err(|_| Error::PersistenceFailed)?;
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    file.persist_noclobber(&path)
        .map_err(|_| Error::SourceChanged)?;
    super::super::sync_config_directory(&directory)
}
