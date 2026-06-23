//! Absolute-position overlap demo. Each layered panel is its own component.
//!
//! Run with: `cargo run --example overlap`.

use iodilos::prelude::*;

const LOREM: &str = "Lorem ipsum odor amet, consectetuer adipiscing elit. \
Lobortis hendrerit nec ipsum dapibus quam. Donec malesuada tincidunt elementum \
mollis vehicula quisque purus. Est volutpat integer, donec sagittis placerat \
fermentum phasellus ipsum sollicitudin. Tempus laoreet ad tempus aptent proin \
per donec lectus. Quisque auctor urna; phasellus urna tortor ligula. Class \
pharetra bibendum tristique, quisque consectetur placerat potenti. Imperdiet ut \
torquent vestibulum eleifend bibendum et. Dictumst vulputate interdum iaculis \
at conubia venenatis.";

/// The inline title that overhangs the top border.
#[component(inline_props)]
fn Title(text: &'static str) -> View {
    view! {
        div(margin_top = -1) {
            p { (text) }
        }
    }
}

/// The body paragraph with the long lorem.
#[component]
fn Body() -> View {
    view! {
        div(padding = 1) {
            p(color = Color::DarkGrey, weight = Weight::Light) {
                (format!("{LOREM} {LOREM}"))
            }
        }
    }
}

/// A floating red panel that lets whatever's beneath show through (no
/// `background_color`).
#[component(inline_props)]
fn TransparentPanel(top: i32, left: i32, text: &'static str) -> View {
    view! {
        div(
            border_color = Color::Red,
            border_style = BorderStyle::DoubleTopBottom,
            padding = 1,
            position = Position::Absolute,
            top = top,
            left = left,
        ) {
            p { (text) }
        }
    }
}

/// A floating red panel that paints `Color::Reset` to fully cover what's
/// beneath it.
#[component(inline_props)]
fn OpaquePanel(top: i32, left: i32, text: &'static str) -> View {
    view! {
        div(
            background_color = Color::Reset,
            border_color = Color::Red,
            border_style = BorderStyle::DoubleTopBottom,
            padding = 1,
            position = Position::Absolute,
            top = top,
            left = left,
        ) {
            p { (text) }
        }
    }
}

#[component]
fn App() -> View {
    view! {
        div(
            border_style = BorderStyle::DoubleLeftRight,
            border_color = Color::Green,
            margin = 1,
            width = 78,
            flex_direction = FlexDirection::Column,
        ) {
            Title(text = " Overlap Example ")
            Body()

            // First panel: transparent — the lorem shows through.
            TransparentPanel(top = 2, left = 4, text = "This element is overlapping the text!")
            // Second panel: opaque — paints its background and covers the lorem.
            OpaquePanel(top = 8, left = 4, text = "We can cover it up by setting a background color.")
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
