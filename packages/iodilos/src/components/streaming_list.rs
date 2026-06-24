//! `StreamingList` — keyed list of richly-shaped items with element-level scroll.
//!
//! Reach for `StreamingList` when each item is a self-contained "card" (its own
//! border, background, padding, internal layout) and the list as a whole should
//! scroll natively — i.e. the surface you would otherwise build by hand-slicing
//! a flat row buffer and rebuilding a single block on every change.
//!
//! How it works:
//! - Items are diffed by key via [`map_keyed`], so a surviving item keeps its
//!   per-item reactive scope and its mapped `View`. Streaming updates target
//!   the item's own signals (created inside the view closure) and re-render in
//!   place — adjacent items are not touched. Reorders / inserts / deletes reuse
//!   the existing per-key work.
//! - The whole list lives inside a single `div(overflow = Hidden, scroll = ...)`
//!   container, so scrolling is just the element-level `scroll` style property
//!   translating the child subtree at paint time. No row caches, no manual
//!   viewport slicing.
//!
//! Compared to [`Tabled`], `StreamingList`:
//! - has **no built-in selection / windowing** — it is unsegmented and follows
//!   the caller's `scroll` value verbatim;
//! - assumes each item already paints its own decoration (border / bg /
//!   padding), so the container is just a clipped viewport, not a chrome layer.
//!
//! # Example
//! ```ignore
//! # use iodilos::prelude::*;
//! # #[derive(Clone, PartialEq)] struct Entry { id: u64, body: String }
//! # fn app(entries: Signal<Vec<Entry>>, scroll: Signal<i32>) -> View {
//! view! {
//!     StreamingList(
//!         items = entries,
//!         key = |e: &Entry| e.id,
//!         view = |e: &Entry| view! {
//!             div(border_style = BorderStyle::Round, padding = 1) {
//!                 p { (e.body.clone()) }
//!             }
//!         },
//!         scroll = scroll,                       // or i32::MAX to stick to bottom
//!     )
//! }
//! # }
//! ```

use std::hash::Hash;

use crate::prelude::*;

/// Props for [`StreamingList`].
#[derive(Props)]
pub struct StreamingListProps<T, K, U, KeyFn, ViewFn>
where
    T: Clone + PartialEq + 'static,
    K: Hash + Eq + Clone + 'static,
{
    /// The items to render. A reactive `Vec<T>` (signal or memo).
    pub items: ReadSignal<Vec<T>>,
    /// Stable identity per item, used by the keyed engine to reuse per-item
    /// scopes/views across list mutations. Must be unique across the list.
    pub key: KeyFn,
    /// View closure: maps each item to a `View`. Runs **once per key** — when
    /// an item survives a list change its closure is **not** re-invoked, so any
    /// streaming updates must drive their own per-item signals created inside
    /// this closure (in the spirit of [`Tabled`]'s `is_selected` memo).
    pub view: ViewFn,
    /// Element-level scroll offset in rows. `0` shows the top; positive values
    /// hide that many rows from the top of the content; `i32::MAX` sticks to
    /// the bottom (the paint path clamps to `content_height − viewport`).
    /// Accepts a static `i32`, a `Signal`, or a closure.
    #[prop(setter(into))]
    pub scroll: MaybeDyn<i32>,
    #[prop(default)]
    pub _phantom: std::marker::PhantomData<(K, U)>,
}

/// A keyed, scrollable list of richly-shaped items.
///
/// See the [module docs](self) for the design and a worked example.
#[allow(clippy::type_complexity)]
#[component]
pub fn StreamingList<T, K, U, KeyFn, ViewFn>(
    props: StreamingListProps<T, K, U, KeyFn, ViewFn>,
) -> View
where
    T: PartialEq + Clone + 'static,
    K: Hash + Eq + Clone + 'static,
    U: Into<View> + Clone + 'static,
    KeyFn: Fn(&T) -> K + 'static,
    ViewFn: Fn(&T) -> U + 'static,
{
    let StreamingListProps {
        items,
        key,
        view,
        scroll,
        ..
    } = props;

    // `map_keyed` hands the view closure a `T` by value, but the caller's API
    // takes `&T` (it reads more naturally for the rich-card case and matches
    // `Tabled`'s `key = |item: &T|` half of the signature). Bridge with a
    // by-ref call inside the move closure.
    let mapped: ReadSignal<Vec<View>> =
        map_keyed(items, move |t: T| view(&t).into(), key);

    // Build the container with the lowercase `div` builder directly instead of
    // going through `view! { div { (...) } }`. The `view!` macro inspects every
    // child expression with its `is_dyn` heuristic and would wrap our explicit
    // `from_dynamic` in a second `from_dynamic` layer, which leaves the marker
    // region empty (the nested dynamic never re-evaluates to the keyed Views).
    // Calling `.children(View::from_dynamic(...))` mounts the dynamic region
    // exactly once, inline, in the container's child list.
    use crate::components::tags;
    let body = View::from_dynamic(move || View::from(mapped.get_clone()));
    tags::div()
        .overflow(Overflow::Hidden)
        .scroll(move || scroll.get())
        .children(body)
        .into()
}

#[cfg(test)]
mod tests {
    use crate::framebuffer::Rect;
    use crate::layout::render as render_buffer;
    use crate::node::TuiNode;
    use crate::prelude::*;
    use crate::reactive::create_root;

    #[derive(Clone, PartialEq, Debug)]
    struct Entry {
        id: u64,
        body: String,
    }

