//! Pure run-builders + the `PromptView` producer that draws the whole prompt.
//!
//! The prompt is one borderless [`iodilos::producer::CellProducer`] leaf
//! (`PromptView`). The framework's own border model can't put the input text
//! *on* the rounded bottom edge `╰─ … ─╯` — `Edges::BOTTOM` is exclusive and
//! `content_rect` insets it, so the text always lands a row above an empty
//! bottom. `PromptView` instead emits every cell itself: the top statusline
//! `╭─ … ─╮`, vertical sides `│ … │` per input row, and the rounded bottom
//! `╰─ … ─╯` on the last (cursor) row. Input still re-wraps at the assigned
//! width via an internal `Spans`.
//!
//! The run-builders below (`statusline_runs`, `input_runs`) decide only *which
//! styled runs* feed the top edge and the input; `PromptView` shapes them and
//! wraps them in the frame glyphs.

use std::borrow::Cow;

use iodilos::framebuffer::Cell;
use iodilos::producer::row;
use iodilos::producer::{CellProducer, Spans};
use iodilos::style::BorderTitleRuns;
use iodilos::text::SpanStyle;
use iodilos::Color;
use unicode_width::UnicodeWidthChar;

use crate::statusline::StatusLine;
use crate::theme::PromptTheme;

fn fg(color: Color) -> SpanStyle {
    SpanStyle {
        fg: Some(color),
        ..SpanStyle::default()
    }
}

/// Build the styled runs painted on the prompt's top border (the statusline):
/// `─ π > ⬢ model > … ▶`. Each segment keeps its own colour. The leading `─ `
/// mirrors the original `╭─ ` look; the framework fills the remaining border
/// with the `─` top glyph in the frame colour.
pub fn statusline_runs(sl: &StatusLine, theme: &PromptTheme) -> BorderTitleRuns {
    let frame = fg(theme.frame);
    let dim = fg(theme.separator);
    let mut runs: BorderTitleRuns = Vec::new();
    runs.push((Cow::Borrowed("─ "), frame));
    runs.push((sl.brand.clone(), fg(sl.brand_color)));
    for f in &sl.fields {
        let fs = fg(f.color);
        runs.push((Cow::Borrowed(" > "), dim));
        runs.push((f.icon.clone(), fs));
        runs.push((Cow::Borrowed(" "), fs));
        runs.push((f.text.clone(), fs));
    }
    runs.push((Cow::Borrowed(" "), dim));
    runs.push((sl.tail.clone(), fg(sl.tail_color)));
    runs
}

/// Draw one input content row (already wrapped to the content width) flanked
/// by the left/right vertical bars, with a 2-space left indent so the text
/// lines up exactly under the bottom row's `╰─ ` indent. Layout:
///   `[│][ ][ ]<text...>[ padding ][│]`
/// The text starts at column 3 (after the 3-char left chrome), matching
/// [`bottom_row`], so wrapping a long line never shifts the previous line left.
fn flanked_row(content: &[Cell], inner_w: usize, bar_style: SpanStyle) -> Vec<Cell> {
    // Layout: [│][ ][ ]<text...blanks...>[ ][ ][│] — symmetric 3-cell chrome
    // on each side so wrapped lines line up with the bottom row's `╰─ ` /
    // ` ─╯` decoration. The text band is `inner_w - 4` cells; shorter content
    // pads with blanks.
    let mut row = Vec::with_capacity(inner_w + 2);
    row.push(row::glyph_cell('│', bar_style));
    row.push(row::glyph_cell(' ', bar_style));
    row.push(row::glyph_cell(' ', bar_style));
    let text_end = inner_w.saturating_sub(1); // index of the last text-band cell + 1
    row::extend_clamped(&mut row, content, text_end);
    row::pad(&mut row, text_end);
    row.push(row::glyph_cell(' ', bar_style));
    row.push(row::glyph_cell(' ', bar_style));
    row.push(row::glyph_cell('│', bar_style));
    row
}

