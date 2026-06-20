//! A streaming-friendly Markdown component library for iodilos.
//!
//! Markdown is parsed with `pulldown-cmark` into a block IR, then rendered into
//! a flat `Vec<iodilos::text::Line>` (X-flat: block chrome drawn as span
//! characters, consumed by one `LineFlow` node). The full public API
//! (`markdown_lines` / `markdown`) lands in Task 10; for now the crate exposes
//! the parse + render primitives.

pub mod highlight;
pub mod inline;
pub mod parser;
pub mod render;
pub mod stream;
pub mod theme;
pub mod wrap;

pub use render::{render_blocks_to_lines, render_to_lines};
pub use stream::StreamingParser;
pub use theme::MarkdownTheme;
