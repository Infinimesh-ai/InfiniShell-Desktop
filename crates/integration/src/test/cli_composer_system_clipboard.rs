//! Linux/Windows 真实窗口与系统剪贴板的零模型验收。

use warp::features::FeatureFlag;
use warp::integration_testing::clipboard::{
    assert_cli_clipboard_draft, finish_cli_clipboard_evidence, open_cli_clipboard_composer,
    setup_cli_system_clipboard, wait_until_cli_clipboard_bootstrapped,
    write_cli_system_clipboard_image, write_cli_system_clipboard_text,
};
use warpui_core::integration::TestStep;

use crate::Builder;

pub fn test_cli_composer_system_clipboard_multiline_and_image() -> Builder {
    FeatureFlag::LocalCLIManagedTasks.set_enabled(true);

    Builder::new()
        .with_real_display()
        .with_setup(setup_cli_system_clipboard)
        .with_step(wait_until_cli_clipboard_bootstrapped())
        .with_step(open_cli_clipboard_composer())
        .with_step(write_cli_system_clipboard_text())
        .with_step(
            TestStep::new("粘贴中文两行并保留草稿")
                .with_keystrokes(&["ctrl-v"])
                .add_named_assertion("中文两行完整且未提交", assert_cli_clipboard_draft(0))
                .with_take_screenshot("chinese-multiline.png"),
        )
        .with_step(write_cli_system_clipboard_image())
        .with_step(
            TestStep::new("粘贴系统 PNG 并显示图片附件")
                .with_keystrokes(&["ctrl-v"])
                .add_named_assertion(
                    "图片唯一且像素完整，草稿未提交",
                    assert_cli_clipboard_draft(1),
                )
                .with_take_screenshot("pasted-image.png"),
        )
        .with_step(finish_cli_clipboard_evidence())
}
