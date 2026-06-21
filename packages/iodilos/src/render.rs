//! Async TUI render driver.
//!
//! Owns terminal setup, event intake, reactive dispatch, layout, painting,
//! buffer diffing, and teardown (ADR-0024 §Render Driver). The paint path is
//! crossterm only — there is no ratatui `Terminal` or `Buffer`. After layout
//! and painting produce a [`Canvas`], the driver diffs it against the previous
//! frame's `Canvas` and emits the minimal ANSI writes (ADR-0024 §12).

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::io::{self, Stdout, Write, stdout};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::reactive::{
    NodeHandle, RootHandle, create_root, on_cleanup, provide_context, use_context,
    use_current_scope,
};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, EventStream, KeyCode,
    KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::LocalBoxFuture;
use futures::stream::{FusedStream, FuturesUnordered};
use futures::{FutureExt as _, StreamExt as _, select};

use crate::canvas::{Canvas, Rect};
use crate::events::{Event, EventKind};
use crate::layout::{RuntimeIndex, render as render_buffer};
use crate::node::{NodeId, SharedHandler, TuiNode};
use crate::view::View;

#[derive(Clone)]
struct TuiRuntime {
    task_tx: UnboundedSender<TuiTask>,
    redraw_tx: UnboundedSender<()>,
}

impl TuiRuntime {
    fn request_redraw(&self) {
        let _ = self.redraw_tx.unbounded_send(());
    }
}

struct TuiTask {
    future: LocalBoxFuture<'static, ()>,
}

struct RedrawFuture {
    inner: LocalBoxFuture<'static, ()>,
    runtime: TuiRuntime,
    scope: NodeHandle,
    cancelled: Rc<Cell<bool>>,
}

impl Future for RedrawFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.cancelled.get() {
            return Poll::Ready(());
        }

        let scope = self.scope;
        let poll = scope.run_in(|| self.inner.as_mut().poll(cx));
        self.runtime.request_redraw();
        poll
    }
}

struct RenderState {
    stdout: Stdout,
    prev: Option<Canvas>,
    nodes: Vec<TuiNode>,
    index: RuntimeIndex,
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    mouse_down: Option<NodeId>,
}

/// Start a future tied to the current TUI reactive scope.
///
/// The future is polled by the TUI render loop, not by a specific runtime's
/// task spawner. Dropping the current reactive scope cancels future polling.
#[cfg_attr(debug_assertions, track_caller)]
pub fn use_future(future: impl Future<Output = ()> + 'static) {
    let runtime = use_context::<TuiRuntime>();
    let scope = use_current_scope();
    let cancelled = Rc::new(Cell::new(false));
    on_cleanup({
        let cancelled = Rc::clone(&cancelled);
        move || cancelled.set(true)
    });

    let future = RedrawFuture {
        inner: future.boxed_local(),
        runtime: runtime.clone(),
        scope,
        cancelled,
    }
    .boxed_local();
    let _ = runtime.task_tx.unbounded_send(TuiTask { future });
    runtime.request_redraw();
}

impl RenderState {
    fn redraw(&mut self) -> io::Result<()> {
        let (width, height) = size()?;
        let area = Rect::new(0, 0, width, height);
        let (canvas, index) = render_buffer(&self.nodes, area, self.focused);
        self.index = index;
        if self.focused.is_none_or(|id| !self.index.contains(id)) {
            self.focused = self.index.first_focusable();
        }
        if let Some(prev) = self.prev.take() {
            diff_and_draw(&mut self.stdout, &prev, &canvas)?;
        } else {
            // First frame: paint the whole canvas.
            canvas.write_ansi(&mut self.stdout)?;
        }
        self.prev = Some(canvas);
        self.stdout.flush()?;
        Ok(())
    }

    fn dispatch(&mut self, event: Event) {
        dispatch_event(&self.nodes, &self.index, self.focused, event);
    }

    fn focus(&mut self, next: Option<NodeId>) {
        if self.focused == next {
            return;
        }
        if let Some(old) = self.focused {
            self.dispatch(Event::new(EventKind::Blur).with_target(Some(old)));
        }
        self.focused = next;
        if let Some(new) = self.focused {
            self.dispatch(Event::new(EventKind::Focus).with_target(Some(new)));
        }
    }

