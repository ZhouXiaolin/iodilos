//! Cell producers: the two-phase leaf model.
//!
//! A leaf view node holds a [`CellProducer`] instead of pre-shaped text. Taffy's
//! measure callback asks `measure(width)` for the content height at a given
//! width; the paint path asks `render(width)` for the shaped [`Cell`] rows.
//! This is what makes [`Cell`](crate::framebuffer::Cell) the single terminal
//! type — there is no intermediate row/segment representation between the
//! component and the framebuffer.
//!
//! The reactive closure still only reads signals and produces a producer (or a
//! producer that swaps its text); the layout/paint two-phase split is what
//! resolves the "I need a width to shape, but width comes from layout" cycle.

use unicode_width::UnicodeWidthChar;

use crate::framebuffer::{Cell, Glyph};
use crate::text::SpanStyle;

/// A leaf content source that shapes itself into [`Cell`] rows at a given
/// width. `measure` answers taffy's content-sizing question ("how tall at this
/// width?"); `render` answers the paint question ("give me the cells").
pub trait CellProducer {
    /// The number of terminal rows this content occupies when shaped at
    /// `width` columns. Used by the taffy measure callback for auto-height.
    fn measure(&self, width: usize) -> usize;

    /// Shape the content into `Vec<Vec<Cell>>` (one inner vector per terminal
    /// row, each exactly `width` cells wide). The paint path writes these rows
    /// into the framebuffer with clipping/scroll applied by the caller.
    fn render(&self, width: usize) -> Vec<Vec<Cell>>;

    /// The intrinsic display width of the content (the widest single line, in
    /// terminal cells). Used for row-axis (horizontal) auto-sizing, where taffy
    /// needs the content's natural width rather than a height-at-width answer.
    fn intrinsic_width(&self) -> usize;

    /// A best-effort unstyled plain-text dump, for `collect_text` and tests.
    /// Producers that carry structured rows may return an approximate value.
    fn plain_text(&self) -> String {
        String::new()
    }
}

/// Reusable, wide-glyph-correct row-construction primitives.
///
/// The cell invariant every shaped row obeys: **each [`Cell`] is exactly one
/// terminal column**, and a wide glyph (CJK, emoji — display width 2) is stored
/// as a `[glyph cell, blanked trailing cell]` pair (the trailing cell is
/// `Cell::default()`, `glyph: None`). The framebuffer writer relies on this: it
/// advances two columns past a wide glyph and skips emitting the blanked cell.
///
/// Custom producers that build rows by hand (borders, statuslines, prompt
/// frames) must honour this invariant — in particular they must **never
/// re-add** a trailing blank cell that an upstream shaper (`Spans`, `Lines`)
/// already added, or CJK text renders with a spurious space between every
/// glyph. Use these helpers instead of re-deriving the rule.
pub mod row {
    use super::*;

    /// Build a single-column glyph cell. For a width-1 frame/border character
    /// this is the whole cell; for content, prefer [`push_glyph`] (which adds
    /// the trailing blank for wide glyphs).
    pub fn glyph_cell(ch: char, style: SpanStyle) -> Cell {
        Cell {
            background: None,
            glyph: Some(Glyph {
                value: ch.to_string(),
                style,
            }),
        }
    }

    /// The display width of a cell: 1 for a normal glyph or a blanked trailing
    /// cell, or the glyph's full width for a wide glyph. Use this when
    /// budgeting columns — it is the authoritative answer.
    pub fn cell_width(cell: &Cell) -> usize {
        cell.glyph.as_ref().map(|g| g.width()).unwrap_or(1)
    }

    /// Push one glyph into `buf` plus its trailing blank cell when the glyph is
    /// wide. This is the canonical "one glyph → N cells" primitive; every cell
    /// added advances the column count by exactly the glyph's display width.
    pub fn push_glyph(buf: &mut Vec<Cell>, ch: char, style: SpanStyle) {
        let cw = ch.width().unwrap_or(0).max(1);
        buf.push(glyph_cell(ch, style));
        if cw > 1 {
            buf.push(Cell::default());
        }
    }

    /// Pad `row` with empty cells up to `width`. Used to make every shaped row
    /// a uniform `width` cells.
    pub fn pad(row: &mut Vec<Cell>, width: usize) {
        while row.len() < width {
            row.push(Cell::default());
        }
    }

