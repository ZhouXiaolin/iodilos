//! View tree abstractions for the TUI backend.

use std::borrow::Cow;
use std::fmt;

use crate::component::Children;
use crate::reactive::{MaybeDyn, ReadSignal, Signal};
use smallvec::{SmallVec, smallvec};

use crate::node::TuiNode;
use crate::surface::{TextRow, TextSegment, TextSurface};

/// A view backed by TUI nodes.
pub struct View<T = TuiNode> {
    pub(crate) nodes: SmallVec<[T; 1]>,
}

impl<T> View<T> {
    pub fn new() -> Self {
        Self {
            nodes: SmallVec::new(),
        }
    }

    pub fn from_node(node: T) -> Self {
        Self {
            nodes: smallvec![node],
        }
    }

    pub fn from_nodes(nodes: Vec<T>) -> Self {
        Self {
            nodes: nodes.into(),
        }
    }

    pub fn from_dynamic<U: Into<Self> + 'static>(f: impl FnMut() -> U + 'static) -> Self
    where
        T: ViewNode,
    {
        T::create_dynamic_view(f)
    }

    pub fn nodes(&self) -> &[T] {
        &self.nodes
    }
}

impl View<TuiNode> {
    /// Build a text-surface view with scroll offset 0.
    pub fn text_surface(surface: TextSurface) -> Self {
        View::from_node(TuiNode::create_text_surface_node(surface, 0))
    }
}

impl<T> Default for View<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for View<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View").finish()
    }
}

impl<T> From<Children<Self>> for View<T> {
    fn from(children: Children<Self>) -> Self {
        children.call()
    }
}

impl<T> From<Vec<View<T>>> for View<T> {
    fn from(nodes: Vec<View<T>>) -> Self {
        Self {
            nodes: nodes.into_iter().flat_map(|v| v.nodes).collect(),
        }
    }
}

impl<T> From<Option<View<T>>> for View<T> {
    fn from(node: Option<View<T>>) -> Self {
        node.unwrap_or_default()
    }
}

impl<T: ViewNode, U: Clone + Into<Self>> From<ReadSignal<U>> for View<T> {
    fn from(signal: ReadSignal<U>) -> Self {
        (move || signal.get_clone()).into()
    }
}

impl<T: ViewNode, U: Clone + Into<Self>> From<Signal<U>> for View<T> {
    fn from(signal: Signal<U>) -> Self {
        (*signal).into()
    }
}

impl<T: ViewNode, U: Clone + Into<Self> + Into<MaybeDyn<U>>> From<MaybeDyn<U>> for View<T> {
    fn from(value: MaybeDyn<U>) -> Self {
        (move || value.get_clone()).into()
    }
}

macro_rules! impl_view_from {
    ($($ty:ty),*) => {
        $(
            impl<T: ViewTuiNode> From<$ty> for View<T> {
                fn from(t: $ty) -> Self {
                    View::from_node(T::create_text_surface_node(TextSurface::from_text(t), 0))
                }
            }
        )*
    };
}

macro_rules! impl_view_from_to_string {
    ($($ty:ty),*) => {
        $(
            impl<T: ViewTuiNode> From<$ty> for View<T> {
                fn from(t: $ty) -> Self {
                    View::from_node(T::create_text_surface_node(
                        TextSurface::from_text(t.to_string()),
                        0,
                    ))
                }
            }
        )*
    };
}

impl_view_from!(&'static str, String, Cow<'static, str>);
impl_view_from_to_string!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

impl<T: ViewTuiNode> From<TextSurface> for View<T> {
    fn from(surface: TextSurface) -> Self {
        View::from_node(T::create_text_surface_node(surface, 0))
    }
}

impl<T: ViewTuiNode> From<TextRow> for View<T> {
    fn from(row: TextRow) -> Self {
        View::from_node(T::create_text_surface_node(TextSurface::from_row(row), 0))
    }
}

impl<T: ViewTuiNode> From<TextSegment> for View<T> {
    fn from(segment: TextSegment) -> Self {
        View::from_node(T::create_text_surface_node(TextSurface::from(segment), 0))
    }
}

impl<T: ViewNode, F: FnMut() -> U + 'static, U: Into<View<T>> + 'static> From<F> for View<T> {
    fn from(f: F) -> Self {
        T::create_dynamic_view(f)
    }
}

macro_rules! impl_from_tuple {
    ($($name:ident),*) => {
        paste::paste! {
            impl<U, $($name),*> From<($($name,)*)> for View<U>
            where
                $($name: Into<View<U>>),*
            {
                fn from(t: ($($name,)*)) -> Self {
                    let ($([<$name:lower>]),*) = t;
                    #[allow(unused_mut)]
                    let mut nodes = SmallVec::new();
                    $(
                        nodes.extend([<$name:lower>].into().nodes);
                    )*
                    View { nodes }
                }
            }
        }
    };
}

impl_from_tuple!();
impl_from_tuple!(A, B);
impl_from_tuple!(A, B, C);
impl_from_tuple!(A, B, C, D);
impl_from_tuple!(A, B, C, D, E);
impl_from_tuple!(A, B, C, D, E, F);
impl_from_tuple!(A, B, C, D, E, F, G);
impl_from_tuple!(A, B, C, D, E, F, G, H);
impl_from_tuple!(A, B, C, D, E, F, G, H, I);
impl_from_tuple!(A, B, C, D, E, F, G, H, I, J);

pub trait ViewNode: Into<View<Self>> + Sized + 'static {
    fn append_child(&mut self, child: Self);

    fn append_view(&mut self, view: View<Self>) {
        for node in view.nodes {
            self.append_child(node);
        }
    }

    fn create_dynamic_view<U: Into<View<Self>> + 'static>(
        mut f: impl FnMut() -> U + 'static,
    ) -> View<Self> {
        f().into()
    }
}

pub trait ViewTuiNode: ViewNode {
    fn create_element(tag: Cow<'static, str>) -> Self;
    fn create_text_node(text: Cow<'static, str>) -> Self;
    fn create_marker_node() -> Self;
    /// Build a text-surface node with the given initial scroll offset.
    fn create_text_surface_node(surface: TextSurface, scroll: i32) -> Self;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_surface_builds_multiline_view() {
        let view = View::text_surface(TextSurface::from_rows(vec![
            TextRow::raw("first"),
            TextRow::from(TextSegment::raw("second")),
        ]));
        let node = &view.nodes()[0];
        let row_count = match node {
            TuiNode::TextSurface { surface, .. } => surface.borrow().row_count(),
            _ => panic!("expected TextSurface, got {node:?}"),
        };
        assert_eq!(row_count, 2);
    }
}
