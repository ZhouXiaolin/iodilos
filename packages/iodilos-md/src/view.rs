//! View-tree renderer: builds an iodilos [`View`] from parsed Markdown blocks,
//! composed from framework primitives.
//!
//! Where the legacy [`crate::render`] path flattened the whole document into one
//! pre-wrapped [`iodilos::producer::Lines`], this module composes a real view
//! tree:
//! - paragraphs / headings / list items are [`iodilos::producer::Spans`] leaves
//!   that **re-wrap at the layout width** (word-wrap for prose);
//! - code / math / mermaid blocks are `div(border_style)` with a `border_title`
//!   language label — the framework draws the frame;
//! - blockquotes are a bar column + content column;
//! - tables / frontmatter (column-aligned grids) stay [`Lines`] leaves, since
//!   their layout does not map cleanly to flexbox.
//!
//! The markdown component ([`crate::Markdown`]) drives this from a reactive
//! content signal; see `component.rs`.

use std::borrow::Cow;

use iodilos::prelude::*;
use iodilos::producer::{Lines, Spans};
use iodilos::style::{BorderStyle, BorderTitleRuns, Edges};
use iodilos::text::{Modifier, SpanStyle};
use iodilos::Color;

use crate::highlight::Highlighter;
use crate::parser::{Block, Inline, List};
use crate::render::{display_width, frontmatter_rows, inline_runs, inlines_to_string, item_marker, table_rows};
use crate::theme::MarkdownTheme;

fn fg(color: Color) -> SpanStyle {
    SpanStyle {
        fg: Some(color),
        ..SpanStyle::default()
    }
}

/// Build a column `View` holding one view per block, with a 1-cell gap between
/// blocks (the blank-line rhythm).
pub fn blocks_to_view(blocks: &[Block], theme: &MarkdownTheme) -> View {
    let hl = Highlighter::new();
    let children: Vec<View> = blocks
        .iter()
        .map(|b| block_to_view(b, theme, &hl))
        .collect();
    View::from(
        tags::div()
            .flex_direction(FlexDirection::Column)
            // `row_gap` is the gap between rows of a column-stack — `column_gap`
            // is mapped to `gap.width` (horizontal) and would be a no-op here,
            // leaving headings glued to the paragraph above them.
            .row_gap(1)
            .children(children),
    )
}

/// Estimate the rendered height (rows) of `blocks` at `width`, by measuring the
/// same producers [`blocks_to_view`] builds. Used by viewers that need to drive
/// a scroll offset (follow-the-tail) for a View-tree document, where the layout
/// height is not directly observable from the reactive layer.
///
/// Indented blocks (lists, blockquotes) are measured at the full `width`, so the
/// estimate is a lower bound — their real content wraps a little taller. This is
/// the safe direction for follow-the-tail (it underscrolls by at most a couple
/// of rows rather than overshooting into blank space).
pub fn blocks_height(blocks: &[Block], width: usize, theme: &MarkdownTheme) -> usize {
    let hl = Highlighter::new();
    let mut total = 0usize;
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            total += 1; // column gap
        }
        total += block_height(block, width, theme, &hl);
    }
    total.max(1)
}

fn block_height(block: &Block, width: usize, theme: &MarkdownTheme, hl: &Highlighter) -> usize {
    let content_w = width.max(1);
    match block {
        Block::Heading { level, inlines } => {
            let color = theme
                .heading
                .get((*level as usize).saturating_sub(1))
                .copied()
                .unwrap_or(theme.heading[5]);
            let style = SpanStyle {
                fg: Some(color),
                add_modifier: Modifier::BOLD,
                ..SpanStyle::default()
            };
            let h = Spans::word_wrap(vec![(inlines_to_string(inlines), style)]).measure(content_w);
            h + if *level <= 2 { 1 } else { 0 }
        }
        Block::Paragraph(inlines) => {
            Spans::word_wrap(inline_runs(inlines, theme, 0)).measure(content_w)
        }
        Block::Rule => 1,
        Block::CodeBlock { code, .. } => {
            let lines = code.lines().chain((code.is_empty()).then_some("")).count();
            lines + 2 // top + bottom border
        }
        Block::Math(src) => {
            crate::latex::to_unicode(src).lines().count().max(1) + 2
        }
        Block::Mermaid { src, diagram } => {
            let resolved = diagram
                .clone()
                .or_else(|| crate::mermaid::render(src));
            let content = resolved.as_deref().unwrap_or(src);
            content.lines().count().max(1) + 2
        }
        Block::List(list) => list_height(list, content_w, theme, hl),
        Block::BlockQuote { blocks, .. } => {
            blocks.iter().map(|b| block_height(b, content_w, theme, hl)).sum::<usize>()
        }
        Block::Table(table) => table_rows(table, theme).len().max(1),
        Block::Frontmatter(pairs) => frontmatter_rows(pairs, theme).len().max(1),
    }
}

