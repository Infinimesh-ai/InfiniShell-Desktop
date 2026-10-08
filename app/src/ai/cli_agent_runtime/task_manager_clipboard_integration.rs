//! 仅用于真实窗口的系统剪贴板与静态视口验收，不启动原生任务或提交模型输入。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use command::blocking::Command;
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::json;
use sha2::{Digest, Sha256};
use warpui::clipboard::ClipboardContent;
use warpui::elements::{ScrollTarget, ScrollToPositionMode, get_rich_content_position_id};
#[cfg(target_os = "linux")]
use warpui::fonts::Properties;
use warpui::integration::{
    ARTIFACTS_DIR_ENV_VAR, AssertionCallback, AssertionOutcome, TestSetupUtils, TestStep,
};
#[cfg(target_os = "linux")]
use warpui::platform::LineStyle;
#[cfg(target_os = "linux")]
use warpui::text_layout::{ClipConfig, DEFAULT_TOP_BOTTOM_RATIO, StyleAndFont, TextStyle};
use warpui::units::Pixels;
use warpui::{App, SingletonEntity, TypedActionView, ViewHandle, WindowId, async_assert};

use super::{LocalCLITaskManagerView, SavedLaunchOptions, TaskManagerAction};
use crate::ai::cli_agent_runtime::coordinator::{
    LocalCLITaskCoordinator, ManagedApproval, ManagedTaskSnapshot, V05ApprovalFixtureInbox,
};
use crate::ai::cli_agent_runtime::{ApprovalDecision, PermissionPolicy};
#[cfg(target_os = "linux")]
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use crate::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use crate::persistence::model::{LocalCliTask, LocalCliTaskState};
use crate::terminal::History;
use crate::terminal::cli_agent::{CLIAgent, CLIAgentInstallModel, CLIAgentVersionStatus};
use crate::workspace::WorkspaceAction;
use uuid::Uuid;
use warp_cli::agent::Harness;

pub const CLI_CLIPBOARD_TEST_NAME: &str = "test_cli_composer_system_clipboard_multiline_and_image";
pub const CLI_CLIPBOARD_TEXT: &str = "中文第一行 ASCII\n第二行 123";
pub const V05_STATIC_TEST_NAME: &str = "test_cli_grok_static_viewport";
pub const V05_APPROVAL_TEST_NAME: &str = "test_cli_grok_approval_viewport";

const IMAGE_ENV: &str = "WARP_TEST_COMPOSER_CLIPBOARD_IMAGE";
const PRODUCER_KEY: &str = "cli-composer-system-clipboard-producer";
const V05_BOTTOM_SCROLL_KEY: &str = "v05-bottom-scroll-px";
const V05_PREVIOUS_SCROLL_KEY: &str = "v05-previous-scroll-px";
const V05_APPROVAL_INBOX_KEY: &str = "v05-approval-fixture-inbox";
const V05_TASK_ID: &str = "00000000-0000-0000-0000-000000000005";
const V05_ALLOW_ID: &str = "v05-synthetic-allow";
const V05_DENY_ID: &str = "v05-synthetic-deny";

struct ClipboardProducer(Child);

