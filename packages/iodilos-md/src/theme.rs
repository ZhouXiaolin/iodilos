//! Color theme for rendered Markdown, modelled after leaf's `MarkdownTheme`.
//!
//! Every color here is a `crossterm::style::Color` — the single color type iodilos
//! re-exports (ADR-0024 §3), so there is no conversion at the paint boundary.

use crossterm::style::Color;

/// A complete color palette for the markdown renderer ([`crate::markdown_surface`]
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
    /// Mermaid rendered text / fallback source color.
    pub mermaid_text: Color,
    /// Mermaid diagram keyword color in fallback source rendering.
    pub mermaid_keyword: Color,
    /// Mermaid arrow/operator color in fallback source rendering.
    pub mermaid_arrow: Color,
    /// Mermaid edge label color in fallback source rendering.
    pub mermaid_label: Color,
    /// Border color of Mermaid block frames.
    pub mermaid_border: Color,
    /// GFM alert accent color for `[!NOTE]`.
    pub alert_note: Color,
    /// GFM alert accent color for `[!TIP]`.
    pub alert_tip: Color,
    /// GFM alert accent color for `[!IMPORTANT]`.
    pub alert_important: Color,
    /// GFM alert accent color for `[!WARNING]`.
    pub alert_warning: Color,
    /// GFM alert accent color for `[!CAUTION]`.
    pub alert_caution: Color,
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
            mermaid_text: Color::Cyan,
            mermaid_keyword: Color::Yellow,
            mermaid_arrow: Color::Green,
            mermaid_label: Color::Magenta,
            mermaid_border: Color::DarkGrey,
            alert_note: Color::Blue,
            alert_tip: Color::Green,
            alert_important: Color::Magenta,
            alert_warning: Color::Yellow,
            alert_caution: Color::Red,
        }
    }
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        Self::default_dark()
    }
}
