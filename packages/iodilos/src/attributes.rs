//! Attribute builders for TUI elements.
//!
//! Non-style attributes are string/bool values stored on the node. Style is
//! authored as flat named properties (`padding = 2`, `color = Color::Blue`,
//! `border_style = BorderStyle::Single`) — each one is a `(name, Box<dyn
//! StylePropValue>)` pair that knows how to write itself into a resolved
//! [`Style`] at layout time. There is no single `style=` value; the `TuiStyle`
//! aggregate and `style()` builder are gone (ADR-0024 §1).
//!
//! Every style property accepts a `MaybeDyn`, so a property may be a static
//! value (`padding = 2`) or a reactive signal (`padding = pad_signal`),
//! per-property (ADR-0024 §7).

use std::borrow::Cow;
use std::rc::Rc;

use crate::reactive::{MaybeDyn, ReadSignal, Signal};

use crate::events::{Event, EventDescriptor, EventHandler};
use crate::node::{AsTuiNode, StyleProp, StylePropValue, TuiNode};
use crate::style::Style;
use crate::view::{View, ViewNode};

type BoxedEventHandler = Box<dyn FnMut(&Event)>;

/// A style-property value that may be static or reactive.
///
/// This mirrors [`MaybeDyn`]'s role but drops its `T: Into<MaybeDyn<T>>`
/// constraint, so it also works for style properties whose value type is an
/// *external* enum re-exported as-is from taffy (`Display`, `FlexDirection`, …)
/// or from crossterm (`Color`) — types for which orphan rules forbid a
/// `From<T> for MaybeDyn<T>` impl. Every flat style property accepts one of
/// these (ADR-0024 §7): `padding = 2`, `color = Color::Blue`, or
/// `padding = pad_signal`.
#[derive(Clone)]
pub enum StyleDyn<T: 'static> {
    /// A static value.
    Static(T),
    /// A reactive value backed by a signal.
    Signal(ReadSignal<T>),
    /// A derived reactive value.
    Derived(Rc<dyn Fn() -> T>),
}

impl<T: 'static> StyleDyn<T> {
    /// Construct a derived style value from a closure, for cases where a
    /// `Signal` is not directly available (e.g. a computed expression).
    pub fn derived(f: impl Fn() -> T + 'static) -> Self {
        StyleDyn::Derived(Rc::new(f))
    }
}

impl<T: Clone + 'static> StyleDyn<T> {
    /// Read the current value, tracking any signal dependency.
    pub fn get(&self) -> T {
        match self {
            StyleDyn::Static(v) => v.clone(),
            StyleDyn::Signal(s) => s.get_clone(),
            StyleDyn::Derived(f) => f(),
        }
    }
}

/// Anything that can be converted into a [`StyleDyn<T>`]. Implemented for the
/// value type itself, signals, closures, and `StyleDyn` itself — the same
/// surface `MaybeDyn` offers for local types, extended to external types
/// (`Color`, `Display`, …) that can't get a `MaybeDyn` impl due to orphan rules.
pub trait IntoStyleDyn<T: 'static> {
    /// Convert into a [`StyleDyn`].
    fn into_style_dyn(self) -> StyleDyn<T>;
}

impl<T: 'static> IntoStyleDyn<T> for StyleDyn<T> {
    fn into_style_dyn(self) -> StyleDyn<T> {
        self
    }
}

// Signals (both local types) feed a `StyleDyn`.
impl<T: 'static> IntoStyleDyn<T> for ReadSignal<T> {
    fn into_style_dyn(self) -> StyleDyn<T> {
        StyleDyn::Signal(self)
    }
}

impl<T: 'static> IntoStyleDyn<T> for Signal<T> {
    fn into_style_dyn(self) -> StyleDyn<T> {
        StyleDyn::Signal(*self)
    }
}

impl<T: 'static, F> IntoStyleDyn<T> for F
where
    F: Fn() -> T + 'static,
{
    fn into_style_dyn(self) -> StyleDyn<T> {
        StyleDyn::Derived(Rc::new(self))
    }
}

/// Generate `IntoStyleDyn<$ty>` for the value type itself and each conversion
/// source type (which become static values via `Into`). This is how `gap = 1`
/// works: `1` (an `i32`) converts to `Gap` through `Gap`'s `From<i32>`.
macro_rules! impl_into_style_dyn_for_value {
    ($ty:ty $(, $from:ty)* $(,)?) => {
        impl IntoStyleDyn<$ty> for $ty {
            fn into_style_dyn(self) -> StyleDyn<$ty> {
                StyleDyn::Static(self)
            }
        }
        $(
            impl IntoStyleDyn<$ty> for $from {
                fn into_style_dyn(self) -> StyleDyn<$ty> {
                    StyleDyn::Static(self.into())
                }
            }
        )*
    };
}

