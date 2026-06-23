//! The `PromptBox` reactive component — a statusline + framed multiline
//! prompt built entirely from framework primitives.
//!
//! The rounded frame is a `div(border_style = BorderStyle::Round)`; the
//! statusline rides on the top border via `border_title`; the editable input is
//! a [`iodilos::producer::Spans`] leaf that re-wraps at the layout width. The
//! component owns its [`PromptModel`] and keyboard handling, so an app just
//! drops `PromptBox(...)` in a `view!` and gets submitted text through
//! `on_submit`.

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use iodilos::prelude::*;
use iodilos::producer::Spans;
use iodilos::style::BorderStyle;

use crate::model::PromptModel;
use crate::render::{input_runs, statusline_runs};
use crate::statusline::StatusLine;
use crate::theme::PromptTheme;

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
    let title = statusline_runs(&statusline, &theme);
    let frame_color = theme.frame;
    let key_model = Rc::clone(&model);
    let view_model = Rc::clone(&model);
    let key_submit = on_submit.clone();

    view! {
        div(
            width = Size::Percent(100.0),
            border_style = BorderStyle::Round,
            border_color = frame_color,
            border_title = title,
            padding_left = 1,
            padding_right = 1,
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
                rev.set(rev.get() + 1);
            },
        ) {
            (move || {
                rev.get();
                let m = view_model.borrow();
                View::leaf(Box::new(Spans::new(input_runs(
                    m.buffer(),
                    m.cursor_char(),
                    &theme,
                ))))
            })
        }
    }
}
