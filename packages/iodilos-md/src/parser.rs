//! Block-level Markdown intermediate representation.
//!
//! [`parse`] runs `pulldown-cmark` over the source and folds the event stream
//! into a flat `Vec<Block>` tree. Keeping an IR (rather than rendering straight
//! from cmark events) decouples the parser from the iodilos renderer and lets
//! each block be rendered independently.
//!
//! Inline styling (bold/italic/strike/links) is **collapsed into plain text**:
//! under route A (no iodilos core changes) a single text leaf carries one
//! style, so we keep inline markers' text but drop their per-run styling. Inline
//! `code` and `math` are kept as distinct leaves ([`Inline::Code`] /
//! [`Inline::Math`]) so the renderer can color them.

use iodilos::text::SpanStyle;
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

use crate::inline::{InlineStyleState, body_style};
use crate::theme::MarkdownTheme;

/// A single block-level Markdown element.
#[derive(Clone, Debug)]
pub enum Block {
    /// An `#`-prefixed heading. `level` is 1..=6.
    Heading { level: u8, inlines: Vec<Inline> },
    /// A paragraph of inline content.
    Paragraph(Vec<Inline>),
    /// A fenced or indented code block.
    CodeBlock { lang: Option<String>, code: String },
    /// A list (ordered or unordered), possibly nested.
    List(List),
    /// A blockquote, containing further blocks. `kind` is the GFM alert kind
    /// (`[!NOTE]` etc.) when the quote opens with an alert marker, else `None`.
    BlockQuote {
        kind: Option<BlockQuoteKind>,
        blocks: Vec<Block>,
    },
    /// A thematic break (`---`).
    Rule,
    /// A GFM table.
    Table(Table),
    /// A block-level display math block (`$$...$$`). The raw LaTeX source is
    /// stored verbatim — terminal rendering cannot do real math typesetting, so
    /// it is shown as a monospace centered block (see the renderer).
    Math(String),
    /// A fenced Mermaid diagram (` ```mermaid `). The raw source is preserved so
    /// the renderer can turn it into terminal text or fall back to colored source.
    /// `diagram` carries a pre-rendered diagram when an upstream caller (the
    /// streaming parser's sticky cache) has already resolved it; `None` means
    /// the renderer parses `src` itself.
    Mermaid { src: String, diagram: Option<String> },
    /// A YAML-style frontmatter block (`---\n…\n---`) flattened to key/value
    /// pairs.
    Frontmatter(Vec<(String, String)>),
}

/// A list, ordered (`1.`) or unordered (`-`/`*`).
#[derive(Clone, Debug)]
pub struct List {
    pub ordered: bool,
    pub items: Vec<ListItem>,
}

/// One list item.
#[derive(Clone, Debug)]
pub struct ListItem {
    /// For task-list items, the checkbox state.
    pub checked: Option<bool>,
    /// The item's inline content (its own text).
    pub inlines: Vec<Inline>,
    /// Child blocks nested under this item (sub-lists, etc.).
    pub children: Vec<Block>,
}

/// A GFM table.
#[derive(Clone, Debug)]
pub struct Table {
    pub headers: Vec<String>,
    pub aligns: Vec<Alignment>,
    pub rows: Vec<Vec<String>>,
}

/// Inline content. Each `Text` run carries the `SpanStyle` resolved from the
/// inline state (bold/italic/strike/link) at parse time, so the renderer can
/// emit one styled surface segment per run without re-deriving state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    /// A styled text run (bold/italic/strike/link resolved to `SpanStyle`).
    Text(String, SpanStyle),
    /// Inline `code` — renderer wraps it with the code style.
    Code(String),
    /// Inline math (`$...$`) or display math (`$$...$$`) mixed into a paragraph.
    /// Raw LaTeX source, rendered monospace like code.
    Math(String),
    /// A soft/hard line break inside a paragraph.
    SoftBreak,
}

/// Parse Markdown source into a list of blocks using the default theme.
pub fn parse(src: &str) -> Vec<Block> {
    parse_with_theme(src, &MarkdownTheme::default())
}

