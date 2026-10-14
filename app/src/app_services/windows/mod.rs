use anyhow::Context;
use registry::register_uri_handler;
#[cfg(feature = "release_bundle")]
use warp_errors::report_error;
use warp_errors::report_if_error;
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
    /// 恢复监视进程不能向父进程转发新窗口请求，否则每次启动都会多开窗口。
    #[error("should not forward arguments from the crash recovery process")]
    IgnoredForCrashRecoveryProcess,
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
    if crate::crash_recovery::is_crash_recovery_process(args) {
        return Err(StartupArgsForwardingError::IgnoredForCrashRecoveryProcess);
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

    // GUI 初始化返回前完成关联恢复；失败保留阶段与 Win32 错误，不静默丢弃。
    report_if_error!(register_uri_handler().context("Could not register Windows URI handler"));
}

#[cfg(all(test, feature = "release_bundle"))]
#[path = "../startup_tests.rs"]
mod startup_tests;