    /// Pad `row` with copies of `fill` up to `width`. Use for borders where the
    /// fill glyph (e.g. `─`) must carry a style.
    pub fn pad_with(row: &mut Vec<Cell>, width: usize, fill: Cell) {
        while row.len() < width {
            row.push(fill.clone());
        }
    }

    /// Copy column-indexed cells from `src` onto the end of `dst`, stopping at
    /// `max` total columns. Wide-glyph safe: `src` is assumed to already carry
    /// its trailing blank cells (as `Spans`/`Lines` produce), so each source
    /// cell advances the column count by exactly 1 and no trailing blank is
    /// re-added. A wide glyph that would straddle the `max` boundary is dropped
    /// (a half-wide glyph at the row edge would mis-render).
    pub fn extend_clamped(dst: &mut Vec<Cell>, src: &[Cell], max: usize) {
        for cell in src {
            if dst.len() >= max {
                break;
            }
            let cw = cell_width(cell);
            if cw > 1 && dst.len() + cw > max {
                break;
            }
            dst.push(cell.clone());
        }
    }
}

/// A single-style plain-text producer: the common case for `From<&str>`,
/// numeric leaves, and dynamic strings. Characters wrap at `width` (character
/// wrapping, matching the previous surface char-wrap). Each glyph keeps its
/// `width` cells — a wide character occupies two cells, the second blanked.
#[derive(Clone, Debug, Default)]
pub struct Plain {
    /// The source text (may contain `\n` to force line breaks).
    pub text: String,
    /// The single style applied to every glyph.
    pub style: SpanStyle,
}

impl Plain {
    /// Construct a plain producer with the default (empty) style.
    pub fn new<T: Into<String>>(text: T) -> Self {
        Self {
            text: text.into(),
            style: SpanStyle::default(),
        }
    }

