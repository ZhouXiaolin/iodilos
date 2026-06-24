//! Default colours for the prompt frame, text, and block cursor.

use iodilos::Color;

/// Colour scheme for the prompt box.
#[derive(Debug, Clone, Copy)]
pub struct PromptTheme {
    /// Frame glyphs (`╭ │ ╰ ─ ╮ ╯`), drawn by `PromptView` as the prompt's
    /// rounded border.
    pub frame: Color,
    /// Dim elements: the ` > ` separators and the leading space before the tail.
    pub separator: Color,
    /// Input body text.
    pub text: Color,
    /// Block-cursor background.
    pub cursor_bg: Color,
    /// Block-cursor foreground (the glyph painted on the cursor cell).
    pub cursor_fg: Color,
}

impl Default for PromptTheme {
    fn default() -> Self {
        Self {
            frame: Color::DarkGrey,
            separator: Color::DarkGrey,
            text: Color::Grey,
            cursor_bg: Color::Cyan,
            cursor_fg: Color::Black,
        }
    }
}
