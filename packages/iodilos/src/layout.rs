//! Taffy layout and terminal painting onto the self-built [`Framebuffer`].
//!
//! This is the crossterm-without-ratatui paint path (ADR-0024 §10–§12):
//! taffy computes layout, a `Framebuffer` holds painted output, and the render
//! driver diffs the `Framebuffer` between frames. The legacy ratatui `Buffer` and
//! `Rect` are gone.

use std::borrow::Cow;
use std::collections::HashMap;

use taffy::prelude::{AvailableSpace, Dimension, FlexDirection, Size};
use taffy::{NodeId as TaffyNodeId, TaffyTree};

use crate::attributes::resolve_style;
use crate::framebuffer::{Cell, Framebuffer, Rect};
use crate::node::{NodeId, TuiNode};
use crate::producer::CellProducer;
use crate::style::{Edges, Style};
use crate::text::SpanStyle;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeNode {
    pub parent: Option<NodeId>,
    pub rect: Rect,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeIndex {
    pub nodes: HashMap<NodeId, RuntimeNode>,
    pub focus_order: Vec<NodeId>,
    pub hit_order: Vec<NodeId>,
}

impl RuntimeIndex {
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn tag(&self, id: NodeId) -> Option<&str> {
        self.nodes.get(&id).and_then(|node| node.tag.as_deref())
    }

    pub fn first_focusable(&self) -> Option<NodeId> {
        self.focus_order.first().copied()
    }

    pub fn next_focus(&self, current: Option<NodeId>, reverse: bool) -> Option<NodeId> {
        if self.focus_order.is_empty() {
            return None;
        }
        let len = self.focus_order.len();
        let index = current
            .and_then(|id| {
                self.focus_order
                    .iter()
                    .position(|candidate| *candidate == id)
            })
            .unwrap_or(if reverse { 0 } else { len - 1 });
        let next = if reverse {
            (index + len - 1) % len
        } else {
            (index + 1) % len
        };
        self.focus_order.get(next).copied()
    }

    pub fn path_to_root(&self, target: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut current = Some(target);
        while let Some(id) = current {
            path.push(id);
            current = self.nodes.get(&id).and_then(|node| node.parent);
        }
        path
    }

    pub fn hit_test(&self, x: u16, y: u16) -> Option<NodeId> {
        self.hit_order.iter().rev().copied().find(|id| {
            self.nodes
                .get(id)
                .is_some_and(|node| contains_point(node.rect, x, y))
        })
    }
}

#[allow(dead_code)]
struct BuiltNode {
    runtime_id: NodeId,
    taffy_id: TaffyNodeId,
    parent: Option<NodeId>,
    tag: Option<String>,
    style: Style,
    focusable: bool,
    children: Vec<BuiltNode>,
    producer: Option<(Rc<RefCell<Box<dyn CellProducer>>>, i32)>,
}

#[derive(Clone)]
enum Measure {
    Producer { producer: Rc<RefCell<Box<dyn CellProducer>>> },
}

struct PaintNode {
    rect: Rect,
    style: Style,
    children: Vec<PaintNode>,
    producer: Option<(Rc<RefCell<Box<dyn CellProducer>>>, i32)>,
}

// PaintNode no longer carries a custom rect-recomputation pass. Taffy computes
// every node's layout correctly at build time; marker-delimited dynamic
// regions are expanded inline by build_children. extract_node resolves
// the taffy results directly.

// ── unified Rect (i32 coordinates) is now the single rect type ──
// The previous Rect → Rect conversion is gone: framebuffer::Rect already
// uses i32 for x/y, identical to the old Rect layout.

/// Lay out `nodes` into `area` and paint the result into a fresh [`Framebuffer`].
/// Also returns a [`RuntimeIndex`] for hit testing, focus, and event bubbling.
pub(crate) fn render(
    nodes: &[TuiNode],
    area: Rect,
    _focused: Option<NodeId>,
) -> (Framebuffer, RuntimeIndex) {
    let mut taffy = TaffyTree::<Measure>::new();
    // Top-level nodes are built exactly like an element's children (markers
    // delimiting dynamic regions are expanded inline), so a bare
    // `view! { Show(..) { .. } }` at the root renders rather than being
    // dropped as a no-op marker pair.
    let built_roots = build_children(&mut taffy, nodes, None);

    let root_children = built_roots
        .iter()
        .map(|node| node.taffy_id)
        .collect::<Vec<_>>();
    let root_style = taffy::style::Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::length(area.width as f32),
            height: Dimension::length(area.height as f32),
        },
        ..taffy::style::Style::default()
    };
    let root = taffy
        .new_with_children(root_style, &root_children)
        .expect("create taffy root");
    let _ = taffy.compute_layout_with_measure(
        root,
        Size {
            width: AvailableSpace::Definite(area.width as f32),
            height: AvailableSpace::Definite(area.height as f32),
        },
        |known, available, _node, context, _style| {
            context.map_or(Size::ZERO, |ctx| match ctx {
                Measure::Producer { producer } => measure_producer(&**producer.borrow(), known, available),
            })
        },
    );

    let mut index = RuntimeIndex::default();
    let paint_nodes = built_roots
        .iter()
        .map(|built| extract_node(&taffy, built, area, &mut index))
        .collect::<Vec<_>>();

    let mut fb = Framebuffer::empty(area);
    // Text paint inherits from the root; layout/border/background do not.
    let root_text = SpanStyle::default();
    for node in &paint_nodes {
        paint_node(&mut fb, node, root_text, area, area);
    }

    (fb, index)
}

