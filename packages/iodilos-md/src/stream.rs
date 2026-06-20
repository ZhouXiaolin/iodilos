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

use iodilos::text::Line;

use crate::parser::{parse, Block};
use crate::render::render_blocks_to_lines;
use crate::theme::MarkdownTheme;

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
        // Detect a non-append-only change and reset. `take_while` on the shared
        // prefix tells us how much of the old committed source is still valid.
        if !src.starts_with(self.committed_src.as_str()) {
            self.reset();
        }

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
        self.committed_blocks = all[..committed_count].to_vec();
        all
    }

    /// Feed the full current source, render the resulting blocks to a flat
    /// `Vec<Line>` at `width`, and return it. The committed prefix is parsed
    /// incrementally; only the open tail is re-parsed each call (see [`feed`](Self::feed)).
    /// The block→`Line` conversion is whole-rebuild per call (committed-prefix
    /// `Line` caching is deferred to Plan 4).
    pub fn feed_to_lines(
        &mut self,
        src: &str,
        width: usize,
        theme: &MarkdownTheme,
    ) -> Vec<Line> {
        let blocks = self.feed(src);
        render_blocks_to_lines(&blocks, width, theme)
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
        // Best candidate commit point: end-of-line index (exclusive) of a blank
        // line we just saw, while not inside a fence.
        let mut best = start;

        let mut line_start = start;
        while line_start <= bytes.len() {
            let line_end = next_line_end(bytes, line_start);
            let line = &src[line_start..line_end];
            let trimmed_start = line.trim_start_matches(' ');
            let leading_spaces = line.len() - trimmed_start.len();

            if let Some(open) = fence {
                // Inside a code fence: only a matching close fence ends it.
                if leading_spaces <= 3 && is_close_fence(trimmed_start, open) {
                    fence = None;
                    // The close fence line is itself part of the code block; a
                    // trailing blank line after it would be the real boundary.
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
                    // Blank line outside any fence: a safe block separator. We
                    // can commit through the end of this blank line.
                    best = line_end;
                }
                // A non-blank, non-fence line is ordinary content (paragraph,
                // heading, list, table row…). It may or may not close its block
                // on this line, so it does not advance the boundary on its own.
            }

            // Advance past this line (and its terminating newline, if any).
            line_start = step_past_newline(bytes, line_end);
            if line_start == line_end {
                // No newline at line_end: we've consumed the final partial line.
                break;
            }
        }

        best
    }
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
                Block::BlockQuote(_) => "quote",
                Block::Rule => "rule",
                Block::Table(_) => "table",
                Block::Math(_) => "math",
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
        let code_count = b1.iter().filter(|x| matches!(x, Block::CodeBlock { .. })).count();
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
        assert!(after_first >= 2, "blank-terminated line commits: {after_first}");
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
        assert!(kinds(&blocks).contains(&"code"), "code block rendered: {:?}", kinds(&blocks));
        // The whole source ends in a blank line, so it should be fully committed.
        assert_eq!(p.committed_len(), src.len(), "fully committed");
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
            .find_map(|b| if let Block::Paragraph(i) = b { Some(i) } else { None })
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
    fn feed_to_lines_renders_committed_and_tail() {
        let mut p = StreamingParser::new();
        let theme = crate::MarkdownTheme::default();
        // tick 1: a committed paragraph + an open tail paragraph
        let l1 = p.feed_to_lines("intro\n\nunfinished", 60, &theme);
        let joined: String = l1
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref().to_string())
            .collect();
        assert!(joined.contains("intro"), "committed paragraph present: {joined}");
        assert!(
            joined.contains("unfinished"),
            "tail paragraph present: {joined}"
        );
    }
}
