//! Incremental (streaming) Markdown parsing for token-by-token input.
//!
//! [`crate::parser::parse`] is stateless: it re-parses the whole source every
//! call. For an LLM-style token stream that appends a few characters per tick,
//! that is O(n) per tick and — more importantly — gets the streaming semantics
//! wrong: an **unclosed fenced code block** makes pulldown-cmark swallow every
//! subsequent line into the code block until end-of-document, so a paragraph
//! that arrives after an unfinished fence disappears inside it.
//!
//! [`StreamingParser`] fixes both. It keeps a small amount of state across
//! ticks and splits the source into two regions:
//!
//! 1. **Committed prefix** — `src[0..committed_len]`. Every block in here is
//!    fully closed, so its parse result is cached in `committed_blocks` and
//!    never re-parsed. The committed length only ever advances (the stream is
//!    assumed to be append-only; a non-append-only change resets the parser).
//! 2. **Tail** — `src[committed_len..]`. This is the part that may still grow
//!    (the block currently being streamed). It is re-parsed with
//!    [`crate::parser::parse`] every tick. Because the tail starts at a safe
//!    block boundary, an unclosed fence inside it only swallows *tail* content
//!    — never a finished block from the prefix — so already-rendered blocks are
//!    stable.
//!
//! "Safe boundary" heuristic: a blank line is a strong block separator in
//! CommonMark, so the parser commits everything up to (and including) the last
//! blank line that is *not* inside an open fence. Fenced code blocks are
//! tracked precisely (same marker, length ≥ opener, line-trailing whitespace
//! only) so a fence keeps the tail open until its real closer arrives — that
//! is what stops an unfinished code block from eating the next paragraph.
//!
//! Tables and paragraphs do not have explicit closers (they end when the next
//! line is a paragraph interrupt). They are therefore always part of the tail
//! until a blank line arrives — the last row/line of such a block simply
//! re-renders each tick, which is the desired streaming behavior.

use crate::parser::{Block, parse};
use crate::render::{inlines_to_string, render_blocks_to_surface};
use crate::view::blocks_to_view;
use iodilos::producer::Lines;
use iodilos::view::View;
use crate::theme::MarkdownTheme;

/// Result of [`StreamingParser::feed_with_split`]: the full block list for
/// this tick plus the index that splits committed (cached) blocks from the
/// open tail. `blocks[0..committed_count]` is the closed prefix; the rest is
/// re-parsed every tick. [`StreamingSurface`] uses `committed_count` to know
/// how much of its rendered-row cache is still valid.
pub struct FeedResult {
    /// The complete block list (committed prefix + open tail).
    pub blocks: Vec<Block>,
    /// Number of leading blocks that are the closed, cached prefix.
    pub committed_count: usize,
    /// `true` if the source was not an append-only extension of the previous
    /// tick's source and the parser reset itself this tick. Render caches that
    /// key off the committed prefix (e.g. [`StreamingSurface`]) must drop their
    /// whole cache when this is set.
    pub reset_happened: bool,
}

/// A stateful, append-only streaming Markdown parser.
///
/// Create one with [`StreamingParser::new`] and call [`feed`](Self::feed) on
/// every tick with the **full** source as it currently stands. The parser
/// caches the closed-block prefix and only re-parses the open tail, returning
/// the complete current block list by value.
pub struct StreamingParser {
    /// Byte length of the committed (cached) prefix of the source.
    committed_len: usize,
    /// Cached parse result of `src[0..committed_len]`.
    committed_blocks: Vec<Block>,
    /// The committed prefix source itself, kept so newly committed slices can be
    /// parsed incrementally (append + reparse only the new slice's blocks).
    committed_src: String,
    /// Sticky cache for the open-tail Mermaid block: `(source at last successful
    /// parse, rendered diagram at that point)`. While a mermaid fence streams
    /// open, partial input intermittently fails to parse; instead of flickering
    /// back to raw source, we keep showing the last successful diagram until the
    /// source parses again or diverges (block switch). Only the open tail is
    /// affected — once the fence closes the block enters the committed prefix
    /// and is parsed for real each tick (the stable final state).
    tail_mermaid: Option<(String, String)>,
}

impl Default for StreamingParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingParser {
    /// Construct an empty streaming parser.
    pub fn new() -> Self {
        Self {
            committed_len: 0,
            committed_blocks: Vec::new(),
            committed_src: String::new(),
            tail_mermaid: None,
        }
    }

