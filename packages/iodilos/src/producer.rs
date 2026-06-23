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
        current.push(Cell {
            background: None,
            glyph: Some(Glyph {
                value: ch.to_string(),
                style,
            }),
        });
        // Blank the wide glyph's trailing cell so the terminal advances two
        // columns without emitting an extra space.
        if cw > 1 {
            current.push(Cell::default());
        }
    }
    rows.push(pad_to_width(current, width));
    rows
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
                return pad_to_width(row, width);
            }
            row.push(Cell {
                background: None,
                glyph: Some(Glyph {
                    value: ch.to_string(),
                    style: *style,
                }),
            });
            if cw > 1 {
                row.push(Cell::default());
            }
        }
    }
    pad_to_width(row, width)
}

/// Pad a partial row up to `width` with empty cells so every row is uniform.
fn pad_to_width(mut row: Vec<Cell>, width: usize) -> Vec<Cell> {
    while row.len() < width {
        row.push(Cell::default());
    }
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
}
