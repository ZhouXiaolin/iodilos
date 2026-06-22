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

use crate::attributes::resolve_style;
use crate::canvas::{Canvas, Rect};
use crate::node::{NodeId, TuiNode};
use crate::style::{Edges, Inset, Style};
use crate::surface::TextSurface;
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
    style: Style,
    focusable: bool,
    children: Vec<BuiltNode>,
    surface: Option<(TextSurface, i32)>,
}

#[derive(Debug, Clone)]
enum Measure {
    Surface { surface: TextSurface },
}

#[derive(Debug)]
struct PaintNode {
    rect: PaintRect,
    style: Style,
    children: Vec<PaintNode>,
    surface: Option<(TextSurface, i32)>,
}

impl PaintNode {
    /// Recompute this node's `rect` to `new_rect` and re-layout the subtree
    /// beneath it against that corrected size.
    ///
    /// Used for an absolutely-positioned overlay box whose `rect` had to be
    /// re-derived from its real containing block (the screen) because taffy
    /// sized it against a collapsed `Dynamic` wrapper (height 0). Taffy laid the
    /// whole subtree out against that zero-height block, so percentage sizes
    /// (`height: 100%`) collapsed to 0 and flex distribution is wrong. This
    /// re-runs a flex pass over the subtree using `new_rect`'s content box:
    /// each in-flow child's main-axis basis is its explicit length/percent or —
    /// for auto-sized leaves — the measured content height at the available
    /// width; remaining space is split by `flex_grow` (overflow shrunk by
    /// `flex_shrink`). Absolute descendants resolve against this node's rect.
    fn recompute_rect(&mut self, new_rect: PaintRect) {
        self.rect = new_rect;
        let content = content_box(self.rect, &self.style);

        let (abs, in_flow): (Vec<usize>, Vec<usize>) = (0..self.children.len())
            .partition(|i| self.children[*i].style.position == taffy::style::Position::Absolute);

        let is_row = self.style.flex_direction == FlexDirection::Row;
        let total = if is_row { content.width } else { content.height } as f32;
        let basis_of = |c: &PaintNode| {
            if is_row {
                basis_main(c, total, true, content)
            } else {
                basis_main(c, total, false, content)
            }
        };
        let (sizes, _rem) = flex_distribute(&self.children, &in_flow, total, basis_of);

        let mut cursor = if is_row { content.x } else { content.y } as f32;
        for &i in &in_flow {
            let main = (sizes[&i]).max(0.0).round() as u16;
            if is_row {
                let cross = resolve_cross(&self.children[i], false, content);
                let r = PaintRect::new(cursor.round() as i32, content.y, main, cross);
                self.children[i].recompute_rect(r);
            } else {
                let cross = resolve_cross(&self.children[i], true, content);
                let r = PaintRect::new(content.x, cursor.round() as i32, cross, main);
                self.children[i].recompute_rect(r);
            }
            cursor += main as f32;
        }

        for &i in &abs {
            let r = absolute_rect_from_insets(&self.children[i].style, self.rect);
            self.children[i].recompute_rect(r);
        }
    }
}

/// A node's content box: border box minus the drawn border edges.
fn content_box(rect: PaintRect, style: &Style) -> PaintRect {
    if style.border_style.is_none() {
        return rect;
    }
    let edges = style.border_edges.unwrap_or(Edges::all());
    let left = u16::from(edges.contains(Edges::LEFT));
    let right = u16::from(edges.contains(Edges::RIGHT));
    let top = u16::from(edges.contains(Edges::TOP));
    let bottom = u16::from(edges.contains(Edges::BOTTOM));
    PaintRect::new(
        rect.x + (left as i32),
        rect.y + (top as i32),
        rect.width.saturating_sub(left).saturating_sub(right),
        rect.height.saturating_sub(top).saturating_sub(bottom),
    )
}