/// The bottom row: `╰─ ` + the last input row + `─` fill + `╯`. The leading
/// `─ ` (matching the top `╭─ `) is part of the rounded look, and the text
/// starts at the same column as [`flanked_row`]'s text so the caret never
/// jumps when a line wraps.
fn bottom_row(content: &[Cell], inner_w: usize, style: SpanStyle) -> Vec<Cell> {
    // Layout: [╰][─][ ]<text...blanks...>[ ][─][╯]. The left and right
    // chrome are symmetric — `╰─ ` on the left and ` ─╯` on the right — so
    // the dash decoration sits flush against each corner with one space of
    // padding between it and the text band. The text band is `inner_w - 4`
    // cells wide; anything shorter pads with blanks (NOT dashes — the dash
    // run is just the corner decoration, not a fill-to-the-edge rule).
    let mut row = Vec::with_capacity(inner_w + 2);
    row.push(row::glyph_cell('╰', style));
    row.push(row::glyph_cell('─', style));
    row.push(row::glyph_cell(' ', style));
    // `Spans::render` pads each row to its full width with `Cell::default()`
    // (glyph = None), which renders as spaces. We pass `content` straight in;
    // the band fills with blanks past the text. Reserve the trailing ` ─╯`
    // (3 cells) by capping the extend at `inner_w - 1` (= `2 + (inner_w - 4) + 1`
    // accounting for the leading 3-cell chrome already pushed and the band).
    let text_end = inner_w.saturating_sub(1); // index where ` ` before `─╯` starts
    row::extend_clamped(&mut row, content, text_end);
    row::pad(&mut row, text_end);
    row.push(row::glyph_cell(' ', style));
    row.push(row::glyph_cell('─', style));
    row.push(row::glyph_cell('╯', style));
    row
}

