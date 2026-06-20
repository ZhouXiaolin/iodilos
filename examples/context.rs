use iodilos::prelude::*;

#[derive(Clone, Copy)]
struct NumberOfTheDay(i32);

fn app() -> View {
    provide_context(NumberOfTheDay(42));
    let number = use_context::<NumberOfTheDay>();

    view! {
        div(border_style = BorderStyle::Round, border_color = Color::Cyan) {
            p {
                "The number of the day is... "
                span(color = Color::Green, weight = Weight::Bold) { (number.0) }
                "!"
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
