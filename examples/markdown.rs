//! Streaming Markdown viewer demo (LineFlow edition).
//!
//! A fixed Markdown document is fed in character-by-character (simulating an
//! LLM token stream). Each tick mutates a `Signal<String>`; a reactive memo
//! re-renders the markdown into `Vec<Line>` at the terminal width and feeds a
//! `LineFlow`. Scroll offset is a `Signal<i32>`; arrow keys / mouse wheel /
//! PgUp-PgDn change it; `F` toggles follow-the-tail.
//!
//! Keys: ↑/↓ scroll 1 line, PgUp/PgDn scroll a page, wheel scrolls, `F`
//! toggles follow, `Q` quits.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind, MouseEventKind};
use crossterm::terminal::size as term_size;
use iodilos::node::TuiNode;
use iodilos::prelude::*;
use iodilos_md::{MarkdownTheme, StreamingParser};
use tokio::time::sleep;

const SAMPLE_MD: &str = "\
# Streaming Markdown in iodilos

This document renders into a single LineFlow. It is fed character-by-character
to simulate a live token stream, just like an LLM typing out an answer.

## Inline styles

Here is a paragraph with `inline code`, **bold**, *italic*, and a ~~strike~~.

## Lists

- First item
- Second item with `code`
  - A nested child
- Third item

1. Step one
2. Step two

- [x] Parse Markdown into blocks
- [x] Render each block to Lines
- [ ] Ship to production

## A quote

> Simplicity is prerequisite for reliability.

---

## Code with highlighting

```rust
fn fib(n: u32) -> u32 {
    match n {
        0 | 1 => 1,
        _ => fib(n - 1) + fib(n - 2),
    }
}
```

## A table

| Feature    | Supported |
|------------|:---------:|
| Headings   |    yes    |
| Code       |    yes    |
| Tables     |    yes    |
| Inline     |    yes    |
";

const CHROME_ROWS: i32 = 3; // status bar + viewport borders
const WHEEL_LINES: i32 = 5;

fn app() -> View {
    let content = create_signal(String::new());
    let offset = create_signal(0i32);
    let follow = create_signal(true);
    let done = create_signal(false);
    let theme = MarkdownTheme::default();

    let (init_cols, init_rows) = term_size().unwrap_or((80, 24));
    let term_cols = create_signal(init_cols);
    let term_rows = create_signal(init_rows);

    use_future(async move {
        let chars: Vec<char> = SAMPLE_MD.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            let end = (i + 3).min(chars.len());
            let chunk: String = chars[i..end].iter().collect();
            let mut buf = content.get_clone();
            buf.push_str(&chunk);
            content.set(buf);
            i = end;
            sleep(Duration::from_millis(25)).await;
        }
        done.set(true);
    });

    let visible_rows =
        create_memo(move || (term_rows.get() as i32).saturating_sub(CHROME_ROWS).max(1));

    // Incremental parser held outside the memo so its committed-prefix cache
    // survives every rebuild; each tick re-parses only the open tail.
    let parser = std::rc::Rc::new(std::cell::RefCell::new(StreamingParser::new()));
    let lines = create_memo(move || {
        let width = (term_cols.get() as i32).saturating_sub(4).max(1) as usize;
        parser.borrow_mut().feed_to_lines(&content.get_clone(), width, &theme)
    });
    let total_lines = create_memo(move || lines.get_clone().len() as i32);
    let max_offset =
        create_memo(move || total_lines.get().saturating_sub(visible_rows.get()).max(0));
    let top_offset = create_memo(move || {
        if follow.get() {
            max_offset.get()
        } else {
            offset.get().min(max_offset.get())
        }
    });

    let scroll_by = move |delta: i32| {
        follow.set(false);
        let max = max_offset.get();
        offset.update(|o| *o = (*o + delta).clamp(0, max));
    };

    view! {
        div(
            flex_direction = FlexDirection::Column,
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            tabindex = "0",
            on:terminal_resize = move |event: Event| {
                if let Some((cols, rows)) = event.resize() {
                    term_cols.set(cols);
                    term_rows.set(rows);
                }
            },
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release { return; }
                match key.code {
                    KeyCode::Up => scroll_by(-1),
                    KeyCode::Down => scroll_by(1),
                    KeyCode::PageUp => scroll_by(-visible_rows.get()),
                    KeyCode::PageDown => scroll_by(visible_rows.get()),
                    KeyCode::Char('f') | KeyCode::Char('F') => follow.set(true),
                    _ => {}
                }
            },
            on:raw_mouse = move |event: Event| {
                let Some(mouse) = event.mouse() else { return; };
                match mouse.kind {
                    MouseEventKind::ScrollUp => scroll_by(-WHEEL_LINES),
                    MouseEventKind::ScrollDown => scroll_by(WHEEL_LINES),
                    _ => {}
                }
            },
        ) {
            div(
                flex_grow = 1.0_f32,
                overflow = Overflow::Hidden,
                border_style = BorderStyle::Single,
                border_color = Color::DarkGrey,
            ) {
                (move || {
                    // Build a LineFlow carrying the current lines + offset.
                    View::from_node(TuiNode::create_line_flow_node(
                        lines.get_clone(),
                        top_offset.get(),
                    ))
                })
            }
            div(
                flex_direction = FlexDirection::Row,
                column_gap = 2,
                height = 1,
                border_style = BorderStyle::Single,
                border_color = Color::DarkGrey,
                border_edges = Edges::TOP,
                padding_left = 1,
            ) {
                p(color = Color::DarkGrey) {
                    (move || if done.get_clone() { "✓ stream complete" } else { "… streaming" })
                }
                p(color = Color::DarkGrey) {
                    (move || if follow.get() { "[F] following" } else { "[F] follow off" })
                }
                p(color = Color::DarkGrey) { "↑/↓ scroll  [Q] quit" }
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    iodilos::render_async(app).await
}
