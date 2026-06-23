//! Animated progress bar driven by a `Signal<f32>`, with the bar and label
//! split into reusable components.
//!
//! Run with: `cargo run --example progress_bar`.

use std::time::Duration;

use futures_timer::Delay;
use iodilos::prelude::*;

/// A horizontal bar whose filled portion is `percent` of the parent width.
#[component(inline_props)]
fn ProgressBar(percent: ReadSignal<f32>, color: Color, total_width: i32) -> View {
    view! {
        div(border_style = BorderStyle::Round, border_color = Color::Blue, width = total_width) {
            div(
                width = move || Size::Percent(percent.get()),
                height = 1,
                background_color = color,
            )
        }
    }
}

/// A right-padded label like "57%".
#[component(inline_props)]
fn PercentLabel(percent: ReadSignal<f32>) -> View {
    let label = create_memo(move || format!("{:.0}%", percent.get()));
    view! {
        div(flex_direction = FlexDirection::Row, gap = 1) {
            p(padding_left = 1) { (label) }
        }
    }
}

#[component]
fn App() -> View {
    let progress = create_signal(0.0_f32);

    use_future(async move {
        loop {
            Delay::new(Duration::from_millis(100)).await;
            let next = (progress.get_untracked() + 2.0).min(100.0);
            progress.set(next);
            if next >= 100.0 { break; }
        }
    });

    view! {
        div(flex_direction = FlexDirection::Column, gap = 1, padding = 1) {
            ProgressBar(percent = *progress, color = Color::Green, total_width = 60)
            PercentLabel(percent = *progress)
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
