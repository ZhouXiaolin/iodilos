//! Render parsed Markdown `Block`s into an iodilos `TextSurface`.
//!
//! This is the X-flat renderer (ADR: iodilos text-flow): every block's chrome
//! (heading underline, code-block frame, table alignment, blockquote bar, list
//! indent) is drawn as styled segment characters into the surface.

use iodilos::Color;
use iodilos::producer::Lines;
use iodilos::text::{Modifier, SpanStyle};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::highlight::Highlighter;
use crate::parser::{Block, Inline, List, ListItem, Table};
use crate::theme::MarkdownTheme;
use crate::wrap::wrap_inline_runs;

/// Render a full markdown source into a text surface at `width` (cells).
pub fn render_to_surface(src: &str, width: usize, theme: &MarkdownTheme) -> Lines {
    let blocks = crate::parser::parse_with_theme(src, theme);
    render_blocks_to_surface(&blocks, width, theme)
}

/// Render already-parsed blocks into a text surface. Used by the streaming
/// path, which keeps its own block list across ticks.
pub fn render_blocks_to_surface(
    blocks: &[Block],
    width: usize,
    theme: &MarkdownTheme,
) -> Lines {
    let hl = Highlighter::new();
    let mut out = Vec::new();
    let mut first = true;
    for block in blocks {
        if !first {
            out.push(vec![]); // blank-line rhythm between blocks
        }
        first = false;
        render_block(block, width, theme, &hl, 0, &mut out);
    }
    Lines::new(out)
}

fn render_block(
    block: &Block,
    width: usize,
    theme: &MarkdownTheme,
    hl: &Highlighter,
    blockquote_depth: usize,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    match block {
        Block::Heading { level, inlines } => render_heading(*level, inlines, theme, out),
        Block::Rule => render_rule(theme, width, out),
        Block::Paragraph(inlines) => render_paragraph(inlines, width, theme, blockquote_depth, out),
        Block::BlockQuote { kind, blocks } => {
            render_blockquote(*kind, blocks, width, theme, hl, blockquote_depth, out)
        }
        Block::List(list) => render_list(list, width, theme, hl, blockquote_depth, 0, out),
        Block::CodeBlock { lang, code } => render_code_block(lang, code, width, theme, hl, out),
        Block::Math(src) => render_math(src, width, theme, out),
        Block::Mermaid { src, diagram } => {
            render_mermaid(src, diagram.as_deref(), width, theme, out)
        }
        Block::Table(table) => render_table(table, width, theme, out),
        Block::Frontmatter(pairs) => render_frontmatter(pairs, width, theme, out),
    }
}

/// Flatten inlines to a single owned string: headings use it for single-style
/// text, and the streaming path uses it to read a paragraph as plain text.
pub(crate) fn inlines_to_string(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t, _) => s.push_str(t),
            Inline::Code(t) => s.push_str(t),
            Inline::Math(t) => s.push_str(&crate::latex::to_unicode(t)),
            Inline::SoftBreak => s.push('\n'),
        }
    }
    s
}

fn render_heading(level: u8, inlines: &[Inline], theme: &MarkdownTheme, out: &mut Vec<Vec<(String, SpanStyle)>>) {
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
    // Flatten inlines into one styled segment; heading text is single-style.
    let text = inlines_to_string(inlines);
    out.push(vec![(text, style)]);
    // H1/H2 get a underline rule.
    if level <= 2 {
        let bar = "─".repeat(width_for_heading(level));
        out.push(vec![(bar,
            SpanStyle {
                fg: Some(color),
                ..SpanStyle::default()
            }, )]);
    }
}

fn width_for_heading(level: u8) -> usize {
    if level == 1 { 40 } else { 20 }
}

fn render_rule(theme: &MarkdownTheme, width: usize, out: &mut Vec<Vec<(String, SpanStyle)>>) {
    let bar = "─".repeat(width.max(1));
    out.push(vec![(bar,
        SpanStyle {
            fg: Some(theme.rule_color),
            ..SpanStyle::default()
        }, )]);
}

fn render_paragraph(
    inlines: &[Inline],
    width: usize,
    theme: &MarkdownTheme,
    blockquote_depth: usize,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    let runs = inline_runs(inlines, theme, blockquote_depth);
    let lines = wrap_inline_runs(runs, &[], &[], width);
    out.extend(lines);
}

