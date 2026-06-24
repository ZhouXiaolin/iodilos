//! jsonl session replay — a `Tabled` baseline demo.
//!
//! Run with:
//!   cargo run --example jsonl_replay -- --mode=static
//!   cargo run --example jsonl_replay -- --mode=append
//!   cargo run --example jsonl_replay -- --mode=stream
//!   cargo run --example jsonl_replay -- --input=<path> --mode=stream
//!
//! This example replays a flown agent session JSONL (one JSON object per line)
//! and renders each `message` entry as a keyed `Tabled` row. It is the
//! baseline for the `Tabled` refactor described in `docs/tabled-design.md`:
//! each future optimisation (taffy-tree persistence, producer caching, cell
//! diff, Synchronized Output, the inline+scrollback paradigm) will be measured
//! against these three modes.
//!
//! Rendering is deliberately naive (design §8): a colored role badge, the
//! message body text truncated to a few lines, no markdown / no tool-result
//! expansion / no avatars.
//!
//! Modes:
//! - `static`  — load all messages up front; ↑/↓ move the selection. Verifies
//!   keyed reuse + sticky window + attribute-level selection reactivity.
//! - `append`  — push messages one at a time on their original timestamp delta;
//!   selection follows the tail. Verifies that appending doesn't re-map
//!   existing rows.
//! - `stream`  — like `append`, but the assistant content is sliced into ~20
//!   char chunks emitted every 60ms. Verifies that re-typing the last row
//!   doesn't rebuild its siblings' subtrees.

use std::env;
use std::fs;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind};
use iodilos::prelude::*;
use iodilos::{FlatRow, TableSection};
use serde::Deserialize;
use tokio::time::sleep;

const DEFAULT_INPUT: &str = "/home/solaren/.flown/agent/sessions/--home-solaren-flown--/2026-06-22T18-09-41-202503048+00-00_019ef086-1b52-7730-bb52-ebdf68d6e900.jsonl";
const MAX_BODY_LINES: usize = 5;

// ----- data layer -----------------------------------------------------------

/// A content block inside a message. The flown JSONL carries content either as
/// a plain string (user text) or an array of typed blocks (assistant tool
/// calls, tool results, text). We keep only what the naive renderer needs.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ContentBlock {
    /// A plain-text block: `{"text": "..."}`.
    Text { text: String },
    /// A tool-call block: `{"name": "...", "arguments": {...}}`.
    ToolCall { name: String, arguments: serde_json::Value },
    /// Anything else — capture its JSON so the badge is still informative.
    Other(serde_json::Value),
}

#[derive(Clone, Debug, Deserialize)]
struct Message {
    role: String,
    #[serde(default)]
    content: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
struct Record {
    // `type` is parsed (e.g. "message", "active_tools_change") so non-message
    // records are distinguishable even though we only render `message` ones.
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    id: String,
    #[serde(default)]
    message: Option<Message>,
}

/// One replayable row: the message id (key), role badge text + color, and the
/// flattened body text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayRow {
    id: String,
    badge: &'static str,
    badge_color: Color,
    body: String,
}

