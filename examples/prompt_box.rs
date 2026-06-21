//! Statusline + framed multiline prompt box demo.
//!
//! Renders a rounded box: the statusline sits on the top border, the framed
//! multiline input below it, and a self-drawn block cursor marks the caret.
//!
//! Keys:
//!   - printable char   -> insert at cursor
//!   - Backspace        -> delete before cursor
//!   - Left / Right     -> move cursor
//!   - Shift+Enter      -> newline (needs kitty keyboard protocol)
//!   - Alt+Enter        -> newline (universal fallback)
//!   - Enter            -> submit & clear
//!   - Ctrl+C           -> quit
//!
//! Real Shift+Enter requires a kitty-keyboard-protocol terminal (kitty,
//! WezTerm, Ghostty, foot, Alacritty ≥0.15, …); on others use Alt+Enter.

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::size as term_size;
use iodilos::node::TuiNode;
use iodilos::prelude::*;
use iodilos_prompt::{PromptModel, PromptTheme, StatusLine, render_prompt_to_surface};

fn app() -> View {
    let model = Rc::new(RefCell::new(PromptModel::new()));
    // Revision counter: bumped on every edit so the surface memo re-renders.
    let rev = create_signal(0u32);
    let statusline = StatusLine::default_mock();
    let theme = PromptTheme::default();

    let (init_cols, _init_rows) = term_size().unwrap_or((80, 24));
    let term_cols = create_signal(init_cols as usize);

    let surface = create_memo({
        let model = Rc::clone(&model);
        let statusline = statusline.clone();
        move || {
            rev.get(); // depend on edits
            let m = model.borrow();
            render_prompt_to_surface(m.buffer(), m.cursor_char(), &statusline, term_cols.get(), &theme)
        }
    });

    view! {
        div(
            flex_direction = FlexDirection::Column,
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            on:terminal_resize = move |event: Event| {
                if let Some((cols, _rows)) = event.resize() {
                    term_cols.set(cols as usize);
                }
            },
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release {
                    return;
                }
                {
                    let mut m = model.borrow_mut();
                    match key.code {
                        KeyCode::Enter => {
                            if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) {
                                m.newline();
                            } else {
                                m.submit();
                            }
                        }
                        KeyCode::Backspace => m.backspace(),
                        KeyCode::Left => m.move_left(),
                        KeyCode::Right => m.move_right(),
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            m.insert_char(c);
                        }
                        _ => {}
                    }
                } // mutable borrow released before rev.set
                rev.set(rev.get() + 1);
            },
        ) {
            div(flex_grow = 1.0_f32) {}
            (move || {
                rev.get(); // depend on edits
                View::from_node(TuiNode::create_text_surface_node(surface.get_clone(), 0))
            })
        }
    }
}

fn main() -> std::io::Result<()> {
    iodilos::render_with(
        app,
        RenderConfig {
            keyboard_enhancement: true,
            quit: QuitPolicy::CtrlCOnly,
        },
    )
}
