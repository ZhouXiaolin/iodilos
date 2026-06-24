//! Element-scroll spike: a bordered container whose children scroll natively
//! (no text re-slicing). Verifies that `scroll` on a `div`:
//!   - shifts the child subtree up, clipping to the element's content box
//!   - keeps the border / background correctly placed (not shifted)
//!   - composes with `overflow = Hidden`
//!   - `i32::MAX` sticks to bottom
//!
//! Run with: `cargo run --example scroll_element`
//!
//! Controls: ↑/↓ scroll 1, PgUp/PgDn scroll 5, End sticks to bottom, Home to top, q to quit.

use crossterm::event::{KeyCode, KeyEventKind};
use iodilos::prelude::*;

const TOTAL_LINES: i32 = 20;
const VIEWPORT: i32 = 6;

#[component]
fn App() -> View {
    // `i32::MAX` is the stick-to-bottom sentinel.
    let offset = create_signal(0i32);
    let at_bottom = create_signal(false);
    // Static list of line indices; Indexed maps each to a rich div.
    let lines = create_signal((0..TOTAL_LINES).collect::<Vec<_>>());

    let clamp = move |mut v: i32| {
        let max = (TOTAL_LINES - VIEWPORT).max(0);
        if v < 0 {
            v = 0;
        }
        if v > max {
            v = max;
        }
        v
    };

    view! {
        div(
            flex_direction = FlexDirection::Column,
            padding = 1,
            gap = 1,
            tabindex = "0",
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release { return; }
                match key.code {
                    KeyCode::Up => { offset.set(clamp(offset.get() - 1)); at_bottom.set(false); }
                    KeyCode::Down => { offset.set(clamp(offset.get() + 1)); }
                    KeyCode::PageUp => { offset.set(clamp(offset.get() - 5)); at_bottom.set(false); }
                    KeyCode::PageDown => { offset.set(clamp(offset.get() + 5)); }
                    KeyCode::Home => { offset.set(0); at_bottom.set(false); }
                    KeyCode::End => { at_bottom.set(true); }
                    KeyCode::Char('q') => iodilos::quit(),
                    _ => {}
                }
            },
        ) {
            // Header / instructions.
            p(color = Color::Cyan) {
                "Element scroll spike — up/down scroll, End=stick-to-bottom, q=quit"
            }

            // The scrollable container: fixed height, hidden overflow, scroll
            // drives the child shift. scroll is reactive (closure form).
            div(
                width = 40,
                height = VIEWPORT,
                overflow = Overflow::Hidden,
                border_style = BorderStyle::Round,
                border_color = Color::Green,
                background_color = Color::DarkGrey,
                scroll = move || if at_bottom.get() { i32::MAX } else { offset.get() },
            ) {
                Indexed(
                    list = lines,
                    view = move |i| {
                        let bg = if i % 5 == 0 {
                            Color::Rgb { r: 60, g: 40, b: 80 }
                        } else {
                            Color::Reset
                        };
                        let marker = if i == 0 { " (TOP)" } else if i == TOTAL_LINES - 1 { " (BOTTOM)" } else { "" };
                        view! {
                            div(
                                flex_direction = FlexDirection::Row,
                                background_color = bg,
                            ) {
                                span(color = Color::Yellow) { (format!("L{:02}", i)) }
                                span(color = Color::White) { (format!("  line content{}", marker)) }
                            }
                        }
                    },
                )
            }

            // Status line showing current offset.
            p(color = Color::DarkGrey) {
                (move || format!("offset={} stick_to_bottom={}", offset.get(), at_bottom.get()))
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