/// An in-flow child's main-axis basis before grow/shrink: an explicit
/// length/percent wins; otherwise an auto-sized leaf is measured at the
/// available cross size (column→width, row→height); otherwise 0.
fn basis_main(child: &PaintNode, container: f32, row: bool, content: PaintRect) -> f32 {
    let size = if row { child.style.width } else { child.style.height };
    match size {
        crate::style::Size::Length(v) => v as f32,
        crate::style::Size::Percent(p) => (p / 100.0) * container,
        _ => measure_auto(child, row, content),
    }
}

/// Cross-axis size of an in-flow child: explicit length/percent wins; otherwise
/// for a column the child fills the content width, for a row the content
/// height. (`align_items: stretch` is the default we support here.)
fn resolve_cross(child: &PaintNode, column: bool, content: PaintRect) -> u16 {
    let size = if column { child.style.width } else { child.style.height };
    let container = if column { content.width } else { content.height } as f32;
    match size {
        crate::style::Size::Length(v) => v as u16,
        crate::style::Size::Percent(p) => ((p / 100.0) * container).round() as u16,
        _ => {
            // Stretch to the content cross axis, but not below the leaf's own
            // measured minimum (a single-line text leaf is 1 tall).
            let measured = measure_auto(child, !column, content).max(1.0) as u16;
            if column {
                content.width.max(measured)
            } else {
                content.height.max(measured)
            }
        }
    }
}

/// Measured content main-size for an auto-sized node at the available cross
/// size (column→width, row→height). A leaf with a surface is measured
/// directly; a container recursively sums its in-flow children's measured
/// main-sizes (plus its own padding/gap), so an auto-height prompt wrapper
/// around a text leaf resolves to the leaf's height, not 0.
fn measure_auto(node: &PaintNode, row: bool, content: PaintRect) -> f32 {
    if let Some((surface, _)) = &node.surface {
        let width = if row { content.height } else { content.width } as usize;
        if row {
            return surface.max_width().max(1) as f32;
        }
        return surface.layout(width.max(1), SpanStyle::default()).height() as f32;
    }
    // Container: sum in-flow children's measured main sizes + padding + gaps.
    let inner = content_box(content, &node.style);
    let cross_size = if row { inner.height } else { inner.width } as usize;
    let gap = match node.style.gap {
        crate::style::Gap::Unset => {
            if row { node.style.row_gap } else { node.style.column_gap }
        }
        other => other,
    };
    let gap_px = match gap {
        crate::style::Gap::Length(v) => v as f32,
        crate::style::Gap::Percent(p) => (p / 100.0) * cross_size as f32,
        _ => 0.0,
    };
    let is_row = node.style.flex_direction == FlexDirection::Row;
    let mut total = 0.0f32;
    let mut count = 0u32;
    for child in &node.children {
        if child.style.position == taffy::style::Position::Absolute {
            continue;
        }
        let basis = if is_row == row {
            // Same axis: measure the child's main size at the cross content size.
            basis_main(child, cross_size as f32, row, inner)
        } else {
            // Cross axis: the child's measured cross size contributes to THIS
            // node's main size only via its own main layout; approximate with
            // the child's auto measure on this node's main axis.
            measure_auto(child, row, inner)
        };
        total += basis;
        count += 1;
    }
    if count > 1 {
        total += gap_px * (count - 1) as f32;
    }
    total
}

