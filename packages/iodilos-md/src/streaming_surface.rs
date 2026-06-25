//! Incremental Markdown **render** cache — the rendering-layer counterpart to
//! [`crate::stream::StreamingParser`].
//!
//! `StreamingParser` caches the committed *parse* (its `Vec<Block>`), but
//! [`crate::StreamingParser::feed_to_surface`] then hands the whole block list
//! to [`crate::render::render_blocks_to_surface`], which re-renders every
//! committed block — and re-runs the `Highlighter` on every code-block line —
//! on every tick. That is O(doc) per token.
//!
//! [`StreamingSurface`] sits next to the parser and caches the **rendered rows**
//! of the committed prefix. Each tick it re-renders only the open tail (plus,
//! on a boundary advance, the newly-committed blocks) and concatenates. Per
//! tick the cost drops from O(doc) to O(tail).
//!
//! # Invariants
//!
//! `cached_committed_rows` obeys the **separator invariant** (the design's Q5):
//! at any moment it is a sequence of per-block content row-groups with exactly
//! one blank-line separator (`vec![]`) *between* groups and **none** after the
//! last group. The boundary with the tail is joined with one separator (when
//! both sides are non-empty), so the final surface never gains or loses a
//! spurious blank line on a boundary advance or an empty tail.
//!
//! # What this is *not*
//!
//! - It does not cache the parse layer's O(committed) re-parse on a boundary
//!   advance (see `StreamingParser::feed`). That is a sibling optimization.
//! - It assumes the [`crate::theme::MarkdownTheme`] is fixed for the lifetime
//!   of the cache — the cache key does not track theme. Run-time theme
//!   switching would need an extra invalidation rule.

use iodilos::Color;
use iodilos::producer::Lines;
use iodilos::text::SpanStyle;

use crate::parser::Block;
use crate::stream::{FeedResult, StreamingParser};
use crate::theme::MarkdownTheme;

/// One rendered terminal row: a list of `(text, style)` runs.
pub type Row = Vec<(String, SpanStyle)>;

/// Per-line highlight result for one source line of a code block: the exact
/// type `Highlighter::highlight_line` returns. Cached across ticks by
/// [`TailCodeHighlightCache`] so the open code block's stable lines are not
/// re-highlighted each tick (T4b).
type LineHighlights = Vec<(String, Option<Color>)>;

/// A single-slot cache of the **open** (tail) code block's per-line highlights.
///
/// While a fenced code block streams (open fence → close fence), the tail is
/// re-rendered in full every tick. Without this cache that means re-running
/// the `Highlighter` on all N lines each tick — O(N) per tick, O(N²) over the
/// block's lifetime (measured: ~400 highlight calls/tick for a 400-line block).
///
/// `highlight_line` is line-independent (it constructs a fresh `HighlightLines`
/// per line — see `highlight.rs`), so "line bytes unchanged ⇒ highlight
/// unchanged" is a *sound* cache. This cache holds the highlights for every
/// line of the current tail code block. On a tick it reuses the longest prefix
/// of cached lines whose source text still matches and re-highlights only the
/// remaining (dirty) lines — typically just the last, still-streaming line.
///
/// Single slot is sufficient because the streaming parser holds at most one
/// open block in the tail at a time. The slot is invalidated whenever the tail
/// block is not a code block, the language changes, or the committed prefix
/// is rebuilt (boundary advance / reset / width change).
#[derive(Default)]
struct TailCodeHighlightCache {
    /// The language string the cached highlights were computed for.
    lang: String,
    /// The source lines (without trailing newline) the highlights correspond
    /// to, kept so a prefix-match can be detected without re-splitting. Index
    /// `i` corresponds to `highlights[i]`.
    lines: Vec<String>,
    /// Cached highlights, parallel to `lines`.
    highlights: Vec<LineHighlights>,
}

