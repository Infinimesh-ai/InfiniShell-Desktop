//! 固定 Grok 生产入口的多技能、空闲新增与冷恢复；原生正文另由运行器逐字节审核。

use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ai::skills::ParsedSkill;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;
use warp_cli::agent::Harness;

use super::{connect, encode_prompt_content};
use crate::ai::agent::ImageContext;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::managed_input::{prepare_managed_input, restore_managed_images};
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeController, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, managed_process,
};

const PROMPT: &str = "请按本轮所选技能的顺序执行。只读取本轮选中技能的 SKILL.md；没有在本回合展开的技能必须重新 read_file，不能使用历史记忆。最终每个技能只输出其独立标记，每行一个，不解释。不执行命令、不修改文件、不访问网络。";
const IMAGE_PROMPT: &str = "请按本轮所选技能的顺序执行。只读取本轮选中技能的 SKILL.md；没有在本回合展开的技能必须重新 read_file，不能使用历史记忆。同时只观察本轮附图的左右纯色，不参考历史图片。最终先按技能顺序输出各自独立标记，每行一个，最后一行输出左右两个大写英文色名，以一个空格分隔。不解释，不执行命令、不修改文件、不访问网络。";

#[derive(Clone, Copy)]
enum Scenario {
    MultiSkill,
    ImageSkill,
    UserSkill,
}

impl Scenario {
    fn prompt(self) -> &'static str {
        match self {
            Self::MultiSkill | Self::UserSkill => PROMPT,
            Self::ImageSkill => IMAGE_PROMPT,
        }
    }

    fn marker_prefix(self) -> &'static str {
        match self {
            Self::MultiSkill | Self::UserSkill => "G06_",
            Self::ImageSkill => "G02_",
        }
    }

    fn skill_root(self, root: &Path, key: &str) -> PathBuf {
        match self {
            Self::UserSkill if key != "alpha" => root.join("home/.grok/skills"),
            Self::MultiSkill | Self::ImageSkill | Self::UserSkill => {
                root.join("project/.grok/skills")
            }
        }
    }
}

/// 仅为显式隔离场景保存已通过生产守卫的原生目录，不输出正文、工具定义或凭据。
pub(super) fn trace_user_skill_refresh(
    generation: Uuid,
    native: &str,
    phase: &str,
    result: &Value,
) {
    if env::var("INFINISHELL_GROK_USER_SKILL_TRACE").as_deref() != Ok("1") {
        return;
    }
    let Some(root) = env::var_os("INFINISHELL_GROK_LIVE_ROOT") else {
        return;
    };
    if fs::read_to_string(PathBuf::from(root).join(".infinishell-grok-user-skill-probe"))
        .ok()
        .as_deref()
        != Some("isolated Grok user skill verification\n")
    {
        return;
    }
    let proof = match phase {
        "reload" => json!({"reloaded":result["result"]["reloaded"]}),
        "ready_catalog" | "refreshed_catalog" => {
            let commands = result["commands"].as_array().map(|commands| {
                commands
                    .iter()
                    .filter(|command| {
                        matches!(
                            command["_meta"]["bareName"].as_str(),
                            Some("isp-g06-alpha" | "isp-g06-beta" | "isp-g06-gamma")
                        )
                    })
                    .map(|command| {
                        json!({"name":command["name"],"input":command["input"],
                    "_meta":{"bareName":command["_meta"]["bareName"],
                        "scope":command["_meta"]["scope"],
                        "qualifiedName":command["_meta"]["qualifiedName"],
                        "path":command["_meta"]["path"]}})
                    })
                    .collect::<Vec<_>>()
            });
            json!({"commands":commands})
        }
        _ => return,
    };
    eprintln!(
        "GROK_USER_SKILL_CATALOG {}",
        json!({"generation":generation,
        "native_session_id":native,"phase":phase,"proof":proof})
    );
}

struct AbortOnDrop(JoinHandle<Result<(), RuntimeError>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "技能证据写入失败".into())
}

fn selection(
    root: &Path,
    keys: &[&str],
    scenario: Scenario,
) -> Result<Vec<SelectedLocalSkill>, String> {
    keys.iter()
        .map(|key| {
            let name = format!("isp-g06-{key}");
            let path = scenario.skill_root(root, key).join(&name).join("SKILL.md");
            Ok(SelectedLocalSkill {
                name,
                path: dunce::canonicalize(path).map_err(|_| "选中技能路径不可解析")?,
            })
        })
        .collect()
}

