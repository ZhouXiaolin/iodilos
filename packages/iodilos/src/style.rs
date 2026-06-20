//! Self-owned length types, flat style property surface, and terminal paint value types.
//!
//! Mirrors iocraft's style boundary: geometry/length types are iodilos's own
//! (supporting `From<int>` and the `pct` suffix), color is `crossterm::style::Color`,
//! and the paint value types (`BorderStyle`, `BorderCharacters`, `Edges`, `Weight`)
//! are first-class rather than hidden behind a builder. The legacy aggregate
//! `TuiStyle` and the `style()` builder are removed.

use bitflags::bitflags;
use crossterm::style::Color;
use crate::text::{Modifier, SpanStyle};
// `MaybeDyn` must be in scope: `impl_into_maybe_dyn!` references the unqualified
// type name in its expansion. Both are re-exported at the crate root (the macro
// via `#[macro_export]`, the type via `pub use reactive::*`).
use crate::{MaybeDyn, impl_into_maybe_dyn};
use taffy::style::{
    AlignContent, AlignItems, Display, FlexDirection, FlexWrap, JustifyContent, Overflow, Position,
};
use taffy::style_helpers::{TaffyAuto as _, TaffyZero as _};
use taffy::{LengthPercentage, LengthPercentageAuto, Rect, Style as TaffyStyle};

/// A percentage in the range `0.0..=100.0`, convertible into any of the
/// library's length types. Expressible in the `view!` macro via the `pct`
/// suffix, e.g. `50pct`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Percent(pub f32);

impl From<f32> for Percent {
    fn from(v: f32) -> Self {
        Percent(v)
    }
}

macro_rules! impl_from_length {
    ($name:ident) => {
        impl From<i16> for $name {
            fn from(l: i16) -> Self {
                $name::Length(l as _)
            }
        }
        impl From<i32> for $name {
            fn from(l: i32) -> Self {
                $name::Length(l as _)
            }
        }
        impl From<u8> for $name {
            fn from(l: u8) -> Self {
                $name::Length(l as _)
            }
        }
        impl From<u16> for $name {
            fn from(l: u16) -> Self {
                $name::Length(l as _)
            }
        }
        impl From<u32> for $name {
            fn from(l: u32) -> Self {
                $name::Length(l as _)
            }
        }
    };
}

macro_rules! impl_from_percent {
    ($name:ident) => {
        impl From<Percent> for $name {
            fn from(p: Percent) -> Self {
                $name::Percent(p.0)
            }
        }
    };
}

macro_rules! new_length_percentage_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub enum $name {
            /// No value set; resolves to zero.
            #[default]
            Unset,
            /// An absolute cell count.
            Length(u32),
            /// A percentage of the parent's width or height.
            Percent(f32),
        }

        impl $name {
            /// `Unset` falls back to `other`, otherwise this value wins.
            /// Mirrors iocraft's per-side fallback (`padding_left` falls back to `padding`).
            fn or(self, other: Self) -> Self {
                match self {
                    $name::Unset => other,
                    _ => self,
                }
            }
        }

        impl From<$name> for LengthPercentage {
            fn from(p: $name) -> Self {
                match p {
                    $name::Unset => LengthPercentage::ZERO,
                    $name::Length(l) => LengthPercentage::length(l as _),
                    $name::Percent(p) => LengthPercentage::percent(p / 100.0),
                }
            }
        }

        impl_from_length!($name);
        impl_from_percent!($name);
    };
}

new_length_percentage_type!(
    /// Space reserved around an element's content, inside the border.
    /// See [MDN: padding](https://developer.mozilla.org/en-US/docs/Web/CSS/padding).
    Padding
);

new_length_percentage_type!(
    /// The gap between rows or columns of flex items.
    /// See [MDN: gap](https://developer.mozilla.org/en-US/docs/Web/CSS/gap).
    Gap
);

