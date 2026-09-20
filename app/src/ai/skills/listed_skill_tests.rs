use std::path::PathBuf;

use ai::skills::parse_skill_content_at_location;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::*;

#[test]
fn descriptor_preserves_user_invocable_from_content() {
    let skill = parse_skill_content_at_location(
        LocalOrRemotePath::Local(PathBuf::from(".claude/skills/example/SKILL.md")),
        "---\nname: example\nuser-invocable: false\n---\n技能正文\n",
        SkillProvider::Claude,
        SkillScope::Project,
    )
    .unwrap();
    let descriptor = SkillDescriptor::from(skill.clone());
    assert_eq!(
        descriptor.user_invocable,
        SkillUserInvocable::Boolean(false)
    );
    let bundled = SkillDescriptor::new_bundled("example".to_owned(), skill, Icon::InfiniShell);
    assert_eq!(bundled.user_invocable, SkillUserInvocable::Boolean(false));

    let encoded = serde_json::to_value(&descriptor).unwrap();
    let restored: SkillDescriptor = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(restored, descriptor);
    let mut legacy = encoded;
    legacy.as_object_mut().unwrap().remove("user_invocable");
    let restored: SkillDescriptor = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.user_invocable, SkillUserInvocable::Unspecified);
}
