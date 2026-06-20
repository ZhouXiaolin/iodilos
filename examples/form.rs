use iodilos::prelude::*;

fn app() -> View {
    let first_name = create_signal(String::new());
    let last_name = create_signal(String::new());
    let life_story = create_signal(String::new());
    let submitted = create_signal(false);
    let greeting = create_memo(move || {
        if submitted.get() {
            format!(
                "Hello, {} {}! What a fascinating life story!",
                first_name.get_clone(),
                last_name.get_clone()
            )
        } else {
            String::new()
        }
    });

    view! {
        div(flex_direction = FlexDirection::Column, align_items = AlignItems::CENTER, margin = 2, gap = 1) {
            div(flex_direction = FlexDirection::Column, align_items = AlignItems::CENTER) {
                p(color = Color::White, weight = Weight::Bold) { "What's your name?" }
                p(color = Color::Grey) { "Tab through fields; click Submit to render the greeting." }
            }
            div(flex_direction = FlexDirection::Row, gap = 1) {
                div(width = 15) { p { "First Name: " } }
                div(background_color = Color::DarkGrey, width = 30) {
                    input(bind:value=first_name, placeholder="first")
                }
            }
            div(flex_direction = FlexDirection::Row, gap = 1) {
                div(width = 15) { p { "Last Name: " } }
                div(background_color = Color::DarkGrey, width = 30) {
                    input(bind:value=last_name, placeholder="last")
                }
            }
            div(flex_direction = FlexDirection::Row, gap = 1) {
                div(width = 15) { p { "Life Story: " } }
                div(background_color = Color::DarkGrey, width = 30, height = 5) {
                    input(bind:value=life_story, placeholder="single-line for now")
                }
            }
            button(on:click=move |_| submitted.set(true)) { "Submit" }
            p(color = Color::Green, weight = Weight::Bold) { (greeting) }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