    fn focus_next(&mut self, reverse: bool) {
        let next = self.index.next_focus(self.focused, reverse);
        self.focus(next);
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if is_quit_key(key) {
            return false;
        }
        if key.kind == KeyEventKind::Press && key.code == KeyCode::Tab {
            self.focus_next(key.modifiers.contains(KeyModifiers::SHIFT));
            return true;
        }

        let kind = match key.kind {
            KeyEventKind::Press => EventKind::KeyDown,
            KeyEventKind::Release => EventKind::KeyUp,
            _ => EventKind::RawKey,
        };
        let target = self.focused;
        self.dispatch(
            Event::new(EventKind::RawKey)
                .with_target(target)
                .with_key(key),
        );
        self.dispatch(Event::new(kind).with_target(target).with_key(key));

        if key.kind == KeyEventKind::Press
            && let Some(target) = target
        {
            match self.index.tag(target) {
                Some("button") if is_activation_key(key) => {
                    self.dispatch(
                        Event::new(EventKind::Click)
                            .with_target(Some(target))
                            .with_key(key),
                    );
                }
                Some("input") => {
                    if let Some(value) = next_input_value(
                        attribute_value_in_nodes(&self.nodes, target, "value"),
                        key,
                    ) {
                        self.dispatch(
                            Event::new(EventKind::Input)
                                .with_target(Some(target))
                                .with_key(key)
                                .with_input_value(value),
                        );
                    }
                }
                _ => {}
            }
        }
        true
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        let target = self.index.hit_test(mouse.column, mouse.row);
        self.dispatch(
            Event::new(EventKind::RawMouse)
                .with_target(target)
                .with_mouse(mouse),
        );

        if self.hovered != target {
            if let Some(old) = self.hovered {
                self.dispatch(
                    Event::new(EventKind::MouseOut)
                        .with_target(Some(old))
                        .with_mouse(mouse),
                );
            }
            self.hovered = target;
            if let Some(new) = target {
                self.dispatch(
                    Event::new(EventKind::MouseOver)
                        .with_target(Some(new))
                        .with_mouse(mouse),
                );
            }
        }

        match mouse.kind {
            MouseEventKind::Down(_) => {
                self.mouse_down = target;
                self.focus(target);
                self.dispatch(
                    Event::new(EventKind::MouseDown)
                        .with_target(target)
                        .with_mouse(mouse),
                );
            }
            MouseEventKind::Up(_) => {
                self.dispatch(
                    Event::new(EventKind::MouseUp)
                        .with_target(target)
                        .with_mouse(mouse),
                );
                if target.is_some() && target == self.mouse_down {
                    self.dispatch(
                        Event::new(EventKind::Click)
                            .with_target(target)
                            .with_mouse(mouse),
                    );
                }
                self.mouse_down = None;
            }
            MouseEventKind::Drag(_) => {
                self.dispatch(
                    Event::new(EventKind::Drag)
                        .with_target(self.mouse_down.or(target))
                        .with_mouse(mouse),
                );
            }
            MouseEventKind::Moved => {
                self.dispatch(
                    Event::new(EventKind::MouseMove)
                        .with_target(target)
                        .with_mouse(mouse),
                );
            }
            MouseEventKind::ScrollDown
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {}
        }
    }
}

