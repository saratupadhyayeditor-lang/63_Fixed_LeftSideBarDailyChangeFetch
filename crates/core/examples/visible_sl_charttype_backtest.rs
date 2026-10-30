//! Backtest: the Visible Straight Line Filter (noise-floor trend read) across
//! the four engine chart types - Candles / Line / Heikin Ashi / Renko - on
//! NIFTY, SENSEX and a few stocks' ATM-strike option premium series.
//!
//! It reuses the EXACT production transform (`algo_core::chart_type::build`) and
//! the EXACT production noise-floor read (`straight_line_trend`, window 5, half
//! the recent mean absolute step), so the gate it simulates is the one the
//! server's `series_dir_noisy` applies for the Visible Straight Line Filter.
//!
//! Data: reads `<dir>/<SYM>_1m.csv` (time,open,high,low,close,volume) when present;
//! otherwise synthesises a deterministic 1-minute series for that symbol. The
//! ATM option premium is modelled as a leveraged rider on the underlying (ATM
//! delta 0.5, elasticity = delta * spot / premium_base), so the filter trades the
//! premium chart exactly like the engine's "Trade In = premium" setting.
//!
//! Run: cargo run -p algo-core --example visible_sl_charttype_backtest --release [dir]

use algo_core::chart_type::{self, ChartKind, RenkoConfig, RenkoMode, RenkoSource};
use algo_core::model::Settings;
use algo_core::Candle;

// ---------------------------------------------------------------------------
// Production noise-floor read (port of the server's `straight_line_trend`).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dir {
    Bull,
    Bear,
    Flat,
}

/// `window`-bar noise floor: the newest step must beat half the recent mean
/// absolute step, else Flat. Mirrors the server read with window = 5.
fn line_trend(vals: &[f64], offset: usize, window: usize) -> Option<Dir> {
    let end = vals.len().checked_sub(offset)?;
    if end < 3 {
        return None;
    }
    let newest = vals[end - 1];
    let previous = vals[end - 2];
    if !newest.is_finite() || !previous.is_finite() {
        return None;
    }
    let step = newest - previous;
    let need = window.max(3);
    let start = end.saturating_sub(need + 1);
    let w: Vec<f64> = (start..end).map(|i| vals[i]).collect();
    if !w.iter().all(|v| v.is_finite()) {
        return None;
    }
    let sum: f64 = w.windows(2).map(|a| (a[1] - a[0]).abs()).sum();
    let mean = sum / (w.len() - 1) as f64;
    let eps = mean * 0.5;
    if step > eps {
        Some(Dir::Bull)
    } else if step < -eps {
        Some(Dir::Bear)
    } else {
        Some(Dir::Flat)
    }
}

// ---------------------------------------------------------------------------
// Data loading / synthesis
// ---------------------------------------------------------------------------

fn load_csv(path: &str) -> Vec<Candle> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 5 {
            continue;
        }
        out.push(Candle {
            time: f[0].trim().parse().unwrap_or(0),
            open: f[1].trim().parse().unwrap_or(0.0),
            high: f[2].trim().parse().unwrap_or(0.0),
            low: f[3].trim().parse().unwrap_or(0.0),
            close: f[4].trim().parse().unwrap_or(0.0),
            volume: f.get(5).and_then(|s| s.trim().parse().ok()).unwrap_or(0.0),
        });
    }
    out
}

