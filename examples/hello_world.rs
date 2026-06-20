use iodilos::prelude::*;

fn app() -> View {
    view! {
        div(border_style = BorderStyle::Round, border_color = Color::Blue) {
            p { "Hello, world!" }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
