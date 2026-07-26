use super::*;
use crate::ai::machine_memory::MachineMemoryContext;

#[test]
fn render_machine_memory_block_none() {
    assert_eq!(render_machine_memory_block(None), None);
}

#[test]
fn render_machine_memory_block_empty() {
    let context = MachineMemoryContext {
        machine_key: "web-01:&\"".to_owned(),
        content: String::new(),
    };
    assert_eq!(
        render_machine_memory_block(Some(&context)).unwrap(),
        "\n\n<machine_memory machine_key=\"web-01:&amp;&quot;\">\n  \
         <fact>Accumulated notes from previous sessions on this same remote machine.\n  \
         They may be stale — verify before relying on them for destructive actions.</fact>\n  \
         <content>\n(no memory recorded for this machine yet)\n  </content>\n  \
         <rules>\n  \
         - When you learn a durable fact about THIS machine (OS/services layout, deploy conventions, gotchas, non-standard paths), call `update_machine_memory` with the full revised memory document.\n  \
         - Never store credentials, tokens or private keys in machine memory.\n  \
         </rules>\n\
         </machine_memory>"
    );
}

#[test]
fn render_machine_memory_block_non_empty() {
    let context = MachineMemoryContext {
        machine_key: "web-01:22".to_owned(),
        content: "## Services\nnginx lives in /opt/nginx & uses <custom.conf>".to_owned(),
    };
    assert_eq!(
        render_machine_memory_block(Some(&context)).unwrap(),
        "\n\n<machine_memory machine_key=\"web-01:22\">\n  \
         <fact>Accumulated notes from previous sessions on this same remote machine.\n  \
         They may be stale — verify before relying on them for destructive actions.</fact>\n  \
         <content>\n## Services\nnginx lives in /opt/nginx &amp; uses &lt;custom.conf&gt;\n  </content>\n  \
         <rules>\n  \
         - When you learn a durable fact about THIS machine (OS/services layout, deploy conventions, gotchas, non-standard paths), call `update_machine_memory` with the full revised memory document.\n  \
         - Never store credentials, tokens or private keys in machine memory.\n  \
         </rules>\n\
         </machine_memory>"
    );
}

#[test]
fn render_known_ssh_machines_block_none() {
    assert_eq!(render_known_ssh_machines_block(None), None);
}

#[test]
fn render_known_ssh_machines_block_escapes_index_and_forbids_auto_connect() {
    assert_eq!(
        render_known_ssh_machines_block(Some(
            "- web-01:22: nginx uses <custom.conf> & /opt/nginx"
        ))
        .unwrap(),
        "\n\n<known_ssh_machines>\n  \
         <fact>These are remote SSH machines known from previous sessions. Use this index to identify a machine when the user refers to it by name.</fact>\n  \
         <machines>\n- web-01:22: nginx uses &lt;custom.conf&gt; &amp; /opt/nginx\n  </machines>\n  \
         <rules>\n  \
         - Use the summaries only as potentially stale context; verify before relying on them for destructive actions.\n  \
         - Connections must be initiated by the user or through SSH Manager. Ask the user to run or suggest that they run `ssh &lt;host&gt;`, using the host portion of the machine key. Never initiate an SSH connection automatically.\n  \
         </rules>\n\
         </known_ssh_machines>"
    );
}

#[test]
fn machine_memory_tool_is_gated_by_machine_context() {
    let mut params = RequestParams::new_for_test(Vec::new(), Vec::new());

    assert!(!available_tool_names(&params)
        .iter()
        .any(|name| name == tools::machine_memory::TOOL_NAME));
    assert!(!build_tools_array(&params)
        .iter()
        .any(|tool| tool.name == tools::machine_memory::TOOL_NAME.into()));

    params.machine_memory = Some(MachineMemoryContext {
        machine_key: "web-01:22".to_owned(),
        content: String::new(),
    });

    assert!(available_tool_names(&params)
        .iter()
        .any(|name| name == tools::machine_memory::TOOL_NAME));
    assert!(build_tools_array(&params)
        .iter()
        .any(|tool| tool.name == tools::machine_memory::TOOL_NAME.into()));
}

#[test]
fn missing_machine_context_returns_intercepted_error_without_writing() {
    let result = dispatch_byop_machine_memory_tool_with(
        None,
        r#"{"content":"remember me"}"#,
        |_, _| -> Result<(), &'static str> { panic!("missing key must not write") },
    );

    assert_eq!(result["_byop_intercepted"], true);
    assert_eq!(result["status"], "error");
    assert_eq!(
        result["message"],
        "not in an ssh session with machine identity"
    );
}

#[test]
fn dispatch_truncates_unicode_and_reports_stored_character_count() {
    let memory = MachineMemoryContext {
        machine_key: "web-01:22".to_owned(),
        content: String::new(),
    };
    let args = serde_json::json!({
        "content": "机".repeat(warp_ssh_manager::MAX_MEMORY_CHARS + 1),
    })
    .to_string();
    let mut written = None;

    let result =
        dispatch_byop_machine_memory_tool_with(Some(&memory), &args, |machine_key, content| {
            written = Some((machine_key.to_owned(), content.to_owned()));
            Ok::<(), &'static str>(())
        });

    let (machine_key, content) = written.unwrap();
    assert_eq!(machine_key, "web-01:22");
    assert_eq!(content.chars().count(), warp_ssh_manager::MAX_MEMORY_CHARS);
    assert_eq!(result["_byop_intercepted"], true);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["stored_chars"], warp_ssh_manager::MAX_MEMORY_CHARS);
}

#[test]
fn dispatch_database_error_keeps_auto_resume_sentinel() {
    let memory = MachineMemoryContext {
        machine_key: "web-01:22".to_owned(),
        content: String::new(),
    };
    let result = dispatch_byop_machine_memory_tool_with(
        Some(&memory),
        r#"{"content":"remember me"}"#,
        |_, _| Err("database unavailable"),
    );

    assert_eq!(result["_byop_intercepted"], true);
    assert_eq!(result["status"], "error");
    assert_eq!(result["message"], "failed to update machine memory");
}
