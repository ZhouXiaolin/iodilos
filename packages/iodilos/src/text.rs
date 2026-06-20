//! ratatui-style text primitives: `SpanStyle`, `Modifier`, `Alignment`,
//! `Span`, `Line` — the content model for `TuiNode::LineFlow`.
//!
//! Structurally aligned with `ratatui-core` 0.1.1 (so `iodilos-md` can mirror
//! `~/leaf`), but self-built — no ratatui dependency (ADR-0024). Color stays
//! `crossterm::style::Color` (ADR-0024 §3).

use std::borrow::Cow;

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

/// Bridge from the legacy scalar `TextStyle` (used by the box `Style`'s text
/// fields) to `SpanStyle`. The box's resolved text style is the *base* onto
/// which each `Span`'s style patches during paint.
impl From<crate::style::TextStyle> for SpanStyle {
    fn from(t: crate::style::TextStyle) -> Self {
        use crate::style::Weight;
        let mut add = Modifier::empty();
        match t.weight {
            Weight::Bold => add |= Modifier::BOLD,
            Weight::Light => add |= Modifier::DIM,
            Weight::Normal => {}
        }
        if t.underline {
            add |= Modifier::UNDERLINED;
        }
        if t.italic {
            add |= Modifier::ITALIC;
        }
        if t.invert {
            add |= Modifier::REVERSED;
        }
        SpanStyle {
            fg: t.color,
            bg: None,
            underline_color: None,
            add_modifier: add,
            sub_modifier: Modifier::empty(),
        }
    }
}

/// Horizontal alignment of a [`Line`] within its area. Mirrors
/// `ratatui_core::layout::Alignment`.
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

/// A styled run of text: the smallest styleable unit. Mirrors
/// `ratatui_core::text::Span` but `'static` (a `TuiNode` outlives any borrow).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    /// The style of this span.
    pub style: SpanStyle,
    /// The content as a clone-on-write static string.
    pub content: Cow<'static, str>,
}

impl Span {
    /// A span with the default (unset) style.
    pub fn raw<T: Into<Cow<'static, str>>>(content: T) -> Self {
        Self {
            style: SpanStyle::default(),
            content: content.into(),
        }
    }

    /// A span with the given style.
    pub fn styled<T: Into<Cow<'static, str>>>(content: T, style: SpanStyle) -> Self {
        Self {
            style,
            content: content.into(),
        }
    }

    /// Unicode display width of this span's content.
    pub fn width(&self) -> usize {
        unicode_width::UnicodeWidthStr::width(self.content.as_ref())
    }
}

impl From<&'static str> for Span {
    fn from(s: &'static str) -> Self {
        Span::raw(s)
    }
}

impl From<String> for Span {
    fn from(s: String) -> Self {
        Span::raw(s)
    }
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
    fn spanstyle_from_legacy_textstyle_maps_scalars() {
        let ts = crate::style::TextStyle {
            color: Some(Color::Yellow),
            weight: crate::style::Weight::Bold,
            underline: true,
            italic: true,
            invert: false,
        };
        let s = SpanStyle::from(ts);
        assert_eq!(s.fg, Some(Color::Yellow));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
        assert!(s.add_modifier.contains(Modifier::ITALIC));
        assert!(!s.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn alignment_default_is_left() {
        assert_eq!(Alignment::default(), Alignment::Left);
    }

    #[test]
    fn span_raw_has_no_style() {
        let s = Span::raw("hi");
        assert_eq!(s.content, Cow::Borrowed("hi"));
        assert_eq!(s.style, SpanStyle::default());
    }

    #[test]
    fn span_width_counts_unicode() {
        assert_eq!(Span::raw("abc").width(), 3);
        // CJK fullwidth chars are width 2 each.
        assert_eq!(Span::raw("你好").width(), 4);
    }
}