/// Parse a record's content into a single flattened body string.
fn body_of(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| {
                let Ok(block) = serde_json::from_value::<ContentBlock>(b.clone()) else {
                    return None;
                };
                match block {
                    ContentBlock::Text { text } => Some(text),
                    ContentBlock::ToolCall { name, arguments } => {
                        Some(format!("[tool:{name}] {}", serde_json::to_string(&arguments).unwrap_or_default()))
                    }
                    ContentBlock::Other(v) => Some(serde_json::to_string(&v).unwrap_or_default()),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn badge_for(role: &str) -> (&'static str, Color) {
    match role {
        "user" => ("user", Color::Cyan),
        "assistant" => ("asst", Color::Magenta),
        "toolResult" | "tool_result" | "tool" => ("tool", Color::Green),
        "system" => ("sys", Color::DarkGrey),
        _ => ("?", Color::Grey),
    }
}

/// Truncate a body to at most `max_lines` lines, marking overflow with `…`.
fn truncate(body: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= max_lines {
        body.to_string()
    } else {
        let mut out = lines[..max_lines].join("\n");
        out.push('\n');
        out.push('…');
        out
    }
}

fn row_of(record: &Record) -> Option<ReplayRow> {
    let msg = record.message.as_ref()?;
    let (badge, badge_color) = badge_for(&msg.role);
    Some(ReplayRow {
        id: record.id.clone(),
        badge,
        badge_color,
        body: truncate(&body_of(&msg.content), MAX_BODY_LINES),
    })
}

fn load_rows(path: &str) -> Result<Vec<ReplayRow>, String> {
    let data = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let mut rows = Vec::new();
    for (i, line) in data.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Record =
            serde_json::from_str(line).map_err(|e| format!("line {}: parse: {e}", i + 1))?;
        if let Some(row) = row_of(&record) {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(format!("no message records found in {path}"));
    }
    Ok(rows)
}

// ----- view ----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Static,
    Append,
    Stream,
}

#[component(inline_props)]
fn App(rows: Vec<ReplayRow>, mode: Mode) -> View {
    let items = create_signal(Vec::<ReplayRow>::new());
    let selected = create_signal(None::<String>);

    // Start a thread of work that fills `items` per the chosen mode.
    spawn_replay(items, selected, rows, mode);

    // sections: a single untitled section so the whole list is one group.
    let sections = create_memo(move || vec![TableSection::new(items.get_clone())]);

    let move_selection = {
        let items = items;
        let selected = selected;
        move |delta: i32| {
            let rows = items.get_clone();
            if rows.is_empty() {
                return;
            }
            let cur = selected.get_clone();
            let idx = match cur {
                Some(id) => rows.iter().position(|r| r.id == id).map(|i| i as i32),
                None => None,
            };
            let base = idx.unwrap_or(if delta > 0 { -1 } else { 0 });
            let next = (base + delta).clamp(0, rows.len() as i32 - 1) as usize;
            selected.set(Some(rows[next].id.clone()));
        }
    };

    view! {
        div(
            flex_direction = FlexDirection::Column,
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            tabindex = "0",
            on:raw_key = move |event: Event| {
                let Some(key) = event.key() else { return; };
                if key.kind == KeyEventKind::Release { return; }
                match key.code {
                    KeyCode::Up => move_selection(-1),
                    KeyCode::Down => move_selection(1),
                    _ => {}
                }
            },
        ) {
            div(
                flex_direction = FlexDirection::Row,
                column_gap = 2,
                padding_left = 1,
                padding_right = 1,
                border_style = BorderStyle::Single,
                border_color = Color::DarkGrey,
                border_edges = Edges::BOTTOM,
            ) {
                p(weight = Weight::Bold) { "jsonl replay" }
                p(color = Color::DarkGrey) {
                    (match mode {
                        Mode::Static => "mode: static",
                        Mode::Append => "mode: append",
                        Mode::Stream => "mode: stream",
                    })
                }
                p(color = Color::DarkGrey) { "↑/↓ select · q quit" }
            }
            div(flex_grow = 1.0_f32, width = Size::Percent(100.0), overflow = Overflow::Hidden) {
                Tabled(
                    sections = sections,
                    selected = *selected,
                    max_visible = 20,
                    key = |row: &ReplayRow| row.id.clone(),
                    view = |row: FlatRow<ReplayRow, String>| match row {
                        FlatRow::Body { item, .. } => {
                            let badge = item.badge;
                            let badge_color = item.badge_color;
                            let body = item.body.clone();
                            view! {
                                div(
                                    flex_direction = FlexDirection::Row,
                                    width = Size::Percent(100.0),
                                    column_gap = 1,
                                    background_color = Color::Reset,
                                ) {
                                    p(color = badge_color, weight = Weight::Bold) { (badge) }
                                    p { (body) }
                                }
                            }
                        }
                        FlatRow::Header { .. } => view! { p { "" } },
                    },
                )
            }
        }
    }
}

/// Drive `items` (and `selected` for follow-tail) according to `mode`.
fn spawn_replay(
    items: Signal<Vec<ReplayRow>>,
    selected: Signal<Option<String>>,
    rows: Vec<ReplayRow>,
    mode: Mode,
) {
    use_future(async move {
        match mode {
            Mode::Static => {
                // One shot: load all, select the first row.
                items.set(rows.clone());
                selected.set(Some(rows[0].id.clone()));
            }
            Mode::Append => {
                // Push rows one at a time (~40ms cadence), selecting the newest.
                for row in rows {
                    items.update(|v| v.push(row.clone()));
                    selected.set(Some(row.id));
                    sleep(Duration::from_millis(40)).await;
                }
            }
            Mode::Stream => {
                // Push rows one at a time; for the LAST row (if it's an
                // assistant message), re-type its body in ~20-char chunks every
                // 60ms to simulate token streaming.
                let total = rows.len();
                let mut pushed: Vec<ReplayRow> = Vec::new();
                for (i, row) in rows.into_iter().enumerate() {
                    pushed.push(row.clone());
                    items.set(pushed.clone());

                    let is_last = i + 1 == total;
                    if is_last && row.badge == "asst" {
                        // Stream the final assistant message in chunks.
                        stream_body(items, row.id.clone(), row.body.clone()).await;
                    } else {
                        selected.set(Some(row.id.clone()));
                        sleep(Duration::from_millis(40)).await;
                    }
                }
            }
        }
    });
}

/// Re-write the last row's body in ~20-char chunks to simulate token streaming.
async fn stream_body(items: Signal<Vec<ReplayRow>>, id: String, full: String) {
    let chunks: Vec<String> = full
        .as_bytes()
        .chunks(20)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    let mut acc = String::new();
    for chunk in chunks {
        acc.push_str(&chunk);
        items.update(|v| {
            if let Some(row) = v.last_mut() {
                if row.id == id {
                    row.body = truncate(&acc, MAX_BODY_LINES);
                }
            }
        });
        sleep(Duration::from_millis(60)).await;
    }
}

// ----- entrypoint ----------------------------------------------------------

fn parse_args() -> Result<(String, Mode), String> {
    let mut input = DEFAULT_INPUT.to_string();
    let mut mode = Mode::Static;
    for arg in env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--input=") {
            input = v.to_string();
        } else if let Some(v) = arg.strip_prefix("--mode=") {
            mode = match v {
                "static" => Mode::Static,
                "append" => Mode::Append,
                "stream" => Mode::Stream,
                other => return Err(format!("unknown --mode={other} (static|append|stream)")),
            };
        } else if arg == "-h" || arg == "--help" {
            println!("usage: jsonl_replay [--input=<path>] [--mode=static|append|stream]");
            std::process::exit(0);
        } else {
            return Err(format!("unknown argument: {arg}"));
        }
    }
    Ok((input, mode))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let (input, mode) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let rows = match load_rows(&input) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    render_async(move || view! { App(rows = rows, mode = mode) }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_maps_known_roles() {
        assert_eq!(badge_for("user"), ("user", Color::Cyan));
        assert_eq!(badge_for("assistant"), ("asst", Color::Magenta));
        assert_eq!(badge_for("toolResult"), ("tool", Color::Green));
        assert_eq!(badge_for("system"), ("sys", Color::DarkGrey));
        assert_eq!(badge_for("wat"), ("?", Color::Grey));
    }

    #[test]
    fn truncate_marks_overflow() {
        let five = "a\nb\nc\nd\ne";
        assert_eq!(truncate(five, 5), five);
        let six = "a\nb\nc\nd\ne\nf";
        let out = truncate(six, 5);
        assert_eq!(out.matches('…').count(), 1);
        assert!(out.lines().count() <= 6); // 5 lines + ellipsis line
        assert!(!out.ends_with('e'));
    }

    #[test]
    fn body_of_flattens_string_and_array() {
        // Plain string content.
        assert_eq!(body_of(&serde_json::json!("hello")), "hello");
        // Array of a text block and a tool-call block.
        let arr = serde_json::json!([
            { "text": "running pwd" },
            { "name": "bash", "arguments": { "command": "pwd" } },
        ]);
        let body = body_of(&arr);
        assert!(body.contains("running pwd"), "text block present: {body}");
        assert!(body.contains("[tool:bash]"), "tool-call block present: {body}");
    }

    #[test]
    fn row_of_parses_real_user_message_record() {
        // A real-shape user message record from a flown session JSONL.
        let line = r#"{"type":"message","id":"019ef086-36ee-7970-b002-09371fc57ab0","timestamp":"2026-06-22T18:09:48.270536184+00:00","message":{"role":"user","content":"当前目录是什么","timestamp":"2026-06-22T18:09:48.236581552Z"}}"#;
        let record: Record = serde_json::from_str(line).unwrap();
        assert_eq!(record.kind, "message");
        let row = row_of(&record).expect("user record yields a row");
        assert_eq!(row.badge, "user");
        assert_eq!(row.badge_color, Color::Cyan);
        assert_eq!(row.body, "当前目录是什么");
    }

    #[test]
    fn row_of_skips_non_message_records() {
        // An active_tools_change record has no `message` → no row.
        let line = r#"{"type":"active_tools_change","id":"abc","timestamp":"t","activeToolNames":["bash"]}"#;
        let record: Record = serde_json::from_str(line).unwrap();
        assert!(row_of(&record).is_none());
    }

    #[test]
    fn load_rows_parses_multiline_session() {
        let tmp = std::env::temp_dir().join("jsonl_replay_test.jsonl");
        let content = concat!(
            r#"{"type":"session","version":3,"id":"s1","timestamp":"t"}"#, "\n",
            r#"{"type":"active_tools_change","id":"a1","timestamp":"t","activeToolNames":["bash"]}"#, "\n",
            r#"{"type":"message","id":"m1","timestamp":"t","message":{"role":"user","content":"hi"}}"#, "\n",
            r#"{"type":"message","id":"m2","timestamp":"t","message":{"role":"assistant","content":[{"text":"hello"}]}}"#, "\n",
        );
        std::fs::write(&tmp, content).unwrap();
        let rows = load_rows(tmp.to_str().unwrap()).unwrap();
        assert_eq!(rows.len(), 2, "only message records become rows");
        assert_eq!(rows[0].id, "m1");
        assert_eq!(rows[0].badge, "user");
        assert_eq!(rows[1].badge, "asst");
        assert_eq!(rows[1].body, "hello");
        let _ = std::fs::remove_file(&tmp);
    }
}
