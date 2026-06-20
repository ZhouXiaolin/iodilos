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

fn swatch(
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
            on:click=move |_| selected.set(index),
            on:mouseover=move |_| hovered.set(Some(index)),
            on:mouseout=move |_| {
                if hovered.get() == Some(index) {
                    hovered.set(None);
                }
            },
        ) {
            p(color = Color::Black, weight = Weight::Bold) { (label) }
        }
    }
}

fn drag_box(color: ReadSignal<Color>, pos: Signal<(i32, i32)>, drag: Signal<DragState>) -> View {
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
            on:mousedown=move |event: Event| {
                let Some(mouse) = event.mouse() else {
                    return;
                };
                drag.set(DragState::Dragging {
                    start: (mouse.column as i32, mouse.row as i32),
                    origin: pos.get(),
                });
            },
            on:drag=move |event: Event| {
                let Some(mouse) = event.mouse() else {
                    return;
                };
                if let DragState::Dragging { start, origin } = drag.get() {
                    pos.set((
                        origin.0 + mouse.column as i32 - start.0,
                        origin.1 + mouse.row as i32 - start.1,
                    ));
                }
            },
            on:mouseup=move |_| drag.set(DragState::Idle),
        ) {
            p(color = Color::Black, weight = Weight::Bold) { "DRAG" }
        }
    }
}

fn app() -> View {
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
            on:raw_key=move |event: Event| {
                let Some(key) = event.key() else {
                    return;
                };
                if key.kind == KeyEventKind::Release {
                    return;
                }
                if let KeyCode::Char(c @ '1'..='3') = key.code {
                    selected.set((c as u8 - b'1') as u32);
                }
            },
            on:raw_mouse=move |event: Event| {
                let Some(mouse) = event.mouse() else {
                    return;
                };
                if matches!(mouse.kind, MouseEventKind::Up(_)) {
                    drag.set(DragState::Idle);
                }
            },
        ) {
            div(
                flex_direction = FlexDirection::Column,
                align_items = AlignItems::CENTER,
                padding_top = 1,
                padding_bottom = 1,
                gap = 1,
            ) {
                p(color = Color::Grey) { "Click swatches or press 1/2/3. Drag the box. Q to quit." }
                div(flex_direction = FlexDirection::Row) {
                    (swatch(0, PALETTE[0].0, PALETTE[0].1, selected, hovered))
                    (swatch(1, PALETTE[1].0, PALETTE[1].1, selected, hovered))
                    (swatch(2, PALETTE[2].0, PALETTE[2].1, selected, hovered))
                }
            }
            div(
                flex_grow = 1.0_f32,
                position = Position::Relative,
                overflow = Overflow::Hidden,
                border_style = BorderStyle::Single,
                border_color = Color::DarkGrey,
                border_edges = Edges::TOP,
            ) {
                (drag_box(color, pos, drag))
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
