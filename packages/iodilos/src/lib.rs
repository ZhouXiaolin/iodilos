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
pub mod canvas;
pub mod component;
pub mod components;
pub mod events;
pub mod layout;
pub mod node;
pub mod noderef;
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
pub use components::custom_element;
pub use component::{Component, Props};
pub use events::{Event, EventKind};
pub use node::{AsTuiNode, NodeId, TuiNode};
pub use noderef::NodeRef;
pub use reactive::*;
pub use render::{render, render_async, use_future};
pub use style::{
    BorderCharacters, BorderStyle, Edges, FlexBasis, Gap, Inset, Margin, Padding, Percent, Size,
    Style, TextStyle, Weight,
};
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
    pub use crossterm::style::Color;
    pub use crate::component::{Component, Props};
    pub use crate::reactive::*;
    pub use iodilos_macros::view;
    pub use taffy::style::{
        AlignContent, AlignItems, Display, FlexDirection, FlexWrap, JustifyContent, Overflow,
        Position,
    };

    pub use crate::attributes::{Attributes, GlobalAttributes, GlobalAttributesExt, SetAttribute};
    pub use crate::components::tags;
    pub use crate::events::{Event, EventKind};
    pub use crate::node::{NodeId, TuiNode};
    pub use crate::noderef::NodeRef;
    pub use crate::render::{render, render_async, use_future};
    pub use crate::style::{
        BorderCharacters, BorderStyle, Edges, FlexBasis, Gap, Inset, Margin, Padding, Percent,
        Size, Style, TextStyle, Weight,
    };
    pub use crate::view::{View, ViewNode, ViewTuiNode};
    pub use crate::{bind, events};
}

/// Re-exports used by `iodilos-macros`.
#[doc(hidden)]
pub mod rt {
    pub use crate::component::{Component, Props, component_scope, element_like_component_builder};
    pub use crate::reactive::*;

    pub use crate::attributes::{Attributes, GlobalAttributes, GlobalAttributesExt, SetAttribute};
    pub use crate::components::{custom_element, tags};
    pub use crate::style::Style;
    pub use crate::view::{View, ViewNode, ViewTuiNode};
    pub use crate::{bind, events};

    pub type Children = crate::component::Children<crate::View>;
}
