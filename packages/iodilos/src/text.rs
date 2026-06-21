//! Terminal text styling primitives used by the surface painter.
//!
//! These types intentionally mirror ratatui's style semantics where useful, but
//! they are not the document model. `surface` owns the row/segment abstraction
//! that components paint through.

use bitflags::bitflags;
use crossterm::style::Color;

bitflags! {
    /// Text modifiers, composable as bitflags. Mirrors `ratatui_core::style::Modifier`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Modifier: u16 {
        const BOLD         = 0b0000_0000_0001;
        const DIM          = 0b0000_0000_0010;
        const ITALIC       = 0b0000_0000_0100;
        const UNDERLINED   = 0b0000_0000_1000;
        const SLOW_BLINK   = 0b0000_0001_0000;
        const RAPID_BLINK  = 0b0000_0010_0000;
        const REVERSED     = 0b0000_0100_0000;
        const HIDDEN       = 0b0000_1000_0000;
        const CROSSED_OUT  = 0b0001_0000_0000;
    }
}

/// A text style: fg/bg/underline colors plus modifier deltas. Mirrors
/// `ratatui_core::style::Style` (all-`Option`, incremental patch semantics)
/// but uses `crossterm::style::Color`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanStyle {
    /// Foreground (text) color.
    pub fg: Option<Color>,
    /// Background color.
    pub bg: Option<Color>,
    /// Underline color (true-color terminals only).
    pub underline_color: Option<Color>,
    /// Modifiers to add.
    pub add_modifier: Modifier,
    /// Modifiers to remove (resolves against an inherited base).
    pub sub_modifier: Modifier,
}

impl SpanStyle {
    /// Construct an empty (all-unset) style.
    pub fn new() -> Self {
        Self::default()
    }

    /// Overlay `other` onto `self`: `other`'s set fields win; modifiers merge
    /// (added bits turn on, sub'd bits turn off). Mirrors ratatui `Style::patch`.
    pub fn patch(self, other: SpanStyle) -> SpanStyle {
        SpanStyle {
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
            underline_color: other.underline_color.or(self.underline_color),
            add_modifier: (self.add_modifier & !other.sub_modifier) | other.add_modifier,
            sub_modifier: (self.sub_modifier & !other.add_modifier) | other.sub_modifier,
        }
    }
}

/// Horizontal alignment of a surface row within its area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    /// Left-aligned (the default).
    #[default]
    Left,
    /// Centered.
    Center,
    /// Right-aligned.
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_composes_with_or() {
        let m = Modifier::BOLD | Modifier::ITALIC;
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::ITALIC));
        assert!(!m.contains(Modifier::DIM));
    }

    #[test]
    fn spanstyle_patch_overlay_wins() {
        let base = SpanStyle {
            fg: Some(Color::Red),
            add_modifier: Modifier::ITALIC,
            ..SpanStyle::default()
        };
        let over = SpanStyle {
            fg: Some(Color::Blue),
            add_modifier: Modifier::BOLD,
            ..SpanStyle::default()
        };
        let p = base.patch(over);
        assert_eq!(p.fg, Some(Color::Blue)); // overlay wins
        assert!(p.add_modifier.contains(Modifier::ITALIC)); // base kept
        assert!(p.add_modifier.contains(Modifier::BOLD)); // overlay added
    }

    #[test]
    fn spanstyle_patch_keeps_base_when_overlay_unset() {
        let base = SpanStyle {
            bg: Some(Color::Green),
            ..SpanStyle::default()
        };
        let p = base.patch(SpanStyle::default());
        assert_eq!(p.bg, Some(Color::Green));
    }

    #[test]
    fn alignment_default_is_left() {
        assert_eq!(Alignment::default(), Alignment::Left);
    }
}
