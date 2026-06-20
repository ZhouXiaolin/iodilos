use iodilos::prelude::*;

fn app() -> View {
    let mut count = create_signal(0i32);
    let value = create_signal(String::new());

    view! {
        // The panel: a column with a 1-cell gap and 1-cell padding, drawn with a
        // rounded single-line border.
        div(
            flex_direction = FlexDirection::Column,
            gap = 1,
            padding = 1,
            border_style = BorderStyle::Round,
        ) {
            p(color = Color::Cyan) { "Value: " (count) }
            // The button row: laid out left-to-right with a 1-cell gap.
            div(flex_direction = FlexDirection::Row, gap = 1) {
                button(on:click=move |_| count -= 1) { "-" }
                button(on:click=move |_| count += 1) { "+" }
                button(on:click=move |_| count.set(0)) { "Reset" }
            }
            input(bind:value=value, placeholder="type here")
            p { "Input: " (value) }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