/// The top statusline row: `╭` + the statusline runs (clipped to `inner_w`) +
/// `─` fill + `╮`.
fn top_row(status_runs: &[(Cow<'static, str>, SpanStyle)], inner_w: usize, frame: SpanStyle) -> Vec<Cell> {
    // Shape the statusline runs into column-indexed cells via `Spans`, then
    // keep only the leading non-padding glyphs (trim the trailing blank cells
    // `Spans` pads with) so the `─` fill below actually paints dashes, not the
    // empty cells the shaper left behind.
    let status_runs_owned: Vec<(String, SpanStyle)> = status_runs
        .iter()
        .map(|(t, s)| (t.to_string(), *s))
        .collect();
    let status_cells: Vec<Cell> = Spans::new(status_runs_owned)
        .render(inner_w.max(1))
        .into_iter()
        .next()
        .unwrap_or_default();
    // Statusline content ends at the last glyph cell; a blanked trailing cell
    // of a wide glyph is content (it must be copied), so we trim only the
    // trailing run of empty (no-glyph, default) cells.
    let trim_end = status_cells
        .iter()
        .rposition(|c| !matches!(c.glyph, None) || c.background.is_some())
        .map(|i| i + 1)
        .unwrap_or(0);
    let status_content = &status_cells[..trim_end.min(status_cells.len())];

    let mut row = Vec::with_capacity(inner_w + 2);
    row.push(row::glyph_cell('╭', frame));
    row::extend_clamped(&mut row, status_content, inner_w + 1);
    // Fill the remainder with `─` up to the right corner — the visible top
    // border line that continues the statusline to `╮`.
    row::pad_with(&mut row, inner_w + 1, row::glyph_cell('─', frame));
    row.push(row::glyph_cell('╮', frame));
    row
}

/// The prompt as one self-drawing producer.
///
/// Owns the pre-computed statusline runs, input runs (with the block cursor
/// already applied), and theme. `render(width)` lays out the rounded frame and
/// re-wraps the input at `width - 2` (inside the vertical bars). Height grows
/// with the input: the cursor always sits on the rounded bottom row.
#[derive(Clone, Debug)]
pub struct PromptView {
    status_runs: BorderTitleRuns,
    input_runs: Vec<(String, SpanStyle)>,
    frame_style: SpanStyle,
}

impl PromptView {
    pub fn new(
        statusline: &StatusLine,
        buffer: &str,
        cursor: usize,
        cursor_visible: bool,
        theme: &PromptTheme,
    ) -> Self {
        Self {
            status_runs: statusline_runs(statusline, theme),
            input_runs: input_runs(buffer, cursor, theme, cursor_visible),
            frame_style: fg(theme.frame),
        }
    }

    /// Wrap the input runs to the content width and return the shaped rows.
    ///
    /// The content width is `inner_w - 4`: every input row reserves a 3-cell
    /// chrome on each side (`╰─ `/`  │` on the bottom, `│  `/`  │` above),
    /// so the text starts at the same column on every line and never drifts
    /// when a line wraps. Reuses the framework's `Spans` producer for
    /// char-wrap + `\n` + wide-glyph handling.
    fn input_rows(&self, inner_w: usize) -> Vec<Vec<Cell>> {
        let content_w = inner_w.saturating_sub(4).max(1);
        Spans::new(self.input_runs.clone()).render(content_w)
    }
}

impl CellProducer for PromptView {
    fn measure(&self, width: usize) -> usize {
        let inner_w = width.saturating_sub(2).max(1);
        let input_rows = self.input_rows(inner_w).len().max(1);
        // 1 statusline row + input rows; never less than 2 (top + bottom).
        (1 + input_rows).max(2)
    }

    fn render(&self, width: usize) -> Vec<Vec<Cell>> {
        let width = width.max(2);
        let inner_w = (width - 2) as usize;
        let frame = self.frame_style;

        let mut rows = Vec::new();
        rows.push(top_row(&self.status_runs, inner_w, frame));

        let input = self.input_rows(inner_w.max(1));
        if input.is_empty() {
            // No input: just a rounded bottom with the leading `─ ` and fill.
            rows.push(bottom_row(&[], inner_w, frame));
        } else {
            let last = input.len() - 1;
            for (i, line) in input.iter().enumerate() {
                if i == last {
                    rows.push(bottom_row(line, inner_w, frame));
                } else {
                    rows.push(flanked_row(line, inner_w, frame));
                }
            }
        }
        rows
    }

    fn intrinsic_width(&self) -> usize {
        // The prompt fills its container width; report a nominal intrinsic
        // width so row-axis auto-sizing stays finite. Width is driven by the
        // layout, not the content. The `4 +` accounts for the 3-cell chrome
        // on each side (`╰─ `/` ─╯`) minus one for the right `─` that lives
        // inside the same band on the bottom row — kept as a small finite
        // baseline so a single-char input doesn't collapse the width.
        4 + self
            .input_runs
            .iter()
            .map(|(s, _)| {
                s.chars()
                    .filter(|&c| c != '\n')
                    .map(|c| c.width().unwrap_or(0).max(1))
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }

    fn plain_text(&self) -> String {
        let mut out = String::new();
        for (run, _) in &self.input_runs {
            out.push_str(run);
        }
        out
    }
}

/// Build the inline-styled runs for the prompt's editable text, with the block
/// cursor applied as a per-cell background.
///
/// The char at `cursor` (when it is a normal char) is covered by the cursor —
/// it gets `cursor_bg`/`cursor_fg`. When the cursor sits at end-of-input or
/// just before a `'\n'`, a trailing block (a styled space) is inserted so the
/// caret is always visible. `'\n'` in the buffer becomes a hard line break in
/// the produced [`iodilos::producer::Spans`]; long lines soft-wrap at the
/// layout width automatically.
///
/// When `cursor_visible` is false (the "off" phase of a blink), the cursor is
/// not drawn — no block background and no trailing block — matching how a
/// blinking caret disappears half the cycle. Pass a blink signal through to
/// implement a standard terminal-style blinking cursor.
pub fn input_runs(
    buffer: &str,
    cursor: usize,
    theme: &PromptTheme,
    cursor_visible: bool,
) -> Vec<(String, SpanStyle)> {
    let text = fg(theme.text);
    let cursor_style = SpanStyle {
        fg: Some(theme.cursor_fg),
        bg: Some(theme.cursor_bg),
        ..SpanStyle::default()
    };
    let chars: Vec<char> = buffer.chars().collect();
    let total = chars.len();
    let covered = (cursor < total && chars[cursor] != '\n').then_some(cursor);

    let mut runs: Vec<(String, SpanStyle)> = Vec::new();
    let mut push = |ch: char, style: SpanStyle| {
        if let Some((s, last_style)) = runs.last_mut() {
            if *last_style == style {
                s.push(ch);
                return;
            }
        }
        runs.push((ch.to_string(), style));
    };

    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\n' {
            if cursor_visible && covered.is_none() && i == cursor {
                push(' ', cursor_style);
            }
            push('\n', text);
            continue;
        }
        // The cursor cell: the block style when visible, else plain text.
        let cell_style = if cursor_visible && covered == Some(i) {
            cursor_style
        } else {
            text
        };
        push(ch, cell_style);
    }
    if cursor_visible && covered.is_none() && cursor == total {
        push(' ', cursor_style);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_input(buffer: &str, cursor: usize) -> Vec<(String, SpanStyle)> {
        input_runs(buffer, cursor, &PromptTheme::default(), true)
    }

    fn cursor_present(runs: &[(String, SpanStyle)]) -> bool {
        let theme = PromptTheme::default();
        runs.iter()
            .any(|(_, s)| s.bg == Some(theme.cursor_bg))
    }

    #[test]
    fn empty_buffer_has_trailing_cursor_block() {
        let runs = render_input("", 0);
        assert!(cursor_present(&runs), "empty buffer must show a cursor");
        let flat: String = runs.iter().map(|(s, _)| &**s).collect();
        assert_eq!(flat, " ");
    }

    #[test]
    fn cursor_covers_char_when_not_at_eol() {
        // "abc", cursor at 1 → 'b' is covered.
        let runs = render_input("abc", 1);
        assert!(cursor_present(&runs));
        let covered = runs
            .iter()
            .find(|(_, s)| s.bg == Some(PromptTheme::default().cursor_bg))
            .unwrap();
        assert_eq!(covered.0, "b");
    }

    #[test]
    fn cursor_at_eol_is_trailing_block() {
        let runs = render_input("abc", 3);
        assert!(cursor_present(&runs));
        // The trailing block is a styled space after "abc".
        let trailing = runs
            .iter()
            .find(|(_, s)| s.bg == Some(PromptTheme::default().cursor_bg))
            .unwrap();
        assert_eq!(trailing.0, " ");
    }

    #[test]
    fn cursor_hidden_phase_omits_block() {
        // Blink "off" phase: no cursor block anywhere, even at EOL where a
        // trailing block would normally be inserted.
        let runs = input_runs("abc", 3, &PromptTheme::default(), false);
        assert!(!cursor_present(&runs), "no cursor block when hidden");
        // The text is still all there, unstyled.
        let flat: String = runs.iter().map(|(s, _)| &**s).collect();
        assert_eq!(flat, "abc");
    }

    #[test]
    fn cursor_hidden_phase_covers_char_as_plain_text() {
        // Blink "off" with cursor mid-buffer: the covered char renders as plain
        // text (no block background), not as a cursor cell.
        let runs = input_runs("abc", 1, &PromptTheme::default(), false);
        assert!(!cursor_present(&runs));
        assert!(
            runs.iter()
                .all(|(_, s)| s.bg.is_none()),
            "no background styling when hidden: {runs:?}"
        );
    }

    #[test]
    fn newline_before_cursor_inserts_trailing_block() {
        // "a\nb", cursor at 1 (on the '\n') → trailing block before the break.
        let runs = render_input("a\nb", 1);
        let flat: String = runs.iter().map(|(s, _)| &**s).collect();
        assert!(flat.contains(" \n"), "trailing block before newline: {flat:?}");
    }

    #[test]
    fn statusline_contains_brand_fields_and_tail() {
        let sl = StatusLine::default_mock();
        let runs = statusline_runs(&sl, &PromptTheme::default());
        let flat: String = runs.iter().map(|(s, _)| &**s).collect();
        assert!(flat.contains("π"), "brand present: {flat}");
        assert!(flat.contains("MiMo-V2.5-Pro++"), "field present: {flat}");
        assert!(flat.contains("master"), "field present: {flat}");
        assert!(flat.contains("▶"), "tail present: {flat}");
    }

    #[test]
    fn statusline_leads_with_frame_dash() {
        let sl = StatusLine::default_mock();
        let runs = statusline_runs(&sl, &PromptTheme::default());
        let flat: String = runs.iter().map(|(s, _)| &**s).collect();
        assert!(
            flat.starts_with("─ "),
            "statusline should lead with the frame dash: {flat:?}"
        );
    }

    #[test]
    fn coalesces_consecutive_same_style_chars() {
        // "abc" cursor at 3 → one text run "abc" + one cursor run " ".
        let runs = render_input("abc", 3);
        assert_eq!(
            runs.len(),
            2,
            "three same-style chars coalesce into one run: {runs:?}"
        );
        assert_eq!(runs[0].0, "abc");
        assert_eq!(runs[1].0, " ");
    }

    // ---- PromptView producer: the whole self-drawn prompt frame ----------

    fn view(buffer: &str, cursor: usize) -> PromptView {
        PromptView::new(
            &StatusLine::default_mock(),
            buffer,
            cursor,
            true,
            &PromptTheme::default(),
        )
    }

    /// Flatten one shaped row of cells to its plain string (for assertions).
    fn row_text(row: &[Cell]) -> String {
        row.iter()
            .map(|c| c.glyph.as_ref().map(|g| g.value.as_str()).unwrap_or(""))
            .collect()
    }

    #[test]
    fn measure_is_two_for_empty_and_single_line() {
        // 1 statusline row + 1 input row (cursor) = 2; never less than 2.
        let v = view("", 0);
        assert_eq!(v.measure(40), 2, "empty buffer → 2 rows");
        let v = view("abc", 3);
        assert_eq!(v.measure(40), 2, "single line → 2 rows");
    }

    #[test]
    fn measure_grows_with_input_lines() {
        // "a\nb\nc" → 3 input rows + 1 statusline = 4.
        let v = view("a\nb\nc", 5);
        assert_eq!(v.measure(40), 4, "three lines → 4 rows");
    }

    #[test]
    fn measure_grows_with_wrapping() {
        // A line longer than the content width wraps to extra rows. The
        // content band is `inner_w - 4` cells (3-cell chrome on each side).
        let v = view("abcdefghij", 10);
        // width 8 → inner_w 6 → content_w 2 → 10 chars + 1 trailing cursor
        // block wrap to 6 rows of 2 → 1 statusline + 6 = 7.
        assert_eq!(v.measure(8), 7, "wrapped line grows height");
    }

    #[test]
    fn render_single_line_is_two_rows_with_input_on_bottom() {
        let v = view("a", 1);
        let rows = v.render(20);
        assert_eq!(rows.len(), 2, "single line → exactly 2 rows");
        let top = row_text(&rows[0]);
        let bottom = row_text(&rows[1]);
        assert!(top.starts_with('╭'), "top-left corner: {top:?}");
        assert!(top.ends_with('╮'), "top-right corner: {top:?}");
        assert!(
            bottom.starts_with("╰─ "),
            "bottom leads with rounded corner + dash: {bottom:?}"
        );
        assert!(bottom.ends_with('╯'), "bottom-right corner: {bottom:?}");
        // The input 'a' sits on the bottom row, NOT a separate middle row.
        assert!(bottom.contains('a'), "input on the bottom row: {bottom:?}");
    }

    /// Regression for "right side of the input row shows trailing spaces
    /// instead of `─╯`": with a short input like `"hi"` on a 20-cell row,
    /// the row should be left/right symmetric — `╰─ hi   ...spaces...   ─╯` —
    /// with the dash decoration sitting flush against each corner (one space
    /// inboard) and the middle band padded with blanks, NOT dashes.
    #[test]
    fn render_bottom_row_is_symmetric_dash_decoration() {
        let v = view("hi", 2);
        let rows = v.render(20);
        let bottom = row_text(&rows[1]);
        assert!(bottom.starts_with("╰─ hi"), "left chrome `╰─ ` + text: {bottom:?}");
        assert!(bottom.ends_with(" ─╯"), "right chrome ` ─╯`: {bottom:?}");
        // The text band between left-chrome and right-chrome is blanks, not
        // dashes — a stray `─` in the middle means the band fills with the
        // corner decoration rather than padding with spaces.
        let mid: String = bottom
            .chars()
            .skip_while(|&c| c != 'i')
            .skip(1) // past 'i'
            .take_while(|&c| c != '─') // up to the right-chrome dash
            .collect();
        assert!(
            mid.chars().all(|c| c == ' '),
            "middle band between text and right-chrome is blanks: {mid:?}"
        );
    }

    #[test]
    fn render_empty_buffer_has_no_middle_row() {
        let v = view("", 0);
        let rows = v.render(20);
        assert_eq!(rows.len(), 2, "empty → top + bottom, no middle row");
        let bottom = row_text(&rows[1]);
        assert!(bottom.starts_with('╰'), "bottom rounded: {bottom:?}");
        // The cursor block (a styled space) is on the bottom row.
        assert!(
            rows[1]
                .iter()
                .any(|c| c.glyph.as_ref().is_some_and(|g| g.style.bg == Some(
                    PromptTheme::default().cursor_bg
                ))),
            "cursor sits on the bottom row"
        );
    }

    #[test]
    fn render_multiline_has_vertical_sides_above_bottom() {
        let v = view("line1\nline2", 11);
        let rows = v.render(30);
        // 1 statusline + 2 input rows = 3.
        assert_eq!(rows.len(), 3);
        // Middle (first input) row flanked by vertical bars.
        let middle = row_text(&rows[1]);
        assert!(
            middle.starts_with('│') && middle.ends_with('│'),
            "middle row has vertical sides: {middle:?}"
        );
        assert!(middle.contains("line1"), "middle row holds line1: {middle:?}");
        // Last (cursor) row is the rounded bottom.
        let bottom = row_text(&rows[2]);
        assert!(
            bottom.starts_with('╰') && bottom.ends_with('╯'),
            "last row is rounded bottom: {bottom:?}"
        );
        assert!(bottom.contains("line2"), "bottom row holds line2: {bottom:?}");
    }

    #[test]
    fn render_content_text_starts_at_same_column_on_every_row() {
        // Regression: a wrapped line's text must start at the same column as the
        // cursor (bottom) row — previously the middle row rendered `│line1`
        // (text at col 1) while the bottom rendered `╰─ line2` (col 3), so
        // pressing Shift+Enter visibly shifted the previous line left.
        let v = view("line1\nline2", 11);
        let rows = v.render(30);
        let middle = &rows[1];
        let bottom = &rows[2];
        // The cell holding 'l' of line1 (middle) and 'l' of line2 (bottom)
        // must be at the same column index.
        let mid_text_col = middle
            .iter()
            .position(|c| c.glyph.as_ref().is_some_and(|g| g.value == "l"))
            .unwrap();
        let bot_text_col = bottom
            .iter()
            .position(|c| c.glyph.as_ref().is_some_and(|g| g.value == "l"))
            .unwrap();
        assert_eq!(
            mid_text_col, bot_text_col,
            "text columns align across rows (no left-drift on wrap): mid={}, bot={}",
            mid_text_col, bot_text_col
        );
        // Both rows are the full width.
        assert_eq!(middle.len(), 30);
        assert_eq!(bottom.len(), 30);
    }

    #[test]
    fn render_statusline_fills_right_side_with_dashes() {
        // Regression: the top border between the statusline tail and `╮` must
        // be `─` dashes, not blank cells. The bug was the shaped statusline
        // row carrying trailing padding cells that masked the `─` fill.
        let v = view("", 0);
        let rows = v.render(40);
        let top = &rows[0];
        assert_eq!(top.first().and_then(|c| c.glyph.as_ref()).map(|g| &g.value), Some(&"╭".to_string()));
        assert_eq!(top.last().and_then(|c| c.glyph.as_ref()).map(|g| &g.value), Some(&"╮".to_string()));
        // Every cell between the statusline content and `╮` that isn't a status
        // glyph must be a `─` (the visible top edge). Find the last non-─,
        // non-corner glyph (the tail), then assert all cells up to `╮` are `─`.
        let last_real = top
            .iter()
            .rposition(|c| {
                c.glyph.as_ref().is_some_and(|g| {
                    let v = &g.value;
                    !v.is_empty() && v != "─" && v != "╮" && v != "╭"
                })
            })
            .unwrap();
        for c in &top[last_real + 1..top.len() - 1] {
            assert_eq!(
                c.glyph.as_ref().map(|g| g.value.as_str()),
                Some("─"),
                "trailing top-border cells must be ─, got {:?}",
                c.glyph.as_ref().map(|g| &g.value)
            );
        }
    }

    #[test]
    fn render_rows_are_exactly_width_cells_wide() {
        let v = view("some long text here", 19);
        for width in [10usize, 20, 40] {
            let rows = v.render(width);
            for (i, row) in rows.iter().enumerate() {
                // Wide glyphs occupy 2 cells each; the cell count equals the
                // terminal column count the framework writes. Our frame rows
                // build to exactly `width` cells.
                assert_eq!(
                    row.len(),
                    width,
                    "row {i} is exactly {width} cells, got {}: {:?}",
                    row.len(),
                    row_text(row)
                );
            }
        }
    }

    #[test]
    fn render_statusline_is_clipped_when_too_long() {
        // A very narrow width: the statusline must clip, not overflow the row.
        let v = view("", 0);
        let rows = v.render(10);
        assert_eq!(rows.len(), 2);
        let top = row_text(&rows[0]);
        assert!(top.starts_with('╭'), "corner present: {top:?}");
        // No panic, row width respected (10 cells).
        assert_eq!(rows[0].len(), 10);
    }

    #[test]
    fn render_cjk_chars_have_no_space_between_them() {
        // Regression: 你好 must render as two adjacent wide glyphs (4 cells:
        // 你 + blank, 好 + blank) with NO spurious space between them. The bug
        // was `extend_clamped` re-adding a trailing blank the shaper already
        // added, producing 你 好 (5 cells).
        let v = view("你好", 2);
        let rows = v.render(20);
        let bottom = &rows[1];
        // Bottom layout: ╰ ─ <space> 你 <blank> 好 <blank> <─ fill…> ╯.
        // Index 3 = 你, 4 = blank, 5 = 好, 6 = blank.
        assert_eq!(bottom[3].glyph.as_ref().unwrap().value, "你");
        assert!(
            bottom[4].glyph.is_none(),
            "你 trailing blank at index 4, not a space: {:?}",
            bottom[4]
        );
        assert_eq!(
            bottom[5].glyph.as_ref().unwrap().value,
            "好",
            "好 immediately after 你's blank, no gap: {:?}",
            row_text(bottom)
        );
        assert!(bottom[6].glyph.is_none(), "好 trailing blank at index 6");
        // The two CJK chars take 4 columns total (3..7), no extra.
    }

    #[test]
    fn render_statusline_cjk_field_does_not_distort() {
        // A statusline with a CJK field must not shift subsequent fields.
        // Build a statusline whose first field is CJK and check the cells line
        // up by column count (wide glyph = glyph + blank).
        use crate::statusline::StatusField;
        let sl = StatusLine {
            brand: Cow::Borrowed("π"),
            brand_color: Color::Magenta,
            fields: vec![StatusField {
                icon: Cow::Borrowed("中"),
                text: Cow::Borrowed("文"),
                color: Color::Cyan,
            }],
            tail: Cow::Borrowed("▶"),
            tail_color: Color::DarkGrey,
        };
        let v = PromptView::new(&sl, "", 0, true, &PromptTheme::default());
        let rows = v.render(30);
        let top = &rows[0];
        assert_eq!(top.len(), 30, "top row is full width");
        assert_eq!(top[0].glyph.as_ref().unwrap().value, "╭");
        // The CJK icon 中 (width 2) must be followed by its blank, then 文.
        let flat = row_text(top);
        assert!(flat.contains('中'), "CJK icon rendered: {flat}");
        assert!(flat.contains('文'), "CJK text rendered: {flat}");
        assert!(
            flat.contains('▶'),
            "tail still present (not shifted off): {flat}"
        );
    }
}
