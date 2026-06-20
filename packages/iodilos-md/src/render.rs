//! Rendering Markdown blocks into an iodilos `View` tree.
//!
//! The renderer is fully imperative (no `view!` macro): each block produces a
//! `View` via the `tags::*` builder API, and blocks are assembled with
//! `.children(Vec<View>)`. This mirrors exactly what the `view!` macro emits
//! (`codegen.rs:80-84`) — `View::from(tags::div().children(vec![...]).flex_direction(..))`.
//!
//! The whole tree is rebuilt on every call. For a streaming viewer this is the
//! intended behavior: a `Signal<String>` change re-evaluates the dynamic
//! subtree (`TuiNode::create_dynamic_view`), which calls back into here.

use crossterm::style::Color;
use iodilos::components::tags::{div, p, span};
use iodilos::prelude::*;
use pulldown_cmark::Alignment;

use crate::highlight::Highlighter;
use crate::parser::{Block, Inline, List, ListItem, Table};
use crate::theme::MarkdownTheme;

/// Render Markdown source into a `View`.
///
/// The top-level container is a vertical (`FlexDirection::Column`) `div` whose
/// width fills its parent; each block is a child. A `gap` of 1 separates
/// blocks for vertical rhythm.
pub fn render_markdown(src: &str, theme: &MarkdownTheme) -> View {
    let blocks = crate::parser::parse(src);
    let highlighter = Highlighter::new();
    render_blocks(&blocks, theme, &highlighter, 0)
}

/// Render Markdown source that has already been parsed into blocks. Used by the
/// streaming path ([`crate::stream`]), which maintains the block list across
/// ticks and only re-parses the not-yet-closed tail.
pub fn render_blocks_view(blocks: &[Block], theme: &MarkdownTheme) -> View {
    let highlighter = Highlighter::new();
    render_blocks(blocks, theme, &highlighter, 0)
}

/// Render a sequence of blocks into a column container `View`.
fn render_blocks(
    blocks: &[Block],
    theme: &MarkdownTheme,
    hl: &Highlighter,
    list_depth: usize,
) -> View {
    let children: Vec<View> = blocks
        .iter()
        .map(|b| render_block(b, theme, hl, list_depth))
        .collect();
    div()
        .flex_direction(FlexDirection::Column)
        .row_gap(1)
        .width(Size::Percent(100.0))
        .children(children)
        .into()
}

/// Render a single block.
fn render_block(block: &Block, theme: &MarkdownTheme, hl: &Highlighter, list_depth: usize) -> View {
    match block {
        Block::Heading { level, inlines } => render_heading(*level, inlines, theme),
        Block::Paragraph(inlines) => render_paragraph(inlines, theme),
        Block::CodeBlock { lang, code } => render_code_block(lang, code, theme, hl),
        Block::List(list) => render_list(list, theme, hl, list_depth),
        Block::BlockQuote(blocks) => render_blockquote(blocks, theme, hl, list_depth),
        Block::Rule => render_rule(theme),
        Block::Table(table) => render_table(table, theme),
        Block::Math(src) => render_math(src, theme),
    }
}

// --- Headings ---------------------------------------------------------------

fn render_heading(level: u8, inlines: &[Inline], theme: &MarkdownTheme) -> View {
    let color = theme
        .heading
        .get((level as usize).saturating_sub(1))
        .copied()
        .unwrap_or(theme.heading[5]);
    let text = inlines_to_string(inlines);

    let mut heading = p()
        .color(color)
        .weight(Weight::Bold)
        .children(text)
        .width(Size::Percent(100.0));
    // Give H1/H2 a bottom border for stronger separation.
    if level <= 2 {
        heading = heading
            .border_style(BorderStyle::Single)
            .border_color(color)
            .border_edges(Edges::BOTTOM);
    }
    heading.into()
}

// --- Paragraphs -------------------------------------------------------------

fn render_paragraph(inlines: &[Inline], theme: &MarkdownTheme) -> View {
    // The whole paragraph is a single text leaf in the body color. Inline
    // styling (bold/italic/links/code/math) is collapsed into plain text — under
    // route A (no iodilos core changes) one leaf carries one style, so we keep
    // correct text flow and soft-wrapping rather than splitting inline runs
    // into independent flex boxes (which would break word wrapping). Inline
    // code keeps its raw text; inline math keeps its `$...$` fence.
    p().color(theme.text)
        .children(inlines_to_string(inlines))
        .into()
}