    /// Feed the full current source and get back the complete block list for
    /// this tick, **plus** the committed/tail split index. Same semantics as
    /// [`feed`](Self::feed); prefer this from renderers (e.g.
    /// [`StreamingSurface`]) that need to know which blocks are the cached
    /// prefix so they can cache their rendered rows.
    pub fn feed_with_split(&mut self, src: &str) -> FeedResult {
        // Detect a non-append-only change and reset. `take_while` on the shared
        // prefix tells us how much of the old committed source is still valid.
        let reset_happened = if !src.starts_with(self.committed_src.as_str()) {
            self.reset();
            true
        } else {
            false
        };

        // Advance the committed boundary over any newly-arrived blank-line
        // separators that sit outside an open fence.
        let new_committed_len = self.advance_boundary(src);

        // Parse any freshly committed slice and append its blocks to the cache.
        if new_committed_len > self.committed_len {
            // The slice (old_len..new_len) is not standalone-parseable on its own
            // (a block may straddle the boundary in edge cases), so instead we
            // re-parse the whole committed prefix and replace the cache. This is
            // O(committed) but only runs when the boundary advances, and the
            // committed prefix is itself bounded by how much has been committed
            // since the last advance.
            let fresh = &src[..new_committed_len];
            self.committed_blocks = parse(fresh);
            self.committed_src.clear();
            self.committed_src.push_str(fresh);
            self.committed_len = new_committed_len;
        }

        // Parse the open tail (may be empty) every tick.
        let tail = &src[self.committed_len..];
        let mut tail_blocks = parse(tail);
        // Capture the tail length BEFORE appending — `Vec::append` moves the
        // elements out of `tail_blocks`, leaving it empty, so reading its length
        // afterwards would wrongly count the whole list as committed and let the
        // tail accumulate across ticks (every block would look "closed").
        let tail_count = tail_blocks.len();

        let mut all = std::mem::take(&mut self.committed_blocks);
        all.append(&mut tail_blocks);
        // The committed prefix is exactly the first `committed_count` blocks; the
        // rest is this tick's tail. Cache the committed portion back and return
        // the full list.
        let committed_count = all.len() - tail_count;
        // Apply the sticky Mermaid cache to the open tail only: resolve a
        // `diagram` for each tail Mermaid block so the renderer need not
        // re-parse (and flicker) on partial input. Committed blocks are left
        // untouched (`diagram = None`) — they are closed and parsed for real.
        self.resolve_tail_mermaid(&mut all[committed_count..]);
        // Absorb streaming-incomplete table data rows: a `|`-prefixed Paragraph
        // right after a confirmed Table is pulldown-cmark's view of a data row
        // whose trailing `|` has not arrived yet. Folding it into the Table
        // now keeps the rendered height monotonic across ticks (no recession
        // when the `|` finally closes). See `absorb_streaming_table_rows`.
        absorb_streaming_table_rows(&mut all, committed_count);
        self.committed_blocks = all[..committed_count].to_vec();
        FeedResult {
            blocks: all,
            committed_count,
            reset_happened,
        }
    }

    /// Feed the full current source and get back the complete block list for
    /// this tick. The parser caches the closed prefix; only the open tail is
    /// re-parsed.
    ///
    /// The source is assumed to be **append-only**: each call's source must
    /// start with the previous call's source. If it does not (the content was
    /// edited or replaced), the parser resets itself and re-parses from scratch.
    pub fn feed(&mut self, src: &str) -> Vec<Block> {
        self.feed_with_split(src).blocks
    }

    /// Feed the full current source, render the resulting blocks to a
    /// `Lines` at `width`, and return it. The committed prefix is parsed
    /// incrementally; only the open tail is re-parsed each call (see [`feed`](Self::feed)).
    /// The block→surface conversion is whole-rebuild per call (committed-prefix
    /// surface caching is deferred).
    pub fn feed_to_surface(
        &mut self,
        src: &str,
        width: usize,
        theme: &MarkdownTheme,
    ) -> Lines {
        let blocks = self.feed(src);
        render_blocks_to_surface(&blocks, width, theme)
    }

    /// Feed the full current source and render the resulting blocks into a
    /// **View tree** (composed from framework primitives: `Spans` leaves for
    /// text, `div(border_style)` for code/math frames, `border_title` for
    /// labels). The committed prefix is parsed incrementally; only the open
    /// tail is re-parsed each call. Prefer this over [`feed_to_surface`] for
    /// new code: text re-wraps at the layout width for free on resize.
    pub fn feed_to_view(&mut self, src: &str, theme: &MarkdownTheme) -> View {
        let blocks = self.feed(src);
        blocks_to_view(&blocks, theme)
    }

    /// The byte length of the committed (cached) prefix. Exposed for tests.
    pub fn committed_len(&self) -> usize {
        self.committed_len
    }

    /// Reset all streaming state, as if the parser were freshly constructed.
    fn reset(&mut self) {
        self.committed_len = 0;
        self.committed_blocks.clear();
        self.committed_src.clear();
        self.tail_mermaid = None;
    }

    /// Resolve the `diagram` of every top-level Mermaid block in the open tail
    /// via the sticky cache. See [`StreamingParser::tail_mermaid`].
    fn resolve_tail_mermaid(&mut self, tail: &mut [Block]) {
        for block in tail.iter_mut() {
            if let Block::Mermaid { src, diagram } = block {
                *diagram = self.resolve_one_mermaid(src);
            }
        }
    }

