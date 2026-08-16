//! Windows PowerShell SSH wrapper 使用的控制台子进程入口。
//!
//! 正式版主程序使用 Windows GUI subsystem，不能保证 PowerShell
//! 直接调用时等待进程或保留交互式 stdio。该二进制不设置 GUI
//! subsystem，仅作为安装包内部的 Rust SSH worker 宿主。

use anyhow::Result;
use warp_core::AppId;
use warp_core::channel::{Channel, ChannelConfig, ChannelState};
use warp_core::features::DEBUG_FLAGS;

fn main() -> Result<()> {
    let mut state = ChannelState::new(
        Channel::Oss,
        ChannelConfig {
            app_id: AppId::new("dev", "infinishell", "InfiniShell"),
            logfile_name: "infinishell.log".into(),
            autoupdate_config: None,
            mcp_static_config: None,
        },
    );
    if cfg!(debug_assertions) {
        state = state.with_additional_features(DEBUG_FLAGS);
    }
    ChannelState::set(state);

    warp::run()
}
