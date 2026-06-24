//! `Tabled` — a sectioned, windowed, selection-aware keyed list.
//!
//! `Tabled` is the component to reach for when rendering a scrollable list of
//! rows grouped into optional titled sections, where one row may be "selected"
//! and the viewport should follow the selection. It replaces the former
//! `ScrollView` + `TableView` pair, which routed list changes through sycamore's
//! coarse-grained `View::from_dynamic` rebuild path (the non-String slow path:
//! destroy + re-insert the whole subtree on every change).
//!
//! How `Tabled` stays fine-grained instead:
//! - The flattened row list is diffed by key via [`map_keyed`], so a row that
//!   survives a list change keeps its per-item reactive scope and mapped view
//!   (reorders/inserts/deletes reuse the existing per-key work).
//! - `is_selected` is a per-row memo built *inside* the keyed map closure: when
//!   the selection moves, only the two rows whose `is_selected` flipped re-patch
//!   their attributes — no `ViewNode` is rebuilt.
//! - Windowing is a private [`Signal<usize>`] (`window_start`) updated by an
//!   effect that reads `selected` and the flattened rows; the visible slice is a
//!   derived memo fed into the keyed engine.
//!
//! See `docs/tabled-design.md` for the full design rationale.

use std::hash::Hash;
use std::rc::Rc;

use crate::prelude::*;

/// A titled group of items within a [`Tabled`].
///
/// `title: None` suppresses the header row entirely (it is neither rendered nor
/// counted towards `max_visible`). `title: Some(_)` emits exactly one header
/// row before the section's items, and that header *does* occupy a slot in the
/// `max_visible` budget — matching the mental model "max_visible = on-screen row
/// cap, headers included".
#[derive(Clone, Debug)]
pub struct TableSection<T> {
    pub title: Option<String>,
    pub items: Vec<T>,
}

impl<T> TableSection<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self { title: None, items }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// Internal key wrapping that makes headers and bodies disjoint keys inside the
/// `map_keyed` engine. Opaque to users: they only ever handle their own `K`.
///
/// `Header(usize)` carries the section index, so two sections with the same
/// title still key distinctly. `Body(K)` carries the user's key verbatim. A
/// user's `K` can never collide with `Header` because the two live in different
/// variants — which is also why "selected points at a header" is statically
/// impossible (the user can only ever produce a `K`, never a `RowKey`).
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) enum RowKey<K> {
    Header(usize),
    Body(K),
}

/// One flattened row handed to the user's `view` closure. Either a section
/// header or a body item. Body rows carry a per-row `is_selected` memo so the
/// caller can bind it directly to an attribute (`background_color = move ||
/// is_selected.get()`), getting attribute-level reactivity when the selection
/// moves.
///
/// Note that `Header` carries no `key`: headers are never selectable and have
/// no meaningful identity to the caller, so there is nothing to expose. This
/// keeps the user's `K` free of any `Default` requirement.
pub enum FlatRow<T, K> {
    Header {
        title: String,
    },
    Body {
        key: K,
        item: T,
        is_selected: ReadSignal<bool>,
    },
}

/// A descriptor for one flattened row, internal to the keyed map closure. This
/// is what `map_keyed` diffs by `RowKey<K>` and hands to the per-row scope.
/// `title` is `Some` iff this is a header; `item` is `Some` iff this is a body.
///
/// `PartialEq` is required by `map_keyed`'s `T: PartialEq` bound (it diffs the
/// list element by value to skip unchanged prefixes/suffixes). Two `RowDesc`s
/// are equal iff every field is equal — in particular a body's `item` must
/// compare equal for the row to be considered unchanged.
#[derive(Clone, Debug, PartialEq)]
struct RowDesc<T, K> {
    row_key: RowKey<K>,
    title: Option<String>,
    item: Option<T>,
}

/// Props for [`Tabled`].
#[derive(Props)]
pub struct TabledProps<T, K, V, KeyFn, ViewFn>
where
    T: Clone + 'static,
    K: Hash + Eq + Clone + 'static,
{
    /// The sectioned list to render.
    pub sections: ReadSignal<Vec<TableSection<T>>>,
    /// The currently selected row, identified by its user key. `None` (or a key
    /// absent from the list) leaves the viewport sticky — see the windowing
    /// table in `docs/tabled-design.md` §5.
    pub selected: ReadSignal<Option<K>>,
    /// Maximum number of rows (headers included) kept on screen at once.
    pub max_visible: usize,
    /// Stable identity for each item. Must be unique across all sections.
    pub key: KeyFn,
    /// Render closure: maps each [`FlatRow`] to a view.
    pub view: ViewFn,
    #[prop(default)]
    pub _phantom: std::marker::PhantomData<V>,
}

