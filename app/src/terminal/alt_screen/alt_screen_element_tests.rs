use std::cell::RefCell;
use std::rc::Rc;

use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_core::semantic_selection::SemanticSelection;
use warpui::elements::{
    ConstrainedBox, DragBarSide, Point, Rect, Resizable, resizable_state_handle,
};
use warpui::event::{DispatchedEvent, ModifiersState};
use warpui::platform::WindowStyle;
use warpui::presenter::DispatchedActionKind;
use warpui::units::IntoLines;
use warpui::{
    AfterLayoutContext, App, AppContext, Element, Entity, EntityIdSet, Event, EventContext,
    LayoutContext, ModelHandle, PaintContext, Presenter, SingletonEntity, SizeConstraint,
    TypedActionView, View, ViewHandle, WindowId, WindowInvalidation,
};

use super::AltScreenElement;
use crate::appearance::Appearance;
use crate::settings::EnforceMinimumContrast;
use crate::terminal::TerminalView;
use crate::terminal::alt_screen::should_intercept_mouse;
use crate::terminal::find::TerminalFindModel;
use crate::terminal::model::ansi::{self, Handler};
use crate::terminal::model::selection::SelectAction;
use crate::terminal::view::TerminalAction;
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, initialize_app_for_terminal_view,
};

#[derive(Clone, Copy)]
struct SubagentLayout {
    min: Vector2F,
    max: Vector2F,
    size: Vector2F,
}

struct ObservedSubagent {
    child: Box<dyn Element>,
    observed: Rc<RefCell<Option<SubagentLayout>>>,
}

impl Element for ObservedSubagent {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = self.child.layout(constraint, ctx, app);
        // 记录父约束原值，避免 Resizable 自身钳零掩盖父布局传入负上限。
        *self.observed.borrow_mut() = Some(SubagentLayout {
            min: constraint.min,
            max: constraint.max,
            size,
        });
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}

struct AltScreenLayoutProbe {
    element: AltScreenElement,
}

impl Element for AltScreenLayoutProbe {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.element.layout(constraint, ctx, app)
    }

    // 本测试只验证真实父子布局，不让极小尺寸样例进入无关的终端网格绘制。
    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, _: Vector2F, _: &mut PaintContext, _: &AppContext) {}

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.element.size()
    }

    fn origin(&self) -> Option<Point> {
        None
    }
}

struct AltScreenLayoutView {
    terminal: ViewHandle<TerminalView>,
    find_model: ModelHandle<TerminalFindModel>,
    pane_size: Vector2F,
    requested_size: Vector2F,
    observed: Rc<RefCell<Option<SubagentLayout>>>,
}

impl Entity for AltScreenLayoutView {
    type Event = ();
}

impl TypedActionView for AltScreenLayoutView {
    type Action = ();
}

impl View for AltScreenLayoutView {
    fn ui_name() -> &'static str {
        "alt_screen_subagent_layout_test"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let (model, render_context) = self.terminal.read(app, |view, app| {
            let model = view.model.lock();
            (
                view.model.clone(),
                view.get_terminal_view_render_context(&model, app),
            )
        });
        let width_resizable = Resizable::new(
            resizable_state_handle(self.requested_size.x()),
            Rect::new().finish(),
        )
        .with_dragbar_side(DragBarSide::Left)
        .finish();
        let subagent = ObservedSubagent {
            child: Resizable::new(
                resizable_state_handle(self.requested_size.y()),
                width_resizable,
            )
            .with_dragbar_side(DragBarSide::Top)
            .finish(),
            observed: self.observed.clone(),
        }
        .finish();
        let element = AltScreenElement::new(
            model,
            render_context,
            self.find_model.clone(),
            EnforceMinimumContrast::Never,
            None,
            Appearance::as_ref(app),
            0.into_lines(),
            None,
            Some(subagent),
        );

        ConstrainedBox::new(AltScreenLayoutProbe { element }.finish())
            .with_max_width(self.pane_size.x())
            .with_max_height(self.pane_size.y())
            .finish()
    }
}

fn assert_subagent_layout(
    pane_size: Vector2F,
    requested_size: Vector2F,
    expected_size: Vector2F,
    expected_max: Vector2F,
) {
    App::test((), move |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, terminal) = add_window_with_id_and_terminal(&mut app, None);
        let model = terminal.read(&app, |view, _| view.model.clone());
        let find_model = app.add_model(|ctx| TerminalFindModel::new(model, ctx));
        let observed = Rc::new(RefCell::new(None));
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| AltScreenLayoutView {
            terminal,
            find_model,
            pane_size,
            requested_size,
            observed: observed.clone(),
        });

        app.update(|ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = EntityIdSet::default();
            updated.insert(ctx.root_view_id(window_id).expect("测试根视图应存在"));
            presenter.invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            // 窄分屏仍放在大窗口中，确保限制来自 pane，而非整窗尺寸。
            presenter.build_scene(vec2f(1000., 700.), 1., None, ctx);
        });

        let layout = observed
            .borrow()
            .expect("真实 AltScreenElement 应布局子浮窗");
        assert_eq!(layout.min, vec2f(0., 0.));
        assert_eq!(layout.max, expected_max);
        assert_eq!(layout.size, expected_size);
    });
}

#[test]
fn alt_screen_subagent_layout_preserves_resized_width_above_thirty_percent() {
    assert_subagent_layout(
        vec2f(1000., 700.),
        vec2f(640., 480.),
        vec2f(640., 480.),
        vec2f(984., 676.),
    );
}

#[test]
fn alt_screen_subagent_layout_reaches_available_width_without_losing_margins() {
    assert_subagent_layout(
        vec2f(1000., 700.),
        vec2f(984., 684.),
        vec2f(984., 676.),
        vec2f(984., 676.),
    );
}