    /// Construct a plain producer with a style.
    pub fn styled<T: Into<String>>(text: T, style: SpanStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

impl CellProducer for Plain {
    fn measure(&self, width: usize) -> usize {
        shape_plain(&self.text, self.style, width).len().max(1)
    }

    fn render(&self, width: usize) -> Vec<Vec<Cell>> {
        let width = width.max(1);
        let mut rows = shape_plain(&self.text, self.style, width);
        if rows.is_empty() {
            rows.push(vec![]);
        }
        rows
    }

    fn intrinsic_width(&self) -> usize {
        self.text
            .split('\n')
            .map(|line| {
                line.chars()
                    .map(|c| c.width().unwrap_or(0).max(1))
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }

    fn plain_text(&self) -> String {
        self.text.clone()
    }
}

/// Pre-wrapped rows of styled runs: the shape `iodilos-md` and `iodilos-prompt`
/// produce (word-wrapped upstream, each run carrying its own [`SpanStyle`] for
/// syntax-highlight colors, inline-code/math colors, frame borders, and the
/// block cursor). The height is fixed by the row count (wrapping is already
/// done); `render` only shapes each run into glyphs at the given width, clipping
/// rather than re-wrapping (the producer's own width was the wrap target).
#[derive(Clone, Debug, Default)]
pub struct Lines {
    /// One entry per terminal row; each entry is a list of `(run, style)`.
    pub rows: Vec<Vec<(String, SpanStyle)>>,
}

impl Lines {
    /// Construct from already-wrapped styled runs.
    pub fn new(rows: Vec<Vec<(String, SpanStyle)>>) -> Self {
        Self { rows }
    }
}

impl CellProducer for Lines {
    fn measure(&self, _width: usize) -> usize {
        self.rows.len().max(1)
    }

    fn render(&self, width: usize) -> Vec<Vec<Cell>> {
        let width = width.max(1);
        self.rows
            .iter()
            .map(|runs| shape_runs(runs, width))
            .collect()
    }

    fn intrinsic_width(&self) -> usize {
        self.rows
            .iter()
            .map(|runs| {
                runs.iter()
                    .map(|(s, _)| {
                        s.chars()
                            .map(|c| c.width().unwrap_or(0).max(1))
                            .sum::<usize>()
                    })
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }

    fn plain_text(&self) -> String {
        let mut out = String::new();
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            for (run, _) in row {
                out.push_str(run);
            }
        }
        out
    }
}

/// Inline-styled runs that wrap together at the layout-assigned width — the
/// multi-style analog of [`Plain`]. Each run carries its own [`SpanStyle`];
/// runs flow into one another and the stream wraps at `width`, honouring `\n`
/// for hard breaks.
///
/// Wrapping is selectable via [`Spans::word_wrap`]: char-wrap (default, breaks
/// anywhere — good for code/identifiers) or word-wrap (breaks at whitespace,
/// falling back to char-break for a single token longer than the line — the
/// right choice for prose).
///
/// Unlike [`Lines`] (pre-wrapped upstream, so it only *clips* on resize),
/// `Spans` re-wraps on every `render(width)` call. This is the leaf that lets
/// rich content — markdown inline runs, a prompt input line with a block
/// cursor — re-flow for free when its container resizes, instead of the
/// caller guessing a width and rebuilding a `Lines` on every change.
#[derive(Clone, Debug, Default)]
pub struct Spans {
    /// Inline runs of `(text, style)`, read left-to-right and wrapped as one stream.
    pub runs: Vec<(String, SpanStyle)>,
    /// When `true`, wrap at word boundaries (whitespace); when `false` (default),
    /// char-wrap. See [`Spans::word_wrap`].
    pub word_wrap: bool,
}

impl Spans {
    /// Construct from inline styled runs, char-wrapping (the default).
    pub fn new(runs: Vec<(String, SpanStyle)>) -> Self {
        Self {
            runs,
            word_wrap: false,
        }
    }

    /// Construct from inline styled runs, **word-wrapping** at whitespace
    /// boundaries (a single token longer than the line still char-breaks). Use
    /// this for prose; use [`Spans::new`] for code/identifiers.
    pub fn word_wrap(runs: Vec<(String, SpanStyle)>) -> Self {
        Self {
            runs,
            word_wrap: true,
        }
    }
}

impl CellProducer for Spans {
    fn measure(&self, width: usize) -> usize {
        self.shape(width.max(1)).len().max(1)
    }

    fn render(&self, width: usize) -> Vec<Vec<Cell>> {
        let width = width.max(1);
        let mut rows = self.shape(width);
        if rows.is_empty() {
            rows.push(vec![]);
        }
        rows
    }

    fn intrinsic_width(&self) -> usize {
        let mut max_w = 0usize;
        let mut line_w = 0usize;
        for (run, _) in &self.runs {
            for ch in run.chars() {
                if ch == '\n' {
                    max_w = max_w.max(line_w);
                    line_w = 0;
                } else {
                    line_w += ch.width().unwrap_or(0).max(1);
                }
            }
        }
        max_w.max(line_w)
    }

    fn plain_text(&self) -> String {
        let mut out = String::new();
        for (run, _) in &self.runs {
            out.push_str(run);
        }
        out
    }
}

impl Spans {
    /// Shape this producer's runs into cell rows at `width`, honouring
    /// [`Spans::word_wrap`].
    fn shape(&self, width: usize) -> Vec<Vec<Cell>> {
        if self.word_wrap {
            shape_spans_word_wrap(&self.runs, width)
        } else {
            shape_spans(&self.runs, width)
        }
    }
}

/// Shape `text` into cell rows, char-wrapping at `width` and honouring `\n`.
/// Each glyph is a [`Cell`] with the given `style`; a wide glyph occupies two
/// cells (the second blanked). This is the previous surface `layout_row`
/// char-wrap, now producing cells directly.
fn shape_plain(text: &str, style: SpanStyle, width: usize) -> Vec<Vec<Cell>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    for ch in text.chars() {
        if ch == '\n' {
            rows.push(pad_to_width(std::mem::take(&mut current), width));
            continue;
        }
        let cw = ch.width().unwrap_or(0).max(1);
        if current.len() + cw > width && !current.is_empty() {
            rows.push(pad_to_width(std::mem::take(&mut current), width));
        }
        // push_glyph blanks the wide glyph's trailing cell so the terminal
        // advances two columns without emitting an extra space.
        row::push_glyph(&mut current, ch, style);
    }
    rows.push(pad_to_width(current, width));
    rows
}

/// Shape a stream of inline-styled runs into cell rows, char-wrapping the whole
/// stream at `width` and honouring `\n`. Each glyph keeps its run's [`SpanStyle`];
/// a wide glyph occupies two cells (the second blanked). This is the
/// multi-style counterpart of [`shape_plain`]: runs flow together and wrap as
/// one stream, so (unlike [`shape_runs`]) a run does not start a new line.
fn shape_spans(runs: &[(String, SpanStyle)], width: usize) -> Vec<Vec<Cell>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    for (run, style) in runs {
        for ch in run.chars() {
            if ch == '\n' {
                rows.push(pad_to_width(std::mem::take(&mut current), width));
                continue;
            }
            let cw = ch.width().unwrap_or(0).max(1);
            if current.len() + cw > width && !current.is_empty() {
                rows.push(pad_to_width(std::mem::take(&mut current), width));
            }
            row::push_glyph(&mut current, ch, *style);
        }
    }
    rows.push(pad_to_width(current, width));
    rows
}

/// Shape a stream of inline-styled runs into cell rows, **word-wrapping** at
/// `width`. Words (maximal non-whitespace runs) stay intact when they fit;
/// whitespace between words is committed only when the following word fits on
/// the same line, otherwise it is dropped (no leading whitespace on wrapped
/// lines). A single word longer than the line char-breaks across rows. `\n`
/// forces a hard break. Each glyph keeps its run's [`SpanStyle`]; a wide glyph
/// occupies two cells.
fn shape_spans_word_wrap(runs: &[(String, SpanStyle)], width: usize) -> Vec<Vec<Cell>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut pending_ws: Vec<Cell> = Vec::new();

    for (run, style) in runs {
        // Split this run into whitespace / non-whitespace tokens, emitting a
        // hard break on '\n'.
        let mut token = String::new();
        let mut token_ws: Option<bool> = None;
        for ch in run.chars() {
            if ch == '\n' {
                drain_token(&mut token, &mut token_ws, *style, &mut current, &mut pending_ws, &mut rows, width);
                rows.push(pad_to_width(std::mem::take(&mut current), width));
                pending_ws.clear();
                continue;
            }
            let is_ws = ch.is_whitespace();
            if token.is_empty() {
                token_ws = Some(is_ws);
            } else if token_ws != Some(is_ws) {
                drain_token(&mut token, &mut token_ws, *style, &mut current, &mut pending_ws, &mut rows, width);
                token_ws = Some(is_ws);
            }
            token.push(ch);
        }
        drain_token(&mut token, &mut token_ws, *style, &mut current, &mut pending_ws, &mut rows, width);
    }
    rows.push(pad_to_width(current, width));
    rows
}

/// Emit one whitespace/non-whitespace token into the line buffer, wrapping to a
/// new row when a word would overflow. Whitespace is buffered in `pending_ws`
/// and committed only once the following word is confirmed to fit.
#[allow(clippy::too_many_arguments)]
fn drain_token(
    token: &mut String,
    token_ws: &mut Option<bool>,
    style: SpanStyle,
    current: &mut Vec<Cell>,
    pending_ws: &mut Vec<Cell>,
    rows: &mut Vec<Vec<Cell>>,
    width: usize,
) {
    let is_ws = match (token_ws.take(), token.is_empty()) {
        (Some(is_ws), false) => is_ws,
        _ => {
            token.clear();
            return;
        }
    };
    let chars: Vec<char> = token.drain(..).collect();
    if is_ws {
        for ch in chars {
            push_cell(pending_ws, ch, style);
        }
        return;
    }
    let word_w: usize = chars.iter().map(|c| c.width().unwrap_or(0).max(1)).sum();
    // If the word doesn't fit with the current line + gap, wrap first (dropping
    // the pending whitespace).
    if !current.is_empty() && current.len() + pending_ws.len() + word_w > width {
        rows.push(pad_to_width(std::mem::take(current), width));
        pending_ws.clear();
    }
    // Commit the pending whitespace gap, then the word.
    current.append(pending_ws);
    for ch in chars {
        let cw = ch.width().unwrap_or(0).max(1);
        if current.len() + cw > width && !current.is_empty() {
            rows.push(pad_to_width(std::mem::take(current), width));
        }
        push_cell(current, ch, style);
    }
}

/// Push one glyph cell (plus a blank trailing cell for wide glyphs) into `buf`.
/// Thin wrapper over the public [`row::push_glyph`] primitive.
fn push_cell(buf: &mut Vec<Cell>, ch: char, style: SpanStyle) {
    row::push_glyph(buf, ch, style);
}

/// Shape one row of styled runs into exactly `width` cells, clipping overflow
/// (no re-wrap — the caller wrapped upstream). A wide glyph occupies two cells.
fn shape_runs(runs: &[(String, SpanStyle)], width: usize) -> Vec<Cell> {
    let mut row = Vec::with_capacity(width);
    for (run, style) in runs {
        for ch in run.chars() {
            if ch == '\n' {
                continue;
            }
            let cw = ch.width().unwrap_or(0).max(1);
            if row.len() + cw > width {
                // Clip the rest of the row; padding to width happens below.
                row::pad(&mut row, width);
                return row;
            }
            row::push_glyph(&mut row, ch, *style);
        }
    }
    row::pad(&mut row, width);
    row
}

/// Pad a partial row up to `width` with empty cells so every row is uniform.
fn pad_to_width(mut row: Vec<Cell>, width: usize) -> Vec<Cell> {
    row::pad(&mut row, width);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Modifier;

    #[test]
    fn plain_wraps_at_width() {
        let p = Plain::new("abcdef");
        assert_eq!(p.measure(3), 2, "two rows at width 3");
        let rows = p.render(3);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 3);
        assert_eq!(rows[1].len(), 3);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "a");
        assert_eq!(rows[1][0].glyph.as_ref().unwrap().value, "d");
    }

