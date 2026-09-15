//! 覆盖状态栏停止按钮到真实 PTY 的中断链路。

use std::time::Duration;

use pathfinder_geometry::vector::vec2f;
use warp::features::FeatureFlag;
use warp::i18n;
use warp::integration_testing::agent_mode::MonitoringCommandFixture;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::current_shell_starter_and_version;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_echo_str, execute_long_running_command,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_input_view_for_tab;
use warp::terminal::shell::ShellType;
use warpui_core::Event;
use warpui_core::integration::TestStep;

use crate::Builder;

pub fn test_stop_task_interrupts_monitoring_loop() -> Builder {
    FeatureFlag::AgentView.set_enabled(true);
    i18n::init(Some("en"));
    let fixture = MonitoringCommandFixture::default();
    let mut builder = Builder::new()
        // 事故命令使用 POSIX shell 语法；Windows 的控制路径由单元测试覆盖。
        .set_should_run_test(|| {
            matches!(
                current_shell_starter_and_version().0.shell_type(),
                ShellType::Bash | ShellType::Zsh
            )
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_long_running_command(
            0,
            "while true; do date; sleep 10; done".to_owned(),
        ))
        .with_step(fixture.attach_to_running_command());

    // 真实显示验证时保存两种语言的同一按钮及提示，不增加另一条重复的 PTY 测试。
    if std::env::var_os("WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS").is_some() {
        for (locale, filename) in [
            ("en", "agent_stop_tooltip_en.png"),
            ("zh-CN", "agent_stop_tooltip_zh_cn.png"),
        ] {
            builder = builder
                .with_step(
                    TestStep::new("切换停止提示的语言")
                        .with_event(Event::MouseMoved {
                            position: vec2f(10.0, 10.0),
                            cmd: false,
                            shift: false,
                            is_synthetic: false,
                        })
                        .with_action(move |app, window_id, _| {
                            i18n::set_locale(locale);
                            single_input_view_for_tab(app, window_id, 0).update(
                                app,
                                |input, ctx| {
                                    input.agent_status_bar().update(ctx, |_, ctx| ctx.notify());
                                    ctx.notify();
                                },
                            );
                        }),
                )
                .with_step(
                    TestStep::new("悬停显示停止提示")
                        .with_hover_over_saved_position("agent_stop_task_button")
                        .set_post_step_pause(Duration::from_millis(800)),
                )
                .with_step(
                    TestStep::new("保存停止按钮和提示的布局")
                        .with_take_screenshot(filename)
                        .add_named_assertion("截图期间监控命令未退出", fixture.running_assertion()),
                );
        }
    }

    let clicked_fixture = fixture.clone();
    builder
        .with_step(
            TestStep::new("鼠标点击一次停止任务")
                .with_click_on_saved_position_fn(move |app, window_id| {
                    clicked_fixture.stop_button_position(app, window_id)
                })
                .set_timeout(Duration::from_secs(5))
                .add_named_assertion("轮询结束且任务已取消", fixture.stopped_assertion()),
        )
        .with_step(
            new_step_with_default_assertions("切回 shell 输入").with_action(|app, window_id, _| {
                single_input_view_for_tab(app, window_id, 0)
                    .update(app, |input, ctx| input.set_input_mode_terminal(true, ctx));
            }),
        )
        .with_step(execute_echo_str(0, "monitor-stop-recovered"))
}
