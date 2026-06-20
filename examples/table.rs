use iodilos::prelude::*;

fn row(id: i32, name: &'static str, email: &'static str, shaded: bool) -> View {
    let bg = if shaded {
        Some(Color::DarkGrey)
    } else {
        None
    };

    view! {
        div(flex_direction = FlexDirection::Row, background_color = move || bg.unwrap_or(Color::Reset)) {
            div(width = Size::Percent(10.0), justify_content = JustifyContent::END, padding_right = 2) {
                p { (id) }
            }
            div(width = Size::Percent(40.0)) {
                p { (name) }
            }
            div(width = Size::Percent(50.0)) {
                p { (email) }
            }
        }
    }
}

fn app() -> View {
    view! {
        div(
            flex_direction = FlexDirection::Column,
            width = 60,
            margin_top = 1,
            margin_bottom = 1,
            border_style = BorderStyle::Round,
            border_color = Color::Cyan,
        ) {
            div(
                flex_direction = FlexDirection::Row,
                border_style = BorderStyle::Single,
                border_edges = Edges::BOTTOM,
                border_color = Color::Grey,
            ) {
                div(width = Size::Percent(10.0), justify_content = JustifyContent::END, padding_right = 2) {
                    p(weight = Weight::Bold, underline = true) { "Id" }
                }
                div(width = Size::Percent(40.0)) {
                    p(weight = Weight::Bold, underline = true) { "Name" }
                }
                div(width = Size::Percent(50.0)) {
                    p(weight = Weight::Bold, underline = true) { "Email" }
                }
            }
            (row(1, "Alice", "alice@example.com", false))
            (row(2, "Bob", "bob@example.com", true))
            (row(3, "Charlie", "charlie@example.com", false))
            (row(4, "David", "david@example.com", true))
            (row(5, "Eve", "eve@example.com", false))
            (row(6, "Frank", "frank@example.com", true))
            (row(7, "Grace", "grace@example.com", false))
            (row(8, "Heidi", "heidi@example.com", true))
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