impl TailCodeHighlightCache {
    /// Compute (or reuse) per-line highlights for a streaming code block,
    /// returning a reference to the full highlights slice. Only lines whose
    /// source text differs from the cached value are re-highlighted.
    ///
    /// `hl` is the shared zero-cost highlighter; `lang` is the raw fence
    /// info-string (may be empty); `code` is the block's current full text.
    fn get(&mut self, lang: &str, code: &str, hl: &crate::highlight::Highlighter) -> &[LineHighlights] {
        // Language change invalidates the whole cache.
        if self.lang != lang {
            self.lang = lang.to_string();
            self.lines.clear();
            self.highlights.clear();
        }

        // Split into source lines, matching render_code_block's iteration:
        // `code.lines()` plus a single empty "" when the code is empty (so an
        // empty block still renders one blank framed line).
        let new_lines: Vec<String> = code
            .lines()
            .chain((code.is_empty()).then_some(""))
            .map(|l| l.to_string())
            .collect();

        // Reuse the longest stable prefix: while cached line i equals the new
        // line i, keep its highlight; everything after is dirty. Truncate the
        // cache to the stable prefix (drops stale tail lines when the block
        // shrank, e.g. on a non-append edit — rare, but correct).
        let stable = self
            .lines
            .iter()
            .zip(new_lines.iter())
            .take_while(|(a, b)| a == b)
            .count();
        self.lines.truncate(stable);
        self.highlights.truncate(stable);

        // Re-highlight only the dirty tail (typically one line — the streaming
        // line currently being appended to).
        for line in &new_lines[stable..] {
            self.lines.push(line.clone());
            self.highlights.push(hl.highlight_line(line, &self.lang));
        }

        &self.highlights
    }
}

/// A streaming Markdown **renderer** that caches the committed prefix's
/// rendered rows, re-rendering only the open tail each tick.
///
/// Wrap a [`StreamingParser`] and call [`render`](Self::render) on every tick
/// with the full current source. See the [module docs](self) for the design.
pub struct StreamingSurface {
    parser: StreamingParser,
    /// Cached rendered rows of the committed prefix. Separator invariant:
    /// row-groups separated by exactly one `vec![]`, none trailing.
    cached_committed_rows: Vec<Row>,
    /// The `src` byte length the cache corresponds to (the parser's
    /// `committed_len` at the time the cache was last (re)built).
    cached_committed_len: usize,
    /// The width the cache was rendered at. A change flushes the whole cache
    /// (every committed row was wrapped at the wrong width).
    cached_width: Option<usize>,
    /// Per-line highlight cache for the **open** (tail) code block (T4b). The
    /// tail is re-rendered in full each tick; without this cache every line of
    /// an open code block is re-highlighted every tick. See [`TailCodeHighlightCache`].
    tail_code: TailCodeHighlightCache,
}

impl Default for StreamingSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingSurface {
    /// Construct an empty streaming renderer wrapping a fresh parser.
    pub fn new() -> Self {
        Self::with_parser(StreamingParser::new())
    }

    /// Construct a renderer wrapping an existing parser (useful if the caller
    /// wants to pre-seed or share parse state).
    pub fn with_parser(parser: StreamingParser) -> Self {
        Self {
            parser,
            cached_committed_rows: Vec::new(),
            cached_committed_len: 0,
            cached_width: None,
            tail_code: TailCodeHighlightCache::default(),
        }
    }

    /// The byte length of the committed (cached) prefix. Exposed for tests.
    pub fn committed_len(&self) -> usize {
        self.parser.committed_len()
    }

    /// Feed the source through the same parser state as [`render`](Self::render)
    /// but **re-render every block** (no committed-row cache). Exposed so the
    /// cache's correctness can be tested against an uncached path that shares
    /// the *exact* parser history — the parser is stateful across ticks, so a
    /// comparison against a separately-constructed parser would be meaningless.
    #[doc(hidden)]
    pub fn render_uncached(&mut self, src: &str, width: usize, theme: &MarkdownTheme) -> Lines {
        let blocks = self.parser.feed(src);
        crate::render::render_blocks_to_surface(&blocks, width, theme)
    }

