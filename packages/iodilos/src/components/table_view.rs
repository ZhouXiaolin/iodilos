use std::rc::Rc;

use crate::prelude::*;
use crate::{ScrollContent, ScrollViewProps, scroll_view};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
}

impl TableRow {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSection {
    pub title: Option<String>,
    pub rows: Vec<TableRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellContext {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub selected: bool,
    pub flat_index: usize,
}

pub type CellFactory = Rc<dyn Fn(&CellContext) -> View>;

#[derive(Clone)]
pub struct TableViewProps {
    pub sections: ReadSignal<Vec<TableSection>>,
    pub selected: ReadSignal<usize>,
    pub max_visible: usize,
    pub cell_factory: CellFactory,
}

pub fn table_view(props: TableViewProps) -> View {
    let sections_for_count = props.sections;
    let total = create_memo(move || row_count(&sections_for_count.get_clone()));
    let sections = props.sections;
    let selected = props.selected;
    let cell_factory = props.cell_factory.clone();
    let content: ScrollContent = Rc::new(move |window| {
        render_table_window(
            &sections.get_clone(),
            window.range.clone(),
            selected.get(),
            &cell_factory,
        )
    });

    scroll_view(ScrollViewProps {
        total,
        anchor: props.selected,
        max_visible: props.max_visible,
        content,
    })
}

fn row_count(sections: &[TableSection]) -> usize {
    sections.iter().map(|section| section.rows.len()).sum()
}

fn render_table_window(
    sections: &[TableSection],
    range: std::ops::Range<usize>,
    selected: usize,
    cell_factory: &CellFactory,
) -> View {
    let mut children = Vec::new();
    let mut flat_index = 0usize;

    for section in sections.iter().cloned() {
        let visible_in_section = section.rows.iter().enumerate().any(|(idx, _)| {
            let i = flat_index + idx;
            range.contains(&i)
        });
        if visible_in_section && let Some(title) = section.title {
            children.push(
                tags::p()
                    .color(Color::DarkGrey)
                    .children(format!(" {title} "))
                    .into(),
            );
        }
        for row in section.rows {
            let current = flat_index;
            if range.contains(&current) {
                let ctx = CellContext {
                    key: row.key,
                    label: row.label,
                    description: row.description,
                    selected: current == selected,
                    flat_index: current,
                };
                children.push(cell_factory(&ctx));
            }
            flat_index += 1;
        }
    }

    View::from_nodes(children.into_iter().flat_map(|view| view.nodes).collect())
}