impl Drop for ClipboardProducer {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

pub fn setup_cli_system_clipboard(utils: &mut TestSetupUtils) {
    #[cfg(target_os = "windows")]
    {
        let profile = utils
            .test_dir()
            .canonicalize()
            .expect("测试 profile 必须存在");
        // 已知目录查询会检查目录存在；只补齐本次私有 profile，不修改系统注册表。
        for relative in ["AppData/Local", "AppData/Roaming", "Documents"] {
            fs::create_dir_all(profile.join(relative)).expect("创建私有 profile 子目录");
        }
        for (name, directory) in [
            ("config_local_dir", warp_core::paths::config_local_dir()),
            ("base_config_dir", warp_core::paths::base_config_dir()),
        ] {
            assert!(directory.is_absolute(), "{name} 必须解析为绝对路径");
            // 配置末级目录可能尚未建立；创建前先验证已有祖先，拒绝落入真实用户目录。
            let existing_parent = directory
                .ancestors()
                .find(|path| path.is_dir())
                .expect("配置目录必须存在可验证的父目录")
                .canonicalize()
                .expect("解析配置目录父路径");
            assert!(
                existing_parent.starts_with(&profile),
                "{name} 必须位于本次私有 profile 内"
            );
            fs::create_dir_all(&directory).expect("创建私有配置目录");
            assert!(
                directory
                    .canonicalize()
                    .expect("解析私有配置目录")
                    .starts_with(&profile),
                "{name} 创建后必须仍位于本次私有 profile 内"
            );
        }
    }

    let source_commit = std::env::var("WARP_TEST_GUI_SOURCE_COMMIT")
        .expect("必须由正式 workflow 传入被验收的提交 SHA");
    assert!(
        source_commit.len() == 40 && source_commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "验收提交必须为完整 Git SHA"
    );
    let output = PathBuf::from(
        std::env::var_os(ARTIFACTS_DIR_ENV_VAR).expect("必须显式指定 GUI 验收产物根目录"),
    );
    fs::create_dir_all(&output).expect("创建 GUI 验收产物根目录");
    // 独立目录拒绝已有结果，防止旧截图被误计为本轮截图。
    let output = output.join(format!("run-{}", std::process::id()));
    fs::create_dir(&output).expect("本轮 GUI 产物目录必须不存在");
    utils.set_env(ARTIFACTS_DIR_ENV_VAR, Some(&output));
    utils.set_env("WARPUI_INTEGRATION_SYSTEM_CLIPBOARD", Some("1"));
    utils.set_env("WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS", Some("1"));

    let image = RgbaImage::from_fn(16, 8, |x, _| {
        if x < 8 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });
    let path = utils.test_dir().join("clipboard-red-blue.png");
    image.save(&path).expect("创建合成 PNG");
    utils.set_env(IMAGE_ENV, Some(&path));
}

pub fn wait_until_cli_clipboard_bootstrapped() -> TestStep {
    // 复用原有全部断言与 20 秒期限，只为本用例补充失败状态，不输出终端原文。
    wait_until_bootstrapped_single_pane_for_tab(0).set_on_failure_handler(
        "记录真实 GUI 引导失败状态",
        |app, window_id| {
            single_terminal_view_for_tab(app, window_id, 0).read(app, |view, ctx| {
                let model = view.model.lock();
                let input_visible = view.is_input_box_visible(&model, ctx);
                let history_initialized = model
                    .block_list()
                    .active_block()
                    .session_id()
                    .is_some_and(|id| History::as_ref(ctx).is_session_initialized(&id));
                let bootstrapped = model.block_list().is_bootstrapped();
                let precmd_done = model.block_list().is_bootstrapping_precmd_done();
                AssertionOutcome::failure(format!(
                    "GUI 引导失败：input_visible={input_visible}, history_initialized={history_initialized}, bootstrapped={bootstrapped}, precmd_done={precmd_done}"
                ))
            })
        },
    )
}

fn composer(app: &App, window_id: WindowId) -> ViewHandle<LocalCLITaskManagerView> {
    let mut views = app
        .views_of_type::<LocalCLITaskManagerView>(window_id)
        .expect("必须存在真实托管输入框");
    assert_eq!(views.len(), 1, "只能存在一个托管输入框");
    views.pop().expect("唯一托管输入框")
}

pub fn open_cli_clipboard_composer() -> TestStep {
    TestStep::new("打开真实托管输入框，不启动任务")
        .with_action(|app, window_id, _| {
            assert!(
                FeatureFlag::LocalCLIManagedTasks.is_enabled(),
                "正式桌面默认配置必须已启用托管任务"
            );
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::OpenLocalCLITaskManager,
            );
        })
        .with_action(|app, window_id, _| {
            // 弹窗打开后焦点属于内容容器；模拟用户选择输入框，再通过真实按键粘贴。
            composer(app, window_id).update(app, |view, ctx| ctx.focus(&view.prompt));
        })
        .add_named_assertion(
            "托管输入框已聚焦且没有任务",
            |app, window_id| {
                let views = app
                    .views_of_type::<LocalCLITaskManagerView>(window_id)
                    .unwrap_or_default();
                if views.len() != 1 {
                    return AssertionOutcome::failure("真实托管输入框尚未显示".into());
                }
                views[0].read(app, |view, ctx| {
                    async_assert!(
                        view.prompt.is_focused(ctx) && view.tasks(ctx).is_empty(),
                        "真实输入框必须聚焦且没有任务；focused={}，task_count={}",
                        view.prompt.is_focused(ctx),
                        view.tasks(ctx).len()
                    )
                })
            },
        )
}

pub fn write_cli_system_clipboard_text() -> TestStep {
    TestStep::new("向系统剪贴板写入中文两行")
        .with_action(|app, _, _| {
            app.update(|ctx| {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(CLI_CLIPBOARD_TEXT.to_owned()));
            });
        })
        .add_named_assertion("系统剪贴板文本精确一致", |app, _| {
            app.update(|ctx| {
                async_assert!(
                    ctx.clipboard().read().plain_text == CLI_CLIPBOARD_TEXT,
                    "系统剪贴板没有保存完整中文两行"
                )
            })
        })
}

