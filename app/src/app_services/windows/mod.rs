use registry::register_uri_handler;
#[cfg(feature = "release_bundle")]
use warp_errors::report_error;
use warpui::AppContext;
#[cfg(feature = "release_bundle")]
use {
    service_impl::forward_uri_to_sole_running_instance,
    single_instance_manager::SingleInstanceManager, thiserror::Error, url::Url,
    warp_core::channel::ChannelState,
};

mod registry;
#[cfg(feature = "release_bundle")]
mod service_impl;
#[cfg(feature = "release_bundle")]
mod single_instance_manager;

#[derive(Error, Debug)]
#[cfg(feature = "release_bundle")]
pub enum StartupArgsForwardingError {
    #[error("should not forward arguments after an auto-update")]
    IgnoredAfterAutoUpdate,
    #[error("there is no other instance of InfiniShell")]
    NoExistingInstance,
    #[error("failed to construct url")]
    CouldNotCreateUrl(#[from] url::ParseError),
    #[error("IPC Client failed to send message")]
    IpcError(#[from] ipc::ClientError),
    #[error("Win32 error")]
    WindowsError(#[from] windows::core::Error),
}

#[cfg(feature = "release_bundle")]
pub fn pass_startup_args_to_existing_instance(
    args: &warp_cli::AppArgs,
) -> Result<(), StartupArgsForwardingError> {
    if args.finish_update {
        return Err(StartupArgsForwardingError::IgnoredAfterAutoUpdate);
    }
    if SingleInstanceManager::is_sole_running_instance()? {
        return Err(StartupArgsForwardingError::NoExistingInstance);
    }

    warpui::r#async::block_on(async {
        if args.urls.is_empty() {
            // If there are no URLs on the command line, send one to open a new
            // window using the same current working directory as this process.
            let mut open_new_url = format!("{}://action/new_window", ChannelState::url_scheme());
            if let Ok(current_dir) = std::env::current_dir() {
                match current_dir.into_os_string().into_string() {
                    Ok(current_dir) => open_new_url.push_str(&format!("?path={}", current_dir)),
                    Err(os_string) => {
                        report_error!(
                            "Failed to convert OsString to String",
                            extra: { "os_string" => ?os_string }
                        );
                    }
                }
            }

            let url = Url::parse(&open_new_url)?;
            forward_uri_to_sole_running_instance(vec![url]).await?
        } else {
            forward_uri_to_sole_running_instance(args.urls.clone()).await?
        }

        Ok(())
    })
}

pub(super) fn init(_ctx: &mut AppContext) {
    #[cfg(feature = "release_bundle")]
    _ctx.add_singleton_model(SingleInstanceManager::new);

    // 注册表 URI handler 注册(warp:// 协议)是纯同步多次写 HKCU,
    // 与主线程任何 UI 初始化都没依赖。冷启动耗时 ~30–60 ms,
    // 丢到后台线程跳出关键路径。失败逻辑原本就是 log::error,
    // 后台跳不会错过任何信息。
    std::thread::Builder::new()
        .name("warp-uri-handler-register".into())
        .spawn(register_uri_handler)
        .ok();
}
