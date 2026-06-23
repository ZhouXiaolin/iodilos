//! The self-built terminal cell grid.
//!
//! A [`Framebuffer`] is a flat grid of [`Cell`], each holding an optional
//! [`Glyph`] with a [`SpanStyle`](crate::text::SpanStyle) and an optional
//! `background` of type `crossterm::style::Color`. Cell widths are measured
//! with `unicode-width`.
//!
//! Painting (layout output) writes into a `Framebuffer`; the render driver
//! diffs the current `Framebuffer` against the previous frame's `Framebuffer`
//! and emits the minimal ANSI writes via crossterm.

use std::fmt;
use std::io::{self, Write};

use crossterm::csi;
use crossterm::style::{Attribute, Color, SetBackgroundColor, SetForegroundColor};
use unicode_width::UnicodeWidthChar;

use crate::text::{Modifier, SpanStyle};

/// Re-export of `crossterm::style::Color` — the in-memory color type, so there
/// is no conversion at the paint boundary.
pub use crossterm::style::Color as CrosstermColor;

/// A single grapheme cluster plus the style it was painted with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The character's string value (a single grapheme, possibly multi-code-point).
    pub value: String,
    /// The style applied to the character.
    pub style: SpanStyle,
}

impl Glyph {
    /// The terminal cell width of this glyph.
    pub fn width(&self) -> usize {
        self.value
            .chars()
            .map(|c| c.width().unwrap_or(0))
            .sum::<usize>()
            .max(1)
    }
}

/// A single cell of a [`Framebuffer`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The background color, if any.
    pub background: Option<Color>,
    /// The glyph drawn into this cell, if any.
    pub glyph: Option<Glyph>,
}

impl Cell {
    /// A cell with no background and no glyph.
    pub fn is_empty(&self) -> bool {
        self.background.is_none() && self.glyph.is_none()
    }
}

/// A rectangular region in terminal-cell coordinates. Positions (`x`, `y`) are
/// signed so that scrolled / absolutely-positioned content may sit off-screen
/// with negative coordinates; width/height are always non-negative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    /// The column of the top-left corner (may be negative for off-screen content).
    pub x: i32,
    /// The row of the top-left corner (may be negative for off-screen content).
    pub y: i32,
    /// The width in cells.
    pub width: u16,
    /// The height in cells.
    pub height: u16,
}

