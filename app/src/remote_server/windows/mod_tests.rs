use super::{bind_listener, proxy};

#[test]
fn named_pipe_listener_binds_inside_tokio_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime should be created");
    let identity_key = format!("windows-listener-test-{}", uuid::Uuid::new_v4());

    runtime.block_on(async {
        let listener = bind_listener(proxy::pipe_name(&identity_key))
            .await
            .expect("named pipe listener should bind inside Tokio runtime");
        drop(listener);
    });
}
