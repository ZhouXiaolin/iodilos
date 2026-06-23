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
pub mod view;
mod wrap;

pub use highlight::Highlighter;
pub use parser::{Block, Inline, List, ListItem, Table, parse, parse_with_theme};
pub use render::{render_blocks_to_surface, render_to_surface};
pub use stream::StreamingParser;
pub use theme::MarkdownTheme;
pub use view::{blocks_height, blocks_to_view};

use iodilos::producer::Lines;
use iodilos::view::View;

/// Render Markdown source into a text surface at `width` (cells), themed.
pub fn markdown_surface(src: &str, width: usize, theme: &MarkdownTheme) -> Lines {
    render_to_surface(src, width, theme)
}

/// Render Markdown into a `Lines` producer view (scroll 0) using the default
/// theme. Legacy pre-wrapped path; prefer [`markdown_view`] for new code.
pub fn markdown(src: &str, width: usize) -> View {
    View::leaf(Box::new(markdown_surface(src, width, &MarkdownTheme::default())))
}

/// Render Markdown source into a **View tree** (framework primitives: `Spans`
/// leaves for text, `div(border_style)` + `border_title` for code/math frames).
/// Text re-wraps at the layout width for free on resize — no width guessing.
pub fn markdown_view(src: &str) -> View {
    let theme = MarkdownTheme::default();
    let blocks = parse_with_theme(src, &theme);
    blocks_to_view(&blocks, &theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_returns_text_surface_view() {
        let view = markdown("# hi", 40);
        let node = &view.nodes()[0];
        let row_count = match node {
            iodilos::node::TuiNode::Leaf { producer, .. } => producer.borrow().measure(40),
            _ => panic!("expected Leaf, got {node:?}"),
        };
        assert!(row_count >= 1);
    }
}