/// Convert parsed `Inline`s into styled `TextSegment`s (one segment per run; inline code
/// and math get their own themed style).
pub(crate) fn inline_runs(
    inlines: &[Inline],
    theme: &MarkdownTheme,
    blockquote_depth: usize,
) -> Vec<(String, SpanStyle)> {
    let _ = blockquote_depth; // already baked into Text style by the parser
    let mut spans = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(t, st) => {
                if t.is_empty() {
                    continue;
                }
                spans.push((t.clone(), *st));
            }
            Inline::Code(t) => {
                spans.push((format!(" {t} "),
                    SpanStyle {
                        fg: Some(theme.code_text),
                        // No background: keep inline code on the same
                        // background as the surrounding text so it doesn't
                        // paint a grey block (which left visible residue when
                        // the surface scrolled).
                        ..SpanStyle::default()
                    }, ));
            }
            Inline::Math(t) => {
                let u = crate::latex::to_unicode(t);
                spans.push((format!(" {u} "),
                    SpanStyle {
                        fg: Some(theme.math_text),
                        ..SpanStyle::default()
                    }, ));
            }
            Inline::SoftBreak => {
                spans.push((" ".to_string(), SpanStyle::default()));
            }
        }
    }
    spans
}

fn render_blockquote(
    kind: Option<pulldown_cmark::BlockQuoteKind>,
    blocks: &[Block],
    width: usize,
    theme: &MarkdownTheme,
    hl: &Highlighter,
    blockquote_depth: usize,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    let depth = blockquote_depth + 1;
    let bar = ("▏ ".to_string(),
        SpanStyle {
            fg: Some(theme.blockquote_marker),
            ..SpanStyle::default()
        }, );
    let prefix = vec![bar];

    // A GFM alert (`> [!NOTE]` …) opens with a colored, bold header line
    // carrying its icon and label, before the quote's body.
    if let Some(k) = kind {
        use pulldown_cmark::BlockQuoteKind;
        let color = match k {
            BlockQuoteKind::Note => theme.alert_note,
            BlockQuoteKind::Tip => theme.alert_tip,
            BlockQuoteKind::Important => theme.alert_important,
            BlockQuoteKind::Warning => theme.alert_warning,
            BlockQuoteKind::Caution => theme.alert_caution,
        };
        let (icon, label) = alert_icon_label(k);
        out.push(vec![
            ("▏ ".to_string(),
                SpanStyle {
                    fg: Some(color),
                    ..SpanStyle::default()
                }, ),
            (format!("{icon} {label}"),
                SpanStyle {
                    fg: Some(color),
                    add_modifier: Modifier::BOLD,
                    ..SpanStyle::default()
                }, ),
        ]);
    }

    // Render inner blocks into a temp buffer, then prepend the bar to each line.
    let mut inner = Vec::new();
    let mut first = true;
    for block in blocks {
        if !first {
            inner.push(vec![]);
        }
        first = false;
        render_block(block, width.saturating_sub(2), theme, hl, depth, &mut inner);
    }
    for mut line in inner {
        let mut spans = prefix.clone();
        spans.append(&mut line);
        out.push(spans);
    }
}

/// GFM alert icon + label per kind (mirrors leaf's `alert_icon_label`).
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

fn render_list(
    list: &List,
    width: usize,
    theme: &MarkdownTheme,
    hl: &Highlighter,
    blockquote_depth: usize,
    indent: usize,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    let indent_str = " ".repeat(indent);
    for (idx, item) in list.items.iter().enumerate() {
        let marker = item_marker(idx, item, list.ordered, theme);
        // First-line prefix = indent (if any) + marker. Omit a zero-width indent
        // span so the marker is genuinely the first span at the top level.
        let mut first_prefix: Vec<(String, SpanStyle)> = Vec::new();
        if indent > 0 {
            first_prefix.push((indent_str.clone(), SpanStyle::default()));
        }
        first_prefix.push(marker.clone());
        // Continuation indent = indent + marker visual width.
        let marker_w = UnicodeWidthStr::width(marker.0.as_str());
        let cont_indent = " ".repeat(indent + marker_w);
        let cont_prefix = vec![(cont_indent.to_string(), SpanStyle::default())];

        let runs = inline_runs(&item.inlines, theme, blockquote_depth);
        let lines = wrap_inline_runs(runs, &first_prefix, &cont_prefix, width);
        out.extend(lines);

        // Nested children render indented further.
        if !item.children.is_empty() {
            let mut inner = Vec::new();
            let mut first = true;
            let child_indent = indent + marker_w;
            for child in &item.children {
                if !first {
                    inner.push(vec![]);
                }
                first = false;
                render_block(
                    child,
                    width.saturating_sub(child_indent),
                    theme,
                    hl,
                    blockquote_depth,
                    &mut inner,
                );
            }
            // Child blocks align with the wrapped continuation text of this item.
            let nest_indent = " ".repeat(child_indent);
            for mut line in inner {
                let mut spans = vec![(nest_indent.clone(), SpanStyle::default())];
                spans.append(&mut line);
                out.push(spans);
            }
        }
    }
}