macro_rules! new_size_type {
    ($(#[$m:meta])* $name:ident, $intrepr:ty, $def:expr) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub enum $name {
            /// The default behavior.
            #[default]
            Unset,
            /// Automatically select a suitable size.
            Auto,
            /// An absolute cell count.
            Length($intrepr),
            /// A percentage of the parent's width or height.
            Percent(f32),
        }

        impl $name {
            #[allow(dead_code)]
            fn or<T: Into<Self>>(self, other: T) -> Self {
                match self {
                    $name::Unset => other.into(),
                    _ => self,
                }
            }
        }

        impl From<$name> for LengthPercentageAuto {
            fn from(p: $name) -> Self {
                match p {
                    $name::Unset => $def,
                    $name::Auto => LengthPercentageAuto::AUTO,
                    $name::Length(l) => LengthPercentageAuto::length(l as _),
                    $name::Percent(p) => LengthPercentageAuto::percent(p / 100.0),
                }
            }
        }

        // `Dimension` is what taffy's `size`/`min_size`/`max_size` fields hold;
        // it converts from `LengthPercentageAuto` (taffy provides that impl).
        impl From<$name> for taffy::style::Dimension {
            fn from(p: $name) -> Self {
                let lpa: LengthPercentageAuto = p.into();
                lpa.into()
            }
        }

        impl_from_length!($name);
        impl_from_percent!($name);
    };
}

new_size_type!(
    /// Space reserved around an element's content, outside the border. May be negative.
    /// See [MDN: margin](https://developer.mozilla.org/en-US/docs/Web/CSS/margin).
    Margin,
    i32,
    LengthPercentageAuto::ZERO
);

new_size_type!(
    /// A width or height of an element.
    /// See [MDN: width](https://developer.mozilla.org/en-US/docs/Web/CSS/width).
    Size,
    u32,
    LengthPercentageAuto::AUTO
);

new_size_type!(
    /// The position of a positioned element.
    /// See [MDN: inset](https://developer.mozilla.org/en-US/docs/Web/CSS/inset).
    Inset,
    i32,
    LengthPercentageAuto::AUTO
);

/// The initial main size of a flex item.
/// See [MDN: flex-basis](https://developer.mozilla.org/en-US/docs/Web/CSS/flex-basis).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FlexBasis {
    /// Use the value of `width`/`height`, or the content size if not set.
    #[default]
    Auto,
    /// An absolute cell count.
    Length(u32),
    /// A percentage of the parent's main size.
    Percent(f32),
}

impl From<FlexBasis> for taffy::style::Dimension {
    fn from(b: FlexBasis) -> Self {
        match b {
            FlexBasis::Auto => taffy::style::Dimension::AUTO,
            FlexBasis::Length(l) => taffy::style::Dimension::length(l as _),
            FlexBasis::Percent(p) => taffy::style::Dimension::percent(p / 100.0),
        }
    }
}

impl_from_length!(FlexBasis);
impl_from_percent!(FlexBasis);

/// Text weight. Mirrors iocraft's `Weight`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Weight {
    /// The normal weight.
    #[default]
    Normal,
    /// The bold weight.
    Bold,
    /// The light weight.
    Light,
}

bitflags! {
    /// A set of edges, used for selective border rendering. Mirrors iocraft's `Edges`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Edges: u8 {
        /// The top edge.
        const TOP = 0b00000001;
        /// The right edge.
        const RIGHT = 0b00000010;
        /// The bottom edge.
        const BOTTOM = 0b00000100;
        /// The left edge.
        const LEFT = 0b00001000;
    }
}

/// A border style. `None` means no border. Mirrors iocraft's `BorderStyle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderStyle {
    /// No border.
    #[default]
    None,
    /// A single-line border with 90-degree corners.
    Single,
    /// A double-line border with 90-degree corners.
    Double,
    /// A single-line border with rounded corners.
    Round,
    /// A single-line border with bold lines and 90-degree corners.
    Bold,
    /// A double-line border on the left and right with a single-line border on top and bottom.
    DoubleLeftRight,
    /// A double-line border on the top and bottom with a single-line border on the left and right.
    DoubleTopBottom,
    /// A simple border of basic ASCII characters.
    Classic,
    /// A border rendered with characters of your choice.
    Custom(BorderCharacters),
}

