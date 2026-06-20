//! Taffy layout and terminal painting onto the self-built [`Canvas`].
//!
//! This is the crossterm-without-ratatui paint path (ADR-0024 §10–§12):
//! taffy computes layout, a `Canvas` holds painted output, and the render
//! driver diffs the `Canvas` between frames. The legacy ratatui `Buffer` and
//! `Rect` are gone.

use std::borrow::Cow;
use std::collections::HashMap;

use taffy::prelude::{AvailableSpace, Dimension, FlexDirection, Size};
use taffy::{NodeId as TaffyNodeId, TaffyTree};
use unicode_width::UnicodeWidthStr;

use crate::attributes::resolve_style;
use crate::canvas::{Canvas, Rect};
use crate::node::{NodeId, TuiNode};
use crate::style::{Edges, Style, TextStyle};
use crate::text::SpanStyle;

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

#[derive(Debug)]
struct BuiltNode {
    runtime_id: NodeId,
    taffy_id: TaffyNodeId,
    parent: Option<NodeId>,
    tag: Option<String>,
    text: String,
    style: Style,
    focusable: bool,
    text_leaf: bool,
    children: Vec<BuiltNode>,
}

#[derive(Debug, Clone)]
struct Measure {
    text: String,
}

#[derive(Debug)]
struct PaintNode {
    tag: Option<String>,
    rect: PaintRect,
    text: String,
    style: Style,
    text_leaf: bool,
    children: Vec<PaintNode>,
}

/// A rectangle in terminal-cell space, used inside the paint pipeline. Unlike
/// the public [`Rect`] (whose coordinates are `u16`), positions here are
/// `i32` so a node may sit at a negative coordinate — this is what makes
/// scrolling work: an in-flow child of an `overflow: hidden` viewport is
/// translated up by a negative `margin_top`, landing above the viewport's
/// origin, and the paint pipeline clips the off-screen portion. Width/height
/// stay `u16` (sizes are never negative).
///
/// The previous implementation clamped positions to `>= 0` in `rect_from_layout`
/// (because `Rect` is `u16`), which silently pinned any scrolled content to the
/// top of its viewport — `margin_top = -10` painted as if it were `0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaintRect {
    x: i32,
    y: i32,
    width: u16,
    height: u16,
}

