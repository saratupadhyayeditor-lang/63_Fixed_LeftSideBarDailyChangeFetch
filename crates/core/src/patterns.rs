//! Candlestick pattern detection.
//!
//! The old app shipped 27 candlestick patterns usable both as chart overlays and
//! as realtime entry rules. `algo-core` had none; this module adds them as normal
//! registered indicators (category "Patterns") so the existing chart, catalog and
//! realtime strategy-condition code all pick them up without special cases.
//!
//! Every pattern emits one series whose value is `1.0` on a bullish signal bar,
//! `-1.0` on a bearish bar and `0.0` otherwise, plus matching up/down arrow
//! markers. A strategy condition therefore reads naturally as:
//!   * bullish  -> `value > 0`  (or `cross_up` a threshold of 0.5)
//!   * bearish  -> `value < 0`  (or `cross_down` a threshold of -0.5)

use crate::model::{
    Candle, IndicatorDef, IndicatorEntry, IndType, Marker, Point, SeriesOut, Settings,
};

// ---------------------------------------------------------------------------
// Shape helpers
// ---------------------------------------------------------------------------

fn body(c: &Candle) -> f64 {
    (c.close - c.open).abs()
}
fn range(c: &Candle) -> f64 {
    (c.high - c.low).abs().max(1e-9)
}
fn upper(c: &Candle) -> f64 {
    c.high - c.open.max(c.close)
}
fn lower(c: &Candle) -> f64 {
    c.open.min(c.close) - c.low
}
fn is_bull(c: &Candle) -> bool {
    c.close > c.open
}
fn is_bear(c: &Candle) -> bool {
    c.close < c.open
}
fn is_doji(c: &Candle) -> bool {
    body(c) <= 0.1 * range(c)
}
fn mid(c: &Candle) -> f64 {
    (c.open + c.close) / 2.0
}
fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn vec0(n: usize) -> Vec<i8> {
    vec![0i8; n]
}

// ---------------------------------------------------------------------------
// Detectors: one pass over the candles, returning a signal per bar
// ---------------------------------------------------------------------------

fn d_doji(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_doji(k) {
            s[i] = if is_bull(k) { 1 } else { -1 };
        }
    }
    s
}

fn d_dragonfly(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_doji(k) && lower(k) >= 0.6 * range(k) && upper(k) <= 0.15 * range(k) {
            s[i] = 1;
        }
    }
    s
}

fn d_gravestone(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_doji(k) && upper(k) >= 0.6 * range(k) && lower(k) <= 0.15 * range(k) {
            s[i] = -1;
        }
    }
    s
}

fn hammerish(k: &Candle) -> bool {
    lower(k) >= 2.0 * body(k) && upper(k) <= body(k).max(0.1 * range(k))
}
fn starish(k: &Candle) -> bool {
    upper(k) >= 2.0 * body(k) && lower(k) <= body(k).max(0.1 * range(k))
}

fn d_hammer(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 0..c.len() {
        if hammerish(&c[i]) {
            s[i] = 1;
        }
    }
    s
}

fn d_inverted_hammer(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 0..c.len() {
        if starish(&c[i]) {
            s[i] = 1;
        }
    }
    s
}

fn d_hanging_man(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        if hammerish(&c[i]) && is_bull(&c[i - 1]) {
            s[i] = -1;
        }
    }
    s
}

fn d_shooting_star(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        if starish(&c[i]) && is_bull(&c[i - 1]) {
            s[i] = -1;
        }
    }
    s
}

fn d_bull_engulf(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        let (p, k) = (&c[i - 1], &c[i]);
        if is_bear(p) && is_bull(k) && k.open <= p.close && k.close >= p.open {
            s[i] = 1;
        }
    }
    s
}

fn d_bear_engulf(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        let (p, k) = (&c[i - 1], &c[i]);
        if is_bull(p) && is_bear(k) && k.open >= p.close && k.close <= p.open {
            s[i] = -1;
        }
    }
    s
}

fn d_bull_harami(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        let (p, k) = (&c[i - 1], &c[i]);
        if is_bear(p) && is_bull(k) && k.open > p.close && k.close < p.open && body(k) < body(p) {
            s[i] = 1;
        }
    }
    s
}

fn d_bear_harami(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        let (p, k) = (&c[i - 1], &c[i]);
        if is_bull(p) && is_bear(k) && k.open < p.close && k.close > p.open && body(k) < body(p) {
            s[i] = -1;
        }
    }
    s
}

fn d_piercing(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        let (p, k) = (&c[i - 1], &c[i]);
        if is_bear(p) && is_bull(k) && k.open < p.close && k.close > mid(p) && k.close < p.open {
            s[i] = 1;
        }
    }
    s
}

