//! Syntax highlighting backed by `syntect`.
//!
//! [`Highlighter::highlight_line`] returns a run-length list of `(text, color)`
//! pairs for one source line. The renderer turns each pair into its own `span`
//! leaf, so per-token colors show without any iodilos core changes.
//!
//! The `SyntaxSet` and theme are loaded once and reused across every line /
//! re-parse. Both are `Send + Sync` via the default-onig / regex-fancy backends.

use std::sync::OnceLock;

use crossterm::style::Color as CrosstermColor;
use syntect::highlighting::{Color as SyntectColor, ThemeSet};
use syntect::parsing::SyntaxSet;

/// A reusable highlighting context. Clone is cheap: the heavy state is shared
/// behind the `OnceLock`.
#[derive(Clone, Copy)]
pub struct Highlighter {
    _priv: (),
}

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

/// Test-only counter of `highlight_line` calls, used by the streaming-cost
/// benchmark to quantify the open-fence re-highlight cost that T4b caches.
#[cfg(test)]
static HIGHLIGHT_CALL_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

impl Default for Highlighter {
    fn default() -> Self {
        // Eagerly initialize the global sets so the first render isn't slow.
        SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines);
        THEME_SET.get_or_init(ThemeSet::load_defaults);
        Self { _priv: () }
    }
}

impl Highlighter {
    /// Build a highlighter using the lazily-initialized global syntax/theme sets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Highlight a single source line for the given language.
    ///
    /// `lang` is matched case-insensitively against syntect's known language
    /// extensions/names. Unknown languages fall back to plain output (no color).
    pub fn highlight_line(&self, line: &str, lang: &str) -> Vec<(String, Option<CrosstermColor>)> {
        #[cfg(test)]
        HIGHLIGHT_CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let ss = match SYNTAX_SET.get() {
            Some(ss) => ss,
            None => return vec![(line.to_string(), None)],
        };
        let ts = match THEME_SET.get() {
            Some(ts) => ts,
            None => return vec![(line.to_string(), None)],
        };
        let theme = match ts.themes.get("base16-ocean.dark") {
            Some(t) => t,
            None => return vec![(line.to_string(), None)],
        };
        let syntax = ss
            .find_syntax_by_token(lang)
            .or_else(|| ss.find_syntax_by_extension(lang))
            .or_else(|| {
                if lang.is_empty() {
                    None
                } else {
                    ss.find_syntax_by_name(lang)
                }
            });

        let Some(syntax) = syntax else {
            // No grammar: emit the whole line uncolored.
            return vec![(line.to_string(), None)];
        };

        use syntect::easy::HighlightLines;
        use syntect::highlighting::Style as SyntectStyle;
        let mut h = HighlightLines::new(syntax, theme);
        // syntect expects the line without a trailing newline, which `str::lines`
        // already guarantees at the call site.
        let ranges: Vec<(SyntectStyle, &str)> = match h.highlight_line(line, ss) {
            Ok(r) => r,
            Err(_) => return vec![(line.to_string(), None)],
        };

        // Merge adjacent runs with identical colors to keep the token list small.
        let mut out: Vec<(String, Option<CrosstermColor>)> = Vec::new();
        for (style, text) in ranges {
            if text.is_empty() {
                continue;
            }
            let color = convert_color(style.foreground);
            if let Some((last_text, last_color)) = out.last_mut()
                && *last_color == color
            {
                last_text.push_str(text);
                continue;
            }
            out.push((text.to_string(), color));
        }
        if out.is_empty() {
            out.push((line.to_string(), None));
        }
        out
    }

    /// Read the test-only global `highlight_line` call counter and reset it.
    /// Exposed so streaming-cost benchmarks can measure the open-fence
    /// re-highlight work that T4b caches away.
    #[cfg(test)]
    pub fn take_call_count() -> usize {
        HIGHLIGHT_CALL_COUNT.swap(0, std::sync::atomic::Ordering::Relaxed)
    }
}

/// Convert a syntect `Color` (8-bit RGB) to a crossterm 256-color `Color`.
/// We use the 6×6×6 color cube so arbitrary RGB maps to a nearby cell.
fn convert_color(c: SyntectColor) -> Option<CrosstermColor> {
    if c.a == 0 {
        // syntect uses alpha==0 to mean "default/inherit" — leave uncolored.
        return None;
    }
    let cube = |v: u8| -> u8 {
        // Map 0..=255 to one of {0..=5} cube indices, matching xterm's cube
        // levels (0,95,135,175,215,255).
        let levels = [0u8, 95, 135, 175, 215, 255];
        let mut best = 0;
        let mut best_dist = u32::MAX;
        for (i, &lv) in levels.iter().enumerate() {
            let d = (lv as i32 - v as i32).unsigned_abs();
            if d < best_dist {
                best_dist = d;
                best = i as u8;
            }
        }
        best
    };
    let r = cube(c.r);
    let g = cube(c.g);
    let b = cube(c.b);
    // 16 + 36*r + 6*g + b
    Some(CrosstermColor::AnsiValue(16 + 36 * r + 6 * g + b))
}