    #[test]
    fn plain_honours_newlines() {
        let p = Plain::new("a\nb");
        assert_eq!(p.measure(10), 2);
    }

    #[test]
    fn plain_styles_each_glyph() {
        let style = SpanStyle {
            fg: Some(crossterm::style::Color::Red),
            add_modifier: Modifier::BOLD,
            ..SpanStyle::default()
        };
        let p = Plain::styled("ab", style);
        let rows = p.render(5);
        let g = rows[0][0].glyph.as_ref().unwrap();
        assert_eq!(g.style.fg, Some(crossterm::style::Color::Red));
        assert!(g.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn plain_wide_glyph_occupies_two_cells() {
        let p = Plain::new("好X");
        let rows = p.render(4);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "好");
        assert!(rows[0][1].glyph.is_none(), "trailing cell blanked");
        assert_eq!(rows[0][2].glyph.as_ref().unwrap().value, "X");
    }

    #[test]
    fn plain_consecutive_wide_glyphs_have_no_space_between() {
        // 你好 — both CJK, width 2 each. Must be 4 cells total with NO space
        // (no spurious blank, no double-counted trailing cell).
        let p = Plain::new("你好");
        let rows = p.render(10);
        assert_eq!(rows[0].len(), 10, "padded to width");
        let glyphs: Vec<&str> = rows[0]
            .iter()
            .filter_map(|c| c.glyph.as_ref().map(|g| g.value.as_str()))
            .collect();
        assert_eq!(glyphs, vec!["你", "好"], "two glyphs, no gap glyph");
        // Column 0,1 = 你 (glyph + blank), 2,3 = 好 (glyph + blank).
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "你");
        assert!(rows[0][1].glyph.is_none(), "你 trailing blank");
        assert_eq!(rows[0][2].glyph.as_ref().unwrap().value, "好");
        assert!(rows[0][3].glyph.is_none(), "好 trailing blank");
    }

