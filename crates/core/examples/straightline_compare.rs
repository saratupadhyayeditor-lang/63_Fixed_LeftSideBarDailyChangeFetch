//! Offline comparison: how well a Straight Line / Trendline indicator filter
//! behaves when it is evaluated on the raw candle series versus the display
//! chart series (Heikin-Ashi / Renko / Line).
//!
//! It reuses the EXACT production code path: [`algo_core::chart_type::build`]
//! turns raw candles into the display series, [`algo_core::compute`] runs the
//! real straight-line indicator on it, and the per-bar direction is read from
//! the point colour that `color_straight_line` stamps (up = bullish, down =
//! bearish) - i.e. the same value the engine's colour-detection filter reads.
//!
//! Metrics reported for each chart mode:
//!   win%      - of bars where the filter has a clear side, the % that match the
//!               real forward move over the next H bars.
//!   flips     - number of direction changes (churn: more flips = more signals).
//!   false%    - of those flips, the % that reverse (whipsaw) within W bars.
//!   lag       - avg bars between a real price turning point and the bar the
//!               filter actually flips to the new side (entry latency).
//!
//! Run: cargo run -p algo-core --example straightline_compare --release

use algo_core::chart_type::{self, ChartKind, RenkoConfig, RenkoMode, RenkoSource};
use algo_core::{Candle, Settings};
use serde_json::json;

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    fn next_f(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn norm(&mut self) -> f64 {
        let u1 = self.next_f().max(1e-12);
        let u2 = self.next_f();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

/// Synthetic 1-min candles with alternating trend-up / trend-down / choppy
/// regimes plus gaussian noise, so the filter is judged on all market states.
fn gen_scenario(seed: u64, seg_len: usize, n_seg: usize, trend_only: bool) -> Vec<Candle> {
    let mut rng = Lcg::new(seed);
    let mut price = 20000.0f64;
    let mut out = Vec::with_capacity(seg_len * n_seg);
    let mut t = 1_700_000_000i64;
    for s in 0..n_seg {
        let regime = if trend_only { s % 2 } else { s % 3 };
        let (drift, vol) = match regime {
            0 => (if trend_only { 0.9 } else { 0.55 }, if trend_only { 1.8 } else { 2.0 }), // up trend
            1 => (if trend_only { -0.9 } else { -0.55 }, if trend_only { 1.8 } else { 2.0 }), // down trend
            _ => (0.0, 3.0),                                                                // choppy
        };
        for _ in 0..seg_len {
            t += 60;
            let o = price;
            let c = o + drift + vol * rng.norm();
            let h = o.max(c) + rng.next_f() * vol * 0.6;
            let l = o.min(c) - rng.next_f() * vol * 0.6;
            out.push(Candle { time: t, open: o, high: h, low: l, close: c, volume: 1000.0 });
            price = c;
        }
    }
    out
}

fn settings(id: &str) -> Settings {
    let mut s = Settings::new();
    s.insert("upColor".into(), json!("up"));
    s.insert("downColor".into(), json!("dn"));
    match id {
        "slconsensus" => {
            s.insert("minAgree".into(), json!(2));
            s.insert("confirm".into(), json!(5));
            s.insert("strength".into(), json!(5));
        }
        "slema" => {
            s.insert("length".into(), json!(9));
            s.insert("source".into(), json!("close"));
        }
        _ => {
            s.insert("trendLen".into(), json!(9));
        }
    }
    s
}

/// Dense per-bar direction (-1/0/+1) read EXACTLY like the engine does: run the
/// production colour rule (`color_straight_line`) over a point at every bar of
/// the series and read the point colour (up = bullish, down = bearish).
fn dir_dense(id: &str, series: &[Candle]) -> Vec<i32> {
    let mut dummy = algo_core::SeriesOut::default();
    dummy.data = series
        .iter()
        .map(|c| algo_core::Point { time: c.time, value: c.close, color: None })
        .collect();
    algo_core::indicators::color_straight_line(id, std::slice::from_mut(&mut dummy), &settings(id), series);
    dummy
        .data
        .iter()
        .map(|p| match p.color.as_deref() {
            Some("up") => 1,
            Some("dn") => -1,
            _ => 0,
        })
        .collect()
}

/// Map the series-bar directions onto raw-bar timestamps (for Renko, which has a
/// different bar count, carry the latest series direction forward).
fn dir_at(id: &str, series: &[Candle], times: &[i64]) -> Vec<i32> {
    let sdir = dir_dense(id, series);
    let mut out = vec![0i32; times.len()];
    let mut j = 0usize;
    let mut cur = 0i32;
    for (i, t) in times.iter().enumerate() {
        while j < series.len() && series[j].time <= *t {
            cur = sdir[j];
            j += 1;
        }
        out[i] = cur;
    }
    out
}

struct Stats {
    win: f64,
    flips: usize,
    false_pct: f64,
    lag: f64,
}

fn evaluate(raw: &[Candle], dir: &[i32]) -> Stats {
    let n = raw.len();
    let w = 10usize; // whipsaw window (bars)

    // Ping-pong strategy: enter on every flip, exit on the next flip. Win% is
    // the share of completed trades that closed in profit (the real strategy
    // win rate). Also counts flips (signal churn) and whipsaw "false" flips.
    let mut false_flips = 0usize;
    let mut trades = 0usize;
    let mut wins = 0usize;
    let mut entry: Option<(i32, f64)> = None;
    let mut prev = 0i32;
    for i in 0..n {
        let d = dir[i];
        if d != 0 && d != prev {
            if prev != 0 {
                // close the opposite trade
                if let Some((side, px)) = entry {
                    let out = raw[i].close;
                    let profit = if side > 0 { out > px } else { out < px };
                    trades += 1;
                    if profit {
                        wins += 1;
                    }
                }
            }
            entry = Some((d, raw[i].close));
            prev = d;
        }
    }
    // Close any open trade at the last bar.
    if let Some((side, px)) = entry {
        let out = raw[n - 1].close;
        let profit = if side > 0 { out > px } else { out < px };
        trades += 1;
        if profit {
            wins += 1;
        }
    }
    // Whipsaw: a flip whose direction reverses back within W bars.
    let mut prev2 = 0i32;
    for i in 0..n {
        if dir[i] != 0 && dir[i] != prev2 {
            if prev2 != 0 {
                if i + w < n && dir[i + w] == -dir[i] {
                    false_flips += 1;
                }
            }
            prev2 = dir[i];
        }
    }
    let false_pct = if trades > 0 { false_flips as f64 / trades as f64 * 100.0 } else { 0.0 };
    let win = if trades > 0 { wins as f64 / trades as f64 * 100.0 } else { 0.0 };
    let flips = trades;

    // Entry lag from real turning points.
    let piv = 8usize;
    let mut lags: Vec<f64> = Vec::new();
    for k in piv..n.saturating_sub(piv) {
        let c = raw[k].close;
        let is_hi = (k - piv..=k + piv).all(|j| raw[j].close <= c + 1e-9);
        let is_lo = (k - piv..=k + piv).all(|j| raw[j].close >= c - 1e-9);
        if !is_hi && !is_lo {
            continue;
        }
        let new_dir = if is_hi { -1 } else { 1 };
        let mut found = None;
        for j in k + 1..n.min(k + 60) {
            if dir[j] == new_dir {
                found = Some((j - k) as f64);
                break;
            }
        }
        if let Some(l) = found {
            lags.push(l);
        } else {
            lags.push(60.0);
        }
    }
    let lag = if lags.is_empty() {
        0.0
    } else {
        lags.iter().sum::<f64>() / lags.len() as f64
    };

    Stats { win, flips, false_pct, lag }
}

fn main() {
    let ids = ["slconsensus", "slema", "autotrend", "trendmaster"];
    let kinds = [
        ("Raw candles", ChartKind::Candles),
        ("Line", ChartKind::Line),
        ("Heikin-Ashi", ChartKind::HeikinAshi),
        ("Renko(ATR)", ChartKind::Renko),
    ];
    let cfg = RenkoConfig {
        mode: RenkoMode::Atr,
        atr_length: 14,
        source: RenkoSource::Close,
        ..Default::default()
    };

    for (suite, trend_only) in [("MIXED regimes (up / down / choppy)", false), ("CLEAN trends (up / down only)", true)] {
        println!("################################################################");
        println!("# SUITE: {suite}   (24 scenarios x 6000 bars, NIFTY-like price)");
        println!("################################################################\n");
        for id in ids {
            println!("=== filter: {id} ===");
            println!("{:<14} {:>8} {:>8} {:>9} {:>8}", "chart", "win%", "signals", "false%", "lag");
            for (name, kind) in kinds {
                let mut agg = Stats { win: 0.0, flips: 0, false_pct: 0.0, lag: 0.0 };
                let scenarios = 24u64;
                for seed in 0..scenarios {
                    let raw = gen_scenario(seed, 200, 30, trend_only);
                    let series = chart_type::build(kind, &raw, &cfg);
                    let times: Vec<i64> = raw.iter().map(|c| c.time).collect();
                    let dir = dir_at(id, &series, &times);
                    let st = evaluate(&raw, &dir);
                    agg.win += st.win;
                    agg.flips += st.flips;
                    agg.false_pct += st.false_pct;
                    agg.lag += st.lag;
                }
                let n = scenarios as f64;
                println!(
                    "{:<14} {:>8.1} {:>8} {:>9.1} {:>8.1}",
                    name,
                    agg.win / n,
                    agg.flips / scenarios as usize,
                    agg.false_pct / n,
                    agg.lag / n
                );
            }
            println!();
        }
    }
}