fn build_node(
    tree: &mut TaffyTree<Measure>,
    node: &TuiNode,
    parent: Option<NodeId>,
) -> Option<BuiltNode> {
    match node {
        TuiNode::Marker { .. } => None,
        TuiNode::Leaf {
            id,
            producer,
            scroll,
        } => {
            let scroll_value = *scroll.borrow();
            let taffy_id = tree
                .new_leaf_with_context(
                    taffy::style::Style::default(),
                    Measure::Producer {
                        producer: Rc::clone(producer),
                    },
                )
                .expect("create leaf");
            Some(BuiltNode {
                runtime_id: *id,
                taffy_id,
                parent,
                tag: None,
                style: Style::default(),
                focusable: false,
                children: Vec::new(),
                producer: Some((Rc::clone(producer), scroll_value)),
            })
        }
        TuiNode::Element(element) => {
            let tag_name = element.tag.to_string();
            let style = resolve_style(&element.style_props, default_style_for_tag(&element.tag));
            let leaf = is_text_leaf(&element.tag, &element.children);
            let focusable = is_focusable(node);
            if leaf {
                let text = element_text(node);
                let display_text = display_text_for_tag(Some(&tag_name), &text).into_owned();
                let producer: Rc<RefCell<Box<dyn CellProducer>>> =
                    Rc::new(RefCell::new(Box::new(crate::producer::Plain::new(display_text))));
                let taffy_id = tree
                    .new_leaf_with_context(
                        style.to_taffy(),
                        Measure::Producer {
                            producer: Rc::clone(&producer),
                        },
                    )
                    .expect("create element leaf");
                Some(BuiltNode {
                    runtime_id: element.id,
                    taffy_id,
                    parent,
                    tag: Some(tag_name),
                    style,
                    focusable,
                    children: Vec::new(),
                    producer: Some((producer, 0)),
                })
            } else {
                let built_children = build_children(tree, &element.children, Some(element.id));
                let child_ids = built_children
                    .iter()
                    .map(|child| child.taffy_id)
                    .collect::<Vec<_>>();
                let taffy_id = tree
                    .new_with_children(style.to_taffy(), &child_ids)
                    .expect("create element");
                Some(BuiltNode {
                    runtime_id: element.id,
                    taffy_id,
                    parent,
                    tag: Some(tag_name),
                    style,
                    focusable,
                    children: built_children,
                    producer: None,
                })
            }
        }
    }
}

/// Build taffy children for a parent element's child list, expanding
/// marker-delimited dynamic regions inline (sycamore-web pattern). `parent` is
/// `None` for the top-level root list, `Some(element_id)` inside an element.
fn build_children(
    tree: &mut TaffyTree<Measure>,
    children: &[TuiNode],
    parent: Option<NodeId>,
) -> Vec<BuiltNode> {
    let mut built = Vec::new();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            TuiNode::Marker {
                slot: Some(content), ..
            } => {
                // Dynamic region: expand content nodes inline into the parent.
                for node in &*content.borrow() {
                    if let Some(b) = build_node(tree, node, parent) {
                        built.push(b);
                    }
                }
                // Skip to the end marker.
                i += 1;
                while i < children.len() {
                    if matches!(&children[i], TuiNode::Marker { slot: None, .. }) {
                        break;
                    }
                    i += 1;
                }
            }
            child => {
                if let Some(b) = build_node(tree, child, parent) {
                    built.push(b);
                }
            }
        }
        i += 1;
    }
    built
}

fn extract_node(
    tree: &TaffyTree<Measure>,
    built: &BuiltNode,
    parent_rect: Rect,
    index: &mut RuntimeIndex,
) -> PaintNode {
    let layout = tree.layout(built.taffy_id).expect("taffy layout");
    let rect = rect_from_layout(layout, parent_rect);
    index.nodes.insert(
        built.runtime_id,
        RuntimeNode {
            parent: built.parent,
            rect,
            tag: built.tag.clone(),
        },
    );
    index.hit_order.push(built.runtime_id);
    if built.focusable {
        index.focus_order.push(built.runtime_id);
    }
    let children = built
        .children
        .iter()
        .map(|child| {
            let paint = extract_node(tree, child, rect, index);
            paint
        })
        .collect();
    PaintNode {
        rect,
        style: built.style.clone(),
        children,
        producer: built.producer.clone(),
    }
}

/// Write one shaped [`Cell`] row into the framebuffer at screen row `y`,
/// starting at column `x`, clipping to `clip_rect`. The producer has already
/// shaped the row to `width` cells; this only copies the in-bounds cells into
/// the framebuffer row. The inherited `text` style was applied when the
/// producer built the cells, so no patching happens here.
fn write_row_into(
    fb: &mut Framebuffer,
    y: i32,
    x: i32,
    clip_rect: &Rect,
    row: &[Cell],
) {
    if y < 0 || y as usize >= fb.height() {
        return;
    }
    let dst = fb.row_mut(y);
    for (col, cell) in row.iter().enumerate() {
        let abs_x = x + col as i32;
        if abs_x < clip_rect.x || abs_x >= clip_rect.right() {
            continue;
        }
        let ux = abs_x as usize;
        if ux >= dst.len() {
            continue;
        }
        dst[ux] = cell.clone();
    }
}