/// Diff `prev` against `next` and emit only the changed cells via crossterm.
///
/// For each cell that differs, move the cursor to that cell and re-emit it
/// with its style deltas. This is the terminal cell-buffer diff from
/// ADR-0024 §12 over the self-built `Canvas` rather than a ratatui `Buffer`.
fn diff_and_draw<W: Write>(w: &mut W, prev: &Canvas, next: &Canvas) -> io::Result<()> {
    use crossterm::csi;
    use crossterm::style::{Color, SetBackgroundColor, SetForegroundColor};

    if prev.size() != next.size() {
        write!(w, csi!("2J"))?;
        return next.write_ansi(w);
    }

    let width = next.width();
    let height = next.height();
    let mut background = None;
    let mut text_style = crate::text::SpanStyle::default();

    for y in 0..height {
        for x in 0..width {
            let prev_cell = prev.cell(x, y);
            let next_cell = next.cell(x, y);
            if prev_cell == next_cell {
                continue;
            }
            let Some(cell) = next_cell else { continue };
            // Position the cursor explicitly at this cell.
            write!(w, csi!("{};{}H"), y + 1, x + 1)?;
            if let Some(ch) = &cell.character {
                let needs_reset = !ch.style.sub_modifier.is_empty()
                    || (ch.style.fg.is_none() && text_style.fg.is_some())
                    || (ch.style.bg.is_none() && text_style.bg.is_some())
                    || (ch.style.underline_color.is_none() && text_style.underline_color.is_some())
                    || (ch.style.add_modifier & !text_style.add_modifier).is_empty()
                        && !text_style.add_modifier.is_empty()
                        && ch.style.add_modifier != text_style.add_modifier;
                if needs_reset {
                    write!(w, csi!("0m"))?;
                    background = None;
                    text_style = crate::text::SpanStyle::default();
                }
                if ch.style.fg != text_style.fg {
                    write!(
                        w,
                        "{}",
                        SetForegroundColor(ch.style.fg.unwrap_or(Color::Reset))
                    )?;
                }
                if ch.style.bg != text_style.bg {
                    write!(
                        w,
                        "{}",
                        SetBackgroundColor(ch.style.bg.unwrap_or(Color::Reset))
                    )?;
                }
                let newly_on = ch.style.add_modifier & !text_style.add_modifier;
                for attr in crate::canvas::modifier_attributes(newly_on) {
                    write!(w, csi!("{}m"), attr.sgr())?;
                }
                text_style = ch.style;
            }
            if cell.background_color != background {
                write!(
                    w,
                    "{}",
                    SetBackgroundColor(cell.background_color.unwrap_or(Color::Reset))
                )?;
                background = cell.background_color;
            }
            if let Some(ch) = &cell.character {
                write!(w, "{}", ch.value)?;
            } else {
                w.write_all(b" ")?;
            }
        }
    }
    write!(w, csi!("0m"))?;
    Ok(())
}

/// Render a TUI view asynchronously and run until `q` or Ctrl-C.
///
/// This function is executor-agnostic: it does not spawn onto tokio, smol, or
/// async-std. Futures registered with [`use_future`] are polled inside this
/// render loop.
pub async fn render_async(view: impl FnOnce() -> View<TuiNode> + 'static) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
    let (task_tx, task_rx) = unbounded();
    let (redraw_tx, redraw_rx) = unbounded();
    let runtime = TuiRuntime { task_tx, redraw_tx };

    let state = Rc::new(RefCell::new(RenderState {
        stdout,
        prev: None,
        nodes: Vec::new(),
        index: RuntimeIndex::default(),
        focused: None,
        hovered: None,
        mouse_down: None,
    }));

    let root = create_root({
        let state = Rc::clone(&state);
        let runtime = runtime.clone();
        move || {
            provide_context(runtime);
            state.borrow_mut().nodes = view().nodes.into_iter().collect();
        }
    });

    let result = run_loop(&state, &root, task_rx, redraw_rx).await;
    root.dispose();

    {
        let mut state = state.borrow_mut();
        disable_raw_mode()?;
        execute!(
            state.stdout,
            LeaveAlternateScreen,
            DisableMouseCapture,
            Show
        )?;
    }

    result
}

/// Render a TUI view using a small local executor and run until `q` or Ctrl-C.
///
/// Applications that already own an async runtime can call [`render_async`]
/// instead.
pub fn render(view: impl FnOnce() -> View<TuiNode> + 'static) -> io::Result<()> {
    let mut pool = futures::executor::LocalPool::new();
    pool.run_until(render_async(view))
}

