use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A single OHLCV candle. `time` is epoch seconds whose value is the IST
/// wall-clock instant treated as naive UTC (matches the old web app's
/// lightweight-charts convention so axis labels read correctly).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Indicator settings map coming from the UI (indicator inputs + style keys).
pub type Settings = BTreeMap<String, Value>;

pub fn num(s: &Settings, key: &str, def: f64) -> f64 {
    s.get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(def)
}

pub fn int(s: &Settings, key: &str, def: i64) -> i64 {
    s.get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(def)
}

pub fn strv(s: &Settings, key: &str, def: &str) -> String {
    s.get(key)
        .and_then(|v| v.as_str())
        .map(|x| x.to_string())
        .unwrap_or_else(|| def.to_string())
}

pub fn boolv(s: &Settings, key: &str, def: bool) -> bool {
    s.get(key)
        .and_then(|v| v.as_bool())
        .or_else(|| s.get(key).and_then(|v| v.as_i64()).map(|i| i != 0))
        .unwrap_or(def)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SeriesKind {
    Line,
    Histogram,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Point {
    pub time: i64,
    pub value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceLine {
    pub price: f64,
    pub color: String,
    pub line_width: f64,
    pub line_style: i32,
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ScaleOpts {
    pub top: f64,
    pub bottom: f64,
}

/// One drawable series produced by an indicator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeriesOut {
    pub kind: SeriesKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_style: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_scale_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_scale: Option<ScaleOpts>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub point_markers: bool,
    #[serde(default = "default_true")]
    pub last_value_visible: bool,
    #[serde(default = "default_true")]
    pub price_line_visible: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_autoscale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub data: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub price_lines: Vec<PriceLine>,
}

fn default_true() -> bool {
    true
}

impl Default for SeriesOut {
    fn default() -> Self {
        Self {
            kind: SeriesKind::Line,
            color: None,
            line_width: None,
            line_style: None,
            price_scale_id: None,
            price_scale: None,
            point_markers: false,
            last_value_visible: true,
            price_line_visible: true,
            exclude_autoscale: false,
            title: None,
            data: Vec::new(),
            price_lines: Vec::new(),
        }
    }
}

impl SeriesOut {
    pub fn line(color: &str, line_width: f64) -> Self {
        SeriesOut {
            kind: SeriesKind::Line,
            color: Some(color.to_string()),
            line_width: Some(line_width),
            ..Default::default()
        }
    }

    pub fn hist(color: &str) -> Self {
        SeriesOut {
            kind: SeriesKind::Histogram,
            color: Some(color.to_string()),
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IndType {
    Overlay,
    Pane,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputOption {
    pub value: String,
    pub label: String,
}

/// Numeric or enum input for an indicator (rendered by the UI menu).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputDef {
    pub key: String,
    pub label: String,
    /// "number" | "source" | "bool" | "enum"
    pub kind: String,
    pub def: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub options: Vec<InputOption>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StyleDef {
    pub key: String,
    pub label: String,
    /// "color" | "number"
    pub kind: String,
    pub def: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
}

/// The catalog definition (everything the UI needs before computing).
#[derive(Clone, Serialize, Deserialize)]
pub struct IndicatorDef {
    pub id: String,
    pub name: String,
    pub full_name: String,
    pub cat: String,
    #[serde(rename = "type")]
    pub kind: IndType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub inputs: Vec<InputDef>,
    #[serde(default)]
    pub style: Vec<StyleDef>,
}

/// The registered compute entry for an indicator.
pub struct IndicatorEntry {
    pub def: IndicatorDef,
    pub compute: ComputeFn,
    pub markers: Option<MarkersFn>,
    /// Reading format used by the value/comparison picker: "price","number","percent","decimal".
    pub reading: &'static str,
}

pub type ComputeFn = fn(&[Candle], &Settings) -> Vec<SeriesOut>;
pub type MarkersFn = fn(&[Candle], &Settings) -> Vec<Marker>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub time: i64,
    pub position: String,
    pub color: String,
    pub shape: String,
    #[serde(default)]
    pub text: String,
    #[serde(default = "one")]
    pub size: f64,
}

fn one() -> f64 {
    1.0
}

/// Timeframe definition matching data_fetcher.TIMEFRAME_CONFIG.
#[derive(Clone, Copy, Debug)]
pub struct Timeframe {
    pub key: &'static str,
    pub label: &'static str,
    pub api_interval: i64,
    pub resample: Option<&'static str>,
}

pub const TIMEFRAMES: &[Timeframe] = &[
    Timeframe { key: "1min", label: "1 Minute", api_interval: 1, resample: None },
    Timeframe { key: "2min", label: "2 Minutes", api_interval: 1, resample: Some("2min") },
    Timeframe { key: "3min", label: "3 Minutes", api_interval: 1, resample: Some("3min") },
    Timeframe { key: "4min", label: "4 Minutes", api_interval: 1, resample: Some("4min") },
    Timeframe { key: "5min", label: "5 Minutes", api_interval: 5, resample: None },
    Timeframe { key: "10min", label: "10 Minutes", api_interval: 5, resample: Some("10min") },
    Timeframe { key: "15min", label: "15 Minutes", api_interval: 15, resample: None },
    Timeframe { key: "30min", label: "30 Minutes", api_interval: 5, resample: Some("30min") },
    Timeframe { key: "1hour", label: "1 Hour", api_interval: 60, resample: None },
    Timeframe { key: "4hour", label: "4 Hours", api_interval: 60, resample: Some("4h") },
    Timeframe { key: "day", label: "Day", api_interval: 0, resample: None },
    Timeframe { key: "week", label: "Week", api_interval: 0, resample: Some("W") },
    Timeframe { key: "month", label: "Month", api_interval: 0, resample: Some("ME") },
    Timeframe { key: "year", label: "Year", api_interval: 0, resample: Some("YE") },
];

pub fn timeframe(key: &str) -> &'static Timeframe {
    TIMEFRAMES
        .iter()
        .find(|t| t.key == key)
        .unwrap_or(&TIMEFRAMES[4])
}
