//! Iteration components `Indexed` and `Keyed`.
//!
//! Ported from `sycamore-web`'s `iter`, drastically simplified for the TUI
//! backend. sycamore-web spends ~300 lines on live-DOM fragment reconciliation
//! (`reconcile_fragments`, `get_nodes_between`, document-fragment juggling);
//! iodilos has no retained DOM — layout/paint is a full rebuild each frame — so
//! none of that applies. These components ride directly on the `map_indexed`/
//! `map_keyed` engines (which reuse per-item computations and manage per-item
//! reactive scopes + disposers) and flatten the resulting `Vec<View>` into a
//! single dynamic region.
//!
//! What these components buy you in iodilos:
//! - **Per-item map-fn reuse**: on a list change, only items whose value (or
//!   key) actually changed re-run the `view` closure; the rest are cached.
//! - **Per-item reactive scope lifetime**: signals/effects created inside the
//!   `view` closure live as long as the item remains in the list, and are
//!   cleaned up (via `on_cleanup`) when it is removed.
//!
//! They do *not* buy node-level paint diffing — that is the framebuffer row
//! diff's job, applied uniformly to the whole frame regardless of which items
//! changed.

use std::hash::Hash;

use crate::prelude::*;

/// Props for [`Indexed`].
#[derive(Props)]
pub struct IndexedProps<T, U, List, F>
where
    List: Into<MaybeDyn<Vec<T>>> + 'static,
    F: Fn(T) -> U + 'static,
    T: 'static,
{
    /// The list to iterate. A `Vec<T>` or any reactive source of `Vec<T>`.
    pub list: List,
    /// The view closure applied to each item.
    pub view: F,
    #[prop(default)]
    pub _phantom: std::marker::PhantomData<(T, U)>,
}

/// Non-keyed iteration (keyed by index).
///
/// Use this instead of directly rendering an array of [`View`]s: only items
/// whose value at a given index changed are re-mapped. For a stable identity
/// per item (so reorders reuse the existing mapped views), use [`Keyed`].
///
/// # Example
/// ```ignore
/// # use iodilos::prelude::*;
/// # fn app() -> View {
/// let fib = create_signal(vec![0, 1, 1, 2, 3, 5, 8]);
/// view! {
///     div {
///         Indexed(
///             list = fib,
///             view = |x| view! { p { (x) } },
///         )
///     }
/// }
/// # }
/// ```
#[component]
pub fn Indexed<T, U, List, F>(props: IndexedProps<T, U, List, F>) -> View
where
    T: PartialEq + Clone + 'static,
    U: Into<View> + Clone + 'static,
    List: Into<MaybeDyn<Vec<T>>> + 'static,
    F: Fn(T) -> U + 'static,
{
    let IndexedProps { list, view, .. } = props;
    let mapped: ReadSignal<Vec<View>> = map_indexed(list, move |x| view(x).into());
    View::from_dynamic(move || View::from(mapped.get_clone()))
}

/// Props for [`Keyed`].
#[derive(Props)]
pub struct KeyedProps<T, K, U, List, F, Key>
where
    List: Into<MaybeDyn<Vec<T>>> + 'static,
    F: Fn(T) -> U + 'static,
    Key: Fn(&T) -> K + 'static,
    T: 'static,
{
    /// The list to iterate. A `Vec<T>` or any reactive source of `Vec<T>`.
    pub list: List,
    /// The view closure applied to each item.
    pub view: F,
    /// The key function returning a unique, stable identity per item.
    pub key: Key,
    #[prop(default)]
    pub _phantom: std::marker::PhantomData<(T, K, U)>,
}

/// Keyed iteration.
///
/// Prefer this over [`Indexed`] when each item has a stable identity: reorders,
/// insertions, and deletions reuse the existing per-item mapped views (matched
/// by key) instead of re-mapping by index.
///
/// # Example
/// ```ignore
/// # use iodilos::prelude::*;
/// # #[derive(Clone, PartialEq)]
/// # struct Item { id: u32, name: &'static str }
/// # fn app(items: Signal<Vec<Item>>) -> View {
/// view! {
///     div {
///         Keyed(
///             list = items,
///             view = |item| view! { p { (item.name) } },
///             key = |item| item.id,
///         )
///     }
/// }
/// # }
/// ```
#[component]
pub fn Keyed<T, K, U, List, F, Key>(props: KeyedProps<T, K, U, List, F, Key>) -> View
where
    T: PartialEq + Clone + 'static,
    K: Hash + Eq + 'static,
    U: Into<View> + Clone + 'static,
    List: Into<MaybeDyn<Vec<T>>> + 'static,
    F: Fn(T) -> U + 'static,
    Key: Fn(&T) -> K + 'static,
{
    let KeyedProps { list, view, key, .. } = props;
    let mapped: ReadSignal<Vec<View>> = map_keyed(list, move |x| view(x).into(), key);
    View::from_dynamic(move || View::from(mapped.get_clone()))
}

#[cfg(test)]
mod tests {
    use crate::framebuffer::Rect;
    use crate::layout::render as render_buffer;
    use crate::node::TuiNode;
    use crate::prelude::*;
    use crate::reactive::create_root;

    #[test]
    fn indexed_renders_and_updates_list() {
        let _ = create_root(|| {
            let list = create_signal(vec![1, 2, 3]);
            let view: View = view! {
                div {
                    Indexed(list = list, view = |x| view! { p { (x) } })
                }
            };
            let nodes: Vec<TuiNode> = view.nodes.into_iter().collect();

            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 5, 6), None);
            let painted = fb.to_string();
            for n in ['1', '2', '3'] {
                assert!(painted.contains(n), "item {n} rendered: {painted}");
            }

            // Append: a new item is mapped (only index 3 re-runs the view closure).
            list.set(vec![1, 2, 3, 4]);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 5, 6), None);
            assert!(fb.to_string().contains('4'), "appended item renders");

            // Shrink: removed item drops out.
            list.set(vec![9]);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 5, 6), None);
            let painted = fb.to_string();
            assert!(painted.contains('9'), "replacement item renders");
            assert!(!painted.contains('1'), "old item gone after replace");
        });
    }

    #[derive(Clone, PartialEq, Debug)]
    struct Item {
        id: u32,
        name: &'static str,
    }

    #[test]
    fn keyed_renders_and_survives_reorder() {
        let _ = create_root(|| {
            let items = create_signal(vec![
                Item { id: 1, name: "aaa" },
                Item { id: 2, name: "bbb" },
            ]);
            let view: View = view! {
                div {
                    Keyed(
                        list = items,
                        view = |item| view! { p { (item.name) } },
                        key = |item| item.id,
                    )
                }
            };
            let nodes: Vec<TuiNode> = view.nodes.into_iter().collect();

            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 5, 6), None);
            let painted = fb.to_string();
            assert!(painted.contains("aaa") && painted.contains("bbb"));

            // Reorder: keys swap positions; both names still render (and the
            // per-item computations were reused by key, not rebuilt).
            items.set(vec![
                Item { id: 2, name: "bbb" },
                Item { id: 1, name: "aaa" },
            ]);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 5, 6), None);
            let painted = fb.to_string();
            assert!(painted.contains("aaa") && painted.contains("bbb"));
        });
    }
}