pub(crate) fn item_marker(idx: usize, item: &ListItem, ordered: bool, theme: &MarkdownTheme) -> (String, SpanStyle) {
    let (text, color) = if let Some(checked) = item.checked {
        (
            if checked {
                "✔ ".to_string()
            } else {
                "☐ ".to_string()
            },
            theme.task_marker,
        )
    } else if ordered {
        (format!("{}. ", idx + 1), theme.list_marker)
    } else {
        ("• ".to_string(), theme.list_marker)
    };
    (text,
        SpanStyle {
            fg: Some(color),
            ..SpanStyle::default()
        }, )
}

fn render_code_block(
    lang: &Option<String>,
    code: &str,
    width: usize,
    theme: &MarkdownTheme,
    hl: &Highlighter,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    let lang_str = lang.as_deref().unwrap_or("");
    let label = if lang_str.trim().is_empty() {
        "text"
    } else {
        lang_str.trim()
    };
    let mut body = Vec::new();
    for line in code.lines().chain((code.is_empty()).then_some("")) {
        let tokens = hl.highlight_line(line, lang_str);
        body.push(if tokens.is_empty() {
            vec![("".to_string(), SpanStyle::default())]
        } else {
            tokens
                .into_iter()
                .map(|(text, color)| {
                    let style = match color {
                        Some(c) => SpanStyle {
                            fg: Some(c),
                            ..SpanStyle::default()
                        },
                        None => SpanStyle::default(),
                    };
                    (text, style)
                })
                .collect()
        });
    }
    render_framed_block(
        Some(label),
        body,
        width,
        theme.code_border,
        theme.code_text,
        out,
    );
}

fn render_math(src: &str, width: usize, theme: &MarkdownTheme, out: &mut Vec<Vec<(String, SpanStyle)>>) {
    let style = SpanStyle {
        fg: Some(theme.math_text),
        ..SpanStyle::default()
    };
    let rendered = crate::latex::to_unicode(src);
    let all_lines: Vec<&str> = rendered.lines().collect();
    let start = all_lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(0);
    let end = all_lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(start, |e| e + 1);
    let body = if all_lines.is_empty() {
        vec![vec![("".to_string(), style)]]
    } else {
        all_lines[start..end]
            .iter()
            .map(|line| vec![((*line).to_string(), style)])
            .collect()
    };
    render_framed_block(None, body, width, theme.math_border, theme.math_text, out);
}

fn render_mermaid(
    src: &str,
    diagram: Option<&str>,
    width: usize,
    theme: &MarkdownTheme,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    // Prefer a pre-resolved diagram (streaming sticky cache); otherwise parse.
    let rendered = diagram
        .map(str::to_owned)
        .or_else(|| crate::mermaid::render(src));
    let use_rendered = rendered.is_some();
    let content = rendered.as_deref().unwrap_or(src);
    let content_style = SpanStyle {
        fg: Some(theme.mermaid_text),
        ..SpanStyle::default()
    };
    let mut body: Vec<Vec<(String, SpanStyle)>> = content
        .lines()
        .map(|line| {
            if use_rendered {
                vec![(line.to_string(), content_style)]
            } else {
                crate::mermaid::colorize_line(line, theme)
            }
        })
        .collect();
    if body.is_empty() {
        body.push(vec![("".to_string(), SpanStyle::default())]);
    }
    render_framed_block(
        Some("mermaid"),
        body,
        width,
        theme.mermaid_border,
        theme.mermaid_label,
        out,
    );
}

fn render_framed_block(
    label: Option<&str>,
    body: Vec<Vec<(String, SpanStyle)>>,
    width: usize,
    border_color: Color,
    label_color: Color,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    let label_width = label.map(display_width).unwrap_or(0);
    let frame_width = width.max(4).max(label_width + 5);
    let inner_width = frame_width.saturating_sub(2);
    let content_width = inner_width.saturating_sub(2);
    let border_style = SpanStyle {
        fg: Some(border_color),
        ..SpanStyle::default()
    };
    let label_style = SpanStyle {
        fg: Some(label_color),
        add_modifier: Modifier::BOLD,
        ..SpanStyle::default()
    };
    if let Some(label) = label {
        let header_fill = inner_width.saturating_sub(label_width + 3);
        out.push(vec![
            ("┌─ ".to_string(), border_style),
            (format!("{label} "), label_style),
            (format!("{}┐", "─".repeat(header_fill)), border_style),
        ]);
    } else {
        out.push(vec![(format!("┌{}┐", "─".repeat(inner_width)), border_style)]);
    }
    for line in body {
        let mut spans = vec![
            ("│".to_string(), border_style),
            (" ".to_string(), SpanStyle::default()),
        ];
        spans.extend(fit_segments_to_width(line, content_width));
        spans.push((" ".to_string(), SpanStyle::default()));
        spans.push(("│".to_string(), border_style));
        out.push(spans);
    }
    out.push(vec![(format!("└{}┘", "─".repeat(inner_width)), border_style)]);
}