impl PaintRect {
    const ZERO: PaintRect = PaintRect {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    fn new(x: i32, y: i32, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Exclusive right edge, saturating at `i32::MAX`.
    fn right(self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// Exclusive bottom edge, saturating at `i32::MAX`.
    fn bottom(self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// Convert to the public [`Rect`], clamping positions into the `u16`
    /// range. Used only when handing a paint rect to canvas cell-writing
    /// methods (which take `u16` coords) — by then the paint pipeline has
    /// already clipped, so any still-negative position is genuinely off-screen
    /// and the clamp is a no-op for visible cells.
    fn to_canvas_rect(self) -> Rect {
        Rect::new(
            self.x.clamp(0, u16::MAX as i32) as u16,
            self.y.clamp(0, u16::MAX as i32) as u16,
            self.width,
            self.height,
        )
    }
}

impl From<Rect> for PaintRect {
    fn from(rect: Rect) -> Self {
        PaintRect::new(rect.x as i32, rect.y as i32, rect.width, rect.height)
    }
}

/// Lay out `nodes` into `area` and paint the result into a fresh [`Canvas`].
/// Also returns a [`RuntimeIndex`] for hit testing, focus, and event bubbling.
pub(crate) fn render(
    nodes: &[TuiNode],
    area: Rect,
    _focused: Option<NodeId>,
) -> (Canvas, RuntimeIndex) {
    let mut taffy = TaffyTree::<Measure>::new();
    let mut built_roots = Vec::new();
    for node in nodes {
        if let Some(built) = build_node(&mut taffy, node, None) {
            built_roots.push(built);
        }
    }

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
            context.map_or(Size::ZERO, |context| {
                measure_text(&context.text, known, available)
            })
        },
    );

    let mut index = RuntimeIndex::default();
    let area_paint: PaintRect = area.into();
    let paint_nodes = built_roots
        .iter()
        .map(|built| extract_node(&taffy, built, area_paint, &mut index))
        .collect::<Vec<_>>();

    let mut canvas = Canvas::empty(area);
    // Text paint inherits from the root; layout/border/background do not.
    let root_text = TextStyle::default();
    for node in &paint_nodes {
        paint_node(&mut canvas, node, root_text, area_paint, area_paint);
    }

    (canvas, index)
}

fn build_node(
    tree: &mut TaffyTree<Measure>,
    node: &TuiNode,
    parent: Option<NodeId>,
) -> Option<BuiltNode> {
    match node {
        TuiNode::Marker { .. } => None,
        TuiNode::TextStatic { id, text } => {
            let text = text.to_string();
            let taffy_id = tree
                .new_leaf_with_context(
                    taffy::style::Style::default(),
                    Measure { text: text.clone() },
                )
                .expect("create text node");
            Some(BuiltNode {
                runtime_id: *id,
                taffy_id,
                parent,
                tag: None,
                text,
                style: Style::default(),
                focusable: false,
                text_leaf: false,
                children: Vec::new(),
            })
        }
        TuiNode::TextDynamic { id, text } => {
            let text = text.borrow().clone();
            let taffy_id = tree
                .new_leaf_with_context(
                    taffy::style::Style::default(),
                    Measure { text: text.clone() },
                )
                .expect("create dynamic text node");
            Some(BuiltNode {
                runtime_id: *id,
                taffy_id,
                parent,
                tag: None,
                text,
                style: Style::default(),
                focusable: false,
                text_leaf: false,
                children: Vec::new(),
            })
        }
        TuiNode::Dynamic { id, view } => {
            let children = view
                .borrow()
                .nodes
                .iter()
                .filter_map(|child| build_node(tree, child, Some(*id)))
                .collect::<Vec<_>>();
            let child_ids = children
                .iter()
                .map(|child| child.taffy_id)
                .collect::<Vec<_>>();
            let taffy_id = tree
                .new_with_children(default_container_style(), &child_ids)
                .expect("create dynamic container");
            Some(BuiltNode {
                runtime_id: *id,
                taffy_id,
                parent,
                tag: None,
                text: String::new(),
                style: Style::default(),
                focusable: false,
                text_leaf: false,
                children,
            })
        }
        TuiNode::Element(element) => {
            let tag_name = element.tag.to_string();
            let style = resolve_style(&element.style_props, default_style_for_tag(&element.tag));
            let leaf = is_text_leaf(&element.tag, &element.children);
            let text = element_text(node);
            let display_text = display_text_for_tag(Some(&tag_name), &text).into_owned();
            let focusable = is_focusable(node);
            if leaf {
                let taffy_id = tree
                    .new_leaf_with_context(style.to_taffy(), Measure { text: display_text })
                    .expect("create element leaf");
                Some(BuiltNode {
                    runtime_id: element.id,
                    taffy_id,
                    parent,
                    tag: Some(tag_name),
                    text,
                    style,
                    focusable,
                    text_leaf: true,
                    children: Vec::new(),
                })
            } else {
                let built_children = element
                    .children
                    .iter()
                    .filter_map(|child| build_node(tree, child, Some(element.id)))
                    .collect::<Vec<_>>();
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
                    text,
                    style,
                    focusable,
                    text_leaf: false,
                    children: built_children,
                })
            }
        }
    }
}

