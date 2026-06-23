//! A reactive terminal UI library.
//!
//! iodilos is a standalone reactive TUI framework, originally derived from the
//! `sycamore-tui` package of the [Sycamore](https://github.com/sycamore-rs/sycamore)
//! project. The reactive primitives (vendored from `sycamore-reactive`) and the
//! component model (vendored from `sycamore-core`) are inlined so that iodilos has
//! no external `sycamore-*` runtime dependency.

#[allow(unused_extern_crates)]
extern crate self as iodilos;

pub mod attributes;
pub mod bind;
pub mod framebuffer;
pub mod component;
pub mod components;
pub mod events;
pub mod layout;
pub mod node;
pub mod noderef;
pub mod producer;
pub mod reactive;
pub mod render;
pub mod style;
pub mod text;
pub mod view;

/// Re-export of the reactive primitives (vendored from `sycamore-reactive`).
pub mod reactive_primitives {
    pub use crate::reactive::*;
}

pub use attributes::{Attributes, GlobalAttributes, GlobalAttributesExt, SetAttribute};
/// The `#[component]` attribute macro for marking fn components.
pub use iodilos_macros::component;
pub use component::{Component, Props};
/// The `Props` derive macro. Coexists with the `Props` trait above under the
/// same name: derive macros live in the macro namespace, traits in the type
/// namespace, so `#[derive(Props)]` (macro) and `Props` (trait) never clash.
pub use iodilos_macros::Props;
pub use components::completion_menu::{CompletionItem, CompletionMenuProps, completion_menu};
pub use components::custom_element;
pub use components::iter::{Indexed, IndexedProps, Keyed, KeyedProps};
pub use components::show::{Show, ShowProps};
pub use components::overlay_box::{OverlayBoxProps, OverlayGeometry, overlay_box};
pub use components::scroll_view::{
    ScrollContent, ScrollViewProps, ScrollWindow, centered_window, scroll_view,
};
pub use components::table_view::{
    CellContext, CellFactory, TableRow, TableSection, TableViewProps, table_view,
};
pub use events::{Event, EventKind};
pub use producer::Spans;
pub use node::{AsTuiNode, NodeId, TuiNode};
pub use noderef::NodeRef;
pub use reactive::*;
pub use render::{
    QuitPolicy, RenderConfig, quit, render, render_async, render_async_with, render_with,
    use_future,
};
pub use style::{
    BorderCharacters, BorderStyle, Edges, FlexBasis, Gap, Inset, Margin, Padding, Percent, Size,
    Style, Weight,
};
pub use text::{Modifier, SpanStyle};
pub use view::{View, ViewNode, ViewTuiNode};

/// The color type, re-exported from crossterm — the single point where the
/// choice of paint backend dictates a public type (ADR-0024 §3).
pub use crossterm::style::Color;

/// Layout-mode enums, re-exported from taffy as-is (ADR-0024 §2).
pub use taffy::style::{
    AlignContent, AlignItems, Display, FlexDirection, FlexWrap, JustifyContent, Overflow, Position,
};

/// The normal import surface for TUI applications.
pub mod prelude {
    pub use crate::component::{Children, Component, Props};
    pub use crate::reactive::*;
    pub use crossterm::style::Color;
    pub use iodilos_macros::{Props, component, view};
    pub use taffy::style::{
        AlignContent, AlignItems, Display, FlexDirection, FlexWrap, JustifyContent, Overflow,
        Position,
    };

    pub use crate::attributes::{Attributes, GlobalAttributes, GlobalAttributesExt, SetAttribute};
    pub use crate::components::completion_menu::{
        CompletionItem, CompletionMenuProps, completion_menu,
    };
    pub use crate::components::iter::{Indexed, IndexedProps, Keyed, KeyedProps};
    pub use crate::components::overlay_box::{OverlayBoxProps, OverlayGeometry, overlay_box};
    pub use crate::components::show::{Show, ShowProps};
    pub use crate::components::scroll_view::{
        ScrollContent, ScrollViewProps, ScrollWindow, centered_window, scroll_view,
    };
    pub use crate::components::table_view::{
        CellContext, CellFactory, TableRow, TableSection, TableViewProps, table_view,
    };
    pub use crate::components::tags;
    pub use crate::events::{Event, EventKind};
    pub use crate::node::{NodeId, TuiNode};
    pub use crate::noderef::NodeRef;
    pub use crate::render::{
        QuitPolicy, RenderConfig, quit, render, render_async, render_async_with, render_with,
        use_future,
    };
    pub use crate::style::{
        BorderCharacters, BorderStyle, Edges, FlexBasis, Gap, Inset, Margin, Padding, Percent,
        Size, Style, Weight,
    };
    pub use crate::producer::{CellProducer, Lines, Plain, Spans};
    pub use crate::text::{Modifier, SpanStyle};
    pub use crate::view::{View, ViewNode, ViewTuiNode};
    pub use crate::{bind, events};
}

/// Re-exports used by `iodilos-macros`.
#[doc(hidden)]
pub mod rt {
    pub use crate::component::{Component, Props, component_scope, element_like_component_builder};
    // The `Props` derive macro re-exported at the same path as the `Props` trait
    // above, so that generated code (`#[derive(::iodilos::rt::Props)]` from
    // `inline_props`) resolves the derive. Different namespaces: the trait lives
    // in the type namespace, the derive in the macro namespace.
    pub use iodilos_macros::Props;
    pub use crate::reactive::*;

    pub use crate::attributes::{Attributes, GlobalAttributes, GlobalAttributesExt, SetAttribute};
    pub use crate::components::{custom_element, tags};
    pub use crate::style::Style;
    pub use crate::view::{View, ViewNode, ViewTuiNode};
    pub use crate::{bind, events};

    pub type Children = crate::component::Children<crate::View>;
}
