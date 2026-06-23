//! A skinned numeric calculator, fully decomposed into components.
//!
//! Run with: `cargo run --example calculator`.

use crossterm::event::{KeyCode, KeyEventKind};
use iodilos::prelude::*;

// ----- theme + styles -------------------------------------------------------

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy)]
struct ButtonStyle {
    color: Color,
    text_color: Color,
    trim_color: Color,
}

impl Theme {
    fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    fn background_color(self) -> Color {
        match self {
            Self::Dark => Color::AnsiValue(237),
            Self::Light => Color::AnsiValue(253),
        }
    }

    fn footer_background_color(self) -> Color {
        match self {
            Self::Dark => Color::AnsiValue(253),
            Self::Light => Color::AnsiValue(237),
        }
    }

    fn footer_text_color(self) -> Color {
        match self {
            Self::Dark => Color::AnsiValue(237),
            Self::Light => Color::AnsiValue(253),
        }
    }

    fn screen_color(self) -> Color { Color::AnsiValue(68) }
    fn screen_text_color(self) -> Color { Color::AnsiValue(231) }
    fn screen_trim_color(self) -> Color { Color::AnsiValue(75) }

    fn numpad(self) -> ButtonStyle {
        match self {
            Self::Dark => ButtonStyle {
                color: Color::AnsiValue(239),
                text_color: Color::AnsiValue(231),
                trim_color: Color::AnsiValue(243),
            },
            Self::Light => ButtonStyle {
                color: Color::AnsiValue(251),
                text_color: Color::AnsiValue(16),
                trim_color: Color::AnsiValue(255),
            },
        }
    }

    fn operator(self) -> ButtonStyle {
        ButtonStyle {
            color: Color::AnsiValue(172),
            text_color: Color::AnsiValue(231),
            trim_color: Color::AnsiValue(215),
        }
    }

    fn clear(self) -> ButtonStyle {
        ButtonStyle {
            color: Color::AnsiValue(161),
            text_color: Color::AnsiValue(231),
            trim_color: Color::AnsiValue(205),
        }
    }

    fn function(self) -> ButtonStyle {
        ButtonStyle {
            color: Color::AnsiValue(66),
            text_color: Color::AnsiValue(231),
            trim_color: Color::AnsiValue(115),
        }
    }
}

// ----- action model ---------------------------------------------------------

#[derive(Clone, Copy)]
struct CalculatorActions {
    expr: Signal<String>,
    clear_on_number: Signal<bool>,
}

impl CalculatorActions {
    fn backspace(self) {
        let current = self.expr.get_clone();
        let mut chars = current.chars().collect::<Vec<_>>();
        chars.pop();
        let next = chars.into_iter().collect::<String>();
        if next.is_empty() {
            self.expr.set("0".to_string());
            self.clear_on_number.set(true);
        } else {
            self.expr.set(next);
            self.clear_on_number.set(false);
        }
    }

    fn number(self, n: u8) {
        if self.clear_on_number.get() {
            self.expr.set(n.to_string());
            self.clear_on_number.set(false);
        } else {
            self.expr.set(format!("{}{n}", self.expr.get_clone()));
        }
    }

    fn decimal(self) {
        if self.clear_on_number.get() {
            self.expr.set("0.".to_string());
            self.clear_on_number.set(false);
        } else if !current_number_has_decimal(&self.expr.get_clone()) {
            self.expr.set(format!("{}.", self.expr.get_clone()));
        }
    }

    fn clear(self) {
        self.expr.set("0".to_string());
        self.clear_on_number.set(true);
    }

    fn operator(self, op: char) {
        if self.clear_on_number.get() {
            self.clear_on_number.set(false);
        }
        if !has_trailing_operator(&self.expr.get_clone()) {
            self.expr.set(format!("{}{op}", self.expr.get_clone()));
        }
    }

