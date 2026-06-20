//! Color theme for rendered Markdown, modelled after leaf's `MarkdownTheme`.
//!
//! Every color here is a `crossterm::style::Color` — the single color type iodilos
//! re-exports (ADR-0024 §3), so there is no conversion at the paint boundary.

use crossterm::style::Color;

/// A complete color palette for the markdown renderer ([`crate::markdown_lines`]
/// / [`crate::render`]). All fields are plain `Color` values so a theme can be
/// assembled with `const` constructors.
#[derive(Clone, Copy, Debug)]
pub struct MarkdownTheme {
    /// Body paragraph text.
    pub text: Color,
    /// Bold/strong inline text.
    pub strong_text: Color,
    /// Link text.
    pub link_text: Color,
    /// Inline `code` text color.
    pub code_text: Color,
    /// Background fill for fenced code blocks.
    pub code_bg: Color,
    /// Border color of the code-block frame.
    pub code_border: Color,
    /// Heading colors, indexed by level 1..=6.
    pub heading: [Color; 6],
    /// Marker character (bullets / numbers) color for lists.
    pub list_marker: Color,
    /// Checkbox glyph color for task-list items.
    pub task_marker: Color,
    /// Blockquote left border color.
    pub blockquote_marker: Color,
    /// Blockquote body text color.
    pub blockquote_text: Color,
    /// Horizontal rule color.
    pub rule_color: Color,
    /// Table header row text color.
    pub table_header: Color,
    /// Table border color.
    pub table_border: Color,
    /// Block-level (`$...$`) math source text color.
    pub math_text: Color,
    /// Background fill for block-level math blocks.
    pub math_bg: Color,
    /// Border color of the block-level math frame.
    pub math_border: Color,
}

impl MarkdownTheme {
    /// A calm dark-terminal palette tuned for readability on default backgrounds.
    pub const fn default_dark() -> Self {
        Self {
            text: Color::Reset,
            strong_text: Color::White,
            link_text: Color::Cyan,
            code_text: Color::Magenta,
            code_bg: Color::DarkGrey,
            code_border: Color::DarkGrey,
            heading: [
                Color::Red,
                Color::Yellow,
                Color::Green,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
            ],
            list_marker: Color::Yellow,
            task_marker: Color::Green,
            blockquote_marker: Color::Blue,
            blockquote_text: Color::DarkGrey,
            rule_color: Color::DarkGrey,
            table_header: Color::Yellow,
            table_border: Color::DarkGrey,
            math_text: Color::Magenta,
            math_bg: Color::DarkGrey,
            math_border: Color::DarkGrey,
        }
    }
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        Self::default_dark()
    }
}
