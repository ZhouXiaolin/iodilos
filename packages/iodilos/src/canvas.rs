//! The self-built terminal cell grid.
//!
//! Replaces the previous ratatui `Buffer`. A [`Canvas`] is a grid of
//! [`CanvasCell`], each holding an optional [`Character`] with a
//! [`SpanStyle`](crate::text::SpanStyle) and an optional `background_color` of
//! type `crossterm::style::Color`. Cell widths are measured with `unicode-width`.
//!
//! Painting (layout output) writes into a `Canvas`; the render driver diffs
//! the current `Canvas` against the previous frame's `Canvas` and emits the
//! minimal ANSI writes via crossterm. This is the crossterm-without-ratatui
//! paint stack from ADR-0024 §10–§12.

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
pub struct Character {
    /// The character's string value (a single grapheme, possibly multi-code-point).
    pub value: String,
    /// The style applied to the character.
    pub style: SpanStyle,
}

impl Character {
    /// The terminal cell width of this character.
    pub fn width(&self) -> usize {
        self.value
            .chars()
            .map(|c| c.width().unwrap_or(0))
            .sum::<usize>()
            .max(1)
    }
}

/// A single cell of a [`Canvas`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanvasCell {
    /// The background color, if any.
    pub background_color: Option<Color>,
    /// The character drawn into this cell, if any.
    pub character: Option<Character>,
}

impl CanvasCell {
    /// A cell with no background and no character.
    pub fn is_empty(&self) -> bool {
        self.background_color.is_none() && self.character.is_none()
    }
}

/// A rectangular region of a [`Canvas`], in (x, y, width, height) cell units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    /// The column of the top-left corner.
    pub x: u16,
    /// The row of the top-left corner.
    pub y: u16,
    /// The width in cells.
    pub width: u16,
    /// The height in cells.
    pub height: u16,
}

impl Rect {
    /// Construct a rect.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The exclusive right edge (`x + width`).
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// The exclusive bottom edge (`y + height`).
    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }
}

/// The self-built terminal cell grid that holds painted output. Equivalent in
/// role to iocraft's `Canvas` and to the previous ratatui `Buffer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    width: usize,
    cells: Vec<Vec<CanvasCell>>,
}

impl Canvas {
    /// Construct an empty canvas of the given dimensions.
    pub fn empty(area: Rect) -> Self {
        let width = area.width.max(1) as usize;
        let height = area.height.max(1) as usize;
        Self {
            width,
            cells: vec![vec![CanvasCell::default(); width]; height],
        }
    }

    /// The width of the canvas in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The height of the canvas in cells.
    pub fn height(&self) -> usize {
        self.cells.len()
    }

    /// The canvas dimensions as a [`Rect`].
    pub fn size(&self) -> Rect {
        Rect::new(0, 0, self.width as u16, self.height() as u16)
    }

    /// Get an immutable reference to the cell at `(x, y)`, or `None` if out of bounds.
    pub fn cell(&self, x: usize, y: usize) -> Option<&CanvasCell> {
        self.cells.get(y).and_then(|row| row.get(x))
    }

    /// Paint a solid `background_color` across the given rect.
    pub fn set_background_color(&mut self, rect: Rect, color: Color) {
        for y in rect.y..rect.bottom() {
            if let Some(row) = self.cells.get_mut(y as usize) {
                for x in rect.x..rect.right() {
                    if let Some(cell) = row.get_mut(x as usize) {
                        cell.background_color = Some(color);
                    }
                }
            }
        }
    }

    /// Apply a text style across the given rect without writing characters.
    pub fn set_style(&mut self, rect: Rect, style: SpanStyle) {
        if style == SpanStyle::default() {
            return;
        }
        for y in rect.y..rect.bottom() {
            if let Some(row) = self.cells.get_mut(y as usize) {
                for x in rect.x..rect.right() {
                    if let Some(cell) = row.get_mut(x as usize)
                        && let Some(character) = cell.character.as_mut()
                    {
                        character.style = character.style.patch(style);
                    }
                }
            }
        }
    }

    /// Clear the characters (not background) in the given rect.
    pub fn clear_text(&mut self, rect: Rect) {
        for y in rect.y..rect.bottom() {
            if let Some(row) = self.cells.get_mut(y as usize) {
                for x in rect.x..rect.right() {
                    if let Some(cell) = row.get_mut(x as usize) {
                        cell.character = None;
                    }
                }
            }
        }
    }

