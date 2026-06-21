//! HTML-shaped retained nodes for the TUI backend.

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::reactive::create_effect;

use crate::attributes::{BoolAttribute, StringAttribute};
use crate::events::Event;
use crate::style::Style;
use crate::surface::TextSurface;
use crate::view::{View, ViewNode, ViewTuiNode};

static NEXT_NODE_ID: AtomicU64 = AtomicU64::new(1);

/// Stable runtime node identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u64);

impl NodeId {
    fn next() -> Self {
        Self(NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Stored event handler.
pub type BoxedHandler = Box<dyn FnMut(&Event) + 'static>;
pub(crate) type SharedHandler = Rc<RefCell<BoxedHandler>>;

/// A flat style property: a named value that knows how to write itself into a
/// [`Style`] when the style is resolved for layout/paint. This is the storage
/// behind the per-property `MaybeDyn` surface (`padding = 2`, `color = …`).
pub trait StylePropValue {
    /// Apply this value to the relevant field of `style`.
    fn apply(&self, style: &mut Style);
}

impl StylePropValue for Box<dyn StylePropValue> {
    fn apply(&self, style: &mut Style) {
        (**self).apply(style);
    }
}

/// A type-erased flat style property, stored as `(name, value)` pairs on an
/// element. Resolution walks these in order, letting later assignments win —
/// matching HTML attribute precedence.
pub type StyleProp = (Cow<'static, str>, Box<dyn StylePropValue>);

/// Element node data, boxed in [`TuiNode`] to keep the enum compact.
pub struct ElementNode {
    pub id: NodeId,
    pub tag: Cow<'static, str>,
    pub attributes: Vec<(Cow<'static, str>, StringAttribute)>,
    pub bool_attributes: Vec<(Cow<'static, str>, BoolAttribute)>,
    pub style_props: Vec<StyleProp>,
    pub children: Vec<TuiNode>,
    pub handlers: Vec<(Cow<'static, str>, SharedHandler)>,
}

/// A node in the retained TUI view tree.
pub enum TuiNode {
    Element(Box<ElementNode>),
    Marker {
        id: NodeId,
    },
    Dynamic {
        id: NodeId,
        view: Rc<RefCell<View<TuiNode>>>,
    },
    /// A text surface leaf. Components produce a logical `TextSurface`; taffy
    /// supplies a rect; the surface module shapes it into visual rows for the
    /// canvas.
    TextSurface {
        id: NodeId,
        surface: Rc<RefCell<TextSurface>>,
        scroll: Rc<RefCell<i32>>,
    },
}

impl TuiNode {
    pub fn id(&self) -> NodeId {
        match self {
            TuiNode::Element(element) => element.id,
            TuiNode::Marker { id }
            | TuiNode::Dynamic { id, .. }
            | TuiNode::TextSurface { id, .. } => *id,
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            TuiNode::Element(element) => Some(element.tag.as_ref()),
            _ => None,
        }
    }

    pub(crate) fn append_attribute(&mut self, name: Cow<'static, str>, value: StringAttribute) {
        match self {
            TuiNode::Element(element) => element.attributes.push((name, value)),
            _ => panic!("can only set attribute on an element"),
        }
    }

    pub(crate) fn append_bool_attribute(&mut self, name: Cow<'static, str>, value: BoolAttribute) {
        match self {
            TuiNode::Element(element) => element.bool_attributes.push((name, value)),
            _ => panic!("can only set bool attribute on an element"),
        }
    }

    pub(crate) fn set_style_prop(&mut self, prop: StyleProp) {
        match self {
            TuiNode::Element(element) => element.style_props.push(prop),
            _ => panic!("can only set style on an element"),
        }
    }

    pub(crate) fn append_handler(&mut self, name: Cow<'static, str>, handler: BoxedHandler) {
        match self {
            TuiNode::Element(element) => element
                .handlers
                .push((name, Rc::new(RefCell::new(handler)))),
            _ => panic!("can only set event handler on an element"),
        }
    }

    pub(crate) fn collect_text(&self, out: &mut String) {
        match self {
            TuiNode::Element(element) => {
                for child in &element.children {
                    child.collect_text(out);
                }
            }
            TuiNode::Dynamic { view, .. } => {
                for node in &view.borrow().nodes {
                    node.collect_text(out);
                }
            }
            TuiNode::Marker { .. } => {}
            TuiNode::TextSurface { surface, .. } => out.push_str(&surface.borrow().plain_text()),
        }
    }

    pub(crate) fn attribute_value(&self, name: &str) -> Option<String> {
        match self {
            TuiNode::Element(element) => {
                element.attributes.iter().rev().find_map(|(key, value)| {
                    (key.as_ref() == name)
                        .then(|| value.get_clone())
                        .flatten()
                        .map(Cow::into_owned)
                })
            }
            _ => None,
        }
    }

    pub(crate) fn bool_attribute(&self, name: &str) -> bool {
        match self {
            TuiNode::Element(element) => element
                .bool_attributes
                .iter()
                .rev()
                .find_map(|(key, value)| (key.as_ref() == name).then(|| value.get()))
                .unwrap_or(false),
            _ => false,
        }
    }

    pub(crate) fn event_handlers(&self, name: &str) -> Vec<SharedHandler> {
        match self {
            TuiNode::Element(element) => element
                .handlers
                .iter()
                .filter(|(handler_name, _)| handler_name.as_ref() == name)
                .map(|(_, handler)| Rc::clone(handler))
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl std::fmt::Debug for TuiNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TuiNode::Element(element) => f
                .debug_struct("Element")
                .field("id", &element.id)
                .field("tag", &element.tag)
                .finish_non_exhaustive(),
            TuiNode::Marker { id } => f.debug_struct("Marker").field("id", id).finish(),
            TuiNode::Dynamic { id, .. } => f
                .debug_struct("Dynamic")
                .field("id", id)
                .finish_non_exhaustive(),
            TuiNode::TextSurface { id, .. } => f
                .debug_struct("TextSurface")
                .field("id", id)
                .finish_non_exhaustive(),
        }
    }
}

impl From<TuiNode> for View<TuiNode> {
    fn from(node: TuiNode) -> Self {
        View::from_node(node)
    }
}

impl ViewNode for TuiNode {
    fn append_child(&mut self, child: Self) {
        match self {
            TuiNode::Element(element) => element.children.push(child),
            _ => panic!("can only append child to an element"),
        }
    }

    fn create_dynamic_view<U: Into<View<Self>> + 'static>(
        mut f: impl FnMut() -> U + 'static,
    ) -> View<Self> {
        if TypeId::of::<U>() == TypeId::of::<String>() {
            let surface = Rc::new(RefCell::new(TextSurface::new()));
            create_effect({
                let surface = Rc::clone(&surface);
                move || {
                    let mut value = Some(f());
                    let value: &mut Option<String> =
                        (&mut value as &mut dyn Any).downcast_mut().unwrap();
                    let s = value.take().unwrap();
                    *surface.borrow_mut() = TextSurface::from_text(s);
                }
            });
            View::from(TuiNode::TextSurface {
                id: NodeId::next(),
                surface,
                scroll: Rc::new(RefCell::new(0)),
            })
        } else {
            let view = Rc::new(RefCell::new(View::new()));
            create_effect({
                let view = Rc::clone(&view);
                move || {
                    *view.borrow_mut() = f().into();
                }
            });
            View::from((
                TuiNode::Marker { id: NodeId::next() },
                TuiNode::Dynamic {
                    id: NodeId::next(),
                    view,
                },
                TuiNode::Marker { id: NodeId::next() },
            ))
        }
    }
}

impl ViewTuiNode for TuiNode {
    fn create_element(tag: Cow<'static, str>) -> Self {
        Self::Element(Box::new(ElementNode {
            id: NodeId::next(),
            tag,
            attributes: Vec::new(),
            bool_attributes: Vec::new(),
            style_props: Vec::new(),
            children: Vec::new(),
            handlers: Vec::new(),
        }))
    }

    fn create_text_node(text: Cow<'static, str>) -> Self {
        Self::create_text_surface_node(TextSurface::from_text(text), 0)
    }

    fn create_marker_node() -> Self {
        Self::Marker { id: NodeId::next() }
    }

    fn create_text_surface_node(surface: TextSurface, scroll: i32) -> Self {
        Self::TextSurface {
            id: NodeId::next(),
            surface: Rc::new(RefCell::new(surface)),
            scroll: Rc::new(RefCell::new(scroll)),
        }
    }
}

/// A builder wrapper that exposes a mutable TUI node.
pub trait AsTuiNode {
    fn as_tui_node(&mut self) -> &mut TuiNode;
}

impl AsTuiNode for TuiNode {
    fn as_tui_node(&mut self) -> &mut TuiNode {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_surface_node_holds_surface_and_scroll() {
        let surface = Rc::new(RefCell::new(TextSurface::from_text("hello")));
        let scroll = Rc::new(RefCell::new(0i32));
        let node = TuiNode::TextSurface {
            id: NodeId::next(),
            surface: surface.clone(),
            scroll: scroll.clone(),
        };
        let mut s = String::new();
        node.collect_text(&mut s);
        assert_eq!(s, "hello");
        assert_eq!(*scroll.borrow(), 0);
        *scroll.borrow_mut() = 3;
        assert_eq!(*scroll.borrow(), 3);
    }

    #[test]
    fn dynamic_string_becomes_text_surface_that_updates() {
        use crate::reactive::create_root;
        use crate::view::View;
        let root = create_root(|| {
            let sig = crate::reactive::create_signal("a".to_string());
            let view: View = (move || sig.get_clone()).into();
            let node = &view.nodes()[0];
            let row_count = match node {
                TuiNode::TextSurface { surface, .. } => surface.borrow().row_count(),
                _ => panic!("expected TextSurface, got {node:?}"),
            };
            assert_eq!(row_count, 1, "one row for \"a\"");
        });
        root.dispose();
    }
}
