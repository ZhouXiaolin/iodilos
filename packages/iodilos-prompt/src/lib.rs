//! A statusline + framed multiline prompt component library for iodilos.
//!
//! The prompt is composed from framework primitives — a rounded `div` border, a
//! `border_title` statusline, and a `Spans` input leaf — wrapped in the reactive
//! [`PromptBox`] component. Pure data (the editing model, statusline, theme)
//! and the run-builders that feed the framework live below.

mod component;
mod model;
pub mod render;
pub mod statusline;
pub mod theme;

pub use component::{PromptBox, PromptBoxProps, PromptSubmit};
pub use model::PromptModel;
pub use render::{input_runs, statusline_runs};
pub use statusline::{StatusField, StatusLine};
pub use theme::PromptTheme;