fn expected(root: &Path, keys: &[&str], scenario: Scenario) -> Result<String, String> {
    selection(root, keys, scenario)?
        .iter()
        .map(|skill| {
            let bytes = fs::read_to_string(&skill.path).map_err(|_| "技能夹具不可读")?;
            let markers: Vec<_> = bytes
                .lines()
                .filter(|line| line.starts_with(scenario.marker_prefix()))
                .collect();
            if markers.len() != 1 || scenario.prompt().contains(markers[0]) {
                return Err("技能隐藏标记数量异常或泄漏到输入".into());
            }
            Ok(markers[0].to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|markers| markers.join("\n"))
}

fn image_input(
    root: &Path,
    case: &str,
    skills: Vec<ParsedSkill>,
    file: &mut File,
) -> Result<Vec<InputContent>, String> {
    let colors = match case {
        "initial" => [[255, 0, 0], [0, 0, 255]],
        "hot" => [[0, 255, 0], [255, 255, 0]],
        "cold" => [[0, 0, 255], [255, 0, 0]],
        _ => return Err("未知图片技能验收回合".into()),
    };
    let mut picture = RgbImage::new(256, 128);
    for (x, _, pixel) in picture.enumerate_pixels_mut() {
        *pixel = Rgb(colors[usize::from(x >= 128)]);
    }
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(picture)
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|_| "PNG 夹具生成失败")?;
    let bytes = encoded.into_inner();
    let store = root.join("state/local-cli-attachments");
    let image = ImageContext {
        data: STANDARD.encode(&bytes),
        mime_type: "image/png".into(),
        file_name: "本轮图片.png".into(),
        is_figma: false,
    };
    let input = prepare_managed_input(
        Harness::Grok,
        IMAGE_PROMPT.into(),
        &[image],
        skills.clone(),
        &store,
    )?;
    let paths = input
        .iter()
        .filter_map(|part| match part {
            InputContent::LocalImage(path) => Some(path.clone()),
            InputContent::Text(_) | InputContent::Skill { .. } => None,
        })
        .collect::<Vec<_>>();
    if paths.len() != 1 {
        return Err("图片技能输入缺少唯一持久图片引用".into());
    }
    let path = paths[0].clone();
    let checkpoint = root.join(format!("{case}-input.json"));
    fs::write(
        &checkpoint,
        serde_json::to_vec(&input).map_err(|_| "图片技能引用编码失败")?,
    )
    .map_err(|_| "图片技能引用保存失败")?;
    let restored: Vec<InputContent> =
        serde_json::from_slice(&fs::read(checkpoint).map_err(|_| "图片技能引用读取失败")?)
            .map_err(|_| "图片技能引用恢复失败")?;
    let images = restore_managed_images(paths, &store)?;
    let rebuilt =
        prepare_managed_input(Harness::Grok, IMAGE_PROMPT.into(), &images, skills, &store)?;
    if restored != input || rebuilt != input || images[0].data != STANDARD.encode(&bytes) {
        return Err("图片与技能持久引用恢复后改变".into());
    }
    // 技能由生产目录校验后编码；这里只提前核对相同图片编码器，不合成原生技能正文。
    let native = encode_prompt_content(vec![InputContent::LocalImage(path.clone())], &store)?;
    if native.len() != 1
        || native[0]["data"] != STANDARD.encode(&bytes)
        || native[0]["mimeType"] != "image/png"
    {
        return Err("生产 ACP 编码未保留图片字节".into());
    }
    let native_bytes = serde_json::to_vec(&native[0]).map_err(|_| "ACP 图片编码失败")?;
    record(
        file,
        json!({"event":"attachment_prepared","case":case,"mime_type":"image/png",
        "byte_count":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes)),
        "native_content_digest":format!("{:x}",Sha256::digest(&native_bytes)),
        "source_path":path,"persistent_reference_restored":true}),
    )?;
    Ok(restored)
}

fn exact_skill_read_permission(call: &Value, cwd: &Path, skill: &Path) -> bool {
    let Some(input) = call["rawInput"].as_object() else {
        return false;
    };
    let Some(target) = input.get("target_file").and_then(Value::as_str) else {
        return false;
    };
    let path = Path::new(target);
    call["kind"] == "read"
        && call["_meta"]["x.ai/tool"]["name"] == "read_file"
        && input
            .keys()
            .all(|key| key == "target_file" || key == "variant")
        && input.get("variant").is_none_or(|value| value == "ReadFile")
        && path.is_absolute()
        && path.starts_with(cwd)
        && dunce::canonicalize(path).ok().as_deref() == Some(skill)
}

