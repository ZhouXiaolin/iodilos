//! Mouse-driven swatches + a draggable box, fully decomposed into components.
//!
//! Run with: `cargo run --example mouse`
//!
//! Every interactive piece is a `#[component(inline_props)]`: `Swatch` per color
//! tile, `DragBox` for the draggable element, plus the small `Toolbar` /
//! `Stage` containers that compose them. The top-level event wiring stays in
//! `App` since it owns the shared signals.

use crossterm::event::{KeyCode, KeyEventKind, MouseEventKind};
use iodilos::prelude::*;

const PALETTE: [(Color, &str); 3] = [
    (Color::Red, "Red"),
    (Color::Green, "Green"),
    (Color::Blue, "Blue"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragState {
    Idle,
    Dragging {
        start: (i32, i32),
        origin: (i32, i32),
    },
}

/// One color tile. Clicking selects it; hovering highlights its border.
#[component(inline_props)]
fn Swatch(
    index: u32,
    color: Color,
    label: &'static str,
    selected: Signal<u32>,
    hovered: Signal<Option<u32>>,
) -> View {
    let border_color = move || {
        if selected.get() == index || hovered.get() == Some(index) {
            Color::White
        } else {
            Color::DarkGrey
        }
    };

    view! {
        div(
            width = 12,
            height = 3,
            margin_left = 1,
            margin_right = 1,
            justify_content = JustifyContent::CENTER,
            align_items = AlignItems::CENTER,
            flex_direction = FlexDirection::Column,
            background_color = color,
            border_style = BorderStyle::Round,
            border_color = border_color,
            on:click = move |_| selected.set(index),
            on:mouseover = move |_| hovered.set(Some(index)),
            on:mouseout = move |_| if hovered.get() == Some(index) {
                hovered.set(None);
            },
        ) {
            p(color = Color::Black, weight = Weight::Bold) { (label) }
        }
    }
}

/// The draggable box. Reacts to `mousedown` / `drag` / `mouseup` and writes
/// its position into the shared signal.
#[component(inline_props)]
fn DragBox(color: ReadSignal<Color>, pos: Signal<(i32, i32)>, drag: Signal<DragState>) -> View {
    view! {
        div(
            width = 10,
            height = 3,
            position = Position::Absolute,
            left = move || Inset::Length(pos.get().0),
            top = move || Inset::Length(pos.get().1),
            justify_content = JustifyContent::CENTER,
            align_items = AlignItems::CENTER,
            background_color = move || color.get(),
            border_style = BorderStyle::Double,
            border_color = Color::White,
            on:mousedown = move |event: Event| {
                let Some(mouse) = event.mouse() else { return; };
                drag.set(DragState::Dragging {
                    start: (mouse.column as i32, mouse.row as i32),
                    origin: pos.get(),
                });
            },
            on:drag = move |event: Event| {
                let Some(mouse) = event.mouse() else { return; };
                if let DragState::Dragging { start, origin } = drag.get() {
                    pos.set((
                        origin.0 + mouse.column as i32 - start.0,
                        origin.1 + mouse.row as i32 - start.1,
                    ));
                }
            },
            on:mouseup = move |_| drag.set(DragState::Idle),
        ) {
            p(color = Color::Black, weight = Weight::Bold) { "DRAG" }
        }
    }
}

/// The header bar: instructions + the row of swatches.
#[component(inline_props)]
fn Toolbar(selected: Signal<u32>, hovered: Signal<Option<u32>>) -> View {
    view! {
        div(
            flex_direction = FlexDirection::Column,
            align_items = AlignItems::CENTER,
            padding_top = 1,
            padding_bottom = 1,
            gap = 1,
        ) {
            p(color = Color::Grey) { "Click swatches or press 1/2/3. Drag the box. Q to quit." }
            div(flex_direction = FlexDirection::Row) {
                Swatch(index = 0, color = PALETTE[0].0, label = PALETTE[0].1, selected = selected, hovered = hovered)
                Swatch(index = 1, color = PALETTE[1].0, label = PALETTE[1].1, selected = selected, hovered = hovered)
                Swatch(index = 2, color = PALETTE[2].0, label = PALETTE[2].1, selected = selected, hovered = hovered)
            }
        }
    }
}

/// The bordered drag stage that owns the absolute-positioned `DragBox`.
#[component(inline_props)]
fn Stage(color: ReadSignal<Color>, pos: Signal<(i32, i32)>, drag: Signal<DragState>) -> View {
    view! {
        div(
            flex_grow = 1.0_f32,
            position = Position::Relative,
            overflow = Overflow::Hidden,
            border_style = BorderStyle::Single,
            border_color = Color::DarkGrey,
            border_edges = Edges::TOP,
        ) {
            DragBox(color = color, pos = pos, drag = drag)
        }
    }
}

#[component]
fn App() -> View {
    let selected = create_signal(0u32);
    let hovered = create_signal(None::<u32>);
    let pos = create_signal((4i32, 2i32));
    let drag = create_signal(DragState::Idle);
    let color = create_memo(move || PALETTE[selected.get() as usize].0);

    view! {
        div(
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            background_color = Color::Black,
            flex_direction = FlexDirection::Column,
            tabindex = "0",
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release { return; }
                if let KeyCode::Char(c @ '1'..='3') = key.code {
                    selected.set((c as u8 - b'1') as u32);
                }
            },
            on:raw_mouse = move |event: Event| {
                let Some(mouse) = event.mouse() else { return; };
                if matches!(mouse.kind, MouseEventKind::Up(_)) {
                    drag.set(DragState::Idle);
                }
            },
        ) {
            Toolbar(selected = selected, hovered = hovered)
            Stage(color = color, pos = pos, drag = drag)
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
