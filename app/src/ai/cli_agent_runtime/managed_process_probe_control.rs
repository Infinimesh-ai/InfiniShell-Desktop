//! 已验证 Windows Codex npm 探针在原认证连接上的协作取消。

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) const COOPERATIVE_EXIT_TIMEOUT: Duration = Duration::from_secs(8);
const STOP_WRITE_TIMEOUT: Duration = Duration::from_millis(100);

pub(super) fn request_stop(stream: &mut TcpStream) -> io::Result<()> {
    let result = stream
        .set_write_timeout(Some(STOP_WRITE_TIMEOUT))
        .and_then(|()| stream.write_all(&[2]));
    // 写入失败时也关闭发送方向，让仍可达的监听者按 EOF 取消；没有重连或新授权。
    let _ = stream.shutdown(Shutdown::Write);
    result
}

pub(super) struct CancellationListener {
    control: TcpStream,
    cancelled: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl CancellationListener {
    /// 调用方必须先核对原握手和完整 launch binding，不能接受其他来源的连接。
    pub(super) fn start(control: TcpStream) -> io::Result<Self> {
        // 握手的十秒期限不是任务时限；空闲探针必须继续等待真实停止或连接断开。
        control.set_read_timeout(None)?;
        let mut reader_control = control.try_clone()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader_cancelled = Arc::clone(&cancelled);
        let reader = thread::Builder::new()
            .name("cli-npm-cancel".to_owned())
            .spawn(move || {
                let mut request = [0_u8; 1];
                // 协议只允许一次停止。停止、EOF、读取错误或其他字节均取消，不输出内容。
                let _ = reader_control.read(&mut request);
                reader_cancelled.store(true, Ordering::Release);
            })?;
        Ok(Self {
            control,
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
        // 即使正常执行没有收到停止字节，也须唤醒阻塞读取并回收本次监听线程。
        let _ = self.control.shutdown(Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
#[path = "managed_process_probe_control_tests.rs"]
mod tests;
