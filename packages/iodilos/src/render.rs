//! Async TUI render driver.
//!
//! Owns terminal setup, event intake, reactive dispatch, layout, painting,
//! buffer diffing, and teardown (ADR-0024 §Render Driver). The paint path is
//! crossterm only — there is no ratatui `Terminal` or `Buffer`. After layout
//! and painting produce a [`Framebuffer`], the driver diffs it against the previous
//! frame's `Framebuffer` and emits the minimal ANSI writes (ADR-0024 §12).

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
    KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::LocalBoxFuture;
use futures::stream::{FusedStream, FuturesUnordered};
use futures::{FutureExt as _, StreamExt as _, select};

use crate::framebuffer::{Framebuffer, Rect};
use crate::events::{Event, EventKind};
use crate::layout::{RuntimeIndex, render as render_buffer};
use crate::node::{NodeId, SharedHandler, TuiNode};
use crate::view::View;

#[derive(Clone)]
struct TuiRuntime {
    task_tx: UnboundedSender<TuiTask>,
    redraw_tx: UnboundedSender<()>,
    quit_tx: UnboundedSender<()>,
}

impl TuiRuntime {
    fn request_redraw(&self) {
        let _ = self.redraw_tx.unbounded_send(());
    }

    fn quit(&self) {
        let _ = self.quit_tx.unbounded_send(());
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
    prev: Option<Framebuffer>,
    nodes: Vec<TuiNode>,
    index: RuntimeIndex,
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    mouse_down: Option<NodeId>,
    quit: QuitPolicy,
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

/// Request the active render loop to exit.
pub fn quit() {
    use_context::<TuiRuntime>().quit();
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

    fn dispatch(&mut self, event: Event) -> bool {
        dispatch_event(&self.nodes, &self.index, self.focused, event)
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
        if is_quit_key(key, self.quit) {
            return false;
        }
        let kind = match key.kind {
            KeyEventKind::Press => EventKind::KeyDown,
            KeyEventKind::Release => EventKind::KeyUp,
            _ => EventKind::RawKey,
        };
        let target = self.focused;
        let raw_stopped = self.dispatch(
            Event::new(EventKind::RawKey)
                .with_target(target)
                .with_key(key),
        );
        if raw_stopped {
            return true;
        }
        if key.kind == KeyEventKind::Press && key.code == KeyCode::Tab {
            self.focus_next(key.modifiers.contains(KeyModifiers::SHIFT));
            return true;
        }
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

/// Diff `prev` against `next` and emit only the changed rows.
///
/// This is the in-window row diff (ADR-0024 §12), ported from oh-my-pi's
/// differential-rendering strategy: each screen row is compared as a whole,
/// and the contiguous envelope of changed rows `[first_changed, last_changed]`
/// is rewritten in place. Within that envelope the cursor is positioned once
/// at the first changed row (column 0) and successive rows are separated by
/// `\r\n` — no per-cell absolute cursor addressing. Each rewritten row is an
/// independent, reset-terminated unit produced by [`Framebuffer::render_row`],
/// so SGR state never bleeds across rows and a row that lost an inline-code or
/// panel background resets it before drawing its first glyph.
///
/// On a size mismatch the previous frame no longer describes the screen, so
/// the whole framebuffer is cleared (`CSI 2J`) and repainted via
/// [`Framebuffer::write_ansi`].
fn diff_and_draw<W: Write>(w: &mut W, prev: &Framebuffer, next: &Framebuffer) -> io::Result<()> {
    use crossterm::csi;

    if prev.size() != next.size() {
        write!(w, csi!("2J"))?;
        return next.write_ansi(w);
    }

    let height = next.height();
    let width = next.width();
    // Find the inclusive envelope of rows that differ. A row differs if any of
    // its cells changed (compared as a whole slice, so a wide glyph that now
    // spans the boundary still marks its row changed).
    let mut first_changed: Option<i32> = None;
    let mut last_changed = 0i32;
    for y in 0..height {
        let y = y as i32;
        if prev.row(y) != next.row(y) {
            first_changed.get_or_insert(y);
            last_changed = y;
        }
    }

    let Some(first) = first_changed else {
        // Nothing changed: no cursor move, no paint. This is the cheapest frame.
        return Ok(());
    };

    // Position the cursor once at the first changed row, column 1, then write
    // every row in the envelope. Successive rows are separated by `\r\n` (CR
    // resets the column, LF moves down a row) — the same contiguous-rewrite
    // scheme oh-my-pi uses, which avoids re-addressing the cursor per row.
    write!(w, csi!("{};1H"), first + 1)?;
    for y in first..=last_changed {
        if y > first {
            w.write_all(b"\r\n")?;
        }
        w.write_all(crate::framebuffer::render_row(next.row(y), width).as_bytes())?;
    }
    write!(w, csi!("0m"))?;
    Ok(())
}

/// Optional configuration for [`render_async_with`] / [`render_with`]. Defaults
/// reproduce the historical behaviour of [`render`] / [`render_async`].
#[derive(Debug, Clone, Default)]
pub struct RenderConfig {
    /// Push the kitty keyboard protocol (CSI u, `DISAMBIGUATE_ESCAPE_KEYS`) so
    /// modifier-bearing keys (Shift+Enter, Alt+Enter, …) are reported
    /// distinctly. No-op on terminals without support; only truly effective on
    /// Unix. Pushed after entering the alt screen and popped before leaving.
    pub keyboard_enhancement: bool,
    /// Which keys quit the render loop. The built-in `'q'` quit is unsuitable
    /// for text-input apps (it fires before `raw_key` dispatch and cannot be
    /// stopped via `stop_propagation`); switch to [`QuitPolicy::CtrlCOnly`].
    pub quit: QuitPolicy,
}

/// Quit-key policy for the render loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QuitPolicy {
    /// Quit on plain `'q'` or `Ctrl+C` (the historical default).
    #[default]
    QOrCtrlC,
    /// Quit only on `Ctrl+C` (lets the app type `'q'`).
    CtrlCOnly,
    /// Never auto-quit; the app handles its own exit.
    None,
}

/// Render a TUI view asynchronously and run until `q` or Ctrl-C.
///
/// This function is executor-agnostic: it does not spawn onto tokio, smol, or
/// async-std. Futures registered with [`use_future`] are polled inside this
/// render loop.
pub async fn render_async(view: impl FnOnce() -> View<TuiNode> + 'static) -> io::Result<()> {
    render_async_with(view, RenderConfig::default()).await
}

/// Like [`render_async`] but with optional [`RenderConfig`] (kitty keyboard
/// protocol, quit-key policy).
pub async fn render_async_with(
    view: impl FnOnce() -> View<TuiNode> + 'static,
    cfg: RenderConfig,
) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
    if cfg.keyboard_enhancement {
        // No-op on unsupported terminals/platforms; swallow the error.
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let (task_tx, task_rx) = unbounded();
    let (redraw_tx, redraw_rx) = unbounded();
    let (quit_tx, quit_rx) = unbounded();
    let runtime = TuiRuntime {
        task_tx,
        redraw_tx,
        quit_tx,
    };

    let state = Rc::new(RefCell::new(RenderState {
        stdout,
        prev: None,
        nodes: Vec::new(),
        index: RuntimeIndex::default(),
        focused: None,
        hovered: None,
        mouse_down: None,
        quit: cfg.quit,
    }));

    let root = create_root({
        let state = Rc::clone(&state);
        let runtime = runtime.clone();
        move || {
            provide_context(runtime);
            state.borrow_mut().nodes = view().nodes.into_iter().collect();
        }
    });

    let result = run_loop(&state, &root, task_rx, redraw_rx, quit_rx).await;
    root.dispose();

    {
        let mut state = state.borrow_mut();
        if cfg.keyboard_enhancement {
            let _ = execute!(state.stdout, PopKeyboardEnhancementFlags);
        }
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

/// Like [`render`] but with optional [`RenderConfig`].
pub fn render_with(
    view: impl FnOnce() -> View<TuiNode> + 'static,
    cfg: RenderConfig,
) -> io::Result<()> {
    let mut pool = futures::executor::LocalPool::new();
    pool.run_until(render_async_with(view, cfg))
}

async fn run_loop(
    state: &Rc<RefCell<RenderState>>,
    root: &RootHandle,
    task_rx: UnboundedReceiver<TuiTask>,
    redraw_rx: UnboundedReceiver<()>,
    quit_rx: UnboundedReceiver<()>,
) -> io::Result<()> {
    state.borrow_mut().redraw()?;
    let mut events = EventStream::new().fuse();
    let mut task_rx = task_rx.fuse();
    let mut redraw_rx = redraw_rx.fuse();
    let mut quit_rx = quit_rx.fuse();
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
            _ = quit_rx.next() => {
                break;
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

fn dispatch_event(
    nodes: &[TuiNode],
    index: &RuntimeIndex,
    focused: Option<NodeId>,
    event: Event,
) -> bool {
    let target = event.target().or(focused);
    let path = target
        .map(|target| index.path_to_root(target))
        .unwrap_or_default();
    if path.is_empty() {
        return false;
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
    let stopped = event.propagation_stopped();
    event.set_current_target(None);
    stopped
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
            TuiNode::Marker {
                slot: Some(content), ..
            } => {
                if let Some(handlers) = event_handlers_in_nodes(&*content.borrow(), id, name) {
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
            TuiNode::Marker {
                slot: Some(content), ..
            } => {
                if let Some(value) = attribute_value_in_nodes(&*content.borrow(), id, name) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_quit_key(key: KeyEvent, policy: QuitPolicy) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }
    let ctrl_c =
        matches!(key.code, KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL));
    match policy {
        QuitPolicy::QOrCtrlC => matches!(key.code, KeyCode::Char('q')) || ctrl_c,
        QuitPolicy::CtrlCOnly => ctrl_c,
        QuitPolicy::None => false,
    }
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
    use crate::framebuffer::{Framebuffer, Rect};
    use crate::components::tags;
    use crate::layout::render as render_buffer;
    use crate::text::SpanStyle;
    use crate::view::View;
    use crate::{Color, bind, events};

    #[test]
    fn use_future_registers_task_with_tui_runtime() {
        let (task_tx, mut task_rx) = unbounded();
        let (redraw_tx, _redraw_rx) = unbounded();
        let (quit_tx, _quit_rx) = unbounded();
        let runtime = TuiRuntime {
            task_tx,
            redraw_tx,
            quit_tx,
        };

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
    fn raw_key_stop_propagation_prevents_tab_focus_change() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::div()
                .on(events::raw_key, {
                    let calls = Rc::clone(&calls);
                    move |event: Event| {
                        calls.borrow_mut().push("raw");
                        if event.key().is_some_and(|key| key.code == KeyCode::Tab) {
                            event.stop_propagation();
                        }
                    }
                })
                .children((
                    tags::button().children("First"),
                    tags::button().children("Second"),
                ))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_buffer, index) = render_buffer(&nodes, Rect::new(0, 0, 30, 5), None);
        let first = index.focus_order[0];
        let second = index.focus_order[1];
        let mut state = RenderState {
            stdout: stdout(),
            prev: None,
            nodes,
            index,
            focused: Some(first),
            hovered: None,
            mouse_down: None,
            quit: QuitPolicy::None,
        };

        assert!(state.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));

        assert_eq!(&*calls.borrow(), &["raw"]);
        assert_eq!(state.focused, Some(first));
        assert_ne!(state.focused, Some(second));
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
        let prev = Framebuffer::empty(Rect::new(0, 0, 4, 2));
        let next = Framebuffer::empty(Rect::new(0, 0, 2, 1));
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
        let mut prev = Framebuffer::empty(Rect::new(0, 0, 2, 1));
        prev.set_text(Rect::new(0, 0, 1, 1), "a", red);
        let mut next = Framebuffer::empty(Rect::new(0, 0, 2, 1));
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

    #[test]
    fn diff_skips_wide_char_trailing_cell() {
        let mut prev = Framebuffer::empty(Rect::new(0, 0, 4, 1));
        prev.set_text(Rect::new(0, 0, 4, 1), "  XY", SpanStyle::default());
        let mut next = Framebuffer::empty(Rect::new(0, 0, 4, 1));
        next.set_text(Rect::new(0, 0, 4, 1), "好XY", SpanStyle::default());

        let mut out = Vec::new();
        diff_and_draw(&mut out, &prev, &next).unwrap();
        let output = String::from_utf8(out).unwrap();
        let idx = output.find("好").expect("wide glyph emitted");
        let after = &output[idx + "好".len()..];

        assert!(
            !after.starts_with("\x1b[1;2H "),
            "diff repainted the wide glyph's trailing cell as a space: {output:?}"
        );
    }

    /// Regression: a cell that previously carried an inline-code background
    /// (via `character.style.bg`) must have that background RESET when the next
    /// frame paints a plain cell on top of it. The diff's per-cell equality
    /// short-circuit must not skip the background change.
    ///
    /// Scenario: row 0 holds `code` with `bg=DarkGrey`; after scrolling, the
    /// same on-screen cell becomes plain text `code` with no background. The
    /// diff must emit `SetBackgroundColor(Reset)` for that cell, otherwise the
    /// DarkGrey block lingers on screen.
    #[test]
    fn diff_resets_character_bg_when_cell_changes_to_plain() {
        use crossterm::style::Color;
        crossterm::style::force_color_output(true);

        let grey = SpanStyle {
            bg: Some(Color::DarkGrey),
            ..SpanStyle::default()
        };
        let mut prev = Framebuffer::empty(Rect::new(0, 0, 6, 1));
        // "  code  " with DarkGrey background (mirrors inline-code rendering).
        prev.set_text(Rect::new(0, 0, 6, 1), " code ", grey);

        let mut next = Framebuffer::empty(Rect::new(0, 0, 6, 1));
        // After scroll: the same on-screen cells now hold plain text, no bg.
        next.set_text(Rect::new(0, 0, 6, 1), "plain", SpanStyle::default());

        let mut out = Vec::new();
        diff_and_draw(&mut out, &prev, &next).unwrap();
        let output = String::from_utf8_lossy(&out);

        // The first changed cell is at (0,0) → cursor-move `\x1b[1;1H`. For the
        // DarkGrey to be cleared, that move must be followed by a SGR that
        // resets/changes the background BEFORE the character is drawn. The bug
        // emits `\x1b[1;1Hp` (move straight to the char), leaving Grey on screen.
        let first_cell = output.split('p').next().unwrap_or("");
        assert!(
            first_cell.contains("0m") || first_cell.contains("49m") || first_cell.contains("48;"),
            "first changed cell must reset/change the lingering bg before its char: {output:?}"
        );
    }

    /// Companion regression: a cell whose previous frame set an opaque
    /// `background_color` (a styled panel, not an inline-code span) must also be
    /// reset when the next frame paints a default-bg cell on top of it. Covers
    /// the second of the two background channels.
    #[test]
    fn diff_resets_node_background_color_when_cell_changes_to_plain() {
        use crossterm::style::Color;
        crossterm::style::force_color_output(true);

        let mut prev = Framebuffer::empty(Rect::new(0, 0, 3, 1));
        // A panel painted an opaque background, then plain text on top.
        prev.set_background_color(Rect::new(0, 0, 3, 1), Color::Blue);
        prev.set_text(Rect::new(0, 0, 3, 1), "abc", SpanStyle::default());

        let mut next = Framebuffer::empty(Rect::new(0, 0, 3, 1));
        next.set_text(Rect::new(0, 0, 3, 1), "xyz", SpanStyle::default());

        let mut out = Vec::new();
        diff_and_draw(&mut out, &prev, &next).unwrap();
        let output = String::from_utf8_lossy(&out);

        // First changed cell at (0,0) → the Blue bg must be reset before the
        // first char `x` is drawn.
        let first_cell = output.split('x').next().unwrap_or("");
        assert!(
            first_cell.contains("0m") || first_cell.contains("49m") || first_cell.contains("48;"),
            "first changed cell must reset the node bg before its char: {output:?}"
        );
    }

    /// When two frames are byte-identical the row diff finds no changed row and
    /// emits nothing — no cursor move, no paint. This is the cheapest frame and
    /// must stay a true no-op.
    #[test]
    fn diff_emits_nothing_when_frames_are_identical() {
        let mut prev = Framebuffer::empty(Rect::new(0, 0, 4, 2));
        prev.set_text(Rect::new(0, 0, 4, 1), "abcd", SpanStyle::default());
        let next = prev.clone();

        let mut out = Vec::new();
        diff_and_draw(&mut out, &prev, &next).unwrap();

        assert!(out.is_empty(), "identical frames must emit nothing: {out:?}");
    }

    /// The diff rewrites only the contiguous envelope of changed rows. A change
    /// in the middle row must position the cursor once at that row (not at row
    /// 1) and must NOT touch the unchanged rows above or below.
    #[test]
    fn diff_rewrites_only_the_changed_row_envelope() {
        let mut prev = Framebuffer::empty(Rect::new(0, 0, 3, 3));
        prev.set_text(Rect::new(0, 0, 3, 1), "aaa", SpanStyle::default());
        prev.set_text(Rect::new(0, 1, 3, 1), "bbb", SpanStyle::default());
        prev.set_text(Rect::new(0, 2, 3, 1), "ccc", SpanStyle::default());

        let mut next = prev.clone();
        // Only the middle row changes: "bbb" -> "bZb".
        next.set_text(Rect::new(1, 1, 1, 1), "Z", SpanStyle::default());

        let mut out = Vec::new();
        diff_and_draw(&mut out, &prev, &next).unwrap();
        let output = String::from_utf8_lossy(&out);

        // The cursor must be addressed to row 2 (the changed row), not row 1.
        assert!(
            output.contains("\x1b[2;1H"),
            "diff should address the cursor to the changed row 2: {output:?}"
        );
        // The unchanged first row's cursor address (row 1) must NOT appear, and
        // neither should the unchanged last row's (row 3).
        assert!(
            !output.contains("\x1b[1;1H"),
            "diff should not rewrite unchanged row 1: {output:?}"
        );
        assert!(
            !output.contains("\x1b[3;1H"),
            "diff should not rewrite unchanged row 3: {output:?}"
        );
        // The changed glyph must be emitted exactly once.
        assert_eq!(output.matches('Z').count(), 1, "Z emitted once: {output:?}");
    }

    /// Acceptance test for the crossterm-without-ratatui paint path: a
    /// tui-counter-shaped view (flat style properties, rounded border, dynamic
    /// text) lays out and paints into the self-built `Framebuffer`, and the painted
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

        // The dynamic counter value (7) is painted into the self-built Framebuffer,
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

    /// Flatten a `Framebuffer` to plain text for assertions.
    fn canvas_to_plain_text(canvas: &Framebuffer) -> String {
        let mut out = String::new();
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                if let Some(cell) = canvas.cell(x as i32, y as i32)
                    && let Some(ch) = &cell.glyph
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

    #[test]
    fn render_config_defaults_match_legacy_behaviour() {
        let cfg = RenderConfig::default();
        assert!(!cfg.keyboard_enhancement);
        assert_eq!(cfg.quit, QuitPolicy::QOrCtrlC);
    }

    #[test]
    fn quit_policy_respects_setting() {
        // Legacy: plain 'q' quits.
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty());
        assert!(is_quit_key(q, QuitPolicy::QOrCtrlC));
        assert!(!is_quit_key(q, QuitPolicy::CtrlCOnly));
        assert!(!is_quit_key(q, QuitPolicy::None));
        // Ctrl+C always quits (except None).
        let ctrlc = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_quit_key(ctrlc, QuitPolicy::QOrCtrlC));
        assert!(is_quit_key(ctrlc, QuitPolicy::CtrlCOnly));
        assert!(!is_quit_key(ctrlc, QuitPolicy::None));
    }
}
