fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/remote_server.proto");
    println!("cargo:rerun-if-changed=proto/diff_state.proto");
    // 调试父级 envelope 时也必须隐藏一次性挑战和绑定凭证。
    let mut config = prost_build::Config::new();
    config.skip_debug([
        ".remote_server.TerminalBindingAck",
        ".remote_server.TerminalBindingBound",
    ]);
    config.compile_protos(
        &["proto/remote_server.proto", "proto/diff_state.proto"],
        &["proto/"],
    )?;
    Ok(())
}
