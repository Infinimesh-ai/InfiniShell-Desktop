//! SSH 密码 / passphrase 自动注入。订阅 terminal pane 的 PTY 输出广播,
//! 匹配到 `password:` / `passphrase:` 行尾提示后**一次性**写入 secret + `\n`。
//!
//! ## 关键设计权衡
//!
//! - **8KB 滑窗 + 行尾严格匹配**:正则 `(?im)(password|passphrase)[^\n]*:\s*$`
//!   仅匹配行尾(避免 motd / banner 里"password" 字样误中)+ 滑窗保证内存上限。
//!
//! - **15s 超时**:典型 SSH 公钥协商 < 2s,密码 prompt < 5s。15s 是公钥认证
//!   失败 + fallback 密码的合理上限。**配公钥免登录的边界**(authorized_keys
//!   配了 + 我们也存了密码):公钥握手成功 → 不会出现 prompt → injector 静默
//!   超时退出,**不会乱注入到登录后的 shell**。
//!
//! - **一次性触发**:匹配后立即 break,injector future 退出 → InactiveReceiver
//!   drop → 后续 PTY 流不再被本注入器看见,**杜绝二次注入**。
//!
//! - **bytes::Regex**:PTY 输出可能含未完整 UTF-8 字节,用 `regex::bytes` 安全。

use std::sync::Arc;
use std::time::Duration;

use async_broadcast::InactiveReceiver;
use warpui::r#async::FutureExt;
use warpui::{ViewContext, WeakViewHandle};
use zeroize::Zeroizing;

use crate::ssh_manager::password_prompt::{
    append_password_submit_byte, bytes_look_like_password_prompt,
};
use crate::terminal::TerminalView;

/// 注入超时上限。
const INJECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 滑窗保留最近这么多字节的 PTY 输出供正则匹配。
const SLIDING_WINDOW_BYTES: usize = 8 * 1024;
/// 当 buffer 超过这个值,drain 到滑窗大小。
const BUFFER_HARD_LIMIT: usize = 16 * 1024;

/// 在 owner=Workspace 上下文 spawn 一个一次性注入任务。Workspace drop 时
/// 任务自动取消;owner 不需要 abort。
///
/// 调用前提:`pty_reads_rx` 由 `terminal_view.inactive_pty_reads_rx(ctx)`
/// 取得,**Some 时才会真正起 future**;wasm / 远端会话拿到 None 直接 no-op。
pub fn spawn_password_injector<O>(
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    terminal_view: WeakViewHandle<TerminalView>,
    secret: Zeroizing<String>,
    ctx: &mut ViewContext<O>,
) where
    O: warpui::View + 'static,
{
    let Some(rx) = pty_reads_rx else {
        log::debug!("ssh secret injector: no pty_reads_rx (non-local session) — skip");
        return;
    };
    if secret.is_empty() {
        log::debug!("ssh secret injector: empty secret — skip");
        return;
    }

    // 起飞即把 in-flight 置 true,通知 OneKey listener 在本次 injection 完结前
    // 不要弹菜单。这样无论是先 injector 注入完才轮到 onekey 看到同一段字节,
    // 还是 onekey 先看到,语义都是统一的:**injector 优先**。
    if let Some(view) = terminal_view.upgrade(ctx) {
        view.update(ctx, |view, _| {
            view.set_ssh_secret_auto_injection_in_flight(true);
        });
    }

    let owned_secret = secret.clone();
    let future = async move {
        match watch_for_prompt(rx).with_timeout(INJECT_TIMEOUT).await {
            Ok(true) => Some(owned_secret),
            Ok(false) | Err(_) => None, // EOF or timeout → no-op
        }
    };
    ctx.spawn(future, move |_owner, secret_opt, ctx| {
        let Some(view) = terminal_view.upgrade(ctx) else {
            log::debug!("ssh secret injector: terminal view dropped before injection");
            return;
        };
        let Some(secret) = secret_opt else {
            log::debug!("ssh secret injector: no prompt seen within timeout");
            view.update(ctx, |view, _| {
                view.set_ssh_secret_auto_injection_in_flight(false);
            });
            return;
        };
        view.update(ctx, |view, ctx| {
            // 把密码 + Enter 作为字节写入 PTY,等同模拟键盘回应交互式 prompt。
            // 此时 ssh 已经在跑(bootstrap 早完成),write_to_pty 直写是正解。
            let mut bytes = secret.as_bytes().to_vec();
            append_password_submit_byte(&mut bytes);
            view.write_to_pty(bytes, ctx);
            view.note_ssh_secret_auto_injected(ctx);
            view.set_ssh_secret_auto_injection_in_flight(false);
        });
    });
}

/// 异步循环:消费 PTY 广播,滑窗追加,**正则一旦命中行尾 prompt 就返回 true**;
/// EOF 返回 false。timeout 由调用方 `with_timeout` 包装。
async fn watch_for_prompt(rx: InactiveReceiver<Arc<Vec<u8>>>) -> bool {
    let mut active = rx.activate_cloned();
    let mut buf: Vec<u8> = Vec::with_capacity(SLIDING_WINDOW_BYTES);
    while let Ok(chunk) = active.recv().await {
        buf.extend_from_slice(&chunk);
        if buf.len() > BUFFER_HARD_LIMIT {
            let drop_n = buf.len() - SLIDING_WINDOW_BYTES;
            buf.drain(..drop_n);
        }
        if bytes_look_like_password_prompt(&buf) {
            return true;
        }
    }
    false
}
