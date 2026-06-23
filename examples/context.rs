use iodilos::prelude::*;

#[derive(Clone, Copy)]
struct NumberOfTheDay(i32);

/// Reads the contextual value and renders the announcement. Splitting it out
/// keeps `App` focused on context wiring.
#[component]
fn Announcement() -> View {
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

#[component]
fn App() -> View {
    provide_context(NumberOfTheDay(42));

    view! {
        Announcement()
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
