use anyhow::anyhow;

use super::*;

fn ssh_info() -> InteractiveSshCommand {
    InteractiveSshCommand {
        host: Some("root@Web-01".to_owned()),
        port: None,
    }
}

#[test]
fn disabled_or_non_legacy_session_skips_loading() {
    let info = ssh_info();
    assert_eq!(
        load_with(false, true, Some(&info), |_| -> anyhow::Result<_> {
            panic!("disabled setting must not access the database")
        }),
        None
    );
    assert_eq!(
        load_with(true, false, Some(&info), |_| -> anyhow::Result<_> {
            panic!("local sessions must not access the database")
        }),
        None
    );
}

#[test]
fn missing_memory_keeps_machine_identity_with_empty_content() {
    let info = ssh_info();
    let context = load_with(true, true, Some(&info), |_| Ok::<_, anyhow::Error>(None)).unwrap();
    assert_eq!(
        context,
        MachineMemoryContext {
            machine_key: "web-01:22".to_owned(),
            content: String::new(),
        }
    );
}

#[test]
fn loaded_memory_is_truncated_on_unicode_boundaries() {
    let info = ssh_info();
    let content = "机".repeat(INJECT_MAX_CHARS + 1);
    let context = load_with(true, true, Some(&info), |_| {
        Ok::<_, anyhow::Error>(Some(content))
    })
    .unwrap();
    assert_eq!(context.content.chars().count(), INJECT_MAX_CHARS);
    assert_eq!(context.content, "机".repeat(INJECT_MAX_CHARS));
}

#[test]
fn database_error_degrades_to_no_memory_context() {
    let info = ssh_info();
    let context = load_with(true, true, Some(&info), |_| {
        Err::<Option<String>, _>(anyhow!("no such table: ssh_machine_memories"))
    });
    assert_eq!(context, None);
}