pub fn reveal_cli_clipboard_draft() -> TestStep {
    TestStep::new("将草稿和附件滚入真实截图视口").with_action(|app, window_id, _| {
        composer(app, window_id).update(app, |view, ctx| {
            // 使用编辑器实际布局位置，避免越界偏移被滚动容器重置到顶部。
            view.body_scroll.scroll_to_position(ScrollTarget {
                position_id: get_rich_content_position_id(&view.prompt.id()),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            ctx.notify();
        });
    })
}

fn red_blue_pixels(bytes: &[u8]) -> bool {
    let Ok(image) = image::load_from_memory_with_format(bytes, ImageFormat::Png) else {
        return false;
    };
    let rgba = image.to_rgba8();
    rgba.dimensions() == (16, 8)
        && rgba.enumerate_pixels().all(|(x, _, pixel)| {
            *pixel
                == if x < 8 {
                    Rgba([255, 0, 0, 255])
                } else {
                    Rgba([0, 0, 255, 255])
                }
        })
}

pub fn write_cli_system_clipboard_image() -> TestStep {
    TestStep::new("通过原生剪贴板生产者提供合成 PNG")
        .with_action(|_, _, data| {
            let path = PathBuf::from(std::env::var_os(IMAGE_ENV).expect("合成图片路径"));
            #[cfg(target_os = "linux")]
            let mut command = {
                let mut command = Command::new("xclip");
                // 前台进程由步骤数据拥有；正常结束或析构时停止，硬超时由专属 Xvfb 关闭连接。
                command.args(["-quiet", "-selection", "clipboard", "-target", "image/png", "-in"]);
                command.arg(&path);
                command
            };
            #[cfg(target_os = "windows")]
            let mut command = {
                let mut command = Command::new("powershell.exe");
                command.args(["-NoProfile", "-NonInteractive", "-STA", "-Command"]);
                command.arg(
                    "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $image=[System.Drawing.Image]::FromFile($env:WARP_TEST_COMPOSER_CLIPBOARD_IMAGE); try { [System.Windows.Forms.Clipboard]::SetImage($image) } finally { $image.Dispose() }",
                );
                command.env(IMAGE_ENV, &path);
                command.kill_on_parent_process_close();
                command
            };
            let child = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("系统图片剪贴板生产者必须能够启动");
            data.insert(PRODUCER_KEY, ClipboardProducer(child));
        })
        .add_named_assertion("系统剪贴板 PNG 像素精确一致", |app, _| {
            app.update(|ctx| {
                let images = ctx.clipboard().read().images.unwrap_or_default();
                async_assert!(
                    images.len() == 1
                        && images[0].mime_type == "image/png"
                        && red_blue_pixels(&images[0].data),
                    "必须实际读取系统剪贴板中的唯一红蓝 PNG"
                )
            })
        })
}

pub fn assert_cli_clipboard_draft(image_count: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        composer(app, window_id).read(app, |view, ctx| {
            let draft = view.prompt.as_ref(ctx).buffer_text(ctx);
            #[cfg(target_os = "linux")]
            {
                // 系统字体已安装不等于实际编辑器能回退；用同一字体和排版路径拒绝缺字截图。
                let appearance = Appearance::as_ref(ctx);
                let editor = view.prompt.as_ref(ctx);
                let font_cache = ctx.font_cache();
                let line_style = LineStyle {
                    font_size: editor.font_size(appearance),
                    line_height_ratio: editor.line_height_ratio(appearance),
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    fixed_width_tab_size: None,
                };
                let style = StyleAndFont::new(
                    editor.font_family(appearance),
                    Properties::default().weight(appearance.monospace_font_weight()),
                    TextStyle::new(),
                );
                for ch in ['中', '文', '第', '一', '行', '二'] {
                    let line = font_cache.text_layout_system().layout_line(
                        &ch.to_string(),
                        line_style,
                        &[(0..1, style)],
                        f32::MAX,
                        ClipConfig::default(),
                    );
                    let glyphs = line
                        .runs
                        .iter()
                        .flat_map(|run| &run.glyphs)
                        .collect::<Vec<_>>();
                    if !line.chars_with_missing_glyphs.is_empty()
                        || glyphs.is_empty()
                        || glyphs.iter().any(|glyph| glyph.id == 0)
                    {
                        return AssertionOutcome::failure(format!(
                            "真实编辑器字体缺少字符 U+{:04X}；glyph_count={}，missing_count={}",
                            ch as u32,
                            glyphs.len(),
                            line.chars_with_missing_glyphs.len()
                        ));
                    }
                }
            }
            let focused = view.prompt.is_focused(ctx);
            let task_count = view.tasks(ctx).len();
            let selected = view.selected_task.is_some();
            let pending_count = view.pending_inputs.len();
            let preparing = view.managed_input.preparing.is_some();
            let processing = view.managed_input.processing_images;
            let images = &view.managed_input.attachments.images;
            let pixels_match = images.iter().all(|image| {
                image.mime_type == "image/png"
                    && STANDARD
                        .decode(&image.data)
                        .is_ok_and(|bytes| red_blue_pixels(&bytes))
            });
            async_assert!(
                draft == CLI_CLIPBOARD_TEXT
                    && focused
                    && images.len() == image_count
                    && pixels_match
                    && !processing
                    && task_count == 0
                    && !selected
                    && pending_count == 0
                    && !preparing,
                "中文草稿、图片附件或零提交状态不符合预期；focused={focused}，\
                 草稿字节数={}，预期字节数={}，UTF8前64字节={:02x?}，LF字节位置={:?}，\
                 task_count={task_count}，selected={selected}，pending_count={pending_count}，\
                 preparing={preparing}，processing={processing}，pixels_match={pixels_match}，\
                 附件数={}，预期={image_count}",
                draft.len(),
                CLI_CLIPBOARD_TEXT.len(),
                &draft.as_bytes()[..draft.len().min(64)],
                draft
                    .match_indices('\n')
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>(),
                images.len()
            )
        })
    })
}

