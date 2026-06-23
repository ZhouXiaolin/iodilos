//! Form with label/input rows abstracted into a `Field` component.
//!
//! Run with: `cargo run --example form`.

use iodilos::prelude::*;

/// One label + input row. The label is fixed-width so multiple `Field`s stack
/// with aligned input boxes.
#[component(inline_props)]
fn Field(
    label: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
    height: i32,
) -> View {
    view! {
        div(flex_direction = FlexDirection::Row, gap = 1) {
            div(width = 15) { p { (label) } }
            div(background_color = Color::DarkGrey, width = 30, height = height) {
                input(bind:value = value, placeholder = placeholder)
            }
        }
    }
}

/// The form heading + subtitle.
#[component]
fn Heading() -> View {
    view! {
        div(flex_direction = FlexDirection::Column, align_items = AlignItems::CENTER) {
            p(color = Color::White, weight = Weight::Bold) { "What's your name?" }
            p(color = Color::Grey) {
                "Tab through fields; click Submit to render the greeting."
            }
        }
    }
}

#[component]
fn App() -> View {
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
        div(
            flex_direction = FlexDirection::Column,
            align_items = AlignItems::CENTER,
            margin = 2,
            gap = 1,
        ) {
            Heading()

            Field(label = "First Name: ", value = first_name, placeholder = "first",            height = 1)
            Field(label = "Last Name: ",  value = last_name,  placeholder = "last",             height = 1)
            Field(label = "Life Story: ", value = life_story, placeholder = "single-line for now", height = 5)

            button(on:click = move |_| submitted.set(true)) { "Submit" }
            p(color = Color::Green, weight = Weight::Bold) { (greeting) }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
