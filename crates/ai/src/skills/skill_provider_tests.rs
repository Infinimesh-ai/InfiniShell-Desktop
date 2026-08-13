use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

use super::{
    SkillProvider, SkillScope, get_provider_for_path, get_scope_for_path, home_skills_path,
    provider_parent_directory_for_skills_root,
};

#[test]
fn infinishell_provider_reads_legacy_serialization() {
    for legacy_name in [r#""Warp""#, r#""Zap""#] {
        assert_eq!(
            serde_json::from_str::<SkillProvider>(legacy_name).unwrap(),
            SkillProvider::InfiniShell
        );
    }
    assert_eq!(
        serde_json::to_string(&SkillProvider::InfiniShell).unwrap(),
        r#""InfiniShell""#
    );
}

#[test]
fn infinishell_provider_reads_legacy_strings() {
    assert_eq!("Warp".parse(), Ok(SkillProvider::InfiniShell));
    assert_eq!("Zap".parse(), Ok(SkillProvider::InfiniShell));
    assert_eq!("InfiniShell".parse(), Ok(SkillProvider::InfiniShell));
    assert_eq!(SkillProvider::InfiniShell.to_string(), "InfiniShell");
}

#[test]
fn warp_home_skills_path_uses_warp_home_path() {
    assert_eq!(
        home_skills_path(SkillProvider::InfiniShell),
        warp_core::paths::warp_home_skills_dir()
    );
}

#[test]
fn warp_home_skill_path_is_home_warp_skill() {
    let Some(warp_home_skills_dir) = warp_core::paths::warp_home_skills_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };
    let path = warp_home_skills_dir.join("my-skill").join("SKILL.md");

    assert_eq!(
        get_provider_for_path(&LocalOrRemotePath::Local(path.clone())),
        Some(SkillProvider::InfiniShell)
    );
    assert_eq!(get_scope_for_path(&path), SkillScope::Home);
}

#[test]
fn remote_provider_path_is_classified_by_structure() {
    let path = LocalOrRemotePath::Remote(RemotePath::new(
        HostId::new("remote-host".to_string()),
        StandardizedPath::try_new("/repo/.claude/skills/my-skill/SKILL.md").unwrap(),
    ));

    assert_eq!(get_provider_for_path(&path), Some(SkillProvider::Claude));
}

#[test]
fn local_project_provider_path_is_classified_by_structure() {
    let path = LocalOrRemotePath::Local(
        std::env::temp_dir()
            .join("repo")
            .join(".claude")
            .join("skills")
            .join("my-skill")
            .join("SKILL.md"),
    );

    assert_eq!(get_provider_for_path(&path), Some(SkillProvider::Claude));
}

#[test]
fn foreign_encoded_remote_provider_path_is_classified_by_structure() {
    let path = LocalOrRemotePath::Remote(RemotePath::new(
        HostId::new("remote-host".to_string()),
        StandardizedPath::try_new(r"C:\repo\.codex\skills\my-skill\SKILL.md").unwrap(),
    ));

    assert_eq!(get_provider_for_path(&path), Some(SkillProvider::Codex));
}

#[test]
fn foreign_encoded_remote_skills_root_resolves_provider_parent_directory() {
    let host_id = HostId::new("remote-host".to_string());
    let skills_root = LocalOrRemotePath::Remote(RemotePath::new(
        host_id.clone(),
        StandardizedPath::try_new(r"C:\repo\.agents\skills").unwrap(),
    ));

    assert_eq!(
        provider_parent_directory_for_skills_root(&skills_root),
        Some(LocalOrRemotePath::Remote(RemotePath::new(
            host_id,
            StandardizedPath::try_new(r"C:\repo").unwrap(),
        )))
    );
}
