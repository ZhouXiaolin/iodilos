//! A table demo, fully decomposed into components.
//!
//! Run with: `cargo run --example table`
//!
//! Every visual piece is a `#[component(inline_props)]`: the header cells (`Th`),
//! the data row (`Row`), and the table frame itself (`Table`). The example body
//! is just `view!` — no helper fns.

use iodilos::prelude::*;

/// A bold, underlined header cell that takes a percentage of the row width.
#[component(inline_props)]
fn Th(label: &'static str, width_pct: f32, align_end: bool) -> View {
    view! {
        div(
            width = Size::Percent(width_pct),
            justify_content = if align_end { JustifyContent::END } else { JustifyContent::START },
            padding_right = if align_end { 2 } else { 0 },
        ) {
            p(weight = Weight::Bold, underline = true) { (label) }
        }
    }
}

/// One data row: id (right-aligned), name, email, with optional shading.
#[component(inline_props)]
fn Row(id: i32, name: &'static str, email: &'static str, shaded: bool) -> View {
    let bg = if shaded { Color::DarkGrey } else { Color::Reset };

    view! {
        div(flex_direction = FlexDirection::Row, background_color = bg) {
            div(width = Size::Percent(10.0), justify_content = JustifyContent::END, padding_right = 2) {
                p { (id) }
            }
            div(width = Size::Percent(40.0)) { p { (name) } }
            div(width = Size::Percent(50.0)) { p { (email) } }
        }
    }
}

#[component]
fn App() -> View {
    view! {
        div(
            flex_direction = FlexDirection::Column,
            width = 60,
            margin_top = 1,
            margin_bottom = 1,
            border_style = BorderStyle::Round,
            border_color = Color::Cyan,
        ) {
            // Header row: a `Single` BOTTOM-edge border separates it from the body.
            div(
                flex_direction = FlexDirection::Row,
                border_style = BorderStyle::Single,
                border_edges = Edges::BOTTOM,
                border_color = Color::Grey,
            ) {
                Th(label = "Id",    width_pct = 10.0, align_end = true)
                Th(label = "Name",  width_pct = 40.0, align_end = false)
                Th(label = "Email", width_pct = 50.0, align_end = false)
            }

            Row(id = 1, name = "Alice",   email = "alice@example.com",   shaded = false)
            Row(id = 2, name = "Bob",     email = "bob@example.com",     shaded = true)
            Row(id = 3, name = "Charlie", email = "charlie@example.com", shaded = false)
            Row(id = 4, name = "David",   email = "david@example.com",   shaded = true)
            Row(id = 5, name = "Eve",     email = "eve@example.com",     shaded = false)
            Row(id = 6, name = "Frank",   email = "frank@example.com",   shaded = true)
            Row(id = 7, name = "Grace",   email = "grace@example.com",   shaded = false)
            Row(id = 8, name = "Heidi",   email = "heidi@example.com",   shaded = true)
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