async fn exercise_turn(
    root: &Path,
    case: &str,
    keys: &[&str],
    scenario: Scenario,
    generation: Uuid,
    native: &str,
    controller: &RuntimeController,
    events: &mut mpsc::Receiver<RuntimeEvent>,
    file: &mut File,
) -> Result<(), String> {
    let skills = selection(root, keys, scenario)?;
    let parsed = skills
        .iter()
        .map(|skill| ai::skills::parse_skill(&skill.path).map_err(|_| "技能解析失败".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let input = match scenario {
        Scenario::MultiSkill | Scenario::UserSkill => prepare_managed_input(
            Harness::Grok,
            PROMPT.into(),
            &[],
            parsed,
            &root.join("attachments"),
        )?,
        Scenario::ImageSkill => image_input(root, case, parsed, file)?,
    };
    let message = Uuid::new_v4();
    let command = RuntimeCommand {
        generation,
        message_id: message,
        action: RuntimeAction::Submit { input },
    };
    let mut answer = expected(root, keys, scenario)?;
    if let Scenario::ImageSkill = scenario {
        let colors = match case {
            "initial" => "RED BLUE",
            "hot" => "GREEN YELLOW",
            "cold" => "BLUE RED",
            _ => return Err("未知图片技能验收回合".into()),
        };
        answer.push('\n');
        answer.push_str(colors);
    }
    let serialized = serde_json::to_string(&command).map_err(|_| "技能输入序列化失败")?;
    if answer.lines().any(|marker| serialized.contains(marker)) {
        return Err("技能秘密正文进入提交参数".into());
    }
    let mut stale = command.clone();
    stale.generation = Uuid::new_v4();
    if !matches!(
        controller.send(stale).await,
        Err(RuntimeError::StaleGeneration)
    ) {
        return Err("旧代次输入未被控制器拒绝".into());
    }
    record(
        file,
        json!({"event":"submitted","case":case,"generation":generation,
        "native_session_id":native,"message_id":message,"input":command.action,
        "selected_skills":skills,"stale_generation_rejected":true,"markers_absent_from_input":true}),
    )?;
    controller
        .send(command.clone())
        .await
        .map_err(|_| "技能提交失败")?;
    controller
        .send(command.clone())
        .await
        .map_err(|_| "运行中消息重投失败")?;
    let mut turn = None;
    let mut started = false;
    let mut controls = HashSet::new();
    let mut approvals = HashSet::new();
    let mut approval_count = 0;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(150), events.recv())
            .await
            .map_err(|_| "技能回合事件超时")?
            .ok_or("技能回合事件流关闭")?;
        if event.generation != generation || event.native_session_id.as_deref() != Some(native) {
            return Err("技能事件关联错误会话或旧代次".into());
        }
        match event.kind {
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                if message_id != message || turn.is_some() || turn_id.is_none() {
                    return Err("技能 ACK 缺失身份、重复或属于其他消息".into());
                }
                turn = turn_id;
                record(
                    file,
                    json!({"event":"accepted","case":case,"generation":generation,
                    "native_session_id":native,"message_id":message,"turn_id":turn}),
                )?;
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                if started || turn.as_ref() != Some(&turn_id) {
                    return Err("技能回合重复或身份错误".into());
                }
                started = true;
            }
            RuntimeEventKind::TextDelta { turn_id, .. }
            | RuntimeEventKind::Progress { turn_id, .. } => {
                if turn.as_ref() != Some(&turn_id) {
                    return Err("技能输出越过回合边界".into());
                }
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                approval_count += 1;
                let permission_root = match scenario {
                    Scenario::UserSkill => root.to_path_buf(),
                    Scenario::MultiSkill | Scenario::ImageSkill => root.join("project"),
                };
                let matched = skills.iter().find(|skill| {
                    exact_skill_read_permission(&details["toolCall"], &permission_root, &skill.path)
                });
                let allowed = approval_count <= 8
                    && turn.as_ref() == Some(&turn_id)
                    && method == "session/request_permission"
                    && details["sessionId"] == native
                    && matched.is_some()
                    && !approvals.contains(&approval_id);
                let decision = if allowed {
                    ApprovalDecision::AllowOnce
                } else {
                    ApprovalDecision::DenyOnce
                };
                record(
                    file,
                    json!({"event":"approval","case":case,"generation":generation,
                    "native_session_id":native,"turn_id":turn_id,"approval_id":approval_id,
                    "tool_call_id":details["toolCall"]["toolCallId"],"allowed_once":allowed,
                    "matched_skill":matched.map(|skill| &skill.name),"method":method}),
                )?;
                let control = Uuid::new_v4();
                controls.insert(control);
                controller
                    .send(RuntimeCommand {
                        generation,
                        message_id: control,
                        action: RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision,
                        },
                    })
                    .await
                    .map_err(|_| "技能审批响应发送失败")?;
                if !allowed {
                    return Err("额外或未精确绑定的工具已拒绝".into());
                }
                approvals.insert(approval_id);
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                if decision != ApprovalDecision::AllowOnce || !approvals.remove(&approval_id) {
                    return Err("审批回执未匹配本轮一次性授权".into());
                }
            }
            RuntimeEventKind::CommandDispatched { message_id, .. } => {
                if !controls.remove(&message_id) {
                    return Err("收到未知控制回执".into());
                }
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                record(
                    file,
                    json!({"event":"finished","case":case,"generation":generation,
                    "native_session_id":native,"message_id":message,"turn_id":turn_id,
                    "outcome":outcome,"output":output,"expected":answer,"approval_count":approval_count}),
                )?;
                if !started
                    || turn.as_ref() != Some(&turn_id)
                    || outcome != TurnOutcome::Completed
                    // 生产输出包含工具执行前的说明；原生最终答案由收据脚本单独核验。
                    || !output.trim().ends_with(&answer)
                    || !approvals.is_empty()
                    || !controls.is_empty()
                {
                    return Err("技能终态、标记、回合关联或审批闭合不匹配".into());
                }
                break;
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                controller
                    .send(RuntimeCommand {
                        generation,
                        message_id: Uuid::new_v4(),
                        action: RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id,
                            call_id: request.call_id,
                            result: Err("技能验收不授权应用本地工具".into()),
                        },
                    })
                    .await
                    .map_err(|_| "额外本地工具拒绝失败")?;
                return Err("意外应用本地工具请求已拒绝".into());
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } => {
                record(
                    file,
                    json!({"event":"request_failed","case":case,"message_id":message_id,"message":message}),
                )?;
                return Err("原生技能请求失败，已保留原始错误".into());
            }
            RuntimeEventKind::SessionReady { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err("技能回合出现未预期事件".into());
            }
        }
    }
    controller
        .send(command)
        .await
        .map_err(|_| "完成后重复消息发送失败")?;
    // 空闲观察只用于发现重复执行；最终仍用原生持久历史核对恰好三次模型输入。
    match tokio::time::timeout(Duration::from_millis(750), events.recv()).await {
        Err(_) => {}
        Ok(Some(_)) => return Err("完成后相同消息产生额外事件".into()),
        Ok(None) => return Err("完成后连接意外关闭".into()),
    }
    record(
        file,
        json!({"event":"replay_quiet","case":case,"message_id":message}),
    )?;
    Ok(())
}

