//! The `Show` conditional component.
//!
//! Ported from `sycamore-web`'s `Show`, simplified for the TUI backend: iodilos
//! has no SSR and no retained DOM, so `Show` is a dynamic region that yields its
//! children when `when` is true and an empty view otherwise. While shown, the
//! children's own reactive regions keep updating in place; when hidden, they
//! are dropped (the dynamic slot is empty) and rebuilt on next show — matching
//! sycamore semantics.

use crate::component::Children;
use crate::prelude::*;

/// Props for [`Show`].
#[derive(Props)]
pub struct ShowProps {
    /// The condition. Accepts a plain `bool` or any reactive source (`Signal`,
    /// `ReadSignal`, closure) via `MaybeDyn`.
    #[prop(setter(into))]
    pub when: MaybeDyn<bool>,
    /// The children rendered when `when` is true.
    pub children: Children<View>,
}

/// A component that only renders its children when `when` is true.
///
/// # Example
/// ```ignore
/// # use iodilos::prelude::*;
/// # fn app(visible: Signal<bool>) -> View {
/// view! {
///     Show(when = visible) {
///         p { "Now you see me" }
///     }
/// }
/// # }
/// ```
#[component]
pub fn Show(props: ShowProps) -> View {
    // Build the children once (their internal reactive regions self-update via
    // shared `Rc`s while the slot is kept alive). Each dynamic re-evaluation
    // clones the cached `View` — cheap, since clones share the producer/marker
    // `Rc`s rather than deep-copying shaped cells.
    let children = props.children.call();
    View::from_dynamic(move || if props.when.get() {
        children.clone()
    } else {
        View::new()
    })
}

#[cfg(test)]
mod tests {
    use crate::framebuffer::Rect;
    use crate::layout::render as render_buffer;
    use crate::node::TuiNode;
    use crate::prelude::*;
    use crate::reactive::create_root;

    #[test]
    fn show_toggles_children_with_condition() {
        let _ = create_root(|| {
            let visible = create_signal(true);
            let view: View = view! {
                Show(when = visible) {
                    p { "shown" }
                }
            };
            let nodes: Vec<TuiNode> = view.nodes.into_iter().collect();

            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 20, 2), None);
            assert!(
                fb.to_string().contains("shown"),
                "children render when when=true"
            );

            // Hide: the dynamic slot empties, "shown" disappears from the frame.
            visible.set(false);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 20, 2), None);
            assert!(
                !fb.to_string().contains("shown"),
                "children hidden when when=false: {:?}",
                fb.to_string()
            );

            // Re-show: children come back.
            visible.set(true);
            let (fb, _) = render_buffer(&nodes, Rect::new(0, 0, 20, 2), None);
            assert!(
                fb.to_string().contains("shown"),
                "children re-render when when flips back to true"
            );
        });
    }
}