pub fn finish_cli_clipboard_evidence() -> TestStep {
    TestStep::new("强制校验本轮截图并保存安全收据")
        .with_action(|app, window_id, data| {
            assert!(
                matches!(
                    assert_cli_clipboard_draft(1)(app, window_id),
                    AssertionOutcome::Success
                ),
                "仅在真实草稿、附件与零提交断言全部通过后保存收据"
            );
            let expected_locale =
                std::env::var("WARP_TEST_GUI_LOCALE").unwrap_or_else(|_| "en".to_owned());
            assert!(
                matches!(expected_locale.as_str(), "en" | "zh-CN"),
                "仅验收英文和简体中文界面"
            );
            let ui_locale = crate::i18n::loader()
                .expect("界面语言必须已初始化")
                .current_languages()
                .first()
                .expect("必须存在实际界面语言")
                .to_string();
            assert_eq!(
                ui_locale, expected_locale,
                "实际界面语言必须匹配本轮验收语言"
            );
            assert!(
                FeatureFlag::LocalCLIManagedTasks.is_enabled(),
                "保存收据时正式桌面默认配置必须仍启用托管任务"
            );
            drop(data.remove::<_, ClipboardProducer>(PRODUCER_KEY));
            let root = PathBuf::from(std::env::var_os(ARTIFACTS_DIR_ENV_VAR).unwrap())
                .join(CLI_CLIPBOARD_TEST_NAME);
            let runs = fs::read_dir(&root)
                .expect("截图目录必须存在")
                .map(|entry| entry.expect("读取截图目录").path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            assert_eq!(runs.len(), 1, "必须只有本轮截图目录");
            let mut screenshots = Vec::new();
            for name in ["chinese-multiline.png", "pasted-image.png"] {
                let bytes = fs::read(runs[0].join(name)).expect("真实窗口截图必须已生成");
                let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                    .expect("真实窗口截图必须为可解码 PNG")
                    .to_rgba8();
                assert!(
                    image.width() >= 640 && image.height() >= 400,
                    "窗口截图尺寸不足"
                );
                let first = image.get_pixel(0, 0);
                assert!(
                    image.pixels().any(|pixel| pixel != first),
                    "窗口截图不能是空白帧"
                );
                screenshots.push(serde_json::json!({
                    "file": name,
                    "sha256": format!("{:x}", Sha256::digest(&bytes)),
                    "width": image.width(),
                    "height": image.height()
                }));
            }
            let mut executable =
                fs::File::open(std::env::current_exe().expect("当前 GUI 测试程序"))
                    .expect("读取当前 GUI 测试程序哈希");
            let mut executable_hash = Sha256::new();
            let mut buffer = [0_u8; 65536];
            loop {
                let count = executable.read(&mut buffer).expect("读取测试程序");
                if count == 0 {
                    break;
                }
                executable_hash.update(&buffer[..count]);
            }
            let receipt = serde_json::json!({
                "schema": 1,
                "test": CLI_CLIPBOARD_TEST_NAME,
                "platform": std::env::consts::OS,
                "ui_locale": ui_locale,
                "source_commit": std::env::var("WARP_TEST_GUI_SOURCE_COMMIT").unwrap(),
                "desktop_default_features_applied": true,
                "managed_feature_test_override": false,
                "managed_feature_enabled": FeatureFlag::LocalCLIManagedTasks.is_enabled(),
                "binary_sha256": format!("{:x}", executable_hash.finalize()),
                "system_clipboard": true,
                "text_exact": true,
                "image_pixels_exact": true,
                "attachment_count": 1,
                "managed_tasks_created": 0,
                "model_inputs": 0,
                "physical_ime_exercised": false,
                "visual_review_required": true,
                "screenshots": screenshots
            });
            fs::write(
                runs[0].join("receipt.safe.json"),
                serde_json::to_vec_pretty(&receipt).expect("序列化安全收据"),
            )
            .expect("保存安全收据");
        })
        .add_named_assertion("验收结束时草稿仍未提交", assert_cli_clipboard_draft(1))
}

pub fn wait_v05_fixed_grok_installation() -> TestStep {
    TestStep::new("等待真实安装扫描识别固定 Grok").add_named_assertion(
        "扫描结果绑定 Grok 1.0.41",
        |app, _| {
            let expected = PathBuf::from(
                std::env::var_os("INFINISHELL_TEST_GROK_EXE").expect("固定 Grok 路径"),
            );
            app.update(|ctx| {
                let installation = CLIAgentInstallModel::as_ref(ctx).installation(CLIAgent::Grok);
                async_assert!(
                    installation.is_some_and(|installation| {
                        installation.version == CLIAgentVersionStatus::Detected("1.0.41".into())
                            && installation
                                .executable
                                .as_deref()
                                .is_some_and(|actual| v05_same_executable(actual, &expected))
                    }),
                    "必须扫描到固定 Grok 1.0.41；当前安装={installation:?}"
                )
            })
        },
    )
}

fn v05_same_executable(actual: &Path, expected: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Windows 的扫描器会把同一 grok.exe 路径返回为 grok.EXE。
        actual
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
    }
    #[cfg(not(target_os = "windows"))]
    {
        actual == expected
    }
}