impl Rect {
    /// Construct a rect.
    pub const fn new(x: i32, y: i32, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    /// The exclusive right edge (`x + width`).
    pub fn right(self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// The exclusive bottom edge (`y + height`).
    pub fn bottom(self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// Intersect this rect with `other`. Returns `None` when the intersection is empty.
    pub fn intersect(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (x < right && y < bottom).then(|| Self {
            x,
            y,
            width: (right - x).clamp(0, u16::MAX as i32) as u16,
            height: (bottom - y).clamp(0, u16::MAX as i32) as u16,
        })
    }

    /// True when `(px, py)` falls inside this rect (inclusive left/top,
    /// exclusive right/bottom).
    pub fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

/// The self-built terminal cell grid that holds painted output. Cells are
/// stored in a single flat `Vec<Cell>` of length `width * height`, row-major
/// (row `y` occupies indices `[y*width, y*width+width)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Framebuffer {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}

impl Framebuffer {
    /// Construct an empty framebuffer of the given dimensions. The rect's
    /// position is ignored; only `width` and `height` are used.
    pub fn empty(area: Rect) -> Self {
        let width = area.width.max(1) as usize;
        let height = area.height.max(1) as usize;
        Self {
            width,
            height,
            cells: vec![Cell::default(); width * height],
        }
    }

    /// The width of the framebuffer in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The height of the framebuffer in cells.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The framebuffer dimensions as a [`Rect`] (position at origin).
    pub fn size(&self) -> Rect {
        Rect::new(0, 0, self.width as u16, self.height as u16)
    }

    /// Get an immutable reference to the cell at `(x, y)`, or `None` if out of
    /// bounds or negative.
    pub fn cell(&self, x: i32, y: i32) -> Option<&Cell> {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            Some(&self.cells[y as usize * self.width + x as usize])
        } else {
            None
        }
    }

    /// The full row `y` as a slice, or an empty slice if out of bounds or
    /// negative.
    pub fn row(&self, y: i32) -> &[Cell] {
        if y >= 0 && (y as usize) < self.height {
            let y = y as usize;
            &self.cells[y * self.width..(y + 1) * self.width]
        } else {
            &[]
        }
    }

    /// The full mutable row `y` as a slice, or an empty slice if out of bounds
    /// or negative.
    pub fn row_mut(&mut self, y: i32) -> &mut [Cell] {
        if y >= 0 && (y as usize) < self.height {
            let y = y as usize;
            &mut self.cells[y * self.width..(y + 1) * self.width]
        } else {
            &mut []
        }
    }

    /// Paint a solid `background` color across the given rect, clamping to the
    /// framebuffer bounds.
    pub fn set_background_color(&mut self, rect: Rect, color: Color) {
        let y0 = rect.y.max(0) as usize;
        let y1 = (rect.bottom().min(self.height as i32)).max(0) as usize;
        let x0 = rect.x.max(0) as usize;
        let x1 = (rect.right().min(self.width as i32)).max(0) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                self.cells[y * self.width + x].background = Some(color);
            }
        }
    }

    /// Clear the glyphs (not background) in the given rect, clamping to the
    /// framebuffer bounds.
    pub fn clear_text(&mut self, rect: Rect) {
        let y0 = rect.y.max(0) as usize;
        let y1 = (rect.bottom().min(self.height as i32)).max(0) as usize;
        let x0 = rect.x.max(0) as usize;
        let x1 = (rect.right().min(self.width as i32)).max(0) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                self.cells[y * self.width + x].glyph = None;
            }
        }
    }

    /// Write `text` into the framebuffer starting at `(rect.x, rect.y)`,
    /// wrapping at `rect.width` and clipping to `rect.height` rows. Each
    /// grapheme keeps its `width` cells (wide characters occupy two cells,
    /// the second left blank). If `rect.x` or `rect.y` is negative the text is
    /// clipped.
    pub fn set_text(&mut self, rect: Rect, text: &str, style: SpanStyle) {
        if rect.width == 0 || rect.height == 0 || rect.right() <= 0 || rect.bottom() <= 0 {
            return;
        }
        let width = rect.width as usize;
        let mut y = rect.y;
        let mut col = 0usize; // column within the current line
        for ch in text.chars() {
            if ch == '\n' {
                col = 0;
                y += 1;
                if y >= rect.bottom() {
                    return;
                }
                continue;
            }
            let cw = ch.width().unwrap_or(0).max(1);
            // If this grapheme does not fit in the remaining line width, wrap.
            if col + cw > width {
                col = 0;
                y += 1;
                if y >= rect.bottom() {
                    return;
                }
            }
            self.place_char(rect.x, y, col, ch, cw, style);
            col += cw;
        }
    }

    /// Place a single character at `(x0 + col, y)`. Wide characters blank the
    /// following cell. Clamps to framebuffer bounds.
    fn place_char(&mut self, x0: i32, y: i32, col: usize, ch: char, cw: usize, style: SpanStyle) {
        let abs_x = x0 + col as i32;
        if abs_x < 0 || y < 0 {
            return;
        }
        let abs_x = abs_x as usize;
        let y = y as usize;
        if abs_x >= self.width || y >= self.height {
            return;
        }
        let cell = &mut self.cells[y * self.width + abs_x];
        cell.glyph = Some(Glyph {
            value: ch.to_string(),
            style,
        });
        // A wide character occupies a second cell, which we blank.
        if cw > 1 && abs_x + 1 < self.width {
            self.cells[y * self.width + abs_x + 1].glyph = None;
        }
    }

    /// Emit the whole framebuffer to `w` as ANSI escape sequences,
    /// cursor-addressing each row to column 0 and rendering each row via
    /// [`render_row`]. Used for the initial full paint and for a size-mismatch
    /// full repaint in the diff path.
    pub fn write_ansi<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write!(w, csi!("0m"))?;
        for y in 0..self.height {
            // Position the cursor at column 0 of the row, then write the row's
            // reset-terminated ANSI. `render_row` handles its own trailing erase
            // (skipped on a full-width row to avoid the pending-wrap flag
            // erasing the last cell).
            write!(w, csi!("{};1H"), y + 1)?;
            w.write_all(render_row(self.row(y as i32), self.width).as_bytes())?;
        }
        write!(w, csi!("0m"))?;
        w.flush()?;
        Ok(())
    }
}