/// The characters used to render a custom border. Mirrors iocraft's `BorderCharacters`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BorderCharacters {
    /// The top-left corner.
    pub top_left: char,
    /// The top-right corner.
    pub top_right: char,
    /// The bottom-left corner.
    pub bottom_left: char,
    /// The bottom-right corner.
    pub bottom_right: char,
    /// The left edge.
    pub left: char,
    /// The right edge.
    pub right: char,
    /// The top edge.
    pub top: char,
    /// The bottom edge.
    pub bottom: char,
}

impl BorderStyle {
    /// Returns `true` if this is `BorderStyle::None`.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns the characters used to render the border, or `None` if there is no border.
    pub fn border_characters(&self) -> Option<BorderCharacters> {
        Some(match self {
            Self::None => return None,
            Self::Single => BorderCharacters {
                top_left: '┌',
                top_right: '┐',
                bottom_left: '└',
                bottom_right: '┘',
                left: '│',
                right: '│',
                top: '─',
                bottom: '─',
            },
            Self::Double => BorderCharacters {
                top_left: '╔',
                top_right: '╗',
                bottom_left: '╚',
                bottom_right: '╝',
                left: '║',
                right: '║',
                top: '═',
                bottom: '═',
            },
            Self::Round => BorderCharacters {
                top_left: '╭',
                top_right: '╮',
                bottom_left: '╰',
                bottom_right: '╯',
                left: '│',
                right: '│',
                top: '─',
                bottom: '─',
            },
            Self::Bold => BorderCharacters {
                top_left: '┏',
                top_right: '┓',
                bottom_left: '┗',
                bottom_right: '┛',
                left: '┃',
                right: '┃',
                top: '━',
                bottom: '━',
            },
            Self::DoubleLeftRight => BorderCharacters {
                top_left: '╓',
                top_right: '╖',
                bottom_left: '╙',
                bottom_right: '╜',
                left: '║',
                right: '║',
                top: '─',
                bottom: '─',
            },
            Self::DoubleTopBottom => BorderCharacters {
                top_left: '╒',
                top_right: '╕',
                bottom_left: '╘',
                bottom_right: '╛',
                left: '│',
                right: '│',
                top: '═',
                bottom: '═',
            },
            Self::Classic => BorderCharacters {
                top_left: '+',
                top_right: '+',
                bottom_left: '+',
                bottom_right: '+',
                left: '|',
                right: '|',
                top: '-',
                bottom: '-',
            },
            Self::Custom(chars) => *chars,
        })
    }
}

/// Inheritable text paint properties. `color`, `weight`, `decoration`,
/// `italic`, and `invert` resolve by walking the ancestor chain (nearer
/// wins), per HTML/CSS inheritance semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    /// The text color.
    pub color: Option<Color>,
    /// The text weight.
    pub weight: Weight,
    /// Whether the text is underlined.
    pub underline: bool,
    /// Whether the text is italicized.
    pub italic: bool,
    /// Whether the text is rendered with reversed foreground/background.
    pub invert: bool,
}

impl TextStyle {
    /// Resolve this style against a parent: for each field, if this value is
    /// "unset" (None / Normal / false), take the parent's value.
    pub(crate) fn inherit(self, parent: TextStyle) -> TextStyle {
        TextStyle {
            color: self.color.or(parent.color),
            weight: if self.weight == Weight::Normal && parent.weight != Weight::Normal {
                parent.weight
            } else {
                self.weight
            },
            underline: self.underline || parent.underline,
            italic: self.italic || parent.italic,
            invert: self.invert || parent.invert,
        }
    }
}

