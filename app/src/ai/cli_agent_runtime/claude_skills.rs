//! 固定 2.1.280 的插件热注册事务；原生确认之前保留输入，不提前派发。

use super::super::local_skills::{CLAUDE_SKILL_PLUGIN_NAME, SelectedLocalSkill};
use std::collections::HashSet;

use super::*;

impl ClaudeProtocol {
    pub(super) fn expire_skill_registrations(&mut self) -> Effects {
        let expired = self
            .pending
            .iter()
            .filter_map(|(id, request)| {
                (matches!(request.kind, PendingKind::ReloadSkills { .. })
                    && request.sent_at.elapsed() >= REQUEST_TIMEOUT)
                    .then_some(id.clone())
            })
            .collect::<Vec<_>>();
        let mut effects = Effects::default();
        for id in expired {
            if let Some(PendingRequest {
                kind: PendingKind::ReloadSkills { message_id, .. },
                ..
            }) = self.pending.remove(&id)
            {
                self.skill_reload_failed = true;
                effects.events.extend(
                    failed(
                        message_id,
                        &crate::t!("cli-agent-claude-skill-reload-failed"),
                    )
                    .events,
                );
            }
        }
        for event in &effects.events {
            self.remember_response(event);
        }
        effects
    }

    pub(super) fn begin_skill_registration(
        &mut self,
        message_id: Uuid,
        input: &[InputContent],
    ) -> Result<Option<Effects>, String> {
        if self.skill_reload_failed {
            return Err(crate::t!("cli-agent-claude-skill-reload-failed"));
        }
        if self
            .pending
            .values()
            .any(|request| matches!(request.kind, PendingKind::ReloadSkills { .. }))
        {
            return Err(crate::t!("cli-agent-claude-skill-reload-pending"));
        }
        let selected = input
            .iter()
            .filter_map(|part| match part {
                InputContent::Skill { name, path } => Some(SelectedLocalSkill {
                    name: name.clone(),
                    path: path.clone(),
                }),
                InputContent::Text(_) | InputContent::LocalImage(_) => None,
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Ok(None);
        }
        let mut names = HashSet::new();
        if selected.iter().any(|skill| !names.insert(&skill.name)) {
            return Err(crate::t!("cli-agent-claude-skill-duplicate"));
        }
        if self.options.permission_policy.is_claude_file_profile() {
            return Err(crate::t!("cli-task-manager-permission-claude-files-skills"));
        }
        if selected.len() > 1
            && input
                .iter()
                .any(|part| matches!(part, InputContent::LocalImage(_)))
        {
            return Err(crate::t!(
                "cli-agent-claude-image-multiple-skills-unverified"
            ));
        }
        if selected.len() > 1
            && (self.probed_version != Some("2.1.280")
                || self.options.permission_policy != PermissionPolicy::Inherit)
        {
            return Err(crate::t!("cli-agent-claude-skill-version-required"));
        }
        let Some(plugin) = &self.skill_plugin else {
            return Ok(None);
        };
        if selected
            .iter()
            .all(|skill| plugin.command_for(&skill.name, &skill.path).is_some())
        {
            return Ok(None);
        }
        if self.probed_version != Some("2.1.280")
            || self.options.permission_policy != PermissionPolicy::Inherit
        {
            return Err(crate::t!("cli-agent-claude-skill-version-required"));
        }
        if self.turns.values().any(|turn| !turn.finished) {
            return Err(crate::t!("cli-agent-claude-skill-reload-idle"));
        }
        let Some(update) = plugin.stage_additions(&selected)? else {
            return Ok(None);
        };
        let request = self.request(
            PendingKind::ReloadSkills {
                message_id,
                input: input.to_vec(),
                session_id: self.session_id.clone(),
                update,
            },
            json!({"subtype":"reload_plugins"}),
        );
        Ok(Some(Effects {
            writes: vec![request],
            events: Vec::new(),
        }))
    }

    pub(super) fn finish_skill_registration(
        &mut self,
        message_id: Uuid,
        input: Vec<InputContent>,
        session_id: Option<String>,
        update: PreparedClaudeSkillUpdate,
        response: &Value,
        success: bool,
    ) -> Effects {
        let registered = self.skill_plugin.as_ref().is_some_and(|plugin| {
            let expected = plugin
                .command_names()
                .into_iter()
                .chain(update.command_names())
                .collect::<Vec<_>>();
            success
                && self.probed_version == Some("2.1.280")
                && self.session_id == session_id
                && !self.turns.values().any(|turn| !turn.finished)
                && response["response"]["error_count"] == 0
                && response["response"]["commands"]
                    .as_array()
                    .is_some_and(|commands| {
                        expected.iter().all(|name| {
                            commands
                                .iter()
                                .filter(|command| command["name"].as_str() == Some(name))
                                .count()
                                == 1
                        })
                    })
                && response["response"]["plugins"]
                    .as_array()
                    .is_some_and(|plugins| {
                        plugins
                            .iter()
                            .filter(|entry| entry["name"] == CLAUDE_SKILL_PLUGIN_NAME)
                            .count()
                            == 1
                            && plugins.iter().any(|entry| {
                                entry["name"] == CLAUDE_SKILL_PLUGIN_NAME
                                    && entry["path"].as_str().is_some_and(|path| {
                                        Path::new(path) == plugin.plugin_directory()
                                    })
                                    && entry["version"] == "0.1.0"
                            })
                    })
        });
        if !registered {
            // 不再向可能只重载一部分的原生连接发送输入；重新连接时从原始路径重建。
            self.skill_reload_failed = true;
            return failed(
                message_id,
                &crate::t!("cli-agent-claude-skill-reload-failed"),
            );
        }
        let plugin = self.skill_plugin.as_mut().expect("已校验插件归属");
        update.commit(plugin);
        self.options.selected_skills = plugin.selected().to_vec();
        self.apply_command(message_id, RuntimeAction::Submit { input })
    }
}

#[cfg(test)]
#[path = "claude_skills_tests.rs"]
mod tests;
