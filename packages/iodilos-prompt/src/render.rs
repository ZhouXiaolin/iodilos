//! Pure run-builders that feed the framework's own border + wrapping.
//!
//! The prompt box is now composed from framework primitives:
//! - the rounded frame is `div(border_style = BorderStyle::Round)` — the
//!   framework draws `╭─│╰─╯`;
//! - the statusline on the top border is [`crate::style::BorderTitleRuns`] via
//!   `border_title`;
//! - the multiline input is a [`iodilos::producer::Spans`] leaf that re-wraps
//!   at the layout-assigned width for free.
//!
//! This module only decides *which runs* go into the title and the input leaf
//! (cursor styling, statusline segments). All geometry — border drawing, text
//! wrapping, wide-glyph cells, scroll clamping — is the framework's job.

use std::borrow::Cow;

use iodilos::style::BorderTitleRuns;
use iodilos::text::SpanStyle;
use iodilos::Color;

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

/// Build the inline-styled runs for the prompt's editable text, with the block
/// cursor applied as a per-cell background.
///
/// The char at `cursor` (when it is a normal char) is covered by the cursor —
/// it gets `cursor_bg`/`cursor_fg`. When the cursor sits at end-of-input or
/// just before a `'\n'`, a trailing block (a styled space) is inserted so the
/// caret is always visible. `'\n'` in the buffer becomes a hard line break in
/// the produced [`iodilos::producer::Spans`]; long lines soft-wrap at the
/// layout width automatically.
pub fn input_runs(buffer: &str, cursor: usize, theme: &PromptTheme) -> Vec<(String, SpanStyle)> {
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
            if covered.is_none() && i == cursor {
                push(' ', cursor_style);
            }
            push('\n', text);
            continue;
        }
        push(ch, if covered == Some(i) { cursor_style } else { text });
    }
    if covered.is_none() && cursor == total {
        push(' ', cursor_style);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_input(buffer: &str, cursor: usize) -> Vec<(String, SpanStyle)> {
        input_runs(buffer, cursor, &PromptTheme::default())
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
}
