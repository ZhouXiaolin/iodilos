use std::time::Duration;

use crossterm::event::{KeyCode, KeyEventKind};
use iodilos::prelude::*;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::sleep;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct LocationData {
    lat: f64,
    lon: f64,
    country: String,
    region: String,
    city: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct WeatherDataCurrent {
    temperature_2m: f32,
    relative_humidity_2m: f32,
    precipitation_probability: f32,
    weather_code: i32,
}

impl WeatherDataCurrent {
    fn description(&self) -> &'static str {
        match self.weather_code {
            0 => "Clear",
            1 => "Mainly Clear",
            2 => "Partly Cloudy",
            3 => "Overcast",
            45 => "Fog",
            48 => "Depositing Rime Fog",
            51 => "Light Drizzle",
            53 => "Moderate Drizzle",
            55 => "Dense Drizzle",
            56 => "Light Freezing Drizzle",
            57 => "Dense Freezing Drizzle",
            61 => "Light Rain",
            63 => "Moderate Rain",
            65 => "Heavy Rain",
            66 => "Light Freezing Rain",
            67 => "Heavy Freezing Rain",
            71 => "Light Snow",
            73 => "Moderate Snow",
            75 => "Heavy Snow",
            77 => "Flurries",
            80 => "Slight Rain Showers",
            81 => "Moderate Rain Showers",
            82 => "Violent Rain Showers",
            85 => "Slight Snow Showers",
            86 => "Heavy Snow Showers",
            95 => "Thunderstorm",
            96 => "Thunderstorm With Slight Hail",
            97 => "Thunderstorm With Heavy Hail",
            _ => "Unknown",
        }
    }

    fn color(&self) -> Color {
        match self.weather_code {
            0 | 1 => Color::Yellow,
            2 | 3 => Color::Grey,
            45 | 48 | 71 | 73 | 75 | 77 | 85 | 86 => Color::White,
            56 | 57 | 66 | 67 => Color::Blue,
            51 | 53 | 55 | 61 | 63 | 65 | 80 | 81 | 82 => Color::Cyan,
            95..=97 => Color::Yellow,
            _ => Color::White,
        }
    }

    fn icon(&self) -> &'static str {
        match self.weather_code {
            0 | 1 => "sun",
            2 => "partly cloudy",
            3 => "cloud",
            45 | 48 => "fog",
            56 | 57 | 66 | 67 => "freezing rain",
            51 | 53 | 55 | 61 | 63 | 65 | 80 | 81 | 82 => "rain",
            71 | 77 => "snow",
            73 | 75 | 85 | 86 => "snow shower",
            95..=97 => "storm",
            _ => "unknown",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct WeatherDataCurrentUnits {
    temperature_2m: String,
    relative_humidity_2m: String,
    precipitation_probability: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct WeatherData {
    location: LocationData,
    current_units: WeatherDataCurrentUnits,
    current: WeatherDataCurrent,
}

#[derive(Clone, Debug)]
enum WeatherState {
    Loading,
    Loaded(Result<WeatherData, String>),
}

fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str, context: &str) -> Result<T, String> {
    ureq::get(url)
        .call()
        .map_err(|err| format!("{context}: {err}"))?
        .into_json()
        .map_err(|err| format!("{context}: {err}"))
}

fn fetch_weather() -> Result<WeatherData, String> {
    let location: LocationData =
        fetch_json("http://ip-api.com/json", "failed to fetch location data")?;
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,relative_humidity_2m,precipitation_probability,weather_code",
        location.lat, location.lon
    );
    let mut data: WeatherData = fetch_json(&url, "failed to fetch weather data")?;
    data.location = location;
    Ok(data)
}

async fn fetch_weather_async() -> Result<WeatherData, String> {
    tokio::task::spawn_blocking(fetch_weather)
        .await
        .map_err(|err| format!("weather task failed: {err}"))?
}

fn loading_view(frame: ReadSignal<usize>) -> View {
    const FRAMES: [&str; 10] = ["|", "/", "-", "\\", "|", "/", "-", "\\", "|", "/"];
    let label = create_memo(move || FRAMES[frame.get() % FRAMES.len()].to_string());

    view! {
        div(
            justify_content = JustifyContent::CENTER,
            align_items = AlignItems::CENTER,
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
        ) {
            p(color = Color::Yellow) { (label) }
            p { " Loading..." }
        }
    }
}

fn weather_data_view(data: WeatherData) -> View {
    let title = format!(
        "Weather for {}, {}, {}",
        data.location.city, data.location.region, data.location.country
    );
    let coords = format!("{:.2}, {:.2}", data.location.lat, data.location.lon);
    let summary = format!(
        "{}  {}  {}",
        data.current.icon(),
        data.current.description(),
        data.current.icon()
    );
    let summary_color = data.current.color();
    let temperature = format!(
        "{:.1}{}",
        data.current.temperature_2m, data.current_units.temperature_2m
    );
    let humidity = format!(
        "{:.1}{}",
        data.current.relative_humidity_2m, data.current_units.relative_humidity_2m
    );
    let precipitation = format!(
        "{:.1}{}",
        data.current.precipitation_probability, data.current_units.precipitation_probability
    );

    view! {
        div(flex_direction = FlexDirection::Column, width = Size::Percent(100.0)) {
            div(
                flex_direction = FlexDirection::Column,
                border_style = BorderStyle::Single,
                border_color = Color::DarkGrey,
                border_edges = Edges::BOTTOM,
                align_items = AlignItems::CENTER,
                width = Size::Percent(100.0),
            ) {
                p { (title) }
                p(color = Color::DarkGrey) { (coords) }
            }
            div(flex_direction = FlexDirection::Column, align_items = AlignItems::CENTER) {
                div(padding = 1) {
                    p(color = summary_color) { (summary) }
                }
                div(flex_direction = FlexDirection::Row) {
                    p(weight = Weight::Bold) { "Temperature: " }
                    p { (temperature) }
                }
                div(flex_direction = FlexDirection::Row) {
                    p(weight = Weight::Bold) { "Humidity: " }
                    p { (humidity) }
                }
                div(flex_direction = FlexDirection::Row) {
                    p(weight = Weight::Bold) { "Chance of Precipitation: " }
                    p { (precipitation) }
                }
            }
        }
    }
}

fn error_view(err: String) -> View {
    view! {
        div(
            flex_direction = FlexDirection::Column,
            justify_content = JustifyContent::CENTER,
            align_items = AlignItems::CENTER,
            width = Size::Percent(100.0),
            height = Size::Percent(100.0),
            padding = 2,
        ) {
            p(weight = Weight::Bold, color = Color::Red) { "Error!" }
            p { (err) }
        }
    }
}

fn app() -> View {
    let state = create_signal(WeatherState::Loading);
    let frame = create_signal(0usize);
    let (reload_tx, mut reload_rx) = mpsc::unbounded_channel::<()>();

    use_future(async move {
        loop {
            sleep(Duration::from_millis(100)).await;
            frame.set((frame.get_untracked() + 1) % 10);
        }
    });

    use_future(async move {
        loop {
            state.set(WeatherState::Loading);
            let result = fetch_weather_async().await;
            state.set(WeatherState::Loaded(result));
            if reload_rx.recv().await.is_none() {
                break;
            }
        }
    });

    view! {
        div(
            width = 70,
            height = 14,
            margin = 1,
            border_style = BorderStyle::Round,
            border_color = Color::Cyan,
            flex_direction = FlexDirection::Column,
            tabindex = "0",
            on:raw_key=move |event: Event| {
                let Some(key) = event.key() else {
                    return;
                };
                if key.kind == KeyEventKind::Release {
                    return;
                }
                if matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R')) {
                    let _ = reload_tx.send(());
                }
            },
        ) {
            div(flex_grow = 1.0_f32, flex_direction = FlexDirection::Column, width = Size::Percent(100.0)) {
                (move || match state.get_clone() {
                    WeatherState::Loading => loading_view(*frame),
                    WeatherState::Loaded(Ok(data)) => weather_data_view(data),
                    WeatherState::Loaded(Err(err)) => error_view(err),
                })
            }
            div(
                width = Size::Percent(100.0),
                border_style = BorderStyle::Single,
                border_color = Color::DarkGrey,
                border_edges = Edges::TOP,
                padding_left = 1,
            ) {
                p { "[R] Reload | [Q] Quit" }
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    render_async(app).await
}
