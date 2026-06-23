//! Placeholder for an `iocraft`-style `use_output` hook (not yet implemented in
//! iodilos). The example just renders the visual shell so you can see what the
//! final demo will look like.
//!
//! Run with: `cargo run --example use_output`.

use iodilos::prelude::*;

#[component]
fn App() -> View {
    view! {
        div(flex_direction = FlexDirection::Column, gap = 1, padding = 1) {
            div(border_style = BorderStyle::Round, border_color = Color::Green) {
                p { "Hello, use_output!" }
            }
            p(color = Color::Grey) {
                "sycamore-tui does not have iocraft's use_output hook yet; this keeps the visual shell for inspection."
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
