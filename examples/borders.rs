//! A quick visual tour of every `BorderStyle`. Each row is its own component
//! so the layout becomes "two rows of styled boxes" at a glance.
//!
//! Run with: `cargo run --example borders`.

use iodilos::prelude::*;

/// One labeled, framed box.
#[component(inline_props)]
fn StyledBox(label: &'static str, style: BorderStyle) -> View {
    view! {
        div(border_style = style) { p { (label) } }
    }
}

/// A horizontal row of `StyledBox`es spaced with a 2-cell gap.
#[component(inline_props)]
fn Row(children: Children<View>) -> View {
    let children = children.call();
    view! {
        div(flex_direction = FlexDirection::Row, gap = 2) {
            (children)
        }
    }
}

#[component]
fn App() -> View {
    view! {
        div(flex_direction = FlexDirection::Column, padding = 2, gap = 1) {
            Row {
                StyledBox(label = "Single", style = BorderStyle::Single)
                StyledBox(label = "Double", style = BorderStyle::Double)
                StyledBox(label = "Round",  style = BorderStyle::Round)
                StyledBox(label = "Bold",   style = BorderStyle::Bold)
            }
            Row {
                StyledBox(label = "DoubleLeftRight", style = BorderStyle::DoubleLeftRight)
                StyledBox(label = "DoubleTopBottom", style = BorderStyle::DoubleTopBottom)
                StyledBox(label = "Classic",         style = BorderStyle::Classic)
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