// Self-owned length types accept their own type plus the integer shorthands.
impl_into_style_dyn_for_value!(crate::style::Percent, f32);
impl_into_style_dyn_for_value!(crate::style::Padding, i32, crate::style::Percent);
impl_into_style_dyn_for_value!(crate::style::Gap, i32, crate::style::Percent);
impl_into_style_dyn_for_value!(crate::style::Margin, i32, crate::style::Percent);
impl_into_style_dyn_for_value!(crate::style::Size, i32, crate::style::Percent);
impl_into_style_dyn_for_value!(crate::style::Inset, i32, crate::style::Percent);
impl_into_style_dyn_for_value!(crate::style::FlexBasis, i32, crate::style::Percent);
impl_into_style_dyn_for_value!(crate::style::BorderStyle);
impl_into_style_dyn_for_value!(crate::style::Weight);
impl_into_style_dyn_for_value!(crate::style::Edges);
impl_into_style_dyn_for_value!(crate::style::BorderTitleRuns);
// Re-exported external enums take only their own type.
impl_into_style_dyn_for_value!(taffy::style::Display);
impl_into_style_dyn_for_value!(taffy::style::FlexDirection);
impl_into_style_dyn_for_value!(taffy::style::FlexWrap);
impl_into_style_dyn_for_value!(taffy::style::Overflow);
impl_into_style_dyn_for_value!(taffy::style::Position);
impl_into_style_dyn_for_value!(taffy::style::AlignItems);
// NOTE: `taffy::style::JustifyContent` is the *same type* as `AlignContent` in
// taffy 0.11, so this single impl covers both; the `justify_content` setter
// routes through it.
impl_into_style_dyn_for_value!(taffy::style::AlignContent);
impl_into_style_dyn_for_value!(crossterm::style::Color);
impl_into_style_dyn_for_value!(f32);
impl_into_style_dyn_for_value!(bool);

/// A possibly dynamic string attribute.
pub type StringAttribute = MaybeDyn<Option<Cow<'static, str>>>;
/// A possibly dynamic boolean attribute.
pub type BoolAttribute = MaybeDyn<bool>;

/// A value that can be applied to a TUI element attribute.
pub trait AttributeValue: AttributeValueBoxed + 'static {
    fn set_self(self, el: &mut TuiNode, name: Cow<'static, str>);
}

impl AttributeValue for StringAttribute {
    fn set_self(self, el: &mut TuiNode, name: Cow<'static, str>) {
        el.append_attribute(name, self);
    }
}

impl AttributeValue for BoolAttribute {
    fn set_self(self, el: &mut TuiNode, name: Cow<'static, str>) {
        el.append_bool_attribute(name, self);
    }
}

impl AttributeValue for Box<dyn StylePropValue> {
    fn set_self(self, el: &mut TuiNode, name: Cow<'static, str>) {
        el.set_style_prop((name, self));
    }
}

/// A wrapper that pairs a [`StyleDyn<T>`] with the field it writes to. This is
/// the concrete storage behind every flat style property.
#[derive(Clone)]
pub struct FlatStyle<T: 'static> {
    value: StyleDyn<T>,
    setter: fn(&T, &mut Style),
}

impl<T: 'static> FlatStyle<T> {
    /// Construct a flat style property that writes `value` via `setter` when
    /// resolved.
    pub fn new(value: StyleDyn<T>, setter: fn(&T, &mut Style)) -> Self {
        Self { value, setter }
    }
}

impl<T: Clone + 'static> StylePropValue for FlatStyle<T> {
    fn apply(&self, style: &mut Style) {
        let value = self.value.get();
        (self.setter)(&value, style);
    }
    fn clone_box(&self) -> Box<dyn StylePropValue> {
        Box::new(self.clone())
    }
}

impl<T: Clone + 'static> AttributeValue for FlatStyle<T> {
    fn set_self(self, el: &mut TuiNode, name: Cow<'static, str>) {
        el.set_style_prop((name, Box::new(self)));
    }
}

/// Resolve every flat style property stored on an element into one [`Style`],
/// applying them in order so later assignments win.
pub(crate) fn resolve_style(props: &[StyleProp], base: Style) -> Style {
    let mut style = base;
    for (_name, value) in props {
        value.apply(&mut style);
    }
    style
}

#[doc(hidden)]
pub trait AttributeValueBoxed: 'static {
    fn set_self_boxed(self: Box<Self>, el: &mut TuiNode, name: Cow<'static, str>);
}

