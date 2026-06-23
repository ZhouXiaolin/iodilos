//! `Indexed` (non-keyed / index-keyed iteration) + `Show`.
//!
//! Run with: `cargo run --example indexed`
//!
//! An append-only event log. `Indexed` reuses each row by index, so appending a
//! line only maps the new index; clearing drops every per-item scope at once.
//! Contrast with `list.rs`, which uses `Keyed` (reuse by stable identity).

use iodilos::prelude::*;

fn app() -> View {
    let log = create_signal(Vec::<String>::new());
    let mut step = create_signal(0u32);

    view! {
        div(
            flex_direction = FlexDirection::Column,
            gap = 1,
            padding = 1,
            border_style = BorderStyle::Round,
        ) {
            p(color = Color::Cyan) { "Events: " (move || log.get_clone().len()) }

            div(flex_direction = FlexDirection::Row, gap = 1) {
                button(on:click = move |_| {
                    step += 1;
                    let s = step.get();
                    log.update(|l| l.push(format!("event #{s}")));
                }) { "+ event" }
                button(on:click = move |_| log.set(Vec::new())) { "clear" }
            }

            // Indexed iteration: the closure runs per index. Appending maps
            // only the new tail; reordering would re-map (no stable identity),
            // which is the trade-off vs `Keyed`.
            Indexed(
                list = log,
                view = |line| view! { p { "• " (line) } },
            )

            // Empty-state hint via `Show`.
            Show(when = move || log.get_clone().is_empty()) {
                p(color = Color::DarkGrey) { "(no events yet)" }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
