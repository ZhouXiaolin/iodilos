//! Text surface model: the common substrate between components, layout, and
//! the terminal cell canvas.
//!
//! `TextSurface` is iodilos's own document-shaped text abstraction. Components
//! produce it, taffy gives it a width, this module turns it into visual rows,
//! and `Canvas` receives those rows as already-shaped cell segments.

use std::borrow::Cow;
use std::ops::Deref;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::text::{Alignment, SpanStyle};

/// A styled text fragment inside a [`TextRow`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextSegment {
    pub style: SpanStyle,
    pub content: Cow<'static, str>,
}

impl TextSegment {
    pub fn raw<T: Into<Cow<'static, str>>>(content: T) -> Self {
        Self {
            style: SpanStyle::default(),
            content: content.into(),
        }
    }

    pub fn styled<T: Into<Cow<'static, str>>>(content: T, style: SpanStyle) -> Self {
        Self {
            style,
            content: content.into(),
        }
    }

    pub fn width(&self) -> usize {
        UnicodeWidthStr::width(self.content.as_ref())
    }
}

impl From<&'static str> for TextSegment {
    fn from(value: &'static str) -> Self {
        Self::raw(value)
    }
}

impl From<String> for TextSegment {
    fn from(value: String) -> Self {
        Self::raw(value)
    }
}

/// One logical row in a [`TextSurface`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRow {
    pub style: SpanStyle,
    pub alignment: Option<Alignment>,
    pub segments: Vec<TextSegment>,
}

impl TextRow {
    pub fn raw<T: Into<Cow<'static, str>>>(content: T) -> Self {
        Self {
            segments: vec![TextSegment::raw(content)],
            ..Default::default()
        }
    }

    pub fn styled<T: Into<Cow<'static, str>>>(content: T, style: SpanStyle) -> Self {
        Self {
            segments: vec![TextSegment::styled(content, style)],
            ..Default::default()
        }
    }

    pub fn from_segments(segments: Vec<TextSegment>) -> Self {
        Self {
            segments,
            ..Default::default()
        }
    }

    pub fn width(&self) -> usize {
        self.segments.iter().map(TextSegment::width).sum()
    }
}

impl From<TextSegment> for TextRow {
    fn from(segment: TextSegment) -> Self {
        Self::from_segments(vec![segment])
    }
}

impl From<Vec<TextSegment>> for TextRow {
    fn from(segments: Vec<TextSegment>) -> Self {
        Self::from_segments(segments)
    }
}

impl From<&'static str> for TextRow {
    fn from(value: &'static str) -> Self {
        Self::raw(value)
    }
}

/// A component-facing text surface.
///
/// Rows are logical rows; layout turns them into visual rows after taffy has
/// supplied a width.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextSurface {
    rows: Vec<TextRow>,
}

impl TextSurface {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_rows(rows: Vec<TextRow>) -> Self {
        Self { rows }
    }

    pub fn from_row(row: TextRow) -> Self {
        Self { rows: vec![row] }
    }

    pub fn raw<T: Into<Cow<'static, str>>>(content: T) -> Self {
        Self::from_text(content)
    }

    pub fn styled<T: Into<Cow<'static, str>>>(content: T, style: SpanStyle) -> Self {
        Self::from_text_with_style(content, style)
    }

    pub fn from_text<T: Into<Cow<'static, str>>>(content: T) -> Self {
        Self::from_text_with_style(content, SpanStyle::default())
    }

    pub fn from_text_with_style<T: Into<Cow<'static, str>>>(content: T, style: SpanStyle) -> Self {
        let content = content.into();
        let rows = content
            .as_ref()
            .split('\n')
            .map(|line| TextRow::styled(line.to_string(), style))
            .collect();
        Self { rows }
    }

    pub fn rows(&self) -> &[TextRow] {
        &self.rows
    }

    pub fn push_row(&mut self, row: TextRow) {
        self.rows.push(row);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn max_width(&self) -> usize {
        self.rows.iter().map(TextRow::width).max().unwrap_or(0)
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (row_idx, row) in self.rows.iter().enumerate() {
            if row_idx > 0 {
                out.push('\n');
            }
            for segment in &row.segments {
                out.push_str(segment.content.as_ref());
            }
        }
        out
    }

    pub fn layout(&self, width: usize, inherited: SpanStyle) -> TextLayout {
        let width = width.max(1);
        let mut rows = Vec::new();
        for row in &self.rows {
            layout_row(row, width, inherited, &mut rows);
        }
        if rows.is_empty() {
            rows.push(VisualRow::default());
        }
        TextLayout { width, rows }
    }
}

