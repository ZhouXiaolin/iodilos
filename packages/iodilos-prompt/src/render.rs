//! Pure rendering of the prompt box into a `TextSurface`.

use iodilos::surface::{TextRow, TextSegment, TextSurface};
use iodilos::text::SpanStyle;
use iodilos::Color;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::statusline::StatusLine;
use crate::theme::PromptTheme;

const TOP_LEFT: &str = "╭─";
const TOP_RIGHT: &str = "─╮";
const MID_LEFT: &str = "│ ";
const MID_RIGHT: &str = " │";
const BOT_LEFT: &str = "╰─";
const BOT_RIGHT: &str = "─╯";

/// Render the prompt (statusline top border + framed multiline input) into a
/// `TextSurface` exactly `width` cells wide. The cursor is drawn as a
/// self-contained block cell (covering the char under it, or a space at EOL).
pub fn render_prompt_to_surface(
    buffer: &str,
    cursor_char: usize,
    statusline: &StatusLine,
    width: usize,
    theme: &PromptTheme,
) -> TextSurface {
    let width = width.max(6); // keep both brackets + ≥2 content cells
    let cw = width.saturating_sub(4).max(1); // content width inside the frame
    let frame = fg(theme.frame);
    let text = fg(theme.text);
    let cursor = SpanStyle {
        fg: Some(theme.cursor_fg),
        bg: Some(theme.cursor_bg),
        ..SpanStyle::default()
    };

    let mut rows: Vec<TextRow> = Vec::new();
    rows.push(top_row(statusline, cw, &frame, theme));
    for line in input_lines(buffer, cursor_char, cw) {
        let (left, right) = if line.is_last {
            (BOT_LEFT, BOT_RIGHT)
        } else {
            (MID_LEFT, MID_RIGHT)
        };
        let mut segs: Vec<TextSegment> = Vec::new();
        segs.push(TextSegment::styled(left, frame));
        let mut content_width = 0usize;
        for cell in &line.cells {
            match cell {
                Cell::Char(ch) => {
                    segs.push(TextSegment::styled(ch.to_string(), text));
                    content_width += char_width(*ch);
                }
                Cell::Covered(ch) => {
                    segs.push(TextSegment::styled(ch.to_string(), cursor));
                    content_width += char_width(*ch);
                }
                Cell::CursorBlock => {
                    segs.push(TextSegment::styled(" ", cursor));
                    content_width += 1;
                }
            }
        }
        if content_width < cw {
            segs.push(TextSegment::styled(" ".repeat(cw - content_width), SpanStyle::default()));
        }
        segs.push(TextSegment::styled(right, frame));
        rows.push(TextRow::from_segments(segs));
    }
    TextSurface::from_rows(rows)
}

fn fg(color: Color) -> SpanStyle {
    SpanStyle {
        fg: Some(color),
        ..SpanStyle::default()
    }
}

fn char_width(ch: char) -> usize {
    // Matches the framework's `layout_row` width rule (zero-width -> 1).
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
}

// --- input line segmentation with inline cursor ---

#[derive(Debug)]
enum Cell {
    /// A normal displayed character.
    Char(char),
    /// A character covered by the block cursor.
    Covered(char),
    /// A trailing block cursor (no char under it: EOL / empty line).
    CursorBlock,
}

#[derive(Debug)]
struct InputLine {
    cells: Vec<Cell>,
    is_last: bool,
}