/// A sectioned, windowed, selection-aware keyed list.
///
/// # Example
/// ```ignore
/// # use iodilos::prelude::*;
/// # use iodilos::{TableSection, FlatRow};
/// # #[derive(Clone)] struct Message { id: String, text: String }
/// # fn app(sections: iodilos::reactive::ReadSignal<Vec<TableSection<Message>>>,
/// #        selected: iodilos::reactive::ReadSignal<Option<String>>) -> View {
/// view! {
///     Tabled(
///         sections,
///         selected = selected,
///         max_visible = 10,
///         key = |item: &Message| item.id.clone(),
///         view = |row| match row {
///             FlatRow::Header { title, .. } =>
///                 view! { p(color = Color::DarkGrey) { (format!(" {title} ")) } },
///             FlatRow::Body { item, is_selected, .. } => view! {
///                 div(background_color = move || if is_selected.get() {
///                     Color::Yellow
///                 } else {
///                     Color::Reset
///                 }) { (item.text.clone()) }
///             },
///         },
///     )
/// }
/// # }
/// ```
///
/// # Selection & windowing
///
/// `selected` is keyed by `K`, not by flat index, so a highlight stays glued to
/// its item across reorders/inserts. The viewport follows the selection with a
/// one-time center on first appearance, then goes sticky: it only scrolls when
/// the selection leaves the visible window (snapping to top or bottom). See
/// `docs/tabled-design.md` §5 for the full boundary table.
#[allow(clippy::type_complexity)]
#[component]
pub fn Tabled<T, K, V, KeyFn, ViewFn>(props: TabledProps<T, K, V, KeyFn, ViewFn>) -> View
where
    T: PartialEq + Clone + 'static,
    K: Hash + Eq + Clone + 'static,
    V: Into<View> + Clone + 'static,
    KeyFn: Fn(&T) -> K + 'static,
    ViewFn: Fn(FlatRow<T, K>) -> V + 'static,
{
    let TabledProps {
        sections,
        selected,
        max_visible,
        key,
        view,
        ..
    } = props;

    // The user's `view` and `key` closures are captured into `Rc` so the
    // per-row map closure (built once, invoked per key by `map_keyed`) can
    // cheaply clone them — mirroring how `sycamore_web::Keyed` handles it.
    let view_fn = Rc::new(view);
    let key_fn: Rc<KeyFn> = Rc::new(key);
    let selected: ReadSignal<Option<K>> = selected;

    // flat_rows: flatten sections into a Vec<RowDesc<T, K>>, tagging each header
    // with RowKey::Header(section_index) and each item with RowKey::Body(k).
    // `title: None` sections contribute no header row at all.
    let flat_rows = create_memo(move || {
        let mut out = Vec::new();
        for (s_idx, section) in sections.get_clone().into_iter().enumerate() {
            if section.title.is_some() {
                out.push(RowDesc {
                    row_key: RowKey::Header(s_idx),
                    title: section.title,
                    item: None,
                });
            }
            for item in section.items {
                let k = (key_fn)(&item);
                out.push(RowDesc {
                    row_key: RowKey::Body(k),
                    title: None,
                    item: Some(item),
                });
            }
        }
        out
    });

    // window_start: private sticky viewport. The effect below mutates it to keep
    // `selected` visible (with a one-time center on first appearance).
    let window_start = create_signal(0usize);
    let window_initialized = create_signal(false);

    // Sticky-window effect. Runs whenever `selected` or `flat_rows` change.
    // Reading both inside makes it re-evaluate on either dependency change.
    create_effect({
        let selected = selected;
        let window_start = window_start;
        let window_initialized = window_initialized;
        move || {
            let rows = flat_rows.get_clone();
            let Some(k) = selected.get_clone() else {
                return; // None: leave the window alone.
            };
            // Find the selected key among body rows. Headers can't match (type).
            let anchor_idx = rows.iter().position(|r| match &r.row_key {
                RowKey::Body(rk) => rk == &k,
                RowKey::Header(_) => false,
            });
            let Some(anchor_idx) = anchor_idx else {
                return; // key absent: silently keep the window (§5).
            };

            let max_visible = max_visible.max(1);
            let start = window_start.get_untracked();
            let end = start + max_visible;

            if !window_initialized.get_untracked() {
                // First appearance: center once, clamped to list bounds.
                let half = max_visible / 2;
                let new_start = anchor_idx
                    .saturating_sub(half)
                    .min(rows.len().saturating_sub(max_visible));
                window_start.set(new_start);
                window_initialized.set(true);
            } else if anchor_idx < start {
                // Above the window: snap so anchor is the top row.
                window_start.set(anchor_idx);
            } else if anchor_idx >= end {
                // Below the window: snap so anchor is the bottom row.
                window_start.set(anchor_idx + 1 - max_visible);
            }
            // else: anchor already visible — sticky, do nothing.
        }
    });

    // windowed: the visible slice of flat rows. Clamped so a shrinking list
    // never hands out an out-of-range start.
    let windowed = {
        let window_start = window_start;
        create_memo(move || {
            let rows = flat_rows.get_clone();
            if rows.is_empty() {
                return Vec::new();
            }
            let max_visible = max_visible.max(1);
            let total = rows.len();
            let start = window_start.get().min(total.saturating_sub(max_visible));
            let end = (start + max_visible).min(total);
            rows[start..end].to_vec()
        })
    };

    // The keyed engine, wired up *once* at component setup (not inside
    // `from_dynamic`, which would re-create per-row scopes every frame). It
    // diffs the windowed slice by RowKey<K>, building per-row scopes that own
    // the `is_selected` memo. Rows that scroll out are disposed; rows that
    // remain keep their scope and mapped view.
    let mapped: ReadSignal<Vec<View>> = map_keyed(
        windowed,
        {
            let view_fn = Rc::clone(&view_fn);
            let selected = selected;
            move |desc: RowDesc<T, K>| {
                let flat_row = match (desc.row_key.clone(), desc.title, desc.item) {
                    (RowKey::Header(_), Some(title), None) => FlatRow::Header { title },
                    (RowKey::Body(k), None, Some(item)) => {
                        let row_key = k.clone();
                        // Per-row is_selected memo, owned by this row's scope.
                        // When `selected` moves, only the rows whose result
                        // flips re-run dependents (i.e. the bg attribute effect)
                        // — the rest are untouched, no ViewNode rebuild.
                        let is_selected = create_memo(move || {
                            selected.with(|s| s.as_ref() == Some(&row_key))
                        });
                        FlatRow::Body {
                            key: k,
                            item,
                            is_selected,
                        }
                    }
                    // RowDesc is built so title is Some iff Header, item is
                    // Some iff Body. Any other combo is a logic bug.
                    _ => unreachable!("RowDesc title/item invariant violated"),
                };
                (*view_fn)(flat_row).into()
            }
        },
        |desc: &RowDesc<T, K>| desc.row_key.clone(),
    );

    // Empty list → render nothing at all (no wrapper div). Otherwise flatten the
    // cached per-row views. `from_dynamic` is used the same way `Keyed` uses it:
    // each entry is a cheap clone of a cached view (sharing producer Rc's), and
    // the real per-row reuse happened inside `map_keyed` above.
    View::from_dynamic(move || {
        let views = mapped.get_clone();
        if views.is_empty() {
            View::new()
        } else {
            View::from(views)
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::framebuffer::Rect;
    use crate::layout::render as render_buffer;
    use crate::node::TuiNode;
    use crate::reactive::create_root;
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    fn paint(view: &View, w: u16, h: u16) -> String {
        let nodes: Vec<TuiNode> = view.nodes.iter().cloned().collect();
        let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, w, h), None);
        fb.to_string()
    }

    #[derive(Clone, Debug, PartialEq, Eq, Default)]
    struct Item {
        id: &'static str,
        text: &'static str,
    }

    fn single_section(items: Vec<Item>) -> Vec<TableSection<Item>> {
        vec![TableSection::new(items)]
    }

    /// Basic render: every item shows up.
    #[test]
    fn renders_all_items_when_within_budget() {
        let _ = create_root(|| {
            let sections = create_signal(single_section(vec![
                Item { id: "a", text: "alpha" },
                Item { id: "b", text: "beta" },
            ]));
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 10,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };
            let painted = paint(&view, 20, 6);
            assert!(painted.contains("alpha") && painted.contains("beta"));
        });
    }

    /// Empty list renders nothing — not even a wrapper.
    #[test]
    fn empty_list_renders_nothing() {
        let _ = create_root(|| {
            let sections = create_signal(Vec::<TableSection<Item>>::new());
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 5,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };
            let painted = paint(&view, 20, 6);
            assert!(
                painted.trim().is_empty(),
                "expected blank frame, got: {painted:?}"
            );
        });
    }

    /// selected = None: viewport defaults to the top (sticky, no movement).
    #[test]
    fn selected_none_leaves_window_sticky() {
        let _ = create_root(|| {
            let items: Vec<Item> = (0..20)
                .map(|i| Item {
                    id: Box::leak(format!("r{i}").into_boxed_str()),
                    text: Box::leak(format!("row{i}").into_boxed_str()),
                })
                .collect();
            let sections = create_signal(single_section(items));
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 5,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };
            // No selection: window stays at 0 → rows 0..5 visible.
            let painted = paint(&view, 20, 8);
            assert!(painted.contains("row0"), "row0 visible at start: {painted}");
            assert!(painted.contains("row4"));
            assert!(!painted.contains("row5"));
        });
    }

    /// First appearance of a selected key centers the window once.
    #[test]
    fn first_selection_centers_window() {
        let _ = create_root(|| {
            let items: Vec<Item> = (0..20)
                .map(|i| Item {
                    id: Box::leak(format!("r{i}").into_boxed_str()),
                    text: Box::leak(format!("row{i}").into_boxed_str()),
                })
                .collect();
            let sections = create_signal(single_section(items));
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 5,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };
            // Pick item at index 10. max_visible=5 → half=2 → centered start=8.
            selected.set(Some("r10"));
            let painted = paint(&view, 20, 8);
            assert!(painted.contains("row8"), "centered window starts at 8: {painted}");
            assert!(painted.contains("row10"));
            assert!(painted.contains("row12"));
            assert!(!painted.contains("row7"));
            assert!(!painted.contains("row13"));
        });
    }

    /// After init, moving selection within the window does not scroll (sticky).
    /// Moving above the window snaps to top; below snaps to bottom.
    #[test]
    fn sticky_then_snap_above_and_below() {
        let _ = create_root(|| {
            let items: Vec<Item> = (0..20)
                .map(|i| Item {
                    id: Box::leak(format!("r{i}").into_boxed_str()),
                    text: Box::leak(format!("row{i}").into_boxed_str()),
                })
                .collect();
            let sections = create_signal(single_section(items));
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 5,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };

            // Center on index 10 first (start=8, window 8..13).
            selected.set(Some("r10"));
            let _ = paint(&view, 20, 8);

            // Move within window — sticky, no scroll.
            selected.set(Some("r9"));
            let painted = paint(&view, 20, 8);
            assert!(painted.contains("row8") && painted.contains("row12"));

            // Move below window (index 15). Snap so 15 is the bottom → 11..16.
            selected.set(Some("r15"));
            let painted = paint(&view, 20, 8);
            assert!(painted.contains("row15"), "snapped below: {painted}");
            assert!(painted.contains("row11"));
            assert!(!painted.contains("row10"));

            // Move above window (index 3). Snap so 3 is the top → 3..8.
            selected.set(Some("r3"));
            let painted = paint(&view, 20, 8);
            assert!(painted.contains("row3"), "snapped above: {painted}");
            assert!(painted.contains("row7"));
            assert!(!painted.contains("row2"));
        });
    }

    /// Headers occupy max_visible budget and render their title.
    #[test]
    fn header_occupies_budget_and_renders() {
        let _ = create_root(|| {
            let sections = create_signal(vec![TableSection::new(vec![
                Item { id: "a", text: "alpha" },
                Item { id: "b", text: "beta" },
            ])
            .with_title("My Section")]);
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 10,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Header { title, .. } => view! { p(color = Color::DarkGrey) { (title.clone()) } },
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                    },
                )
            };
            let painted = paint(&view, 20, 6);
            assert!(painted.contains("My Section"), "header rendered: {painted}");
            assert!(painted.contains("alpha") && painted.contains("beta"));
        });
    }

    /// `title: None` sections emit no header row and don't occupy budget.
    #[test]
    fn title_none_section_omits_header() {
        let _ = create_root(|| {
            let sections = create_signal(vec![TableSection::new(vec![
                Item { id: "a", text: "alpha" },
            ])]);
            let selected = create_signal(None);
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 10,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { title, .. } => view! { p { (format!("HEADER:{title}")) } },
                    },
                )
            };
            let painted = paint(&view, 20, 6);
            assert!(painted.contains("alpha"));
            assert!(!painted.contains("HEADER:"), "no header for title:None section");
        });
    }

    /// Keyed reuse: appending a row does not re-map existing rows' view closures.
    /// We count invocations of the view closure via a shared counter.
    #[test]
    fn append_reuses_existing_mapped_views() {
        let _ = create_root(|| {
            let sections = create_signal(single_section(vec![
                Item { id: "a", text: "alpha" },
                Item { id: "b", text: "beta" },
            ]));
            let selected = create_signal(None);
            let counter = Rc::new(Cell::new(0));

            let view: View = {
                let counter = Rc::clone(&counter);
                view! {
                    Tabled(
                        sections = *sections,
                        selected = *selected,
                        max_visible = 10,
                        key = |item: &Item| item.id,
                        view = move |row: FlatRow<Item, &'static str>| {
                            counter.set(counter.get() + 1);
                            match row {
                                FlatRow::Body { item, .. } => view! { p { (item.text) } },
                                FlatRow::Header { .. } => view! { p { "H" } },
                            }
                        },
                    )
                }
            };
            let _ = paint(&view, 20, 6);
            let after_initial = counter.get();
            assert_eq!(after_initial, 2, "two body rows mapped initially");

            // Append a third row: only the new one should re-run the view.
            sections.update(|s| {
                s[0].items.push(Item { id: "c", text: "gamma" });
            });
            let _ = paint(&view, 20, 6);
            assert_eq!(
                counter.get(),
                3,
                "only the appended row re-mapped; existing rows reused"
            );
        });
    }

    /// is_selected flips reactively when selection moves (render smoke test: the
    /// body views keep evaluating their background binding across moves without
    /// rebuilding nodes — the append counter above guards the reuse invariant).
    #[test]
    fn selection_moves_attribute_level() {
        let _ = create_root(|| {
            let sections = create_signal(single_section(vec![
                Item { id: "a", text: "alpha" },
                Item { id: "b", text: "beta" },
                Item { id: "c", text: "gamma" },
            ]));
            let selected = create_signal(Some("a"));
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 10,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, is_selected, .. } => view! {
                            div(background_color = move || if is_selected.get() {
                                Color::Yellow
                            } else {
                                Color::Reset
                            }) {
                                p { (item.text) }
                            }
                        },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };
            let painted_a = paint(&view, 20, 6);
            assert!(painted_a.contains("alpha"));

            selected.set(Some("c"));
            let painted_c = paint(&view, 20, 6);
            assert!(painted_c.contains("gamma"));
        });
    }

    /// selected points at a key not in the list → window unchanged (silent).
    #[test]
    fn selected_absent_key_is_silent() {
        let _ = create_root(|| {
            let items: Vec<Item> = (0..10)
                .map(|i| Item {
                    id: Box::leak(format!("r{i}").into_boxed_str()),
                    text: Box::leak(format!("row{i}").into_boxed_str()),
                })
                .collect();
            let sections = create_signal(single_section(items));
            let selected = create_signal(Some("r5"));
            let view: View = view! {
                Tabled(
                    sections = *sections,
                    selected = *selected,
                    max_visible = 5,
                    key = |item: &Item| item.id,
                    view = |row: FlatRow<Item, &'static str>| match row {
                        FlatRow::Body { item, .. } => view! { p { (item.text) } },
                        FlatRow::Header { .. } => view! { p { "H" } },
                    },
                )
            };
            // First center on r5 (half=2 → start=3, window 3..8).
            let _ = paint(&view, 20, 8);

            // Point at a nonexistent key: window must stay at 3..8.
            selected.set(Some("nope"));
            let painted = paint(&view, 20, 8);
            assert!(painted.contains("row3") && painted.contains("row7"));
            assert!(!painted.contains("row2") && !painted.contains("row8"));
        });
    }
}
