//! Render parsed Markdown `Block`s into a flat `Vec<iodilos::text::Line>`.
//!
//! This is the X-flat renderer (ADR: iodilos text-flow): every block's chrome
//! (heading underline, code-block frame, table alignment, blockquote bar, list
//! indent) is drawn as styled span characters into the line buffer. The whole
//! document becomes one `LineFlow`'s worth of `Line`s.

use iodilos::text::{Line, Modifier, Span, SpanStyle};

use crate::highlight::Highlighter;
use crate::parser::{Block, Inline};
use crate::theme::MarkdownTheme;

/// Render a full markdown source into a flat line list at `width` (cells).
pub fn render_to_lines(src: &str, width: usize, theme: &MarkdownTheme) -> Vec<Line> {
    let blocks = crate::parser::parse_with_theme(src, theme);
    render_blocks_to_lines(&blocks, width, theme)
}

/// Render already-parsed blocks into a flat line list. Used by the streaming
/// path, which keeps its own block list across ticks.
pub fn render_blocks_to_lines(blocks: &[Block], width: usize, theme: &MarkdownTheme) -> Vec<Line> {
    let hl = Highlighter::new();
    let mut out = Vec::new();
    let mut first = true;
    for block in blocks {
        if !first {
            out.push(Line::raw("")); // blank-line rhythm between blocks
        }
        first = false;
        render_block(block, width, theme, &hl, 0, &mut out);
    }
    out
}

fn render_block(
    block: &Block,
    width: usize,
    theme: &MarkdownTheme,
    hl: &Highlighter,
    blockquote_depth: usize,
    out: &mut Vec<Line>,
) {
    let _ = (hl, blockquote_depth);
    match block {
        Block::Heading { level, inlines } => render_heading(*level, inlines, theme, out),
        Block::Rule => render_rule(theme, width, out),
        Block::Paragraph(_)
        | Block::CodeBlock { .. }
        | Block::List(_)
        | Block::BlockQuote(_)
        | Block::Table(_)
        | Block::Math(_) => todo!("later tasks"),
    }
}

/// Flatten inlines to a single owned string (used where one leaf carries one
/// style, e.g. headings).
fn inlines_to_string(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t, _) => s.push_str(t),
            Inline::Code(t) => s.push_str(t),
            Inline::Math(t) => s.push_str(t),
            Inline::SoftBreak => s.push('\n'),
        }
    }
    s
}

fn render_heading(level: u8, inlines: &[Inline], theme: &MarkdownTheme, out: &mut Vec<Line>) {
    let color = theme
        .heading
        .get((level as usize).saturating_sub(1))
        .copied()
        .unwrap_or(theme.heading[5]);
    let style = SpanStyle {
        fg: Some(color),
        add_modifier: Modifier::BOLD,
        ..SpanStyle::default()
    };
    // Flatten inlines into one styled span; heading text is single-style.
    let text = inlines_to_string(inlines);
    out.push(Line::from(vec![Span::styled(text, style)]));
    // H1/H2 get a underline rule.
    if level <= 2 {
        let bar = "─".repeat(width_for_heading(level));
        out.push(Line::from(vec![Span::styled(
            bar,
            SpanStyle {
                fg: Some(color),
                ..SpanStyle::default()
            },
        )]));
    }
}

fn width_for_heading(level: u8) -> usize {
    if level == 1 { 40 } else { 20 }
}

fn render_rule(theme: &MarkdownTheme, width: usize, out: &mut Vec<Line>) {
    let bar = "─".repeat(width.max(1));
    out.push(Line::from(vec![Span::styled(
        bar,
        SpanStyle {
            fg: Some(theme.rule_color),
            ..SpanStyle::default()
        },
    )]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_renders_bold_colored_line() {
        let theme = MarkdownTheme::default();
        let lines = render_to_lines("# Title", 40, &theme);
        assert!(!lines.is_empty());
        let first = &lines[0];
        assert_eq!(first.spans.len(), 1);
        assert!(first.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(first.spans[0].style.fg, Some(theme.heading[0]));
    }

    #[test]
    fn rule_renders_bar_line() {
        let theme = MarkdownTheme::default();
        let lines = render_to_lines("---", 10, &theme);
        let bar_line = &lines[0];
        let s: String = bar_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s, "──────────");
    }

    #[test]
    fn highlight_known_language_produces_some_color() {
        let hl = Highlighter::new();
        let toks = hl.highlight_line("fn main() {}", "rust");
        assert!(!toks.is_empty());
        assert!(
            toks.iter().any(|(_, c)| c.is_some()),
            "expected at least one colored token: {toks:?}"
        );
    }

    #[test]
    fn highlight_unknown_language_is_uncolored() {
        let hl = Highlighter::new();
        let toks = hl.highlight_line("hello", "totally-not-a-language-xyz");
        assert_eq!(toks.len(), 1);
        assert!(toks[0].1.is_none(), "unknown lang should be uncolored");
    }

    #[test]
    fn highlight_rust_emits_distinct_token_colors() {
        // A line with keywords, types, and numbers should produce several runs
        // and at least two distinct colors — proving per-token coloring.
        let hl = Highlighter::new();
        let toks = hl.highlight_line("fn add(a: u32, b: u32) -> u32 { a + b }", "rust");
        let colors: Vec<_> = toks.iter().filter_map(|(_, c)| *c).collect();
        assert!(
            colors.len() >= 2,
            "expected multiple colored runs, got {toks:?}"
        );
        let distinct: std::collections::HashSet<_> = colors.iter().collect();
        assert!(
            distinct.len() >= 2,
            "expected at least 2 distinct colors, got {colors:?}"
        );
    }
}
