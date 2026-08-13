//! Zap M4:项目主机会话路由器 —— `run_command_on_hosts` 的执行核心。
//!
//! 单例 model([`ProjectHostSessionRouter`]),职责:
//! 1. 维护 node_id → 终端视图的注册表(`WorkspaceView::open_ssh_terminal`
//!    打开 SSH tab 时注册,访问时惰性清理死引用);
//! 2. 串行驱动一次批量执行:逐台校验 node → 复用/新开 SSH 会话 → 等待就绪
//!    → 执行命令 → 等待 block 完成 → 聚合 JSON 结果回执行器;
//! 3. 金丝雀语义:canary=true 时第一台失败即中止其余主机。
//!
//! ## 等待机制(MVP:定时轮询)
//!
//! 会话就绪与命令完成都用 500ms 间隔的 `Timer` 轮询(就绪上限 60s,完成上
//! 限为请求的 timeout_seconds)。没有采用事件订阅:bootstrap 完成信号散落在
//! `TerminalView::handle_session_bootstrapped` 内部,block 完成信号在各终端
//! 各自的 `ModelEventDispatcher` 上,跨 pane 订阅需要给每个目标终端建一套
//! executor 级别的 channel 管线,MVP 阶段轮询已足够(检查本身只读且廉价)。
//!
//! ## 终端锁纪律
//!
//! 路由器自身从不直接碰 `TerminalModel::lock()`;所有需要锁的检查都收敛在
//! `TerminalView` 的专用方法里(`has_active_long_running_block` /
//! `project_agent_block_finished` / `project_agent_block_result`),每个方法
//! 单次上锁、作用域即表达式,不跨 await、不与其他 model 锁嵌套。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use serde_json::{Value, json};
use warp_ssh_manager::{SshRepository, resolve_machine_key};
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity, ViewHandle, WeakViewHandle};
use zap_projects::ProjectRepository;

use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_providers::tools::project_hosts::BatchArgs;
use crate::terminal::view::TerminalView;

/// 单主机输出注入上限(Unicode 字符),超出截断并追加中文标记。
pub const OUTPUT_MAX_CHARS: usize = 10_000;
/// 输出截断标记。
const OUTPUT_TRUNCATION_MARKER: &str = "…(输出超限已截断)";
/// 就绪 / 完成轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 新开会话的就绪等待上限。
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// 单主机执行结果状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchHostStatus {
    Ok,
    Error,
    Timeout,
    Busy,
    SessionNotReady,
    CanaryAborted,
}

impl BatchHostStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BatchHostStatus::Ok => "ok",
            BatchHostStatus::Error => "error",
            BatchHostStatus::Timeout => "timeout",
            BatchHostStatus::Busy => "busy",
            BatchHostStatus::SessionNotReady => "session_not_ready",
            BatchHostStatus::CanaryAborted => "canary_aborted",
        }
    }
}

/// 单主机执行结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchHostResult {
    pub node_id: String,
    /// `host:port` 展示串;node 解析失败时为空。
    pub host: String,
    pub status: BatchHostStatus,
    pub exit_code: Option<i32>,
    pub output: String,
    pub duration_ms: u64,
}

impl BatchHostResult {
    /// 未执行即失败(unknown node / 项目归属校验不过 / 会话丢失等)。
    fn failed(node_id: String, host: String, message: String) -> Self {
        BatchHostResult {
            node_id,
            host,
            status: BatchHostStatus::Error,
            exit_code: None,
            output: message,
            duration_ms: 0,
        }
    }

    /// 金丝雀中止后剩余主机的占位结果。
    fn skipped_by_canary(node_id: String) -> Self {
        BatchHostResult {
            node_id,
            host: String::new(),
            status: BatchHostStatus::CanaryAborted,
            exit_code: None,
            output: String::new(),
            duration_ms: 0,
        }
    }
}

/// 按 Unicode 字符截断输出,超限时追加截断标记。
pub(crate) fn truncate_output(output: &str) -> String {
    if output.chars().count() <= OUTPUT_MAX_CHARS {
        return output.to_owned();
    }
    let mut truncated: String = output.chars().take(OUTPUT_MAX_CHARS).collect();
    truncated.push_str(OUTPUT_TRUNCATION_MARKER);
    truncated
}

/// 金丝雀失败判定:非 ok 状态或退出码非 0。
pub(crate) fn is_canary_failure(result: &BatchHostResult) -> bool {
    result.status != BatchHostStatus::Ok || result.exit_code != Some(0)
}

pub(crate) fn host_result_to_json(result: &BatchHostResult) -> Value {
    json!({
        "node_id": result.node_id,
        "host": result.host,
        "status": result.status.as_str(),
        "exit_code": result.exit_code,
        "output": result.output,
        "duration_ms": result.duration_ms,
    })
}