    /// Write `text` into the canvas starting at `(rect.x, rect.y)`, wrapping at
    /// `rect.width` and clipping to `rect.height` rows. Each grapheme keeps its
    /// `width` cells (wide characters occupy two cells, the second left blank).
    pub fn set_text(&mut self, rect: Rect, text: &str, style: SpanStyle) {
        if rect.width == 0 || rect.height == 0 {
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

    /// Paint one row of styled segments at row `y`, starting at column
    /// `rect.x`, clipping to `rect.width`. No wrapping — the caller has already
    /// wrapped. Used by the text-surface paint path.
    pub fn set_segments(&mut self, rect: Rect, y: u16, segments: &[(&str, SpanStyle)]) {
        if rect.width == 0 || y >= rect.bottom() {
            return;
        }
        let mut col = 0usize;
        let width = rect.width as usize;
        for (text, style) in segments {
            for ch in text.chars() {
                if ch == '\n' {
                    continue;
                }
                let cw = ch.width().unwrap_or(0).max(1);
                if col + cw > width {
                    break;
                }
                self.place_char(rect.x, y, col, ch, cw, *style);
                col += cw;
            }
            if col >= width {
                break;
            }
        }
    }

    /// Place a single character at `(x0 + col, y)`. Wide characters blank the
    /// following cell.
    fn place_char(&mut self, x0: u16, y: u16, col: usize, ch: char, cw: usize, style: SpanStyle) {
        let abs_x = x0 as usize + col;
        let row = match self.cells.get_mut(y as usize) {
            Some(row) => row,
            None => return,
        };
        if abs_x >= row.len() {
            return;
        }
        row[abs_x].character = Some(Character {
            value: ch.to_string(),
            style,
        });
        // A wide character occupies a second cell, which we blank.
        if cw > 1 && abs_x + 1 < row.len() {
            row[abs_x + 1].character = None;
        }
    }

    /// Emit the whole canvas to `w` as ANSI escape sequences, cursor-addressing
    /// each row to column 0. Used for the initial full paint.
    pub fn write_ansi<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let mut background = None;
        let mut text_style = SpanStyle::default();
        write!(w, csi!("0m"))?;
        for y in 0..self.height() {
            queue_move_to(w, 0, y as u16)?;
            let row = &self.cells[y];
            // Emit cells left-to-right. A wide glyph occupies two columns: the
            // terminal advances its cursor two columns when we write it, and
            // the following cell is the glyph's blanked second half — skip it,
            // otherwise the extra space shifts the rest of the row right and
            // clips the last column.
            let mut skip_next = false;
            for cell in row {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                emit_cell(w, cell, &mut background, &mut text_style)?;
                if cell.character.as_ref().is_some_and(|c| c.width() > 1) {
                    skip_next = true;
                }
            }
            if background.is_some() {
                write!(w, "{}", SetBackgroundColor(Color::Reset))?;
                background = None;
            }
            write!(w, csi!("K"))?; // clear to end of line
        }
        write!(w, csi!("0m"))?;
        w.flush()?;
        Ok(())
    }
}

impl fmt::Display for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Unstyled plain-text representation, useful for tests/debug.
        for y in 0..self.height() {
            let row = &self.cells[y];
            for cell in row {
                if let Some(c) = &cell.character {
                    f.write_str(&c.value)?;
                } else {
                    f.write_str(" ")?;
                }
            }
            f.write_str("\n")?;
        }
        Ok(())
    }
}

/// Move the cursor to `(x, y)`.
fn queue_move_to<W: Write>(w: &mut W, x: u16, y: u16) -> io::Result<()> {
    write!(w, csi!("{};{}H"), y + 1, x + 1)
}

