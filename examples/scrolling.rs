//! A 100-line virtual scroll backed by a single offset signal, fully
//! componentised.
//!
//! Run with: `cargo run --example scrolling`.
//!
//! The viewport, the inline `Spans` of rendered lines, and the bottom
//! prev/next buttons are each their own component.

use crossterm::event::{KeyCode, KeyEventKind, MouseEventKind};
use iodilos::prelude::*;

const LINE_COUNT: i32 = 100;
const VIEWPORT_HEIGHT: i32 = 8;

fn max_offset() -> i32 { (LINE_COUNT - VIEWPORT_HEIGHT).max(0) }

fn visible_lines(offset: i32) -> String {
    (offset..offset + VIEWPORT_HEIGHT)
        .map(|i| format!("Line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The bordered scrolling region: text inside a `Hidden` overflow box.
#[component(inline_props)]
fn Viewport(text: ReadSignal<String>) -> View {
    view! {
        div(
            border_style = BorderStyle::DoubleLeftRight,
            border_color = Color::Green,
            width = 78,
            height = 10,
            overflow = Overflow::Hidden,
        ) {
            div(overflow = Overflow::Hidden, width = Size::Percent(100.0), height = Size::Percent(100.0)) {
                p { (text) }
            }
        }
    }
}

/// Prev / Next button row.
#[component(inline_props)]
fn ScrollControls(
    on_prev: impl FnMut(Event) + 'static,
    on_next: impl FnMut(Event) + 'static,
) -> View {
    view! {
        div(flex_direction = FlexDirection::Row, gap = 1) {
            button(on:click = on_prev) { "Prev" }
            button(on:click = on_next) { "Next" }
        }
    }
}

#[component]
fn App() -> View {
    let offset = create_signal(0i32);
    let text = create_memo(move || visible_lines(offset.get()));
    let scroll_by =
        move |delta: i32| offset.set((offset.get() + delta).clamp(0, max_offset()));

    view! {
        div(
            flex_direction = FlexDirection::Column,
            padding = 2,
            align_items = AlignItems::CENTER,
            gap = 1,
            tabindex = "0",
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release { return; }
                match key.code {
                    KeyCode::Up => scroll_by(-1),
                    KeyCode::Down => scroll_by(1),
                    KeyCode::PageUp => scroll_by(-VIEWPORT_HEIGHT),
                    KeyCode::PageDown => scroll_by(VIEWPORT_HEIGHT),
                    KeyCode::Home => offset.set(0),
                    KeyCode::End  => offset.set(max_offset()),
                    _ => {}
                }
            },
            on:raw_mouse = move |event: Event| {
                let Some(mouse) = event.mouse() else { return; };
                match mouse.kind {
                    MouseEventKind::ScrollUp => scroll_by(-3),
                    MouseEventKind::ScrollDown => scroll_by(3),
                    _ => {}
                }
            },
        ) {
            p { "Use arrow keys or mouse wheel to scroll. Press q to quit." }
            Viewport(text = text)
            ScrollControls(
                on_prev = move |_| scroll_by(-1),
                on_next = move |_| scroll_by(1),
            )
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
