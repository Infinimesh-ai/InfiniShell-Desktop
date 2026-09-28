//! 补全文件是独立的 Homebrew artifact，使用既有无覆盖认领/恢复原语逐项发布。

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use super::{ConfigBackup, ConfigDesired, ConfigKind, Error};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "CompletionRecord", into = "CompletionRecord")]
pub(super) struct Completion {
    forward: ConfigBackup,
    reverse: ConfigBackup,
}

// 每个脚本只保存新旧各一份 Base64，避免 Vec<u8> JSON 数组及正反账本的重复膨胀。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionRecord {
    path: PathBuf,
    before: Option<String>,
    after: String,
    mode: Option<u32>,
    forward_stage: Option<PathBuf>,
    reverse_stage: Option<PathBuf>,
}

impl From<Completion> for CompletionRecord {
    fn from(value: Completion) -> Self {
        Self {
            path: value.forward.path,
            before: value.forward.before.map(|bytes| STANDARD.encode(bytes)),
            after: STANDARD.encode(value.reverse.after.unwrap_or_default()),
            mode: value.forward.before_mode,
            forward_stage: value.forward.restore_stage,
            reverse_stage: value.reverse.restore_stage,
        }
    }
}

impl TryFrom<CompletionRecord> for Completion {
    type Error = &'static str;

    fn try_from(value: CompletionRecord) -> Result<Self, Self::Error> {
        let decode = |text: String| {
            if text.len() > 4 * (super::MAX_CONFIG as usize + 2) / 3 {
                return Err("补全账本过大");
            }
            let bytes = STANDARD.decode(text).map_err(|_| "补全账本编码无效")?;
            if bytes.len() as u64 > super::MAX_CONFIG {
                return Err("补全账本过大");
            }
            Ok(bytes)
        };
        let before = value.before.map(decode).transpose()?;
        let after = decode(value.after)?;
        if after.is_empty() {
            return Err("补全账本输出为空");
        }
        Ok(Self {
            forward: ConfigBackup {
                kind: ConfigKind::BrewCompletion,
                path: value.path.clone(),
                before: before.clone(),
                after: before.clone(),
                desired: Some(ConfigDesired {
                    bytes: Some(after.clone()),
                }),
                before_mode: value.mode,
                restore_stage: value.forward_stage,
            },
            reverse: ConfigBackup {
                kind: ConfigKind::BrewCompletion,
                path: value.path,
                before,
                after: Some(after),
                desired: None,
                before_mode: value.mode,
                restore_stage: value.reverse_stage,
            },
        })
    }
}

impl Completion {
    pub(super) fn prepare(
        path: PathBuf,
        old_generated: &[u8],
        generated: Vec<u8>,
    ) -> Result<Self, Error> {
        super::plain_ancestors(&path)?;
        let before = super::read_optional_config(&path)?;
        if before.as_ref().is_some_and(|bytes| bytes != old_generated) || generated.is_empty() {
            return Err(Error::SourceChanged);
        }
        let mode = if before.is_some() {
            let metadata = fs::symlink_metadata(&path).map_err(|_| Error::SourceChanged)?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
                || metadata.mode() & 0o7022 != 0
            {
                return Err(Error::UnsupportedSource);
            }
            let file = fs::File::open(&path).map_err(|_| Error::SourceChanged)?;
            super::package_tree::reject_extra_permissions(&file)?;
            Some(metadata.mode() & 0o777)
        } else {
            Some(0o644)
        };
        let mut forward = ConfigBackup {
            kind: ConfigKind::BrewCompletion,
            path: path.clone(),
            before: before.clone(),
            after: before.clone(),
            desired: Some(ConfigDesired {
                bytes: Some(generated.clone()),
            }),
            before_mode: mode,
            restore_stage: None,
        };
        let mut reverse = ConfigBackup {
            kind: ConfigKind::BrewCompletion,
            path,
            before,
            after: Some(generated),
            desired: None,
            before_mode: mode,
            restore_stage: None,
        };
        super::plan_config_publish(&mut forward, true)?;
        super::plan_config_publish(&mut reverse, false)?;
        Ok(Self { forward, reverse })
    }