/// The author-facing flat style surface. Each field is one named style
/// property. All fields accept `MaybeDyn` at the attribute layer; this struct
/// holds the statically-resolved value used by layout and paint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    // --- layout-mode enums, re-exported from taffy ---
    /// `display`.
    pub display: Display,
    /// `flex_direction`.
    pub flex_direction: FlexDirection,
    /// `flex_wrap`.
    pub flex_wrap: FlexWrap,
    /// `overflow`.
    pub overflow: Overflow,
    /// `position`.
    pub position: Position,
    /// `align_items`.
    pub align_items: Option<AlignItems>,
    /// `align_content`.
    pub align_content: Option<AlignContent>,
    /// `justify_content`.
    pub justify_content: Option<JustifyContent>,

    // --- sizing (self-owned length types) ---
    /// `width`.
    pub width: Size,
    /// `height`.
    pub height: Size,
    /// `min_width`.
    pub min_width: Size,
    /// `min_height`.
    pub min_height: Size,
    /// `max_width`.
    pub max_width: Size,
    /// `max_height`.
    pub max_height: Size,
    /// `flex_basis`.
    pub flex_basis: FlexBasis,
    /// `flex_grow`.
    pub flex_grow: f32,
    /// `flex_shrink`.
    pub flex_shrink: Option<f32>,

    // --- spacing (self-owned length types; per-side falls back to the aggregate) ---
    /// `padding` (all sides).
    pub padding: Padding,
    /// `padding_top`.
    pub padding_top: Padding,
    /// `padding_right`.
    pub padding_right: Padding,
    /// `padding_bottom`.
    pub padding_bottom: Padding,
    /// `padding_left`.
    pub padding_left: Padding,
    /// `margin` (all sides).
    pub margin: Margin,
    /// `margin_top`.
    pub margin_top: Margin,
    /// `margin_right`.
    pub margin_right: Margin,
    /// `margin_bottom`.
    pub margin_bottom: Margin,
    /// `margin_left`.
    pub margin_left: Margin,
    /// `gap` (both axes).
    pub gap: Gap,
    /// `column_gap` / `row_gap`.
    pub column_gap: Gap,
    /// `row_gap`.
    pub row_gap: Gap,
    /// `inset` (all sides).
    pub inset: Inset,
    /// `top`.
    pub top: Inset,
    /// `right`.
    pub right: Inset,
    /// `bottom`.
    pub bottom: Inset,
    /// `left`.
    pub left: Inset,

    // --- border / background / text paint (do NOT inherit) ---
    /// `border_style`.
    pub border_style: BorderStyle,
    /// `border_color`.
    pub border_color: Option<Color>,
    /// `border_edges` (defaults to all edges).
    pub border_edges: Option<Edges>,
    /// `background_color`.
    pub background_color: Option<Color>,

    // --- inheritable text paint ---
    /// `color` (text color).
    pub color: Option<Color>,
    /// `weight`.
    pub weight: Weight,
    /// `underline`.
    pub underline: bool,
    /// `italic`.
    pub italic: bool,
    /// `invert`.
    pub invert: bool,
    /// `dim` (faint text).
    pub dim: bool,
    /// `crossed_out` (strikethrough).
    pub crossed_out: bool,
    /// `underline_color`.
    pub underline_color: Option<Color>,
}