    /// Basic shape: items render, list mutations diff by key, removed items
    /// drop out.
    #[test]
    fn streaming_list_renders_and_updates() {
        let _ = create_root(|| {
            let entries = create_signal(vec![
                Entry { id: 1, body: "aaa".into() },
                Entry { id: 2, body: "bbb".into() },
            ]);
            let view: View = view! {
                StreamingList(
                    items = *entries,
                    key = |e: &Entry| e.id,
                    view = |e: &Entry| {
                        let body = e.body.clone();
                        view! { p { (body) } }
                    },
                    scroll = 0,
                )
            };
            let nodes: Vec<TuiNode> = view.nodes.into_iter().collect();

            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 4), None);
            let painted = fb.to_string();
            assert!(painted.contains("aaa"), "first item rendered: {painted}");
            assert!(painted.contains("bbb"), "second item rendered: {painted}");

            // Append: a new key is mapped; surviving keys keep their view.
            entries.set(vec![
                Entry { id: 1, body: "aaa".into() },
                Entry { id: 2, body: "bbb".into() },
                Entry { id: 3, body: "ccc".into() },
            ]);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 4), None);
            assert!(fb.to_string().contains("ccc"), "appended item renders");

            // Remove: the dropped key's view disposes; remaining keys persist.
            entries.set(vec![Entry { id: 3, body: "ccc".into() }]);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 4), None);
            let painted = fb.to_string();
            assert!(painted.contains("ccc"));
            assert!(!painted.contains("aaa"));
            assert!(!painted.contains("bbb"));
        });
    }

    /// `scroll` shifts the visible window: with a viewport shorter than the
    /// content, the top of the content is hidden.
    #[test]
    fn streaming_list_scroll_shifts_window() {
        let _ = create_root(|| {
            let entries = create_signal(
                (0..8).map(|i| Entry { id: i, body: format!("L{i}") }).collect(),
            );
            let scroll = create_signal(0i32);
            let view: View = view! {
                div(width = 6, height = 3) {
                    StreamingList(
                        items = *entries,
                        key = |e: &Entry| e.id,
                        view = |e: &Entry| {
                            let body = e.body.clone();
                            view! { p { (body) } }
                        },
                        scroll = scroll,
                    )
                }
            };
            let nodes: Vec<TuiNode> = view.nodes.into_iter().collect();

            // scroll = 0 → top of the list visible.
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 3), None);
            let painted = fb.to_string();
            assert!(painted.contains("L0"), "scroll=0 shows L0: {painted}");
            assert!(!painted.contains("L4"), "scroll=0 hides L4: {painted}");

            // scroll = 3 → first three rows hidden.
            scroll.set(3);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 3), None);
            let painted = fb.to_string();
            assert!(!painted.contains("L0"));
            assert!(painted.contains("L3"), "scroll=3 shows L3 at top: {painted}");

            // i32::MAX → stick to bottom.
            scroll.set(i32::MAX);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 3), None);
            let painted = fb.to_string();
            assert!(painted.contains("L7"), "stick-to-bottom shows L7: {painted}");
            assert!(!painted.contains("L0"));

            // Negative scroll = "scrolled up N rows from stick-to-bottom".
            // viewport=3 of 8 lines stick-to-bottom shows L5,L6,L7; -2 hides
            // the bottom two so the window slides up to L3,L4,L5.
            scroll.set(-2);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 6, 3), None);
            let painted = fb.to_string();
            assert!(painted.contains("L3"), "scroll=-2 shows L3 at top: {painted}");
            assert!(painted.contains("L5"), "scroll=-2 still shows L5 at bottom: {painted}");
            assert!(!painted.contains("L7"));
        });
    }

    /// A surviving item's view closure is **not** re-invoked on list changes:
    /// streaming updates must go through a per-item signal created inside the
    /// view closure. This is the load-bearing property for streaming markdown
    /// (assistant body keeps the same id; its body signal mutates).
    #[test]
    fn streaming_list_per_item_signal_drives_updates() {
        use std::cell::Cell;
        use std::rc::Rc;

        let _ = create_root(|| {
            // Count how many times the view closure runs for id=1. With a
            // proper keyed engine + per-item signal, it runs exactly once even
            // though the body changes three times.
            let view_invocations: Rc<Cell<u32>> = Rc::new(Cell::new(0));
            let invocations = view_invocations.clone();

            #[derive(Clone, PartialEq)]
            struct Item {
                id: u64,
                body: Signal<String>,
            }

            let item1 = Item {
                id: 1,
                body: create_signal(String::from("initial")),
            };
            let body1 = item1.body;
            let entries = create_signal(vec![item1]);

            let view: View = view! {
                StreamingList(
                    items = *entries,
                    key = |e: &Item| e.id,
                    view = move |e: &Item| {
                        invocations.set(invocations.get() + 1);
                        let body = e.body;
                        view! { p { (move || body.get_clone()) } }
                    },
                    scroll = 0,
                )
            };
            let nodes: Vec<TuiNode> = view.nodes.into_iter().collect();

            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 10, 2), None);
            assert!(fb.to_string().contains("initial"));

            // Streaming-style updates to the per-item signal.
            body1.set("token1".into());
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 10, 2), None);
            assert!(fb.to_string().contains("token1"));

            body1.set("token1+2".into());
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 10, 2), None);
            assert!(fb.to_string().contains("token1+2"));

            // The view closure ran exactly once for id=1, not once per body.set.
            assert_eq!(view_invocations.get(), 1, "view closure must be reused");
        });
    }
}