    #[test]
    fn row_primitives_are_wide_glyph_safe() {
        use super::row;
        let style = SpanStyle::default();

        // push_glyph adds exactly one trailing blank for a wide glyph.
        let mut buf = Vec::new();
        row::push_glyph(&mut buf, '你', style);
        row::push_glyph(&mut buf, '好', style);
        assert_eq!(buf.len(), 4, "two wide glyphs = 4 cells");
        assert_eq!(buf[0].glyph.as_ref().unwrap().value, "你");
        assert!(buf[1].glyph.is_none());
        assert_eq!(buf[2].glyph.as_ref().unwrap().value, "好");
        assert!(buf[3].glyph.is_none());

        // extend_clamped copies column-indexed cells verbatim — it must NOT
        // re-add trailing cells. Build a producer-shaped source (glyph+blank
        // pairs) and clamp into a row that already has a prefix.
        let src: Vec<Cell> = buf.clone();
        let mut dst = vec![row::glyph_cell('│', style)];
        row::extend_clamped(&mut dst, &src, 5); // prefix(1) + 4 content = 5
        assert_eq!(
            dst.len(),
            5,
            "no double-counted blanks: prefix + 4 source cells"
        );
        assert_eq!(dst[1].glyph.as_ref().unwrap().value, "你");
        assert_eq!(dst[3].glyph.as_ref().unwrap().value, "好");
    }

