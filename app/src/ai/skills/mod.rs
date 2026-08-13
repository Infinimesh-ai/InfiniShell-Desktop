use std::path::{Path, PathBuf};

use ai::skills::SkillPathOrigin;
use warp_util::local_or_remote_path::LocalOrRemotePath;

mod telemetry;
pub use telemetry::{SkillOpenOrigin, SkillTelemetryEvent};
#[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
mod remote;
#[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
pub(crate) use remote::bundled_skill_snapshot_protos;
#[cfg(feature = "local_fs")]
mod bundled;
#[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
pub(crate) use bundled::{BundledSkill, BundledSkillActivation};

cfg_if::cfg_if! {
    if #[cfg(not(feature = "local_fs"))] {
        mod dummy_skill_manager;
        pub use dummy_skill_manager::{
            SkillInventoryDuplicate, SkillInventoryItem, SkillManager,
        };
    }
}

pub use ai::skills::SkillReference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum SkillManagerEvent {
    SkillsChanged {
        home_skills_changed: bool,
    },
    /// Zap 自有:skill 清单发生变化,Skill 管理面板据此刷新。
    InventoryChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActiveSkillLookupError {
    #[error("Bundled skills are not available on this remote session")]
    BundledSkillsUnavailable,
    #[error("Skill not found: {reference}")]
    NotFound { reference: SkillReference },
}

impl ActiveSkillLookupError {
    pub(crate) fn for_reference(reference: &SkillReference, path_origin: &SkillPathOrigin) -> Self {
        if matches!(path_origin, SkillPathOrigin::Unavailable)
            && matches!(reference, SkillReference::BundledSkillId(_))
        {
            Self::BundledSkillsUnavailable
        } else {
            Self::NotFound {
                reference: reference.clone(),
            }
        }
    }
}

#[cfg(not(target_family = "wasm"))]
mod global_skills;
#[cfg(not(target_family = "wasm"))]
pub use global_skills::{filter_skills_by_spec, resolve_skill_repos};

mod listed_skill;
pub use listed_skill::SkillDescriptor;

mod skill_utils;
// Zap:上游的 `list_skills_if_changed`(云端差量发送)在本地化后简化为每轮全量的
// `list_skills`;`skill_path_from_file_path` 亦已更名为 `skill_path_from_location`。
pub use skill_utils::{
    icon_override_for_skill_name, list_skills, render_skill_button, skill_path_from_location,
};
pub trait SkillPathQuery {
    fn to_skill_location(&self) -> LocalOrRemotePath;
}

impl SkillPathQuery for LocalOrRemotePath {
    fn to_skill_location(&self) -> LocalOrRemotePath {
        self.clone()
    }
}

impl SkillPathQuery for Path {
    fn to_skill_location(&self) -> LocalOrRemotePath {
        LocalOrRemotePath::Local(self.to_path_buf())
    }
}

impl SkillPathQuery for PathBuf {
    fn to_skill_location(&self) -> LocalOrRemotePath {
        LocalOrRemotePath::Local(self.clone())
    }
}

#[cfg(not(target_family = "wasm"))]
mod resolve_skill_spec;
#[cfg(not(target_family = "wasm"))]
pub use resolve_skill_spec::{
    ResolveSkillError, ResolvedSkill, clone_repo_for_skill, resolve_skill_spec,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        mod skill_manager;
        pub use skill_manager::{
            SkillInventoryDuplicate, SkillInventoryItem, SkillManager,
            extract_skill_parent_directory,
        };
        #[allow(unused_imports)]
        pub use skill_manager::{SkillWatcher, read_skills_from_directories};
    }
}