    /// Return the diagram text to display for a single tail Mermaid `src`, or
    /// `None` to let the renderer fall back to colored source. Sticky rules:
    /// - fresh parse succeeds → refresh the cache, return the new diagram;
    /// - parse fails but `src` only grew since the last success → reuse the
    ///   cached diagram (no flicker);
    /// - parse fails and `src` diverged (different block / non-append edit) →
    ///   drop the stale cache; the renderer falls back to source.
    fn resolve_one_mermaid(&mut self, src: &str) -> Option<String> {
        if let Some(rendered) = crate::mermaid::render(src) {
            self.tail_mermaid = Some((src.to_string(), rendered.clone()));
            return Some(rendered);
        }
        if let Some((ok_src, ok_rendered)) = &self.tail_mermaid
            && src.starts_with(ok_src.as_str())
        {
            return Some(ok_rendered.clone());
        }
        self.tail_mermaid = None;
        None
    }

    /// Scan from `committed_len` forward, tracking fence state, and return the
    /// furthest safe commit boundary: the end of the last line that precedes a
    /// blank line, while not inside an open fence. If there is no such boundary
    /// beyond the current commit point, returns the current `committed_len`.
    ///
    /// A line "precedes a blank line" when the *next* line is blank — we commit
    /// up to the end of that next blank line, so the blank separator itself is
    /// owned by the committed side and the tail starts cleanly at a block start.
    fn advance_boundary(&self, src: &str) -> usize {
        let bytes = src.as_bytes();
        let start = self.committed_len;
        if start > bytes.len() {
            return bytes.len();
        }

        let mut fence: Option<Fence> = None;
        // Best candidate commit point: end-of-line index (exclusive) past a
        // line that closes a block — a blank line, a close fence, an ATX
        // heading, a thematic rule, or a math close — while not inside an open
        // fence.
        let mut best = start;

        let mut line_start = start;
        while line_start < bytes.len() {
            let line_end = next_line_end(bytes, line_start);
            let line = &src[line_start..line_end];
            let trimmed_start = line.trim_start_matches(' ');
            let leading_spaces = line.len() - trimmed_start.len();
            let line_past = step_past_newline(bytes, line_end);

            if let Some(open) = fence {
                // Inside a code fence: only a matching close fence ends it.
                if leading_spaces <= 3 && is_close_fence(trimmed_start, open) {
                    fence = None;
                    // Mechanism 2: a closed code fence is a settled block —
                    // commit through the close-fence line's newline so the
                    // whole fenced block enters the committed prefix and its
                    // rows are cached (no per-tick re-render of the code).
                    best = line_past;
                }
                // Any other line (including blanks) is code content: never a
                // commit boundary while inside a fence.
            } else if leading_spaces <= 3 {
                if let Some(open) = open_fence(trimmed_start) {
                    // An opening fence begins a code block: do not commit past
                    // the start of this line. Everything up to here (if a blank
                    // preceded) may already be captured in `best`.
                    fence = Some(open);
                } else if trimmed_start.is_empty() {
                    // Blank line outside any fence: a safe block separator. The
                    // separator itself belongs to the committed side (see the
                    // module/fn doc comments), so commit through the blank
                    // line *and* its terminating newline. `line_end` points AT
                    // the `\n`; advancing past it leaves the tail starting
                    // cleanly at the next line.
                    best = line_past;
                }
                // Any other non-blank line is ordinary content (paragraph,
                // heading, list, table row…). It may still grow, so it does not
                // advance the boundary on its own. (ATX headings, thematic
                // rules, and display-math `$$` are also settled once their
                // construct completes, but committing them early either causes
                // streaming-height recessions — partial `#`/`# A` lines parse
                // to a transient artefact that collapses when the line
                // completes, see `table_streaming_height_never_recedes` — or,
                // for `$$`, shares no clean open/close marker with the fence
                // tracker. The performance win on these single-line blocks is
                // negligible anyway; only fenced code blocks, whose open-close
                // span is the real streaming hot spot, get the early-commit
                // treatment.)
            }

            // Advance past this line (and its terminating newline, if any).
            line_start = line_past;
            if line_start == line_end {
                // No newline at line_end: we've consumed the final partial line.
                break;
            }
        }

        best
    }
}

/// Streaming-table absorb: once a Table is confirmed (header + separator
/// arrived), pulldown-cmark still emits a *half-arrived* data row (no trailing
/// `|`) as a standalone Paragraph that wraps taller than one table row. When
/// the trailing `|` arrives, pulldown-cmark re-absorbs that Paragraph into the
/// Table — the total height *recedes*, so a follow-tail viewport jumps upward.
/// Folding such a Paragraph into the preceding Table's rows here makes the
/// incomplete and complete states render identically (one table row each), so
/// the height is monotonic across ticks. Only the open tail (from
/// `committed_count`) is eligible — committed blocks are already closed. See
/// `docs/superpowers/specs/2026-06-21-table-streaming-absorb-design.md`.
fn absorb_streaming_table_rows(all: &mut Vec<Block>, committed_count: usize) {
    let mut i = committed_count;
    while i + 1 < all.len() {
        let absorb =
            matches!(all[i], Block::Table(_)) && paragraph_is_streaming_table_row(&all[i + 1]);
        if absorb {
            // `remove` shifts later blocks down by one; the block that lands
            // at i+1 may itself be another absorbable `|`-row (several data
            // rows streaming back-to-back), so do not advance i here.
            let Block::Paragraph(inlines) = all.remove(i + 1) else {
                unreachable!("guarded by paragraph_is_streaming_table_row");
            };
            let line = inlines_to_string(&inlines);
            if let Block::Table(table) = &mut all[i] {
                table.rows.push(split_table_row(&line));
            }
        } else {
            i += 1;
        }
    }
}