    #[test]
    fn plain_empty_text_yields_one_row() {
        let p = Plain::new("");
        assert_eq!(p.measure(5), 1);
        let rows = p.render(5);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 5);
    }

    #[test]
    fn lines_preserves_per_run_styles() {
        let rows = Lines::new(vec![vec![
            ("a".to_string(), SpanStyle::default()),
            (
                "b".to_string(),
                SpanStyle {
                    fg: Some(crossterm::style::Color::Blue),
                    ..SpanStyle::default()
                },
            ),
        ]]);
        assert_eq!(rows.measure(10), 1);
        let out = rows.render(10);
        assert_eq!(out[0][0].glyph.as_ref().unwrap().style.fg, None);
        assert_eq!(
            out[0][1].glyph.as_ref().unwrap().style.fg,
            Some(crossterm::style::Color::Blue)
        );
        assert_eq!(out[0].len(), 10, "row padded to width");
    }

    #[test]
    fn lines_clips_overflow_without_rewrapping() {
        let rows = Lines::new(vec![vec![("abcdef".to_string(), SpanStyle::default())]]);
        let out = rows.render(3);
        assert_eq!(out[0].len(), 3);
        assert_eq!(out[0][0].glyph.as_ref().unwrap().value, "a");
        assert_eq!(out[0][2].glyph.as_ref().unwrap().value, "c");
    }

    #[test]
    fn plain_text_dump() {
        let p = Plain::new("hello");
        assert_eq!(p.plain_text(), "hello");
        let rows = Lines::new(vec![
            vec![("a".to_string(), SpanStyle::default())],
            vec![("b".to_string(), SpanStyle::default())],
        ]);
        assert_eq!(rows.plain_text(), "a\nb");
    }

    #[test]
    fn spans_wraps_across_runs_at_width() {
        // "aaa" then "bbb" = "aaabbb", width 3 → two rows "aaa" / "bbb".
        let s = Spans::new(vec![
            ("aaa".to_string(), SpanStyle::default()),
            ("bbb".to_string(), SpanStyle::default()),
        ]);
        assert_eq!(s.measure(3), 2);
        let rows = s.render(3);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "a");
        assert_eq!(rows[1][0].glyph.as_ref().unwrap().value, "b");
    }

    #[test]
    fn spans_keeps_per_run_style_across_wrap() {
        // "A" (plain) + "BBB" (blue) at width 2 → row0 "AB", row1 "B_".
        // The wrapped 'B' must keep the blue run's style.
        let blue = SpanStyle {
            fg: Some(crossterm::style::Color::Blue),
            ..SpanStyle::default()
        };
        let s = Spans::new(vec![
            ("A".to_string(), SpanStyle::default()),
            ("BBB".to_string(), blue),
        ]);
        let rows = s.render(2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().style.fg, None);
        assert_eq!(
            rows[0][1].glyph.as_ref().unwrap().style.fg,
            Some(crossterm::style::Color::Blue)
        );
        assert_eq!(
            rows[1][0].glyph.as_ref().unwrap().style.fg,
            Some(crossterm::style::Color::Blue),
            "wrapped char must keep its run's style"
        );
    }

    #[test]
    fn spans_rewraps_when_width_changes() {
        // The core difference vs Lines: Spans re-wraps, it does not clip.
        let s = Spans::new(vec![("abcdef".to_string(), SpanStyle::default())]);
        assert_eq!(s.measure(2), 3, "three rows at width 2");
        assert_eq!(s.measure(6), 1, "one row at width 6");
        let rows = s.render(2);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "a");
        assert_eq!(rows[1][0].glyph.as_ref().unwrap().value, "c");
        assert_eq!(rows[2][0].glyph.as_ref().unwrap().value, "e");
    }

    #[test]
    fn spans_honours_newline_across_runs() {
        let s = Spans::new(vec![
            ("ab".to_string(), SpanStyle::default()),
            ("\ncd".to_string(), SpanStyle::default()),
        ]);
        assert_eq!(s.measure(10), 2);
    }

    #[test]
    fn spans_wide_glyph_occupies_two_cells() {
        let s = Spans::new(vec![("好X".to_string(), SpanStyle::default())]);
        let rows = s.render(4);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "好");
        assert!(rows[0][1].glyph.is_none(), "trailing cell of wide glyph blanked");
        assert_eq!(rows[0][2].glyph.as_ref().unwrap().value, "X");
    }

    #[test]
    fn spans_empty_yields_one_row() {
        let s = Spans::new(vec![]);
        assert_eq!(s.measure(5), 1);
        let rows = s.render(5);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 5);
    }

    #[test]
    fn spans_plain_text_concatenates_runs() {
        let s = Spans::new(vec![
            ("hel".to_string(), SpanStyle::default()),
            ("lo".to_string(), SpanStyle::default()),
        ]);
        assert_eq!(s.plain_text(), "hello");
    }

    #[test]
    fn spans_intrinsic_width_is_longest_line() {
        // Two lines via \n: "aaa" (3) and "bb" (2) → 3.
        let s = Spans::new(vec![("aaa\nbb".to_string(), SpanStyle::default())]);
        assert_eq!(s.intrinsic_width(), 3);
    }

    #[test]
    fn word_wrap_breaks_at_spaces_not_mid_word() {
        // "alpha beta" at width 6 → "alpha " / "beta" (beta never split).
        let s = Spans::word_wrap(vec![("alpha beta".to_string(), SpanStyle::default())]);
        let rows = s.render(6);
        assert!(rows.len() >= 2, "should wrap: {:?}", rows);
        // Every row's text, joined — "beta" must be intact somewhere.
        let text: String = rows
            .iter()
            .flat_map(|r| r.iter().filter_map(|c| c.glyph.as_ref().map(|g| g.value.clone())))
            .collect();
        assert!(text.contains("beta"), "word intact: {text}");
        assert!(text.contains("alpha"), "word intact: {text}");
    }

    #[test]
    fn word_wrap_drops_leading_whitespace_on_wrapped_lines() {
        // Wrapped continuation lines must not start with the inter-word space.
        let s = Spans::word_wrap(vec![("aa bb cc".to_string(), SpanStyle::default())]);
        let rows = s.render(5);
        assert!(rows.len() >= 2);
        for row in &rows {
            // The first glyph of each row should not be a space (unless the row
            // is entirely padding).
            if let Some(cell) = row.iter().find(|c| c.glyph.is_some()) {
                assert_ne!(
                    cell.glyph.as_ref().unwrap().value,
                    " ",
                    "wrapped line must not lead with whitespace"
                );
            }
        }
    }

    #[test]
    fn word_wrap_char_breaks_overlong_token() {
        // A single 8-char word at width 3 must char-break across rows.
        let s = Spans::word_wrap(vec![("abcdefgh".to_string(), SpanStyle::default())]);
        let rows = s.render(3);
        assert_eq!(rows.len(), 3, "8 chars / width 3 → 3 rows: {:?}", rows);
    }

    #[test]
    fn word_wrap_keeps_per_run_style() {
        let blue = SpanStyle {
            fg: Some(crossterm::style::Color::Blue),
            ..SpanStyle::default()
        };
        let s = Spans::word_wrap(vec![("aa ".to_string(), SpanStyle::default()), ("bb".to_string(), blue)]);
        let rows = s.render(10);
        // "aa " plain then "bb" blue on one line.
        let b_cell = rows[0]
            .iter()
            .find(|c| c.glyph.as_ref().is_some_and(|g| g.value == "b"))
            .unwrap();
        assert_eq!(b_cell.glyph.as_ref().unwrap().style.fg, Some(crossterm::style::Color::Blue));
    }
}
