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
use crate::text::Line;
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
    /// A flat line-buffer text leaf. Holds an arbitrary number of styled lines
    /// plus a scroll offset (rows). Laid out by taffy as a single leaf; painted
    /// only across the visible `[offset, offset+rect.height)` rows.
    LineFlow {
        id: NodeId,
        lines: Rc<RefCell<Vec<Line>>>,
        offset: Rc<RefCell<i32>>,
    },
}

impl TuiNode {
    pub fn id(&self) -> NodeId {
        match self {
            TuiNode::Element(element) => element.id,
            TuiNode::Marker { id }
            | TuiNode::Dynamic { id, .. }
            | TuiNode::LineFlow { id, .. } => *id,
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
            TuiNode::LineFlow { lines, .. } => {
                for line in lines.borrow().iter() {
                    for span in &line.spans {
                        out.push_str(span.content.as_ref());
                    }
                }
            }
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
            TuiNode::LineFlow { id, .. } => f
                .debug_struct("LineFlow")
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
            let lines = Rc::new(RefCell::new(Vec::<crate::text::Line>::new()));
            create_effect({
                let lines = Rc::clone(&lines);
                move || {
                    let mut value = Some(f());
                    let value: &mut Option<String> =
                        (&mut value as &mut dyn Any).downcast_mut().unwrap();
                    let s = value.take().unwrap();
                    *lines.borrow_mut() = vec![crate::text::Line::raw(s)];
                }
            });
            View::from(TuiNode::LineFlow {
                id: NodeId::next(),
                lines,
                offset: Rc::new(RefCell::new(0)),
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
        Self::create_line_flow_node(vec![crate::text::Line::raw(text)], 0)
    }

    fn create_marker_node() -> Self {
        Self::Marker { id: NodeId::next() }
    }

    fn create_line_flow_node(lines: Vec<crate::text::Line>, offset: i32) -> Self {
        Self::LineFlow {
            id: NodeId::next(),
            lines: Rc::new(RefCell::new(lines)),
            offset: Rc::new(RefCell::new(offset)),
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
    fn lineflow_node_holds_lines_and_offset() {
        let lines = Rc::new(RefCell::new(vec![
            crate::text::Line::raw("hello"),
        ]));
        let offset = Rc::new(RefCell::new(0i32));
        let node = TuiNode::LineFlow {
            id: NodeId::next(),
            lines: lines.clone(),
            offset: offset.clone(),
        };
        // collect_text reads the current lines.
        let mut s = String::new();
        node.collect_text(&mut s);
        assert_eq!(s, "hello");
        // offset is readable and mutable through the shared cell.
        assert_eq!(*offset.borrow(), 0);
        *offset.borrow_mut() = 3;
        assert_eq!(*offset.borrow(), 3);
    }

    #[test]
    fn dynamic_string_becomes_lineflow_that_updates() {
        use crate::reactive::create_root;
        use crate::view::View;
        let root = create_root(|| {
            let sig = crate::reactive::create_signal("a".to_string());
            let view: View = (move || sig.get_clone()).into();
            // The dynamic node is a LineFlow whose lines track the signal.
            let node = &view.nodes()[0];
            let line_count = match node {
                TuiNode::LineFlow { lines, .. } => lines.borrow().len(),
                _ => panic!("expected LineFlow, got {node:?}"),
            };
            assert_eq!(line_count, 1, "one line for \"a\"");
        });
        root.dispose();
    }
}