/// Resolve an in-flow set of children's main-axis sizes by flexbox: each gets
/// its basis, then leftover space is grown (or overflow shrunk).
fn flex_distribute(
    children: &[PaintNode],
    in_flow: &[usize],
    total: f32,
    basis_of: impl Fn(&PaintNode) -> f32,
) -> (std::collections::HashMap<usize, f32>, f32) {
    let mut sizes: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();
    let mut remaining = total;
    let mut total_grow = 0.0f32;
    for &i in in_flow {
        let basis = basis_of(&children[i]).min(remaining.max(0.0));
        sizes.insert(i, basis);
        remaining -= basis;
        total_grow += children[i].style.flex_grow;
    }
    if total_grow > 0.0 && remaining > 0.0 {
        for &i in in_flow {
            let frac = children[i].style.flex_grow / total_grow;
            *sizes.get_mut(&i).unwrap() += frac * remaining;
        }
        remaining = 0.0;
    } else if remaining < 0.0 {
        let total_shrink: f32 = in_flow
            .iter()
            .map(|&i| children[i].style.flex_shrink.unwrap_or(1.0))
            .sum();
        if total_shrink > 0.0 {
            for &i in in_flow {
                let s = children[i].style.flex_shrink.unwrap_or(1.0) / total_shrink;
                *sizes.get_mut(&i).unwrap() += s * remaining;
            }
        }
        remaining = 0.0;
    }
    (sizes, remaining)
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
            context.map_or(Size::ZERO, |ctx| match ctx {
                Measure::Surface { surface } => measure_surface(surface, known, available),
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
    let root_text = SpanStyle::default();
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
        TuiNode::TextSurface {
            id,
            surface,
            scroll,
        } => {
            let surface_snapshot = surface.borrow().clone();
            let scroll_value = *scroll.borrow();
            let taffy_id = tree
                .new_leaf_with_context(
                    taffy::style::Style::default(),
                    Measure::Surface {
                        surface: surface_snapshot.clone(),
                    },
                )
                .expect("create text surface leaf");
            Some(BuiltNode {
                runtime_id: *id,
                taffy_id,
                parent,
                tag: None,
                style: Style::default(),
                focusable: false,
                children: Vec::new(),
                surface: Some((surface_snapshot, scroll_value)),
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
                style: Style::default(),
                focusable: false,
                children,
                surface: None,
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
                let surface = TextSurface::from_text(display_text);
                let taffy_id = tree
                    .new_leaf_with_context(
                        style.to_taffy(),
                        Measure::Surface {
                            surface: surface.clone(),
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
                    surface: Some((surface, 0)),
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
                    style,
                    focusable,
                    children: built_children,
                    surface: None,
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
        .map(|child| {
            let (child_parent, child_rect_override) = if built.tag.is_none()
                && child.style.position == taffy::style::Position::Absolute
            {
                // A `Dynamic` wrapper (no tag) collapses to zero height when it
                // is a flex item after a non-growing sibling, and taffy makes
                // the Dynamic the containing block for its absolute child — so
                // taffy sizes the child to 0. Reparent the child to the screen
                // (its true containing block) and re-derive its rect from the
                // containing block minus the child's own insets, instead of the
                // (collapsed) size taffy computed.
                let containing = parent_rect;
                let r = absolute_rect_from_insets(&child.style, containing);
                (containing, Some(r))
            } else {
                (rect, None)
            };
            let mut paint = extract_node(tree, child, child_parent, index);
            if let Some(r) = child_rect_override {
                paint.rect = r;
                // Keep the runtime index in sync so hit testing/focus use the
                // corrected on-screen rect, not the collapsed taffy size.
                index.nodes.insert(child.runtime_id, RuntimeNode {
                    parent: child.parent,
                    rect: r.to_canvas_rect(),
                    tag: child.tag.clone(),
                });
                // The child's subtree was laid out by taffy against a
                // zero-height containing block, so every descendant's size is
                // wrong. Re-layout the subtree against the corrected rect so a
                // `height: 100%` content root fills the overlay and its flex
                // children anchor to the bottom instead of collapsing to 0.
                paint.recompute_rect(r);
            }
            paint
        })
        .collect();
    PaintNode {
        rect,
        style: built.style.clone(),
        children,
        surface: built.surface.clone(),
    }
}

/// Resolve an absolutely-positioned node's rect from a containing block and the
/// node's own insets (`top`/`right`/`bottom`/`left`, falling back to `inset`).
/// Used when the real containing block (the screen) differs from the one taffy
/// resolved (a collapsed wrapper), so the size can't be trusted from layout.
///
/// Horizontal insets (`left`/`right`) resolve percentages against the
/// containing block's width; vertical ones (`top`/`bottom`) against its height
/// — matching CSS and `LengthPercentage::resolve` in taffy.
fn absolute_rect_from_insets(style: &Style, containing: PaintRect) -> PaintRect {
    // Mirrors `Inset::or` (private in style.rs): `Unset` falls back to the
    // aggregate `inset`, otherwise the per-side value wins. `Auto` on an
    // absolutely-positioned edge means "not pinned on this side" (no offset),
    // so it resolves to 0 here.
    let pct_of = |p: f32, axis: u16| -> i32 { ((p / 100.0) * axis as f32).round() as i32 };
    let resolve = |side: Inset, fallback: Inset, axis: u16| -> i32 {
        let resolved = match side {
            Inset::Unset => fallback,
            other => other,
        };
        match resolved {
            Inset::Length(v) => v as i32,
            Inset::Percent(p) => pct_of(p, axis),
            Inset::Unset | Inset::Auto => 0,
        }
    };
    let top = resolve(style.top, style.inset, containing.height);
    let bottom = resolve(style.bottom, style.inset, containing.height);
    let left = resolve(style.left, style.inset, containing.width);
    let right = resolve(style.right, style.inset, containing.width);
    let x = containing.x + left;
    let y = containing.y + top;
    let width = containing.width.saturating_sub((left + right).max(0) as u16);
    let height = containing.height.saturating_sub((top + bottom).max(0) as u16);
    PaintRect::new(x, y, width, height)
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
    parent_text: SpanStyle,
    clip: PaintRect,
    screen: PaintRect,
) {
    let has_size = node.rect.width != 0 && node.rect.height != 0;

    // Resolve the inheritable text style: this node's text-paint fields inherit
    // from the parent's.
    let text = parent_text.patch(node.style.text_span_style());
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

        if let Some((surface, scroll)) = &node.surface {
            let width = node.rect.width as usize;
            let layout = surface.layout(width, text);
            let clip_rect = intersect_rect(node.rect, clip).unwrap_or(PaintRect::ZERO);
            // Wipe any stale characters from the previous frame inside this
            // node's visible rect BEFORE painting the new rows. Without this, a
            // text surface that does not cover every cell of its rect (e.g. a
            // manually-scrolled transcript pre-sliced to `viewport_rows`) would
            // leave old characters in the canvas; the diff path then either
            // short-circuits on equal cells or keeps the previous frame's
            // inline-code background visible, producing the "bg shifts on
            // scroll" artefact. `clear_text` resets the canvas cells to their
            // default character (None), so the diff emits a space and a bg
            // reset for those cells.
            if clip_rect.width > 0 && clip_rect.height > 0 {
                canvas.clear_text(clip_rect.to_canvas_rect());
            }
            // The on-screen window for this node is its clipped rect. The node's
            // own `rect.height` is its *natural* content height (taffy does not
            // shrink a child of an `overflow: hidden` parent), so the visible
            // height is the clipped rect's height. Clamp `scroll` to
            // `[0, total - visible_height]` so a large value means "stick to
            // bottom": the caller passes a sentinel (e.g. `i32::MAX`) and the
            // last `visible_height` rows land inside the clip without the caller
            // having to know the viewport height. A scroll of 0 (and any in-range
            // value) behaves exactly as before — `i < scroll` skips head rows and
            // the clip_rect clips the tail.
            let visible_height = clip_rect.height as usize;
            let total = layout.rows().len();
            let max_scroll = total.saturating_sub(visible_height);
            let scroll = (*scroll).max(0) as usize;
            let scroll = scroll.min(max_scroll);
            for (i, row) in layout.rows().iter().enumerate() {
                if i < scroll {
                    continue;
                }
                let y = node.rect.y + (i - scroll) as i32;
                if y < clip_rect.y || y >= clip_rect.bottom() {
                    continue;
                }
                let segs: Vec<(&str, SpanStyle)> = row
                    .segments
                    .iter()
                    .map(|segment| (segment.content.as_str(), segment.style))
                    .collect();
                canvas.set_segments(clip_rect.to_canvas_rect(), y as u16, &segs);
            }
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

fn measure_surface(
    surface: &TextSurface,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
) -> Size<f32> {
    let raw_width = surface.max_width() as f32;
    let available_width = match available.width {
        AvailableSpace::Definite(w) => w.max(1.0),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => raw_width.max(1.0),
    };
    let width = known
        .width
        .unwrap_or(raw_width.min(available_width).max(1.0));
    let height = known.height.unwrap_or_else(|| {
        surface
            .layout(width as usize, SpanStyle::default())
            .height() as f32
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
        || (tag == "button"
            && children
                .iter()
                .all(|child| matches!(child, TuiNode::TextSurface { .. } | TuiNode::Marker { .. })))
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

    /// A text-surface leaf with a large `scroll` value must clamp to the bottom
    /// of its content: only the last `visible_height` rows paint, even though
    /// the surface carries more. This is the layout-driven "stick to bottom"
    /// path the transcript relies on — the component passes a sentinel scroll
    /// and the paint path resolves the real window from taffy's height.
    #[test]
    fn text_surface_with_large_scroll_sticks_to_bottom() {
        use crate::surface::TextSurface;
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = create_root(|| {
            // A 4-tall viewport whose child surface holds 5 lines and a sentinel
            // scroll (i32::MAX). Only lines 1..4 should paint; line 0 is above
            // the clamped window.
            let surface = TextSurface::from_text("line 0\nline 1\nline 2\nline 3\nline 4");
            let view: View = tags::div()
                .width(10)
                .height(4)
                .overflow(taffy::style::Overflow::Hidden)
                .children(View::from_node(TuiNode::create_text_surface_node(
                    surface,
                    i32::MAX,
                )))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 12, 6), None);
        let painted = canvas_to_plain_text(&canvas);
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
        use crate::surface::TextSurface;
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let surface = TextSurface::from_text("line 0\nline 1\nline 2\nline 3\nline 4");
            let view: View = tags::div()
                .width(10)
                .height(4)
                .overflow(taffy::style::Overflow::Hidden)
                .children(View::from_node(TuiNode::create_text_surface_node(
                    surface, 0,
                )))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 12, 6), None);
        let painted = canvas_to_plain_text(&canvas);
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
            !seven
                .style
                .add_modifier
                .contains(crate::text::Modifier::REVERSED),
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

        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 20, 4), None);
        let painted = canvas_to_plain_text(&canvas);

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
        use crate::surface::{TextRow, TextSegment, TextSurface};

        let surface = TextSurface::from_row(TextRow::from(TextSegment::raw("abcdef")));
        let layout = surface.layout(3, SpanStyle::default());

        assert_eq!(layout.height(), 2);
        assert_eq!(layout.rows()[0].segments[0].content, "a");
        assert_eq!(layout.rows()[0].segments[1].content, "b");
        assert_eq!(layout.rows()[0].segments[2].content, "c");
        assert_eq!(layout.rows()[1].segments[0].content, "d");
    }

    #[test]
    fn text_surface_layout_resolves_segment_style_patched_on_base() {
        use crate::surface::{TextRow, TextSegment, TextSurface};
        use crate::text::{Modifier, SpanStyle};

        let surface = TextSurface::from_row(TextRow::from_segments(vec![
            TextSegment::raw("a"),
            TextSegment::styled(
                "b",
                SpanStyle {
                    add_modifier: Modifier::BOLD,
                    ..SpanStyle::default()
                },
            ),
        ]));
        let layout = surface.layout(10, SpanStyle::default());

        assert_eq!(layout.height(), 1);
        assert!(
            !layout.rows()[0].segments[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            layout.rows()[0].segments[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn text_surface_measures_wrapped_height() {
        use crate::surface::{TextRow, TextSegment, TextSurface};
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::div()
                .width(3)
                .children(View::from_node(TuiNode::create_text_surface_node(
                    TextSurface::from_row(TextRow::from(TextSegment::raw("abcdef"))),
                    0,
                )))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (_canvas, _index) = render(&nodes, Rect::new(0, 0, 3, 10), None);
        root.dispose();
    }

    #[test]
    fn text_surface_paints_scroll_window_only() {
        use crate::surface::{TextRow, TextSurface};
        use crate::view::ViewTuiNode;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let surface = TextSurface::from_rows(vec![
                TextRow::raw("a"),
                TextRow::raw("b"),
                TextRow::raw("c"),
                TextRow::raw("d"),
                TextRow::raw("e"),
            ]);
            let leaf = TuiNode::create_text_surface_node(surface, 1);
            let view: View = tags::div()
                .width(5)
                .height(2)
                .overflow(taffy::style::Overflow::Hidden)
                .children(View::from_node(leaf))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 5, 2), None);
        let painted = canvas_to_plain_text(&canvas);
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
        use crate::surface::{TextRow, TextSegment};
        use crate::text::SpanStyle;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let row = TextRow::from_segments(vec![
                TextSegment::raw("plain "),
                TextSegment::styled(
                    "bold",
                    SpanStyle {
                        add_modifier: crate::text::Modifier::BOLD,
                        ..SpanStyle::default()
                    },
                ),
                TextSegment::styled(
                    " red",
                    SpanStyle {
                        fg: Some(crate::Color::Red),
                        ..SpanStyle::default()
                    },
                ),
            ]);
            let view: View = tags::div().width(20).children(View::from(row)).into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 20, 1), None);
        // "plain " occupies cols 0..6, "bold" cols 6..10, " red" cols 10..14.
        let plain_cell = canvas.cell(0, 0).unwrap().character.as_ref().unwrap();
        let bold_cell = canvas.cell(6, 0).unwrap().character.as_ref().unwrap();
        let red_cell = canvas.cell(10, 0).unwrap().character.as_ref().unwrap();
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
        use crate::surface::TextRow;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::div()
                .width(6)
                .children(View::from(TextRow::raw("好XY")))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 6, 1), None);

        assert_eq!(
            canvas.cell(0, 0).unwrap().character.as_ref().unwrap().value,
            "好"
        );
        assert!(canvas.cell(1, 0).unwrap().character.is_none());
        assert_eq!(
            canvas.cell(2, 0).unwrap().character.as_ref().unwrap().value,
            "X"
        );
        assert_eq!(
            canvas.cell(3, 0).unwrap().character.as_ref().unwrap().value,
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
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 1, 1), None);
        let cell = canvas.cell(0, 0).unwrap().character.as_ref().unwrap();
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
        use crate::surface::{TextRow, TextSegment};
        use crate::text::{Modifier, SpanStyle};
        // One span "ABCDEF" styled BOLD, width 3 → two rows "ABC" / "DEF".
        // Both rows' graphemes must carry BOLD.
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let row = TextRow::from_segments(vec![TextSegment::styled(
                "ABCDEF",
                SpanStyle {
                    add_modifier: Modifier::BOLD,
                    ..SpanStyle::default()
                },
            )]);
            let view: View = tags::div().width(3).children(View::from(row)).into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 3, 2), None);
        for y in 0..2 {
            for x in 0..3 {
                let cell = canvas.cell(x, y).unwrap().character.as_ref().unwrap();
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
        use crate::surface::TextSurface;
        let mut nodes = Vec::new();
        let root = crate::reactive::create_root(|| {
            let view: View = tags::button()
                .children(View::from(TextSurface::from_text("hi")))
                .into();
            nodes = view.nodes.into_iter().collect();
        });
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 10, 1), None);
        let painted = canvas_to_plain_text(&canvas);
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
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 40, 10), None);
        let painted = canvas_to_plain_text(&canvas);

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
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 40, 10), None);
        let painted = canvas_to_plain_text(&canvas);
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
        let (canvas, _index) = render(&nodes, Rect::new(0, 0, 80, 24), None);
        let painted = canvas_to_plain_text(&canvas);

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
