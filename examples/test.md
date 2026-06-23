# Streaming Markdown in iodilos

This document renders into a View tree of framework components. It is fed
character-by-character to simulate a live token stream, just like an LLM typing
out an answer. Paragraphs are Spans leaves that re-wrap at the terminal width.

## Inline styles

Here is a paragraph with `inline code`, **bold**, *italic*, and a ~~strike~~.
Inline math works too: $E = mc^2$ and $x^2 + y^2 = z^2$.

## Lists

- First item
- Second item with `code`
  - A nested child
- Third item

1. Step one
2. Step two

- [x] Parse Markdown into blocks
- [x] Render each block via framework components
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

## Math

Display math:

$$
\\int_{0}^{\\infty} e^{-x^2} dx = \\frac{\\sqrt{\\pi}}{2}
$$

## A table

| Feature    | Supported |
|------------|:---------:|
| Headings   |    yes    |
| Code       |    yes    |
| Math       |    yes    |
| Tables     |    yes    |
| Inline     |    yes    |
