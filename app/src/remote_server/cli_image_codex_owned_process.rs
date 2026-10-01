//! 专属 Codex 票据的终止能力；普通消费者只复用共享的只读身份。

use std::io;

pub(super) use super::super::cli_image_native_lifetime::Token;

impl Token {
    /// 调用者先证明子进程属于当前持久票据；这里仅防止 PID 复用。
    pub(super) fn terminate(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            command::managed::linux_signal_owned_process(
                self.linux_identity()?,
                &self.boot,
                libc::SIGTERM,
            )
        }
        #[cfg(target_os = "macos")]
        {
            command::managed::macos_signal_owned_process(
                self.macos_identity()?,
                &self.boot,
                libc::SIGTERM,
            )
        }
    }
}
