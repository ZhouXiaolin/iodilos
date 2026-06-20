//! HTML-like events for terminal input.

use std::cell::Cell;
use std::rc::Rc;

use crossterm::event::{KeyEvent, MouseEvent};

use crate::node::NodeId;

/// Event kind names used by `on:*` handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Click,
    Input,
    KeyDown,
    KeyUp,
    Focus,
    Blur,
    MouseDown,
    MouseUp,
    MouseMove,
    MouseOver,
    MouseOut,
    Drag,
    TerminalResize,
    RawKey,
    RawMouse,
}

impl EventKind {
    pub const fn name(self) -> &'static str {
        match self {
            EventKind::Click => "click",
            EventKind::Input => "input",
            EventKind::KeyDown => "keydown",
            EventKind::KeyUp => "keyup",
            EventKind::Focus => "focus",
            EventKind::Blur => "blur",
            EventKind::MouseDown => "mousedown",
            EventKind::MouseUp => "mouseup",
            EventKind::MouseMove => "mousemove",
            EventKind::MouseOver => "mouseover",
            EventKind::MouseOut => "mouseout",
            EventKind::Drag => "drag",
            EventKind::TerminalResize => "terminal_resize",
            EventKind::RawKey => "raw_key",
            EventKind::RawMouse => "raw_mouse",
        }
    }
}

#[derive(Debug)]
struct EventData {
    kind: EventKind,
    target: Option<NodeId>,
    current_target: Cell<Option<NodeId>>,
    propagation_stopped: Cell<bool>,
    key: Option<KeyEvent>,
    mouse: Option<MouseEvent>,
    input_value: Option<String>,
    resize: Option<(u16, u16)>,
}

/// Event object delivered to handlers.
#[derive(Debug, Clone)]
pub struct Event(Rc<EventData>);

impl Event {
    pub fn new(kind: EventKind) -> Self {
        Self(Rc::new(EventData {
            kind,
            target: None,
            current_target: Cell::new(None),
            propagation_stopped: Cell::new(false),
            key: None,
            mouse: None,
            input_value: None,
            resize: None,
        }))
    }

    pub(crate) fn with_target(mut self, target: Option<NodeId>) -> Self {
        Rc::get_mut(&mut self.0)
            .expect("new event should be unique")
            .target = target;
        self
    }

    pub(crate) fn with_key(mut self, key: KeyEvent) -> Self {
        Rc::get_mut(&mut self.0)
            .expect("new event should be unique")
            .key = Some(key);
        self
    }

    pub(crate) fn with_mouse(mut self, mouse: MouseEvent) -> Self {
        Rc::get_mut(&mut self.0)
            .expect("new event should be unique")
            .mouse = Some(mouse);
        self
    }

    pub(crate) fn with_input_value(mut self, value: String) -> Self {
        Rc::get_mut(&mut self.0)
            .expect("new event should be unique")
            .input_value = Some(value);
        self
    }

    pub(crate) fn with_resize(mut self, columns: u16, rows: u16) -> Self {
        Rc::get_mut(&mut self.0)
            .expect("new event should be unique")
            .resize = Some((columns, rows));
        self
    }

    pub fn kind(&self) -> EventKind {
        self.0.kind
    }

    pub fn target(&self) -> Option<NodeId> {
        self.0.target
    }

    pub fn current_target(&self) -> Option<NodeId> {
        self.0.current_target.get()
    }

    pub(crate) fn set_current_target(&self, current_target: Option<NodeId>) {
        self.0.current_target.set(current_target);
    }

    pub fn key(&self) -> Option<&KeyEvent> {
        self.0.key.as_ref()
    }

    pub fn mouse(&self) -> Option<&MouseEvent> {
        self.0.mouse.as_ref()
    }

    pub fn input_value(&self) -> Option<&str> {
        self.0.input_value.as_deref()
    }

    pub fn resize(&self) -> Option<(u16, u16)> {
        self.0.resize
    }

    pub fn stop_propagation(&self) {
        self.0.propagation_stopped.set(true);
    }

    pub fn propagation_stopped(&self) -> bool {
        self.0.propagation_stopped.get()
    }
}

/// Description of an event type.
pub trait EventDescriptor {
    type EventTy: Clone;
    const KIND: EventKind;
    const NAME: &'static str;

    fn extract(event: &Event) -> Option<Self::EventTy>;
}

macro_rules! impl_event {
    ($name:ident, $kind:expr) => {
        #[allow(non_camel_case_types)]
        pub struct $name;

        impl EventDescriptor for $name {
            type EventTy = Event;
            const KIND: EventKind = $kind;
            const NAME: &'static str = $kind.name();

            fn extract(event: &Event) -> Option<Self::EventTy> {
                (event.kind() == $kind).then(|| event.clone())
            }
        }
    };
}

impl_event!(click, EventKind::Click);
impl_event!(input, EventKind::Input);
impl_event!(keydown, EventKind::KeyDown);
impl_event!(keyup, EventKind::KeyUp);
impl_event!(focus, EventKind::Focus);
impl_event!(blur, EventKind::Blur);
impl_event!(mousedown, EventKind::MouseDown);
impl_event!(mouseup, EventKind::MouseUp);
impl_event!(mousemove, EventKind::MouseMove);
impl_event!(mouseover, EventKind::MouseOver);
impl_event!(mouseout, EventKind::MouseOut);
impl_event!(drag, EventKind::Drag);
impl_event!(terminal_resize, EventKind::TerminalResize);
impl_event!(raw_key, EventKind::RawKey);
impl_event!(raw_mouse, EventKind::RawMouse);

/// A handler that can receive an event descriptor's concrete event value.
pub trait EventHandler<E: EventDescriptor, R = ()>: 'static {
    fn call(&mut self, event: E::EventTy);
}

impl<E, F> EventHandler<E, ()> for F
where
    E: EventDescriptor,
    F: FnMut(E::EventTy) + 'static,
{
    fn call(&mut self, event: E::EventTy) {
        self(event);
    }
}