async fn run_loop(
    state: &Rc<RefCell<RenderState>>,
    root: &RootHandle,
    task_rx: UnboundedReceiver<TuiTask>,
    redraw_rx: UnboundedReceiver<()>,
) -> io::Result<()> {
    state.borrow_mut().redraw()?;
    let mut events = EventStream::new().fuse();
    let mut task_rx = task_rx.fuse();
    let mut redraw_rx = redraw_rx.fuse();
    let mut tasks = FuturesUnordered::new();

    loop {
        select! {
            event = events.next() => {
                let Some(event) = event else { break };
                let event = event?;
                let keep_running = root.run_in(|| handle_terminal_event(state, event));
                if !keep_running {
                    break;
                }
                state.borrow_mut().redraw()?;
            }
            task = task_rx.next() => {
                if let Some(task) = task {
                    tasks.push(task.future);
                } else if tasks.is_empty() && redraw_rx.is_terminated() && events.is_terminated() {
                    break;
                }
            }
            _ = redraw_rx.next() => {
                state.borrow_mut().redraw()?;
            }
            _ = tasks.next() => {
                state.borrow_mut().redraw()?;
            }
        }
    }
    Ok(())
}

fn handle_terminal_event(state: &Rc<RefCell<RenderState>>, event: CrosstermEvent) -> bool {
    match event {
        CrosstermEvent::Key(key) => state.borrow_mut().handle_key(key),
        CrosstermEvent::Mouse(mouse) => {
            state.borrow_mut().handle_mouse(mouse);
            true
        }
        CrosstermEvent::Resize(columns, rows) => {
            state.borrow_mut().dispatch(
                Event::new(EventKind::TerminalResize)
                    .with_target(None)
                    .with_resize(columns, rows),
            );
            true
        }
        CrosstermEvent::FocusGained | CrosstermEvent::FocusLost | CrosstermEvent::Paste(_) => true,
    }
}

fn dispatch_event(nodes: &[TuiNode], index: &RuntimeIndex, focused: Option<NodeId>, event: Event) {
    let target = event.target().or(focused);
    let path = target
        .map(|target| index.path_to_root(target))
        .unwrap_or_default();
    if path.is_empty() {
        return;
    }
    for id in path {
        event.set_current_target(Some(id));
        if let Some(handlers) = event_handlers_in_nodes(nodes, id, event.kind().name()) {
            for handler in handlers {
                handler.borrow_mut()(&event);
                if event.propagation_stopped() {
                    break;
                }
            }
            if event.propagation_stopped() {
                break;
            }
        }
    }
    event.set_current_target(None);
}

fn event_handlers_in_nodes(
    nodes: &[TuiNode],
    id: NodeId,
    name: &str,
) -> Option<Vec<SharedHandler>> {
    for node in nodes {
        if node.id() == id {
            return Some(node.event_handlers(name));
        }
        match node {
            TuiNode::Element(element) => {
                if let Some(handlers) = event_handlers_in_nodes(&element.children, id, name) {
                    return Some(handlers);
                }
            }
            TuiNode::Dynamic { view, .. } => {
                let view = view.borrow();
                if let Some(handlers) = event_handlers_in_nodes(view.nodes.as_slice(), id, name) {
                    return Some(handlers);
                }
            }
            _ => {}
        }
    }
    None
}

