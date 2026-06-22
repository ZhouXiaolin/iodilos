use std::rc::Rc;

use crate::prelude::*;
use crate::{CellContext, CellFactory, TableRow, TableSection, TableViewProps, table_view};

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
    pub selected: ReadSignal<usize>,
    pub max_visible: usize,
    pub border_color: Color,
}

pub fn completion_menu(props: CompletionMenuProps) -> View {
    let items = props.items;
    let items_for_visibility = items;
    let sections = create_memo(move || {
        vec![TableSection {
            title: None,
            rows: items
                .get_clone()
                .into_iter()
                .map(|item| {
                    TableRow::new(item.label.clone(), item.label).with_description(item.description)
                })
                .collect(),
        }]
    });

    let cell_factory: CellFactory = Rc::new(|ctx: &CellContext| {
        let marker = if ctx.selected { "▶ " } else { "  " };
        let fg = if ctx.selected {
            Color::Black
        } else {
            Color::Grey
        };
        let description = ctx.description.clone().unwrap_or_default();
        let row = tags::div()
            .flex_direction(FlexDirection::Row)
            .width(Size::Percent(100.0))
            .column_gap(2);
        let row = if ctx.selected {
            row.background_color(Color::Yellow)
        } else {
            row
        };

        View::from(
            row.children((
                tags::p()
                    .color(fg)
                    .children(format!("{marker}{}", ctx.label)),
                tags::p().color(Color::DarkGrey).children(description),
            )),
        )
    });

    View::from_dynamic(move || {
        if items_for_visibility.get_clone().is_empty() {
            return View::new();
        }
        View::from(
            tags::div()
                .border_style(BorderStyle::Round)
                .border_color(props.border_color)
                .padding_left(1)
                .padding_right(1)
                .children(table_view(TableViewProps {
                    sections,
                    selected: props.selected,
                    max_visible: props.max_visible,
                    cell_factory: Rc::clone(&cell_factory),
                })),
        )
    })
}