#[test]
fn alt_screen_subagent_layout_clips_remembered_size_to_narrow_pane() {
    assert_subagent_layout(
        vec2f(320., 200.),
        vec2f(640., 480.),
        vec2f(304., 176.),
        vec2f(304., 176.),
    );
}

#[test]
fn alt_screen_subagent_layout_never_passes_negative_limits_to_resizable() {
    assert_subagent_layout(
        vec2f(4., 4.),
        vec2f(640., 480.),
        vec2f(0., 0.),
        vec2f(0., 0.),
    );
}

fn dispatch_drag_event(
    presenter: &mut Presenter,
    window_id: WindowId,
    event: Event,
    observed_actions: &mut Vec<&'static str>,
    ctx: &mut AppContext,
) {
    let result = presenter.dispatch_event(event, ctx);
    // 保持真实窗口派发的逆序处理方式，同时记录选择与鼠标上报动作。
    for dispatched in result.actions.into_iter().rev() {
        match dispatched.kind {
            DispatchedActionKind::Legacy { name, arg } => {
                ctx.dispatch_action_for_view(window_id, dispatched.view_id, name, arg.as_ref());
            }
            DispatchedActionKind::Typed(action) => {
                if let Some(terminal_action) =
                    action.as_ref().as_any().downcast_ref::<TerminalAction>()
                {
                    if let TerminalAction::AltSelect(selection) = terminal_action {
                        observed_actions.push(match selection {
                            SelectAction::Begin { .. } => "begin",
                            SelectAction::Update { .. } => "update",
                            SelectAction::End => "end",
                        });
                    } else if matches!(terminal_action, TerminalAction::AltMouseAction(..)) {
                        observed_actions.push("mouse-report");
                    }
                }
                ctx.dispatch_typed_action_for_view(window_id, dispatched.view_id, action.as_ref());
            }
        }
    }
}

fn assert_alt_screen_drag_without_rerender(
    sgr_mouse: bool,
    modifiers: ModifiersState,
    expected_actions: &'static [&'static str],
    expected_text: Option<&'static str>,
) {
    App::test((), move |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);
        let size_info = terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_mode(ansi::Mode::SwapScreen {
                save_cursor_and_clear_screen: true,
            });
            if sgr_mouse {
                model.set_mode(ansi::Mode::SgrMouse);
            }
            assert_eq!(
                should_intercept_mouse(&model, modifiers.shift, ctx),
                expected_text.is_some(),
            );
            // 固定七列夹具，跨自动换行选择 J 到 a，能发现拖动坐标未更新。
            assert_eq!(view.size_info().columns, 7);
            for character in "ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefg".chars() {
                model.alt_screen_mut().input(character);
            }
            *view.size_info()
        });
        let start_position = vec2f(
            2. * size_info.cell_width_px.as_f32(),
            2. * size_info.cell_height_px.as_f32() - 1.,
        );
        let end_position = vec2f(
            5. * size_info.cell_width_px.as_f32(),
            5. * size_info.cell_height_px.as_f32() - 1.,
        );

        app.update(|ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = EntityIdSet::default();
            updated.insert(ctx.root_view_id(window_id).expect("测试根视图应存在"));
            presenter.invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            presenter.build_scene(
                vec2f(size_info.pane_width_px, size_info.pane_height_px),
                1.,
                None,
                ctx,
            );
            let frame_count = presenter.frame_count();
            let mut observed_actions = Vec::new();

            // 三个事件共用同一个已绘制元素，不在事件之间重建选择状态快照。
            for event in [
                Event::LeftMouseDown {
                    position: start_position,
                    modifiers,
                    click_count: 1,
                    is_first_mouse: false,
                },
                Event::LeftMouseDragged {
                    position: end_position,
                    modifiers,
                },
                Event::LeftMouseUp {
                    position: end_position,
                    modifiers,
                },
            ] {
                dispatch_drag_event(&mut presenter, window_id, event, &mut observed_actions, ctx);
            }
            assert_eq!(observed_actions, expected_actions);

            // 松开后仍不重绘，额外拖动和松开不能继续派发选择更新或重复结束。
            let mut after_release = Vec::new();
            for event in [
                Event::LeftMouseDragged {
                    position: start_position,
                    modifiers,
                },
                Event::LeftMouseUp {
                    position: start_position,
                    modifiers,
                },
            ] {
                dispatch_drag_event(&mut presenter, window_id, event, &mut after_release, ctx);
            }
            assert!(after_release.iter().all(|action| *action == "mouse-report"));
            assert_eq!(presenter.frame_count(), frame_count);
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.is_selecting());
            let selected_text = view.model.lock().selection_to_string(
                &SemanticSelection::mock(true, ""),
                false,
                ctx,
            );
            assert_eq!(selected_text.as_deref(), expected_text);
        });
    });
}

#[test]
fn alt_screen_drag_selects_without_intermediate_render() {
    assert_alt_screen_drag_without_rerender(
        false,
        ModifiersState::default(),
        &["begin", "update", "end"],
        Some("JKLMNOPQRSTUVWXYZ[\\]^_`a"),
    );
}

#[test]
fn alt_screen_shift_drag_selects_with_sgr_without_intermediate_render() {
    assert_alt_screen_drag_without_rerender(
        true,
        ModifiersState {
            shift: true,
            ..Default::default()
        },
        &["begin", "update", "end"],
        Some("JKLMNOPQRSTUVWXYZ[\\]^_`a"),
    );
}

#[test]
fn alt_screen_sgr_drag_reports_mouse_without_starting_selection() {
    assert_alt_screen_drag_without_rerender(
        true,
        ModifiersState::default(),
        &["mouse-report", "mouse-report", "mouse-report"],
        None,
    );
}
