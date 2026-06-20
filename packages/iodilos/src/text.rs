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
}