pub fn assert_v05_fixed_grok() -> AssertionCallback {
    Box::new(|app, window_id| {
        let expected =
            PathBuf::from(std::env::var_os("INFINISHELL_TEST_GROK_EXE").expect("固定 Grok 路径"));
        composer(app, window_id).read(app, |view, ctx| {
            let installation = CLIAgentInstallModel::as_ref(ctx).installation(CLIAgent::Grok);
            async_assert!(
                view.harness == Harness::Grok
                    && installation.is_some_and(|installation| {
                        installation.version == CLIAgentVersionStatus::Detected("1.0.41".into())
                            && installation
                                .executable
                                .as_deref()
                                .is_some_and(|actual| v05_same_executable(actual, &expected))
                    }),
                "必须在真实界面选中扫描到的固定 Grok 1.0.41；当前安装={installation:?}"
            )
        })
    })
}

pub fn select_v05_grok_policy(policy: PermissionPolicy) -> TestStep {
    TestStep::new("选择固定 Grok 策略")
        .with_action(move |app, window_id, _| {
            composer(app, window_id).update(app, |view, ctx| {
                view.handle_action(&TaskManagerAction::SelectHarness(Harness::Grok), ctx);
                view.handle_action(&TaskManagerAction::SelectPermission(policy), ctx);
                // 策略按钮与说明位于安装信息之后，截图时滚入可见区域。
                view.body_scroll.scroll_to(Pixels::new(450.0));
                ctx.notify();
            });
        })
        .add_named_assertion(
            "Grok 固定版本与所选策略均生效",
            move |app, window_id| {
                if !matches!(
                    assert_v05_fixed_grok()(app, window_id),
                    AssertionOutcome::Success
                ) {
                    return AssertionOutcome::failure("未扫描到固定 Grok 1.0.41".into());
                }
                composer(app, window_id).read(app, |view, ctx| {
                    async_assert!(view.permission == policy, "策略未被界面接受：{policy:?}")
                })
            },
        )
}

pub fn advance_v05_grok_scroll() -> TestStep {
    TestStep::new("逐段滚动 Grok 策略视口").with_action(|app, window_id, data| {
        composer(app, window_id).update(app, |view, ctx| {
            data.insert(
                V05_PREVIOUS_SCROLL_KEY,
                view.body_scroll.scroll_start().as_f32(),
            );
            // 裁剪滚动容器对远超内容高度的目标会重置到顶部，逐段推进才可抵达底部。
            view.body_scroll.scroll_by(Pixels::new(250.0));
            ctx.notify();
        });
    })
}

pub fn scroll_v05_grok(position: &'static str) -> TestStep {
    TestStep::new("滚动 Grok 策略视口")
        .with_action(move |app, window_id, data| {
            composer(app, window_id).update(app, |view, ctx| {
                let pixels = match position {
                    "top" => 0.0,
                    // 底部截图完成布局后，当前偏移就是裁剪后的真实最大值。
                    "middle" => {
                        let bottom = view.body_scroll.scroll_start().as_f32();
                        data.insert(V05_BOTTOM_SCROLL_KEY, bottom);
                        bottom / 2.0
                    }
                    "bottom" => {
                        let bottom = view.body_scroll.scroll_start().as_f32();
                        let previous = *data
                            .get::<_, f32>(V05_PREVIOUS_SCROLL_KEY)
                            .expect("必须先逐段滚动");
                        assert_eq!(bottom, previous, "滚动尚未抵达底部");
                        bottom
                    }
                    other => panic!("未知滚动位置：{other}"),
                };
                view.body_scroll.scroll_to(Pixels::new(pixels));
                ctx.notify();
            });
        })
        .add_named_assertion(
            "滚动位置已改变且没有启动模型任务",
            move |app, window_id| {
                composer(app, window_id).read(app, |view, ctx| {
                    let offset = view.body_scroll.scroll_start().as_f32();
                    let normal_fully_visible = offset == 0.0
                        && std::env::var("WARP_TEST_GUI_SIZE").as_deref() == Ok("normal");
                    async_assert!(
                        view.tasks(ctx).is_empty()
                            && (position == "top" || offset > 0.0 || normal_fully_visible),
                        "滚动位置或任务数错误：{position}，offset={offset}，tasks={}",
                        view.tasks(ctx).len()
                    )
                })
            },
        )
}