fn list_height(list: &List, width: usize, theme: &MarkdownTheme, hl: &Highlighter) -> usize {
    let mut total = 0usize;
    for (idx, item) in list.items.iter().enumerate() {
        let marker = item_marker(idx, item, list.ordered, theme);
        let marker_w = display_width(&marker.0);
        let content_w = width.saturating_sub(marker_w).max(1);
        total += Spans::word_wrap(inline_runs(&item.inlines, theme, 0)).measure(content_w);
        if !item.children.is_empty() {
            let child_w = width.saturating_sub(marker_w).max(1);
            total += item
                .children
                .iter()
                .map(|c| block_height(c, child_w, theme, hl))
                .sum::<usize>();
        }
    }
    total.max(1)
}

fn block_to_view(block: &Block, theme: &MarkdownTheme, hl: &Highlighter) -> View {
    match block {
        Block::Heading { level, inlines } => heading_view(*level, inlines, theme),
        Block::Rule => rule_view(theme),
        Block::Paragraph(inlines) => paragraph_view(inlines, theme),
        Block::BlockQuote { kind, blocks } => blockquote_view(*kind, blocks, theme, hl),
        Block::List(list) => list_view(list, theme, hl),
        Block::CodeBlock { lang, code } => code_view(lang, code, theme, hl),
        Block::Math(src) => math_view(src, theme),
        Block::Mermaid { src, diagram } => mermaid_view(src, diagram.as_deref(), theme),
        Block::Table(table) => lines_leaf(table_rows(table, theme)),
        Block::Frontmatter(pairs) => lines_leaf(frontmatter_rows(pairs, theme)),
    }
}

fn heading_view(level: u8, inlines: &[Inline], theme: &MarkdownTheme) -> View {
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
    let text = inlines_to_string(inlines);
    let mut div = tags::div()
        // Stack the heading text and (optional) underline rule vertically;
        // the default row direction would place the rule beside the text and
        // collapse it to zero width, hiding the underline entirely.
        .flex_direction(FlexDirection::Column)
        .children(spans_leaf(vec![(text, style)], true));
    if level <= 2 {
        // H1/H2 get an underline rule, drawn as the heading box's own BOTTOM
        // border so it always reserves a row (a zero-height child div with a
        // TOP edge would collapse and never paint).
        div = div
            .border_style(BorderStyle::Single)
            .border_edges(Edges::BOTTOM)
            .border_color(color);
    }
    View::from(div)
}

fn paragraph_view(inlines: &[Inline], theme: &MarkdownTheme) -> View {
    spans_leaf(inline_runs(inlines, theme, 0), true)
}

fn rule_view(theme: &MarkdownTheme) -> View {
    View::from(
        tags::div()
            .border_style(BorderStyle::Single)
            .border_edges(Edges::TOP)
            .border_color(theme.rule_color),
    )
}

fn code_view(lang: &Option<String>, code: &str, theme: &MarkdownTheme, hl: &Highlighter) -> View {
    let lang_str = lang.as_deref().unwrap_or("");
    let label = if lang_str.trim().is_empty() {
        "text"
    } else {
        lang_str.trim()
    };
    let rows: Vec<Vec<(String, SpanStyle)>> = code
        .lines()
        .chain((code.is_empty()).then_some(""))
        .map(|line| highlight_tokens_to_runs(hl.highlight_line(line, lang_str)))
        .collect();
    let title = label_title(label, theme.code_text);
    framed(title, Lines::new(rows), theme.code_border)
}

fn math_view(src: &str, theme: &MarkdownTheme) -> View {
    let style = fg(theme.math_text);
    let rendered = crate::latex::to_unicode(src);
    let rows: Vec<Vec<(String, SpanStyle)>> = rendered
        .lines()
        .map(|l| vec![(l.to_string(), style)])
        .collect();
    framed(None, Lines::new(rows), theme.math_border)
}

