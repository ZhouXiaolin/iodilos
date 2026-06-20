//! A streaming-friendly Markdown component library for iodilos.
//!
//! [`render_markdown`] parses Markdown source with `pulldown-cmark` and returns
//! an iodilos [`View`] tree built imperatively (no `view!` macro). Because the
//! whole tree is rebuilt on every call, a streaming viewer can drive it from a
//! `Signal<String>`:
//!
//! ```ignore
//! let content = create_signal(String::new());
//! // ...in a use_future, push chunks then content.set(buffer)
//! view! {
//!     div { (move || iodilos_md::render_markdown(&content.get_clone(), &theme)) }
//! }
//! ```
//!
//! The parenthesized closure returns `View` (not `String`), so iodilos takes
//! the full `Dynamic` rebuild path (`TuiNode::create_dynamic_view`).

pub mod highlight;
pub mod inline;
pub mod parser;
pub mod render;
pub mod stream;
pub mod theme;

pub use render::{
    estimate_blocks_lines, estimate_lines, estimate_lines_with_width, render_blocks_view,
    render_markdown,
};
pub use stream::StreamingParser;
pub use theme::MarkdownTheme;

use iodilos::prelude::*;

/// Render Markdown using the default theme. Convenience wrapper around
/// [`render_markdown`].
pub fn markdown(src: &str) -> View {
    render_markdown(src, &MarkdownTheme::default())
}