fn fit_segments_to_width(segments: Vec<(String, SpanStyle)>, target_width: usize) -> Vec<(String, SpanStyle)> {
    let mut out = Vec::new();
    let mut used = 0usize;

    for segment in segments {
        if used >= target_width {
            break;
        }
        let mut text = String::new();
        let mut text_width = 0usize;
        for ch in segment.0.chars() {
            let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
            if used + text_width + width > target_width {
                break;
            }
            text.push(ch);
            text_width += width;
        }
        if !text.is_empty() {
            out.push((text, segment.1));
            used += text_width;
        }
    }

    if used < target_width {
        out.push((" ".repeat(target_width - used), SpanStyle::default()));
    }
    out
}

fn render_table(table: &Table, width: usize, theme: &MarkdownTheme, out: &mut Vec<Vec<(String, SpanStyle)>>) {
    use pulldown_cmark::Alignment;
    let col_count = table
        .headers
        .len()
        .max(table.rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if col_count == 0 {
        return;
    }
    // Natural column widths = widest cell in each column.
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
    // Fit the table to `width`: total = Σ(widths[i] + 2 padding) + (col_count+1)
    // bars. If it overflows, shrink columns proportionally (each kept ≥ 1) so
    // long cells wrap inside their cell instead of blowing the frame past the
    // terminal. This keeps the table shape intact on resize.
    widths = fit_column_widths(&widths, width);

    let border_style = SpanStyle {
        fg: Some(theme.table_border),
        ..SpanStyle::default()
    };
    let bar = ("│".to_string(), border_style);
    let header_style = SpanStyle {
        fg: Some(theme.table_header),
        add_modifier: Modifier::BOLD,
        ..SpanStyle::default()
    };
    out.push(table_rule("┌", "─", "┬", "┐", &widths, border_style));

    // Render one logical row (header or data). Each cell is word-wrapped to its
    // column width; cells in the same row are padded to the tallest so the bar
    // on the right lines up across every visual line of the row.
    let push_row = |out: &mut Vec<Vec<(String, SpanStyle)>>, cells: &[String], style: SpanStyle| {
        let wrapped: Vec<Vec<String>> = (0..col_count)
            .map(|i| {
                let content = cells.get(i).map(String::as_str).unwrap_or("");
                wrap_cell(content, widths[i])
            })
            .collect();
        let max_lines = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        let aligns: Vec<Alignment> = (0..col_count)
            .map(|i| table.aligns.get(i).copied().unwrap_or(Alignment::Left))
            .collect();
        for line_idx in 0..max_lines {
            let mut spans = vec![bar.clone()];
            for i in 0..col_count {
                let cell_line = wrapped[i].get(line_idx).map(String::as_str).unwrap_or("");
                let padded = pad_cell(cell_line, widths[i], aligns[i]);
                spans.push((" ".to_string(), SpanStyle::default()));
                spans.push((padded, style));
                spans.push((" ".to_string(), SpanStyle::default()));
                spans.push(bar.clone());
            }
            out.push(spans);
        }
    };
    push_row(out, &table.headers, header_style);
    // Header/body separator row (╞══╪══╡), aligned to the column widths so the
    // crossings sit under the column bars. Matches leaf's table rendering.
    out.push(table_rule("╞", "═", "╪", "╡", &widths, border_style));
    for (i, row) in table.rows.iter().enumerate() {
        // Inner horizontal rule (├──┼──┤) between body rows: the first row
        // follows the header separator directly; each later row is preceded
        // by a rule so every data row sits in its own framed cell instead of
        // stacking against the next with no border between them.
        if i > 0 {
            out.push(table_rule("├", "─", "┼", "┤", &widths, border_style));
        }
        push_row(out, row, SpanStyle::default());
    }
    out.push(table_rule("└", "─", "┴", "┘", &widths, border_style));
}

/// The rendered width of a table given its column `widths`: each column costs
/// its content width + 2 padding cells, plus one `│` bar per column boundary.
fn table_total_width(widths: &[usize]) -> usize {
    widths.iter().map(|w| w + 2).sum::<usize>() + widths.len() + 1
}

/// Shrink `natural` column widths so the table fits in `width` cells, keeping
/// each column ≥ 1. Columns that already fit are returned unchanged. Shrinking
/// is proportional to the natural width so wide columns give up more cells
/// than narrow ones. The result never exceeds `width` (when `width` is large
/// enough to hold the natural table, the natural widths are returned as-is —
/// we do not stretch columns to fill the terminal).
fn fit_column_widths(natural: &[usize], width: usize) -> Vec<usize> {
    let cols = natural.len();
    if cols == 0 {
        return Vec::new();
    }
    let total = table_total_width(natural);
    if total <= width {
        return natural.to_vec();
    }
    // Budget for column content = width minus the fixed chrome (bars + padding).
    // chrome = (cols + 1) bars + 2 padding * cols.
    let chrome = (cols + 1) + 2 * cols;
    let target = width.saturating_sub(chrome).max(cols); // ≥ 1 per column
    // Iteratively reclaim: while the sum exceeds the target and some column is
    // still > 1, shave one cell off the widest column. This converges and keeps
    // every column ≥ 1, distributing the shrink to whichever column is widest
    // at each step (proportional in effect).
    let mut w = natural.to_vec();
    while w.iter().sum::<usize>() > target && w.iter().any(|&c| c > 1) {
        // Find the widest column (first one on ties) and shrink it.
        let idx = w
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| **c)
            .map(|(i, _)| i)
            .expect("at least one column > 1");
        w[idx] -= 1;
    }
    w
}