fn v05_file_sha256(path: PathBuf) -> String {
    let mut file = fs::File::open(path).expect("读取本轮可执行文件");
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer).expect("读取可执行文件摘要");
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    format!("{:x}", digest.finalize())
}

pub fn finish_v05_static_evidence() -> TestStep {
    TestStep::new("核对 Grok 静态截图并保存部分收据").with_action(|app, window_id, data| {
        assert!(matches!(assert_v05_fixed_grok()(app, window_id), AssertionOutcome::Success));
        let locale = std::env::var("WARP_TEST_GUI_LOCALE").expect("界面语言");
        assert!(matches!(locale.as_str(), "en" | "zh-CN"));
        assert_eq!(
            crate::i18n::loader().expect("本地化加载器").current_languages()[0].to_string(),
            locale
        );
        let requested_size = std::env::var("WARP_TEST_GUI_SIZE").expect("视口尺寸");
        let bottom_scroll_px = *data
            .get::<_, f32>(V05_BOTTOM_SCROLL_KEY)
            .expect("必须在底部截图后记录实际滚动范围");
        assert!(bottom_scroll_px.is_finite() && bottom_scroll_px >= 0.0);
        if requested_size == "compact" {
            assert!(bottom_scroll_px > 0.0, "紧凑视口必须覆盖真实滚动区域");
        }
        let expected = match requested_size.as_str() {
            "compact" => (800.0, 600.0),
            "normal" => (1280.0, 800.0),
            _ => panic!("未知视口尺寸：{requested_size}"),
        };
        let bounds = app.window_bounds(&window_id).expect("真实窗口边界");
        assert_eq!(
            (bounds.width(), bounds.height()),
            expected,
            "真实窗口尺寸必须匹配本轮视口"
        );
        let root = PathBuf::from(std::env::var_os(ARTIFACTS_DIR_ENV_VAR).expect("截图根目录"))
            .join(V05_STATIC_TEST_NAME);
        let runs = fs::read_dir(&root)
            .expect("截图目录")
            .map(|entry| entry.expect("截图目录项").path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1, "只允许本轮真实窗口截图");
        let mut screenshots = Vec::new();
        for name in [
            "inherit.png", "fixed-read.png", "fixed-files.png", "skill-disabled.png",
            "scroll-top.png", "scroll-middle.png", "scroll-bottom.png",
        ] {
            let bytes = fs::read(runs[0].join(name)).expect("真实窗口截图");
            let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .expect("截图必须是 PNG")
                .to_rgba8();
            assert!(image.width() >= 640 && image.height() >= 400, "截图尺寸不足");
            let first = image.get_pixel(0, 0);
            assert!(image.pixels().any(|pixel| pixel != first), "截图不能是空白帧");
            screenshots.push(serde_json::json!({
                "file": name,
                "sha256": format!("{:x}", Sha256::digest(&bytes)),
                "width": image.width(),
                "height": image.height()
            }));
        }
        let receipt = serde_json::json!({
            "schema": 1,
            "test": V05_STATIC_TEST_NAME,
            "platform": std::env::consts::OS,
            "ui_locale": locale,
            "requested_size": requested_size,
            "window_width": bounds.width(),
            "window_height": bounds.height(),
            "bottom_scroll_px": bottom_scroll_px,
            "scroll_mode": if bottom_scroll_px > 0.0 { "scrollable" } else { "fully_visible" },
            "source_commit": std::env::var("WARP_TEST_GUI_SOURCE_COMMIT").expect("源码提交"),
            "binary_sha256": v05_file_sha256(std::env::current_exe().expect("当前 GUI 测试程序")),
            "grok_sha256": v05_file_sha256(PathBuf::from(std::env::var_os("INFINISHELL_TEST_GROK_EXE").expect("固定 Grok 路径"))),
            "native_version": "1.0.41",
            "model_inputs": 0,
            "approval_states_exercised": false,
            "visual_review_required": true,
            "v05_complete": false,
            "screenshots": screenshots
        });
        fs::write(
            runs[0].join("receipt.safe.json"),
            serde_json::to_vec_pretty(&receipt).expect("序列化收据"),
        )
        .expect("保存部分收据");
    })
}

