//! A statusline + framed multiline prompt component library for iodilos.
//!
//! Renders a rounded prompt box — statusline on the top border, framed
//! multiline input below, a self-drawn block cursor — into an iodilos
//! `TextSurface`. Pure rendering + an editing model; reactive wiring is left
//! to the application (see `examples/prompt_box.rs`).

mod model;
pub mod render;
pub mod statusline;
pub mod theme;

pub use model::PromptModel;
pub use render::render_prompt_to_surface;
pub use statusline::{StatusField, StatusLine};
pub use theme::PromptTheme;

use iodilos::view::View;

/// Render a non-reactive snapshot of the prompt into a `View` (scroll 0) using
/// the default theme. Reactive apps drive `render_prompt_to_surface` from a
/// memo instead (see `examples/prompt_box.rs`).
pub fn prompt_view(buffer: &str, cursor: usize, statusline: &StatusLine, width: usize) -> View {
    let surface =
        render_prompt_to_surface(buffer, cursor, statusline, width, &PromptTheme::default());
    View::text_surface(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_view_returns_text_surface_node() {
        let view = prompt_view("hi", 2, &StatusLine::default_mock(), 60);
        match &view.nodes()[0] {
            iodilos::node::TuiNode::TextSurface { surface, .. } => {
                assert!(surface.borrow().row_count() >= 2);
            }
            other => panic!("expected TextSurface node, got {other:?}"),
        }
    }
}
