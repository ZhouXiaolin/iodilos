//! Mermaid → terminal text rendering, modelled after leaf's `mermaid.rs`.
//!
//! `mmdflux` handles the common diagram-to-text path. If a diagram is not
//! supported, the markdown renderer falls back to colored Mermaid source.

use std::fmt::Write;

use iodilos::text::SpanStyle;

use mmdflux::{OutputFormat, RenderConfig, render_diagram};

use crate::theme::MarkdownTheme;

pub(crate) fn render(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("pie") {
        return render_pie(trimmed);
    }
    render_diagram(trimmed, OutputFormat::Text, &RenderConfig::default()).ok()
}

fn render_pie(content: &str) -> Option<String> {
    let mut title = String::new();
    let mut entries: Vec<(String, f64)> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("pie") {
            let rest = rest.trim();
            if let Some(t) = rest.strip_prefix("title") {
                title = t.trim().to_string();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("title") {
            title = rest.trim().to_string();
            continue;
        }
        if let Some((label_part, value_part)) = line.rsplit_once(':')
            && let Ok(value) = value_part.trim().parse::<f64>()
        {
            entries.push((label_part.trim().trim_matches('"').to_string(), value));
        }
    }

    if entries.is_empty() {
        return None;
    }
    let total: f64 = entries.iter().map(|(_, v)| *v).sum();
    if total <= 0.0 {
        return None;
    }

    let max_label_width = entries.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let bar_max = 32;
    let mut out = String::new();

    if !title.is_empty() {
        let _ = writeln!(out, "{title}");
    }

    for (label, value) in &entries {
        let pct = value / total * 100.0;
        let bar_units = pct / 100.0 * bar_max as f64;
        let filled = bar_units as usize;
        let half = (bar_units * 2.0) as usize % 2 == 1;
        let bar: String = "█".repeat(filled) + if half { "▌" } else { "" };
        let _ = writeln!(
            out,
            "{bar:<bw$} {label:<lw$} {pct:>5.1}%",
            bw = bar_max + 1,
            lw = max_label_width,
        );
    }

    Some(out)
}

pub(crate) fn colorize_line(line: &str, theme: &MarkdownTheme) -> Vec<(String, SpanStyle)> {
    let keyword_style = SpanStyle {
        fg: Some(theme.mermaid_keyword),
        ..SpanStyle::default()
    };
    let arrow_style = SpanStyle {
        fg: Some(theme.mermaid_arrow),
        ..SpanStyle::default()
    };
    let label_style = SpanStyle {
        fg: Some(theme.mermaid_label),
        ..SpanStyle::default()
    };
    let default_style = SpanStyle {
        fg: Some(theme.mermaid_text),
        ..SpanStyle::default()
    };

    let mut spans: Vec<(String, SpanStyle)> = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        if let Some(pos) = rest.find('|') {
            let before = &rest[..pos];
            if !before.is_empty() {
                tokenize_segment(
                    before,
                    keyword_style,
                    arrow_style,
                    default_style,
                    &mut spans,
                );
            }
            let after_pipe = &rest[pos + 1..];
            if let Some(end) = after_pipe.find('|') {
                let label_content = &after_pipe[..end];
                spans.push((format!("|{label_content}|"),
                    label_style, ));
                rest = &after_pipe[end + 1..];
            } else {
                spans.push(("|".to_string(), default_style));
                rest = after_pipe;
            }
            continue;
        }

        tokenize_segment(rest, keyword_style, arrow_style, default_style, &mut spans);
        break;
    }

    if spans.is_empty() {
        spans.push((line.to_string(), default_style));
    }
    spans
}

fn tokenize_segment(
    segment: &str,
    keyword_style: SpanStyle,
    arrow_style: SpanStyle,
    default_style: SpanStyle,
    spans: &mut Vec<(String, SpanStyle)>,
) {
    let mut i = 0;
    let bytes = segment.as_bytes();

    while i < bytes.len() {
        if let Some((arrow, len)) = try_match_arrow(&segment[i..]) {
            spans.push((arrow.to_string(), arrow_style));
            i += len;
            continue;
        }

        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
            {
                i += 1;
            }
            let word = &segment[start..i];
            let style = if is_keyword(word) {
                keyword_style
            } else {
                default_style
            };
            spans.push((word.to_string(), style));
            continue;
        }

        let start = i;
        while i < segment.len() {
            let b = bytes[i];
            if b.is_ascii_alphabetic() || b == b'_' || b == b'|' {
                break;
            }
            if try_match_arrow(&segment[i..]).is_some() {
                break;
            }
            if b < 0x80 {
                i += 1;
            } else {
                let ch = segment[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
        }
        if i > start {
            spans.push((segment[start..i].to_string(),
                default_style, ));
        }
    }
}

fn try_match_arrow(s: &str) -> Option<(&'static str, usize)> {
    for pattern in ["-.->", "==>", "-->", "---", "-.-", "-..", "->", "--"] {
        if s.starts_with(pattern) {
            return Some((pattern, pattern.len()));
        }
    }
    None
}

fn is_keyword(word: &str) -> bool {
    is_diagram_keyword(word) || is_direction_keyword(word) || is_structure_keyword(word)
}

fn is_diagram_keyword(word: &str) -> bool {
    matches!(
        word,
        "flowchart"
            | "graph"
            | "sequenceDiagram"
            | "classDiagram"
            | "stateDiagram"
            | "stateDiagram-v2"
            | "erDiagram"
            | "gantt"
            | "pie"
            | "journey"
            | "gitGraph"
            | "mindmap"
            | "timeline"
            | "sankey-beta"
            | "quadrantChart"
            | "requirementDiagram"
            | "C4Context"
            | "block-beta"
            | "xychart-beta"
            | "kanban"
            | "architecture-beta"
    )
}

fn is_direction_keyword(word: &str) -> bool {
    matches!(word, "TB" | "TD" | "BT" | "LR" | "RL")
}

fn is_structure_keyword(word: &str) -> bool {
    matches!(
        word,
        "subgraph"
            | "end"
            | "section"
            | "title"
            | "participant"
            | "actor"
            | "loop"
            | "alt"
            | "else"
            | "opt"
            | "par"
            | "critical"
            | "break"
            | "rect"
            | "note"
            | "activate"
            | "deactivate"
            | "class"
            | "state"
            | "dateFormat"
            | "axisFormat"
            | "style"
            | "classDef"
            | "click"
    )
}
