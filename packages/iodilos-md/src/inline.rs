//! Inline style resolution for markdown `Inline` runs. Ported from leaf's
//! `spans.rs`, but emits iodilos `SpanStyle` (not ratatui `Style`) and uses
//! `crossterm::style::Color`.

use iodilos::text::{Modifier, SpanStyle};

use crate::theme::MarkdownTheme;

/// Which inline style spans are currently open (tracked across cmark events).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct InlineStyleState {
    pub in_strong: bool,
    pub in_em: bool,
    pub in_strike: bool,
    pub in_link: bool,
}

impl InlineStyleState {
    pub fn modifiers(self) -> Modifier {
        let mut m = Modifier::empty();
        if self.in_strong {
            m |= Modifier::BOLD;
        }
        if self.in_em {
            m |= Modifier::ITALIC;
        }
        if self.in_strike {
            m |= Modifier::CROSSED_OUT;
        }
        m
    }
}

/// Resolve the body text style for the current inline state, themed for the
/// given blockquote depth (depth > 0 → italic blockquote text).
pub fn body_style(
    theme: &MarkdownTheme,
    blockquote_depth: usize,
    state: InlineStyleState,
) -> SpanStyle {
    let mut style = if state.in_link {
        let mut s = SpanStyle {
            fg: Some(theme.link_text),
            add_modifier: Modifier::UNDERLINED,
            ..SpanStyle::default()
        };
        if blockquote_depth > 0 {
            s.add_modifier |= Modifier::ITALIC;
        }
        s
    } else if blockquote_depth > 0 {
        SpanStyle {
            fg: Some(theme.blockquote_text),
            add_modifier: Modifier::ITALIC,
            ..SpanStyle::default()
        }
    } else {
        SpanStyle {
            fg: Some(theme.text),
            ..SpanStyle::default()
        }
    };
    if state.in_strong && !state.in_link {
        style.fg = Some(theme.strong_text);
    }
    style.add_modifier |= state.modifiers();
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_style_plain_text() {
        let theme = MarkdownTheme::default();
        let s = body_style(&theme, 0, InlineStyleState::default());
        assert_eq!(s.fg, Some(theme.text));
        assert_eq!(s.add_modifier, Modifier::empty());
    }

    #[test]
    fn body_style_bold_uses_strong_color() {
        let theme = MarkdownTheme::default();
        let s = body_style(
            &theme,
            0,
            InlineStyleState {
                in_strong: true,
                ..Default::default()
            },
        );
        assert_eq!(s.fg, Some(theme.strong_text));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn body_style_link_is_underlined() {
        let theme = MarkdownTheme::default();
        let s = body_style(
            &theme,
            0,
            InlineStyleState {
                in_link: true,
                ..Default::default()
            },
        );
        assert_eq!(s.fg, Some(theme.link_text));
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }
}