/// Paint a node into the framebuffer: background, border, then text, recursing with
/// the inherited text style (ADR-0024 §6). Layout properties, `border_*`, and
/// container `background_color` do not inherit.
///
/// `clip` is the region a normally-flowing child is clipped to (its parent's
/// content box when the parent hides overflow, else the screen). `screen` is
/// always the full drawable area: a `Position::Absolute` child lives in its
/// containing block rather than the parent's flex flow, so it is clipped to
/// `screen` instead — this lets a freely-dragged box roam the whole terminal
/// without being erased at the parent's edge (ADR-0008, mirroring iodilos's
/// `draw_tree`). A zero-size node (e.g. a transparent `Dynamic` container that
/// collapsed because its only child is out-of-flow) still recurses into its
/// children; it just draws nothing of its own.
fn paint_node(
    fb: &mut Framebuffer,
    node: &PaintNode,
    parent_text: SpanStyle,
    clip: Rect,
    screen: Rect,
) {
    let has_size = node.rect.width != 0 && node.rect.height != 0;

    // Resolve the inheritable text style: this node's text-paint fields inherit
    // from the parent's.
    let text = parent_text.patch(node.style.text_span_style());
    if has_size {
        if let Some(bg) = node.style.background_color
            && let Some(intersected) = node.rect.intersect(clip)
        {
            // A background is opaque: it must cover whatever was painted into
            // these cells earlier (e.g. text beneath an absolutely-positioned
            // box), so erase the characters first, then fill the background.
            // Mirrors iocraft's `View::draw`, which `clear_text`s before
            // `set_background_color`. This node's own border/text are drawn
            // afterwards and so still show on top.
            fb.clear_text(intersected);
            fb.set_background_color(intersected, bg);
        }

        if let Some(border_chars) = node.style.border_style.border_characters() {
            // The border uses only its own color, not inherited text paint.
            let border_style = SpanStyle {
                fg: node.style.border_color,
                ..SpanStyle::default()
            };
            paint_border_clipped(
                fb,
                node.rect,
                border_chars,
                node.style.border_edges,
                border_style,
                clip,
            );
        }

        if let Some((producer, scroll)) = &node.producer {
            let width = node.rect.width as usize;
            let mut rows = producer.borrow().render(width);
            // Patch each cell's glyph style with the inherited text style so
            // ancestor properties (dim, crossed_out, underline_color, fg, bg)
            // propagate through the tree, matching the legacy TextSurface
            // layout behaviour.
            for row in &mut rows {
                for cell in row.iter_mut() {
                    if let Some(glyph) = &mut cell.glyph {
                        glyph.style = text.patch(glyph.style);
                    }
                }
            }
            let clip_rect = node.rect.intersect(clip).unwrap_or_default();
            // Wipe any stale glyphs from the previous frame inside this node's
            // visible rect BEFORE painting the new rows. Without this, a leaf
            // that does not cover every cell of its rect would leave old glyphs
            // in the framebuffer; the diff path then either short-circuits on
            // equal cells or keeps the previous frame's inline-code background
            // visible, producing the "bg shifts on scroll" artefact. `clear_text`
            // resets the cells to their default glyph (None).
            if clip_rect.width > 0 && clip_rect.height > 0 {
                fb.clear_text(clip_rect);
            }
            // The on-screen window for this node is its clipped rect. The node's
            // own `rect.height` is its *natural* content height (taffy does not
            // shrink a child of an `overflow: hidden` parent), so the visible
            // height is the clipped rect's height. Clamp `scroll` to
            // `[0, total - visible_height]` so a large value means "stick to
            // bottom": the caller passes a sentinel (e.g. `i32::MAX`) and the
            // last `visible_height` rows land inside the clip without the caller
            // having to know the viewport height.
            let visible_height = clip_rect.height as usize;
            let total = rows.len();
            let max_scroll = total.saturating_sub(visible_height);
            let scroll = (*scroll).max(0) as usize;
            let scroll = scroll.min(max_scroll);
            for (i, row) in rows.iter().enumerate() {
                if i < scroll {
                    continue;
                }
                let y = node.rect.y + (i - scroll) as i32;
                if y < clip_rect.y || y >= clip_rect.bottom() {
                    continue;
                }
                write_row_into(fb, y, node.rect.x, &clip_rect, row);
            }
        }
    }

    let child_clip = if node.style.overflow == taffy::style::Overflow::Visible {
        clip
    } else {
        clip.intersect(content_rect(node)).unwrap_or_default()
    };
    // Sort children by z-index so higher values paint on top (painter's algorithm).
    let mut sorted: Vec<&PaintNode> = node.children.iter().collect();
    sorted.sort_by_key(|child| child.style.z_index);
    for child in sorted {
        // An absolutely-positioned child lives in its containing block, not this
        // node's flex flow, so clip it to the screen rather than the parent.
        let bounds = if child.style.position == taffy::style::Position::Absolute {
            screen
        } else {
            child_clip
        };
        paint_node(fb, child, text, bounds, screen);
    }
}

fn paint_border_clipped(
    fb: &mut Framebuffer,
    rect: Rect,
    chars: crate::style::BorderCharacters,
    edges: Option<Edges>,
    style: SpanStyle,
    clip: Rect,
) {
    let edges = edges.unwrap_or(Edges::all());
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let right = rect.x + (rect.width as i32) - 1;
    let bottom = rect.y + (rect.height as i32) - 1;

    if edges.contains(Edges::TOP) {
        let left_border_size = u16::from(edges.contains(Edges::LEFT));
        let right_border_size = u16::from(edges.contains(Edges::RIGHT));
        if edges.contains(Edges::LEFT) {
            paint_text_clipped_raw(
                fb,
                Rect::new(rect.x, rect.y, 1, 1),
                &chars.top_left.to_string(),
                style,
                clip,
            );
        }
        let width = rect
            .width
            .saturating_sub(left_border_size)
            .saturating_sub(right_border_size);
        paint_text_clipped_raw(
            fb,
            Rect::new(rect.x + (left_border_size as i32), rect.y, width, 1),
            &chars.top.to_string().repeat(width as usize),
            style,
            clip,
        );
        if edges.contains(Edges::RIGHT) {
            paint_text_clipped_raw(
                fb,
                Rect::new(right, rect.y, 1, 1),
                &chars.top_right.to_string(),
                style,
                clip,
            );
        }
    }
    for y in rect.y + 1..bottom {
        if edges.contains(Edges::LEFT) {
            paint_text_clipped_raw(
                fb,
                Rect::new(rect.x, y, 1, 1),
                &chars.left.to_string(),
                style,
                clip,
            );
        }
        if edges.contains(Edges::RIGHT) {
            paint_text_clipped_raw(
                fb,
                Rect::new(right, y, 1, 1),
                &chars.right.to_string(),
                style,
                clip,
            );
        }
    }
    if edges.contains(Edges::BOTTOM) {
        let left_border_size = u16::from(edges.contains(Edges::LEFT));
        let right_border_size = u16::from(edges.contains(Edges::RIGHT));
        if edges.contains(Edges::LEFT) {
            paint_text_clipped_raw(
                fb,
                Rect::new(rect.x, bottom, 1, 1),
                &chars.bottom_left.to_string(),
                style,
                clip,
            );
        }
        let width = rect
            .width
            .saturating_sub(left_border_size)
            .saturating_sub(right_border_size);
        paint_text_clipped_raw(
            fb,
            Rect::new(rect.x + (left_border_size as i32), bottom, width, 1),
            &chars.bottom.to_string().repeat(width as usize),
            style,
            clip,
        );
        if edges.contains(Edges::RIGHT) {
            paint_text_clipped_raw(
                fb,
                Rect::new(right, bottom, 1, 1),
                &chars.bottom_right.to_string(),
                style,
                clip,
            );
        }
    }
}