/// Whether `block` is a Paragraph that reads like one streaming table data
/// row: a single line (no soft break → no `\n` once flattened) whose trimmed
/// text starts with `|`. That is pulldown-cmark's rendering of an incomplete
/// `| a | b` (no trailing `|`).
fn paragraph_is_streaming_table_row(block: &Block) -> bool {
    let Block::Paragraph(inlines) = block else {
        return false;
    };
    let line = inlines_to_string(inlines);
    !line.contains('\n') && line.trim_start().starts_with('|')
}

/// Split a `|`-delimited table data line into trimmed cells, tolerating a
/// missing leading/trailing `|` (the streaming-incomplete case):
/// `| a | b` → `["a","b"]`, `| a | b |` → `["a","b"]`, `| a` → `["a"]`.
fn split_table_row(line: &str) -> Vec<String> {
    let s = line.trim();
    let s = s.strip_prefix('|').unwrap_or(s);
    let s = s.strip_suffix('|').unwrap_or(s);
    s.split('|').map(|c| c.trim().to_string()).collect()
}

/// An open code fence's identity: the marker character and its run length.
#[derive(Clone, Copy, Debug)]
struct Fence {
    ch: char,
    len: usize,
}

/// If `line` (already left-trimmed of up to 3 spaces) opens a fenced code
/// block, return its [`Fence`]. Rules mirror pulldown-cmark: a run of 3+ `` ` ``
/// or 3+ `~`. Backtick fences must not contain a backtick later on the line
/// (info string restriction); tilde fences may carry any info string.
fn open_fence(line: &str) -> Option<Fence> {
    let bytes = line.as_bytes();
    let first = *bytes.first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == first {
        len += 1;
    }
    if len < 3 {
        return None;
    }
    // Backtick fences: no further backtick allowed on the line.
    if first == b'`' && bytes[len..].contains(&b'`') {
        return None;
    }
    Some(Fence {
        ch: first as char,
        len,
    })
}

/// Whether `line` (left-trimmed) is a closing fence for the open `fence`:
/// same marker, run length ≥ opener, and the rest of the line is only spaces.
fn is_close_fence(line: &str, fence: Fence) -> bool {
    let bytes = line.as_bytes();
    let first = match bytes.first() {
        Some(&b) if b as char == fence.ch => b,
        _ => return false,
    };
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == first {
        len += 1;
    }
    if len < fence.len {
        return false;
    }
    // Remainder must be whitespace only.
    bytes[len..].iter().all(|&b| b == b' ')
}

/// Return the byte index just past the end of the line starting at `start`,
/// **excluding** any trailing newline. Slicing `src[start..line_end]` yields the
/// line's content without the `\n`.
fn next_line_end(bytes: &[u8], start: usize) -> usize {
    match bytes[start..].iter().position(|&b| b == b'\n') {
        Some(pos) => start + pos,
        None => bytes.len(),
    }
}

