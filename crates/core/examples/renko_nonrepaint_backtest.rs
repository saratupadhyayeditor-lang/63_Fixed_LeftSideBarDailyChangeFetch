//! Real-data backtest: does the NON-REPAINT Renko mode change trade outcomes
//! versus the legacy (repainting) Renko mode?
//!
//! It uses the EXACT production transform [`algo_core::chart_type::build`] to
//! turn real 1-minute OHLC candles into Renko bricks, once with
//! `non_repaint = false` (legacy: the trailing forming candle also builds bricks)
//! and once with `non_repaint = true` (bricks only from closed candles).
//!
//! Strategy (the way the engine trades a Renko series): a brick's colour is its
//! direction; on every COLOUR FLIP enter in the new direction at that brick's
//! close, exit at the next flip's close. Optional fixed stop-loss of N brick
//! boxes is simulated on the raw 1-minute candles between the entry and exit
//! bricks (adverse excursion -> stop fill).
//!
//! Metrics per symbol/mode: bricks, trades, win%, net points, %return on the
//! notional of the first entry, profit factor and max drawdown (points).
//!
//! Run: cargo run -p algo-core --example renko_nonrepaint_backtest --release <dir>
//!   <dir> holds SYMBOL_1m.csv files (time,open,high,low,close,volume).

use algo_core::chart_type::{self, ChartKind, RenkoConfig, RenkoMode, RenkoSource};
use algo_core::Candle;
use std::collections::BTreeMap;

fn load_csv(path: &str) -> Vec<Candle> {
    let text = std::fs::read_to_string(path).expect("read csv");
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 6 {
            continue;
        }
        out.push(Candle {
            time: f[0].trim().parse().unwrap_or(0),
            open: f[1].trim().parse().unwrap_or(0.0),
            high: f[2].trim().parse().unwrap_or(0.0),
            low: f[3].trim().parse().unwrap_or(0.0),
            close: f[4].trim().parse().unwrap_or(0.0),
            volume: f[5].trim().parse().unwrap_or(0.0),
        });
    }
    out
}

fn raw_index_at(raw: &[Candle], t: i64) -> usize {
    match raw.binary_search_by_key(&t, |c| c.time) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    }
}