struct Xorshift(u64);
impl Xorshift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Standard normal via Box-Muller.
    fn normal(&mut self) -> f64 {
        let u1 = ((self.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
        let u2 = ((self.next_u64() >> 11) as f64) / (1u64 << 53) as f64;
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

/// Deterministic 1-minute ATM option premium series for one symbol.
/// `spot` = underlying start, `minute_sigma` = per-minute underlying vol,
/// `prem_base` = ATM premium at start, `bars` = number of 1-min bars.
fn synth_premium(symbol: &str, spot0: f64, minute_sigma: f64, prem_base: f64, bars: usize) -> Vec<Candle> {
    let seed = symbol.bytes().fold(0x9E3779B97F4A7C15u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001B3));
    let mut rng = Xorshift(seed | 1);
    // ATM elasticity: d(prem)/prem = k * d(spot)/spot, k = delta * spot / prem.
    let k = 0.5 * spot0 / prem_base;
    let mut prem = prem_base;
    let mut t0: i64 = 0; // synthetic minute index (ms)
    let mut out = Vec::with_capacity(bars);
    let mut prev_close = prem_base;
    for _ in 0..bars {
        let r = rng.normal() * minute_sigma;
        // premium % move from underlying % move via elasticity
        let dp = (k * r).exp();
        prem *= dp;
        let open = prev_close;
        let close = prem;
        let wick = (rng.normal().abs() * 0.4 + 0.2) * minute_sigma * k * close;
        let high = open.max(close) + wick;
        let low = (open.min(close) - wick).max(0.05);
        out.push(Candle { time: t0, open, high, low, close, volume: 1000.0 });
        prev_close = close;
        t0 += 60_000;
    }
    out
}

// ---------------------------------------------------------------------------
// Backtest
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Res {
    bars: usize,
    trades: usize,
    wins: usize,
    net: f64,
    gross_win: f64,
    gross_loss: f64,
    max_dd: f64,
    sl_hits: usize,
    ret_pct: f64,
    entry_px: f64,
}

fn backtest(chart: &[Candle], line: &[f64], sl_pct: f64) -> Res {
    let mut r = Res { bars: chart.len(), ..Default::default() };
    let n = chart.len().min(line.len());
    if n < 8 {
        return r;
    }
    r.entry_px = chart[0].close.abs().max(1e-9);
    let mut pos = 0i32;
    let mut entry = 0.0f64;
    let mut peak = 0.0f64;
    let mut equity = 0.0f64;

    for i in 3..n {
        let d = match line_trend(&line[..=i], 0, 5) {
            Some(Dir::Bull) => 1,
            Some(Dir::Bear) => -1,
            _ => 0,
        };
        let px = chart[i].close;
        if d == 0 {
            continue;
        }
        if pos == 0 {
            pos = d;
            entry = px;
        } else if d != pos {
            let pnl = if pos > 0 { px - entry } else { entry - px };
            r.trades += 1;
            r.net += pnl;
            equity += pnl;
            peak = peak.max(equity);
            r.max_dd = r.max_dd.max(peak - equity);
            if pnl > 0.0 {
                r.wins += 1;
                r.gross_win += pnl;
            } else {
                r.gross_loss += -pnl;
            }
            pos = d;
            entry = px;
        } else if sl_pct > 0.0 {
            // Fixed % stop from entry, checked on this bar's extreme.
            let stop = entry * (1.0 - sl_pct / 100.0);
            let hit = if pos > 0 { chart[i].low <= stop } else { chart[i].high >= entry * (1.0 + sl_pct / 100.0) };
            if hit {
                let exit = if pos > 0 { stop } else { entry * (1.0 + sl_pct / 100.0) };
                let pnl = if pos > 0 { exit - entry } else { entry - exit };
                r.trades += 1;
                r.sl_hits += 1;
                r.net += pnl;
                equity += pnl;
                peak = peak.max(equity);
                r.max_dd = r.max_dd.max(peak - equity);
                if pnl > 0.0 {
                    r.wins += 1;
                    r.gross_win += pnl;
                } else {
                    r.gross_loss += -pnl;
                }
                pos = 0;
            }
        }
    }
    r.ret_pct = r.net / r.entry_px * 100.0;
    r
}

fn row(label: &str, r: &Res, cost_pct_side: f64) {
    let win = if r.trades > 0 { r.wins as f64 / r.trades as f64 * 100.0 } else { 0.0 };
    let pf = if r.gross_loss > 1e-9 { r.gross_win / r.gross_loss } else { f64::INFINITY };
    let cost = r.trades as f64 * 2.0 * cost_pct_side / 100.0 * r.entry_px;
    let net_c = r.net - cost;
    println!(
        "{:<28} {:>6} {:>7} {:>6} {:>9} {:>8} {:>9} {:>8} {:>6}",
        label,
        r.bars,
        r.trades,
        format!("{:.0}%", win),
        format!("{:+.1}", r.net),
        format!("{:+.1}%", r.ret_pct),
        format!("{:+.1}", net_c),
        format!("{:.2}", if pf.is_finite() { pf } else { 99.99 }),
        format!("{:.0}%", if r.trades > 0 { r.sl_hits as f64 / r.trades as f64 * 100.0 } else { 0.0 }),
    );
}

fn renko_cfg() -> RenkoConfig {
    // ATR brick, non-repaint (only closed bars build bricks) - the engine default shape.
    RenkoConfig { mode: RenkoMode::Atr, atr_length: 14, percentage: 0.1, non_repaint: true, source: RenkoSource::Close, ..Default::default() }
}

fn sl_settings(id: &str) -> Settings {
    let pairs: &[(&str, f64)] = match id {
        "slema" => &[("length", 9.0), ("swing", 5.0), ("minLeg", 5.0)],
        "stzigzag" => &[("atrPeriod", 10.0), ("factorFast", 1.0), ("factorSlow", 0.1), ("swing", 5.0), ("minLeg", 5.0)],
        _ => &[("structLen", 5.0), ("symLegs", 3.0), ("eqTol", 25.0), ("projCount", 1.0)],
    };
    pairs.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect()
}

/// Line values for id over a chart series (output index 0), padded/truncated to
/// the series length.
fn line_values(id: &str, chart: &[Candle]) -> Vec<f64> {
    let out = algo_core::compute(id, chart, &sl_settings(id));
    let s = match out.first() {
        Some(s) => s,
        None => return vec![f64::NAN; chart.len()],
    };
    let mut v: Vec<f64> = s.data.iter().map(|p| p.value).collect();
    while v.len() < chart.len() {
        v.push(f64::NAN);
    }
    v.truncate(chart.len());
    v
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp/opencode/bt_data".into());

    // Every `*_1m.csv` in the directory is a real series (symbol = file stem).
    let mut files: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "csv").unwrap_or(false) {
                if let Some(s) = p.to_str() {
                    files.push(s.to_string());
                }
            }
        }
    }
    files.sort();

    let kinds = [
        ("Candles", ChartKind::Candles),
        ("Line", ChartKind::Line),
        ("HeikinAshi", ChartKind::HeikinAshi),
        ("Renko", ChartKind::Renko),
    ];
    let inds = ["slema", "stzigzag", "supplydemand"];
    let cost_pct_side = 0.05; // %/side notional (brokerage + slippage + spread)

    if !files.is_empty() {
        for path in &files {
            let prem = load_csv(path);
            if prem.len() < 30 {
                continue;
            }
            let name = std::path::Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(path)
                .to_string();
            println!("===============================================================================");
            println!(
                "### {name}  [real csv]  {} bars  first {:.2}  last {:.2}   (ATM premium series)",
                prem.len(),
                prem.first().unwrap().close,
                prem.last().unwrap().close
            );
            println!("===============================================================================");
            println!(
                "{:<28} {:>6} {:>7} {:>6} {:>9} {:>8} {:>9} {:>8} {:>6}",
                "chart / indicator", "bars", "trades", "win%", "net", "ret%", "netCost", "PF", "SLhit%"
            );
            for (kname, kind) in kinds {
                let chart = match kind {
                    ChartKind::Candles => prem.clone(),
                    k => chart_type::build(k, &prem, &renko_cfg()),
                };
                for id in inds {
                    let line = line_values(id, &chart);
                    let r = backtest(&chart, &line, 0.0);
                    row(&format!("{kname} / {id}"), &r, cost_pct_side);
                }
            }
            println!();
        }
        return;
    }

    // No CSVs: deterministic synthetic fallback (structure demo only).
    let instruments: [(&str, f64, f64, f64); 5] = [
        ("NIFTY", 24000.0, 0.00055, 150.0),
        ("SENSEX", 80000.0, 0.00055, 300.0),
        ("RELIANCE", 2900.0, 0.00090, 45.0),
        ("HDFCBANK", 1700.0, 0.00085, 28.0),
        ("TCS", 4100.0, 0.00090, 70.0),
    ];
    let bars = 375 * 5;
    for (sym, spot0, vol, prem_base) in instruments {
        let prem = synth_premium(sym, spot0, vol, prem_base, bars);
        println!("===============================================================================");
        println!("### {sym}  [synthetic]  {} bars", prem.len());
        println!("===============================================================================");
        println!(
            "{:<28} {:>6} {:>7} {:>6} {:>9} {:>8} {:>9} {:>8} {:>6}",
            "chart / indicator", "bars", "trades", "win%", "net", "ret%", "netCost", "PF", "SLhit%"
        );
        for (kname, kind) in kinds {
            let chart = match kind {
                ChartKind::Candles => prem.clone(),
                k => chart_type::build(k, &prem, &renko_cfg()),
            };
            for id in inds {
                let line = line_values(id, &chart);
                let r = backtest(&chart, &line, 0.0);
                row(&format!("{kname} / {id}"), &r, cost_pct_side);
            }
        }
        println!();
    }
}
