//! Statusline + framed multiline prompt box demo (component edition).
//!
//! The prompt is the [`PromptBox`] component — it owns its editing model and
//! keyboard handling, renders a rounded frame with the statusline on the top
//! border, and a `Spans` input leaf that re-wraps on resize. This example just
//! docks it at the bottom of the screen and echoes submitted lines above it.
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

use iodilos::prelude::*;
use iodilos_prompt::PromptBox;

/// The scrolling transcript above the prompt: one grey paragraph per submitted
/// line, newest at the bottom.
#[component(inline_props)]
fn Transcript(history: Signal<Vec<String>>) -> View {
    view! {
        div(flex_grow = 1.0_f32, overflow = Overflow::Hidden, padding_left = 1) {
            Keyed(
                list = history,
                view = |line| view! { p(color = Color::Grey) { (line) } },
                key = |line| line.clone(),
            )
        }
    }
}

#[component]
fn App() -> View {
    let history = create_signal(Vec::<String>::new());

    view! {
        div(
            flex_direction = FlexDirection::Column,
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
        ) {
            Transcript(history = history)

            // The prompt, docked at the bottom. It owns its own editing state
            // and keyboard handling; on submit it appends to `history`.
            PromptBox(on_submit = move |text: &str| {
                if !text.is_empty() {
                    history.update(|h| h.push(text.to_string()));
                }
            })
        }
    }
}

fn main() -> std::io::Result<()> {
    iodilos::render_with(
        App,
        RenderConfig {
            keyboard_enhancement: true,
            quit: QuitPolicy::CtrlCOnly,
        },
    )
}