    /// Feed the full current source and render the complete surface at `width`,
    /// re-rendering only the open tail on most ticks. Signature matches
    /// [`StreamingParser::feed_to_surface`] so call sites swap over with no
    /// shape change.
    ///
    /// # When the cache is rebuilt
    ///
    /// The committed prefix is cached and reused while the parser's
    /// `committed_len` (the byte boundary) and the render `width` are both
    /// stable. On either of:
    /// - a parser reset (the source was not an append-only extension),
    /// - a boundary advance (`committed_len` grew), or
    /// - a width change,
    ///
    /// the committed-row cache is flushed and the committed prefix is
    /// re-rendered wholesale. Why a full flush on a *boundary advance* rather
    /// than appending only new blocks: when the parser advances `committed_len`
    /// it re-parses `src[..committed_len]` from scratch, and the block
    /// structure inside the committed prefix can evolve — a list gains an item,
    /// a table gains a row, even though the *count* of top-level blocks may be
    /// unchanged. Caching by block-count is unsafe; only the byte boundary is a
    /// reliable cache key. The per-tick win survives because most ticks move
    /// only the open tail (the streaming block) without advancing the boundary;
    /// a boundary advance (a blank line arriving) does incur one O(committed)
    /// re-render, matching the parser's own O(committed) re-parse on the same
    /// tick.
    pub fn render(&mut self, src: &str, width: usize, theme: &MarkdownTheme) -> Lines {
        let FeedResult {
            blocks,
            committed_count,
            reset_happened,
        } = self.parser.feed_with_split(src);
        let new_len = self.parser.committed_len();

        // Decide whether the committed-row cache must be rebuilt. The cache is
        // valid only while its (committed_len, width) key is unchanged AND no
        // reset happened this tick.
        let key_changed = reset_happened
            || self.cached_committed_len != new_len
            || self.cached_width != Some(width);

        if key_changed {
            self.cached_width = Some(width);
            // Re-render the committed prefix wholesale, routing code blocks
            // through the highlight cache (T4b commit reuse): a code block that
            // *just* committed this tick is still in `tail_code`, so its lines
            // are all stable and re-render with zero highlight calls. The
            // separator rhythm (one blank row between blocks, none trailing)
            // matches render_blocks_to_surface exactly.
            self.cached_committed_rows = self.render_blocks_cached(
                &blocks[..committed_count],
                width,
                theme,
            );
            self.cached_committed_len = new_len;
        }

        // tail: always fully re-rendered (it is the block currently streaming).
        let mut rows = self.cached_committed_rows.clone();
        let has_tail_blocks = committed_count < blocks.len();
        if has_tail_blocks {
            // Q5 rule B: one separator between committed and tail when the
            // committed side has blocks and there is at least one tail block.
            // The separator is keyed on *block presence*, not on whether the
            // tail rendered any rows — an empty-ish tail block (e.g. a
            // blockquote with no text yet) still counts as a block and must be
            // separated, matching the uncached `render_blocks_to_surface` which
            // inserts the separator before every non-first block regardless of
            // its rendered height.
            if !rows.is_empty() {
                rows.push(vec![]);
            }
            // T4b: route tail code blocks through the highlight cache too, so
            // the open block's stable lines aren't re-highlighted each tick.
            rows.extend(self.render_blocks_cached(
                &blocks[committed_count..],
                width,
                theme,
            ));
        }
        Lines::new(rows)
    }

