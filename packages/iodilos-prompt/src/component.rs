//! The `PromptBox` reactive component — a statusline + framed multiline
//! prompt built from framework primitives.
//!
//! The whole prompt (statusline, rounded frame, editable input) is one
//! borderless [`crate::render::PromptView`] producer leaf. The framework's
//! border model can't place the input text *on* the rounded bottom edge
//! (`Edges::BOTTOM` is exclusive, so the text lands a row above an empty
//! bottom), so `PromptView` draws every cell itself and the input/cursor sit
//! directly on the `╰─ … ─╯` row. The component owns its [`PromptModel`] and
//! keyboard handling, so an app just drops `PromptBox(...)` in a `view!` and
//! gets submitted text through `on_submit`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use futures_timer::Delay;
use iodilos::prelude::*;

use crate::model::PromptModel;
use crate::render::PromptView;
use crate::statusline::StatusLine;
use crate::theme::PromptTheme;

/// Half-period of the cursor blink in milliseconds. Matches the conventional
/// terminal caret blink (~530ms on/off, the DEC VT default).
const BLINK_HALF_PERIOD_MS: u64 = 530;

/// Submit callback for [`PromptBox`]: receives the submitted buffer text.
/// Construct from any `Fn(&str)`; pass it as the `on_submit` prop.
#[derive(Clone)]
pub struct PromptSubmit(pub Rc<dyn Fn(&str)>);

impl<F: Fn(&str) + 'static> From<F> for PromptSubmit {
    fn from(f: F) -> Self {
        Self(Rc::new(f))
    }
}

/// Props for [`PromptBox`].
#[derive(Props)]
pub struct PromptBoxProps {
    /// Statusline rendered on the top border. Defaults to
    /// [`StatusLine::default_mock`].
    #[prop(default_code = "StatusLine::default_mock()")]
    pub statusline: StatusLine,
    /// Colour scheme. Defaults to [`PromptTheme::default`].
    #[prop(default)]
    pub theme: PromptTheme,
    /// Called with the buffer text when the user presses Enter. Optional.
    #[prop(default, setter(into))]
    pub on_submit: Option<PromptSubmit>,
}

/// A statusline + framed multiline prompt box.
///
/// Owns its editing state and keyboard handling. Printable chars insert at the
/// caret, Backspace deletes, Left/Right move, Shift/Alt+Enter inserts a newline,
/// and Enter submits (firing `on_submit`) and clears. The frame, statusline,
/// and text wrapping are all the framework's job — this component only decides
/// which styled runs go into the border title and the input leaf.
///
/// # Example
/// ```ignore
/// # use iodilos::prelude::*;
/// # use iodilos_prompt::PromptBox;
/// # fn app() -> View {
/// view! {
///     PromptBox(on_submit = move |text: &str| { println!("{text}"); })
/// }
/// # }
/// ```
#[component]
pub fn PromptBox(props: PromptBoxProps) -> View {
    let PromptBoxProps {
        statusline,
        theme,
        on_submit,
    } = props;
    let model = Rc::new(RefCell::new(PromptModel::new()));
    let rev = create_signal(0u32);
    // Blink state: `blink` is whether the caret is currently drawn; `blink_tick`
    // is bumped on every keystroke so the timer can re-sync the "on" phase to
    // the last input (caret stays solid while typing, then resumes blinking).
    let blink = create_signal(true);
    let blink_tick = create_signal(0u32);
    let key_model = Rc::clone(&model);
    let view_model = Rc::clone(&model);
    let key_submit = on_submit.clone();
    let sl = statusline;

    // Cursor blink timer. Toggles `blink` every half-period; restarts the "on"
    // phase whenever `blink_tick` changes (i.e. on input), so the caret holds
    // steady while the user types then resumes blinking after they pause.
    use_future({
        let blink = blink;
        let blink_tick = blink_tick;
        async move {
            loop {
                Delay::new(Duration::from_millis(BLINK_HALF_PERIOD_MS)).await;
                let seen_tick = blink_tick.get_untracked();
                // Flip the caret. Reading tick again after the delay lets us
                // detect an input during the wait and force it back on.
                blink.set(!blink.get_untracked());
                if blink_tick.get_untracked() != seen_tick {
                    blink.set(true);
                }
            }
        }
    });

    view! {
        div(
            width = Size::Percent(100.0),
            tabindex = "0",
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release {
                    return;
                }
                let submitted = {
                    let mut m = key_model.borrow_mut();
                    match key.code {
                        KeyCode::Enter => {
                            if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) {
                                m.newline();
                                None
                            } else {
                                Some(m.submit())
                            }
                        }
                        KeyCode::Backspace => {
                            m.backspace();
                            None
                        }
                        KeyCode::Left => {
                            m.move_left();
                            None
                        }
                        KeyCode::Right => {
                            m.move_right();
                            None
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            m.insert_char(c);
                            None
                        }
                        _ => None,
                    }
                };
                if let Some(text) = submitted
                    && let Some(cb) = &key_submit
                {
                    (cb.0)(&text);
                }
                // Any caret-affecting key restarts the blink "on" phase.
                blink.set(true);
                blink_tick.set(blink_tick.get() + 1);
                rev.set(rev.get() + 1);
            },
        ) {
            (move || {
                rev.get();
                let cursor_visible = blink.get();
                let m = view_model.borrow();
                View::leaf(Box::new(PromptView::new(
                    &sl,
                    m.buffer(),
                    m.cursor_char(),
                    cursor_visible,
                    &theme,
                )))
            })
        }
    }
}