/// Split `buffer` into display lines of width `cw`, inserting the cursor cell
/// at the right place. `'\n'` is a hard break (not displayed); long lines
/// soft-wrap at `cw` by display width.
fn input_lines(buffer: &str, cursor: usize, cw: usize) -> Vec<InputLine> {
    let chars: Vec<char> = buffer.chars().collect();
    let total = chars.len();
    // Cursor covers the char at `cursor` if that char is a normal (non-newline)
    // char; otherwise it is a trailing block at the end of its line.
    let covered: Option<usize> = (cursor < total && chars[cursor] != '\n').then_some(cursor);

    let mut lines: Vec<Vec<Cell>> = Vec::new();
    let mut cur: Vec<Cell> = Vec::new();
    let mut col = 0usize;

    let push_trailing_block = |cur: &mut Vec<Cell>, lines: &mut Vec<Vec<Cell>>, col: &mut usize| {
        if *col + 1 > cw && !cur.is_empty() {
            lines.push(std::mem::take(cur));
            *col = 0;
        }
        cur.push(Cell::CursorBlock);
        *col += 1;
    };

    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\n' {
            if covered.is_none() && i == cursor {
                push_trailing_block(&mut cur, &mut lines, &mut col);
            }
            lines.push(std::mem::take(&mut cur));
            col = 0;
            continue;
        }
        let w = char_width(ch);
        if col + w > cw && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            col = 0;
        }
        cur.push(if covered == Some(i) {
            Cell::Covered(ch)
        } else {
            Cell::Char(ch)
        });
        col += w;
    }
    if covered.is_none() && cursor == total {
        push_trailing_block(&mut cur, &mut lines, &mut col);
    }
    lines.push(std::mem::take(&mut cur));

    if lines.is_empty() {
        lines.push(Vec::new());
    }
    let last = lines.len() - 1;
    lines
        .into_iter()
        .enumerate()
        .map(|(i, cells)| InputLine {
            cells,
            is_last: i == last,
        })
        .collect()
}

// --- top (statusline) row ---

fn top_row(statusline: &StatusLine, cw: usize, frame: &SpanStyle, theme: &PromptTheme) -> TextRow {
    let mut content = statusline_segments(statusline, theme);
    truncate_to_width(&mut content, cw);
    let used = segments_width(&content);
    if used < cw {
        content.push(TextSegment::styled("─".repeat(cw - used), *frame));
    }
    let mut segs = Vec::with_capacity(content.len() + 2);
    segs.push(TextSegment::styled(TOP_LEFT, *frame));
    segs.extend(content);
    segs.push(TextSegment::styled(TOP_RIGHT, *frame));
    TextRow::from_segments(segs)
}

fn statusline_segments(sl: &StatusLine, theme: &PromptTheme) -> Vec<TextSegment> {
    let dim = fg(theme.separator);
    let mut segs = Vec::new();
    segs.push(TextSegment::styled(sl.brand.clone(), fg(sl.brand_color)));
    for f in &sl.fields {
        let fs = fg(f.color);
        segs.push(TextSegment::styled(" > ", dim));
        segs.push(TextSegment::styled(f.icon.clone(), fs));
        segs.push(TextSegment::styled(" ", fs));
        segs.push(TextSegment::styled(f.text.clone(), fs));
    }
    segs.push(TextSegment::styled(" ", dim));
    segs.push(TextSegment::styled(sl.tail.clone(), fg(sl.tail_color)));
    segs
}

fn truncate_to_width(segs: &mut Vec<TextSegment>, maxw: usize) {
    let mut acc = 0usize;
    let mut keep = segs.len();
    for (i, s) in segs.iter().enumerate() {
        let w = UnicodeWidthStr::width(s.content.as_ref());
        if acc + w > maxw {
            keep = i;
            break;
        }
        acc += w;
    }
    segs.truncate(keep);
}