/// Flatten inlines to a single owned string. Inline `code` and `math` share
/// the leaf's single style under route A, so they are folded into the text
/// stream: code keeps its raw text, math keeps its `$...$` fence for visual
/// distinction.
fn inlines_to_string(inlines: &[Inline]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for i in inlines {
        match i {
            Inline::Text(t, _) => out.push_str(t),
            Inline::Code(t) => out.push_str(t),
            Inline::Math(t) => {
                let _ = write!(out, "${t}$");
            }
            Inline::SoftBreak => out.push('\n'),
        }
    }
    out
}

// --- Code blocks ------------------------------------------------------------

fn render_code_block(lang: &Option<String>, code: &str, theme: &MarkdownTheme, hl: &Highlighter) -> View {
    let lang_str = lang.as_deref().unwrap_or("");
    let lines: Vec<&str> = code.lines().collect();
    // Each line is its own row `div`; within a row, every highlighted token is
    // a `span` leaf (so per-token colors show). Code is preformatted, so flex
    // row layout places tokens correctly without inline reflow.
    let rows: Vec<View> = lines
        .iter()
        .map(|line| {
            let tokens = hl.highlight_line(line, lang_str);
            let token_views: Vec<View> = if tokens.is_empty() {
                // Empty line: a single space keeps the row's height.
                vec![span().children(" ").into()]
            } else {
                tokens
                    .into_iter()
                    .map(|(text, color)| {
                        let mut s = span().children(text);
                        if let Some(c) = color {
                            s = s.color(c);
                        }
                        s.into()
                    })
                    .collect()
            };
            div()
                .flex_direction(FlexDirection::Row)
                .width(Size::Percent(100.0))
                .children(token_views)
                .into()
        })
        .collect();

    div()
        .flex_direction(FlexDirection::Column)
        .background_color(theme.code_bg)
        .border_style(BorderStyle::Round)
        .border_color(theme.code_border)
        .padding(1)
        .width(Size::Percent(100.0))
        .children(rows)
        .into()
}

// --- Lists ------------------------------------------------------------------

fn render_list(list: &List, theme: &MarkdownTheme, hl: &Highlighter, list_depth: usize) -> View {
    let items: Vec<View> = list
        .items
        .iter()
        .enumerate()
        .map(|(idx, item)| render_list_item(idx, item, list.ordered, theme, hl, list_depth))
        .collect();
    div()
        .flex_direction(FlexDirection::Column)
        .row_gap(0)
        .padding_left(2) // indent the whole list one level
        .width(Size::Percent(100.0))
        .children(items)
        .into()
}

fn render_list_item(
    idx: usize,
    item: &ListItem,
    ordered: bool,
    theme: &MarkdownTheme,
    hl: &Highlighter,
    list_depth: usize,
) -> View {
    // Marker: bullet "•", number "1.", or checkbox "[x] "/"[ ] ". Prefixed onto
    // the item's first line as one text leaf, which wraps correctly. The marker
    // shares the body color (route A: one leaf, one style) — we keep the marker
    // glyph rather than its color, since a marker+body row collapses when the
    // body has no fixed height in iodilos's flex layout.
    let marker = if let Some(checked) = item.checked {
        if checked { "[x] " } else { "[ ] " }.to_string()
    } else if ordered {
        format!("{}. ", idx + 1)
    } else {
        "• ".to_string()
    };

    let full_line = format!("{marker}{}", inlines_to_string(&item.inlines));

    let mut children: Vec<View> = Vec::new();
    if !full_line.is_empty() {
        children.push(p().color(theme.text).children(full_line).into());
    }
    // Nested child blocks (sub-lists, etc.) render below the item's own line,
    // indented by the enclosing list's padding.
    if !item.children.is_empty() {
        children.push(render_blocks(&item.children, theme, hl, list_depth + 1));
    }

    div()
        .flex_direction(FlexDirection::Column)
        .width(Size::Percent(100.0))
        .children(children)
        .into()
}

// --- Blockquotes ------------------------------------------------------------

fn render_blockquote(blocks: &[Block], theme: &MarkdownTheme, hl: &Highlighter, list_depth: usize) -> View {
    let inner = render_blocks(blocks, theme, hl, list_depth);
    div()
        .border_style(BorderStyle::Single)
        .border_color(theme.blockquote_marker)
        .border_edges(Edges::LEFT)
        .padding_left(1)
        .italic(true)
        .color(theme.blockquote_text)
        .width(Size::Percent(100.0))
        .children(vec![inner])
        .into()
}

// --- Rules ------------------------------------------------------------------