async fn exercise_connection(
    root: &Path,
    previous: Option<String>,
    scenario: Scenario,
    file: &mut File,
) -> Result<String, String> {
    let generation = Uuid::new_v4();
    let resumed = previous.is_some();
    let selected = match (scenario, resumed) {
        (Scenario::MultiSkill | Scenario::UserSkill, true) => {
            selection(root, &["alpha", "beta", "gamma"], scenario)
        }
        (Scenario::MultiSkill | Scenario::UserSkill, false) | (Scenario::ImageSkill, true) => {
            selection(root, &["alpha", "beta"], scenario)
        }
        (Scenario::ImageSkill, false) => selection(root, &["alpha"], scenario),
    }?;
    let state_dir = root.join("state");
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少固定 CLI 路径")?,
        ),
        cwd: dunce::canonicalize(root.join("project")).map_err(|_| "隔离项目路径不可解析")?,
        state_dir: state_dir.clone(),
        target: previous
            .clone()
            .map_or(SessionTarget::New, |native_session_id| {
                SessionTarget::Resume { native_session_id }
            }),
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: selected.clone(),
    };
    record(
        file,
        json!({"event":"connecting","generation":generation,"resumed":resumed,
        "requested_native_session_id":previous,"selected_skills":selected,"policy":"inherit",
        "parent_ceiling":null,"profile_override":false,"model_override":false,"local_tools":false}),
    )?;
    let connection = connect(options).map_err(|_| "生产 Grok 连接拒绝此配置")?;
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = AbortOnDrop(tokio::spawn(connection.task));
    let result = async {
        let ready = tokio::time::timeout(Duration::from_secs(90), events.recv()).await
            .map_err(|_| "生产握手超时")?.ok_or("生产握手事件流关闭")?;
        let native = ready.native_session_id.ok_or("握手缺少原生会话")?;
        let RuntimeEventKind::SessionReady { verified_cli_version, effective_permissions } = ready.kind else {
            return Err("首个生产事件不是初始化成功".to_owned());
        };
        let model = &effective_permissions["reportedMetadata"]["models"]["currentModelId"];
        if ready.generation != generation || verified_cli_version.as_deref() != Some("1.0.41")
            || effective_permissions["requestedPolicy"] != "inherit" || model != "grok-4.7"
            || previous.as_ref().is_some_and(|id| id != &native) {
            return Err("固定版本、实际模型、权限或恢复会话不匹配".into());
        }
        record(file, json!({"event":"ready","generation":generation,"resumed":resumed,
            "native_session_id":native,"version":verified_cli_version,"model":model,"policy":"inherit"}))?;
        if resumed {
            // 此处明确由验收调用方提供已接受技能；不冒充协调器已经持久恢复。
            match tokio::time::timeout(Duration::from_millis(750), events.recv()).await {
                Err(_) => {},
                Ok(Some(_)) => return Err("恢复后未提交新消息却出现执行事件".into()),
                Ok(None) => return Err("冷恢复空闲连接关闭".into()),
            }
            record(file, json!({"event":"resume_idle_without_replay","generation":generation,"native_session_id":native}))?;
            let keys = match scenario {
                Scenario::MultiSkill => &["gamma", "beta"],
                Scenario::ImageSkill => &["beta", "alpha"],
                Scenario::UserSkill => &["gamma", "alpha"],
            };
            exercise_turn(root, "cold", keys, scenario, generation, &native, &controller, &mut events, file).await?;
        } else {
            let (initial, hot, added_key): (&[&str], &[&str], &str) = match scenario {
                Scenario::MultiSkill | Scenario::UserSkill => (&["alpha", "beta"], &["gamma"], "gamma"),
                Scenario::ImageSkill => (&["alpha"], &["alpha", "beta"], "beta"),
            };
            exercise_turn(root, "initial", initial, scenario, generation, &native, &controller, &mut events, file).await?;
            let added = format!("isp-g06-{added_key}");
            let target = scenario.skill_root(root, added_key).join(&added);
            if target.exists() { return Err("热新增技能在首轮前已经发布".into()); }
            fs::rename(root.join("unpublished").join(added), &target).map_err(|_| "热新增技能发布失败")?;
            record(file, json!({"event":"skill_published","generation":generation,"native_session_id":native,
                "path":target.join("SKILL.md"),"after_case":"initial","same_connection":true}))?;
            exercise_turn(root, "hot", hot, scenario, generation, &native, &controller, &mut events, file).await?;
        }
        Ok(native)
    }.await;
    // 无论模型或证据断言是否失败，都先请求同一生产连接关闭；失败仍原样报告。
    let shutdown = Uuid::new_v4();
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: shutdown,
            action: RuntimeAction::Shutdown,
        })
        .await;
    let closed = tokio::time::timeout(Duration::from_secs(35), &mut task.0).await;
    let task_ok = matches!(&closed, Ok(Ok(Ok(()))));
    let receipt = managed_process::confirmed_exit(&state_dir, generation)
        .ok()
        .flatten();
    let mut extra_turns = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.kind,
            RuntimeEventKind::TurnStarted { .. }
                | RuntimeEventKind::TurnFinished { .. }
                | RuntimeEventKind::MessageAccepted { .. }
        ) {
            extra_turns += 1;
        }
    }
    record(
        file,
        json!({"event":"cleanup","generation":generation,"resumed":resumed,
        "task_finished_ok":task_ok,"receipt":receipt,"extra_turn_events":extra_turns}),
    )?;
    if let Err(reason) = result {
        return Err(reason);
    }
    if !task_ok
        || extra_turns != 0
        || !receipt
            .as_ref()
            .is_some_and(|value| value.cleanup_confirmed && value.exit_code == Some(0))
    {
        return Err("生产进程未自然退出或清理未确认，不能用强制回收代替".into());
    }
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要固定且已认证的 Grok、生产监督者和三次在线模型输入；原生证据须由配套运行器审核"]
async fn real_grok_multi_skill_hot_add_and_resume() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-multi-skill-probe")).unwrap(),
        "isolated Grok multi skill verification\n"
    );
    let mut file = File::create(root.join("events.ndjson")).unwrap();
    record(&mut file, json!({"event":"started","max_native_inputs":3,"production_connect":true,
        "candidate_flags_used":false,"coordinator_persistence_verified":false,"gui_verified":false})).unwrap();
    let result = async {
        let first = exercise_connection(&root, None, Scenario::MultiSkill, &mut file).await?;
        let resumed = exercise_connection(&root, Some(first.clone()), Scenario::MultiSkill, &mut file).await?;
        if resumed != first { return Err("冷恢复更换了原生会话".to_owned()); }
        record(&mut file, json!({"event":"adapter_flow_passed","native_session_id":first,"native_inputs":3,
            "native_byte_audit_pending":true,"runtime_generations":2,"coordinator_persistence_verified":false}))
    }.await;
    if let Err(reason) = &result {
        record(
            &mut file,
            json!({"event":"acceptance_failed","reason":reason}),
        )
        .unwrap();
    }
    assert!(result.is_ok(), "Grok 生产技能流程失败，原始证据已保留");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要固定且已认证的 Grok、生产监督者和三次图片加技能模型输入；原生图片与技能源须由配套运行器审核"]
