//! 已验证 Windows Codex npm 探针在原认证连接上的协作取消。

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) const COOPERATIVE_EXIT_TIMEOUT: Duration = Duration::from_secs(8);
const STOP_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn request_stop(stream: &mut TcpStream) -> io::Result<()> {
    let result = stream
        .set_write_timeout(Some(STOP_WRITE_TIMEOUT))
        .and_then(|()| stream.write_all(&[2]));
    // 写入失败时也关闭发送方向，让仍可达的监听者按 EOF 取消；没有重连或新授权。
    let _ = stream.shutdown(Shutdown::Write);
    result
}

pub(super) struct CancellationListener {
    cancelled: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl CancellationListener {
    /// 调用方必须先核对原握手和完整 launch binding，不能接受其他来源的连接。
    pub(super) fn start(control: TcpStream) -> io::Result<Self> {
        // Windows 接收超时后的连接不能继续复用；改用非阻塞读取及可唤醒的短等待。
        control.set_read_timeout(None)?;
        control.set_nonblocking(true)?;
        let mut reader_control = control;
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader_cancelled = Arc::clone(&cancelled);
        let reader = thread::Builder::new()
            .name("cli-npm-cancel".to_owned())
            .spawn(move || {
                let mut request = [0_u8; 1];
                while !reader_cancelled.load(Ordering::Acquire) {
                    match reader_control.read(&mut request) {
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::park_timeout(CANCELLATION_POLL_INTERVAL);
                        }
                        // 停止、EOF、其他错误或非预期字节均取消，不输出控制内容。
                        Ok(_) | Err(_) => {
                            reader_cancelled.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            cancelled,
            reader: Some(reader),
        })
    }

    pub(super) fn token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }
}

impl Drop for CancellationListener {
    fn drop(&mut self) {
        // 先发布取消再唤醒；unpark 令牌会保留，覆盖读取结束但尚未 park 的竞态。
        self.cancelled.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            reader.thread().unpark();
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
#[path = "managed_process_probe_control_tests.rs"]
mod tests;