#[derive(Default)]
struct Res {
    bricks: usize,
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

/// Run the brick-flip ping-pong strategy over one series. `sl_boxes` = 0 means
/// no stop; >0 uses that many brick boxes as a fixed stop measured on the raw
/// candles between entry and exit.
fn backtest(raw: &[Candle], series: &[Candle], sl_boxes: f64) -> Res {
    let mut r = Res { bricks: series.len(), ..Default::default() };
    if series.len() < 3 {
        return r;
    }
    let dir = |b: &Candle| if b.close >= b.open { 1i32 } else { -1i32 };
    let first_entry_px = series[1].close.abs().max(1e-9);
    r.entry_px = first_entry_px;

    let mut prev = dir(&series[0]);
    let mut pos: Option<(i32, f64, f64, usize)> = None; // side, entry_px, box, entry_raw_idx
    let mut equity = 0.0f64;
    let mut peak = 0.0f64;

    let close_trade = |r: &mut Res, side: i32, entry_px: f64, exit_px: f64| {
        let pnl = if side > 0 { exit_px - entry_px } else { entry_px - exit_px };
        r.trades += 1;
        r.net += pnl;
        if pnl > 0.0 {
            r.wins += 1;
            r.gross_win += pnl;
        } else {
            r.gross_loss += -pnl;
        }
    };

    for i in 1..series.len() {
        let d = dir(&series[i]);
        if d != prev {
            // Exit the current position at this flip brick's close.
            if let Some((side, entry_px, boxsz, eidx)) = pos.take() {
                let exit_px = series[i].close;
                let mut px = exit_px;
                if sl_boxes > 0.0 {
                    let stop = boxsz * sl_boxes;
                    let xidx = raw_index_at(raw, series[i].time);
                    for row in raw.iter().take(xidx + 1).skip(eidx + 1) {
                        let hit = if side > 0 { row.low <= entry_px - stop } else { row.high >= entry_px + stop };
                        if hit {
                            px = if side > 0 { entry_px - stop } else { entry_px + stop };
                            r.sl_hits += 1;
                            break;
                        }
                    }
                }
                let pnl = if side > 0 { px - entry_px } else { entry_px - px };
                close_trade(&mut r, side, entry_px, px);
                equity += pnl;
                peak = peak.max(equity);
                r.max_dd = r.max_dd.max(peak - equity);
            }
            // Open a new position in the new direction at this flip's close.
            let boxsz = (series[i].close - series[i].open).abs().max(1e-9);
            pos = Some((d, series[i].close, boxsz, raw_index_at(raw, series[i].time)));
        }
        prev = d;
    }
    // Close any open position at the last brick.
    if let Some((side, entry_px, boxsz, eidx)) = pos.take() {
        let exit_px = series.last().unwrap().close;
        let mut px = exit_px;
        if sl_boxes > 0.0 {
            let stop = boxsz * sl_boxes;
            let xidx = raw.len() - 1;
            for row in raw.iter().take(xidx + 1).skip(eidx + 1) {
                let hit = if side > 0 { row.low <= entry_px - stop } else { row.high >= entry_px + stop };
                if hit {
                    px = if side > 0 { entry_px - stop } else { entry_px + stop };
                    r.sl_hits += 1;
                    break;
                }
            }
        }
        let pnl = if side > 0 { px - entry_px } else { entry_px - px };
        close_trade(&mut r, side, entry_px, px);
    }
    r.ret_pct = r.net / first_entry_px * 100.0;
    r
}

fn row(label: &str, r: &Res, cost_pct_side: f64) {
    let win = if r.trades > 0 { r.wins as f64 / r.trades as f64 * 100.0 } else { 0.0 };
    let pf = if r.gross_loss > 1e-9 { r.gross_win / r.gross_loss } else { f64::INFINITY };
    // Round-trip transaction cost: brokerage + slippage + spread.
    let cost = r.trades as f64 * 2.0 * cost_pct_side / 100.0 * r.entry_px;
    let net_c = r.net - cost;
    println!(
        "{:<34} {:>6} {:>7} {:>7} {:>9} {:>8} {:>9} {:>8} {:>6} {:>8}",
        label,
        r.bricks,
        r.trades,
        format!("{:.0}%", win),
        format!("{:+.1}", r.net),
        format!("{:+.2}%", r.ret_pct),
        format!("{:+.1}", net_c),
        format!("{:+.2}%", net_c / r.entry_px * 100.0),
        format!("{:.2}", if pf.is_finite() { pf } else { 99.99 }),
        format!("{:.0}%", if r.trades > 0 { r.sl_hits as f64 / r.trades as f64 * 100.0 } else { 0.0 }),
    );
}

/// Intra-bar "phantom brick" exposure: a brick whose timestamp equals the LAST
/// (still-forming) raw candle is one the legacy chart draws before the bar has
/// closed - it can move / vanish until the bar closes. Non-repaint never exposes
/// such a brick (it only ever shows committed, closed-candle bricks).
fn phantom_brick_bars(raw: &[Candle], cfg: &RenkoConfig) -> (usize, usize) {
    let mut bars_with_phantom = 0usize;
    let mut total_phantom = 0usize;
    for t in 1..raw.len() {
        let series = chart_type::build(ChartKind::Renko, &raw[..=t], cfg);
        let last_raw_time = raw[t].time;
        let n = series.iter().rev().take_while(|b| b.time >= last_raw_time).count();
        if n > 0 {
            bars_with_phantom += 1;
            total_phantom += n;
        }
    }
    (bars_with_phantom, total_phantom)
}

fn cfg(mode: RenkoMode, non_repaint: bool) -> RenkoConfig {
    RenkoConfig { mode, atr_length: 14, percentage: 0.1, non_repaint, source: RenkoSource::Close, ..Default::default() }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp/opencode/bt_data".into());
    let symbols = ["NIFTY", "RELIANCE", "HDFCBANK", "TCS"];
    let mut all: BTreeMap<String, Vec<f64>> = BTreeMap::new();

    for sym in symbols {
        let path = format!("{dir}/{sym}_1m.csv");
        let raw = match std::fs::read_to_string(&path) {
            Ok(_) => load_csv(&path),
            Err(_) => {
                println!("(skip {sym}: {path} not found)\n");
                continue;
            }
        };
        println!("=================================================================");
        println!("### {sym}   {} 1-min bars   last close {:.2}", raw.len(), raw.last().unwrap().close);
        println!("=================================================================");
        println!(
            "{:<34} {:>6} {:>7} {:>7} {:>9} {:>8} {:>9} {:>8} {:>6} {:>8}",
            "mode", "bricks", "trades", "win%", "netPts", "ret%", "netCost", "retC%", "PF", "SLhit%"
        );
        let cost = 0.03; // % per side (brokerage + slippage + spread), round-trip = 2x
        for (mode_name, mode) in [("ATR(14)", RenkoMode::Atr), ("Pct(0.1%)", RenkoMode::Percentage)] {
            for (nr_name, nr) in [("legacy", false), ("non-repaint", true)] {
                let series = chart_type::build(ChartKind::Renko, &raw, &cfg(mode, nr));
                let r = backtest(&raw, &series, 0.0);
                row(&format!("{mode_name} {nr_name}"), &r, cost);
                all.entry(format!("{mode_name} {nr_name}")).or_default().push(r.ret_pct);
            }
            // Stop-loss variant on ATR only (1 box fixed stop).
            if mode == RenkoMode::Atr {
                for (nr_name, nr) in [("legacy", false), ("non-repaint", true)] {
                    let series = chart_type::build(ChartKind::Renko, &raw, &cfg(mode, nr));
                    let r = backtest(&raw, &series, 1.0);
                    row(&format!("ATR(14) {nr_name} +1boxSL"), &r, cost);
                    all.entry(format!("ATR(14) {nr_name} +1boxSL")).or_default().push(r.ret_pct);
                }
            }
        }
        // Intra-bar phantom-brick exposure (bricks drawn from the forming candle).
        let (leg_bars, leg_tot) = phantom_brick_bars(&raw, &cfg(RenkoMode::Atr, false));
        let (nr_bars, nr_tot) = phantom_brick_bars(&raw, &cfg(RenkoMode::Atr, true));
        println!(
            "phantom bricks (ATR): legacy {leg_tot} over {leg_bars} bars ({:.0}% of bars) | non-repaint {nr_tot} over {nr_bars} bars",
            leg_bars as f64 / raw.len() as f64 * 100.0
        );
        println!();
    }

    println!("=================================================================");
    println!("### SUMMARY  avg %return over symbols (4 symbols)");
    println!("=================================================================");
    for (k, v) in &all {
        let avg = v.iter().sum::<f64>() / v.len() as f64;
        println!("{:<40} {:+.2}%   (per-symbol: {:?})", k, avg, v.iter().map(|x| (x * 100.0).round() / 100.0).collect::<Vec<_>>());
    }
}
