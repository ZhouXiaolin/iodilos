//! A streaming-friendly Markdown component library for iodilos.
//!
//! Markdown is rendered into a flat `Vec<iodilos::text::Line>` (X-flat: block
//! chrome drawn as span characters), wrapped in a single `LineFlow` node. The
//! streaming viewer drives it from a `Signal<String>` + a width signal.

pub mod highlight;
pub mod inline;
pub mod parser;
pub mod render;
pub mod stream;
pub mod theme;
pub mod wrap;

pub use highlight::Highlighter;
pub use parser::{parse, parse_with_theme, Block, Inline, List, ListItem, Table};
pub use render::{render_blocks_to_lines, render_to_lines};
pub use stream::StreamingParser;
pub use theme::MarkdownTheme;

use iodilos::text::Line;
use iodilos::view::View;

/// Render Markdown source into a flat line list at `width` (cells), themed.
pub fn markdown_lines(src: &str, width: usize, theme: &MarkdownTheme) -> Vec<Line> {
    render_to_lines(src, width, theme)
}

/// Render Markdown into a `LineFlow` view (offset 0) using the default theme.
/// The caller wraps this in an `overflow: hidden` div and drives the `LineFlow`'s
/// offset for scrolling.
pub fn markdown(src: &str, width: usize) -> View {
    View::line_flow(markdown_lines(src, width, &MarkdownTheme::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_returns_lineflow_view() {
        let view = markdown("# hi", 40);
        let node = &view.nodes()[0];
        let n_lines = match node {
            iodilos::node::TuiNode::LineFlow { lines, .. } => lines.borrow().len(),
            _ => panic!("expected LineFlow, got {node:?}"),
        };
        assert!(n_lines >= 1);
    }
}
