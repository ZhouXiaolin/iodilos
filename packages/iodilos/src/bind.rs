//! Two-way bindings for TUI controls.

use std::borrow::Cow;

use crate::reactive::Signal;

use crate::attributes::{SetAttribute, StringAttribute};
use crate::events::{Event, EventDescriptor, input};

/// Description for a bindable property.
pub trait BindDescriptor {
    type Event: EventDescriptor<EventTy = Event>;
    type ValueTy: Clone + std::fmt::Display + 'static;
    const TARGET_PROPERTY: &'static str;
    fn extract_value(event: &Event) -> Option<Self::ValueTy>;
}

/// Bind an input's string value.
#[allow(non_camel_case_types)]
pub struct value;

impl BindDescriptor for value {
    type Event = input;
    type ValueTy = String;
    const TARGET_PROPERTY: &'static str = "value";

    fn extract_value(event: &Event) -> Option<Self::ValueTy> {
        event.input_value().map(ToOwned::to_owned)
    }
}

pub(crate) fn install_bind<B>(el: &mut crate::node::TuiNode, signal: Signal<B::ValueTy>)
where
    B: BindDescriptor,
{
    let scope = crate::reactive::use_current_scope();
    let handler = move |event: &Event| {
        if let Some(event) = B::Event::extract(event)
            && let Some(bound_value) = B::extract_value(&event)
        {
            scope.run_in(|| signal.set(bound_value));
        }
    };
    el.append_handler(
        <B::Event as EventDescriptor>::NAME.into(),
        Box::new(handler),
    );

    let signal_clone = signal;
    el.set_attribute(
        B::TARGET_PROPERTY,
        StringAttribute::from(move || Some(Cow::Owned(signal_clone.get_clone().to_string()))),
    );
}