fn paint_text_clipped_raw(
    fb: &mut Framebuffer,
    rect: Rect,
    text: &str,
    style: SpanStyle,
    clip: Rect,
) {
    if text.is_empty() || rect.width == 0 || rect.height == 0 {
        return;
    }
    let width = rect.width as usize;
    let mut y = rect.y;
    let mut col = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            col = 0;
            y += 1;
            if y >= rect.bottom() {
                return;
            }
            continue;
        }
        let cw = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(0)
            .max(1);
        if col + cw > width {
            col = 0;
            y += 1;
            if y >= rect.bottom() {
                return;
            }
        }
        let x = rect.x + (col as i32);
        if clip.contains(x, y) {
            fb.set_text(
                Rect::new(x, y, cw as u16, 1),
                &ch.to_string(),
                style,
            );
        }
        col += cw;
    }
}

fn content_rect(node: &PaintNode) -> Rect {
    if node.style.border_style.is_none() {
        return node.rect;
    }
    let edges = node.style.border_edges.unwrap_or(Edges::all());
    let left = u16::from(edges.contains(Edges::LEFT));
    let right = u16::from(edges.contains(Edges::RIGHT));
    let top = u16::from(edges.contains(Edges::TOP));
    let bottom = u16::from(edges.contains(Edges::BOTTOM));
    Rect::new(
        node.rect.x + (left as i32),
        node.rect.y + (top as i32),
        node.rect.width.saturating_sub(left).saturating_sub(right),
        node.rect.height.saturating_sub(top).saturating_sub(bottom),
    )
}

fn measure_producer(
    producer: &dyn CellProducer,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
) -> Size<f32> {
    let raw_width = producer.intrinsic_width() as f32;
    let available_width = match available.width {
        AvailableSpace::Definite(w) => w.max(1.0),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => raw_width.max(1.0),
    };
    let width = known
        .width
        .unwrap_or(raw_width.min(available_width).max(1.0));
    let height =
        known
            .height
            .unwrap_or_else(|| producer.measure(width.max(1.0) as usize) as f32);
    Size { width, height }
}

fn rect_from_layout(layout: &taffy::Layout, parent_rect: Rect) -> Rect {
    let x = parent_rect.x as f32 + layout.location.x;
    let y = parent_rect.y as f32 + layout.location.y;
    Rect::new(
        x.round() as i32,
        y.round() as i32,
        layout.size.width.round().max(0.0) as u16,
        layout.size.height.round().max(0.0) as u16,
    )
}

fn default_style_for_tag(tag: &str) -> Style {
    let mut style = Style::default();
    if tag == "div" {
        style.flex_direction = FlexDirection::Column;
    }
    if tag == "input" {
        style.height = crate::style::Size::Length(1);
    }
    style
}

fn is_text_leaf(tag: &str, children: &[TuiNode]) -> bool {
    // An element with marker-delimited dynamic content must be a container,
    // not a leaf — the slot nodes are expanded inline during build.
    if children
        .iter()
        .any(|c| matches!(c, TuiNode::Marker { slot: Some(..), .. }))
    {
        return false;
    }
    matches!(tag, "span" | "p" | "input")
        || (tag == "button"
            && children
                .iter()
                .all(|child| matches!(child, TuiNode::Leaf { .. } | TuiNode::Marker { .. })))
}

fn element_text(node: &TuiNode) -> String {
    if node.tag() == Some("input") {
        return node
            .attribute_value("value")
            .or_else(|| node.attribute_value("placeholder"))
            .unwrap_or_default();
    }
    let mut text = String::new();
    node.collect_text(&mut text);
    text
}

fn display_text_for_tag<'a>(tag: Option<&str>, text: &'a str) -> Cow<'a, str> {
    match tag {
        Some("button") => Cow::Owned(format!("[ {text} ]")),
        Some("input") => Cow::Owned(format!("{text} ")),
        _ => Cow::Borrowed(text),
    }
}

fn is_focusable(node: &TuiNode) -> bool {
    if node.bool_attribute("disabled") {
        return false;
    }
    matches!(node.tag(), Some("button" | "input")) || node.attribute_value("tabindex").is_some()
}

