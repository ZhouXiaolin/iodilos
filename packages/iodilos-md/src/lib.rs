//! A streaming-friendly Markdown component library for iodilos.
//!
//! Markdown is rendered into an iodilos `TextSurface` (X-flat: block chrome
//! drawn as segment characters). The
//! streaming viewer drives it from a `Signal<String>` + a width signal.

mod frontmatter;
pub mod highlight;
pub mod inline;
pub mod latex;
pub mod mermaid;
pub mod parser;
pub mod render;
pub mod stream;
pub mod theme;
mod wrap;

pub use highlight::Highlighter;
pub use parser::{Block, Inline, List, ListItem, Table, parse, parse_with_theme};
pub use render::{render_blocks_to_surface, render_to_surface};
pub use stream::StreamingParser;
pub use theme::MarkdownTheme;

use iodilos::surface::TextSurface;
use iodilos::view::View;

/// Render Markdown source into a text surface at `width` (cells), themed.
pub fn markdown_surface(src: &str, width: usize, theme: &MarkdownTheme) -> TextSurface {
    render_to_surface(src, width, theme)
}

/// Render Markdown into a `TextSurface` view (scroll 0) using the default theme.
/// The caller wraps this in an `overflow: hidden` div and drives the surface's
/// offset for scrolling.
pub fn markdown(src: &str, width: usize) -> View {
    View::text_surface(markdown_surface(src, width, &MarkdownTheme::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_returns_text_surface_view() {
        let view = markdown("# hi", 40);
        let node = &view.nodes()[0];
        let row_count = match node {
            iodilos::node::TuiNode::TextSurface { surface, .. } => surface.borrow().row_count(),
            _ => panic!("expected TextSurface, got {node:?}"),
        };
        assert!(row_count >= 1);
    }
}
