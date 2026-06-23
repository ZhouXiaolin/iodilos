//! Mini "directional pad" demo: clicking the buttons moves a `@` inside a
//! bordered play area.
//!
//! Run with: `cargo run --example use_input`.

use iodilos::prelude::*;

const AREA_WIDTH: i32 = 40;
const AREA_HEIGHT: i32 = 9;
const FACE: &str = "@";

/// The bordered play area: a fixed-size box with `@` offset by `(x, y)`.
#[component(inline_props)]
fn PlayArea(x: ReadSignal<i32>, y: ReadSignal<i32>) -> View {
    view! {
        div(
            border_style = BorderStyle::Round,
            border_color = Color::Green,
            height = Size::Length((AREA_HEIGHT + 2) as u32),
            width  = Size::Length((AREA_WIDTH + 2) as u32),
        ) {
            div(
                padding_left = move || Padding::Length(x.get() as u32),
                padding_top  = move || Padding::Length(y.get() as u32),
            ) {
                p { (FACE) }
            }
        }
    }
}

/// The 4-button d-pad. Each handler clamps the position into the area.
#[component(inline_props)]
fn DPad(x: Signal<i32>, y: Signal<i32>) -> View {
    view! {
        div(flex_direction = FlexDirection::Row, gap = 1) {
            button(on:click = move |_| y.set((y.get() - 1).max(0)))                { "Up" }
            button(on:click = move |_| y.set((y.get() + 1).min(AREA_HEIGHT - 1))) { "Down" }
            button(on:click = move |_| x.set((x.get() - 1).max(0)))                { "Left" }
            button(on:click = move |_| x.set((x.get() + 1).min(AREA_WIDTH - 1)))  { "Right" }
        }
    }
}

#[component]
fn App() -> View {
    let x = create_signal(0i32);
    let y = create_signal(0i32);

    view! {
        div(
            flex_direction = FlexDirection::Column,
            padding = 2,
            align_items = AlignItems::CENTER,
            gap = 1,
        ) {
            p { "Use the buttons to move. Press q to exit." }
            PlayArea(x = *x, y = *y)
            DPad(x = x, y = y)
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
