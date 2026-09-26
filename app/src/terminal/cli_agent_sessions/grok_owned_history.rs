//! 显式继续普通 Grok 历史：只产生新的进程代，不复制或重投任何旧输入。

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
#[cfg(feature = "local_fs")]
use serde_json::{Value, json};
use uuid::Uuid;

use super::grok_owned_launch::GrokOwnedLaunch;
#[cfg(feature = "local_fs")]
use crate::persistence::local_cli_tasks::grok_terminal::GrokTerminalOwner;
#[cfg(feature = "local_fs")]
use crate::persistence::model::{LocalCliTask, LocalCliTaskState};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistorySource {
    pub(crate) task_id: String,
    pub(crate) generation: i64,
    pub(crate) launch_id: Uuid,
    pub(crate) session_id: Uuid,
    pub(crate) manifest_path: PathBuf,
    pub(crate) manifest_sha256: String,
    pub(crate) cwd: PathBuf,
}

impl HistorySource {
    #[cfg(feature = "local_fs")]
    pub(crate) fn from_task(task: &LocalCliTask) -> io::Result<(Self, GrokTerminalOwner)> {
        let config: Value = serde_json::from_str(&task.config_json).map_err(|_| invalid())?;
        let owner: GrokTerminalOwner =
            serde_json::from_value(config["grok_terminal"].clone()).map_err(|_| invalid())?;
        if task.version != 1
            || task.harness != "grok"
            || task.generation < 1
            || task.revision < 0
            || task.parent_task_id.is_some()
            || task.parent_generation.is_some()
            || config["execution_kind"] != "grok_owned_terminal"
            || owner.version != 1
            || owner.launch_id.is_nil()
            || owner.session_id.is_nil()
            || owner.binding_id.is_nil()
            || owner.input_revision.is_nil()
            || owner.permission_revision.is_nil()
            || owner.cli_version != "1.0.41"
            || owner.model_id != "grok-4.7"
            || owner.permission_mode != "default"
            || task.native_session_id.as_deref() != Some(owner.session_id.to_string().as_str())
        {
            return Err(invalid());
        }
        let source = Self {
            task_id: task.task_id.clone(),
            generation: task.generation,
            launch_id: owner.launch_id,
            session_id: owner.session_id,
            manifest_path: PathBuf::from(config["launch_manifest"].as_str().ok_or_else(invalid)?),
            manifest_sha256: config["launch_sha256"].as_str().ok_or_else(invalid)?.into(),
            cwd: PathBuf::from(&task.working_directory),
        };
        source.validate()?;
        Ok((source, owner))
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        if !Uuid::parse_str(&self.task_id).is_ok_and(|id| !id.is_nil())
            || self.generation < 1
            || self.launch_id.is_nil()
            || self.session_id.is_nil()
            || !self.manifest_path.is_absolute()
            || !self.cwd.is_absolute()
            || self.manifest_sha256.len() != 64
            || !self
                .manifest_sha256
                .bytes()
                .all(|value| value.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn restore(&self) -> io::Result<GrokOwnedLaunch> {
        self.validate()?;
        let launch = GrokOwnedLaunch::restore(&self.manifest_path, &self.manifest_sha256)?;
        if launch.launch_id() != self.launch_id
            || launch.session_id() != self.session_id
            || launch.working_directory() != self.cwd
        {
            return Err(invalid());
        }
        Ok(launch)
    }

    /// 任务状态、Stop、socket 消失都不代表退出；三平台后端必须核实旧原生进程身份。
    pub(crate) fn verify_exited(&self) -> io::Result<()> {
        self.restore()?.confirm_history_exit()
    }

    pub(crate) fn validate_target(&self, cwd: &Path, session: Uuid) -> io::Result<()> {
        self.validate()?;
        if self.cwd != cwd || self.session_id != session {
            return Err(invalid());
        }
        Ok(())
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn continued_task(
        &self,
        previous: &LocalCliTask,
        launch: &GrokOwnedLaunch,
    ) -> io::Result<(LocalCliTask, GrokTerminalOwner)> {
        let (current, _) = Self::from_task(previous)?;
        if current.task_id != self.task_id
            || current.generation != self.generation
            || current.launch_id != self.launch_id
            || launch.launch_id() == self.launch_id
            || launch.session_id() != self.session_id
            || launch.working_directory() != self.cwd
        {
            return Err(invalid());
        }
        let owner = GrokTerminalOwner {
            version: 1,
            launch_id: launch.launch_id(),
            session_id: self.session_id,
            binding_id: Uuid::new_v4(),
            input_revision: Uuid::new_v4(),
            permission_revision: Uuid::new_v4(),
            cli_version: "1.0.41".into(),
            model_id: "grok-4.7".into(),
            permission_mode: "default".into(),
        };
        let mut task = previous.clone();
        task.generation = previous.generation.checked_add(1).ok_or_else(invalid)?;
        task.revision = 0;
        task.state = LocalCliTaskState::Queued;
        task.result = None;
        task.terminal_evidence = None;
        task.config_json = json!({"execution_kind":"grok_owned_terminal", "grok_terminal":owner,
            "launch_manifest":launch.manifest_path(), "launch_sha256":launch.manifest_sha256(),
            "history_source":self})
        .to_string();
        Ok((task, owner))
    }
}

fn invalid() -> io::Error {
    io::Error::other("Grok 普通历史会话身份不匹配")
}
