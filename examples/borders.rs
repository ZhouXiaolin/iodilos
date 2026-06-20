use iodilos::prelude::*;

fn app() -> View {
    view! {
        div(flex_direction = FlexDirection::Column, padding = 2, gap = 1) {
            div(flex_direction = FlexDirection::Row, gap = 2) {
                div(border_style = BorderStyle::Single) { p { "Single" } }
                div(border_style = BorderStyle::Double) { p { "Double" } }
                div(border_style = BorderStyle::Round) { p { "Round" } }
                div(border_style = BorderStyle::Bold) { p { "Bold" } }
            }
            div(flex_direction = FlexDirection::Row, gap = 2) {
                div(border_style = BorderStyle::DoubleLeftRight) { p { "DoubleLeftRight" } }
                div(border_style = BorderStyle::DoubleTopBottom) { p { "DoubleTopBottom" } }
                div(border_style = BorderStyle::Classic) { p { "Classic" } }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