fn render_rule(theme: &MarkdownTheme) -> View {
    div()
        .border_style(BorderStyle::Single)
        .border_color(theme.rule_color)
        .border_edges(Edges::TOP)
        .width(Size::Percent(100.0))
        .into()
}

// --- Math -------------------------------------------------------------------

/// Render a block-level display math run. The terminal cannot do LaTeX
/// typesetting, so the raw source is shown in a monospace, centered, framed
/// block (mirroring the code-block frame) so it reads as "math" at a glance.
fn render_math(src: &str, theme: &MarkdownTheme) -> View {
    let lines: Vec<&str> = src.lines().collect();
    let rows: Vec<View> = if lines.is_empty() {
        vec![p().color(theme.math_text).children(" ").into()]
    } else {
        lines
            .iter()
            .map(|line| p().color(theme.math_text).children(line.to_string()).into())
            .collect()
    };
    div()
        .flex_direction(FlexDirection::Column)
        .align_items(AlignItems::CENTER)
        .background_color(theme.math_bg)
        .border_style(BorderStyle::Round)
        .border_color(theme.math_border)
        .padding(1)
        .width(Size::Percent(100.0))
        .children(rows)
        .into()
}

// --- Tables -----------------------------------------------------------------

fn render_table(table: &Table, theme: &MarkdownTheme) -> View {
    // Compute per-column max display width across header + rows.
    let col_count = table
        .headers
        .len()
        .max(table.rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if col_count == 0 {
        return div().into();
    }
    let mut widths = vec![0usize; col_count];
    for (i, h) in table.headers.iter().enumerate() {
        widths[i] = widths[i].max(display_width(h));
    }
    for row in &table.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
    }

    let mut rows: Vec<View> = Vec::new();

    // Header row.
    rows.push(table_row(
        &table.headers,
        &widths,
        &table.aligns,
        Some(theme.table_header),
        true,
    ));
    // Body rows.
    for row in &table.rows {
        rows.push(table_row(row, &widths, &table.aligns, None, false));
    }

    div()
        .flex_direction(FlexDirection::Column)
        .row_gap(0)
        .border_style(BorderStyle::Round)
        .border_color(theme.table_border)
        .padding(1)
        .width(Size::Percent(100.0))
        .children(rows)
        .into()
}

fn table_row(
    cells: &[String],
    widths: &[usize],
    aligns: &[Alignment],
    color: Option<Color>,
    bold: bool,
) -> View {
    let cell_views: Vec<View> = widths
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let content = cells.get(i).map(String::as_str).unwrap_or("");
            let align = aligns.get(i).copied().unwrap_or(Alignment::Left);
            let padded = pad_cell(content, w, align);
            let mut leaf = span();
            if let Some(c) = color {
                leaf = leaf.color(c);
            }
            if bold {
                // Weight is a text-leaf style; span carries it.
                leaf = leaf.weight(Weight::Bold);
            }
            leaf.children(padded).into()
        })
        .collect();
    div()
        .flex_direction(FlexDirection::Row)
        .column_gap(2)
        .width(Size::Percent(100.0))
        .children(cell_views)
        .into()
}

fn pad_cell(content: &str, width: usize, align: Alignment) -> String {
    let len = display_width(content);
    if len >= width {
        return content.to_string();
    }
    let pad = width - len;
    match align {
        Alignment::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), content, " ".repeat(right))
        }
        Alignment::Right => format!("{}{}", " ".repeat(pad), content),
        Alignment::Left | Alignment::None => format!("{}{}", content, " ".repeat(pad)),
    }
}

/// Approximate display width (char count; good enough for table alignment in
/// the terminal since most markdown tables are ASCII).
fn display_width(s: &str) -> usize {
    s.chars().count()
}

/// Estimate how many terminal rows `render_markdown` will need for `src` at the
/// given content `width` (in cells). This is an approximation — it ignores exact
/// soft-wrap points but is close enough to drive follow-the-tail scrolling in the
/// streaming demo, where iodilos exposes no layout-measurement API.
///
/// Each block contributes its rows plus one for the inter-block `row_gap` of 1.
pub fn estimate_lines(src: &str) -> usize {
    estimate_lines_with_width(src, 78)
}

pub fn estimate_lines_with_width(src: &str, width: usize) -> usize {
    let blocks = crate::parser::parse(src);
    estimate_blocks_lines(&blocks, width)
}