fn mermaid_view(src: &str, diagram: Option<&str>, theme: &MarkdownTheme) -> View {
    let rendered = diagram.map(str::to_owned).or_else(|| crate::mermaid::render(src));
    let use_rendered = rendered.is_some();
    let content = rendered.as_deref().unwrap_or(src);
    let content_style = fg(theme.mermaid_text);
    let rows: Vec<Vec<(String, SpanStyle)>> = content
        .lines()
        .map(|line| {
            if use_rendered {
                vec![(line.to_string(), content_style)]
            } else {
                crate::mermaid::colorize_line(line, theme)
            }
        })
        .collect();
    let title = label_title("mermaid", theme.mermaid_label);
    framed(title, Lines::new(rows), theme.mermaid_border)
}

fn blockquote_view(
    kind: Option<pulldown_cmark::BlockQuoteKind>,
    blocks: &[Block],
    theme: &MarkdownTheme,
    hl: &Highlighter,
) -> View {
    let mut inner: Vec<View> = Vec::new();
    if let Some(k) = kind {
        inner.push(alert_header_view(k, theme));
    }
    for block in blocks {
        inner.push(block_to_view(block, theme, hl));
    }
    View::from(
        tags::div()
            .flex_direction(FlexDirection::Row)
            .children((
                // The quote bar column.
                tags::div()
                    .width(1)
                    .children(tags::p().color(theme.blockquote_marker).children("▏")),
                // Content column.
                tags::div()
                    .flex_grow(1.0)
                    .padding_left(1)
                    .flex_direction(FlexDirection::Column)
                    .children(inner),
            )),
    )
}

fn alert_header_view(kind: pulldown_cmark::BlockQuoteKind, theme: &MarkdownTheme) -> View {
    use pulldown_cmark::BlockQuoteKind;
    let color = match kind {
        BlockQuoteKind::Note => theme.alert_note,
        BlockQuoteKind::Tip => theme.alert_tip,
        BlockQuoteKind::Important => theme.alert_important,
        BlockQuoteKind::Warning => theme.alert_warning,
        BlockQuoteKind::Caution => theme.alert_caution,
    };
    let (icon, label) = alert_icon_label(kind);
    spans_leaf(
        vec![
            (format!("{icon} {label}"), SpanStyle {
                fg: Some(color),
                add_modifier: Modifier::BOLD,
                ..SpanStyle::default()
            },)],
        true,
    )
}

fn alert_icon_label(kind: pulldown_cmark::BlockQuoteKind) -> (&'static str, &'static str) {
    use pulldown_cmark::BlockQuoteKind;
    match kind {
        BlockQuoteKind::Note => ("[i]", "Note"),
        BlockQuoteKind::Tip => ("[*]", "Tip"),
        BlockQuoteKind::Important => ("[!]", "Important"),
        BlockQuoteKind::Warning => ("[!]", "Warning"),
        BlockQuoteKind::Caution => ("[x]", "Caution"),
    }
}

fn list_view(list: &List, theme: &MarkdownTheme, hl: &Highlighter) -> View {
    let mut items: Vec<View> = Vec::new();
    for (idx, item) in list.items.iter().enumerate() {
        let marker = item_marker(idx, item, list.ordered, theme);
        let marker_w = display_width(&marker.0) as i32;
        let content = inline_runs(&item.inlines, theme, 0);
        let mut item_children: Vec<View> = vec![View::from(
            tags::div()
                .flex_direction(FlexDirection::Row)
                .children((
                    tags::div().width(marker_w).children(
                        tags::p()
                            .color(marker.1.fg.unwrap_or(Color::Reset))
                            .children(marker.0),
                    ),
                    tags::div()
                        .flex_grow(1.0)
                        .children(spans_leaf(content, true)),
                )),
        )];
        // Nested children align under the item's content.
        if !item.children.is_empty() {
            let nested: Vec<View> = item
                .children
                .iter()
                .map(|c| block_to_view(c, theme, hl))
                .collect();
            item_children.push(View::from(
                tags::div()
                    .padding_left(marker_w)
                    .flex_direction(FlexDirection::Column)
                    .children(nested),
            ));
        }
        items.push(View::from(
            tags::div()
                .flex_direction(FlexDirection::Column)
                .children(item_children),
        ));
    }
    View::from(
        tags::div()
            .flex_direction(FlexDirection::Column)
            .children(items),
    )
}

