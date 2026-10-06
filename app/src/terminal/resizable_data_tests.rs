use warpui::WindowId;

use super::{ModalSizes, ResizableData};

#[test]
fn cli_subagent_sizes_remember_latest_resize_in_same_window() {
    let window_id = WindowId::new();
    let mut data = ResizableData::default();
    data.insert(window_id, ModalSizes::default());

    data.set_cli_subagent_size(window_id, 640., 480.);
    let first_conversation_sizes = data.get_all_handles(window_id).expect("窗口尺寸应存在");
    assert_eq!(first_conversation_sizes.cli_subagent_width, 640.);
    assert_eq!(first_conversation_sizes.cli_subagent_height, 480.);

    data.set_cli_subagent_size(window_id, 720., 420.);
    let next_conversation_sizes = data.get_all_handles(window_id).expect("窗口尺寸应存在");
    assert_eq!(next_conversation_sizes.cli_subagent_width, 720.);
    assert_eq!(next_conversation_sizes.cli_subagent_height, 420.);
}

#[test]
fn cli_subagent_sizes_do_not_leak_to_another_window() {
    let first_window_id = WindowId::new();
    let second_window_id = WindowId::new();
    let mut data = ResizableData::default();
    data.insert(first_window_id, ModalSizes::default());
    data.insert(second_window_id, ModalSizes::default());

    data.set_cli_subagent_size(first_window_id, 640., 480.);

    let second_window_sizes = data
        .get_all_handles(second_window_id)
        .expect("第二个窗口尺寸应存在");
    assert_eq!(second_window_sizes.cli_subagent_width, 360.);
    assert_eq!(second_window_sizes.cli_subagent_height, 320.);

    data.set_cli_subagent_size(second_window_id, 720., 420.);

    let first_window_sizes = data
        .get_all_handles(first_window_id)
        .expect("第一个窗口尺寸应存在");
    assert_eq!(first_window_sizes.cli_subagent_width, 640.);
    assert_eq!(first_window_sizes.cli_subagent_height, 480.);
}
