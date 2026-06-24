//! Completion menu — a single-section [`Tabled`] specialised for completion
//! candidates.
//!
//! Selection is keyed by the item's `label` (`ReadSignal<Option<String>>`):
//! when candidates are inserted/removed, the highlight stays glued to its label
//! rather than drifting with a flat index. Internally it routes through
//! [`Tabled`] with a single `title: None` section, so no header is ever drawn —
//! the `FlatRow::Header` branch is statically `unreachable!()`.

use crate::prelude::*;

/// One completion candidate. `label` doubles as the selection key, so labels
/// must be unique within a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub description: String,
}

impl CompletionItem {
    pub fn new(label: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: description.into(),
        }
    }
}

#[derive(Clone)]
pub struct CompletionMenuProps {
    pub items: ReadSignal<Vec<CompletionItem>>,
    /// Currently highlighted candidate by label. `None` = no highlight.
    pub selected: ReadSignal<Option<String>>,
    pub max_visible: usize,
    pub border_color: Color,
}

pub fn completion_menu(props: CompletionMenuProps) -> View {
    let items = props.items;
    let selected = props.selected;

    // A single untitled section: no header row is emitted, so the
    // `FlatRow::Header` arm in the view closure is unreachable. We wrap the
    // items in a memo so `Tabled` sees a `ReadSignal<Vec<TableSection<_>>>`.
    let sections = create_memo(move || vec![TableSection::new(items.get_clone())]);

    // Empty list → the whole popup disappears (border included), matching the
    // §7 contract: a completion menu with nothing to show renders nothing.
    View::from_dynamic(move || {
        if items.get_clone().is_empty() {
            return View::new();
        }
        let max_visible = props.max_visible;
        let border_color = props.border_color;
        view! {
            div(
                border_style = BorderStyle::Round,
                border_color = border_color,
                padding_left = 1,
                padding_right = 1,
            ) {
                Tabled(
                    sections = sections,
                    selected = selected,
                    max_visible = max_visible,
                    key = |item: &CompletionItem| item.label.clone(),
                    view = |row: FlatRow<CompletionItem, String>| match row {
                        FlatRow::Header { .. } => {
                            unreachable!("completion_menu never renders a header")
                        }
                        FlatRow::Body { item, is_selected, .. } => {
                            let label = item.label.clone();
                            let description = item.description.clone();
                            // Selection paints the whole row's bg with the
                            // highlight color and flips the label fg to Black
                            // for contrast against yellow (Grey-on-Yellow is
                            // hard to read). The `▶` marker prefixes the label
                            // as an extra cue. The description's DarkGrey fg
                            // is left alone — it stays a low-emphasis subtitle
                            // on both states, and the bg alone is enough to
                            // associate it with the selected row.
                            view! {
                                div(
                                    flex_direction = FlexDirection::Row,
                                    width = Size::Percent(100.0),
                                    column_gap = 2,
                                    background_color = move || if is_selected.get() {
                                        Color::Yellow
                                    } else {
                                        Color::Reset
                                    },
                                ) {
                                    p(color = move || if is_selected.get() {
                                        Color::Black
                                    } else {
                                        Color::Grey
                                    }) {
                                        (move || if is_selected.get() {
                                            format!("▶ {label}")
                                        } else {
                                            format!("  {label}")
                                        })
                                    }
                                    p(color = Color::DarkGrey) { (description) }
                                }
                            }
                        }
                    },
                )
            }
        }
    })
}