impl From<TextRow> for TextSurface {
    fn from(row: TextRow) -> Self {
        Self::from_row(row)
    }
}

impl From<TextSegment> for TextSurface {
    fn from(segment: TextSegment) -> Self {
        Self::from_row(TextRow::from(segment))
    }
}

impl From<&'static str> for TextSurface {
    fn from(value: &'static str) -> Self {
        Self::from_text(value)
    }
}

impl From<String> for TextSurface {
    fn from(value: String) -> Self {
        Self::from_text(value)
    }
}

impl Deref for TextSurface {
    type Target = [TextRow];

    fn deref(&self) -> &Self::Target {
        self.rows()
    }
}

/// Width-shaped rows ready for canvas painting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLayout {
    width: usize,
    rows: Vec<VisualRow>,
}

impl TextLayout {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn rows(&self) -> &[VisualRow] {
        &self.rows
    }
}

/// One visual terminal row after wrapping and alignment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualRow {
    pub segments: Vec<VisualSegment>,
}

impl VisualRow {
    pub fn width(&self) -> usize {
        self.segments.iter().map(VisualSegment::width).sum()
    }
}

/// A cell-ready visual segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualSegment {
    pub content: String,
    pub style: SpanStyle,
}

impl VisualSegment {
    pub fn width(&self) -> usize {
        UnicodeWidthStr::width(self.content.as_str())
    }
}

fn layout_row(row: &TextRow, width: usize, inherited: SpanStyle, out: &mut Vec<VisualRow>) {
    let base = inherited.patch(row.style);
    let alignment = row.alignment.unwrap_or_default();
    let mut current = VisualRow::default();
    let mut col = 0usize;

    for segment in &row.segments {
        let style = base.patch(segment.style);
        for ch in segment.content.chars() {
            if ch == '\n' {
                push_aligned_row(out, std::mem::take(&mut current), width, alignment, base);
                col = 0;
                continue;
            }
            let cw = ch.width().unwrap_or(0).max(1);
            if col + cw > width && !current.segments.is_empty() {
                push_aligned_row(out, std::mem::take(&mut current), width, alignment, base);
                col = 0;
            }
            current.segments.push(VisualSegment {
                content: ch.to_string(),
                style,
            });
            col += cw;
        }
    }

    push_aligned_row(out, current, width, alignment, base);
}

fn push_aligned_row(
    out: &mut Vec<VisualRow>,
    mut row: VisualRow,
    width: usize,
    alignment: Alignment,
    base: SpanStyle,
) {
    let row_width = row.width();
    let pad = match alignment {
        Alignment::Left => 0,
        Alignment::Center => width.saturating_sub(row_width) / 2,
        Alignment::Right => width.saturating_sub(row_width),
    };
    if pad > 0 {
        row.segments.insert(
            0,
            VisualSegment {
                content: " ".repeat(pad),
                style: base,
            },
        );
    }
    out.push(row);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    #[test]
    fn surface_from_text_splits_rows() {
        let surface = TextSurface::from_text("a\nb");
        assert_eq!(surface.row_count(), 2);
        assert_eq!(surface.plain_text(), "a\nb");
    }

    #[test]
    fn segment_width_counts_unicode() {
        assert_eq!(TextSegment::raw("abc").width(), 3);
        assert_eq!(TextSegment::raw("你好").width(), 4);
    }

    #[test]
    fn layout_wraps_at_width() {
        let surface = TextSurface::from_row(TextRow::from(TextSegment::raw("abcdef")));
        let layout = surface.layout(3, SpanStyle::default());
        assert_eq!(layout.height(), 2);
        assert_eq!(layout.rows()[0].width(), 3);
        assert_eq!(layout.rows()[1].width(), 3);
    }

    #[test]
    fn layout_resolves_styles() {
        let surface = TextSurface::from_row(TextRow::from_segments(vec![
            TextSegment::styled(
                "a",
                SpanStyle {
                    fg: Some(Color::Red),
                    ..SpanStyle::default()
                },
            ),
            TextSegment::raw("b"),
        ]));
        let layout = surface.layout(
            10,
            SpanStyle {
                bg: Some(Color::Blue),
                ..SpanStyle::default()
            },
        );
        assert_eq!(layout.rows()[0].segments[0].style.fg, Some(Color::Red));
        assert_eq!(layout.rows()[0].segments[0].style.bg, Some(Color::Blue));
        assert_eq!(layout.rows()[0].segments[1].style.bg, Some(Color::Blue));
    }
}
