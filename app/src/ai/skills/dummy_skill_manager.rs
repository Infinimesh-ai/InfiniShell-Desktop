use std::path::PathBuf;

use ai::skills::{ParsedSkill, SkillPathOrigin, SkillProvider, SkillReference, SkillScope};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::skills::{
    ActiveSkillLookupError, SkillDescriptor, SkillManagerEvent, SkillPathQuery,
};

/// Skill 管理面板用的清单条目:同名 skill 的一份具体副本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInventoryDuplicate {
    pub path: PathBuf,
    pub name: String,
    pub description: String,
    pub content: String,
    pub provider: SkillProvider,
    pub scope: SkillScope,
}

/// 同名 skill 的一组副本,`default_skill` 是按 provider 优先级选出的生效项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInventoryItem {
    pub name: String,
    pub default_skill: SkillInventoryDuplicate,
    pub duplicates: Vec<SkillInventoryDuplicate>,
}

impl SkillInventoryItem {
    pub fn has_duplicates(&self) -> bool {
        self.duplicates.len() > 1
    }
}

pub struct SkillManager {}

impl SkillManager {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {}
    }

    pub fn get_skills_for_working_directory(
        &self,
        _working_directory: Option<&LocalOrRemotePath>,
        _ctx: &AppContext,
    ) -> Vec<SkillDescriptor> {
        vec![]
    }

    pub fn get_skills_for_working_directory_with_origin(
        &self,
        _working_directory: Option<&LocalOrRemotePath>,
        _path_origin: &SkillPathOrigin,
        _ctx: &AppContext,
    ) -> Vec<SkillDescriptor> {
        vec![]
    }

    pub fn list_skill_inventory(&self, ctx: &AppContext) -> Vec<SkillInventoryItem> {
        let _ = ctx;
        vec![]
    }

    pub fn skill_by_path<P: SkillPathQuery + ?Sized>(
        &self,
        _skill_path: &P,
    ) -> Option<&ParsedSkill> {
        None
    }
    pub fn reference_for_skill_path<P: SkillPathQuery + ?Sized>(
        &self,
        skill_path: &P,
    ) -> SkillReference {
        SkillReference::Path(skill_path.to_skill_location())
    }

    pub fn skill_by_reference(&self, _reference: &SkillReference) -> Option<&ParsedSkill> {
        None
    }

    pub fn find_skill_by_name(&self, _name: &str) -> Option<&ParsedSkill> {
        None
    }

    pub fn active_skill_by_reference(
        &self,
        _reference: &SkillReference,
        _ctx: &AppContext,
    ) -> Option<&ParsedSkill> {
        None
    }
    pub fn active_skill_by_reference_with_origin(
        &self,
        reference: &SkillReference,
        path_origin: &SkillPathOrigin,
        _ctx: &AppContext,
    ) -> Result<&ParsedSkill, ActiveSkillLookupError> {
        Err(ActiveSkillLookupError::for_reference(
            reference,
            path_origin,
        ))
    }

    pub fn active_local_bundled_skill(&self, _id: &str, _ctx: &AppContext) -> Option<&ParsedSkill> {
        None
    }

    pub fn skill_exists_for_any_provider(
        &self,
        _skill: &SkillDescriptor,
        _providers: &[SkillProvider],
    ) -> bool {
        false
    }

    pub fn best_supported_provider(
        &self,
        skill: &SkillDescriptor,
        _supported_providers: &[SkillProvider],
    ) -> SkillProvider {
        skill.provider
    }
}

impl Entity for SkillManager {
    type Event = SkillManagerEvent;
}

impl SingletonEntity for SkillManager {}