/// 聚合批量结果:全部 ok 才算整体 ok。
pub(crate) fn aggregate_results(results: &[BatchHostResult], canary_aborted: bool) -> Value {
    let all_ok = results
        .iter()
        .all(|result| result.status == BatchHostStatus::Ok);
    let status = if all_ok { "ok" } else { "error" };
    json!({
        "status": status,
        "canary_aborted": canary_aborted,
        "results": results.iter().map(host_result_to_json).collect::<Vec<Value>>(),
    })
}

/// 已解析的当前目标主机。
#[derive(Clone, Debug)]
struct CurrentHost {
    node_id: String,
    /// `host:port` 展示串。
    host_display: String,
    /// 归一化 machine key(与会话实际连接目标比对,防止 tab 被复用去连了别的主机)。
    machine_key: String,
}

/// 当前主机所处的等待阶段(轮询状态机)。
#[derive(Clone, Debug)]
enum HostPhase {
    /// 等待(新开的)会话连接就绪。
    WaitingReady {
        host: CurrentHost,
        deadline: Instant,
    },
    /// 命令已派发,等待 block 完成。
    WaitingFinish {
        host: CurrentHost,
        action_id: AIAgentActionId,
        started: Instant,
        deadline: Instant,
    },
}

/// 一次批量执行的全部状态。
struct ActiveBatch {
    args: BatchArgs,
    conversation_id: AIConversationId,
    done_tx: oneshot::Sender<Value>,
    results: Vec<BatchHostResult>,
    /// 下一个要处理的 node_ids 下标。
    index: usize,
    canary_aborted: bool,
    phase: Option<HostPhase>,
}

#[derive(Clone, Debug)]
pub enum ProjectHostRouterEvent {
    /// 请求 WorkspaceView 为该 node 打开一个 SSH 会话 tab
    /// (订阅方需先 `claim_pending_open` 去重,多窗口场景只允许一个赢家)。
    OpenHostSession { node_id: String },
}

#[derive(Default)]
pub struct ProjectHostSessionRouter {
    /// node_id → 终端视图弱引用;访问时惰性清理死引用。
    sessions: HashMap<String, WeakViewHandle<TerminalView>>,
    /// 已发出 OpenHostSession、等待某个 WorkspaceView 认领的 node。
    pending_open: HashSet<String>,
    /// 当前批次(同一时刻最多一个;并发调用直接报错返回)。
    batch: Option<ActiveBatch>,
}

impl ProjectHostSessionRouter {
    pub fn new() -> Self {
        Default::default()
    }

    /// 注册 node_id → 终端视图(`open_ssh_terminal` 打开 SSH tab 时调用)。
    pub fn register_session(&mut self, node_id: String, view: WeakViewHandle<TerminalView>) {
        self.sessions.insert(node_id, view);
    }

    /// WorkspaceView 处理 OpenHostSession 前认领;返回 `false` 表示已被
    /// 其他窗口认领(或事件已过期),调用方应忽略。
    pub fn claim_pending_open(&mut self, node_id: &str) -> bool {
        self.pending_open.remove(node_id)
    }

    /// 启动一次批量执行。结果(聚合 JSON)经 `done_tx` 送回执行器的
    /// async future;若已有批次在跑则立即回错误。
    pub fn start_batch(
        &mut self,
        args: BatchArgs,
        conversation_id: AIConversationId,
        done_tx: oneshot::Sender<Value>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.batch.is_some() {
            let _ = done_tx.send(json!({
                "status": "error",
                "message": "另一个 run_command_on_hosts 批次正在执行,请等它结束后重试",
            }));
            return;
        }
        self.batch = Some(ActiveBatch {
            args,
            conversation_id,
            done_tx,
            results: Vec::new(),
            index: 0,
            canary_aborted: false,
            phase: None,
        });
        self.step(ctx);
    }

