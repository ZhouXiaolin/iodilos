use std::time::Duration;

use futures_timer::Delay;
use iodilos::prelude::*;

fn app() -> View {
    let progress = create_signal(0.0_f32);
    let label = create_memo(move || format!("{:.0}%", progress.get()));

    use_future(async move {
        loop {
            Delay::new(Duration::from_millis(100)).await;
            let next = (progress.get_untracked() + 2.0).min(100.0);
            progress.set(next);
            if next >= 100.0 {
                break;
            }
        }
    });

    view! {
        div(flex_direction = FlexDirection::Column, gap = 1, padding = 1) {
            div(border_style = BorderStyle::Round, border_color = Color::Blue, width = 60) {
                div(
                    width = move || Size::Percent(progress.get()),
                    height = 1,
                    background_color = Color::Green,
                )
            }
            div(flex_direction = FlexDirection::Row, gap = 1) {
                p(padding_left = 1) { (label) }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