fn segments_width(segs: &[TextSegment]) -> usize {
    segs.iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn render(buffer: &str, cursor: usize, width: usize) -> TextSurface {
        render_prompt_to_surface(buffer, cursor, &StatusLine::default_mock(), width, &PromptTheme::default())
    }

    fn row_text(s: &TextSurface, i: usize) -> String {
        s.rows()[i]
            .segments
            .iter()
            .map(|x| x.content.as_ref().to_string())
            .collect()
    }

    fn row_width(s: &TextSurface, i: usize) -> usize {
        UnicodeWidthStr::width(row_text(s, i).as_str())
    }

    fn has_cursor_cell(s: &TextSurface, theme: &PromptTheme) -> bool {
        s.rows().iter().any(|r| {
            r.segments
                .iter()
                .any(|x| x.style.bg == Some(theme.cursor_bg))
        })
    }

    #[test]
    fn empty_buffer_is_two_rows_top_and_bottom() {
        let theme = PromptTheme::default();
        let s = render("", 0, 60);
        assert_eq!(s.row_count(), 2);
        let top = row_text(&s, 0);
        let bot = row_text(&s, 1);
        assert!(top.starts_with("╭─"));
        assert!(top.ends_with("─╮"));
        assert!(bot.starts_with("╰─"));
        assert!(bot.ends_with("─╯"));
        assert!(has_cursor_cell(&s, &theme), "empty buffer must show a cursor");
    }

    #[test]
    fn single_line_input_uses_bottom_brackets() {
        let s = render("hello", 5, 60);
        assert_eq!(s.row_count(), 2);
        assert!(row_text(&s, 1).contains("hello"));
        assert!(row_text(&s, 1).starts_with("╰─"));
        assert!(row_text(&s, 1).ends_with("─╯"));
    }

    #[test]
    fn multiline_uses_side_walls_then_bottom() {
        let s = render("aa\nbb", 5, 60);
        assert_eq!(s.row_count(), 3);
        assert!(row_text(&s, 1).starts_with("│ "));
        assert!(row_text(&s, 1).ends_with(" │"));
        assert!(row_text(&s, 2).starts_with("╰─"));
        assert!(row_text(&s, 2).ends_with("─╯"));
    }

    #[test]
    fn soft_wrap_produces_side_walls_then_bottom() {
        // width 12 -> cw 8; "abcdefghijkl" (12 chars) wraps to two lines of 8 then 4.
        let s = render("abcdefghijkl", 12, 12);
        assert_eq!(s.row_count(), 3); // top + 2 input lines
        assert!(row_text(&s, 1).starts_with("│ "));
        assert!(row_text(&s, 2).starts_with("╰─"));
        assert!(row_text(&s, 1).contains("abcdefgh"));
        assert!(row_text(&s, 2).contains("ijkl"));
    }

    #[test]
    fn every_row_fits_width_no_double_wrap() {
        for &(buf, cur, w) in &[
            ("", 0usize, 60usize),
            ("hello", 5, 60),
            ("aa\nbb", 5, 60),
            ("abcdefghijkl", 12, 12),
            ("这是一个中文测试行用来检查换行对齐", 4, 30),
            ("🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀", 5, 20),
        ] {
            let s = render(buf, cur, w);
            for i in 0..s.row_count() {
                assert!(
                    row_width(&s, i) <= w,
                    "row {i} width {} > {w} for buf={buf:?}",
                    row_width(&s, i)
                );
            }
        }
    }

    #[test]
    fn cursor_covers_char_when_not_at_eol() {
        let theme = PromptTheme::default();
        let s = render("abc", 1, 60); // cursor before 'b'
        assert!(has_cursor_cell(&s, &theme));
    }

    #[test]
    fn cursor_wraps_when_trailing_block_overflows_line() {
        // cw 2; "ab" at EOF -> trailing cursor block wraps to a new bottom line.
        let s = render("ab", 2, 6);
        assert_eq!(s.row_count(), 3); // top + "ab" + cursor line
        assert!(row_text(&s, 1).starts_with("│ "));
        assert!(row_text(&s, 2).starts_with("╰─"));
    }

    #[test]
    fn top_row_contains_statusline_brand_and_tail() {
        let s = render("", 0, 80);
        let top = row_text(&s, 0);
        assert!(top.contains("π"));
        assert!(top.contains("MiMo-V2.5-Pro++"));
        assert!(top.contains("master"));
        assert!(top.contains("▶"));
    }

    #[test]
    fn dbg_every_row_exactly_width() {
        for &w in &[
            30usize, 34, 40, 50, 60, 72, 73, 74, 75, 76, 77, 80, 100, 120, 200,
        ] {
            for &(buf, cur) in &[
                ("", 0usize),
                ("hello world", 5usize),
                ("你好世界 test 你好", 3usize),
            ] {
                let s = render(buf, cur, w);
                for i in 0..s.row_count() {
                    let txt = row_text(&s, i);
                    let rw = row_width(&s, i);
                    assert_eq!(rw, w, "width={w} buf={buf:?} row{i} rw={rw} txt={txt:?}");
                }
            }
        }
    }
}
