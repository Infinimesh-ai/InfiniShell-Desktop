use super::*;

#[test]
fn fixed_skill_prompt_preserves_selection_order_and_rejects_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let home = root.join("profile");
    std::fs::create_dir_all(home.join("grok")).unwrap();
    let selected: Vec<_> = ["alpha", "beta"]
        .into_iter()
        .map(|name| {
            let source = root.join(name);
            std::fs::create_dir(&source).unwrap();
            let path = source.join("SKILL.md");
            std::fs::write(&path, format!("技能 {name} 的合成正文。\n")).unwrap();
            SelectedLocalSkill {
                name: name.into(),
                path,
            }
        })
        .collect();
    let ceiling = GrokSkillCeilingV1::capture(&selected).unwrap();
    ceiling.prepare_home(&home).unwrap();
    let commands = Value::Array(
        selected
            .iter()
            .map(|skill| {
                json!({
                    "name": skill.name,
                    "input": null,
                    "_meta": {
                        "qualifiedName": format!("user:{}", skill.name),
                        "scope": "user",
                        "bareName": skill.name,
                        "path": home.join("grok/skills").join(&skill.name).join("SKILL.md"),
                    },
                })
            })
            .collect(),
    );
    let skill_input = |skill: &SelectedLocalSkill| InputContent::Skill {
        name: skill.name.clone(),
        path: skill.path.clone(),
    };

    let encoded = ceiling
        .encode_prompt(
            &home,
            &commands,
            vec![
                InputContent::Text("按所选顺序调用。".into()),
                skill_input(&selected[1]),
                skill_input(&selected[0]),
            ],
        )
        .unwrap();
    assert_eq!(encoded.len(), 1);
    let text = encoded[0]["text"].as_str().unwrap();
    assert!(text.starts_with("按所选顺序调用。\n\n"));
    assert!(text.contains(r#"["user:beta","user:alpha"]"#));
    assert!(
        ceiling
            .encode_prompt(
                &home,
                &commands,
                vec![skill_input(&selected[1]), skill_input(&selected[1])],
            )
            .is_err()
    );
    assert_eq!(ceiling.selected(), selected);
}