async fn real_grok_image_skill_hot_add_and_resume() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-image-skill-probe")).unwrap(),
        "isolated Grok image skill verification\n"
    );
    // 必须启用既有原生终态历史投影；适配器的发送摘要不能替代原生消费证明。
    assert_eq!(
        env::var("INFINISHELL_GROK_MANAGED_IMAGE_TRACE").as_deref(),
        Ok("1")
    );
    let mut file = File::create(root.join("events.ndjson")).unwrap();
    record(
        &mut file,
        json!({"event":"started","scope":"grok_image_skill_hot_add_and_resume",
        "max_native_inputs":3,"production_connect":true,"candidate_flags_used":false,
        "coordinator_persistence_verified":false,"gui_verified":false}),
    )
    .unwrap();
    let result = async {
        let first = exercise_connection(&root, None, Scenario::ImageSkill, &mut file).await?;
        let resumed = exercise_connection(&root, Some(first.clone()), Scenario::ImageSkill, &mut file).await?;
        if resumed != first { return Err("图片技能冷恢复更换了原生会话".to_owned()); }
        record(&mut file, json!({"event":"adapter_flow_passed","scope":"grok_image_skill_hot_add_and_resume",
            "native_session_id":first,"native_inputs":3,"native_byte_audit_pending":true,"runtime_generations":2,
            "coordinator_persistence_verified":false,"gui_verified":false}))
    }.await;
    if let Err(reason) = &result {
        record(
            &mut file,
            json!({"event":"acceptance_failed","reason":reason}),
        )
        .unwrap();
    }
    assert!(
        result.is_ok(),
        "Grok 生产图片与技能流程失败，原始证据已保留"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要固定且已认证的 Grok、生产监督者和三次模型输入；用户技能目录、原生字节及自然清理须由配套运行器审核"]
async fn real_grok_user_skill_hot_add_and_resume() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-user-skill-probe")).unwrap(),
        "isolated Grok user skill verification\n"
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_USER_SKILL_TRACE").as_deref(),
        Ok("1")
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    let mut file = File::create(root.join("events.ndjson")).unwrap();
    record(
        &mut file,
        json!({"event":"started","scope":"grok_user_skill_hot_add_and_resume",
        "max_native_inputs":3,"production_connect":true,"candidate_flags_used":false,
        "coordinator_persistence_verified":false,"gui_verified":false}),
    )
    .unwrap();
    let result = async {
        let first = exercise_connection(&root, None, Scenario::UserSkill, &mut file).await?;
        let resumed = exercise_connection(&root, Some(first.clone()), Scenario::UserSkill, &mut file).await?;
        if resumed != first { return Err("用户技能冷恢复更换了原生会话".to_owned()); }
        record(&mut file, json!({"event":"adapter_flow_passed","scope":"grok_user_skill_hot_add_and_resume",
            "native_session_id":first,"native_inputs":3,"native_byte_audit_pending":true,"runtime_generations":2,
            "coordinator_persistence_verified":false,"gui_verified":false}))
    }.await;
    if let Err(reason) = &result {
        record(
            &mut file,
            json!({"event":"acceptance_failed","reason":reason}),
        )
        .unwrap();
    }
    assert!(result.is_ok(), "Grok 用户技能流程失败，原始证据已保留");
}