impl Style {
    /// Convert this flat style into a taffy `Style` for layout. Mirrors
    /// iocraft's `From<LayoutStyle> for Style`, including the per-side
    /// fallback (`padding_left` falls back to `padding`) and the border-box
    /// sizing taffy needs when a border is present.
    pub(crate) fn to_taffy(&self) -> TaffyStyle {
        let border = if self.border_style.is_none() {
            Rect::zero()
        } else {
            let edges = self.border_edges.unwrap_or(Edges::all());
            Rect {
                top: if edges.contains(Edges::TOP) {
                    LengthPercentage::length(1.0)
                } else {
                    LengthPercentage::ZERO
                },
                bottom: if edges.contains(Edges::BOTTOM) {
                    LengthPercentage::length(1.0)
                } else {
                    LengthPercentage::ZERO
                },
                left: if edges.contains(Edges::LEFT) {
                    LengthPercentage::length(1.0)
                } else {
                    LengthPercentage::ZERO
                },
                right: if edges.contains(Edges::RIGHT) {
                    LengthPercentage::length(1.0)
                } else {
                    LengthPercentage::ZERO
                },
            }
        };

        TaffyStyle {
            display: self.display,
            size: taffy::geometry::Size {
                width: self.width.into(),
                height: self.height.into(),
            },
            min_size: taffy::geometry::Size {
                width: self.min_width.into(),
                height: self.min_height.into(),
            },
            max_size: taffy::geometry::Size {
                width: self.max_width.into(),
                height: self.max_height.into(),
            },
            gap: taffy::geometry::Size {
                width: self.gap.or(self.column_gap).into(),
                height: self.gap.or(self.row_gap).into(),
            },
            padding: Rect {
                left: self.padding_left.or(self.padding).into(),
                right: self.padding_right.or(self.padding).into(),
                top: self.padding_top.or(self.padding).into(),
                bottom: self.padding_bottom.or(self.padding).into(),
            },
            margin: Rect {
                left: self.margin_left.or(self.margin).into(),
                right: self.margin_right.or(self.margin).into(),
                top: self.margin_top.or(self.margin).into(),
                bottom: self.margin_bottom.or(self.margin).into(),
            },
            inset: Rect {
                left: self.left.or(self.inset).into(),
                right: self.right.or(self.inset).into(),
                top: self.top.or(self.inset).into(),
                bottom: self.bottom.or(self.inset).into(),
            },
            overflow: taffy::geometry::Point {
                x: self.overflow,
                y: self.overflow,
            },
            position: self.position,
            flex_direction: self.flex_direction,
            flex_wrap: self.flex_wrap,
            flex_basis: self.flex_basis.into(),
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink.unwrap_or(1.0),
            align_items: self.align_items,
            align_content: self.align_content,
            justify_content: self.justify_content,
            border,
            ..TaffyStyle::default()
        }
    }

    /// The inheritable text-style portion of this style, as a `SpanStyle` (the
    /// single text-style type). Used by layout paint as the base onto which
    /// each `Span`'s style patches.
    pub(crate) fn text_span_style(&self) -> SpanStyle {
        let mut add = Modifier::empty();
        match self.weight {
            Weight::Bold => add |= Modifier::BOLD,
            Weight::Light => add |= Modifier::DIM,
            Weight::Normal => {}
        }
        if self.underline {
            add |= Modifier::UNDERLINED;
        }
        if self.italic {
            add |= Modifier::ITALIC;
        }
        if self.invert {
            add |= Modifier::REVERSED;
        }
        if self.dim {
            add |= Modifier::DIM;
        }
        if self.crossed_out {
            add |= Modifier::CROSSED_OUT;
        }
        SpanStyle {
            fg: self.color,
            bg: None,
            underline_color: self.underline_color,
            add_modifier: add,
            sub_modifier: Modifier::empty(),
        }
    }
}