pub fn prepare_v05_approval_fixture() -> TestStep {
    TestStep::new("在真实 Grok 窗口放入合成审批卡片")
        .with_action(|app, window_id, data| {
            assert!(matches!(assert_v05_fixed_grok()(app, window_id), AssertionOutcome::Success));
            let cwd = std::env::current_dir().expect("夹具工作目录");
            let config = SavedLaunchOptions {
                permission_policy: PermissionPolicy::GrokRestrictedFilesV1,
                permission_ceiling: None,
                claude_profile: None,
                grok_profile: None,
                model: Some("grok-4.7".into()),
                local_tools: None,
                selected_skills: Vec::new(),
            };
            let snapshot = ManagedTaskSnapshot {
                task: LocalCliTask {
                    version: 1,
                    task_id: V05_TASK_ID.into(),
                    parent_task_id: None,
                    parent_generation: None,
                    harness: "grok".into(),
                    working_directory: cwd.to_string_lossy().into_owned(),
                    config_json: serde_json::to_string(&config).expect("夹具配置序列化"),
                    native_session_id: Some("v05-synthetic-session".into()),
                    generation: 1,
                    revision: 0,
                    state: LocalCliTaskState::WaitingForUser,
                    result: None,
                    terminal_evidence: None,
                },
                ready: true,
                connected: true,
                active_turn_id: Some("v05-synthetic-turn".into()),
                approvals: vec![
                    ManagedApproval {
                        approval_id: V05_ALLOW_ID.into(),
                        turn_id: "v05-synthetic-turn".into(),
                        method: "session/request_permission".into(),
                        details: json!({
                            "sessionId": "v05-synthetic-session",
                            "toolCall": {
                                "toolCallId": "v05-read-file",
                                "toolName": "GrokBuild:read_file",
                                "input": {"file_path": cwd.join("src/main.rs")}
                            }
                        }),
                    },
                    ManagedApproval {
                        approval_id: V05_DENY_ID.into(),
                        turn_id: "v05-synthetic-turn".into(),
                        method: "session/request_permission".into(),
                        details: json!({
                            "sessionId": "v05-synthetic-session",
                            "toolCall": {
                                "toolCallId": "v05-write-file",
                                "toolName": "GrokBuild:write_file",
                                "input": {
                                    "file_path": cwd.join("src/long-name-for-approval-layout.rs"),
                                    "content": "A fixed synthetic approval payload for viewport layout"
                                }
                            }
                        }),
                    },
                ],
                output: String::new(),
                error: None,
            };
            let inbox = app.update(|ctx| {
                LocalCLITaskCoordinator::handle(ctx)
                    .update(ctx, |model, ctx| model.install_v05_approval_fixture(snapshot, ctx))
            });
            data.insert(V05_APPROVAL_INBOX_KEY, inbox);
            composer(app, window_id).update(app, |view, ctx| {
                view.handle_action(&TaskManagerAction::SelectTask(V05_TASK_ID.into()), ctx);
            });
        })
        .add_named_assertion("合成审批仅在固定 Grok 面板展示", |app, window_id| {
            composer(app, window_id).read(app, |view, ctx| {
                let snapshot = view.selected_snapshot(ctx);
                async_assert!(
                    view.harness == Harness::Grok
                        && view.permission == PermissionPolicy::GrokRestrictedFilesV1
                        && view.selected_task.as_deref() == Some(V05_TASK_ID)
                        && snapshot.as_ref().is_some_and(|snapshot| snapshot.approvals.len() == 2)
                        && view.approval_buttons.len() == 2,
                    "合成审批快照未显示于真实 Grok 面板"
                )
            })
        })
}

pub fn reveal_v05_approval(index: usize, decision: ApprovalDecision) -> TestStep {
    let position_id = match decision {
        ApprovalDecision::AllowOnce => format!("v05-approval-allow-{index}"),
        ApprovalDecision::DenyOnce => format!("v05-approval-deny-{index}"),
    };
    TestStep::new("将 Grok 审批按钮滚入真实视口")
        .with_action(move |app, window_id, _| {
            composer(app, window_id).update(app, |view, ctx| {
                view.body_scroll.scroll_to_position(ScrollTarget {
                    position_id: position_id.clone(),
                    mode: ScrollToPositionMode::FullyIntoView,
                });
                ctx.notify();
            });
        })
        .add_named_assertion(
            "审批卡片保持在同一任务和代次",
            |app, window_id| {
                composer(app, window_id).read(app, |view, ctx| {
                    async_assert!(
                        view.selected_snapshot(ctx).is_some_and(|snapshot| {
                            snapshot.task.task_id == V05_TASK_ID
                                && snapshot.task.generation == 1
                                && snapshot.approvals.len() == 2
                        }),
                        "审批卡片切换了任务或代次"
                    )
                })
            },
        )
}

