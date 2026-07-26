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
