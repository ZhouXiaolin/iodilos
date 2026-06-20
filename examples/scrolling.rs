use crossterm::event::{KeyCode, KeyEventKind, MouseEventKind};
use iodilos::prelude::*;

const LINE_COUNT: i32 = 100;
const VIEWPORT_HEIGHT: i32 = 8;

fn max_offset() -> i32 {
    (LINE_COUNT - VIEWPORT_HEIGHT).max(0)
}

fn clamp_offset(offset: i32) -> i32 {
    offset.clamp(0, max_offset())
}

fn visible_lines(offset: i32) -> String {
    (offset..offset + VIEWPORT_HEIGHT)
        .map(|i| format!("Line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn app() -> View {
    let offset = create_signal(0i32);
    let text = create_memo(move || visible_lines(offset.get()));
    let scroll_by = move |delta: i32| offset.set(clamp_offset(offset.get() + delta));

    view! {
        div(
            flex_direction = FlexDirection::Column,
            padding = 2,
            align_items = AlignItems::CENTER,
            gap = 1,
            tabindex = "0",
            on:raw_key=move |event: Event| {
                let Some(key) = event.key() else {
                    return;
                };
                if key.kind == KeyEventKind::Release {
                    return;
                }
                match key.code {
                    KeyCode::Up => scroll_by(-1),
                    KeyCode::Down => scroll_by(1),
                    KeyCode::PageUp => scroll_by(-VIEWPORT_HEIGHT),
                    KeyCode::PageDown => scroll_by(VIEWPORT_HEIGHT),
                    KeyCode::Home => offset.set(0),
                    KeyCode::End => offset.set(max_offset()),
                    _ => {}
                }
            },
            on:raw_mouse=move |event: Event| {
                let Some(mouse) = event.mouse() else {
                    return;
                };
                match mouse.kind {
                    MouseEventKind::ScrollUp => scroll_by(-3),
                    MouseEventKind::ScrollDown => scroll_by(3),
                    _ => {}
                }
            },
        ) {
            p { "Use arrow keys or mouse wheel to scroll. Press q to quit." }
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
            div(flex_direction = FlexDirection::Row, gap = 1) {
                button(on:click=move |_| scroll_by(-1)) { "Prev" }
                button(on:click=move |_| scroll_by(1)) { "Next" }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
