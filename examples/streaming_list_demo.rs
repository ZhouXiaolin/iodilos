//! StreamingList demo — keyed scrollable list of richly-shaped items.
//!
//! Each item is a self-contained "card" with its own border/bg/padding, and
//! the whole list scrolls natively via the element `scroll` style. Compared to
//! `scroll_element.rs` (which hand-builds the same container with an
//! `Indexed`), this drives the keyed engine end-to-end: appended items keep
//! their identity, and a per-item signal (the body string) updates in place
//! without re-running the view closure.
//!
//! Run with: `cargo run --example streaming_list_demo`
//!
//! Controls:
//!   ↑/↓     scroll one row
//!   PgUp/Dn scroll five rows
//!   Home    top
//!   End     stick to bottom
//!   a       append a new card
//!   s       stream a token onto the last card's body
//!   q       quit

use crossterm::event::{KeyCode, KeyEventKind};
use iodilos::prelude::*;

#[derive(Clone, PartialEq)]
struct Card {
    id: u64,
    title: String,
    // Per-item Signal: streaming updates target this, not the parent Vec.
    body: Signal<String>,
}

#[component]
fn App() -> View {
    let offset = create_signal(0i32);
    let at_bottom = create_signal(true);
    let next_id = create_signal(3u64);

    // Seed the list with three cards.
    let cards = create_signal(vec![
        Card {
            id: 0,
            title: "card 0".into(),
            body: create_signal("Initial body for card 0.".into()),
        },
        Card {
            id: 1,
            title: "card 1".into(),
            body: create_signal("Initial body for card 1.".into()),
        },
        Card {
            id: 2,
            title: "card 2".into(),
            body: create_signal("Initial body for card 2.".into()),
        },
    ]);

    let clamp_offset = move |v: i32| v.max(0);

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
                    KeyCode::Up => { offset.set(clamp_offset(offset.get() - 1)); at_bottom.set(false); }
                    KeyCode::Down => { offset.set(clamp_offset(offset.get() + 1)); }
                    KeyCode::PageUp => { offset.set(clamp_offset(offset.get() - 5)); at_bottom.set(false); }
                    KeyCode::PageDown => { offset.set(clamp_offset(offset.get() + 5)); }
                    KeyCode::Home => { offset.set(0); at_bottom.set(false); }
                    KeyCode::End => { at_bottom.set(true); offset.set(0); }
                    KeyCode::Char('a') => {
                        let id = next_id.get();
                        next_id.set(id + 1);
                        let mut v = cards.get_clone();
                        v.push(Card {
                            id,
                            title: format!("card {id}"),
                            body: create_signal(format!("Fresh body for card {id}.")),
                        });
                        cards.set(v);
                    }
                    KeyCode::Char('s') => {
                        // Stream a token onto the last card's body via its
                        // per-item Signal. The view closure is NOT re-run; the
                        // body's own from_dynamic reactive region patches in place.
                        let v = cards.get_clone();
                        if let Some(last) = v.last() {
                            let body = last.body;
                            let mut s = body.get_clone();
                            s.push_str(" +tok");
                            body.set(s);
                        }
                    }
                    KeyCode::Char('q') => iodilos::quit(),
                    _ => {}
                }
            },
        ) {
            p(color = Color::Cyan) {
                "StreamingList demo — ↑↓ scroll, End stick-to-bottom, a append, s stream, q quit"
            }

            // The scrollable viewport. Fixed height; the list of cards
            // overflows and `scroll` drives the element-level shift.
            div(
                width = 50,
                height = 10,
                border_style = BorderStyle::Round,
                border_color = Color::Green,
            ) {
                StreamingList(
                    items = *cards,
                    key = |c: &Card| c.id,
                    view = |c: &Card| {
                        let title = c.title.clone();
                        let body = c.body;
                        let bg = if c.id % 2 == 0 {
                            Color::Rgb { r: 30, g: 30, b: 60 }
                        } else {
                            Color::Rgb { r: 60, g: 30, b: 30 }
                        };
                        view! {
                            div(
                                flex_direction = FlexDirection::Column,
                                border_style = BorderStyle::Single,
                                border_color = Color::DarkGrey,
                                background_color = bg,
                                padding_left = 1,
                                padding_right = 1,
                            ) {
                                p(color = Color::Yellow, weight = Weight::Bold) { (title) }
                                p(color = Color::White) { (move || body.get_clone()) }
                            }
                        }
                    },
                    scroll = move || if at_bottom.get() { i32::MAX } else { offset.get() },
                )
            }

            // Status line.
            p(color = Color::DarkGrey) {
                (move || format!(
                    "cards={} offset={} stick_to_bottom={}",
                    cards.get_clone().len(),
                    offset.get(),
                    at_bottom.get(),
                ))
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