// The length value types each participate in `MaybeDyn` so a `MaybeDyn<T>` for
// these local types can still be passed where a `StyleDyn<T>` is expected (see
// `IntoStyleDyn for MaybeDyn<T>`). External re-exported types (`Color`,
// `Display`, …) cannot get a `From` impl for `MaybeDyn` due to orphan rules, so
// they are instead passed as plain values or signals and converted directly to
// `StyleDyn` via `IntoStyleDyn`.
impl_into_maybe_dyn!(Percent);
impl_into_maybe_dyn!(Padding);
impl_into_maybe_dyn!(Gap);
impl_into_maybe_dyn!(Margin);
impl_into_maybe_dyn!(Size);
impl_into_maybe_dyn!(Inset);
impl_into_maybe_dyn!(FlexBasis);
impl_into_maybe_dyn!(BorderStyle);
impl_into_maybe_dyn!(Weight);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_span_style_assembles_modifiers_and_underline_color() {
        let style = Style {
            color: Some(Color::Yellow),
            weight: Weight::Bold,
            italic: true,
            dim: true,
            crossed_out: true,
            underline_color: Some(Color::Cyan),
            ..Style::default()
        };
        let s = style.text_span_style();
        assert_eq!(s.fg, Some(Color::Yellow));
        assert!(s.add_modifier.contains(crate::text::Modifier::BOLD));
        assert!(s.add_modifier.contains(crate::text::Modifier::ITALIC));
        assert!(s.add_modifier.contains(crate::text::Modifier::DIM));
        assert!(s.add_modifier.contains(crate::text::Modifier::CROSSED_OUT));
        assert_eq!(s.underline_color, Some(Color::Cyan));
    }

    #[test]
    fn length_types_convert_from_int_and_percent() {
        let padding: Padding = 2.into();
        assert_eq!(padding, Padding::Length(2));
        let gap: Gap = Percent(50.0).into();
        assert_eq!(gap, Gap::Percent(50.0));
        let width: Size = 10.into();
        assert_eq!(width, Size::Length(10));
        let basis: FlexBasis = Percent(25.0).into();
        assert_eq!(basis, FlexBasis::Percent(25.0));
    }

    #[test]
    fn padding_to_length_percentage_is_zero_when_unset() {
        let unset = Padding::Unset;
        let lp: LengthPercentage = unset.into();
        assert_eq!(lp, LengthPercentage::ZERO);
    }

    #[test]
    fn flat_style_maps_to_taffy_layout_and_paint() {
        let style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            width: Size::Length(12),
            height: Size::Length(3),
            padding: Padding::Length(1),
            padding_top: Padding::Length(2),
            color: Some(Color::Yellow),
            background_color: Some(Color::Blue),
            weight: Weight::Bold,
            ..Style::default()
        };

        let taffy = style.to_taffy();
        assert_eq!(taffy.display, Display::Flex);
        assert_eq!(taffy.flex_direction, FlexDirection::Row);
        assert_eq!(taffy.size.width, taffy::style::Dimension::length(12.0));
        assert_eq!(taffy.size.height, taffy::style::Dimension::length(3.0));
        // padding_top wins over the aggregate padding.
        assert_eq!(taffy.padding.top, LengthPercentage::length(2.0));
        assert_eq!(taffy.padding.left, LengthPercentage::length(1.0));

        let text = style.text_span_style();
        assert_eq!(text.fg, Some(Color::Yellow));
        assert!(text.add_modifier.contains(crate::text::Modifier::BOLD));
        assert_eq!(style.background_color, Some(Color::Blue));
    }

    #[test]
    fn border_style_maps_to_unit_border_box() {
        let style = Style {
            border_style: BorderStyle::Single,
            ..Style::default()
        };
        let taffy = style.to_taffy();
        assert_eq!(taffy.border.top, LengthPercentage::length(1.0));
        assert_eq!(taffy.border.left, LengthPercentage::length(1.0));

        let none = Style::default();
        assert!(none.to_taffy().border == Rect::zero());
    }

    #[test]
    fn text_paint_inherits_along_ancestor_chain() {
        let parent = TextStyle {
            color: Some(Color::Blue),
            weight: Weight::Bold,
            italic: true,
            ..TextStyle::default()
        };
        // Child only sets color; it inherits weight and italic from the parent.
        let child = TextStyle {
            color: Some(Color::Yellow),
            ..TextStyle::default()
        };
        let resolved = child.inherit(parent);
        assert_eq!(resolved.color, Some(Color::Yellow)); // nearer wins
        assert_eq!(resolved.weight, Weight::Bold); // inherited
        assert!(resolved.italic); // inherited
    }
}