fn extract_node(
    tree: &TaffyTree<Measure>,
    built: &BuiltNode,
    parent_rect: PaintRect,
    index: &mut RuntimeIndex,
) -> PaintNode {
    let layout = tree.layout(built.taffy_id).expect("taffy layout");
    let rect = rect_from_layout(layout, parent_rect);
    // The runtime index (hit testing, focus) only cares about on-screen
    // elements, so it stores the public u16 Rect clamped to the screen — a
    // node scrolled above the viewport is not clickable.
    index.nodes.insert(
        built.runtime_id,
        RuntimeNode {
            parent: built.parent,
            rect: rect.to_canvas_rect(),
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
        .map(|child| extract_node(tree, child, rect, index))
        .collect();
    PaintNode {
        tag: built.tag.clone(),
        rect,
        text: built.text.clone(),
        style: built.style.clone(),
        text_leaf: built.text_leaf,
        children,
    }
}

/// Paint a node into the canvas: background, border, then text, recursing with
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
    canvas: &mut Canvas,
    node: &PaintNode,
    parent_text: TextStyle,
    clip: PaintRect,
    screen: PaintRect,
) {
    let has_size = node.rect.width != 0 && node.rect.height != 0;

    // Resolve the inheritable text style: this node's text-paint fields inherit
    // from the parent's.
    let text = node.style.text_style().inherit(parent_text);
    if has_size {
        if let Some(bg) = node.style.background_color
            && let Some(rect) = intersect_rect(node.rect, clip)
        {
            // A background is opaque: it must cover whatever was painted into
            // these cells earlier (e.g. text beneath an absolutely-positioned
            // box), so erase the characters first, then fill the background.
            // Mirrors iocraft's `View::draw`, which `clear_text`s before
            // `set_background_color`. This node's own border/text are drawn
            // afterwards and so still show on top.
            let canvas_rect = rect.to_canvas_rect();
            canvas.clear_text(canvas_rect);
            canvas.set_background_color(canvas_rect, bg);
        }

        if let Some(border_chars) = node.style.border_style.border_characters() {
            // The border uses only its own color, not inherited text paint.
            let border_style = SpanStyle {
                fg: node.style.border_color,
                ..SpanStyle::default()
            };
            paint_border_clipped(
                canvas,
                node.rect,
                border_chars,
                node.style.border_edges,
                border_style,
                clip,
            );
        }

        // Only leaf/inline tags emit text; containers delegate to their children.
        if node.text_leaf {
            let display_text = display_text_for_tag(node.tag.as_deref(), &node.text);
            let span_style = SpanStyle::from(text);
            paint_text_clipped(canvas, node.rect, &display_text, span_style, clip);
        }
    }

    let child_clip = if node.style.overflow == taffy::style::Overflow::Visible {
        clip
    } else {
        intersect_rect(clip, content_rect(node)).unwrap_or(PaintRect::ZERO)
    };
    for child in &node.children {
        // An absolutely-positioned child lives in its containing block, not this
        // node's flex flow, so clip it to the screen rather than the parent.
        let bounds = if child.style.position == taffy::style::Position::Absolute {
            screen
        } else {
            child_clip
        };
        paint_node(canvas, child, text, bounds, screen);
    }
}

fn paint_border_clipped(
    canvas: &mut Canvas,
    rect: PaintRect,
    chars: crate::style::BorderCharacters,
    edges: Option<Edges>,
    style: SpanStyle,
    clip: PaintRect,
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
                canvas,
                PaintRect::new(rect.x, rect.y, 1, 1),
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
            canvas,
            PaintRect::new(rect.x + (left_border_size as i32), rect.y, width, 1),
            &chars.top.to_string().repeat(width as usize),
            style,
            clip,
        );
        if edges.contains(Edges::RIGHT) {
            paint_text_clipped_raw(
                canvas,
                PaintRect::new(right, rect.y, 1, 1),
                &chars.top_right.to_string(),
                style,
                clip,
            );
        }
    }
    for y in rect.y + 1..bottom {
        if edges.contains(Edges::LEFT) {
            paint_text_clipped_raw(
                canvas,
                PaintRect::new(rect.x, y, 1, 1),
                &chars.left.to_string(),
                style,
                clip,
            );
        }
        if edges.contains(Edges::RIGHT) {
            paint_text_clipped_raw(
                canvas,
                PaintRect::new(right, y, 1, 1),
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
                canvas,
                PaintRect::new(rect.x, bottom, 1, 1),
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
            canvas,
            PaintRect::new(rect.x + (left_border_size as i32), bottom, width, 1),
            &chars.bottom.to_string().repeat(width as usize),
            style,
            clip,
        );
        if edges.contains(Edges::RIGHT) {
            paint_text_clipped_raw(
                canvas,
                PaintRect::new(right, bottom, 1, 1),
                &chars.bottom_right.to_string(),
                style,
                clip,
            );
        }
    }
}

fn paint_text_clipped(
    canvas: &mut Canvas,
    rect: PaintRect,
    text: &str,
    style: SpanStyle,
    clip: PaintRect,
) {
    paint_text_clipped_raw(canvas, rect, text, style, clip);
}

fn paint_text_clipped_raw(
    canvas: &mut Canvas,
    rect: PaintRect,
    text: &str,
    style: SpanStyle,
    clip: PaintRect,
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
        if contains_point_i32(clip, x, y) {
            // `contains_point_i32` against a screen-space clip guarantees x,y >= 0,
            // but clamp defensively so a stray negative can never wrap into a huge u16.
            canvas.set_text(
                Rect::new(x.max(0) as u16, y.max(0) as u16, cw as u16, 1),
                &ch.to_string(),
                style,
            );
        }
        col += cw;
    }
}