    /// Render a slice of blocks into rows, routing `CodeBlock`s through the
    /// `tail_code` highlight cache and every other block type through the
    /// normal renderer. Output is byte-identical to `render_blocks_to_surface`
    /// (same separator rhythm: one blank row between blocks, none trailing);
    /// the only difference is that code blocks skip the Highlighter when their
    /// lines are already cached.
    ///
    /// Single-slot caveat: `tail_code` holds one code block's highlights. When
    /// a slice contains multiple code blocks (only possible in the *committed*
    /// prefix, never the tail), the cache is reused for the first and each
    /// subsequent code block with a different `code`/`lang` re-highlights in
    /// full (a cache miss) — but that only happens on the boundary-advance
    /// rebuild, the same event that already re-renders the whole prefix.
    fn render_blocks_cached(&mut self, blocks: &[Block], width: usize, theme: &MarkdownTheme) -> Vec<Row> {
        let hl = crate::highlight::Highlighter::new();
        let mut out: Vec<Row> = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            if i > 0 {
                out.push(vec![]); // blank-line rhythm between blocks
            }
            match block {
                Block::CodeBlock { lang, code } => {
                    let lang_str = lang.as_deref().unwrap_or("");
                    let highlights = self.tail_code.get(lang_str, code, &hl).to_vec();
                    crate::render::render_code_block_with_highlights(
                        lang_str,
                        &highlights,
                        width,
                        theme,
                        &mut out,
                    );
                }
                other => {
                    // A non-code block renders normally. We do NOT invalidate
                    // `tail_code` here: `get()` self-invalidates whenever it's
                    // called with a different lang or non-matching code text,
                    // so a stale entry simply recomputes on the next code block
                    // and is harmless in the meantime. (Eager invalidation is
                    // only an optimization and the helper couldn't reliably tell
                    // a tail slice from a committed slice anyway.)
                    crate::render::render_block_into(other, width, theme, &hl, &mut out);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface_text(lines: &Lines) -> String {
        lines
            .rows
            .iter()
            .flat_map(|r| r.iter())
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>()
            .join("│")
    }

    /// The cached streaming renderer's per-tick output must equal the
    /// **non-cached** streaming renderer's output at every tick. Both run the
    /// same incremental parser; the cache's only job is to avoid re-rendering
    /// committed blocks, so the two must be byte-identical. This is the
    /// correctness golden test for the committed-row cache (Q3) and the
    /// separator invariant (Q5).
    ///
    /// Note: it deliberately does *not* compare against a one-shot
    /// `render_to_surface(chunk)` of the partial chunk — the streaming parser
    /// is designed to render partial input more gracefully than a naive
    /// full-parse of that same fragment (e.g. it keeps an open fence from
    /// swallowing the next paragraph), so the two legitimately differ on
    /// incomplete ticks. The cache under test preserves the *streaming* result,
    /// not the one-shot one.
    fn assert_cached_matches_uncached_stream(src: &str, width: usize) {
        let theme = MarkdownTheme::default();
        // Two identical renderers driven in lock-step through every prefix.
        // `cached` uses the committed-row cache; `uncached` re-renders every
        // block each tick. Both share the *same* parser history (the parser is
        // stateful, so the only valid comparison is against a path that walked
        // the identical tick sequence). The cache's job is to be invisible —
        // identical output, fewer re-renders.
        let mut cached = StreamingSurface::new();
        let mut uncached = StreamingSurface::new();
        let chars: Vec<char> = src.chars().collect();
        for end in 1..=chars.len() {
            let chunk: String = chars[..end].iter().collect();
            let c = cached.render(&chunk, width, &theme);
            let u = uncached.render_uncached(&chunk, width, &theme);
            assert_eq!(
                c.rows, u.rows,
                "tick at len {end} (chunk {chunk:?}): cached stream diverged from uncached stream"
            );
        }
    }

    #[test]
    fn stream_matches_one_shot_paragraph_and_code() {
        let src = "# Title\n\nintro paragraph that wraps\n\n```rust\nfn main() {}\n```\n\ntail para\n";
        assert_cached_matches_uncached_stream(src, 24);
    }

    #[test]
    fn stream_matches_one_shot_list_and_quote() {
        let src = "- one\n- two with a longer item\n  - nested\n\n> quoted\n> more\n";
        assert_cached_matches_uncached_stream(src, 20);
    }

    #[test]
    fn stream_matches_one_shot_table() {
        let src = "| H1 | H2 |\n|----|----|\n| a  | b  |\n| cc | dd |\n";
        assert_cached_matches_uncached_stream(src, 24);
    }

    #[test]
    fn stream_matches_one_shot_long_code_block() {
        // The headline case for the committed-row cache + mechanism 2 (close
        // fence commits early): a long fenced code block followed by prose.
        let code = (1..=20)
            .map(|i| format!("    let x{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let src = format!("```rust\nfn demo() {{\n{code}\n}}\n```\n\nAfter the block, more prose streams here.\n");
        assert_cached_matches_uncached_stream(&src, 30);
    }

    #[test]
    fn width_change_re_renders_committed_prefix() {
        let theme = MarkdownTheme::default();
        let mut surf = StreamingSurface::new();
        // Stream a committed paragraph at width 30.
        surf.render("aaa bbb ccc\n\n", 30, &theme);
        assert_eq!(surf.cached_width, Some(30));

        // Re-render the same source at width 10; output must equal a fresh
        // one-shot render at width 10.
        let streamed = surf.render("aaa bbb ccc\n\n", 10, &theme);
        let one_shot = crate::render::render_to_surface("aaa bbb ccc\n\n", 10, &theme);
        assert_eq!(streamed.rows, one_shot.rows);
        assert_eq!(surf.cached_width, Some(10));
    }

    #[test]
    fn non_append_change_resets_cache() {
        let theme = MarkdownTheme::default();
        let mut surf = StreamingSurface::new();
        surf.render("aaa\n\nbbb\n\n", 40, &theme);
        let committed_before = surf.committed_len();
        assert!(committed_before > 0);

        // A completely different, non-prefix source triggers a parser reset;
        // the row cache must drop (no stale rows bleed into the new content).
        let streamed = surf.render("completely different\n\n", 40, &theme);
        let one_shot = crate::render::render_to_surface("completely different\n\n", 40, &theme);
        assert_eq!(streamed.rows, one_shot.rows);
    }

    #[test]
    fn empty_tail_yields_no_trailing_separator() {
        // Once everything is committed (ends in a blank line), the surface must
        // not carry a trailing blank-line separator (Q5: tail-empty case).
        let theme = MarkdownTheme::default();
        let src = "# H\n\nbody\n\n";
        let mut surf = StreamingSurface::new();
        let streamed = surf.render(src, 40, &theme);
        // The last row must not be an empty separator row.
        assert!(
            !streamed.rows.last().is_some_and(|r| r.is_empty()),
            "trailing separator present: {:?}",
            streamed.rows
        );
        let one_shot = crate::render::render_to_surface(src, 40, &theme);
        assert_eq!(streamed.rows, one_shot.rows);
    }

    #[test]
    fn surface_text_smoke() {
        // Sanity: a simple paragraph renders visible text.
        let theme = MarkdownTheme::default();
        let mut surf = StreamingSurface::new();
        let lines = surf.render("hello world\n\n", 40, &theme);
        let text = surface_text(&lines);
        assert!(text.contains("hello"));
    }

    // ------------------------------------------------------------------
    // T4b: per-line highlight cache for the *open* (streaming) code block.
    //
    // These are the two correctness/perf gates for the TailCodeHighlightCache.
    // `t4b_open_fence_rehighlights_at_most_dirty_line` is the perf gate and the
    // reason T4b exists; it must pass once the cache is wired into the tail
    // render path. `t4b_cached_output_matches_uncached` is the correctness gate.
    // ------------------------------------------------------------------

    /// The headline T4b assertion: while an N-line code block streams open, the
    /// highlighter is invoked at most a small constant number of times per tick
    /// (one for the dirty tail line, plus slack for the close-fence tick),
    /// **not** O(N). Before T4b this peak was ~N+1; the gate is peak ≤ 2.
    ///
    /// Run with: `cargo test -p iodilos-md t4b_open_fence -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn t4b_open_fence_rehighlights_at_most_dirty_line() {
        const GRAN: usize = 30; // ~one code line per tick, matching the benchmark
        const N: usize = 400;
        let theme = MarkdownTheme::default();
        let src = long_code_doc(N);
        let mut surf = StreamingSurface::new();
        let chars: Vec<char> = src.chars().collect();

        crate::highlight::Highlighter::take_call_count(); // reset
        let mut peak = 0usize;
        let mut end = GRAN;
        while end <= chars.len() {
            let chunk: String = chars[..end].iter().collect();
            surf.render(&chunk, 60, &theme);
            let calls = crate::highlight::Highlighter::take_call_count();
            peak = peak.max(calls);
            end += GRAN;
        }
        surf.render(&src, 60, &theme);
        let calls = crate::highlight::Highlighter::take_call_count();
        peak = peak.max(calls);

        eprintln!("\n[T4b gate] N={N} lines, peak highlight calls/tick = {peak} (budget ≤ 2)");
        // Budget 2: one for the dirty line, one for the close-fence tick where
        // the block may be re-rendered once. The pre-T4b value here is ~N+1.
        assert!(
            peak <= 2,
            "T4b regression: peak highlight calls/tick = {peak}, expected ≤ 2 (was ~{} before T4b)",
            N + 1
        );
    }

    /// T4b must not change the rendered output: the cached surface must remain
    /// byte-identical to the uncached stream at every tick. This is the T4b
    /// correctness gate — it reuses the existing property harness over a
    /// code-block-heavy document.
    #[test]
    fn t4b_cached_output_matches_uncached() {
        // Long fenced code block + prose, exercised at multiple widths so the
        // cache is built, extended, invalidated by width, and rebuilt.
        let code = (1..=30)
            .map(|i| format!("    let x{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let src = format!(
            "```rust\nfn demo() {{\n{code}\n}}\n```\n\nAfter the block, more prose.\n"
        );
        assert_cached_matches_uncached_stream(&src, 30);
        assert_cached_matches_uncached_stream(&src, 60);
        // Mid-stream a second code block to force a tail block-type change
        // (code → paragraph → code), exercising cache invalidation.
        let src2 = format!("{src}\n\n```python\nprint(1)\nprint(2)\n```\n");
        assert_cached_matches_uncached_stream(&src2, 40);
    }

    // ------------------------------------------------------------------
    // Random/bursty chunk streaming — models how an LLM actually emits tokens:
    // not line-by-line, but in variable-length bursts (a few chars, a partial
    // line, sometimes a whole line). This stresses the open-fence highlight
    // cache differently from uniform char or uniform line stepping: the dirty
    // tail line grows and completes across *several* ticks, and lines can be
    // added mid-line. These tests confirm T4b stays correct (byte-identical to
    // uncached) and effective (peak ≤ 2) under that arrival pattern.
    // ------------------------------------------------------------------

    /// A tiny deterministic PRNG (xorshift32) so the random-chunk tests are
    /// reproducible without adding a `rand` dependency. Seed fixed per test.
    fn next_chunk_size(state: &mut u32, max: usize) -> usize {
        // xorshift32
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *state = x;
        // Map to 1..=max (always advance at least one char so the stream ends).
        1 + (x as usize % max)
    }

    /// Drive both a cached and an uncached renderer in lock-step through the
    /// *same* append-only prefix of `src`, but advancing by **random variable
    /// chunk sizes** (1..=`max_chunk` chars per tick) rather than one char or
    /// one line at a time. Asserts the cached output is byte-identical to the
    /// uncached output at every tick.
    ///
    /// The append-only prefix model matches how LLM streaming works: each tick
    /// appends some new text to the body; the parser treats the body as a
    /// growing string, never an arbitrary substring. Variable chunk sizes
    /// emulate bursty token arrival.
    fn assert_cached_matches_uncached_random_chunks(src: &str, width: usize, seed: u32, max_chunk: usize) {
        let theme = MarkdownTheme::default();
        let mut cached = StreamingSurface::new();
        let mut uncached = StreamingSurface::new();
        let chars: Vec<char> = src.chars().collect();
        let mut state = seed;
        let mut end = 0usize;
        loop {
            // Always advance at least one char; the last tick lands exactly on
            // chars.len() so the full document is compared.
            let step = next_chunk_size(&mut state, max_chunk).min(chars.len() - end);
            end += step;
            let chunk: String = chars[..end].iter().collect();
            let c = cached.render(&chunk, width, &theme);
            let u = uncached.render_uncached(&chunk, width, &theme);
            assert_eq!(
                c.rows, u.rows,
                "random-chunk tick at end={end} (chunk {chunk:?}, seed={seed}, max_chunk={max_chunk}): \
                 cached diverged from uncached",
            );
            if end == chars.len() {
                break;
            }
        }
    }

    /// Correctness under bursty token arrival: a long code block streamed in
    /// small random chunks (1..=4 chars/tick) must stay byte-identical to the
    /// uncached stream at every tick. Two different seeds for coverage.
    #[test]
    fn t4b_cached_matches_uncached_random_small_chunks() {
        let code = (1..=40)
            .map(|i| format!("    let x{i} = {i}; // note {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let src = format!(
            "# Heading\n\nintro\n\n```rust\nfn demo() {{\n{code}\n}}\n```\n\ntail prose here.\n"
        );
        assert_cached_matches_uncached_random_chunks(&src, 50, 0x1234_5678, 4);
        assert_cached_matches_uncached_random_chunks(&src, 50, 0xDEAD_BEEF, 4);
    }

    /// Correctness under larger random bursts (1..=12 chars/tick), exercising
    /// the case where a single tick sometimes completes a whole code line and
    /// sometimes spans a line boundary mid-burst.
    #[test]
    fn t4b_cached_matches_uncached_random_bursts() {
        let src = "```rust\nfn a() { let x = 1; }\nfn b() { let y = 2; }\n```\n\nafter\n";
        assert_cached_matches_uncached_random_chunks(src, 30, 0x0BAD_F00D, 12);
        // Two code blocks of different langs, with width change stress.
        let src2 = format!("{src}\n\n```python\nfor i in range(10):\n    print(i)\n```\n");
        assert_cached_matches_uncached_random_chunks(&src2, 40, 0x_C0FFEE, 12);
    }

    /// Perf under bursty arrival: even with tiny random chunks, peak
    /// highlight calls/tick stays ≤ 2 (one dirty line, re-highlighted as it
    /// grows char-by-char across ticks). Pre-T4b this was O(open-block-lines).
    /// Run with: `cargo test -p iodilos-md t4b_random_burst -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn t4b_random_burst_peak_highlights_bounded() {
        const N: usize = 200;
        const MAX_CHUNK: usize = 4; // 1..=4 chars/tick: very bursty
        const SEED: u32 = 0x1234_5678;
        let theme = MarkdownTheme::default();
        let src = long_code_doc(N);
        let chars: Vec<char> = src.chars().collect();
        let mut surf = StreamingSurface::new();

        crate::highlight::Highlighter::take_call_count(); // reset
        let mut peak = 0usize;
        let mut total = 0usize;
        let mut state = SEED;
        let mut end = 0usize;
        loop {
            let step = next_chunk_size(&mut state, MAX_CHUNK).min(chars.len() - end);
            end += step;
            let chunk: String = chars[..end].iter().collect();
            surf.render(&chunk, 60, &theme);
            let calls = crate::highlight::Highlighter::take_call_count();
            peak = peak.max(calls);
            total += calls;
            if end == chars.len() {
                break;
            }
        }
        eprintln!(
            "\n[T4b random-burst] N={N} lines, max_chunk={MAX_CHUNK}, seed={SEED:#x}: \
             peak highlight calls/tick = {peak}, total = {total} (budget peak ≤ 2)"
        );
        assert!(
            peak <= 2,
            "T4b regression under random burst: peak highlight calls/tick = {peak}, expected ≤ 2",
        );
    }

    // ------------------------------------------------------------------
    // Streaming-cost benchmark for the T4b decision (open-fence highlight
    // re-work). Marked #[ignore] — run with `cargo test t4b -- --ignored`.
    // Prints: per-tick highlight calls peak, total calls, quadratic ratio,
    // and worst-tick wall time.
    // ------------------------------------------------------------------

    /// Build a markdown doc that is a single fenced code block of `n_lines`
    /// lines of Rust, followed by a trailing paragraph (so close-fence + the
    /// commit it triggers are exercised). Lines are deliberately non-trivial
    /// (mixed tokens) so syntect does real work, not the plain-text fallback.
    fn long_code_doc(n_lines: usize) -> String {
        let mut s = String::from("```rust\n");
        for i in 0..n_lines {
            // Vary tokens so lines aren't byte-identical (defeats no syntect
            // cache, but is more representative of real generated code).
            s.push_str(&format!(
                "fn step_{i}(x: u32) -> u32 {{ let y = x.wrapping_add({i}); y * 2 }}\n"
            ));
        }
        s.push_str("```\n\ntail paragraph after the block.\n");
        s
    }

    /// Stream `src` into a fresh surface tick-by-tick, recording the highlighter
    /// call count for *each* tick and the wall time of each tick. Returns
    /// aggregate stats.
    ///
    /// `granularity` controls how many chars advance per tick. Real LLM tokens
    /// batch into multi-char chunks (often ~one line at a time once a code
    /// block is open); char-by-char is the pathological worst case and makes
    /// the parser's own O(committed) re-parse dominate, obscuring the render
    /// cost T4b targets.
    #[allow(clippy::type_complexity)]
    fn stream_and_measure(src: &str, width: usize, granularity: usize) -> (usize, usize, f64, std::time::Duration) {
        let theme = MarkdownTheme::default();
        let mut surf = StreamingSurface::new();
        let chars: Vec<char> = src.chars().collect();

        crate::highlight::Highlighter::take_call_count(); // reset
        let mut peak = 0usize;
        let mut total = 0usize;
        let mut worst = std::time::Duration::ZERO;
        let mut n_ticks = 0usize;
        let mut end = granularity;
        while end <= chars.len() {
            let chunk: String = chars[..end].iter().collect();
            let t0 = std::time::Instant::now();
            surf.render(&chunk, width, &theme);
            let dt = t0.elapsed();
            let calls = crate::highlight::Highlighter::take_call_count();
            peak = peak.max(calls);
            total += calls;
            if dt > worst {
                worst = dt;
            }
            n_ticks += 1;
            end += granularity;
        }
        // Final tick with the full source (in case granularity didn't divide evenly).
        let t0 = std::time::Instant::now();
        surf.render(src, width, &theme);
        let dt = t0.elapsed();
        let calls = crate::highlight::Highlighter::take_call_count();
        peak = peak.max(calls);
        total += calls;
        if dt > worst {
            worst = dt;
        }
        n_ticks += 1;
        let avg = total as f64 / n_ticks.max(1) as f64;
        (peak, total, avg, worst)
    }

    #[test]
    #[ignore]
    fn t4b_open_fence_highlight_cost() {
        // The headline question: as the open code block grows, how does the
        // per-tick highlight cost scale? T4b targets the worst-tick peak.
        //
        // Uses per-line granularity (~30 chars/tick = a typical code line).
        // Char-by-char would make the parser's own O(committed) re-parse
        // dominate the wall time, obscuring the *render* cost T4b targets.
        const GRAN: usize = 30;
        eprintln!("\n=== T4b open-fence streaming highlight cost (width=60, {GRAN} chars/tick) ===");
        eprintln!(
            "{:>7} | {:>10} | {:>10} | {:>10} | {:>10} | {:>12}",
            "lines", "totalCalls", "avgPerTick", "peakPerTick", "worstMs", "worstUs"
        );
        for &n in &[20usize, 50, 100, 200, 400] {
            let src = long_code_doc(n);
            let (peak, total, avg, worst) = stream_and_measure(&src, 60, GRAN);
            eprintln!(
                "{:>7} | {:>10} | {:>10.1} | {:>10} | {:>10.2} | {:>12.1}",
                n,
                total,
                avg,
                peak,
                worst.as_secs_f64() * 1000.0,
                worst.as_secs_f64() * 1_000_000.0,
            );
        }
        eprintln!("\nLegend: totalCalls = sum of highlight_line calls over all ticks;");
        eprintln!("        avgPerTick = totalCalls / totalTicks;");
        eprintln!("        peakPerTick = max calls in a single tick (the T4b target).");
        eprintln!("        worstMs/worstUs = wall time of the slowest single tick.");
        eprintln!("Frame budget reference: 16.6ms (60fps), 8.3ms (120fps).");
        eprintln!("Quadratic check: if T4b is justified, peakPerTick should grow ~linearly");
        eprintln!("        with `lines` (it's the open-block re-highlight), and totalCalls");
        eprintln!("        should grow ~quadratically (sum 1+2+...+N).");
    }

    #[test]
    #[ignore]
    fn t4b_isolate_highlight_vs_total() {
        // Isolate how much of the worst-tick wall time is *highlighting* vs
        // everything else (parser re-parse + row rendering). Highlights the
        // 400-line block's 400 lines in isolation, once, and times it. That's
        // an upper bound on the per-tick work T4b caches away (all but the one
        // dirty line). Pre-T4b the full streaming worst tick at 400 lines was
        // 566ms; this shows highlighting alone is the dominant cost there.
        const N: usize = 400;
        let src = long_code_doc(N);
        // Strip the fence markers and trailing paragraph — just the code lines.
        let code_lines: Vec<&str> = src.lines().skip(1).take(N).collect();
        let hl = crate::highlight::Highlighter::new();
        // Reset the call counter so it reflects only the timed loop below.
        crate::highlight::Highlighter::take_call_count();
        let t0 = std::time::Instant::now();
        let mut runs = 0;
        for l in &code_lines {
            let _ = hl.highlight_line(l, "rust");
            runs += 1;
        }
        let dt = t0.elapsed();
        let calls = crate::highlight::Highlighter::take_call_count();
        eprintln!("\n=== highlight-only: {N} lines, {calls} calls, {runs} runs ===");
        eprintln!("total {:.3}ms, per-line {:.3}µs",
            dt.as_secs_f64() * 1000.0,
            dt.as_secs_f64() * 1_000_000.0 / runs as f64,
        );
        eprintln!("Pre-T4b the full streaming worst tick at {N} lines was ~566ms (all N");
        eprintln!("lines re-highlighted each tick); highlighting dominates it, confirming");
        eprintln!("T4b (which caches N-1 of these lines per tick) targets the right cost.");
    }
}