fn d_dark_cloud(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        let (p, k) = (&c[i - 1], &c[i]);
        if is_bull(p) && is_bear(k) && k.open > p.close && k.close < mid(p) && k.close > p.open {
            s[i] = -1;
        }
    }
    s
}

fn d_morning_star(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 2..c.len() {
        let (a, b, k) = (&c[i - 2], &c[i - 1], &c[i]);
        if is_bear(a) && body(b) < body(a) && is_bull(k) && k.close > mid(a) {
            s[i] = 1;
        }
    }
    s
}

fn d_evening_star(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 2..c.len() {
        let (a, b, k) = (&c[i - 2], &c[i - 1], &c[i]);
        if is_bull(a) && body(b) < body(a) && is_bear(k) && k.close < mid(a) {
            s[i] = -1;
        }
    }
    s
}

fn d_morning_doji_star(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 2..c.len() {
        let (a, b, k) = (&c[i - 2], &c[i - 1], &c[i]);
        if is_bear(a) && is_doji(b) && is_bull(k) && k.close > mid(a) {
            s[i] = 1;
        }
    }
    s
}

fn d_three_white_soldiers(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 2..c.len() {
        let (a, b, k) = (&c[i - 2], &c[i - 1], &c[i]);
        if is_bull(a) && is_bull(b) && is_bull(k) && b.close > a.close && k.close > b.close {
            s[i] = 1;
        }
    }
    s
}

fn d_three_black_crows(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 2..c.len() {
        let (a, b, k) = (&c[i - 2], &c[i - 1], &c[i]);
        if is_bear(a) && is_bear(b) && is_bear(k) && b.close < a.close && k.close < b.close {
            s[i] = -1;
        }
    }
    s
}

fn d_bull_marubozu(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_bull(k) && body(k) >= 0.9 * range(k) {
            s[i] = 1;
        }
    }
    s
}

fn d_bear_marubozu(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_bear(k) && body(k) >= 0.9 * range(k) {
            s[i] = -1;
        }
    }
    s
}

fn d_spinning_top(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        let r = range(k);
        if body(k) <= 0.3 * r && upper(k) >= 0.2 * r && lower(k) >= 0.2 * r {
            s[i] = if is_bull(k) { 1 } else { -1 };
        }
    }
    s
}

fn d_inside_bar(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        if c[i].high < c[i - 1].high && c[i].low > c[i - 1].low {
            s[i] = if is_bull(&c[i]) { 1 } else { -1 };
        }
    }
    s
}

fn d_outside_bar(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        if c[i].high > c[i - 1].high && c[i].low < c[i - 1].low {
            s[i] = if is_bull(&c[i]) { 1 } else { -1 };
        }
    }
    s
}

fn d_tweezer_bottom(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        if is_bear(&c[i - 1]) && is_bull(&c[i]) && near(c[i].low, c[i - 1].low, 0.001 * c[i].low.max(1.0)) {
            s[i] = 1;
        }
    }
    s
}

fn d_tweezer_top(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for i in 1..c.len() {
        if is_bull(&c[i - 1]) && is_bear(&c[i]) && near(c[i].high, c[i - 1].high, 0.001 * c[i].high.max(1.0)) {
            s[i] = -1;
        }
    }
    s
}

fn d_bull_belt_hold(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_bull(k) && body(k) >= 0.7 * range(k) && near(k.open, k.low, 0.05 * range(k)) {
            s[i] = 1;
        }
    }
    s
}

