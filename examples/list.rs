//! Dynamic lists with the new component shell: `#[component]` / `#[derive(Props)]`,
//! plus the `Keyed` and `Show` control-flow components.
//!
//! Run with: `cargo run --example list`
//!
//! Keys: `+` / `-` buttons (click or focus+Enter) grow/shrink the list. Watch
//! the empty-state hint toggle via `Show`, and the rows update via `Keyed`.

use iodilos::prelude::*;

/// A presentational row component.
///
/// Demonstrates an explicit `#[derive(Props)]` struct (vs `inline_props`):
/// useful when you want named, type-checked, optionally-defaulted fields on a
/// stable type. The `view!` macro invokes it as `Row(index=.., label=..)`.
#[derive(Props)]
struct RowProps {
    index: i32,
    label: String,
}

#[component]
fn Row(props: RowProps) -> View {
    view! {
        div(flex_direction = FlexDirection::Row, gap = 1) {
            p(color = Color::DarkGrey) { (props.index) "." }
            p { (props.label) }
        }
    }
}

fn app() -> View {
    let mut count = create_signal(0i32);
    // Derive `[1, 2, ..., count]` reactively from the counter. `items` is a
    // `ReadSignal<Vec<i32>>`, which `Keyed` accepts as its `list`.
    let items = create_memo(move || (1..=count.get()).collect::<Vec<i32>>());

    view! {
        div(
            flex_direction = FlexDirection::Column,
            gap = 1,
            padding = 1,
            border_style = BorderStyle::Round,
        ) {
            p(color = Color::Cyan) { "Items: " (count) }

            div(flex_direction = FlexDirection::Row, gap = 1) {
                button(on:click = move |_| count -= 1) { "-" }
                button(on:click = move |_| count += 1) { "+" }
            }

            // Keyed iteration: each value is its own key. On a counter change,
            // only items whose value actually changed re-run the `view`
            // closure; the rest are reused (matched by key), and each item's
            // reactive scope is cleaned up when it leaves the list.
            Keyed(
                list = items,
                key = |n| *n,
                view = |n| view! {
                    Row(index = n, label = format!("entry #{n}"))
                },
            )

            // `Show` renders an empty-state hint only while the list is empty.
            Show(when = move || count.get() == 0) {
                p(color = Color::DarkGrey) { "(empty — press + to add)" }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