/// Word-wrap a cell's text to `width` cells, breaking at spaces when possible
/// and hard-breaking any single token longer than `width`. Returns one or more
/// lines (never empty: an empty cell yields a single empty line).
fn wrap_cell(content: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for source_line in content.split('\n') {
        if source_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut col = 0usize;
        for word in source_line.split_whitespace() {
            let word_w = display_width(word);
            // A word that itself exceeds the width hard-breaks char-by-char.
            if word_w > width {
                // Flush whatever is pending onto its own line first.
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    col = 0;
                }
                let mut rest = word;
                while display_width(rest) > width {
                    let (head, tail) = split_at_width(rest, width);
                    lines.push(head.to_string());
                    rest = tail;
                }
                if !rest.is_empty() {
                    current.push_str(rest);
                    col = display_width(rest);
                }
                continue;
            }
            let sep = if current.is_empty() { 0 } else { 1 };
            if col + sep + word_w > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
                col = word_w;
            } else {
                if sep == 1 {
                    current.push(' ');
                }
                current.push_str(word);
                col += sep + word_w;
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Split `s` at `width` display cells, returning `(head, tail)` on a character
/// boundary (never splitting a wide char). `head` measures exactly ≤ `width`.
fn split_at_width(s: &str, width: usize) -> (&str, &str) {
    let mut used = 0usize;
    for (i, ch) in s.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if used + cw > width {
            return (&s[..i], &s[i..]);
        }
        used += cw;
    }
    (s, "")
}

fn table_rule(
    left: &str,
    fill: &str,
    cross: &str,
    right: &str,
    widths: &[usize],
    style: SpanStyle,
) -> Vec<(String, SpanStyle)> {
    let mut spans = vec![(left.to_string(), style)];
    for (i, w) in widths.iter().enumerate() {
        spans.push((fill.repeat(w + 2), style));
        spans.push((if i + 1 < widths.len() { cross } else { right }.to_string(),
            style, ));
    }
    spans
}

fn render_frontmatter(
    pairs: &[(String, String)],
    _width: usize,
    theme: &MarkdownTheme,
    out: &mut Vec<Vec<(String, SpanStyle)>>,
) {
    use pulldown_cmark::Alignment;
    if pairs.is_empty() {
        return;
    }
    // Two-column key|value table (key bold), mirroring leaf's frontmatter.
    let key_w = pairs
        .iter()
        .map(|(k, _)| display_width(k))
        .max()
        .unwrap_or(0);
    let val_w = pairs
        .iter()
        .map(|(_, v)| display_width(v))
        .max()
        .unwrap_or(0);
    let border = SpanStyle {
        fg: Some(theme.table_border),
        ..SpanStyle::default()
    };
    let key_style = SpanStyle {
        fg: Some(theme.table_header),
        add_modifier: Modifier::BOLD,
        ..SpanStyle::default()
    };
    let bar = ("│".to_string(), border);
    for (k, v) in pairs {
        out.push(vec![
            bar.clone(),
            (pad_cell(k, key_w, Alignment::Left), key_style),
            bar.clone(),
            (pad_cell(v, val_w, Alignment::Left), SpanStyle::default()),
            bar.clone(),
        ]);
    }
}

fn pad_cell(content: &str, width: usize, align: pulldown_cmark::Alignment) -> String {
    let len = display_width(content);
    if len >= width {
        return content.to_string();
    }
    let pad = width - len;
    match align {
        pulldown_cmark::Alignment::Center => {
            let l = pad / 2;
            format!("{}{}{}", " ".repeat(l), content, " ".repeat(pad - l))
        }
        pulldown_cmark::Alignment::Right => format!("{}{}", " ".repeat(pad), content),
        pulldown_cmark::Alignment::Left | pulldown_cmark::Alignment::None => {
            format!("{}{}", content, " ".repeat(pad))
        }
    }
}

pub(crate) fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Render a single table into its styled-run rows at natural column widths
/// (no shrinking to a target width). Used by the View-tree renderer, which
/// hands the rows to a `Lines` leaf and lets `overflow: hidden` clip if the
/// terminal is narrower than the natural table.
pub(crate) fn table_rows(table: &Table, theme: &MarkdownTheme) -> Vec<Vec<(String, SpanStyle)>> {
    let mut out = Vec::new();
    // A very large width disables `fit_column_widths` shrinking (natural widths).
    render_table(table, usize::MAX, theme, &mut out);
    out
}

/// Render frontmatter key/value pairs into styled-run rows.
pub(crate) fn frontmatter_rows(
    pairs: &[(String, String)],
    theme: &MarkdownTheme,
) -> Vec<Vec<(String, SpanStyle)>> {
    let mut out = Vec::new();
    render_frontmatter(pairs, usize::MAX, theme, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(row: &Vec<(String, SpanStyle)>) -> String {
        row.iter().map(|s| s.0.as_str()).collect()
    }

    fn row_widths(surface: &Lines) -> Vec<usize> {
        surface
            .rows
            .iter()
            .map(|row| display_width(&row_text(row)))
            .collect()
    }

    #[test]
    fn heading_renders_bold_colored_line() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("# Title", 40, &theme);
        assert!(!lines.rows.is_empty());
        let first = &lines.rows[0];
        assert_eq!(first.len(), 1);
        assert!(
            first[0]
                .1
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(first[0].1.fg, Some(theme.heading[0]));
    }

    #[test]
    fn rule_renders_bar_line() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("---", 10, &theme);
        let bar_line = &lines.rows[0];
        let s: String = bar_line
            
            .iter()
            .map(|s| s.0.as_str())
            .collect();
        assert_eq!(s, "──────────");
    }

    #[test]
    fn paragraph_renders_inline_runs_as_spans() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("hello world", 40, &theme);
        // one paragraph line; its spans carry the body color.
        let para = &lines.rows[0];
        let text: String = para.iter().map(|s| s.0.as_str()).collect();
        assert!(text.contains("hello"), "text present: {text}");
        assert!(text.contains("world"), "text present: {text}");
        assert!(
            para.iter().all(|s| s.1.fg == Some(theme.text)),
            "body color on every span: {para:?}"
        );
    }

    #[test]
    fn paragraph_bold_run_keeps_strong_color() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("**bold**", 40, &theme);
        let para = &lines.rows[0];
        let bold_span = para
            
            .iter()
            .find(|s| s.0.as_str() == "bold")
            .expect("bold span");
        assert_eq!(bold_span.1.fg, Some(theme.strong_text));
        assert!(bold_span.1.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn blockquote_draws_bar_prefix() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("> quoted text here", 40, &theme);
        // First non-empty line carries the blockquote bar span.
        let l = lines.rows.iter()
            .find(|l| !l.is_empty())
            .expect("a line");
        let first_span = &l[0];
        assert!(
            first_span.0.contains('▏'),
            "expected blockquote bar, got {:?}",
            first_span.0
        );
    }

    #[test]
    fn alert_blockquote_renders_icon_and_label() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("> [!NOTE]\n> body text", 40, &theme);
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str())
            .collect();
        assert!(
            joined.contains("Note"),
            "GFM alert should render its label: {joined}"
        );
        assert!(
            joined.contains("body text"),
            "alert body still rendered: {joined}"
        );
    }

    #[test]
    fn unordered_list_draws_bullet_markers() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("- one\n- two\n- three", 40, &theme);
        let bullets = lines.rows.iter()
            .filter(|l| {
                l
                    .first()
                    .is_some_and(|s| s.0.as_str().starts_with("•"))
            })
            .count();
        assert_eq!(bullets, 3, "three bullet markers: {lines:?}");
    }

    // NOTE: `task_list_draws_checkbox` (asserting raw `[x]`/`[ ]`) and
    // `framed_block_inside_task_list_aligns_to_continuation_indent`
    // (asserting a 4-space child indent for task items) were removed: both
    // were stale and contradicted the shipped renderer. Task-list checkboxes
    // render as ✔/☐ glyphs (see `task_list_uses_glyph_markers_not_brackets`
    // in stream.rs and `item_marker` in render.rs), and the continuation
    // indent is `indent + rendered_marker_width` — for a task item the
    // rendered marker is `"✔ "` (width 2), so children indent by 2, matching
    // a regular bullet, with no code path producing 4.

    #[test]
    fn code_block_emits_highlighted_spans_with_frame() {
        let theme = MarkdownTheme::default();
        let src = "```rust\nfn main() {}\n```\n";
        let lines = render_to_surface(src, 40, &theme);
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str().to_string())
            .collect();
        assert!(joined.contains('│'), "frame side bars: {joined}");
        assert!(joined.contains("fn main()"), "code text present: {joined}");
        // At least one span is colored (rust highlighting).
        assert!(
            lines.rows.iter()
                .flat_map(|l| l.iter())
                .any(|s| s.1.fg.is_some()),
            "some highlighted span"
        );
    }

    #[test]
    fn code_block_frame_rows_have_consistent_width() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("```rust\nfn main() {}\nlet x = 1;\n```", 32, &theme);
        let widths = row_widths(&lines);
        assert_eq!(widths, vec![32, 32, 32, 32], "aligned code frame");
        assert!(row_text(&lines.rows[0]).starts_with("┌─ rust "));
        assert!(row_text(&lines.rows[3]).starts_with('└'));
    }

    #[test]
    fn math_block_renders_unicode_in_frame() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("$$\\int_0^1 x\\,dx$$\n", 40, &theme);
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str().to_string())
            .collect();
        assert!(
            joined.contains('∫') && joined.contains('₀') && joined.contains('¹'),
            "math source converted to unicode: {joined}"
        );
    }

    #[test]
    fn math_block_frame_rows_have_consistent_width() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("$$\\frac{a+b}{c}$$\n", 30, &theme);
        let widths = row_widths(&lines);
        assert_eq!(widths, vec![30, 30, 30], "aligned math frame");
        assert!(
            !row_text(&lines.rows[0]).contains("latex"),
            "display math frame should not show an explicit language label"
        );
        assert!(row_text(&lines.rows[2]).ends_with('┘'));
    }

    #[test]
    fn mermaid_block_renders_flowchart_frame() {
        let theme = MarkdownTheme::default();
        let src =
            "```mermaid\nflowchart TD\n    A[Start] --> B{Ready?}\n    B -->|yes| C[Ship]\n```";
        let lines = render_to_surface(src, 46, &theme);
        let widths = row_widths(&lines);
        assert!(
            widths.iter().all(|w| *w == 46),
            "aligned mermaid frame widths: {widths:?}"
        );
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str())
            .collect();
        assert!(joined.contains("mermaid"), "label present: {joined}");
    }

    #[test]
    fn table_renders_aligned_columns() {
        let theme = MarkdownTheme::default();
        let src = "| H1 | H2 |\n|----|----|\n| a  | b  |\n";
        let lines = render_to_surface(src, 40, &theme);
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str().to_string())
            .collect();
        assert!(joined.contains("H1"), "header present: {joined}");
        assert!(joined.contains('│'), "column bars: {joined}");
    }

    #[test]
    fn table_renders_separator_between_header_and_body() {
        let theme = MarkdownTheme::default();
        let src = "| H1 | H2 |\n|----|----|\n| a  | b  |";
        let lines = render_to_surface(src, 40, &theme);
        // A separator row (horizontal rules joined by crossings) sits between
        // the header and the first data row, matching leaf's table rendering.
        let has_sep = lines.rows.iter().any(|l| {
            let t: String = l.iter().map(|s| s.0.as_str()).collect();
            t.contains('═') && t.contains('╪')
        });
        assert!(has_sep, "expected a header/body separator row: {lines:?}");
    }

    #[test]
    fn table_renders_closed_frame_with_aligned_rows() {
        let theme = MarkdownTheme::default();
        let src = "| H1 | H2 |\n|----|----|\n| a  | b  |";
        let lines = render_to_surface(src, 40, &theme);
        let texts: Vec<String> = lines.rows.iter().map(row_text).collect();
        let widths = row_widths(&lines);

        assert_eq!(texts.first().map(String::as_str), Some("┌────┬────┐"));
        assert_eq!(texts.last().map(String::as_str), Some("└────┴────┘"));
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "table frame rows should align: {texts:?}"
        );
    }

    #[test]
    fn table_never_overflows_render_width() {
        // A long cell that would naturally exceed the terminal width must be
        // shrunk/wrapped so every rendered row fits inside `width`.
        let theme = MarkdownTheme::default();
        let src = "| name | description |\n|------|--------------|\n| alpha | this is a very long description that would blow past a narrow terminal width |\n";
        let width = 30;
        let lines = render_to_surface(src, width, &theme);
        let widths = row_widths(&lines);
        let max = *widths.iter().max().unwrap_or(&0);
        assert!(
            max <= width,
            "every table row must fit in width {width}, but a row was {max} cells wide: {:?}",
            lines.rows.iter().map(row_text).collect::<Vec<_>>()
        );
        // The long content must still be present (wrapped, not truncated).
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str())
            .collect();
        assert!(
            joined.contains("long") && joined.contains("description"),
            "wrapped content should still be present: {joined}"
        );
    }

    #[test]
    fn table_keeps_frame_shape_when_wrapping() {
        // When cells wrap to multiple visual lines, the frame stays rectangular:
        // every row still starts with │ and ends with │, and the top/bottom
        // rules are present and closed.
        let theme = MarkdownTheme::default();
        let src = "| a | b |\n|---|---|\n| short | a considerably longer cell value |\n";
        let lines = render_to_surface(src, 24, &theme);
        let texts: Vec<String> = lines.rows.iter().map(row_text).collect();
        let first = texts.first().cloned().unwrap_or_default();
        let last = texts.last().cloned().unwrap_or_default();
        assert!(first.starts_with('┌') && first.ends_with('┐'), "top rule: {first:?}");
        assert!(last.starts_with('└') && last.ends_with('┘'), "bottom rule: {last:?}");
        for (i, t) in texts.iter().enumerate() {
            if i == 0 || i == texts.len() - 1 {
                continue;
            }
            // Skip the horizontal separator rule rows (they use ─/═/┼, not │…│).
            if t.contains('─') || t.contains('═') {
                continue;
            }
            assert!(t.starts_with('│'), "data row must start with bar: {t:?}");
            assert!(t.ends_with('│'), "data row must end with bar: {t:?}");
        }
    }

    #[test]
    fn table_shrinks_columns_when_width_too_small() {
        // Direct unit test for the column-width fitter: a table whose natural
        // width overflows `width` gets every column shrunk so the total fits.
        let natural = vec![20usize, 30];
        let fitted = fit_column_widths(&natural, 24);
        let total = table_total_width(&fitted);
        assert!(
            total <= 24,
            "fitted table total {total} must fit in 24: {fitted:?}"
        );
        assert!(
            fitted.iter().all(|&w| w >= 1),
            "every column kept ≥ 1: {fitted:?}"
        );
    }

    #[test]
    fn table_fit_leaves_natural_widths_when_they_fit() {
        // When the terminal is wide enough, the natural widths are returned
        // unchanged (no stretching to fill the width).
        let natural = vec![3usize, 4];
        let fitted = fit_column_widths(&natural, 80);
        assert_eq!(fitted, natural, "natural widths preserved when they fit");
    }

    #[test]
    fn frontmatter_renders_as_key_value_table() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("---\ntitle: Hi\nauthor: Sol\n---\n\nbody", 40, &theme);
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str())
            .collect();
        // Rendered as a two-column key|value table (mirrors leaf's frontmatter).
        assert!(joined.contains('│'), "frontmatter table borders: {joined}");
        assert!(joined.contains("title"), "key rendered: {joined}");
        assert!(joined.contains("Hi"), "value rendered: {joined}");
    }

    #[test]
    fn inline_math_converts_latex_to_unicode() {
        let theme = MarkdownTheme::default();
        let lines = render_to_surface("the $x^2$ term", 40, &theme);
        let joined: String = lines.rows.iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str())
            .collect();
        assert!(
            joined.contains('²'),
            "inline math x^2 should convert to x²: {joined}"
        );
        assert!(
            !joined.contains('$'),
            "inline math delimiters should be hidden by default: {joined}"
        );
    }

    const SAMPLE: &str = "# H\n\npara `code`.\n\n- a\n  - b\n- c\n\n> q\n\n---\n\n```rust\nfn x() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";

    #[test]
    fn render_sample_to_lines_does_not_panic() {
        let theme = MarkdownTheme::default();
        let _lines = render_to_surface(SAMPLE, 60, &theme);
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

    #[test]
    fn table_cell_with_inline_code_keeps_content() {
        // Inline `code` inside a table cell used to be dropped (only `Text`
        // events were routed to the cell buffer), so `| `path` | desc |`
        // rendered an empty first column. Guard against that regression.
        let theme = MarkdownTheme::default();
        let src = "| 路径 | 说明 |\n|------|------|\n| `Cargo.toml` | 工作空间根配置 |\n| `docs/` | 架构决策记录 |\n";
        let surface = render_to_surface(src, 60, &theme);
        let joined: String = surface
            .rows
            .iter()
            .flat_map(|r| r.iter())
            .map(|s| s.0.as_str())
            .collect();
        assert!(
            joined.contains("Cargo.toml"),
            "inline-code cell content must be rendered: {joined}"
        );
        assert!(
            joined.contains("docs/"),
            "inline-code cell content must be rendered: {joined}"
        );
        assert!(
            joined.contains("工作空间根配置"),
            "sibling text cell preserved: {joined}"
        );
    }
}
