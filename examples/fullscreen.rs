use iodilos::prelude::*;

/// Centered "card" component: rounded border, padded interior, takes any
/// children.
#[component(inline_props)]
fn Card(children: Children<View>) -> View {
    let children = children.call();
    view! {
        div(
            border_style = BorderStyle::Round,
            border_color = Color::Blue,
            margin_bottom = 2,
            padding_top = 2,
            padding_bottom = 2,
            padding_left = 8,
            padding_right = 8,
        ) {
            (children)
        }
    }
}

#[component]
fn App() -> View {
    view! {
        div(
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            background_color = Color::DarkGrey,
            border_style = BorderStyle::Double,
            border_color = Color::Blue,
            flex_direction = FlexDirection::Column,
            align_items = AlignItems::CENTER,
            justify_content = JustifyContent::CENTER,
        ) {
            Card {
                p { "Current Time: static sycamore-tui port" }
            }
            p { "Press q to quit." }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
