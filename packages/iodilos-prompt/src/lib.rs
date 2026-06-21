//! A statusline + framed multiline prompt component library for iodilos.
//!
//! Renders a rounded prompt box — statusline on the top border, framed
//! multiline input below, a self-drawn block cursor — into an iodilos
//! `TextSurface`. Pure rendering + an editing model; reactive wiring is left
//! to the application (see `examples/prompt_box.rs`).

mod model;
pub mod statusline;
pub mod theme;

pub use model::PromptModel;
pub use statusline::{StatusField, StatusLine};
pub use theme::PromptTheme;