// --- shared helpers ---

/// A framed box (framework border + optional title) wrapping a `Lines` body.
/// Used for code / math / mermaid blocks.
fn framed(title: Option<BorderTitleRuns>, body: Lines, border_color: Color) -> View {
    let mut div = tags::div()
        .border_style(BorderStyle::Single)
        .border_color(border_color)
        .padding_left(1)
        .padding_right(1);
    if let Some(title) = title {
        div = div.border_title(title);
    }
    View::from(div.children(View::leaf(Box::new(body))))
}

/// A bare `Lines` leaf — used for blocks (tables, frontmatter) whose `rows`
/// already carry their own frame characters, so wrapping them in another
/// `framed` would draw a double border.
fn lines_leaf(rows: Vec<Vec<(String, SpanStyle)>>) -> View {
    View::leaf(Box::new(Lines::new(rows)))
}

/// Build a `border_title` for a framed block: a leading space + bold label.
fn label_title(label: &str, color: Color) -> Option<BorderTitleRuns> {
    Some(vec![
        (Cow::Borrowed(" "), fg(color)),
        (
            Cow::Owned(label.to_string()),
            SpanStyle {
                fg: Some(color),
                add_modifier: Modifier::BOLD,
                ..SpanStyle::default()
            },
        ),
    ])
}

/// A word-wrapping `Spans` leaf from styled runs.
fn spans_leaf(runs: Vec<(String, SpanStyle)>, word_wrap: bool) -> View {
    let spans = if word_wrap {
        Spans::word_wrap(runs)
    } else {
        Spans::new(runs)
    };
    View::leaf(Box::new(spans))
}

/// Convert highlighter tokens `(text, maybe_color)` into styled runs.
fn highlight_tokens_to_runs(tokens: Vec<(String, Option<Color>)>) -> Vec<(String, SpanStyle)> {
    if tokens.is_empty() {
        return vec![("".to_string(), SpanStyle::default())];
    }
    tokens
        .into_iter()
        .map(|(text, color)| {
            let style = match color {
                Some(c) => fg(c),
                None => SpanStyle::default(),
            };
            (text, style)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_with_theme;
    use iodilos::node::TuiNode;

    fn render(src: &str) -> View {
        let theme = MarkdownTheme::default();
        let blocks = parse_with_theme(src, &theme);
        blocks_to_view(&blocks, &theme)
    }

    /// Walk a view tree and collect the plain text of every leaf producer.
    fn collect_text(nodes: &[TuiNode], out: &mut String) {
        for node in nodes {
            match node {
                TuiNode::Element(el) => collect_text(&el.children, out),
                TuiNode::Leaf { producer, .. } => out.push_str(&producer.borrow().plain_text()),
                TuiNode::Marker { slot: Some(c), .. } => collect_text(&c.borrow(), out),
                TuiNode::Marker { .. } => {}
            }
        }
    }

    fn text_of(view: &View) -> String {
        let mut s = String::new();
        collect_text(view.nodes(), &mut s);
        s
    }

    #[test]
    fn heading_text_present() {
        let view = render("# Title");
        assert!(text_of(&view).contains("Title"));
    }

    #[test]
    fn paragraph_text_present() {
        let view = render("hello world");
        assert!(text_of(&view).contains("hello"));
        assert!(text_of(&view).contains("world"));
    }

    #[test]
    fn code_block_body_present() {
        let view = render("```rust\nfn main() {}\n```");
        let text = text_of(&view);
        assert!(text.contains("fn main()"), "code body: {text}");
    }

    #[test]
    fn blockquote_content_present() {
        let view = render("> quoted text");
        assert!(text_of(&view).contains("quoted"));
    }

    #[test]
    fn list_items_present() {
        let view = render("- one\n- two");
        let text = text_of(&view);
        assert!(text.contains("one") && text.contains("two"));
    }

    #[test]
    fn table_content_present() {
        let view = render("| H1 | H2 |\n|----|----|\n| a  | b  |");
        let text = text_of(&view);
        assert!(text.contains("H1"), "header: {text}");
        assert!(text.contains("H2"));
    }

    #[test]
    fn render_sample_does_not_panic() {
        let sample = "# H\n\npara `code`.\n\n- a\n  - b\n- c\n\n> q\n\n---\n\n```rust\nfn x() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let _ = render(sample);
    }
}
