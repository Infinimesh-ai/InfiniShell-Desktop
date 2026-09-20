mod subscribers;

#[cfg(not(target_family = "wasm"))]
mod local_directory;
#[cfg(not(target_family = "wasm"))]
pub(crate) use local_directory::LocalDirectorySkillWatcher;

mod skill_watcher;
pub use skill_watcher::{SkillWatcher, SkillWatcherEvent};

mod utils;
pub use utils::{extract_skill_parent_directory, read_skills_from_directories};
