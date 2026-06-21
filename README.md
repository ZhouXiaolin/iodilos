# iodilos

A standalone reactive terminal UI library, derived from the `sycamore-tui`
package of the [Sycamore](https://github.com/sycamore-rs/sycamore) project.

The reactive primitives (from `sycamore-reactive`) and the component model
(from `sycamore-core`) are vendored inline, so iodilos has no external
`sycamore-*` runtime dependency.

## Layout

```
packages/
  iodilos/          # main crate (runtime + vendored reactive/component)
  iodilos-macros/   # view! proc-macro + inlined view-syntax parser
  iodilos-md/       # streaming Markdown component library (pulldown-cmark + syntect)
examples/           # single-file examples (cargo run --example <name>)
```

## Streaming Markdown

`iodilos-md` parses Markdown into an iodilos `View` tree imperatively (no
`view!` macro), so it rebuilds cleanly when driven from a `Signal<String>`.
The `markdown` example feeds a document in character-by-character to simulate a
token stream:

```
cargo run --example markdown
```

Keys: `↑`/`↓` and mouse wheel scroll, `PgUp`/`PgDn` page, `F` toggles
follow-the-tail, `Q` quits.


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

Run an example:

```
cargo run --example counter
```
