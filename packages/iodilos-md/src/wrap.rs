//! Word-boundary line wrapping for markdown inline runs. Ported from leaf's
//! `wrapping.rs::push_wrapped_prefixed_lines`, emitting iodilos text-surface rows.

use iodilos::text::SpanStyle;
use unicode_width::UnicodeWidthStr;

/// Wrap `runs` into one or more `TextRow`s at `width` cells, breaking at word
/// boundaries. `first_prefix` is prepended to the first line; `continuation_prefix`
/// to wrapped continuations (both may be empty). Each run keeps its own style;
/// adjacent runs of equal style are not merged (callers may merge if desired).
pub fn wrap_inline_runs(
    runs: Vec<(String, SpanStyle)>,
    first_prefix: &[(String, SpanStyle)],
    continuation_prefix: &[(String, SpanStyle)],
    width: usize,
) -> Vec<Vec<(String, SpanStyle)>> {
    let prefix_w = |prefix: &[(String, SpanStyle)]| -> usize {
        prefix
            .iter()
            .map(|s| UnicodeWidthStr::width(s.0.as_str()))
            .sum()
    };
    let first_w = prefix_w(first_prefix);
    let cont_w = prefix_w(continuation_prefix);
    let max_w = width.saturating_sub(first_w.max(cont_w)).max(8);

    let total_w: usize = runs
        .iter()
        .map(|s| UnicodeWidthStr::width(s.0.as_str()))
        .sum();
    if total_w <= max_w {
        let mut all: Vec<(String, SpanStyle)> = first_prefix.to_vec();
        all.extend(runs);
        return vec![all];
    }

    let mut lines: Vec<Vec<(String, SpanStyle)>> = Vec::new();
    let mut current: Vec<(String, SpanStyle)> = first_prefix.to_vec();
    let mut current_w = 0usize;
    let mut started = false;

    let flush = |lines: &mut Vec<Vec<(String, SpanStyle)>>,
                 current: &mut Vec<(String, SpanStyle)>,
                 current_w: &mut usize,
                 started: &mut bool| {
        if *started {
            lines.push(std::mem::take(current));
            *current = continuation_prefix.to_vec();
            *current_w = 0;
            *started = false;
        }
    };

    for span in runs {
        let style = span.1;
        // Split the span into whitespace and non-whitespace tokens.
        let mut token = String::new();
        let mut token_ws = false;
        for ch in span.0.chars() {
            let is_ws = ch.is_whitespace();
            if token.is_empty() {
                token_ws = is_ws;
            } else if token_ws != is_ws {
                emit_token(
                    &mut token,
                    token_ws,
                    style,
                    &mut lines,
                    &mut current,
                    &mut current_w,
                    &mut started,
                    max_w,
                    &flush,
                );
                token_ws = is_ws;
            }
            token.push(ch);
        }
        emit_token(
            &mut token,
            token_ws,
            style,
            &mut lines,
            &mut current,
            &mut current_w,
            &mut started,
            max_w,
            &flush,
        );
    }
    if started {
        lines.push(current);
    }
    lines
}

/// Emit one whitespace/non-whitespace token into the current line, flushing to
/// a new line (via `flush`) when it would overflow `max_w`. The explicit state
/// threading (lines/current/current_w/started + the flush callback) mirrors
/// leaf's `wrapping.rs` structure.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn emit_token(
    token: &mut String,
    is_ws: bool,
    style: SpanStyle,
    lines: &mut Vec<Vec<(String, SpanStyle)>>,
    current: &mut Vec<(String, SpanStyle)>,
    current_w: &mut usize,
    started: &mut bool,
    max_w: usize,
    flush: &dyn Fn(&mut Vec<Vec<(String, SpanStyle)>>, &mut Vec<(String, SpanStyle)>, &mut usize, &mut bool),
) {
    if token.is_empty() {
        return;
    }
    let w = UnicodeWidthStr::width(token.as_str());
    if is_ws {
        if *started && *current_w + w <= max_w {
            current.push((std::mem::take(token), style));
            *current_w += w;
        } else {
            token.clear();
        }
        return;
    }
    if *started && *current_w + w > max_w {
        flush(lines, current, current_w, started);
    }
    if w <= max_w {
        current.push((std::mem::take(token), style));
        *current_w += w;
        *started = true;
        return;
    }
    // Token longer than the line: hard-break char by char.
    for ch in token.drain(..) {
        let cw = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(0)
            .max(1);
        if *started && *current_w + cw > max_w {
            flush(lines, current, current_w, started);
        }
        current.push((ch.to_string(), style));
        *current_w += cw;
        *started = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_line() {
        let runs = vec![("hello world".to_string(), SpanStyle::default())];
        let lines = wrap_inline_runs(runs, &[], &[], 40);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn long_text_wraps_at_word_boundary() {
        let runs = vec![("alpha beta gamma delta epsilon".to_string(), SpanStyle::default())];
        let lines = wrap_inline_runs(runs, &[], &[], 12);
        assert!(lines.len() > 1, "should wrap: {lines:?}");
        // No line exceeds width (12).
        for l in &lines {
            let w: usize = l
                
                .iter()
                .map(|s| UnicodeWidthStr::width(s.0.as_str()))
                .sum();
            assert!(w <= 12, "line over width: {w}");
        }
    }

    #[test]
    fn continuation_prefix_applied_to_wrapped_lines() {
        let runs = vec![("alpha beta gamma delta epsilon zeta".to_string(), SpanStyle::default())];
        let cont = vec![("  ".to_string(), SpanStyle::default())]; // 2-space indent
        let lines = wrap_inline_runs(runs, &[], &cont, 14);
        assert!(lines.len() > 1);
        // Wrapped (non-first) lines start with the continuation prefix.
        for l in &lines[1..] {
            assert_eq!(l[0].0.as_str(), "  ");
        }
    }
}