fn d_bear_belt_hold(c: &[Candle]) -> Vec<i8> {
    let mut s = vec0(c.len());
    for (i, k) in c.iter().enumerate() {
        if is_bear(k) && body(k) >= 0.7 * range(k) && near(k.open, k.high, 0.05 * range(k)) {
            s[i] = -1;
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Series / marker emitters
// ---------------------------------------------------------------------------

fn series(c: &[Candle], det: fn(&[Candle]) -> Vec<i8>) -> Vec<SeriesOut> {
    let sig = det(c);
    let mut s = SeriesOut::line("#26a69a", 1.0);
    s.point_markers = true;
    s.title = Some("signal".into());
    s.data = c
        .iter()
        .enumerate()
        .map(|(i, k)| Point {
            time: k.time,
            value: sig.get(i).copied().unwrap_or(0) as f64,
            color: match sig.get(i).copied().unwrap_or(0) {
                1 => Some("#26a69a".into()),
                -1 => Some("#ef5350".into()),
                _ => None,
            },
        })
        .collect();
    vec![s]
}

fn mark(c: &[Candle], det: fn(&[Candle]) -> Vec<i8>) -> Vec<Marker> {
    let sig = det(c);
    c.iter()
        .enumerate()
        .filter_map(|(i, k)| match sig.get(i).copied().unwrap_or(0) {
            1 => Some(Marker {
                time: k.time,
                position: "belowBar".into(),
                color: "#26a69a".into(),
                shape: "arrowUp".into(),
                text: String::new(),
                size: 1.0,
            }),
            -1 => Some(Marker {
                time: k.time,
                position: "aboveBar".into(),
                color: "#ef5350".into(),
                shape: "arrowDown".into(),
                text: String::new(),
                size: 1.0,
            }),
            _ => None,
        })
        .collect()
}

fn pat_def(id: &str, name: &str, full: &str) -> IndicatorDef {
    IndicatorDef {
        id: id.into(),
        name: name.into(),
        full_name: full.into(),
        cat: "Patterns".into(),
        kind: IndType::Overlay,
        format: None,
        hidden: false,
        inputs: Vec::new(),
        style: Vec::new(),
    }
}

/// Append all pattern entries to the shared indicator registry.
pub fn registry_into(v: &mut Vec<IndicatorEntry>) {
    macro_rules! add {
        ($id:literal, $name:literal, $full:literal, $det:ident) => {{
            fn _series(c: &[Candle], _o: &Settings) -> Vec<SeriesOut> {
                series(c, $det)
            }
            fn _markers(c: &[Candle], _o: &Settings) -> Vec<Marker> {
                mark(c, $det)
            }
            v.push(IndicatorEntry {
                def: pat_def($id, $name, $full),
                compute: _series,
                markers: Some(_markers),
                reading: "number",
            });
        }};
    }

    add!("cdl_doji", "Doji", "Doji", d_doji);
    add!("cdl_dragonfly", "Dragonfly Doji", "Dragonfly Doji", d_dragonfly);
    add!("cdl_gravestone", "Gravestone Doji", "Gravestone Doji", d_gravestone);
    add!("cdl_hammer", "Hammer", "Hammer", d_hammer);
    add!("cdl_inverted_hammer", "Inverted Hammer", "Inverted Hammer", d_inverted_hammer);
    add!("cdl_hanging_man", "Hanging Man", "Hanging Man", d_hanging_man);
    add!("cdl_shooting_star", "Shooting Star", "Shooting Star", d_shooting_star);
    add!("cdl_bull_engulfing", "Bullish Engulfing", "Bullish Engulfing", d_bull_engulf);
    add!("cdl_bear_engulfing", "Bearish Engulfing", "Bearish Engulfing", d_bear_engulf);
    add!("cdl_bull_harami", "Bullish Harami", "Bullish Harami", d_bull_harami);
    add!("cdl_bear_harami", "Bearish Harami", "Bearish Harami", d_bear_harami);
    add!("cdl_piercing", "Piercing Line", "Piercing Line", d_piercing);
    add!("cdl_dark_cloud", "Dark Cloud Cover", "Dark Cloud Cover", d_dark_cloud);
    add!("cdl_morning_star", "Morning Star", "Morning Star", d_morning_star);
    add!("cdl_evening_star", "Evening Star", "Evening Star", d_evening_star);
    add!("cdl_morning_doji_star", "Morning Doji Star", "Morning Doji Star", d_morning_doji_star);
    add!("cdl_three_white_soldiers", "Three White Soldiers", "Three White Soldiers", d_three_white_soldiers);
    add!("cdl_three_black_crows", "Three Black Crows", "Three Black Crows", d_three_black_crows);
    add!("cdl_bull_marubozu", "Bullish Marubozu", "Bullish Marubozu", d_bull_marubozu);
    add!("cdl_bear_marubozu", "Bearish Marubozu", "Bearish Marubozu", d_bear_marubozu);
    add!("cdl_spinning_top", "Spinning Top", "Spinning Top", d_spinning_top);
    add!("cdl_inside_bar", "Inside Bar", "Inside Bar", d_inside_bar);
    add!("cdl_outside_bar", "Outside Bar", "Outside Bar", d_outside_bar);
    add!("cdl_tweezer_bottom", "Tweezer Bottom", "Tweezer Bottom", d_tweezer_bottom);
    add!("cdl_tweezer_top", "Tweezer Top", "Tweezer Top", d_tweezer_top);
    add!("cdl_bull_belt_hold", "Bullish Belt Hold", "Bullish Belt Hold", d_bull_belt_hold);
    add!("cdl_bear_belt_hold", "Bearish Belt Hold", "Bearish Belt Hold", d_bear_belt_hold);
}