fn contains_point(rect: Rect, x: u16, y: u16) -> bool {
    let x = x as i32;
    let y = y as i32;
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

#[cfg(test)]
mod tests {
    use crate::reactive::create_root;

    use super::*;
    use crate::attributes::{GlobalAttributes, GlobalAttributesExt};
    use crate::components::tags;
    use crate::style::{BorderCharacters, BorderStyle};
    use crate::view::View;

    #[test]
    fn focus_order_uses_html_like_focusable_elements() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::div()
                .children(vec![
                    View::from(tags::p().children("plain text")),
                    View::from(tags::button().children("Save")),
                    View::from(tags::input().disabled(true)),
                    View::from(tags::div().tabindex("0").children("Custom")),
                    View::from(tags::input()),
                ])
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_canvas, index) = render(&nodes, Rect::new(0, 0, 40, 10), None);
        let tags = index
            .focus_order
            .iter()
            .map(|id| index.tag(*id).expect("focusable nodes should be elements"))
            .collect::<Vec<_>>();

        assert_eq!(tags, ["button", "div", "input"]);
        assert_eq!(index.first_focusable(), index.focus_order.first().copied());
        assert_eq!(
            index.next_focus(index.focus_order.first().copied(), false),
            index.focus_order.get(1).copied()
        );
        assert_eq!(
            index.next_focus(index.focus_order.first().copied(), true),
            index.focus_order.last().copied()
        );
        root.dispose();
    }

    #[test]
    fn structured_button_paints_as_container_not_bracketed_text() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::button()
                .border_style(BorderStyle::Custom(BorderCharacters {
                    top: '▁',
                    ..Default::default()
                }))
                .border_edges(Edges::TOP)
                .children(tags::div().height(3).children(tags::span().children("7")))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render(&nodes, Rect::new(0, 0, 12, 5), None);
        let painted = canvas_to_plain_text(&fb);

        assert!(
            painted.contains('▁'),
            "custom button trim should draw: {painted}"
        );
        assert!(
            painted.contains('7'),
            "button child text should draw: {painted}"
        );
        assert!(
            !painted.contains("[ 7 ]"),
            "structured button should not use default bracket chrome: {painted}"
        );
        root.dispose();
    }

    #[test]
    fn top_only_border_spans_full_width() {
        let mut fb = Framebuffer::empty(Rect::new(0, 0, 6, 2));
        paint_border_clipped(
            &mut fb,
            Rect::new(0, 0, 6, 2).into(),
            BorderCharacters {
                top: '▁',
                ..Default::default()
            },
            Some(Edges::TOP),
            SpanStyle::default(),
            Rect::new(0, 0, 6, 2).into(),
        );

        let painted = canvas_to_plain_text(&fb);
        assert!(
            painted.starts_with("▁▁▁▁▁▁"),
            "top-only border should fill both ends: {painted}"
        );
    }

    #[test]
    fn overflow_hidden_clips_children_to_content_box() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::div()
                .width(12)
                .height(4)
                .border_style(BorderStyle::Single)
                .overflow(taffy::style::Overflow::Hidden)
                .children(tags::p().children("line 0\nline 1\nline 2\nline 3\nline 4"))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render(&nodes, Rect::new(0, 0, 20, 8), None);
        let painted = canvas_to_plain_text(&fb);

        assert!(
            painted.contains("line 0"),
            "first content line should paint: {painted}"
        );
        assert!(
            painted.contains("line 1"),
            "second content line should paint: {painted}"
        );
        assert!(
            !painted.contains("line 2"),
            "content after the bordered viewport should be clipped: {painted}"
        );
        root.dispose();
    }

    /// Regression for in-flow scrolling: an `overflow: hidden` viewport with an
    /// in-flow child translated up by a negative `margin_top` must show the
    /// child's TAIL (it scrolled up) and clip its head. Before the `Rect`
    /// change, `rect_from_layout` clamped the child's negative y to 0, so the
    /// content stayed pinned to the top and never scrolled — the markdown
    /// example's follow-the-tail appeared frozen at the first line.
    #[test]
    fn negative_margin_scrolls_in_flow_child_within_overflow_hidden() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            // Viewport: 12 wide, 4 tall, bordered (content box is 2 rows).
            // Child has 5 lines and a -3 top margin → lines 0,1,2 scroll above
            // the viewport; lines 3,4 should be visible inside the box.
            let view: View = tags::div()
                .width(12)
                .height(4)
                .border_style(BorderStyle::Single)
                .overflow(taffy::style::Overflow::Hidden)
                .children(
                    tags::div()
                        .margin_top(-3)
                        .children(tags::p().children("line 0\nline 1\nline 2\nline 3\nline 4")),
                )
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render(&nodes, Rect::new(0, 0, 20, 8), None);
        let painted = canvas_to_plain_text(&fb);

        // The tail scrolled into view.
        assert!(
            painted.contains("line 3"),
            "scrolled tail (line 3) should be visible: {painted}"
        );
        assert!(
            painted.contains("line 4"),
            "scrolled tail (line 4) should be visible: {painted}"
        );
        // The head scrolled out and must be clipped by the viewport.
        assert!(
            !painted.contains("line 0"),
            "scrolled-off head (line 0) should be clipped: {painted}"
        );
        assert!(
            !painted.contains("line 1"),
            "scrolled-off head (line 1) should be clipped: {painted}"
        );
        root.dispose();
    }

    /// A text-surface leaf with a large `scroll` value must clamp to the bottom
    /// of its content: only the last `visible_height` rows paint, even though
    /// the surface carries more. This is the layout-driven "stick to bottom"
    /// path the transcript relies on — the component passes a sentinel scroll
    /// and the paint path resolves the real window from taffy's height.
    #[test]
    fn text_surface_with_large_scroll_sticks_to_bottom() {
        use crate::producer::Plain;
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = create_root(|| {
            // A 4-tall viewport whose child leaf holds 5 lines and a sentinel
            // scroll (i32::MAX). Only lines 1..4 should paint; line 0 is above
            // the clamped window.
            let producer: Box<dyn CellProducer> = Box::new(Plain::new("line 0\nline 1\nline 2\nline 3\nline 4"));
            let view: View = tags::div()
                .width(10)
                .height(4)
                .overflow(taffy::style::Overflow::Hidden)
                .children(View::from_node(TuiNode::create_leaf_node(producer, i32::MAX)))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 12, 6), None);
        let painted = canvas_to_plain_text(&fb);
        assert!(
            painted.contains("line 4"),
            "stick-to-bottom must show the last line: {painted}"
        );
        assert!(
            painted.contains("line 3"),
            "stick-to-bottom must show the second-to-last line: {painted}"
        );
        assert!(
            !painted.contains("line 0"),
            "stick-to-bottom must clip the first line: {painted}"
        );
        root.dispose();
    }

    /// A `scroll` of 0 keeps the existing behaviour (paint from the top), so a
    /// surface taller than its viewport clips the BOTTOM, not the head. This
    /// guards the backward-compatibility of the scroll clamp.
    #[test]
    fn text_surface_with_zero_scroll_shows_from_top() {
        use crate::producer::Plain;
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let producer: Box<dyn CellProducer> = Box::new(Plain::new("line 0\nline 1\nline 2\nline 3\nline 4"));
            let view: View = tags::div()
                .width(10)
                .height(4)
                .overflow(taffy::style::Overflow::Hidden)
                .children(View::from_node(TuiNode::create_leaf_node(producer, 0)))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 12, 6), None);
        let painted = canvas_to_plain_text(&fb);
        assert!(
            painted.contains("line 0"),
            "scroll 0 shows the first line: {painted}"
        );
        assert!(
            !painted.contains("line 4"),
            "scroll 0 clips the last line: {painted}"
        );
        root.dispose();
    }

    #[test]
    fn focused_button_does_not_invert_child_text_by_default() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::button()
                .children(tags::div().children(tags::span().children("7")))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (_canvas, index) = render(&nodes, Rect::new(0, 0, 12, 5), None);
        let focused = index
            .focus_order
            .first()
            .copied()
            .expect("button should be focusable");
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 12, 5), Some(focused));

        let mut seven = None;
        for y in 0..fb.height() {
            for x in 0..fb.width() {
                if let Some(cell) = fb.cell(x as i32, y as i32)
                    && let Some(character) = &cell.glyph
                    && character.value == "7"
                {
                    seven = Some(character);
                }
            }
        }
        let seven = seven.expect("button child text should paint");

        assert!(
            !seven
                .style
                .add_modifier
                .contains(crate::text::Modifier::REVERSED),
            "focus should not force reversed-video text"
        );
        root.dispose();
    }

    fn canvas_to_plain_text(fb: &Framebuffer) -> String {
        // Delegates to the `Display` impl, which flattens the framebuffer to
        // unstyled plain text (one '\n'-terminated line per row).
        fb.to_string()
    }

    /// An absolutely-positioned element with a `background_color` must cover the
    /// text beneath it: the underlying characters are erased across the box's
    /// whole rect, not left showing through where the box's own text doesn't
    /// reach. This mirrors iocraft's `View::draw`, which `clear_text`s before
    /// painting the background. A `Color::Reset` background is the "cover with
    /// the terminal default colour" case used by the `overlap` example.
    #[test]
    fn absolute_background_color_covers_underlying_text() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::div()
                .children(vec![
                    View::from(tags::p().children("xxxxxxxxxx underlying")),
                    // Overlay is wider than its own text, so the cells past "fg"
                    // are covered only by the background -- if clear_text is
                    // skipped, "xxxxxxxxxx" shows through there.
                    View::from(
                        tags::div()
                            .position(taffy::style::Position::Absolute)
                            .top(0)
                            .left(0)
                            .width(10)
                            .height(1)
                            .background_color(crate::Color::Reset)
                            .children(tags::p().children("fg")),
                    ),
                ])
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render(&nodes, Rect::new(0, 0, 24, 3), None);
        let row0 = canvas_to_plain_text(&fb)
            .lines()
            .next()
            .unwrap()
            .to_owned();

        // The overlay's own text paints at the start...
        assert!(
            row0.starts_with("fg"),
            "overlay text should paint on top: {row0:?}"
        );
        // ...and the eight cells it does NOT reach (cols 2-9) must be blanked
        // by the background, not show the "xxxxxxxxxx" beneath.
        assert_eq!(
            &row0[..10],
            "fg        ",
            "background_color should erase the text it covers: {row0:?}"
        );
        assert!(
            !row0.contains("xxxxxxxx"),
            "covered text must not bleed through the background: {row0:?}"
        );
        root.dispose();
    }

    #[test]
    fn dynamic_wrapper_does_not_collapse_absolute_child_containing_block() {
        let mut nodes = Vec::new();

        let root = create_root(|| {
            let view: View = tags::div()
                .width(crate::style::Size::Percent(100.0))
                .height(crate::style::Size::Percent(100.0))
                .children((
                    tags::p().children("under"),
                    View::from_dynamic(|| {
                        tags::div()
                            .position(taffy::style::Position::Absolute)
                            .top(0)
                            .right(0)
                            .bottom(0)
                            .left(0)
                            .width(crate::style::Size::Percent(100.0))
                            .height(crate::style::Size::Percent(100.0))
                            .background_color(crate::Color::Reset)
                            .children(tags::p().children("overlay"))
                    }),
                ))
                .into();
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render(&nodes, Rect::new(0, 0, 20, 4), None);
        let painted = canvas_to_plain_text(&fb);

        assert!(
            painted
                .lines()
                .next()
                .unwrap_or_default()
                .starts_with("overlay"),
            "absolute overlay should paint from the parent origin: {painted}"
        );
        assert!(
            !painted.contains("under"),
            "overlay background should cover underlying content: {painted}"
        );
        root.dispose();
    }

    #[test]
    fn text_surface_layout_breaks_at_width() {
        use crate::producer::Plain;

        let rows = Plain::new("abcdef").render(3);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].glyph.as_ref().unwrap().value, "a");
        assert_eq!(rows[0][1].glyph.as_ref().unwrap().value, "b");
        assert_eq!(rows[0][2].glyph.as_ref().unwrap().value, "c");
        assert_eq!(rows[1][0].glyph.as_ref().unwrap().value, "d");
    }

    #[test]
    fn text_surface_layout_resolves_segment_style_patched_on_base() {
        use crate::producer::Lines;
        use crate::text::{Modifier, SpanStyle};

        // Lines with styled runs; style is per-glyph, not inherited from a
        // parent "base style" — each glyph carries its run's style directly.
        let rows = Lines::new(vec![vec![
            ("a".to_string(), SpanStyle::default()),
            (
                "b".to_string(),
                SpanStyle {
                    add_modifier: Modifier::BOLD,
                    ..SpanStyle::default()
                },
            ),
        ]])
        .render(10);

        assert_eq!(rows.len(), 1);
        assert!(
            !rows[0][0]
                .glyph
                .as_ref()
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            rows[0][1]
                .glyph
                .as_ref()
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn text_surface_measures_wrapped_height() {
        use crate::producer::Plain;
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let producer: Box<dyn CellProducer> = Box::new(Plain::new("abcdef"));
            let view: View = tags::div()
                .width(3)
                .children(View::from_node(TuiNode::create_leaf_node(producer, 0)))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (_canvas, _index) = render(&nodes, Rect::new(0, 0, 3, 10), None);
        root.dispose();
    }

    #[test]
    fn text_surface_paints_scroll_window_only() {
        use crate::producer::Plain;
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let producer: Box<dyn CellProducer> = Box::new(Plain::new("a\nb\nc\nd\ne"));
            let leaf = TuiNode::create_leaf_node(producer, 1);
            let view: View = tags::div()
                .width(5)
                .height(2)
                .overflow(taffy::style::Overflow::Hidden)
                .children(View::from_node(leaf))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 5, 2), None);
        let painted = canvas_to_plain_text(&fb);
        assert!(painted.contains('b'), "offset row 0 visible: {painted}");
        assert!(painted.contains('c'), "offset row 1 visible: {painted}");
        assert!(
            !painted.contains('a'),
            "scrolled-off head clipped: {painted}"
        );
        assert!(!painted.contains('d'), "below viewport clipped: {painted}");
        root.dispose();
    }

    #[test]
    fn text_surface_paints_per_segment_styles() {
        use crate::producer::Lines;
        use crate::text::SpanStyle;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let rows = Lines::new(vec![vec![
                ("plain ".to_string(), SpanStyle::default()),
                (
                    "bold".to_string(),
                    SpanStyle {
                        add_modifier: crate::text::Modifier::BOLD,
                        ..SpanStyle::default()
                    },
                ),
                (
                    " red".to_string(),
                    SpanStyle {
                        fg: Some(crate::Color::Red),
                        ..SpanStyle::default()
                    },
                ),
            ]]);
            let view: View = tags::div()
                .width(20)
                .children(View::leaf(Box::new(rows)))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 20, 1), None);
        // "plain " occupies cols 0..6, "bold" cols 6..10, " red" cols 10..14.
        let plain_cell = fb.cell(0, 0).unwrap().glyph.as_ref().unwrap();
        let bold_cell = fb.cell(6, 0).unwrap().glyph.as_ref().unwrap();
        let red_cell = fb.cell(10, 0).unwrap().glyph.as_ref().unwrap();
        assert_eq!(plain_cell.value, "p");
        assert!(
            !plain_cell
                .style
                .add_modifier
                .contains(crate::text::Modifier::BOLD)
        );
        assert!(
            bold_cell
                .style
                .add_modifier
                .contains(crate::text::Modifier::BOLD)
        );
        assert_eq!(red_cell.style.fg, Some(crate::Color::Red));
        root.dispose();
    }

    #[test]
    fn text_surface_places_ascii_after_cjk_by_display_width() {
        use crate::producer::Plain;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::div()
                .width(6)
                .children(View::leaf(Box::new(Plain::new("好XY"))))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 6, 1), None);

        assert_eq!(
            fb.cell(0, 0).unwrap().glyph.as_ref().unwrap().value,
            "好"
        );
        assert!(fb.cell(1, 0).unwrap().glyph.is_none());
        assert_eq!(
            fb.cell(2, 0).unwrap().glyph.as_ref().unwrap().value,
            "X"
        );
        assert_eq!(
            fb.cell(3, 0).unwrap().glyph.as_ref().unwrap().value,
            "Y"
        );
        root.dispose();
    }

    #[test]
    fn dim_and_crossed_out_builders_set_style_fields() {
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::div()
                .dim(true)
                .crossed_out(true)
                .underline_color(crate::Color::Cyan)
                .children("x")
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 1, 1), None);
        let cell = fb.cell(0, 0).unwrap().glyph.as_ref().unwrap();
        assert!(cell.style.add_modifier.contains(crate::text::Modifier::DIM));
        assert!(
            cell.style
                .add_modifier
                .contains(crate::text::Modifier::CROSSED_OUT)
        );
        root.dispose();
    }

    #[test]
    fn text_surface_wrap_carries_style_across_break() {
        use crate::producer::Plain;
        use crate::text::{Modifier, SpanStyle};
        // One styled string "ABCDEF" with BOLD, width 3 → two rows "ABC" / "DEF".
        // Both rows' graphemes must carry BOLD.
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::div()
                .width(3)
                .children(View::leaf(Box::new(Plain::styled(
                    "ABCDEF",
                    SpanStyle {
                        add_modifier: Modifier::BOLD,
                        ..SpanStyle::default()
                    },
                ))))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 3, 2), None);
        for y in 0..2 {
            for x in 0..3 {
                let cell = fb.cell(x as i32, y as i32).unwrap().glyph.as_ref().unwrap();
                assert!(
                    cell.style.add_modifier.contains(Modifier::BOLD),
                    "grapheme at ({x},{y}) should be bold: {:?}",
                    cell.style
                );
            }
        }
        root.dispose();
    }

    #[test]
    fn element_with_text_surface_child_is_text_leaf() {
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::button()
                .children(View::leaf(Box::new(crate::producer::Plain::new("hi"))))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 10, 1), None);
        let painted = canvas_to_plain_text(&fb);
        assert!(
            painted.contains("[ hi ]"),
            "button chrome should paint: {painted}"
        );
        root.dispose();
    }

    /// Reproduces the flown `overlay_layer` nesting: a full-screen Column root
    /// holding [grow-child, shrink-child, Dynamic(absolute full-bleed overlay)].
    /// The overlay's own content root is `width/height: Percent(100)`. The
    /// overlay must stretch to cover the whole screen, hiding the shrink-child's
    /// text beneath it. Regression for the btw overlay failing to cover.
    #[test]
    fn dynamic_wrapped_absolute_full_bleed_covers_sibling_when_root_is_column() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let view: View = tags::div()
                .flex_direction(FlexDirection::Column)
                .width(crate::style::Size::Percent(100.0))
                .height(crate::style::Size::Percent(100.0))
                .children((
                    // grow child (transcript)
                    tags::div()
                        .flex_grow(1.0)
                        .children(tags::p().children("GROW")),
                    // shrink child (prompt) — this must be hidden by the overlay
                    tags::div()
                        .flex_shrink(0.0)
                        .children(tags::p().children("SHOULD-BE-COVERED")),
                    // overlay_layer: a Dynamic wrapping an absolute full-bleed box
                    View::from_dynamic(|| {
                        View::from(
                            tags::div()
                                .position(taffy::style::Position::Absolute)
                                .top(0)
                                .right(0)
                                .bottom(0)
                                .left(0)
                                .background_color(crate::Color::Reset)
                                .children(
                                    tags::div()
                                        .flex_direction(FlexDirection::Column)
                                        .width(crate::style::Size::Percent(100.0))
                                        .height(crate::style::Size::Percent(100.0))
                                        .children(tags::p().children("OVERLAY")),
                                ),
                        )
                    }),
                ))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 40, 10), None);
        let painted = canvas_to_plain_text(&fb);

        assert!(
            !painted.contains("SHOULD-BE-COVERED"),
            "full-bleed overlay must cover the shrink sibling: {painted:?}"
        );
        root.dispose();
    }

    /// Regression for issue 3: a full-bleed overlay's content — a column of a
    /// growing transcript and a non-growing prompt — must anchor the prompt to
    /// the BOTTOM of the screen, not collapse both children to the top. Before
    /// the `extract_node` re-layout, taffy sized the overlay's content root to
    /// 0 (collapsed Dynamic containing block), so the prompt painted at row 0.
    #[test]
    fn full_bleed_overlay_column_anchors_prompt_to_bottom() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let view: View = tags::div()
                .flex_direction(FlexDirection::Column)
                .width(crate::style::Size::Percent(100.0))
                .height(crate::style::Size::Percent(100.0))
                .children((
                    tags::p().children("GROW"),
                    tags::p().children("SHRINK"),
                    View::from_dynamic(|| {
                        View::from(
                            tags::div()
                                .position(taffy::style::Position::Absolute)
                                .top(0)
                                .right(0)
                                .bottom(0)
                                .left(0)
                                .background_color(crate::Color::Reset)
                                .children(
                                    tags::div()
                                        .flex_direction(FlexDirection::Column)
                                        .width(crate::style::Size::Percent(100.0))
                                        .height(crate::style::Size::Percent(100.0))
                                        .children((
                                            // transcript grows to fill
                                            tags::div()
                                                .flex_grow(1.0)
                                                .children(tags::p().children("TOP")),
                                            // prompt stays at the bottom
                                            tags::div()
                                                .flex_shrink(0.0)
                                                .children(tags::p().children("BOTTOM-PROMPT")),
                                        )),
                                ),
                        )
                    }),
                ))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        // 10 rows: transcript fills 9, prompt on row 9 (the last).
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 40, 10), None);
        let painted = canvas_to_plain_text(&fb);
        let lines: Vec<&str> = painted.lines().collect();

        assert_eq!(lines.len(), 10);
        assert!(
            lines[9].contains("BOTTOM-PROMPT"),
            "prompt must anchor to the last row, got row 9 = {:?}; full painted:\n{painted}",
            lines[9]
        );
        assert!(
            !lines[0].contains("BOTTOM-PROMPT"),
            "prompt must not collapse to the top row"
        );
        root.dispose();
    }

    /// Regression for issue 2: an inset overlay (percentage insets on all four
    /// sides) nested in a collapsed Dynamic must keep its percentage margins —
    /// 1/8 inset resolves to real rows/cols, not 0 (which previously made the
    /// box fill the whole screen instead of sitting centered with a margin).
    #[test]
    fn inset_overlay_keeps_percentage_margins_in_collapsed_dynamic() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let view: View = tags::div()
                .flex_direction(FlexDirection::Column)
                .width(crate::style::Size::Percent(100.0))
                .height(crate::style::Size::Percent(100.0))
                .children((
                    tags::p().children("filler"),
                    View::from_dynamic(|| {
                        View::from(
                            tags::div()
                                .position(taffy::style::Position::Absolute)
                                // 1/8 inset on every side, like the /model picker.
                                .top(crate::style::Inset::Percent(12.5))
                                .right(crate::style::Inset::Percent(12.5))
                                .bottom(crate::style::Inset::Percent(12.5))
                                .left(crate::style::Inset::Percent(12.5))
                                .background_color(crate::Color::Reset)
                                .border_style(BorderStyle::Round)
                                .border_color(crate::Color::Cyan)
                                .children(tags::p().children("PICK")),
                        )
                    }),
                ))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        // 80×24 → 12.5% = 10 cols / 3 rows of margin.
        let (fb, _index) = render(&nodes, Rect::new(0, 0, 80, 24), None);
        let painted = canvas_to_plain_text(&fb);

        // The top border (╭) sits at row 3 (12.5% of 24 ≈ 3) and column 10
        // (12.5% of 80 = 10) — not at (0,0), which would mean the percentage
        // inset collapsed to 0.
        let border_row = painted.lines().position(|l| l.contains('╭'));
        assert_eq!(
            border_row,
            Some(3),
            "1/8 vertical inset should put the top border at row 3; full painted:\n{painted}"
        );
        let border_line = painted.lines().nth(3).unwrap_or_default();
        let border_col = border_line.find('╭');
        assert_eq!(
            border_col,
            Some(10),
            "1/8 horizontal inset should put the left border at col 10, got row 3: {border_line:?}"
        );
        root.dispose();
    }
}