impl<T> AttributeValueBoxed for T
where
    T: AttributeValue,
{
    fn set_self_boxed(self: Box<Self>, el: &mut TuiNode, name: Cow<'static, str>) {
        self.set_self(el, name);
    }
}

impl AttributeValue for Box<dyn AttributeValue> {
    fn set_self(self, el: &mut TuiNode, name: Cow<'static, str>) {
        self.set_self_boxed(el, name);
    }
}

/// Implemented by types that can receive attributes and event handlers.
pub trait SetAttribute {
    fn set_attribute(&mut self, name: &'static str, value: impl AttributeValue);
    fn set_event_handler(&mut self, name: &'static str, value: impl FnMut(&Event) + 'static);
}

impl<T> SetAttribute for T
where
    T: AsTuiNode,
{
    fn set_attribute(&mut self, name: &'static str, value: impl AttributeValue) {
        value.set_self(self.as_tui_node(), name.into());
    }

    fn set_event_handler(&mut self, name: &'static str, value: impl FnMut(&Event) + 'static) {
        self.as_tui_node()
            .append_handler(name.into(), Box::new(value));
    }
}

/// Global builder methods available on all TUI elements.
pub trait GlobalAttributes: AsTuiNode + Sized {
    fn attr(mut self, name: &'static str, value: impl Into<StringAttribute>) -> Self {
        self.set_attribute(name, value.into());
        self
    }

    fn bool_attr(mut self, name: &'static str, value: impl Into<BoolAttribute>) -> Self {
        self.set_attribute(name, value.into());
        self
    }

    fn prop(mut self, name: &'static str, value: impl Into<StringAttribute>) -> Self {
        self.set_attribute(name, value.into());
        self
    }

    /// Apply a flat style property (`name` is retained for debugging; the
    /// value carries its own field setter). The `view!` macro routes every
    /// named style attribute through this.
    fn style_prop<T: Clone + 'static>(
        mut self,
        name: &'static str,
        value: impl IntoStyleDyn<T>,
        setter: fn(&T, &mut Style),
    ) -> Self {
        self.set_attribute(name, FlatStyle::new(value.into_style_dyn(), setter));
        self
    }

    fn id(self, value: impl Into<StringAttribute>) -> Self {
        self.attr("id", value)
    }

    fn class(self, value: impl Into<StringAttribute>) -> Self {
        self.attr("class", value)
    }

    fn value(self, value: impl Into<StringAttribute>) -> Self {
        self.attr("value", value)
    }

    fn placeholder(self, value: impl Into<StringAttribute>) -> Self {
        self.attr("placeholder", value)
    }

    fn tabindex(self, value: impl Into<StringAttribute>) -> Self {
        self.attr("tabindex", value)
    }

    fn disabled(self, value: impl Into<BoolAttribute>) -> Self {
        self.bool_attr("disabled", value)
    }

    fn on<E, R>(mut self, _: E, mut handler: impl EventHandler<E, R>) -> Self
    where
        E: EventDescriptor,
    {
        let scope = crate::reactive::use_current_scope();
        let wrapper = move |event: &Event| {
            if let Some(event) = E::extract(event) {
                scope.run_in(|| handler.call(event));
            }
        };
        self.set_event_handler(E::NAME, wrapper);
        self
    }

    fn bind<B>(mut self, _: B, signal: crate::reactive::Signal<B::ValueTy>) -> Self
    where
        B: crate::bind::BindDescriptor,
    {
        crate::bind::install_bind::<B>(self.as_tui_node(), signal);
        self
    }

    fn r#ref(mut self, noderef: crate::noderef::NodeRef) -> Self {
        let id = TuiNode::id(self.as_tui_node());
        noderef.set(id);
        self
    }

    fn spread(mut self, attributes: Attributes) -> Self {
        attributes.apply_self(self.as_tui_node());
        self
    }

    fn children(mut self, children: impl Into<View>) -> Self {
        self.as_tui_node().append_view(children.into());
        self
    }
}

impl<T: AsTuiNode> GlobalAttributes for T {}

/// Generate builder methods for flat style properties whose field type equals
/// the value type. The generated method takes `impl Into<MaybeDyn<$ty>>` and
/// writes `s.$field = *v` at resolve time.
macro_rules! style_methods {
    ($(($method:ident, $ty:ty, $field:ident));* $(;)?) => {
        $(
            #[doc = concat!("Set the `", stringify!($method), "` style property. Accepts a static value or a reactive signal.")]
            fn $method(self, value: impl $crate::attributes::IntoStyleDyn<$ty>) -> Self {
                self.style_prop(stringify!($method), value, |v, s| s.$field = *v)
            }
        )*
    };
}