fn attribute_value_in_nodes(nodes: &[TuiNode], id: NodeId, name: &str) -> Option<String> {
    for node in nodes {
        if node.id() == id {
            return node.attribute_value(name);
        }
        match node {
            TuiNode::Element(element) => {
                if let Some(value) = attribute_value_in_nodes(&element.children, id, name) {
                    return Some(value);
                }
            }
            TuiNode::Dynamic { view, .. } => {
                let view = view.borrow();
                if let Some(value) = attribute_value_in_nodes(view.nodes.as_slice(), id, name) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_quit_key(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && (matches!(key.code, KeyCode::Char('q'))
            || matches!(key.code, KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL)))
}

fn is_activation_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Enter | KeyCode::Char(' '))
}

fn next_input_value(value: Option<String>, key: KeyEvent) -> Option<String> {
    let mut value = value.unwrap_or_default();
    match key.code {
        KeyCode::Char(ch) => {
            value.push(ch);
            Some(value)
        }
        KeyCode::Backspace => {
            value.pop();
            Some(value)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::reactive::{Signal, create_root, create_signal};

    use super::*;
    use crate::attributes::{GlobalAttributes, GlobalAttributesExt};
    use crate::canvas::{Canvas, Rect};
    use crate::components::tags;
    use crate::layout::render as render_buffer;
    use crate::text::SpanStyle;
    use crate::view::View;
    use crate::{Color, bind, events};

    #[test]
    fn use_future_registers_task_with_tui_runtime() {
        let (task_tx, mut task_rx) = unbounded();
        let (redraw_tx, _redraw_rx) = unbounded();
        let runtime = TuiRuntime { task_tx, redraw_tx };

        let root = create_root(|| {
            provide_context(runtime);
            use_future(async {});
        });

        let task = futures::executor::block_on(task_rx.next());
        assert!(task.is_some(), "use_future should register a TUI task");
        root.dispose();
    }

    #[test]
    fn click_dispatch_reaches_dynamic_subtrees_and_bubbles() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let dynamic_calls = Rc::clone(&calls);
            let view: View = tags::div()
                .on(events::click, {
                    let calls = Rc::clone(&calls);
                    move |_| calls.borrow_mut().push("parent")
                })
                .children(View::from_dynamic(move || {
                    View::from(
                        tags::button()
                            .on(events::click, {
                                let calls = Rc::clone(&dynamic_calls);
                                move |_| calls.borrow_mut().push("child")
                            })
                            .children("Run"),
                    )
                }))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_buffer, index) = render_buffer(&nodes, Rect::new(0, 0, 20, 5), None);
        let target = index
            .focus_order
            .first()
            .copied()
            .expect("dynamic button should be focusable");

        dispatch_event(
            &nodes,
            &index,
            None,
            Event::new(EventKind::Click).with_target(Some(target)),
        );

        assert_eq!(&*calls.borrow(), &["child", "parent"]);
        root.dispose();
    }

    #[test]
    fn stop_propagation_prevents_bubbling() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::div()
                .on(events::click, {
                    let calls = Rc::clone(&calls);
                    move |_| calls.borrow_mut().push("parent")
                })
                .children(
                    tags::button()
                        .on(events::click, {
                            let calls = Rc::clone(&calls);
                            move |event: Event| {
                                calls.borrow_mut().push("child");
                                event.stop_propagation();
                            }
                        })
                        .children("Run"),
                )
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_buffer, index) = render_buffer(&nodes, Rect::new(0, 0, 20, 5), None);
        let target = index
            .focus_order
            .first()
            .copied()
            .expect("button should be focusable");

        dispatch_event(
            &nodes,
            &index,
            None,
            Event::new(EventKind::Click).with_target(Some(target)),
        );

        assert_eq!(&*calls.borrow(), &["child"]);
        root.dispose();
    }

    #[test]
    fn dynamic_handler_can_update_signal_that_rebuilds_dynamic_subtree() {
        let mut nodes = Vec::new();
        let mut show_signal: Option<Signal<bool>> = None;

        let root = create_root(|| {
            let show = create_signal(true);
            show_signal = Some(show);
            let view: View = tags::div()
                .children(View::from_dynamic(move || {
                    if show.get() {
                        View::from(
                            tags::button()
                                .on(events::click, move |_| show.set(false))
                                .children("Hide"),
                        )
                    } else {
                        View::from(tags::p().children("Hidden"))
                    }
                }))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_buffer, index) = render_buffer(&nodes, Rect::new(0, 0, 20, 5), None);
        let target = index
            .focus_order
            .first()
            .copied()
            .expect("dynamic button should be focusable");

        dispatch_event(
            &nodes,
            &index,
            None,
            Event::new(EventKind::Click).with_target(Some(target)),
        );

        assert!(!show_signal.expect("signal should be captured").get());
        let (_buffer, index) = render_buffer(&nodes, Rect::new(0, 0, 20, 5), None);
        assert!(index.focus_order.is_empty());
        root.dispose();
    }

    #[test]
    fn bind_value_updates_signal_from_input_event() {
        let mut nodes = Vec::new();
        let mut value: Option<Signal<String>> = None;

        let root = create_root(|| {
            let text = create_signal(String::from("a"));
            value = Some(text);
            let view: View = tags::input().bind(bind::value, text).into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_buffer, index) = render_buffer(&nodes, Rect::new(0, 0, 20, 5), None);
        let target = index
            .focus_order
            .first()
            .copied()
            .expect("input should be focusable");

        dispatch_event(
            &nodes,
            &index,
            None,
            Event::new(EventKind::Input)
                .with_target(Some(target))
                .with_input_value(String::from("abc")),
        );

        assert_eq!(value.expect("signal should be captured").get_clone(), "abc");
        root.dispose();
    }

    #[test]
    fn diff_repaints_whole_canvas_when_size_changes() {
        let prev = Canvas::empty(Rect::new(0, 0, 4, 2));
        let next = Canvas::empty(Rect::new(0, 0, 2, 1));
        let mut out = Vec::new();

        diff_and_draw(&mut out, &prev, &next).unwrap();
        let output = String::from_utf8_lossy(&out);

        assert!(
            output.contains("\x1b[2J"),
            "resized canvas should clear before full repaint: {output:?}"
        );
    }

    #[test]
    fn diff_does_not_inherit_style_from_skipped_unchanged_cells() {
        crossterm::style::force_color_output(true);

        let red = SpanStyle {
            fg: Some(Color::Red),
            ..SpanStyle::default()
        };
        let mut prev = Canvas::empty(Rect::new(0, 0, 2, 1));
        prev.set_text(Rect::new(0, 0, 1, 1), "a", red);
        let mut next = Canvas::empty(Rect::new(0, 0, 2, 1));
        next.set_text(Rect::new(0, 0, 1, 1), "a", red);
        next.set_text(Rect::new(1, 0, 1, 1), "b", red);

        let mut out = Vec::new();
        diff_and_draw(&mut out, &prev, &next).unwrap();
        let output = String::from_utf8_lossy(&out);

        assert!(
            output.contains("38;5;9m"),
            "changed red cell must emit its own foreground color: {output:?}"
        );
    }

    /// Acceptance test for the crossterm-without-ratatui paint path: a
    /// tui-counter-shaped view (flat style properties, rounded border, dynamic
    /// text) lays out and paints into the self-built `Canvas`, and the painted
    /// output contains the counter label and button text.
    #[test]
    fn flat_style_view_paints_into_canvas() {
        use crossterm::style::Color;
        use taffy::style::FlexDirection;

        use crate::style::BorderStyle;

        let mut nodes = Vec::new();
        let mut count_signal: Option<Signal<i32>> = None;

        let root = create_root(|| {
            let count = create_signal(7i32);
            count_signal = Some(count);
            // Mirrors tui-counter: a column panel with border, a label, and a
            // row of buttons — all authored with flat style properties.
            let view: View = tags::div()
                .flex_direction(FlexDirection::Column)
                .padding(1)
                .gap(1)
                .border_style(BorderStyle::Single)
                .children(vec![
                    View::from(
                        tags::p()
                            .color(Color::Cyan)
                            .children("Count: ")
                            .children(count),
                    ),
                    View::from(
                        tags::div()
                            .flex_direction(FlexDirection::Row)
                            .gap(1)
                            .children(tags::button().children("-")),
                    ),
                ])
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (canvas, _index) = render_buffer(&nodes, Rect::new(0, 0, 24, 8), None);
        let painted = canvas_to_plain_text(&canvas);

        // The dynamic counter value (7) is painted into the self-built Canvas,
        // the label appears inside the padded border, and the flat
        // `border_style` property draws a complete box.
        assert!(painted.contains("Count:"), "label painted: {painted}");
        assert!(painted.contains('7'), "count painted: {painted}");
        assert!(
            painted.contains("[ - ]"),
            "button chrome should not be clipped: {painted}"
        );
        assert!(
            painted.contains('┌'),
            "top-left border corner drawn: {painted}"
        );
        assert!(
            painted.contains('┐'),
            "top-right border corner drawn: {painted}"
        );
        root.dispose();
    }

    /// Flatten a `Canvas` to plain text for assertions.
    fn canvas_to_plain_text(canvas: &Canvas) -> String {
        let mut out = String::new();
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                if let Some(cell) = canvas.cell(x, y)
                    && let Some(ch) = &cell.character
                {
                    out.push_str(&ch.value);
                } else {
                    out.push(' ');
                }
            }
            out.push('\n');
        }
        out
    }
}
