//! Word-boundary wrapping for markdown inline runs (leaf `wrapping.rs` port).

use iodilos::text::{Line, Span};

/// Placeholder; replaced with a real word-boundary wrapper in Task 5.
pub fn wrap_inline_runs(
    runs: Vec<Span>,
    _first_prefix: &[Span],
    _continuation_prefix: &[Span],
    _width: usize,
) -> Vec<Line> {
    vec![Line::from(runs)]
}