impl fmt::Display for Framebuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Unstyled plain-text representation, useful for tests/debug.
        for y in 0..self.height {
            for cell in self.row(y as i32) {
                if let Some(g) = &cell.glyph {
                    f.write_str(&g.value)?;
                } else {
                    f.write_str(" ")?;
                }
            }
            f.write_str("\n")?;
        }
        Ok(())
    }
}

/// Render one framebuffer row as a self-contained, reset-terminated ANSI
/// string. The row is an independent unit: it opens with `\x1b[0m` to
/// establish a clean attribute baseline (so it never inherits state from the
/// previous row), then emits each cell's SGR deltas against a running tracker
/// (fg/bg/modifier are only written when they change), and closes with
/// `\x1b[0m`. A wide glyph's blanked trailing cell is skipped, and — unless the
/// row fills the whole terminal width — a trailing `\x1b[K` (erase to
/// end-of-line) clears any stale cells from the previous occupant. A full-width
/// row deliberately omits the erase: on exactly-full rows the terminal keeps a
/// pending-wrap flag, and `CSI K` would erase the last cell we just drew.
///
/// The caller is responsible for cursor positioning and inter-row separation
/// (`\r\n`); this function only produces the bytes for the row itself. This is
/// the in-window row-diff primitive shared by both the full-paint and the
/// changed-row-rewrite paths.
pub(crate) fn render_row(row: &[Cell], term_width: usize) -> String {
    let mut out = String::new();
    out.push_str("\x1b[0m");
    let mut background: Option<Color> = None;
    let mut text_style = SpanStyle::default();

    let mut i = 0;
    while i < row.len() {
        let cell = &row[i];
        // The terminal already shows this row's bytes from the previous frame;
        // we are rewriting it wholesale, so the running trackers start clean and
        // we just walk left-to-right emitting SGR deltas.
        let needs_reset = match &cell.glyph {
            Some(c) => {
                !c.style.sub_modifier.is_empty()
                    || (c.style.fg.is_none() && text_style.fg.is_some())
                    || (c.style.bg.is_none() && text_style.bg.is_some())
                    || (c.style.underline_color.is_none() && text_style.underline_color.is_some())
                    || (c.style.add_modifier & !text_style.add_modifier).is_empty()
                        && !text_style.add_modifier.is_empty()
                        && c.style.add_modifier != text_style.add_modifier
            }
            None => {
                !text_style.add_modifier.is_empty()
                    || text_style.fg.is_some()
                    || text_style.bg.is_some()
                    || text_style.underline_color.is_some()
            }
        };
        if needs_reset {
            out.push_str("\x1b[0m");
            background = None;
            text_style = SpanStyle::default();
        }

        if let Some(c) = &cell.glyph {
            if c.style.fg != text_style.fg {
                out.push_str(&SetForegroundColor(c.style.fg.unwrap_or(Color::Reset)).to_string());
            }
            if c.style.bg != text_style.bg {
                out.push_str(&SetBackgroundColor(c.style.bg.unwrap_or(Color::Reset)).to_string());
            }
            // Only add modifiers that turned on relative to the running style.
            let newly_on = c.style.add_modifier & !text_style.add_modifier;
            for attr in modifier_attributes(newly_on) {
                out.push_str(&format!("\x1b[{}m", attr.sgr()));
            }
            text_style = c.style;
        }

        if cell.background != background {
            out.push_str(&SetBackgroundColor(cell.background.unwrap_or(Color::Reset)).to_string());
            background = cell.background;
        }

        if let Some(c) = &cell.glyph {
            out.push_str(&c.value);
            // A wide glyph consumes two terminal columns: advance past its
            // blanked trailing cell so we don't emit it as a space (which would
            // add an extra column and clip the last column on a real terminal).
            if c.width() > 1 {
                i += 2;
                continue;
            }
        } else {
            out.push(' ');
        }
        i += 1;
    }

    out.push_str("\x1b[0m");
    // Erase trailing stale cells unless the row fills the whole width (where a
    // trailing `CSI K` would eat the last cell via the pending-wrap flag).
    if row.len() < term_width {
        out.push_str("\x1b[K");
    }
    out
}