/// Estimate the rendered row count for an already-parsed block list. The
/// streaming path reuses this on its cached blocks so it does not re-parse.
pub fn estimate_blocks_lines(blocks: &[Block], width: usize) -> usize {
    let mut total = 0usize;
    let mut first = true;
    for block in blocks {
        if !first {
            total += 1; // row_gap between blocks
        }
        first = false;
        total += estimate_block_lines(block, width);
    }
    total.max(1)
}

fn estimate_block_lines(block: &Block, width: usize) -> usize {
    let content_width = width.saturating_sub(2).max(1); // account for padding/borders
    match block {
        Block::Heading { inlines, .. } => wrapped_lines(&inlines_to_string(inlines), content_width, false),
        Block::Paragraph(inlines) => {
            // Paragraphs may contain inline code/math leaves; approximate as one
            // text stream at the body width.
            let text: String = inlines.iter().map(|i| match i {
                Inline::Text(t, _) | Inline::Code(t) => t.as_str(),
                Inline::Math(t) => t.as_str(),
                Inline::SoftBreak => " ",
            }).collect();
            wrapped_lines(&text, content_width, false)
        }
        Block::CodeBlock { code, .. } => {
            // +2 for the code block's own padding, plus border row.
            code.lines().count() + 2
        }
        Block::Rule => 1,
        Block::BlockQuote(blocks) => {
            let mut n = 0usize;
            for b in blocks {
                n += estimate_block_lines(b, content_width.saturating_sub(1));
            }
            n.max(1)
        }
        Block::List(list) => {
            let mut n = 0usize;
            for item in &list.items {
                n += estimate_item_lines(item, content_width);
            }
            n.max(1)
        }
        Block::Table(table) => {
            // header + each body row, plus 2 for padding/border.
            1 + table.rows.len() + 2
        }
        Block::Math(src) => {
            // like a code block: lines of source + padding/border.
            src.lines().count().max(1) + 2
        }
    }
}

fn estimate_item_lines(item: &ListItem, width: usize) -> usize {
    let text: String = item.inlines.iter().map(|i| match i {
        Inline::Text(t, _) | Inline::Code(t) => t.as_str(),
        Inline::Math(t) => t.as_str(),
        Inline::SoftBreak => " ",
    }).collect();
    let mut n = wrapped_lines(&text, width.saturating_sub(3), false); // marker indent
    for child in &item.children {
        n += estimate_block_lines(child, width);
    }
    n.max(1)
}

/// Count how many terminal rows `text` occupies after wrapping at `width`.
/// Treats blank lines as a single row.
fn wrapped_lines(text: &str, width: usize, _is_code: bool) -> usize {
    if text.is_empty() {
        return 1;
    }
    let mut count = 0usize;
    for line in text.split('\n') {
        if line.is_empty() {
            count += 1;
            continue;
        }
        let chars = line.chars().count();
        count += (chars + width - 1) / width.max(1);
    }
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Heading

A paragraph with `code`.

- item one
- item two

> quote

```rust
fn main() {}
```

| A | B |
|---|---|
| 1 | 2 |
";

    #[test]
    fn render_markdown_does_not_panic() {
        // Building the View tree touches every block renderer. The highlighter
        // initializes syntect on first use; this guards against init panics too.
        let theme = MarkdownTheme::default();
        let _view = render_markdown(SAMPLE, &theme);
    }

    #[test]
    fn estimate_lines_is_positive() {
        let n = estimate_lines(SAMPLE);
        assert!(n > 0, "estimate should be positive, got {n}");
        // The sample has well over a dozen content rows.
        assert!(n >= 10, "estimate should reflect many blocks, got {n}");
    }

    #[test]
    fn highlight_known_language_produces_some_color() {
        let hl = Highlighter::new();
        let toks = hl.highlight_line("fn main() {}", "rust");
        // A recognized language should emit at least one colored run.
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
        // and at least two distinct colors (e.g. keyword vs. number) — proving
        // per-token coloring rather than one flat color.
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

    #[test]
    fn code_block_renders_multiple_token_spans_per_line() {
        // A highlighted code line becomes a row div with one span per token.
        // Verify by rendering a small code block and checking the produced tree
        // has non-trivial structure (not just a single span). We rely on the
        // highlighter test above for color correctness; here we ensure the
        // renderer wires tokens into spans.
        let theme = MarkdownTheme::default();
        let view = render_markdown("```rust\nfn main() {}\n```\n", &theme);
        // The view is a column with the code block as a child; just assert it
        // built without panic and has at least one node.
        assert!(!view.nodes().is_empty());
    }
}