/// Given the end of a line, return the start of the next line (skipping the
/// single `\n` if present). If there is no newline, returns `line_end`
/// unchanged so the caller can detect end-of-input.
fn step_past_newline(bytes: &[u8], line_end: usize) -> usize {
    if line_end < bytes.len() && bytes[line_end] == b'\n' {
        line_end + 1
    } else {
        line_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Block, Inline};

    /// Collect block "kinds" for compact assertion: a short tag per variant.
    fn kinds(blocks: &[Block]) -> Vec<&'static str> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Heading { .. } => "heading",
                Block::Paragraph(_) => "paragraph",
                Block::CodeBlock { .. } => "code",
                Block::List(_) => "list",
                Block::BlockQuote { .. } => "quote",
                Block::Frontmatter(_) => "frontmatter",
                Block::Rule => "rule",
                Block::Table(_) => "table",
                Block::Math(_) => "math",
                Block::Mermaid { .. } => "mermaid",
            })
            .collect()
    }

    #[test]
    fn unclosed_fence_does_not_swallow_following_paragraph() {
        // Two ticks of a stream:
        //   tick 1: a paragraph, then an unclosed code fence, then the start of
        //           a paragraph that pulldown-cmark would normally swallow into
        //           the code block.
        //   tick 2: the fence finally closes and a real paragraph follows.
        let tick1 = "intro paragraph\n\n```rust\nfn half() {\n\nSHOULD_BE_SEPARATE";
        let tick2 = format!("{tick1}\n}}\n```\n\nreal paragraph after");

        let mut p = StreamingParser::new();
        let b1 = p.feed(tick1);

        // The intro paragraph is committed (it is followed by a blank line).
        // The fence and everything after stay in the tail and must NOT cause the
        // intro to vanish or merge.
        assert!(
            b1.iter().any(|x| matches!(x, Block::Paragraph(_))),
            "tick1 should still show the intro paragraph: {:?}",
            kinds(&b1)
        );

        // The unclosed fence must not have swallowed the intro into a code block:
        // there should be at most one code block, and it should be the fence, not
        // the whole document.
        let code_count = b1
            .iter()
            .filter(|x| matches!(x, Block::CodeBlock { .. }))
            .count();
        assert!(
            code_count <= 1,
            "tick1 should not over-produce code blocks: {:?}",
            kinds(&b1)
        );

        let b2 = p.feed(&tick2);
        let k2 = kinds(&b2);
        // After closure we see: paragraph, code, paragraph — and crucially the
        // final real paragraph is its own paragraph, not inside the code block.
        assert!(k2.contains(&"paragraph"), "intro paragraph present: {k2:?}");
        assert!(k2.contains(&"code"), "code block present: {k2:?}");
        // The last block is the trailing real paragraph.
        assert_eq!(
            k2.last(),
            Some(&"paragraph"),
            "trailing paragraph should be its own block, not swallowed: {k2:?}"
        );
    }

    #[test]
    fn committed_prefix_only_advances() {
        // The committed length must be monotonic across appends.
        let mut p = StreamingParser::new();
        let _ = p.feed("a\n\n");
        let after_first = p.committed_len();
        assert!(
            after_first >= 2,
            "blank-terminated line commits: {after_first}"
        );
        let _ = p.feed("a\n\nb\n\n");
        let after_second = p.committed_len();
        assert!(
            after_second >= after_first,
            "commit never recedes: {after_second} < {after_first}"
        );
    }

    #[test]
    fn closed_fence_commits() {
        // A fully closed code fence followed by a blank line commits the fence.
        let mut p = StreamingParser::new();
        let src = "before\n\n```rust\nfn x() {}\n```\n\nafter\n\n";
        let blocks = p.feed(src);
        assert!(
            kinds(&blocks).contains(&"code"),
            "code block rendered: {:?}",
            kinds(&blocks)
        );
        // The whole source ends in a blank line, so it should be fully committed.
        assert_eq!(p.committed_len(), src.len(), "fully committed");
    }

    #[test]
    fn closed_fence_commits_without_trailing_blank_line() {
        // Mechanism 2: a closed code fence settles its block immediately — no
        // need to wait for a trailing blank line. So `...```\ntail` commits the
        // whole fenced block the moment the close fence arrives; the tail is the
        // following content.
        let mut p = StreamingParser::new();
        let src = "intro\n\n```rust\nfn x() {}\n```\nbody still streaming";
        let _ = p.feed(src);
        // The committed prefix must reach *past* the close fence (the ` ``` `
        // line plus its newline), i.e. past `...```\n`, even though no blank
        // line follows it.
        let close_fence_end = src.find("```\n").map(|i| i + "```\n".len()).unwrap();
        assert!(
            p.committed_len() >= close_fence_end,
            "close fence should commit immediately: committed_len={} < close_fence_end={}",
            p.committed_len(),
            close_fence_end
        );
    }

    #[test]
    fn closed_math_fence_commits_without_trailing_blank_line() {
        // (Mechanism 2 was scoped down to fenced *code* only — display-math `$$`
        // has no clean fence marker shared with the tracker, so it is not
        // early-committed. This test documents that the math block is NOT
        // committed early; it commits at the next blank line like before.)
        let mut p = StreamingParser::new();
        let src = "intro\n\n$$\\int_0^1 x\\,dx$$\nbody still streaming";
        let _ = p.feed(src);
        let close_end = src
            .find("$$\n")
            .map(|i| i + "$$\n".len())
            .unwrap();
        assert!(
            p.committed_len() < close_end,
            "math block should NOT early-commit (no trailing blank line yet): committed_len={}, close_end={}",
            p.committed_len(),
            close_end
        );
    }

    #[test]
    fn open_tail_does_not_accumulate_across_ticks() {
        // Regression: while a fenced code block is streaming open, each tick must
        // show ONE growing code block (typewriter), not one NEW code block per
        // tick. The bug was that `Vec::append` empties the tail vec, so
        // `committed_count = all.len() - tail_blocks.len()` counted the whole
        // list as committed and let the tail pile up.
        let mut p = StreamingParser::new();
        let full = "intro\n\n```rust\nfn a() {\n    let x = 1;\n}\n```\n\nafter\n\n";
        let chars: Vec<char> = full.chars().collect();

        let mut prev_code_count = usize::MAX;
        let mut ticks_seen = 0;
        for end in 8..=chars.len() {
            let chunk: String = chars[..end].iter().collect();
            let blocks = p.feed(&chunk);
            let code_count = blocks
                .iter()
                .filter(|b| matches!(b, Block::CodeBlock { .. }))
                .count();
            // At most ONE code block at any time while the fence is open.
            assert!(
                code_count <= 1,
                "tick at len {end}: open fence must not multiply code blocks (got {code_count}): {:?}",
                kinds(&blocks)
            );
            // Once we have a code block, it must never grow back to zero and
            // then reappear as a *new* block — the count is 0 or 1, monotonically
            // "1" once seen until the fence closes.
            if code_count > 0 {
                ticks_seen += 1;
            }
            prev_code_count = code_count;
        }
        let _ = prev_code_count;
        assert!(ticks_seen > 0, "expected to see a streaming code block");
    }

    #[test]
    fn tilde_fence_tracked() {
        // Tilde fences are a separate marker and must not be closed by backticks.
        let mut p = StreamingParser::new();
        // Unclosed tilde fence: nothing past the preceding blank should commit.
        let b = p.feed("p1\n\n~~~\ncode line\n\nnot-yet-closed");
        assert!(b.iter().any(|x| matches!(x, Block::Paragraph(_))));
        let mid = p.committed_len();
        // Closing the tilde fence with a tilde line lets the code block commit.
        let b2 = p.feed("p1\n\n~~~\ncode line\n\nnot-yet-closed\n~~~\n\ndone\n\n");
        assert!(b2.iter().any(|x| matches!(x, Block::CodeBlock { .. })));
        assert!(p.committed_len() > mid);
    }

    #[test]
    fn inline_math_streams() {
        let mut p = StreamingParser::new();
        let blocks = p.feed("energy is $E=mc^2$ here\n\n");
        let para = blocks
            .iter()
            .find_map(|b| {
                if let Block::Paragraph(i) = b {
                    Some(i)
                } else {
                    None
                }
            })
            .expect("a paragraph");
        assert!(
            para.iter().any(|i| matches!(i, Inline::Math(_))),
            "inline math leaf preserved: {para:?}"
        );
    }

    #[test]
    fn display_math_block_streams() {
        let mut p = StreamingParser::new();
        let blocks = p.feed("intro\n\n$$\\int_0^1 x\\,dx$$\n\noutro\n\n");
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Math(_))),
            "display math promoted to a block: {:?}",
            kinds(&blocks)
        );
    }

    #[test]
    fn non_append_change_resets() {
        let mut p = StreamingParser::new();
        let _ = p.feed("aaa\n\nbbb\n\n");
        let committed = p.committed_len();
        assert!(committed > 0);
        // A shorter, non-prefix source should reset without panicking.
        let blocks = p.feed("completely different\n\n");
        assert!(blocks.iter().any(|b| matches!(b, Block::Paragraph(_))));
    }

    #[test]
    fn feed_to_surface_renders_committed_and_tail() {
        let mut p = StreamingParser::new();
        let theme = crate::MarkdownTheme::default();
        // tick 1: a committed paragraph + an open tail paragraph
        let l1 = p.feed_to_surface("intro\n\nunfinished", 60, &theme);
        let joined: String = l1
            .rows
            .iter()
            .flat_map(|l| l.iter())
            .map(|s| s.0.as_str().to_string())
            .collect();
        assert!(
            joined.contains("intro"),
            "committed paragraph present: {joined}"
        );
        assert!(
            joined.contains("unfinished"),
            "tail paragraph present: {joined}"
        );
    }

    /// Extract the `diagram` of the last Mermaid block in the list (the tail
    /// block while a mermaid fence is streaming open).
    fn tail_mermaid_diagram(blocks: &[Block]) -> Option<String> {
        blocks.iter().rev().find_map(|b| match b {
            Block::Mermaid { diagram, .. } => diagram.clone(),
            _ => None,
        })
    }

    #[test]
    fn tail_mermaid_sticky_reuses_last_success_when_parse_fails() {
        // Streaming mermaid (fence still open) grows character by character.
        // The full `ok` content parses; appending an unterminated node `D[`
        // makes the whole thing fail. Sticky should keep the failed tick
        // showing the last successful diagram instead of falling back to raw.
        let ok = "flowchart TD\n    A[Start] --> B{Ready?}\n    B -->|yes| C[Ship]";
        let rendered_ok = crate::mermaid::render(ok).expect("fixture: ok content renders");
        assert!(
            crate::mermaid::render(&format!("{ok}\n    D[")).is_none(),
            "fixture: ok + unterminated node must fail"
        );

        let mut p = StreamingParser::new();
        let d1 = {
            let b = p.feed(&format!("```mermaid\n{ok}"));
            tail_mermaid_diagram(&b)
        };
        assert_eq!(
            d1.as_deref(),
            Some(rendered_ok.as_str()),
            "tick1 full content parses → diagram = rendered result"
        );

        let d2 = {
            let b = p.feed(&format!("```mermaid\n{ok}\n    D["));
            tail_mermaid_diagram(&b)
        };
        assert_eq!(
            d2.as_deref(),
            Some(rendered_ok.as_str()),
            "tick2 parse fails → sticky reuses last success, not None/raw"
        );
    }

    #[test]
    fn tail_mermaid_never_parsed_falls_back_to_raw() {
        // No prior success → no sticky. Raw source throughout.
        let bad = "flowchart TD\n    A[";
        assert!(
            crate::mermaid::render(bad).is_none(),
            "fixture: bad content never parses"
        );
        let mut p = StreamingParser::new();
        let b1 = p.feed(&format!("```mermaid\n{bad}"));
        assert_eq!(tail_mermaid_diagram(&b1), None, "never parsed → no diagram");
        let b2 = p.feed(&format!("```mermaid\n{bad}\n    B["));
        assert_eq!(
            tail_mermaid_diagram(&b2),
            None,
            "still never parsed → no diagram"
        );
    }

    #[test]
    fn closed_mermaid_drops_sticky_and_reparses() {
        // Once the fence closes and commits, the block leaves the tail: its
        // `diagram` is None so the renderer re-parses the stable full source
        // (the "final judgement" after closure, per the agreed semantics).
        let ok = "flowchart TD\n    A[Start] --> B{Ready?}\n    B -->|yes| C[Ship]";
        let mut p = StreamingParser::new();
        let b1 = p.feed(&format!("```mermaid\n{ok}"));
        assert!(
            tail_mermaid_diagram(&b1).is_some(),
            "open fence resolves a sticky diagram"
        );
        // Close the fence + blank line → mermaid commits out of the tail.
        let b2 = p.feed(&format!("```mermaid\n{ok}\n```\n\n"));
        let diagram = b2.iter().find_map(|b| match b {
            Block::Mermaid { diagram, .. } => diagram.clone(),
            _ => None,
        });
        assert_eq!(
            diagram, None,
            "closed/committed mermaid carries no sticky diagram — re-parse for real"
        );
    }

    #[test]
    fn switched_mermaid_does_not_reuse_other_blocks_cache() {
        // A streams and parses (cache = A). A closes. B streams with content
        // that fails to parse AND does not extend A → B falls back to raw,
        // never reusing A's cached diagram.
        let a = "flowchart TD\n    A[Start] --> B{Ready?}\n    B -->|yes| C[Ship]";
        let b = "flowchart TD\n    X --> Y\n    Z[";
        crate::mermaid::render(a).expect("fixture: a renders");
        assert!(
            crate::mermaid::render(b).is_none(),
            "fixture: b fails to parse"
        );
        let mut p = StreamingParser::new();
        // tick1: A open and parses → cache = A.
        p.feed(&format!("```mermaid\n{a}"));
        // tick2: A closes and commits, then B streams open with bad content.
        let blocks = p.feed(&format!("```mermaid\n{a}\n```\n\n```mermaid\n{b}"));
        assert_eq!(
            tail_mermaid_diagram(&blocks),
            None,
            "B (different block, unparseable) must not reuse A's cache"
        );
    }

    #[test]
    fn table_streaming_height_never_recedes() {
        // Regression: streaming a GFM table char-by-char, the total surface row
        // count must never *shrink* between ticks. Root cause (see
        // docs/superpowers/specs/2026-06-21-table-streaming-absorb-design.md):
        // pulldown-cmark parses an incomplete table data row (no trailing `|`)
        // as a standalone Paragraph following the Table; when the trailing `|`
        // arrives the Paragraph is absorbed into the Table, and — because the
        // streaming Paragraph wraps taller than a single table row — the total
        // height *recedes*, making a follow-tail viewport jump upward.
        //
        // Fix under test: once a Table is confirmed (header + separator
        // arrived), a following single-line `|`-prefixed Paragraph is absorbed
        // directly into the table's rows, so the incomplete and complete states
        // render with the same height and no recession occurs.
        let heading = "# A table\n\n";
        let table = "| Feature    | Supported |\n\
|------------|:---------:|\n\
| Headings   |    yes    |\n\
| Code       |    yes    |\n";
        let full = format!("{heading}{table}");
        let chars: Vec<char> = full.chars().collect();

        let mut p = StreamingParser::new();
        let theme = crate::MarkdownTheme::default();
        let width = 40; // width at which a Δ=-1 recession was observed

        let mut prev_total: usize = 0;
        let mut recessions: Vec<String> = Vec::new();
        for end in 1..=chars.len() {
            let chunk: String = chars[..end].iter().collect();
            let surface = p.feed_to_surface(&chunk, width, &theme);
            let total = surface.rows.len();
            if total < prev_total {
                let snippet: String = chars[end.saturating_sub(12)..end].iter().collect();
                recessions.push(format!("end={end} {prev_total}→{total} …{snippet:?}"));
            }
            prev_total = total;
        }
        assert!(
            recessions.is_empty(),
            "row count must be monotonic non-decreasing across streaming ticks, \
             but observed recessions: [{}]",
            recessions.join(", ")
        );
    }

    #[test]
    fn table_body_rows_have_inner_horizontal_rules() {
        // A multi-row table must draw a horizontal rule (├─┼─┤) between each
        // pair of body rows, not only the top, header/body separator, and
        // bottom. Without these inner rules the table reads as "header + outer
        // frame only" and the inner row borders are missing.
        let src = "| H1  | H2  |\n|-----|-----|\n| a   | b   |\n| c   | d   |\n";
        let blocks = crate::parser::parse(src);
        let theme = crate::MarkdownTheme::default();
        let surface = crate::render::render_blocks_to_surface(&blocks, 40, &theme);
        let rows: Vec<String> = surface
            .rows
            .iter()
            .map(|r| r.iter().map(|s| s.0.as_str()).collect())
            .collect();
        let has_inner_rule = rows
            .iter()
            .any(|t| t.starts_with('├') && t.contains('┼') && t.ends_with('┤'));
        assert!(
            has_inner_rule,
            "expected an inner body rule (├─┼─┤) between data rows, got:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    fn task_list_uses_glyph_markers_not_brackets() {
        // Task list items render with ✔ (done) / ☐ (todo) glyphs instead of
        // the raw `[x]` / `[ ]` checkbox characters.
        let src = "- [ ] todo\n- [x] done\n";
        let blocks = crate::parser::parse(src);
        let theme = crate::MarkdownTheme::default();
        let surface = crate::render::render_blocks_to_surface(&blocks, 40, &theme);
        let text: String = surface
            .rows
            .iter()
            .flat_map(|r| r.iter())
            .map(|s| s.0.as_str())
            .collect();
        assert!(
            text.contains('✔'),
            "checked item should use ✔, got: {text:?}"
        );
        assert!(
            text.contains('☐'),
            "unchecked item should use ☐, got: {text:?}"
        );
        assert!(!text.contains("[x]"), "should not show raw [x]: {text:?}");
        assert!(!text.contains("[ ]"), "should not show raw [ ]: {text:?}");
    }

    #[test]
    fn _dbg_parse_incomplete_table() {
        let cases: &[&str] = &[
            "| Feature | Supported |\n|---|---|\n| Headings | yes",
            "| Feature | Supported |\n|---|---|\n| Headings | yes |",
            "| Feature | Supported |\n|---|---|\n| Headings | yes |\n| Code | yes",
            "| Feature | Supported |\n|---|---|\n| Headings | yes |\n| Code | yes |",
        ];
        for c in cases {
            let blocks = crate::parser::parse(c);
            let k: Vec<String> = blocks
                .iter()
                .map(|b| match b {
                    Block::Table(t) => format!("T(rows={},hdr={})", t.rows.len(), t.headers.len()),
                    Block::Paragraph(i) => format!("P({:?})", inlines_to_string(i)),
                    _ => "?".to_string(),
                })
                .collect();
            eprintln!("[{c:?}] => {k:?}");
        }
    }

    #[test]
    fn _dbg_stream_table_ticks() {
        let heading = "# A table\n\n";
        let table = "| Feature    | Supported |\n\
|------------|:---------:|\n\
| Headings   |    yes    |\n\
| Code       |    yes    |\n";
        let full = format!("{heading}{table}");
        let chars: Vec<char> = full.chars().collect();
        let mut p = StreamingParser::new();
        let theme = crate::MarkdownTheme::default();
        let mut prev = 0usize;
        for end in 1..=chars.len() {
            let chunk: String = chars[..end].iter().collect();
            let blocks = p.feed(&chunk);
            let surface = crate::render::render_blocks_to_surface(&blocks, 40, &theme);
            let total = surface.rows.len();
            if total != prev {
                let k: Vec<String> = blocks
                    .iter()
                    .map(|b| match b {
                        Block::Table(t) => format!("T{}", t.rows.len()),
                        Block::Paragraph(i) => format!("P({:?})", inlines_to_string(i)),
                        Block::Heading { .. } => "H".to_string(),
                        _ => "?".to_string(),
                    })
                    .collect();
                eprintln!(
                    "end={end:3} rows={total:3} Δ={:+} clen={} {k:?}",
                    total as i32 - prev as i32,
                    p.committed_len()
                );
                prev = total;
            }
        }
    }

    #[test]
    fn _dbg_render_table_dump() {
        let src = "| Feature    | Supported |\n\
|------------|:---------:|\n\
| Headings   |    yes    |\n\
| Code       |    yes    |\n";
        let blocks = crate::parser::parse(src);
        let theme = crate::MarkdownTheme::default();
        let surface = crate::render::render_blocks_to_surface(&blocks, 40, &theme);
        eprintln!("--- full table render ---");
        for (i, row) in surface.rows.iter().enumerate() {
            let text: String = row.iter().map(|s| s.0.as_str()).collect();
            eprintln!("{i:2}|{text}");
        }
    }

    #[test]
    fn _dbg_task_list_dump() {
        let src = "- [ ] todo with a fairly long wrapping label here\n\
- [x] done\n\
  - [ ] nested unchecked\n\
  - [x] nested checked\n";
        let blocks = crate::parser::parse(src);
        let theme = crate::MarkdownTheme::default();
        let surface = crate::render::render_blocks_to_surface(&blocks, 36, &theme);
        eprintln!("--- task list render ---");
        for (i, row) in surface.rows.iter().enumerate() {
            let text: String = row.iter().map(|s| s.0.as_str()).collect();
            eprintln!("{i:2}|{text}|");
        }
    }

    #[test]
    fn _dbg_mermaid_decision_shape() {
        let src = "flowchart TD\n    A[Start] --> B{Ready?}\n    B -->|yes| C[Go]";
        match crate::mermaid::render(src) {
            Some(out) => {
                eprintln!("--- mmdflux decision render ---");
                for (i, line) in out.lines().enumerate() {
                    eprintln!("{i:2}|{line}");
                }
            }
            None => eprintln!("mermaid::render returned None"),
        }
    }
}