/// Map each set `Modifier` bit to its crossterm `Attribute`.
pub(crate) fn modifier_attributes(m: Modifier) -> Vec<Attribute> {
    let mut out = Vec::new();
    let pairs = [
        (Modifier::BOLD, Attribute::Bold),
        (Modifier::DIM, Attribute::Dim),
        (Modifier::ITALIC, Attribute::Italic),
        (Modifier::UNDERLINED, Attribute::Underlined),
        (Modifier::SLOW_BLINK, Attribute::SlowBlink),
        (Modifier::RAPID_BLINK, Attribute::RapidBlink),
        (Modifier::REVERSED, Attribute::Reverse),
        (Modifier::HIDDEN, Attribute::Hidden),
        (Modifier::CROSSED_OUT, Attribute::CrossedOut),
    ];
    for (flag, attr) in pairs {
        if m.contains(flag) {
            out.push(attr);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_framebuffer_has_requested_dimensions() {
        let fb = Framebuffer::empty(Rect::new(0, 0, 5, 3));
        assert_eq!(fb.width(), 5);
        assert_eq!(fb.height(), 3);
        assert_eq!(fb.size(), Rect::new(0, 0, 5, 3));
    }

    #[test]
    fn flat_storage_tiles_rows_with_no_overlap_or_gap() {
        let fb = Framebuffer::empty(Rect::new(0, 0, 5, 3));
        assert_eq!(fb.row(0).len(), 5);
        assert_eq!(fb.row(2).len(), 5);
        // Negative / out-of-bounds rows return empty slices.
        assert_eq!(fb.row(-1).len(), 0);
        assert_eq!(fb.row(3).len(), 0);
    }

    #[test]
    fn set_text_writes_and_wraps() {
        let mut fb = Framebuffer::empty(Rect::new(0, 0, 3, 2));
        fb.set_text(Rect::new(0, 0, 3, 2), "abcd", SpanStyle::default());
        // 'a','b','c' on row 0; 'd' wraps to row 1.
        assert_eq!(fb.cell(0, 0).unwrap().glyph.as_ref().unwrap().value, "a");
        assert_eq!(fb.cell(2, 0).unwrap().glyph.as_ref().unwrap().value, "c");
        assert_eq!(fb.cell(0, 1).unwrap().glyph.as_ref().unwrap().value, "d");
    }

    #[test]
    fn write_ansi_emits_reset_and_reset_at_end() {
        crossterm::style::force_color_output(true);

        let mut fb = Framebuffer::empty(Rect::new(0, 0, 2, 1));
        fb.set_text(
            Rect::new(0, 0, 2, 1),
            "ab",
            crate::text::SpanStyle {
                fg: Some(Color::Red),
                add_modifier: crate::text::Modifier::BOLD,
                ..crate::text::SpanStyle::default()
            },
        );
        let mut out = Vec::new();
        fb.write_ansi(&mut out).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("\x1b[0m"), "should reset at start and end");
        // crossterm emits red foreground as a 256-color sequence (`38;5;9`).
        assert!(s.contains("38;5;9m"), "should set red fg: {s}");
        assert!(s.contains("[1m"), "should set bold: {s}");
    }

    #[test]
    fn set_text_with_spanstyle_emits_bg_and_crossed_out() {
        crossterm::style::force_color_output(true);
        let mut fb = Framebuffer::empty(Rect::new(0, 0, 2, 1));
        fb.set_text(
            Rect::new(0, 0, 2, 1),
            "ab",
            crate::text::SpanStyle {
                bg: Some(Color::Blue),
                add_modifier: crate::text::Modifier::CROSSED_OUT,
                ..crate::text::SpanStyle::default()
            },
        );
        let mut out = Vec::new();
        fb.write_ansi(&mut out).unwrap();
        let s = String::from_utf8_lossy(&out);
        // crossterm 256-color blue background is `48;5;12`.
        assert!(s.contains("48;5;12m"), "should set blue bg: {s}");
        // crossed-out SGR is 9.
        assert!(s.contains("[9m"), "should set crossed-out: {s}");
    }

    #[test]
    fn write_ansi_skips_wide_char_trailing_cell() {
        // A width-2 glyph consumes two terminal columns; the framebuffer blanks
        // the second one (glyph = None). `write_ansi` emits cells sequentially,
        // so the terminal advances its cursor two columns for the glyph — if we
        // then also emit the blank trailing cell as a space, that space is an
        // EXTRA column that shifts the rest of the row right and clips the last
        // column on a real terminal. The trailing cell must be skipped.
        use crate::text::SpanStyle;
        let mut fb = Framebuffer::empty(Rect::new(0, 0, 4, 1));
        fb.set_text(Rect::new(0, 0, 4, 1), "好XY", SpanStyle::default());
        let mut out = Vec::new();
        fb.write_ansi(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let wide = "好";
        let idx = s.find(wide).expect("wide glyph emitted");
        let after = &s[idx + wide.len()..];
        assert!(
            !after.starts_with(' '),
            "wide char's trailing cell was emitted as a space, shifting the row: {after:?}"
        );
    }

    #[test]
    fn write_ansi_keeps_last_column_on_a_full_row() {
        // Regression for the prompt-box right-border disappearing: when a row is
        // filled to the very last terminal column, emitting `CSI K` (erase to
        // end-of-line) after the last cell erases it, because the terminal's
        // cursor is left on that last cell with a deferred wrap pending.
        // `write_ansi` must keep the printed glyph for the final column.
        use crate::text::SpanStyle;
        let mut fb = Framebuffer::empty(Rect::new(0, 0, 5, 1));
        fb.set_text(Rect::new(0, 0, 5, 1), "abcdZ", SpanStyle::default());
        let mut out = Vec::new();
        fb.write_ansi(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // The 'Z' must be emitted (not erased by the trailing clear-to-EOL).
        let idx = s.find('Z').expect("last column char emitted");
        let tail = &s[idx + 1..];
        assert!(
            !tail.starts_with("\x1b[K"),
            "trailing clear-to-EOL erases the last-column char on a full row: {tail:?}"
        );
    }

    #[test]
    fn cell_negative_returns_none() {
        let fb = Framebuffer::empty(Rect::new(0, 0, 5, 3));
        assert!(fb.cell(-1, 0).is_none());
        assert!(fb.cell(0, -1).is_none());
        assert!(fb.cell(5, 0).is_none());
    }

    #[test]
    fn set_text_negative_rect_does_not_panic() {
        let mut fb = Framebuffer::empty(Rect::new(0, 0, 5, 3));
        // Rect starting off-screen should clamp gracefully without panicking.
        fb.set_text(Rect::new(-10, 0, 5, 3), "x", SpanStyle::default());
        // The glyph is off-screen (column -10), so nothing is visible at (0,0).
        assert!(fb.cell(0, 0).unwrap().glyph.is_none());
    }

    #[test]
    fn rect_contains_works_with_negative_coordinates() {
        let rect = Rect::new(-5, -3, 10, 6);
        assert!(rect.contains(0, 0));
        assert!(rect.contains(-5, -3));
        assert!(!rect.contains(-6, 0));
        assert!(!rect.contains(0, 3)); // bottom is -3 + 6 = 3, exclusive
    }
}