/// Like [`style_methods!`], but for fields of type `Option<$ty>`: the author
/// passes a plain `$ty` value and it is wrapped in `Some(..)`.
macro_rules! style_methods_optional {
    ($(($method:ident, $ty:ty, $field:ident));* $(;)?) => {
        $(
            #[doc = concat!("Set the `", stringify!($method), "` style property. Accepts a static value or a reactive signal.")]
            fn $method(self, value: impl $crate::attributes::IntoStyleDyn<$ty>) -> Self {
                self.style_prop(stringify!($method), value, |v, s| s.$field = Some(*v))
            }
        )*
    };
}

impl<T: AsTuiNode> GlobalAttributesExt for T {}

/// Extension trait carrying the generated flat-style builder methods. Split
/// from [`GlobalAttributes`] so the macro-generated methods live next to their
/// shared `style_prop` helper. Implemented for every `AsTuiNode` below.
pub trait GlobalAttributesExt: GlobalAttributes {
    style_methods! {
        (display, taffy::style::Display, display);
        (flex_direction, taffy::style::FlexDirection, flex_direction);
        (flex_wrap, taffy::style::FlexWrap, flex_wrap);
        (overflow, taffy::style::Overflow, overflow);
        (position, taffy::style::Position, position);
        (z_index, i32, z_index);
        (width, crate::style::Size, width);
        (height, crate::style::Size, height);
        (min_width, crate::style::Size, min_width);
        (min_height, crate::style::Size, min_height);
        (max_width, crate::style::Size, max_width);
        (max_height, crate::style::Size, max_height);
        (flex_basis, crate::style::FlexBasis, flex_basis);
        (flex_grow, f32, flex_grow);
        (padding, crate::style::Padding, padding);
        (padding_top, crate::style::Padding, padding_top);
        (padding_right, crate::style::Padding, padding_right);
        (padding_bottom, crate::style::Padding, padding_bottom);
        (padding_left, crate::style::Padding, padding_left);
        (margin, crate::style::Margin, margin);
        (margin_top, crate::style::Margin, margin_top);
        (margin_right, crate::style::Margin, margin_right);
        (margin_bottom, crate::style::Margin, margin_bottom);
        (margin_left, crate::style::Margin, margin_left);
        (gap, crate::style::Gap, gap);
        (column_gap, crate::style::Gap, column_gap);
        (row_gap, crate::style::Gap, row_gap);
        (inset, crate::style::Inset, inset);
        (top, crate::style::Inset, top);
        (right, crate::style::Inset, right);
        (bottom, crate::style::Inset, bottom);
        (left, crate::style::Inset, left);
        (border_style, crate::style::BorderStyle, border_style);
        (weight, crate::style::Weight, weight);
        (underline, bool, underline);
        (italic, bool, italic);
        (invert, bool, invert);
        (dim, bool, dim);
        (crossed_out, bool, crossed_out);
    }

    style_methods_optional! {
        (align_items, taffy::style::AlignItems, align_items);
        (align_content, taffy::style::AlignContent, align_content);
        (justify_content, taffy::style::JustifyContent, justify_content);
        (flex_shrink, f32, flex_shrink);
        (border_edges, crate::style::Edges, border_edges);
        (background_color, crossterm::style::Color, background_color);
        (border_color, crossterm::style::Color, border_color);
        (color, crossterm::style::Color, color);
        (underline_color, crossterm::style::Color, underline_color);
    }

    /// Set the `border_title` style property: styled runs painted on the top
    /// border (a legend). `Vec` is not `Copy`, so this is a hand-written method
    /// rather than a `style_methods_optional!` entry. Accepts a static value or
    /// any reactive source (signal / closure) via [`IntoStyleDyn`].
    fn border_title(
        self,
        value: impl IntoStyleDyn<crate::style::BorderTitleRuns>,
    ) -> Self {
        self.style_prop("border_title", value, |v, s| {
            s.border_title = Some(v.clone())
        })
    }
}
/// A spreadable attribute bag.
#[derive(Default)]
pub struct Attributes {
    values: Vec<(Cow<'static, str>, Box<dyn AttributeValue>)>,
    event_handlers: Vec<(Cow<'static, str>, BoxedEventHandler)>,
}

impl Attributes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_self(self, el: &mut TuiNode) {
        for (name, value) in self.values {
            value.set_self(el, name);
        }
        for (name, handler) in self.event_handlers {
            el.append_handler(name, handler);
        }
    }
}

impl SetAttribute for Attributes {
    fn set_attribute(&mut self, name: &'static str, value: impl AttributeValue) {
        self.values.push((name.into(), Box::new(value)));
    }

    fn set_event_handler(&mut self, name: &'static str, value: impl FnMut(&Event) + 'static) {
        self.event_handlers.push((name.into(), Box::new(value)));
    }
}
