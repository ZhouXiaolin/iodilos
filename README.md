# iodilos

A standalone reactive terminal UI library (reactive primitives derived from
Sycamore). Two component libraries are being prepared for the **flown** project:

- **Reactive core** — signals, memos, effects with automatic dependency tracking
- **Framebuffer renderer** — Cell-based framebuffer with frame-diffing ANSI output via crossterm, minimal writes
- **Cell producers** — two-phase leaf model (`Plain` char-wrap, `Lines` pre-wrapped styled runs) that shape content into Cell rows at layout time
- **Flexbox layout** — powered by taffy
- **`view!` macro** — declarative UI DSL

## iodilos-md — streaming Markdown

Parses Markdown into a `Lines` producer (pre-wrapped styled runs) — syntax highlighting
via `syntect`, inline LaTeX via `unicodeit`, Mermaid flowcharts via `mmdflux`.
Driven from a `Signal<String>` + width signal:

```
cargo run --example markdown
```

Keys: `↑`/`↓` and mouse wheel scroll, `PgUp`/`PgDn` page, `F` toggles
follow-the-tail, `Q` quits.

## iodilos-prompt — framed multiline prompt

Statusline + rounded-frame prompt box with block cursor and `PromptModel`
editing model. Pure rendering; reactive wiring left to the application:

```
cargo run --example prompt_box
```

Keys: printable chars insert, `Backspace` deletes, `←`/`→` move cursor,
`Shift+Enter`/`Alt+Enter` newline, `Enter` submits, `Ctrl+C` quits.

## Layout

```
packages/
  iodilos/          # main crate (runtime + reactive + layout + canvas)
  iodilos-macros/   # view! proc-macro
  iodilos-md/       # streaming Markdown (for flown)
  iodilos-prompt/   # framed multiline prompt (for flown)
examples/           # cargo run --example <name>
```

## Quick start

```rust
use iodilos::prelude::*;

fn app() -> View {
    view! {
        div(border_style = BorderStyle::Round, border_color = Color::Blue) {
            p { "Hello, world!" }
        }
    }
}

fn main() -> std::io::Result<()> {
    render(app)
}
```

```
cargo run --example counter
```