/// Parse Markdown source into a list of blocks, resolving inline styles with
/// `theme`. Exposed so the renderer (and streaming path) reuse one theme across
/// parse and render.
pub fn parse_with_theme(src: &str, theme: &MarkdownTheme) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_MATH);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);
    opts.insert(Options::ENABLE_GFM); // GFM blockquote alert tags ([!NOTE] …)
    let (src, frontmatter) = crate::frontmatter::extract_frontmatter(src);
    let parser = Parser::new_ext(src, opts);

    let mut p = ParseState::new(theme);
    if let Some(pairs) = frontmatter {
        p.push_block(Block::Frontmatter(pairs));
    }
    for ev in parser {
        p.event(ev);
    }
    p.finish()
}

/// Build a paragraph block from accumulated inlines, promoting a lone math run
/// to a standalone [`Block::Math`] (the common `$$...$$` case, which cmark
/// emits as a single-inline paragraph). Math mixed with surrounding text stays
/// inline.
fn paragraph_block(inlines: Vec<Inline>) -> Block {
    if inlines.len() == 1 && matches!(inlines[0], Inline::Math(_)) {
        let Inline::Math(src) = inlines.into_iter().next().unwrap() else {
            unreachable!()
        };
        Block::Math(src)
    } else {
        Block::Paragraph(inlines)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpecialCodeBlock {
    Latex,
    Mermaid,
}

fn special_code_block_kind(lang: Option<&str>) -> Option<SpecialCodeBlock> {
    let lang = lang?.split_whitespace().next()?.to_ascii_lowercase();
    match lang.as_str() {
        "latex" | "tex" => Some(SpecialCodeBlock::Latex),
        "mermaid" => Some(SpecialCodeBlock::Mermaid),
        _ => None,
    }
}

/// The mutable accumulator for a single `parse` pass.
struct ParseState<'a> {
    /// Color theme, used to resolve inline body styles.
    theme: &'a MarkdownTheme,
    /// Top-level finished blocks.
    top: Vec<Block>,
    /// Open containers (lists, blockquotes, tables).
    stack: Vec<Frame>,
    /// Inline accumulator for the in-progress paragraph / heading / item.
    inlines: Vec<Inline>,
    /// Current inline style state (bold/italic/strike/link open spans).
    inline_state: InlineStyleState,
    /// Heading level while inside a `<hN>`, else `None`.
    in_heading: Option<u8>,
    /// `Some` while inside a code block; the body is appended here.
    code_buf: Option<String>,
    code_lang: Option<String>,
    /// `Some` while inside a table cell; text is appended here.
    cell_buf: Option<String>,
}

impl<'a> ParseState<'a> {
    fn new(theme: &'a MarkdownTheme) -> Self {
        Self {
            theme,
            top: Vec::new(),
            stack: Vec::new(),
            inlines: Vec::new(),
            inline_state: InlineStyleState::default(),
            in_heading: None,
            code_buf: None,
            code_lang: None,
            cell_buf: None,
        }
    }

    /// Number of open blockquote frames on the stack (drives blockquote styling).
    fn blockquote_depth(&self) -> usize {
        self.stack
            .iter()
            .filter(|f| matches!(f, Frame::Quote { .. }))
            .count()
    }

    fn event(&mut self, ev: Event) {
        // While inside a code block, only End(CodeBlock) matters; everything
        // else is body text (cmark emits Text events for the code contents).
        if let Some(buf) = self.code_buf.as_mut() {
            match ev {
                Event::Text(t) => buf.push_str(t.as_ref()),
                Event::End(TagEnd::CodeBlock) => {
                    let code = self.code_buf.take().unwrap_or_default();
                    let lang = self.code_lang.take();
                    match special_code_block_kind(lang.as_deref()) {
                        Some(SpecialCodeBlock::Latex) => self.push_block(Block::Math(code)),
                        Some(SpecialCodeBlock::Mermaid) => self.push_block(Block::Mermaid {
                            src: code,
                            diagram: None,
                        }),
                        None => self.push_block(Block::CodeBlock { lang, code }),
                    }
                }
                _ => {}
            }
            return;
        }

        // Inside a table cell, route text to the cell buffer.
        if self.cell_buf.is_some() && matches!(self.stack.last(), Some(Frame::Table(_))) {
            match ev {
                Event::Text(t) => {
                    if let Some(buf) = self.cell_buf.as_mut() {
                        buf.push_str(t.as_ref());
                    }
                }
                Event::End(TagEnd::TableCell) => {
                    let cell = self.cell_buf.take().unwrap_or_default();
                    if let Some(Frame::Table(tf)) = self.stack.last_mut() {
                        if tf.in_header {
                            tf.table.headers.push(cell);
                        } else {
                            tf.row.push(cell);
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        match ev {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                let inlines = std::mem::take(&mut self.inlines);
                self.push_block(paragraph_block(inlines));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                self.in_heading = Some(heading_level(level));
            }
            Event::End(TagEnd::Heading(_)) => {
                let level = self.in_heading.take().unwrap_or(1);
                let block = Block::Heading {
                    level,
                    inlines: std::mem::take(&mut self.inlines),
                };
                self.push_block(block);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                self.code_buf = Some(String::new());
                self.code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let lang = lang.into_string();
                        if lang.is_empty() { None } else { Some(lang) }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            // CodeBlock End is handled by the code_buf branch above.
            Event::End(TagEnd::CodeBlock) => {}

            Event::Start(Tag::List(start)) => {
                // A nested list begins: flush any in-progress inlines into the
                // currently-open item (its parent's own text) before opening the
                // new list, so the parent's text is not attributed to a child.
                self.flush_inlines_to_open_item();
                let ordered = start.is_some();
                self.stack.push(Frame::List(ListFrame {
                    list: List {
                        ordered,
                        items: Vec::new(),
                    },
                    item: None,
                }));
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(Frame::List(lf)) = self.stack.pop() {
                    self.push_block(Block::List(lf.list));
                }
            }
            Event::Start(Tag::Item) => {
                // A new item starts: any pending inlines belong to the previous
                // item, so flush them first.
                self.flush_inlines_to_open_item();
                if let Some(Frame::List(lf)) = self.stack.last_mut() {
                    lf.item = Some(ListItem {
                        checked: None,
                        inlines: Vec::new(),
                        children: Vec::new(),
                    });
                }
            }
            Event::End(TagEnd::Item) => {
                let pending = std::mem::take(&mut self.inlines);
                if let Some(Frame::List(lf)) = self.stack.last_mut()
                    && let Some(item) = lf.item.as_mut()
                {
                    if !pending.is_empty() {
                        item.inlines.extend(pending);
                    }
                    let finished = lf.item.take().expect("item open");
                    lf.list.items.push(finished);
                }
            }
            Event::TaskListMarker(checked) => {
                if let Some(Frame::List(lf)) = self.stack.last_mut()
                    && let Some(item) = lf.item.as_mut()
                {
                    item.checked = Some(checked);
                }
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                self.stack.push(Frame::Quote {
                    kind,
                    blocks: Vec::new(),
                });
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if let Some(Frame::Quote { kind, blocks }) = self.stack.pop() {
                    self.push_block(Block::BlockQuote { kind, blocks });
                }
            }

            Event::Start(Tag::Table(aligns)) => {
                self.stack.push(Frame::Table(TableFrame {
                    table: Table {
                        headers: Vec::new(),
                        aligns: aligns.into_iter().collect(),
                        rows: Vec::new(),
                    },
                    row: Vec::new(),
                    in_header: true,
                }));
            }
            Event::Start(Tag::TableHead) => {
                if let Some(Frame::Table(tf)) = self.stack.last_mut() {
                    tf.in_header = true;
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(Frame::Table(tf)) = self.stack.last_mut() {
                    tf.in_header = false;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(Frame::Table(tf)) = self.stack.last_mut() {
                    tf.row = Vec::new();
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(Frame::Table(tf)) = self.stack.last_mut() {
                    let row = std::mem::take(&mut tf.row);
                    tf.table.rows.push(row);
                }
            }
            Event::Start(Tag::TableCell) => {
                if matches!(self.stack.last(), Some(Frame::Table(_))) {
                    self.cell_buf = Some(String::new());
                }
            }
            // TableCell End is handled by the cell_buf branch above.
            Event::End(TagEnd::TableCell) => {}
            Event::End(TagEnd::Table) => {
                if let Some(Frame::Table(tf)) = self.stack.pop() {
                    self.push_block(Block::Table(tf.table));
                }
            }

            Event::Rule => self.push_block(Block::Rule),

            // Inline accumulation.
            Event::Text(t) => {
                let st = body_style(self.theme, self.blockquote_depth(), self.inline_state);
                push_inline(&mut self.inlines, Inline::Text(t.into_string(), st));
            }
            Event::Code(t) => push_inline(&mut self.inlines, Inline::Code(t.into_string())),
            // Both inline (`$...$`) and display (`$$...$$`) math arrive as
            // inline runs carrying the raw LaTeX source. A display math run
            // that ends up alone in its paragraph is promoted to a block above.
            // Math never merges with a trailing Text run — keep it a distinct
            // leaf so the renderer can color it.
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.inlines.push(Inline::Math(t.into_string()));
            }
            Event::SoftBreak | Event::HardBreak => self.inlines.push(Inline::SoftBreak),
            Event::FootnoteReference(t) => push_inline(
                &mut self.inlines,
                Inline::Text(t.into_string(), SpanStyle::default()),
            ),
            Event::Start(Tag::Strong) => self.inline_state.in_strong = true,
            Event::End(TagEnd::Strong) => self.inline_state.in_strong = false,
            Event::Start(Tag::Emphasis) => self.inline_state.in_em = true,
            Event::End(TagEnd::Emphasis) => self.inline_state.in_em = false,
            Event::Start(Tag::Strikethrough) => self.inline_state.in_strike = true,
            Event::End(TagEnd::Strikethrough) => self.inline_state.in_strike = false,
            Event::Start(Tag::Link { .. }) => self.inline_state.in_link = true,
            Event::End(TagEnd::Link) => self.inline_state.in_link = false,
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Block> {
        if !self.inlines.is_empty() {
            self.top
                .push(paragraph_block(std::mem::take(&mut self.inlines)));
        }
        self.top
    }

    /// Push a finished block into the innermost open container, or the top level.
    fn push_block(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some(Frame::Quote { blocks, .. }) => blocks.push(block),
            Some(Frame::List(lf)) => {
                if let Some(item) = lf.item.as_mut() {
                    item.children.push(block);
                } else if let Some(last) = lf.list.items.last_mut() {
                    last.children.push(block);
                }
            }
            Some(Frame::Table(_)) => {
                // A block inside a table is unexpected; drop defensively.
            }
            None => self.top.push(block),
        }
    }

    /// Flush the in-progress `inlines` into the currently-open list item (if
    /// any). Called when a new list or item starts, so that the previous item's
    /// trailing text is attributed to it rather than leaking into the next.
    fn flush_inlines_to_open_item(&mut self) {
        let pending = std::mem::take(&mut self.inlines);
        if pending.is_empty() {
            return;
        }
        if let Some(Frame::List(lf)) = self.stack.last_mut()
            && let Some(item) = lf.item.as_mut()
        {
            item.inlines.extend(pending);
            return;
        }
        // No open item: restore so a later flush (e.g. paragraph end) still gets them.
        self.inlines = pending;
    }
}

/// Push an inline, merging with a trailing `Text` of equal style when possible.
fn push_inline(inlines: &mut Vec<Inline>, inline: Inline) {
    if let (Some(Inline::Text(prev, prev_style)), Inline::Text(t, st)) =
        (inlines.last_mut(), &inline)
        && prev_style == st
    {
        prev.push_str(t);
        return;
    }
    inlines.push(inline);
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// A container frame on the open-container stack.
enum Frame {
    List(ListFrame),
    Quote {
        kind: Option<BlockQuoteKind>,
        blocks: Vec<Block>,
    },
    Table(TableFrame),
}

struct ListFrame {
    list: List,
    item: Option<ListItem>,
}

struct TableFrame {
    table: Table,
    row: Vec<String>,
    in_header: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(src: &str) -> Vec<Block> {
        parse(src)
    }

    #[test]
    fn parses_heading_and_paragraph() {
        let b = blocks("# Title\n\nbody text");
        assert!(matches!(b[0], Block::Heading { level: 1, .. }));
        assert!(matches!(b[1], Block::Paragraph(_)));
    }

    #[test]
    fn separates_inline_code_from_text() {
        let b = blocks("a `b` c");
        let Block::Paragraph(inlines) = &b[0] else {
            panic!("not a paragraph");
        };
        // Plain text "a " then code "b" then plain " c".
        assert_eq!(inlines.len(), 3);
        assert!(matches!(inlines[0], Inline::Text(_, _)));
        assert!(matches!(inlines[1], Inline::Code(_)));
        assert!(matches!(inlines[2], Inline::Text(_, _)));
    }

    #[test]
    fn different_style_runs_do_not_merge() {
        // Bold and plain text now carry different SpanStyles, so they stay as
        // separate runs (the merge only coalesces equal-style adjacent runs).
        let b = blocks("**bold** and plain");
        let Block::Paragraph(inlines) = &b[0] else {
            panic!("not a paragraph");
        };
        assert_eq!(
            inlines.len(),
            2,
            "bold + plain should be two runs: {inlines:?}"
        );
    }

    #[test]
    fn bold_and_italic_carry_style() {
        let b = parse("**bold** and *italic*");
        let Block::Paragraph(inlines) = &b[0] else {
            panic!("not a paragraph");
        };
        let has_bold = inlines.iter().any(|i| matches!(
            i,
            Inline::Text(t, s) if t == "bold" && s.add_modifier.contains(iodilos::text::Modifier::BOLD)
        ));
        let has_italic = inlines.iter().any(|i| matches!(
            i,
            Inline::Text(t, s) if t == "italic" && s.add_modifier.contains(iodilos::text::Modifier::ITALIC)
        ));
        assert!(has_bold, "bold run styled: {inlines:?}");
        assert!(has_italic, "italic run styled: {inlines:?}");
    }

    #[test]
    fn parses_unordered_list_with_nesting() {
        let b = blocks("- a\n- b\n  - c\n- d");
        let list = b
            .iter()
            .find_map(|x| {
                if let Block::List(l) = x {
                    Some(l)
                } else {
                    None
                }
            })
            .expect("a list");
        assert!(!list.ordered);
        assert_eq!(list.items.len(), 3);
        // The parent item keeps its own text ("b"), separate from the nested
        // child ("c") — a regression guard for the shared inline buffer. Both
        // carry the plain body style resolved by the parser.
        let body = body_style(&MarkdownTheme::default(), 0, InlineStyleState::default());
        assert_eq!(
            list.items[1].inlines,
            vec![Inline::Text("b".to_string(), body)]
        );
        assert_eq!(list.items[1].children.len(), 1);
        let nested = list.items[1].children.first().unwrap();
        let Block::List(nested_list) = nested else {
            panic!("expected nested list, got {nested:?}");
        };
        assert_eq!(nested_list.items.len(), 1);
        assert_eq!(
            nested_list.items[0].inlines,
            vec![Inline::Text("c".to_string(), body)]
        );
    }

    #[test]
    fn parses_ordered_list() {
        let b = blocks("1. one\n2. two\n3. three");
        let list = b
            .iter()
            .find_map(|x| {
                if let Block::List(l) = x {
                    Some(l)
                } else {
                    None
                }
            })
            .expect("a list");
        assert!(list.ordered);
        assert_eq!(list.items.len(), 3);
    }

    #[test]
    fn parses_task_list_markers() {
        let b = blocks("- [x] done\n- [ ] todo");
        let list = b
            .iter()
            .find_map(|x| {
                if let Block::List(l) = x {
                    Some(l)
                } else {
                    None
                }
            })
            .expect("a list");
        assert_eq!(list.items[0].checked, Some(true));
        assert_eq!(list.items[1].checked, Some(false));
    }

    #[test]
    fn parses_code_block_with_lang() {
        let b = blocks("```rust\nfn main() {}\n```");
        let code = b
            .iter()
            .find_map(|x| {
                if let Block::CodeBlock { lang, code } = x {
                    Some((lang.clone(), code.clone()))
                } else {
                    None
                }
            })
            .expect("a code block");
        assert_eq!(code.0.as_deref(), Some("rust"));
        assert!(code.1.contains("fn main()"));
    }

    #[test]
    fn parses_blockquote() {
        let b = blocks("> quoted\n> still quoted");
        assert!(matches!(b[0], Block::BlockQuote { .. }));
    }

    #[test]
    fn parses_rule() {
        let b = blocks("a\n\n---\n\nb");
        assert!(b.iter().any(|x| matches!(x, Block::Rule)));
    }

    #[test]
    fn parses_table() {
        let src = "| H1 | H2 |\n|----|----|\n| a  | b  |\n| c  | d  |";
        let b = blocks(src);
        let table = b
            .iter()
            .find_map(|x| {
                if let Block::Table(t) = x {
                    Some(t)
                } else {
                    None
                }
            })
            .expect("a table");
        assert_eq!(table.headers, vec!["H1".to_string(), "H2".to_string()]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0], vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn parses_inline_math_as_inline_leaf() {
        // Inline math mixed with text stays an inline leaf inside the paragraph.
        let b = blocks("energy is $E=mc^2$ here");
        let Block::Paragraph(inlines) = &b[0] else {
            panic!("expected a paragraph, got {b:?}");
        };
        let math = inlines
            .iter()
            .find_map(|i| {
                if let Inline::Math(s) = i {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .expect("an inline math leaf");
        assert_eq!(math, "E=mc^2");
    }

    #[test]
    fn parses_display_math_as_block() {
        // A $$...$$ run alone in its paragraph is promoted to a block-level Math.
        let b = blocks("intro\n\n$$\\int_0^1 x\\,dx$$\n\noutro");
        let math = b
            .iter()
            .find_map(|x| {
                if let Block::Math(s) = x {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .expect("a block-level Math");
        assert!(math.contains("\\int_0^1"), "math source preserved: {math}");
    }

    #[test]
    fn parses_fenced_latex_as_math_block() {
        let b = blocks("```latex\nx^2 + y^2 = z^2\n```");
        let math = b
            .iter()
            .find_map(|x| {
                if let Block::Math(s) = x {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .expect("a fenced latex block promoted to Math");
        assert!(math.contains("x^2"), "latex source preserved: {math}");
    }

    #[test]
    fn parses_fenced_mermaid_as_mermaid_block() {
        let b = blocks("```mermaid\nflowchart TD\n    A --> B\n```");
        let diagram = b
            .iter()
            .find_map(|x| {
                if let Block::Mermaid { src: s, .. } = x {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .expect("a fenced mermaid block");
        assert!(
            diagram.contains("flowchart TD"),
            "mermaid source preserved: {diagram}"
        );
    }

    #[test]
    fn smart_punctuation_converts_double_dash_to_en_dash() {
        // Options::SMART turns `--` into an en dash (U+2013), matching leaf's
        // `Options::all()` behaviour (leaf enables SMART implicitly).
        let b = blocks("a -- b");
        let Block::Paragraph(inlines) = &b[0] else {
            panic!("expected a paragraph, got {b:?}");
        };
        let joined: String = inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text(t, _) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(
            joined.contains('\u{2013}'),
            "SMART should convert `--` to en dash: {joined:?}"
        );
    }

    #[test]
    fn parses_gfm_alert_kind() {
        // GFM alert `> [!NOTE]` → a BlockQuote carrying `kind = Note`.
        // Requires Options::ENABLE_BLOCK_QUOTE_ALERT_KIND (leaf uses Options::all()).
        let b = blocks("> [!NOTE]\n> body text");
        let Block::BlockQuote { kind, blocks } = &b[0] else {
            panic!("expected a blockquote, got {b:?}");
        };
        assert!(
            matches!(kind, Some(pulldown_cmark::BlockQuoteKind::Note)),
            "alert kind preserved: {kind:?}"
        );
        assert!(!blocks.is_empty(), "alert body preserved");
    }

    #[test]
    fn parses_yaml_frontmatter_as_block() {
        let b = parse("---\ntitle: Hi\nauthor: Sol\n---\n\nbody");
        let Some(Block::Frontmatter(pairs)) = b.first() else {
            panic!("expected frontmatter first, got {b:?}");
        };
        assert!(
            pairs.iter().any(|(k, v)| k == "title" && v == "Hi"),
            "title pair: {pairs:?}"
        );
        assert!(
            pairs.iter().any(|(k, v)| k == "author" && v == "Sol"),
            "author pair: {pairs:?}"
        );
        // The body after the frontmatter is preserved as a following block.
        assert!(b.len() >= 2, "body preserved after frontmatter: {b:?}");
    }
}
