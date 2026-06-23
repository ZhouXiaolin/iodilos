use iodilos::prelude::*;

/// A static heading component.
///
/// Demonstrates `#[component(inline_props)]`: the `Heading_Props { title }`
/// struct is synthesised directly from the function parameter — no separate
/// `#[derive(Props)]` struct needed.
#[component(inline_props)]
fn Heading(title: &'static str) -> View {
    view! {
        p(color = Color::Yellow) { (title) }
    }
}

/// The +/- / Reset button row.
#[component(inline_props)]
fn Controls(count: Signal<i32>) -> View {
    let mut count = count;
    view! {
        div(flex_direction = FlexDirection::Row, gap = 1) {
            button(on:click = move |_| count -= 1)        { "-" }
            button(on:click = move |_| count += 1)        { "+" }
            button(on:click = move |_| count.set(0))      { "Reset" }
        }
    }
}

#[component]
fn App() -> View {
    let count = create_signal(0i32);

    view! {
        // The panel: a column with a 1-cell gap and 1-cell padding, drawn with a
        // rounded single-line border.
        div(
            flex_direction = FlexDirection::Column,
            gap = 1,
            padding = 1,
            border_style = BorderStyle::Round,
        ) {
            Heading(title = "Counter")

            p(color = Color::Cyan) { "Value: " (count) }

            Controls(count = count)

            // `Show` renders its children only while `when` is true. The
            // condition is reactive, so the hint appears/disappears as the
            // counter changes.
            Show(when = move || count.get() != 0) {
                p(color = Color::DarkGrey) { "press Reset to clear" }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