pub fn click_v05_approval(index: usize, decision: ApprovalDecision) -> TestStep {
    let (approval_id, position_id) = match (index, decision) {
        (0, ApprovalDecision::AllowOnce) => (V05_ALLOW_ID, "v05-approval-allow-0"),
        (1, ApprovalDecision::DenyOnce) => (V05_DENY_ID, "v05-approval-deny-1"),
        _ => panic!("未知合成审批按钮：{index}/{decision:?}"),
    };
    TestStep::new("鼠标点击真实 Grok 审批按钮")
        .with_click_on_saved_position(position_id)
        .add_named_assertion_with_data_from_prior_step(
            "审批动作按原任务、代次和决策进入应用队列且按钮禁用",
            move |app, window_id, data| {
                let inbox = data
                    .get_mut::<_, V05ApprovalFixtureInbox>(V05_APPROVAL_INBOX_KEY)
                    .expect("合成审批接收器");
                let observed = match inbox.observe(approval_id, decision) {
                    Ok(observed) => observed,
                    Err(error) => return AssertionOutcome::failure(error),
                };
                composer(app, window_id).read(app, |view, ctx| {
                    let buttons = &view.approval_buttons[index];
                    let disabled = buttons.allow.as_ref(ctx).is_disabled()
                        && buttons.deny.as_ref(ctx).is_disabled();
                    async_assert!(
                        observed
                            && disabled
                            && buttons.task_id == V05_TASK_ID
                            && buttons.task_generation == 1,
                        "审批点击未进入夹具队列或按钮仍可重复点击：observed={observed}，disabled={disabled}"
                    )
                })
            },
        )
}

pub fn finish_v05_approval_evidence() -> TestStep {
    TestStep::new("核对合成审批 GUI 截图和动作收据").with_action(|app, window_id, data| {
        let inbox = data
            .get_mut::<_, V05ApprovalFixtureInbox>(V05_APPROVAL_INBOX_KEY)
            .expect("合成审批接收器");
        assert_eq!(inbox.observed_count(), 2, "必须观察到允许和拒绝两次点击");
        assert!(!inbox.has_unexpected_request(), "出现额外审批动作");
        assert!(matches!(assert_v05_fixed_grok()(app, window_id), AssertionOutcome::Success));
        let locale = std::env::var("WARP_TEST_GUI_LOCALE").expect("界面语言");
        assert!(matches!(locale.as_str(), "en" | "zh-CN"));
        assert_eq!(
            crate::i18n::loader().expect("本地化加载器").current_languages()[0].to_string(),
            locale
        );
        let size = std::env::var("WARP_TEST_GUI_SIZE").expect("视口尺寸");
        let expected = match size.as_str() {
            "compact" => (800.0, 600.0),
            "normal" => (1280.0, 800.0),
            _ => panic!("未知视口尺寸：{size}"),
        };
        let bounds = app.window_bounds(&window_id).expect("真实窗口边界");
        assert_eq!((bounds.width(), bounds.height()), expected);
        let root = PathBuf::from(std::env::var_os(ARTIFACTS_DIR_ENV_VAR).expect("截图根目录"))
            .join(V05_APPROVAL_TEST_NAME);
        let runs = fs::read_dir(&root)
            .expect("截图目录")
            .map(|entry| entry.expect("截图目录项").path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1, "只允许本轮真实窗口截图");
        let mut screenshots = Vec::new();
        for name in [
            "allow-ready.png", "allow-pending.png", "deny-ready.png", "deny-pending.png",
        ] {
            let bytes = fs::read(runs[0].join(name)).expect("真实窗口截图");
            let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .expect("截图必须是 PNG")
                .to_rgba8();
            assert!(image.width() >= 640 && image.height() >= 400, "截图尺寸不足");
            let first = image.get_pixel(0, 0);
            assert!(image.pixels().any(|pixel| pixel != first), "截图不能是空白帧");
            screenshots.push(json!({
                "file": name,
                "sha256": format!("{:x}", Sha256::digest(&bytes)),
                "width": image.width(),
                "height": image.height()
            }));
        }
        let receipt = json!({
            "schema": 1,
            "test": V05_APPROVAL_TEST_NAME,
            "platform": std::env::consts::OS,
            "ui_locale": locale,
            "requested_size": size,
            "window_width": bounds.width(),
            "window_height": bounds.height(),
            "source_commit": std::env::var("WARP_TEST_GUI_SOURCE_COMMIT").expect("源码提交"),
            "binary_sha256": v05_file_sha256(std::env::current_exe().expect("当前 GUI 测试程序")),
            "grok_sha256": v05_file_sha256(PathBuf::from(std::env::var_os("INFINISHELL_TEST_GROK_EXE").expect("固定 Grok 路径"))),
            "native_version": "1.0.41",
            "model_inputs": 0,
            "synthetic_approval_cards": 2,
            "app_approval_actions": ["AllowOnce", "DenyOnce"],
            "native_approval_requested": false,
            "native_approval_resolved": false,
            "visual_review_required": true,
            "v05_complete": false,
            "screenshots": screenshots
        });
        fs::write(
            runs[0].join("receipt.safe.json"),
            serde_json::to_vec_pretty(&receipt).expect("序列化收据"),
        )
        .expect("保存合成审批收据");
    })
}