    pub(super) fn validate(&self, expected: &Path) -> Result<(), Error> {
        if self.forward.path != expected
            || self.reverse.path != expected
            || !matches!(self.forward.kind, ConfigKind::BrewCompletion)
            || !matches!(self.reverse.kind, ConfigKind::BrewCompletion)
            || self.forward.before != self.forward.after
            || self.forward.before != self.reverse.before
            || self
                .forward
                .desired
                .as_ref()
                .and_then(|desired| desired.bytes.as_ref())
                != self.reverse.after.as_ref()
            || self.reverse.desired.is_some()
            || self.forward.before_mode != self.reverse.before_mode
        {
            return Err(Error::RecoveryRequired);
        }
        super::config_restore_stage(&self.forward)?;
        super::config_restore_stage(&self.reverse)?;
        Ok(())
    }

    pub(super) fn verify_original(&self) -> Result<(), Error> {
        if super::read_optional_config(&self.forward.path)? != self.forward.before {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    pub(super) fn publish(&self) -> Result<(), Error> {
        super::publish_config(&self.forward, true)
    }

    pub(super) fn verify_published(&self) -> Result<(), Error> {
        if super::read_optional_config(&self.forward.path)? != self.reverse.after {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    pub(super) fn verify_rollback(&self) -> Result<(), Error> {
        self.validate(&self.forward.path)?;
        super::plain_ancestors(&self.forward.path)?;
        let forward_claimed = verify_claimed(&self.forward)?;
        let reverse_claimed = verify_claimed(&self.reverse)?;
        let actual = super::read_optional_config(&self.forward.path)?;
        if actual == self.forward.before {
            return Ok(());
        }
        if self.forward.before == self.reverse.after
            || self.reverse.restore_stage.is_none()
            || self.reverse.before_mode.is_some_and(|mode| mode > 0o777)
        {
            return Err(Error::RecoveryRequired);
        }
        if actual == self.reverse.after {
            // 逆向认领后原路径再次出现，即使字节相同也不能覆盖该文件。
            return if reverse_claimed.is_none() {
                Ok(())
            } else {
                Err(Error::RecoveryRequired)
            };
        }
        if actual.is_some() {
            return Err(Error::SourceChanged);
        }
        if reverse_claimed == self.reverse.after {
            return Ok(());
        }
        // 正向认领后尚未发布：原文件必须仍在账本指定的 claimed 中。
        if self.forward.restore_stage.is_some()
            && forward_claimed.is_some()
            && forward_claimed == self.forward.after
        {
            return Ok(());
        }
        Err(Error::RecoveryRequired)
    }

    pub(super) fn rollback(&self) -> Result<(), Error> {
        self.verify_rollback()?;
        let actual = super::read_optional_config(&self.forward.path)?;
        if actual == self.forward.before {
            return Ok(());
        }
        if actual.is_none() {
            let claimed = self
                .reverse
                .restore_stage
                .as_ref()
                .map(|stage| stage.join("claimed"));
            if let Some(claimed) = claimed
                && super::read_optional_config(&claimed)?.is_some()
            {
                // 逆向认领已经完成，直接继续原有回滚，不重建会与它冲突的新文件。
                return super::publish_config(&self.reverse, false);
            }
            // 原文件已被认领而新文件尚未出现，先收敛发布，再按逆向账本恢复旧内容。
            self.publish()?;
        } else if actual != self.reverse.after {
            return Err(Error::SourceChanged);
        }
        super::publish_config(&self.reverse, false)
    }

    pub(super) fn cleanup(&self) -> Result<(), Error> {
        super::cleanup_config_restore(&self.forward)?;
        super::cleanup_config_restore(&self.reverse)
    }
}

// 只读取两份既有发布账本；未知暂存内容不能等其他 artifact 已回滚后才发现。
fn verify_claimed(config: &ConfigBackup) -> Result<Option<Vec<u8>>, Error> {
    let Some(stage) = super::config_restore_stage(config)? else {
        return Ok(None);
    };
    match fs::symlink_metadata(stage) {
        Ok(metadata) if !metadata.is_dir() => return Err(Error::RecoveryRequired),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::RecoveryRequired),
    }
    for entry in fs::read_dir(stage).map_err(|_| Error::RecoveryRequired)? {
        if entry.map_err(|_| Error::RecoveryRequired)?.file_name() != "claimed" {
            return Err(Error::RecoveryRequired);
        }
    }
    let claimed = super::read_optional_config(&stage.join("claimed"))?;
    if claimed.is_some() && claimed != config.after {
        return Err(Error::RecoveryRequired);
    }
    Ok(claimed)
}