    /// 推进状态机:处理下一台主机或收尾。
    fn step(&mut self, ctx: &mut ModelContext<Self>) {
        let (index, total, canary_aborted, canceled) = {
            let Some(batch) = self.batch.as_ref() else {
                return;
            };
            (
                batch.index,
                batch.args.node_ids.len(),
                batch.canary_aborted,
                batch.done_tx.is_canceled(),
            )
        };
        // 执行器侧的 future 已被取消(action 取消/会话销毁),整批终止。
        if canceled {
            log::info!("[project-hosts] batch receiver canceled; aborting batch");
            self.batch = None;
            return;
        }
        if canary_aborted {
            if let Some(batch) = self.batch.as_mut() {
                while batch.index < batch.args.node_ids.len() {
                    let node_id = batch.args.node_ids[batch.index].clone();
                    batch
                        .results
                        .push(BatchHostResult::skipped_by_canary(node_id));
                    batch.index += 1;
                }
            }
            self.finish();
            return;
        }
        if index >= total {
            self.finish();
            return;
        }

        let node_id = {
            let Some(batch) = self.batch.as_ref() else {
                return;
            };
            batch.args.node_ids[index].clone()
        };
        match resolve_target(&node_id) {
            Ok(target) => {
                if let Some(view) = self.ready_view(&target.node_id, &target.machine_key, ctx) {
                    self.begin_execute(target, view, ctx);
                } else {
                    // 没有可复用的就绪会话:请求 WorkspaceView 开一个,然后轮询等就绪。
                    if let Some(batch) = self.batch.as_mut() {
                        batch.phase = Some(HostPhase::WaitingReady {
                            host: target,
                            deadline: Instant::now() + READY_TIMEOUT,
                        });
                    }
                    self.pending_open.insert(node_id.clone());
                    ctx.emit(ProjectHostRouterEvent::OpenHostSession { node_id });
                    self.schedule_tick(ctx);
                }
            }
            Err(message) => {
                self.push_result_and_advance(
                    BatchHostResult::failed(node_id, String::new(), message),
                    ctx,
                );
            }
        }
    }

