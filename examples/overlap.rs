use iodilos::prelude::*;

const LOREM: &str = "Lorem ipsum odor amet, consectetuer adipiscing elit. \
Lobortis hendrerit nec ipsum dapibus quam. Donec malesuada tincidunt elementum \
mollis vehicula quisque purus. Est volutpat integer, donec sagittis placerat \
fermentum phasellus ipsum sollicitudin. Tempus laoreet ad tempus aptent proin \
per donec lectus. Quisque auctor urna; phasellus urna tortor ligula. Class \
pharetra bibendum tristique, quisque consectetur placerat potenti. Imperdiet ut \
torquent vestibulum eleifend bibendum et. Dictumst vulputate interdum iaculis \
at conubia venenatis.";

fn app() -> View {
    view! {
        div(
            border_style = BorderStyle::DoubleLeftRight,
            border_color = Color::Green,
            margin = 1,
            width = 78,
            flex_direction = FlexDirection::Column,
        ) {
            div(margin_top = -1) {
                p { " Overlap Example " }
            }
            div(padding = 1) {
                p(color = Color::DarkGrey, weight = Weight::Light) { (format!("{LOREM} {LOREM}")) }
            }
            div(
                border_color = Color::Red,
                border_style = BorderStyle::DoubleTopBottom,
                padding = 1,
                position = Position::Absolute,
                top = 2,
                left = 4,
            ) {
                p { "This element is overlapping the text!" }
            }
            div(
                background_color = Color::Reset,
                border_color = Color::Red,
                border_style = BorderStyle::DoubleTopBottom,
                padding = 1,
                position = Position::Absolute,
                top = 8,
                left = 4,
            ) {
                p { "We can cover it up by setting a background color." }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