/// Emit a single cell, tracking the running background/text style so unchanged
/// attributes aren't re-emitted.
fn emit_cell<W: Write>(
    w: &mut W,
    cell: &CanvasCell,
    background: &mut Option<Color>,
    text_style: &mut SpanStyle,
) -> io::Result<()> {
    let needs_reset = match &cell.character {
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
        write!(w, csi!("0m"))?;
        *background = None;
        *text_style = SpanStyle::default();
    }

    if let Some(c) = &cell.character {
        if c.style.fg != text_style.fg {
            write!(
                w,
                "{}",
                SetForegroundColor(c.style.fg.unwrap_or(Color::Reset))
            )?;
        }
        if c.style.bg != text_style.bg {
            write!(
                w,
                "{}",
                SetBackgroundColor(c.style.bg.unwrap_or(Color::Reset))
            )?;
        }
        // Only add modifiers that turned on relative to the running style.
        let newly_on = c.style.add_modifier & !text_style.add_modifier;
        for attr in modifier_attributes(newly_on) {
            write!(w, csi!("{}m"), attr.sgr())?;
        }
        *text_style = c.style;
    }

    if cell.background_color != *background {
        write!(
            w,
            "{}",
            SetBackgroundColor(cell.background_color.unwrap_or(Color::Reset))
        )?;
        *background = cell.background_color;
    }

    if let Some(c) = &cell.character {
        write!(w, "{}", c.value)?;
    } else {
        w.write_all(b" ")?;
    }
    Ok(())
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
    fn empty_canvas_has_requested_dimensions() {
        let canvas = Canvas::empty(Rect::new(0, 0, 5, 3));
        assert_eq!(canvas.width(), 5);
        assert_eq!(canvas.height(), 3);
        assert_eq!(canvas.size(), Rect::new(0, 0, 5, 3));
    }

    #[test]
    fn set_text_writes_and_wraps() {
        let mut canvas = Canvas::empty(Rect::new(0, 0, 3, 2));
        canvas.set_text(Rect::new(0, 0, 3, 2), "abcd", SpanStyle::default());
        // 'a','b','c' on row 0; 'd' wraps to row 1.
        assert_eq!(
            canvas.cell(0, 0).unwrap().character.as_ref().unwrap().value,
            "a"
        );
        assert_eq!(
            canvas.cell(2, 0).unwrap().character.as_ref().unwrap().value,
            "c"
        );
        assert_eq!(
            canvas.cell(0, 1).unwrap().character.as_ref().unwrap().value,
            "d"
        );
    }

    #[test]
    fn write_ansi_emits_reset_and_reset_at_end() {
        crossterm::style::force_color_output(true);

        let mut canvas = Canvas::empty(Rect::new(0, 0, 2, 1));
        canvas.set_text(
            Rect::new(0, 0, 2, 1),
            "ab",
            crate::text::SpanStyle {
                fg: Some(Color::Red),
                add_modifier: crate::text::Modifier::BOLD,
                ..crate::text::SpanStyle::default()
            },
        );
        let mut out = Vec::new();
        canvas.write_ansi(&mut out).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("\x1b[0m"), "should reset at start and end");
        // crossterm emits red foreground as a 256-color sequence (`38;5;9`).
        assert!(s.contains("38;5;9m"), "should set red fg: {s}");
        assert!(s.contains("[1m"), "should set bold: {s}");
    }

    #[test]
    fn set_text_with_spanstyle_emits_bg_and_crossed_out() {
        crossterm::style::force_color_output(true);
        let mut canvas = Canvas::empty(Rect::new(0, 0, 2, 1));
        canvas.set_text(
            Rect::new(0, 0, 2, 1),
            "ab",
            crate::text::SpanStyle {
                bg: Some(Color::Blue),
                add_modifier: crate::text::Modifier::CROSSED_OUT,
                ..crate::text::SpanStyle::default()
            },
        );
        let mut out = Vec::new();
        canvas.write_ansi(&mut out).unwrap();
        let s = String::from_utf8_lossy(&out);
        // crossterm 256-color blue background is `48;5;12`.
        assert!(s.contains("48;5;12m"), "should set blue bg: {s}");
        // crossed-out SGR is 9.
        assert!(s.contains("[9m"), "should set crossed-out: {s}");
    }

    #[test]
    fn set_segments_writes_each_segment_with_its_style() {
        let mut canvas = Canvas::empty(Rect::new(0, 0, 5, 1));
        let segs: &[(&str, crate::text::SpanStyle)] = &[
            ("ab", crate::text::SpanStyle::default()),
            (
                "cd",
                crate::text::SpanStyle {
                    fg: Some(Color::Red),
                    ..crate::text::SpanStyle::default()
                },
            ),
        ];
        canvas.set_segments(Rect::new(0, 0, 5, 1), 0, segs);
        assert_eq!(
            canvas.cell(0, 0).unwrap().character.as_ref().unwrap().value,
            "a"
        );
        assert_eq!(
            canvas.cell(2, 0).unwrap().character.as_ref().unwrap().value,
            "c"
        );
        assert_eq!(
            canvas
                .cell(2, 0)
                .unwrap()
                .character
                .as_ref()
                .unwrap()
                .style
                .fg,
            Some(Color::Red)
        );
        assert_eq!(
            canvas
                .cell(0, 0)
                .unwrap()
                .character
                .as_ref()
                .unwrap()
                .style
                .fg,
            None
        );
    }

    #[test]
    fn write_ansi_skips_wide_char_trailing_cell() {
        // A width-2 glyph consumes two terminal columns; the canvas blanks the
        // second one (character = None). `write_ansi` emits cells sequentially,
        // so the terminal advances its cursor two columns for the glyph — if we
        // then also emit the blank trailing cell as a space, that space is an
        // EXTRA column that shifts the rest of the row right and clips the last
        // column on a real terminal. The trailing cell must be skipped.
        use crate::text::SpanStyle;
        let mut canvas = Canvas::empty(Rect::new(0, 0, 4, 1));
        canvas.set_text(Rect::new(0, 0, 4, 1), "好XY", SpanStyle::default());
        let mut out = Vec::new();
        canvas.write_ansi(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let wide = "好";
        let idx = s.find(wide).expect("wide glyph emitted");
        let after = &s[idx + wide.len()..];
        assert!(
            !after.starts_with(' '),
            "wide char's trailing cell was emitted as a space, shifting the row: {after:?}"
        );
    }
}