    /// 就绪且连接目标与 node 匹配的已注册视图;死引用惰性清理。
    fn ready_view(
        &mut self,
        node_id: &str,
        machine_key: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Option<ViewHandle<TerminalView>> {
        let weak = self.sessions.get(node_id).cloned()?;
        let Some(view) = weak.upgrade(ctx) else {
            self.sessions.remove(node_id);
            return None;
        };
        let endpoint = view.read(ctx, |view, app| view.project_host_ssh_endpoint(app))?;
        let (host, port) = endpoint;
        let session_key = resolve_machine_key(host.as_deref(), port.as_deref())?;
        (session_key == machine_key).then_some(view)
    }

    /// 在就绪视图上派发命令并进入完成等待阶段。
    fn begin_execute(
        &mut self,
        host: CurrentHost,
        view: ViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) {
        let busy = view.read(ctx, |view, _app| view.has_active_long_running_block());
        if busy {
            self.push_result_and_advance(
                BatchHostResult {
                    node_id: host.node_id,
                    host: host.host_display,
                    status: BatchHostStatus::Busy,
                    exit_code: None,
                    output: "目标会话正有长时间运行的命令,跳过执行".to_owned(),
                    duration_ms: 0,
                },
                ctx,
            );
            return;
        }

        let (command, conversation_id, timeout) = {
            let Some(batch) = self.batch.as_ref() else {
                return;
            };
            (
                batch.args.command.clone(),
                batch.conversation_id,
                Duration::from_secs(batch.args.timeout_seconds),
            )
        };
        // 每台主机合成独立 action_id,blocklist 用它回查执行 block。
        let action_id = AIAgentActionId::from(uuid::Uuid::new_v4().to_string());
        let executed = view.update(ctx, |view, ctx| {
            view.execute_project_agent_command(&command, action_id.clone(), conversation_id, ctx)
        });
        if !executed {
            self.push_result_and_advance(
                BatchHostResult::failed(
                    host.node_id,
                    host.host_display,
                    "目标会话没有可执行命令的 session".to_owned(),
                ),
                ctx,
            );
            return;
        }
        let now = Instant::now();
        if let Some(batch) = self.batch.as_mut() {
            batch.phase = Some(HostPhase::WaitingFinish {
                host,
                action_id,
                started: now,
                deadline: now + timeout,
            });
        }
        self.schedule_tick(ctx);
    }

    /// 500ms 轮询一拍。
    fn schedule_tick(&mut self, ctx: &mut ModelContext<Self>) {
        // spawn 的 future 默认脱管运行(丢弃返回的 handle 不会 abort)。
        ctx.spawn(Timer::after(POLL_INTERVAL), |me, _timer, ctx| {
            me.poll_tick(ctx)
        });
    }

    fn poll_tick(&mut self, ctx: &mut ModelContext<Self>) {
        let (phase, canceled) = {
            let Some(batch) = self.batch.as_ref() else {
                return;
            };
            (batch.phase.clone(), batch.done_tx.is_canceled())
        };
        if canceled {
            log::info!("[project-hosts] batch receiver canceled; aborting batch");
            self.batch = None;
            return;
        }
        let Some(phase) = phase else {
            return;
        };
        match phase {
            HostPhase::WaitingReady { host, deadline } => {
                if let Some(view) = self.ready_view(&host.node_id, &host.machine_key, ctx) {
                    self.begin_execute(host, view, ctx);
                } else if Instant::now() >= deadline {
                    self.push_result_and_advance(
                        BatchHostResult {
                            node_id: host.node_id,
                            host: host.host_display,
                            status: BatchHostStatus::SessionNotReady,
                            exit_code: None,
                            output: format!("会话在 {} 秒内未连接就绪", READY_TIMEOUT.as_secs()),
                            duration_ms: 0,
                        },
                        ctx,
                    );
                } else {
                    self.schedule_tick(ctx);
                }
            }
            HostPhase::WaitingFinish {
                host,
                action_id,
                started,
                deadline,
            } => {
                let view = self
                    .sessions
                    .get(&host.node_id)
                    .cloned()
                    .and_then(|weak| weak.upgrade(ctx));
                let Some(view) = view else {
                    self.push_result_and_advance(
                        BatchHostResult::failed(
                            host.node_id,
                            host.host_display,
                            "执行期间目标会话被关闭".to_owned(),
                        ),
                        ctx,
                    );
                    return;
                };
                let finished = view.read(ctx, |view, _app| {
                    view.project_agent_block_finished(&action_id)
                });
                match finished {
                    Some(true) => {
                        let block_result = view.read(ctx, |view, _app| {
                            view.project_agent_block_result(&action_id)
                        });
                        let (exit_code, output) = block_result.unwrap_or((None, String::new()));
                        let status = if exit_code == Some(0) {
                            BatchHostStatus::Ok
                        } else {
                            BatchHostStatus::Error
                        };
                        self.push_result_and_advance(
                            BatchHostResult {
                                node_id: host.node_id,
                                host: host.host_display,
                                status,
                                exit_code,
                                output: truncate_output(&output),
                                duration_ms: elapsed_ms(started),
                            },
                            ctx,
                        );
                    }
                    Some(false) | None => {
                        if Instant::now() >= deadline {
                            // 超时:带上当前输出快照(block 未出现时为空)。
                            let snapshot = view
                                .read(ctx, |view, _app| {
                                    view.project_agent_block_result(&action_id)
                                })
                                .map(|(_exit_code, output)| output)
                                .unwrap_or_default();
                            self.push_result_and_advance(
                                BatchHostResult {
                                    node_id: host.node_id,
                                    host: host.host_display,
                                    status: BatchHostStatus::Timeout,
                                    exit_code: None,
                                    output: truncate_output(&snapshot),
                                    duration_ms: elapsed_ms(started),
                                },
                                ctx,
                            );
                        } else {
                            self.schedule_tick(ctx);
                        }
                    }
                }
            }
        }
    }

    /// 记录一台主机的结果、评估金丝雀并推进到下一台。
    fn push_result_and_advance(&mut self, result: BatchHostResult, ctx: &mut ModelContext<Self>) {
        {
            let Some(batch) = self.batch.as_mut() else {
                return;
            };
            if batch.args.canary && batch.index == 0 && is_canary_failure(&result) {
                batch.canary_aborted = true;
            }
            batch.results.push(result);
            batch.index += 1;
            batch.phase = None;
        }
        self.step(ctx);
    }

    /// 收尾:聚合 JSON 结果送回执行器 future。
    fn finish(&mut self) {
        let Some(batch) = self.batch.take() else {
            return;
        };
        let value = aggregate_results(&batch.results, batch.canary_aborted);
        if batch.done_tx.send(value).is_err() {
            log::warn!("[project-hosts] batch receiver dropped before result delivery");
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 校验 node 并解析目标主机:node 必须存在于 ssh_servers,且归属至少一个项目。
fn resolve_target(node_id: &str) -> Result<CurrentHost, String> {
    let server = warp_ssh_manager::with_conn(|conn| Ok(SshRepository::get_server(conn, node_id)?));
    let server = match server {
        Ok(Some(server)) => server,
        Ok(None) => return Err("unknown node_id".to_owned()),
        Err(err) => return Err(format!("SSH 节点查询失败: {err}")),
    };
    // 项目归属校验:不属于任何项目的 node 拒绝执行(工具仅面向项目会话)。
    let in_project =
        zap_projects::with_conn(|conn| Ok(ProjectRepository::projects_for_node(conn, node_id)?))
            .map(|projects: Vec<String>| !projects.is_empty());
    match in_project {
        Ok(true) => {}
        Ok(false) => return Err("node 不属于任何项目,拒绝执行".to_owned()),
        Err(err) => return Err(format!("项目归属查询失败: {err}")),
    }
    let port_str = server.port.to_string();
    let Some(machine_key) = resolve_machine_key(Some(&server.host), Some(&port_str)) else {
        return Err("无法解析主机地址".to_owned());
    };
    Ok(CurrentHost {
        node_id: node_id.to_owned(),
        host_display: format!("{}:{}", server.host, server.port),
        machine_key,
    })
}

impl Entity for ProjectHostSessionRouter {
    type Event = ProjectHostRouterEvent;
}

impl SingletonEntity for ProjectHostSessionRouter {}

#[cfg(test)]
#[path = "session_router_tests.rs"]
mod tests;