    fn percent(self) {
        if self.clear_on_number.get() {
            self.clear_on_number.set(false);
        }
        if !has_trailing_operator(&self.expr.get_clone()) {
            self.expr.set(format!("{}%", self.expr.get_clone()));
        }
    }

    fn plus_minus(self) {
        if self.clear_on_number.get() {
            self.clear_on_number.set(false);
        }
        if !has_trailing_operator(&self.expr.get_clone()) {
            self.expr.set(format!("-({})", self.expr.get_clone()));
        }
    }

    fn equals(self) {
        let expression = self.expr.get_clone().replace('×', "*").replace('÷', "/");
        if let Ok(value) = mexprp::eval::<f64>(&expression) {
            self.expr.set(value.to_string());
            self.clear_on_number.set(true);
        }
    }

    fn key(self, event: Event) {
        let Some(key) = event.key() else { return; };
        if key.kind == KeyEventKind::Release { return; }
        match key.code {
            KeyCode::Char('/') => self.operator('÷'),
            KeyCode::Char('*') => self.operator('×'),
            KeyCode::Char('+') => self.operator('+'),
            KeyCode::Char('-') => self.operator('-'),
            KeyCode::Char(c @ '0'..='9') => self.number((c as u8) - b'0'),
            KeyCode::Char('.') => self.decimal(),
            KeyCode::Char('%') => self.percent(),
            KeyCode::Char('=') | KeyCode::Enter => self.equals(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Char('c') => self.clear(),
            _ => {}
        }
    }
}

fn has_trailing_operator(expr: &str) -> bool {
    matches!(expr.chars().last(), Some('+' | '-' | '×' | '÷'))
}

fn current_number_has_decimal(expr: &str) -> bool {
    expr.rsplit(['+', '-', '×', '÷'])
        .next()
        .is_some_and(|n| n.contains('.'))
}

// ----- view components ------------------------------------------------------

/// The expression display at the top of the calculator.
#[component(inline_props)]
fn Screen(content: ReadSignal<String>, theme: ReadSignal<Theme>) -> View {
    view! {
        div(
            width = Size::Percent(100.0),
            border_style = BorderStyle::Custom(BorderCharacters { top: '▁', ..Default::default() }),
            border_edges = Edges::TOP,
            border_color = move || theme.get().screen_trim_color(),
        ) {
            div(
                width = Size::Percent(100.0),
                background_color = move || theme.get().screen_color(),
                padding = 1,
                justify_content = JustifyContent::END,
            ) {
                p(color = move || theme.get().screen_text_color()) { (content) }
            }
        }
    }
}

/// One skinned key. `on_click` runs when the button fires.
#[component(inline_props)]
fn Key(
    label: &'static str,
    style: ButtonStyle,
    on_click: impl FnMut(Event) + 'static,
) -> View {
    view! {
        button(
            on:click = on_click,
            flex_grow = 1.0_f32,
            margin_left = 1,
            margin_right = 1,
            border_style = BorderStyle::Custom(BorderCharacters { top: '▁', ..Default::default() }),
            border_edges = Edges::TOP,
            border_color = style.trim_color,
            padding = 0,
        ) {
            div(
                background_color = style.color,
                justify_content = JustifyContent::CENTER,
                align_items = AlignItems::CENTER,
                height = 3,
                flex_grow = 1.0_f32,
                width = Size::Percent(100.0),
            ) {
                span(color = style.text_color, weight = Weight::Bold) { (label) }
            }
        }
    }
}

/// One row of four `Key`s — its children are populated by the caller.
#[component(inline_props)]
fn KeyRow(children: Children<View>) -> View {
    let children = children.call();
    view! {
        div(flex_direction = FlexDirection::Row, width = Size::Percent(100.0)) {
            (children)
        }
    }
}

/// The bottom statusline.
#[component(inline_props)]
fn Footer(theme: ReadSignal<Theme>) -> View {
    view! {
        div(
            height = 1,
            background_color = move || theme.get().footer_background_color(),
            padding_left = 1,
        ) {
            p(color = move || theme.get().footer_text_color()) { "[T] Toggle Theme [Q] Quit" }
        }
    }
}

/// The 4×5 button pad + screen, parameterised by the current theme.
#[component(inline_props)]
fn Calculator(theme: ReadSignal<Theme>) -> View {
    let expr = create_signal(String::from("0"));
    let clear_on_number = create_signal(true);
    let actions = CalculatorActions { expr, clear_on_number };
    let content = create_memo(move || expr.get_clone());

    // Style buckets recomputed on each rebuild — flipping the theme rebuilds
    // the calculator subtree, which is cheap (no scope churn for inner keys).
    let current = theme.get();
    let numpad = current.numpad();
    let op = current.operator();
    let func = current.function();
    let clr = current.clear();

    view! {
        div(
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            flex_direction = FlexDirection::Column,
            padding_left = 1,
            padding_right = 1,
            on:raw_key = move |event: Event| actions.key(event),
        ) {
            div(padding_left = 1, padding_right = 1) {
                Screen(content = content, theme = theme)
            }

            KeyRow {
                Key(label = "←", style = func, on_click = move |_| actions.backspace())
                Key(label = "±", style = func, on_click = move |_| actions.plus_minus())
                Key(label = "%", style = func, on_click = move |_| actions.percent())
                Key(label = "÷", style = op,   on_click = move |_| actions.operator('÷'))
            }
            KeyRow {
                Key(label = "7", style = numpad, on_click = move |_| actions.number(7))
                Key(label = "8", style = numpad, on_click = move |_| actions.number(8))
                Key(label = "9", style = numpad, on_click = move |_| actions.number(9))
                Key(label = "×", style = op,     on_click = move |_| actions.operator('×'))
            }
            KeyRow {
                Key(label = "4", style = numpad, on_click = move |_| actions.number(4))
                Key(label = "5", style = numpad, on_click = move |_| actions.number(5))
                Key(label = "6", style = numpad, on_click = move |_| actions.number(6))
                Key(label = "-", style = op,     on_click = move |_| actions.operator('-'))
            }
            KeyRow {
                Key(label = "1", style = numpad, on_click = move |_| actions.number(1))
                Key(label = "2", style = numpad, on_click = move |_| actions.number(2))
                Key(label = "3", style = numpad, on_click = move |_| actions.number(3))
                Key(label = "+", style = op,     on_click = move |_| actions.operator('+'))
            }
            KeyRow {
                Key(label = "C", style = clr,    on_click = move |_| actions.clear())
                Key(label = "0", style = numpad, on_click = move |_| actions.number(0))
                Key(label = ".", style = numpad, on_click = move |_| actions.decimal())
                Key(label = "=", style = op,     on_click = move |_| actions.equals())
            }
        }
    }
}

// ----- top-level app --------------------------------------------------------

#[component]
fn App() -> View {
    let theme = create_signal(Theme::default());

    view! {
        div(
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            background_color = move || theme.get().background_color(),
            flex_direction = FlexDirection::Column,
            gap = 1,
            on:raw_key = move |event: Event| {
                if let Some(key) = event.key()
                    && key.kind != KeyEventKind::Release
                    && matches!(key.code, KeyCode::Char('t'))
                {
                    theme.set(theme.get().toggled());
                    event.stop_propagation();
                }
            },
        ) {
            div(flex_grow = 1.0_f32) {
                div(max_width = 120, max_height = 40, flex_grow = 1.0_f32) {
                    // The calculator subtree rebuilds when the theme flips —
                    // a thin dynamic region so the inner button styles refresh.
                    (move || view! { Calculator(theme = *theme) })
                }
            }
            Footer(theme = *theme)
        }
    }
}

fn main() -> std::io::Result<()> {
    render(App)
}