fn content_rect(node: &PaintNode) -> PaintRect {
    if node.style.border_style.is_none() {
        return node.rect;
    }
    let edges = node.style.border_edges.unwrap_or(Edges::all());
    let left = u16::from(edges.contains(Edges::LEFT));
    let right = u16::from(edges.contains(Edges::RIGHT));
    let top = u16::from(edges.contains(Edges::TOP));
    let bottom = u16::from(edges.contains(Edges::BOTTOM));
    PaintRect::new(
        node.rect.x + (left as i32),
        node.rect.y + (top as i32),
        node.rect.width.saturating_sub(left).saturating_sub(right),
        node.rect.height.saturating_sub(top).saturating_sub(bottom),
    )
}

fn intersect_rect(a: PaintRect, b: PaintRect) -> Option<PaintRect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (x < right && y < bottom).then(|| {
        PaintRect::new(
            x,
            y,
            (right - x).clamp(0, u16::MAX as i32) as u16,
            (bottom - y).clamp(0, u16::MAX as i32) as u16,
        )
    })
}

fn measure_text(
    text: &str,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
) -> Size<f32> {
    let raw_width = text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0) as f32;
    let available_width = match available.width {
        AvailableSpace::Definite(width) => width.max(1.0),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => raw_width.max(1.0),
    };
    let width = known
        .width
        .unwrap_or(raw_width.min(available_width).max(1.0));
    let height = known.height.unwrap_or_else(|| {
        let lines = text.lines().count().max(1) as f32;
        let wrapped = if width > 0.0 {
            (raw_width / width).ceil().max(1.0)
        } else {
            1.0
        };
        lines.max(wrapped)
    });
    Size { width, height }
}

fn rect_from_layout(layout: &taffy::Layout, parent_rect: PaintRect) -> PaintRect {
    let x = parent_rect.x as f32 + layout.location.x;
    let y = parent_rect.y as f32 + layout.location.y;
    PaintRect::new(
        x.round() as i32,
        y.round() as i32,
        layout.size.width.round().max(0.0) as u16,
        layout.size.height.round().max(0.0) as u16,
    )
}

fn default_container_style() -> taffy::style::Style {
    taffy::style::Style {
        flex_direction: FlexDirection::Column,
        ..taffy::style::Style::default()
    }
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
    matches!(tag, "span" | "p" | "input")
        || children.iter().all(|child| {
            matches!(
                child,
                TuiNode::TextStatic { .. } | TuiNode::TextDynamic { .. } | TuiNode::Marker { .. }
            )
        })
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
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

/// Paint-pipeline variant of [`contains_point`]: the clip rect and the candidate
/// coordinates may be negative (a node scrolled above the viewport's origin), so
/// the test runs in `i32`. A point with `x < 0` or `y < 0` is off the left/top
/// edge of the terminal and never visible.
fn contains_point_i32(rect: PaintRect, x: i32, y: i32) -> bool {
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

        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 12, 5), None);
        let painted = canvas_to_plain_text(&canvas);

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
        let mut canvas = Canvas::empty(Rect::new(0, 0, 6, 2));
        paint_border_clipped(
            &mut canvas,
            Rect::new(0, 0, 6, 2).into(),
            BorderCharacters {
                top: '▁',
                ..Default::default()
            },
            Some(Edges::TOP),
            SpanStyle::default(),
            Rect::new(0, 0, 6, 2).into(),
        );

        let painted = canvas_to_plain_text(&canvas);
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

        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 20, 8), None);
        let painted = canvas_to_plain_text(&canvas);

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
    /// child's TAIL (it scrolled up) and clip its head. Before the `PaintRect`
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

        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 20, 8), None);
        let painted = canvas_to_plain_text(&canvas);

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
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 12, 5), Some(focused));

        let mut seven = None;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                if let Some(cell) = canvas.cell(x, y)
                    && let Some(character) = &cell.character
                    && character.value == "7"
                {
                    seven = Some(character);
                }
            }
        }
        let seven = seven.expect("button child text should paint");

        assert!(
            !seven.style.add_modifier.contains(crate::text::Modifier::REVERSED),
            "focus should not force reversed-video text"
        );
        root.dispose();
    }

    fn canvas_to_plain_text(canvas: &Canvas) -> String {
        let mut out = String::new();
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                if let Some(cell) = canvas.cell(x, y)
                    && let Some(ch) = &cell.character
                {
                    out.push_str(&ch.value);
                } else {
                    out.push(' ');
                }
            }
            out.push('\n');
        }
        out
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

        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 24, 3), None);
        let row0 = canvas_to_plain_text(&canvas)
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
}
