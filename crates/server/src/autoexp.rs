//! Auto Experiment engine.
//!
//! Generates indicator-filter strategy combinations, backtests each of them over
//! a multi-year window of DAILY candles for a chosen universe (indices / MCX
//! commodities / F&O stocks), ranks them by net result and lets the operator save
//! the best ones into the shared Universal Template Pool.
//!
//! Every instrument is screened for BOTH sides independently, so the operator
//! gets two separate ranked lists (bullish and bearish) instead of a single
//! winner per instrument.
//!
//! It is deliberately NOT a trading engine: it never opens positions, never
//! touches the broker order book and shares no state with the trading engines.
//! Its durable state lives in its own file (`algodhan_autoexp_experiment.json`)
//! which is separate from the Auto Experiment clone engine's book. It only READS
//! candles and WRITES templates.
//!
//! The entry rule is evaluated with the engine's own [`realtime::filter_gate`],
//! so a strategy that wins here behaves exactly the same when its template is
//! applied on any trading tab.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use algo_core::model::Candle;
use serde_json::{json, Value};

use crate::broker::DhanState;
use crate::realtime::{self, persist_template_pool, Settings, Strategy, TemplatePool};

/// The experiment's own durable file. Kept distinct from the Auto Experiment
/// clone engine's `algodhan_autoexp_state.json` book so the two never collide.
const STATE_FILE: &str = "algodhan_autoexp_experiment.json";
/// Bars of warm-up before the first simulated entry, so slow indicators (EMA
/// 100/200, Supertrend, ...) have a valid read on the earliest trade.
const WARMUP: usize = 205;

// ---------------------------------------------------------------------------
// Filter catalogue used for generation
// ---------------------------------------------------------------------------

/// Bullish filter tokens that read cleanly on daily bars. Kept small on purpose:
/// every token here multiplies the combination count, and the pairs are what the
/// engine actually trades on.
const BULL_FILTERS: &[&str] = &[
    "IncUp",
    "IncUpAll",
    "BullEmaTrend9",
    "BullEmaTrend21",
    "BullEmaTrend50",
    "BullEma9_21",
    "BullEma21_35",
    "BullEma50_100",
    "BullMeetEma9_21",
    "BullMeetEma21_35",
];

const BEAR_FILTERS: &[&str] = &[
    "IncDown",
    "IncDownAll",
    "BearEmaTrend9",
    "BearEmaTrend21",
    "BearEmaTrend50",
    "BearEma9_21",
    "BearEma21_35",
    "BearEma50_100",
    "BearMeetEma9_21",
    "BearMeetEma21_35",
];

/// One stop/target/trail prescription. `trail` is the profit-giveback percent
/// used by the position manager (`stop = entry + (peak-entry)*(1-trail/100)`).
#[derive(Clone, Copy, Debug)]
pub struct ExitVariant {
    pub label: &'static str,
    pub sl_pct: f64,
    pub rr: f64,
    pub trail_pct: f64,
}

const EXIT_VARIANTS: &[ExitVariant] = &[
    ExitVariant { label: "SL2 RR2", sl_pct: 2.0, rr: 2.0, trail_pct: 0.0 },
    ExitVariant { label: "SL1.5 RR1.5", sl_pct: 1.5, rr: 1.5, trail_pct: 0.0 },
    ExitVariant { label: "SL2 RR2 Trail5", sl_pct: 2.0, rr: 2.0, trail_pct: 5.0 },
];

// ---------------------------------------------------------------------------
// Process-wide state
// ---------------------------------------------------------------------------

fn pool_slot() -> &'static Mutex<Option<TemplatePool>> {
    static POOL: OnceLock<Mutex<Option<TemplatePool>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(None))
}

fn stop_flag() -> &'static AtomicBool {
    static STOP: AtomicBool = AtomicBool::new(false);
    &STOP
}

/// Register the shared Universal Template Pool. Called once from `main`.
pub fn init(pool: TemplatePool) {
    if let Ok(mut g) = pool_slot().lock() {
        *g = Some(pool);
    }
}

fn template_pool() -> Option<TemplatePool> {
    pool_slot().lock().ok().and_then(|g| g.clone())
}

fn state_store() -> &'static Mutex<Value> {
    static ST: OnceLock<Mutex<Value>> = OnceLock::new();
    ST.get_or_init(|| Mutex::new(default_state()))
}

fn default_state() -> Value {
    json!({
        "ok": true,
        "running": false,
        "phase": "idle",
        "universe": "",
        "years": 3,
        "done": 0,
        "total": 0,
        "current": "",
        "startedAt": 0,
        "finishedAt": 0,
        "error": "",
        "results": [],
        "bullish": [],
        "bearish": [],
        "saved": 0
    })
}

fn state_path() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("ALGODHAN_STATE_DIR") {
        return std::path::PathBuf::from(dir).join(STATE_FILE);
    }
    let dir = std::env::var("ALGODHAN_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("data"));
    if std::fs::create_dir_all(&dir).is_ok() {
        dir.join(STATE_FILE)
    } else {
        std::env::temp_dir().join(STATE_FILE)
    }
}

fn load_state() -> Value {
    let mut v = std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(default_state);
    // A run can never survive a process restart: clear the transient flags.
    v["running"] = json!(false);
    if v.get("phase").and_then(|x| x.as_str()) == Some("running")
        || v.get("phase").and_then(|x| x.as_str()) == Some("stopping")
    {
        v["phase"] = json!("idle");
    }
    v
}

/// Load the persisted experiment state into memory. Called once from `main`.
pub fn boot() {
    let loaded = load_state();
    if let Ok(mut g) = state_store().lock() {
        *g = loaded;
    }
}

fn persist_state(v: &Value) {
    if let Ok(bytes) = serde_json::to_vec(v) {
        let _ = std::fs::write(state_path(), bytes);
    }
}

fn update<F: FnOnce(&mut Value)>(f: F) {
    if let Ok(mut g) = state_store().lock() {
        f(&mut g);
        persist_state(&g);
    }
}

pub fn status_json() -> Value {
    state_store()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| json!({ "ok": false }))
}

// ---------------------------------------------------------------------------
// Universe resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Instrument {
    security_id: i64,
    name: String,
    segment: String,
    instrument: String,
    lot: f64,
    universe: String,
}

fn universe(kind: &str) -> Vec<Instrument> {
    match kind {
        "indices" => crate::marketanalysis::INDIA
            .iter()
            .map(|(sid, name, seg)| Instrument {
                security_id: *sid,
                name: (*name).to_string(),
                segment: (*seg).to_string(),
                instrument: "INDEX".to_string(),
                lot: 1.0,
                universe: "INDICES".to_string(),
            })
            .collect(),
        "fno" => {
            let mut out = Vec::new();
            for (sid, name, seg) in crate::market::liquid_stocks() {
                let lot = crate::scrip::get()
                    .and_then(|sc| sc.lot_for(&name, &seg))
                    .map(|(l, _)| l)
                    .unwrap_or(1.0);
                out.push(Instrument {
                    security_id: sid,
                    name,
                    segment: seg,
                    instrument: "EQUITY".to_string(),
                    lot,
                    universe: "FNO".to_string(),
                });
                // The whole liquid basket is far too many Dhan calls for one
                // background sweep; the most liquid names are the useful ones.
                if out.len() >= 40 {
                    break;
                }
            }
            out
        }
        _ => crate::market::commodity_rows()
            .into_iter()
            .map(|c| Instrument {
                security_id: c.security_id,
                name: c.name,
                segment: "MCX_COMM".to_string(),
                instrument: "FUTCOM".to_string(),
                lot: c.lot,
                universe: "COMMODITIES".to_string(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Combination generation
// ---------------------------------------------------------------------------

/// Every single filter and every unordered pair, per side. A single filter runs
/// alone; a pair is combined with strict AND so the saved template reproduces the
/// exact same entry rule.
fn combos(filters: &[&str]) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    for (i, a) in filters.iter().enumerate() {
        out.push((vec![(*a).to_string()], (*a).to_string()));
        for b in filters.iter().skip(i + 1) {
            let short = format!("{}+{}", a, b);
            out.push((vec![(*a).to_string(), (*b).to_string()], short));
        }
    }
    out
}

fn settings_for(keys: &[String], v: ExitVariant) -> Settings {
    let mut s = Settings::default();
    s.all_in_one = true;
    s.require_condition = true;
    for k in keys {
        s.filters.insert(k.clone(), true);
    }
    s.manual_sl = true;
    s.manual_sl_pct = v.sl_pct;
    s.rr_enabled = true;
    s.rr_value = v.rr;
    if v.trail_pct > 0.0 {
        s.manual_trail_sl = true;
        s.manual_trail_sl_pct = v.trail_pct;
    }
    s
}

fn strategy_for(inst: &Instrument, bull: bool, name: &str) -> Strategy {
    Strategy {
        id: format!("autoexp_{}_{}", inst.security_id, if bull { "b" } else { "s" }),
        name: name.to_string(),
        category: if bull { "BULLISH".into() } else { "BEARISH".into() },
        timeframe: "1day".into(),
        security_id: inst.security_id,
        exchange_segment: inst.segment.clone(),
        instrument: inst.instrument.clone(),
        trading_symbol: inst.name.clone(),
        side: if bull { "BUY".into() } else { "SELL".into() },
        enabled: true,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Backtest simulator
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug)]
struct SimResult {
    trades: usize,
    wins: usize,
    pnl: f64,
    max_dd: f64,
    profit_factor: f64,
}

/// Simulate one (candles, side, filter set, exit variant) run.
///
/// Entry: when the engine gate passes on bar `i` and the book is flat, enter at
/// that bar's close. Exit: stop-loss / risk-reward target / profit-giveback
/// trail evaluated intraday on each following bar (gap-aware: an open beyond the
/// level fills at the open). This mirrors the live position manager's rules
/// without any of the live-only concerns (margin, slices, broker routing).
fn simulate(candles: &[Candle], settings: &Settings, strat: &Strategy, is_buy: bool) -> SimResult {
    let n = candles.len();
    let mut r = SimResult::default();
    if n <= WARMUP + 5 {
        return r;
    }
    let mut equity = 0.0f64;
    let mut peak_equity = 0.0f64;
    let mut open: Option<(f64, f64, f64, f64)> = None; // (entry, sl, tp, peak)
    let mut i = WARMUP;
    while i < n {
        let c = candles[i];
        if let Some((entry, mut sl, tp, mut run_peak)) = open {
            let mut exit: Option<f64> = None;
            if is_buy {
                if c.low <= sl {
                    exit = Some(c.open.min(sl));
                } else if tp > 0.0 && c.high >= tp {
                    exit = Some(c.open.max(tp));
                }
            } else {
                if c.high >= sl {
                    exit = Some(c.open.max(sl));
                } else if tp > 0.0 && c.low <= tp {
                    exit = Some(c.open.min(tp));
                }
            }
            if let Some(px) = exit {
                let pnl = if is_buy { px - entry } else { entry - px };
                r.trades += 1;
                if pnl > 0.0 {
                    r.wins += 1;
                }
                r.pnl += pnl;
                equity += pnl;
                peak_equity = peak_equity.max(equity);
                r.max_dd = r.max_dd.max(peak_equity - equity);
                open = None;
                i += 1;
                continue;
            }
            // Ratchet the profit-giveback trail behind the best price.
            let trail_pct = if settings.manual_trail_sl { settings.manual_trail_sl_pct } else { 0.0 };
            if trail_pct > 0.0 {
                if is_buy {
                    run_peak = run_peak.max(c.high);
                    if run_peak > entry {
                        let trailed = entry + (run_peak - entry) * (1.0 - trail_pct / 100.0);
                        sl = sl.max(trailed);
                    }
                } else {
                    run_peak = run_peak.min(c.low);
                    if run_peak < entry {
                        let trailed = entry - (entry - run_peak) * (1.0 - trail_pct / 100.0);
                        sl = sl.min(trailed);
                    }
                }
            }
            open = Some((entry, sl, tp, run_peak));
            i += 1;
            continue;
        }

        let offset = n - 1 - i;
        if realtime::filter_gate(settings, strat, candles, offset) {
            let entry = c.close;
            if entry <= 0.0 {
                i += 1;
                continue;
            }
            let sl = if is_buy {
                entry * (1.0 - settings.manual_sl_pct / 100.0)
            } else {
                entry * (1.0 + settings.manual_sl_pct / 100.0)
            };
            let risk = (entry - sl).abs();
            let tp = if settings.rr_enabled && risk > 0.0 {
                if is_buy { entry + risk * settings.rr_value } else { entry - risk * settings.rr_value }
            } else {
                0.0
            };
            open = Some((entry, sl, tp, entry));
        }
        i += 1;
    }
    // Close any still-open trade at the last close so the metrics stay honest.
    if let Some((entry, _sl, _tp, _peak)) = open {
        let px = candles[n - 1].close;
        let pnl = if is_buy { px - entry } else { entry - px };
        r.trades += 1;
        if pnl > 0.0 {
            r.wins += 1;
        }
        r.pnl += pnl;
    }
    // Profit factor: gross wins / gross losses (approximated from the average).
    if r.trades > 0 {
        let win_rate = r.wins as f64 / r.trades as f64;
        r.profit_factor = if win_rate < 1.0 { win_rate / (1.0 - win_rate) } else { 99.0 };
    }
    r
}

// ---------------------------------------------------------------------------
// Run driver
// ---------------------------------------------------------------------------

/// Best (combo, variant) result for one side of one instrument, or `None` when
/// nothing clears the minimum-trade filter.
///
/// Screening is two-stage so a wide combo space stays affordable: every combo is
/// simulated ONCE with the baseline exit variant, then only the winning combo is
/// re-simulated with the alternate exit prescriptions. That cuts the sweep from
/// `combos x variants` full walks to `combos + variants`.
fn best_for_side(
    candles: &[Candle],
    inst: &Instrument,
    bull: bool,
    min_trades: usize,
) -> Option<Value> {
    if candles.len() <= WARMUP + 5 {
        return None;
    }
    let filters = if bull { BULL_FILTERS } else { BEAR_FILTERS };
    let baseline = EXIT_VARIANTS[0];
    let all = combos(filters);
    let mut best: Option<(f64, usize)> = None;
    for (idx, (keys, _short)) in all.iter().enumerate() {
        // Check the stop flag between combinations so a long sweep aborts
        // promptly instead of finishing the whole instrument first.
        if stop_flag().load(Ordering::Relaxed) {
            return None;
        }
        let settings = settings_for(keys, baseline);
        let strat = strategy_for(inst, bull, "");
        let res = simulate(candles, &settings, &strat, bull);
        if res.trades < min_trades {
            continue;
        }
        let win_rate = res.wins as f64 / res.trades as f64;
        let score = res.pnl * (0.5 + win_rate / 2.0);
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, idx));
        }
    }
    let (_, idx) = best?;
    let (keys, short) = &all[idx];
    // Re-run the winning combo against every exit prescription and keep the best.
    let mut best_row: Option<(f64, Value)> = None;
    for v in EXIT_VARIANTS {
        let settings = settings_for(keys, *v);
        let strat_name = format!("{} {}", inst.name, short);
        let strat = strategy_for(inst, bull, &strat_name);
        let res = simulate(candles, &settings, &strat, bull);
        if res.trades < min_trades {
            continue;
        }
        let win_rate = if res.trades > 0 {
            res.wins as f64 / res.trades as f64 * 100.0
        } else {
            0.0
        };
        let score = res.pnl * (0.5 + win_rate / 200.0);
        let row = json!({
            "securityId": inst.security_id,
            "symbol": inst.name,
            "universe": inst.universe,
            "segment": inst.segment,
            "instrument": inst.instrument,
            "lot": inst.lot,
            "side": if bull { "BULLISH" } else { "BEARISH" },
            "filters": keys,
            "combo": short,
            "variant": v.label,
            "slPct": v.sl_pct,
            "rr": v.rr,
            "trailPct": v.trail_pct,
            "trades": res.trades,
            "wins": res.wins,
            "winRate": (win_rate * 10.0).round() / 10.0,
            "pnlPoints": (res.pnl * 100.0).round() / 100.0,
            "pnl": (res.pnl * inst.lot * 100.0).round() / 100.0,
            "maxDdPoints": (res.max_dd * 100.0).round() / 100.0,
            "maxDd": (res.max_dd * inst.lot * 100.0).round() / 100.0,
            "profitFactor": (res.profit_factor * 100.0).round() / 100.0,
            "score": (score * 100.0).round() / 100.0,
            "bars": candles.len()
        });
        let better = best_row.as_ref().map(|(s, _)| score > *s).unwrap_or(true);
        if better {
            best_row = Some((score, row));
        }
    }
    best_row.map(|(_, row)| row)
}

fn sort_rows(rows: &mut [Value]) {
    rows.sort_by(|a, b| {
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Refresh the two per-side lists from the full result set. Bullish and bearish
/// are ranked independently so each tab shows its own top performers.
fn side_lists(rows: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut bull: Vec<Value> = rows
        .iter()
        .filter(|r| r.get("side").and_then(|x| x.as_str()) == Some("BULLISH"))
        .cloned()
        .collect();
    let mut bear: Vec<Value> = rows
        .iter()
        .filter(|r| r.get("side").and_then(|x| x.as_str()) == Some("BEARISH"))
        .cloned()
        .collect();
    sort_rows(&mut bull);
    sort_rows(&mut bear);
    (bull, bear)
}

async fn run(dhan: DhanState, universe_kind: String, years: i64, min_trades: usize) {
    stop_flag().store(false, Ordering::Relaxed);
    let list = universe(&universe_kind);
    let total = list.len();
    update(|s| {
        s["running"] = json!(true);
        s["phase"] = json!("running");
        s["universe"] = json!(universe_kind);
        s["years"] = json!(years);
        s["done"] = json!(0);
        s["total"] = json!(total);
        s["current"] = json!("");
        s["error"] = json!("");
        s["results"] = json!([]);
        s["bullish"] = json!([]);
        s["bearish"] = json!([]);
        s["saved"] = json!(0);
        s["savedNames"] = json!([]);
        s["startedAt"] = json!(crate::market::now_secs() * 1000);
        s["finishedAt"] = json!(0);
    });
    if total == 0 {
        update(|s| {
            s["running"] = json!(false);
            s["phase"] = json!("done");
            s["error"] = json!("no instruments resolved for this universe");
            s["finishedAt"] = json!(crate::market::now_secs() * 1000);
        });
        return;
    }

    let mut rows: Vec<Value> = Vec::new();
    let mut stopped = false;
    for (idx, inst) in list.iter().enumerate() {
        if stop_flag().load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        update(|s| {
            s["current"] = json!(inst.name);
            s["done"] = json!(idx);
        });
        let candles = match dhan
            .fetch_daily_history(inst.security_id, &inst.segment, &inst.instrument, years)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                update(|s| {
                    s["current"] = json!(format!("{}: {}", inst.name, e));
                });
                continue;
            }
        };
        if stop_flag().load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        // Screen BOTH sides so bullish and bearish each get a representative.
        if let Some(row) = best_for_side(&candles, inst, true, min_trades) {
            rows.push(row);
        }
        if !stop_flag().load(Ordering::Relaxed) {
            if let Some(row) = best_for_side(&candles, inst, false, min_trades) {
                rows.push(row);
            }
        }
        // Publish incrementally so the tables fill up DURING the run instead of
        // staying empty until the very end.
        let mut sorted = rows.clone();
        sort_rows(&mut sorted);
        let (bull, bear) = side_lists(&sorted);
        update(|s| {
            s["results"] = json!(sorted);
            s["bullish"] = json!(bull);
            s["bearish"] = json!(bear);
            s["done"] = json!(idx + 1);
        });
    }

    sort_rows(&mut rows);
    let (bull, bear) = side_lists(&rows);

    update(|s| {
        s["running"] = json!(false);
        s["phase"] = json!(if stopped { "stopped" } else { "done" });
        s["results"] = json!(rows);
        s["bullish"] = json!(bull);
        s["bearish"] = json!(bear);
        s["finishedAt"] = json!(crate::market::now_secs() * 1000);
        s["current"] = json!("");
    });
}

pub fn start(dhan: DhanState, v: &Value) -> Value {
    let running = state_store()
        .lock()
        .map(|g| g.get("running").and_then(|x| x.as_bool()).unwrap_or(false))
        .unwrap_or(false);
    if running {
        return json!({ "ok": false, "error": "experiment already running" });
    }
    let kind = v
        .get("universe")
        .and_then(|x| x.as_str())
        .unwrap_or("commodities")
        .to_string();
    let years = v.get("years").and_then(|x| x.as_i64()).unwrap_or(3).clamp(1, 8);
    let min_trades = v
        .get("minTrades")
        .and_then(|x| x.as_i64())
        .unwrap_or(8)
        .clamp(1, 1000) as usize;
    let dhan2 = dhan.clone();
    tokio::spawn(async move {
        run(dhan2, kind, years, min_trades).await;
    });
    json!({ "ok": true, "started": true })
}

pub fn stop() -> Value {
    stop_flag().store(true, Ordering::Relaxed);
    update(|s| {
        if s.get("running").and_then(|x| x.as_bool()) == Some(true) {
            s["phase"] = json!("stopping");
        }
    });
    json!({ "ok": true })
}

pub fn clear() -> Value {
    let running = state_store()
        .lock()
        .map(|g| g.get("running").and_then(|x| x.as_bool()).unwrap_or(false))
        .unwrap_or(false);
    if running {
        return json!({ "ok": false, "error": "stop the experiment first" });
    }
    update(|s| {
        *s = default_state();
    });
    json!({ "ok": true })
}

// ---------------------------------------------------------------------------
// Save winners to the Universal Template Pool
// ---------------------------------------------------------------------------

fn sanitize(s: &str) -> String {
    // Tokenise on whitespace; inside each token collapse every run of unsafe
    // chars to a single '+' and trim stray '+' from the ends. This keeps a
    // symbol like "GOLD 28AUG26/TEST" readable ("GOLD 28AUG26+TEST") without
    // ever producing "++" or leading/trailing separators.
    s.split_whitespace()
        .map(|tok| {
            let mut out = String::with_capacity(tok.len());
            let mut last_plus = false;
            for c in tok.chars() {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    out.push(c);
                    last_plus = false;
                } else if !last_plus {
                    out.push('+');
                    last_plus = true;
                }
            }
            out.trim_matches('+').to_string()
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_template(inst_row: &Value, serial: usize) -> (String, Value) {
    let symbol = inst_row.get("symbol").and_then(|x| x.as_str()).unwrap_or("?");
    let universe = inst_row.get("universe").and_then(|x| x.as_str()).unwrap_or("AE");
    let combo = inst_row.get("combo").and_then(|x| x.as_str()).unwrap_or("filter");
    let side = inst_row.get("side").and_then(|x| x.as_str()).unwrap_or("BULLISH");
    let variant = inst_row.get("variant").and_then(|x| x.as_str()).unwrap_or("");
    let name = sanitize(&format!("AE-{universe}-{serial:03} {symbol} {side} {combo} {variant}"));
    let keys: Vec<String> = inst_row
        .get("filters")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let sl_pct = inst_row.get("slPct").and_then(|x| x.as_f64()).unwrap_or(2.0);
    let rr = inst_row.get("rr").and_then(|x| x.as_f64()).unwrap_or(2.0);
    let trail = inst_row.get("trailPct").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let variant = ExitVariant { label: "AE", sl_pct, rr, trail_pct: trail };
    let settings = settings_for(&keys, variant);
    let settings_json = serde_json::to_value(&settings).unwrap_or(json!({}));
    let bull = side == "BULLISH";
    let instrument = inst_row.get("instrument").and_then(|x| x.as_str()).unwrap_or("FUTCOM");
    let segment = inst_row.get("segment").and_then(|x| x.as_str()).unwrap_or("MCX_COMM");
    let sid = inst_row.get("securityId").and_then(|x| x.as_i64()).unwrap_or(0);
    let lot = inst_row.get("lot").and_then(|x| x.as_f64()).unwrap_or(1.0);
    let strat = json!({
        "id": format!("autoexp_{}_{}", sid, if bull { "b" } else { "s" }),
        "name": name,
        "category": side,
        "timeframe": "1day",
        "securityId": sid,
        "exchangeSegment": segment,
        "instrument": instrument,
        "tradingSymbol": symbol,
        "side": if bull { "BUY" } else { "SELL" },
        "enabled": true,
        "conditions": [],
        "synthetic": false,
        "lot": lot
    });
    let tpl = json!({
        "name": name.clone(),
        "side": if bull { "bullish" } else { "bearish" },
        "savedAt": crate::market::now_secs() * 1000,
        "settings": settings_json,
        "selected": [],
        "strategies": [strat],
        "origin": "Auto Experiment",
        "originId": "autoexp"
    });
    (name, tpl)
}

/// Insert the given result rows into the Universal Template Pool, continuing the
/// `AE-...` serial counter. Returns `(saved names, new serial)`.
fn save_rows(pool: &TemplatePool, rows: &[Value], wanted: Option<&[String]>) -> (Vec<String>, usize) {
    let existing: Vec<String> = pool.lock().map(|p| p.keys().cloned().collect()).unwrap_or_default();
    let mut serial: usize = existing
        .iter()
        .filter_map(|n| n.strip_prefix("AE-"))
        .filter_map(|rest| rest.split('-').nth(1))
        .filter_map(|num| num.parse::<usize>().ok())
        .max()
        .unwrap_or(0);

    let mut saved_names: Vec<String> = Vec::new();
    let mut inserted: Vec<(String, Value)> = Vec::new();
    for row in rows {
        let key = format!(
            "{}|{}|{}|{}",
            row.get("securityId").and_then(|x| x.as_i64()).unwrap_or(0),
            row.get("side").and_then(|x| x.as_str()).unwrap_or(""),
            row.get("combo").and_then(|x| x.as_str()).unwrap_or(""),
            row.get("variant").and_then(|x| x.as_str()).unwrap_or("")
        );
        if let Some(list) = wanted {
            if !list.iter().any(|k| *k == key) {
                continue;
            }
        }
        serial += 1;
        let (name, tpl) = build_template(row, serial);
        inserted.push((name.clone(), tpl));
        saved_names.push(name);
    }
    if !inserted.is_empty() {
        if let Ok(mut p) = pool.lock() {
            for (name, tpl) in &inserted {
                p.insert(name.clone(), tpl.clone());
            }
        }
        persist_template_pool(pool);
    }
    (saved_names, serial)
}

pub fn save(v: &Value) -> Value {
    let Some(pool) = template_pool() else {
        return json!({ "ok": false, "error": "template pool not initialised" });
    };
    let (rows, save_all) = state_store()
        .lock()
        .map(|g| {
            let rows = g.get("results").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            let all = v.get("all").and_then(|x| x.as_bool()).unwrap_or(true);
            (rows, all)
        })
        .unwrap_or((Vec::new(), true));
    if rows.is_empty() {
        return json!({ "ok": false, "error": "no results to save - run an experiment first" });
    }

    let wanted: Option<Vec<String>> = if save_all {
        None
    } else {
        v.get("keys").and_then(|x| x.as_array()).map(|a| {
            a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
        })
    };

    let (saved_names, serial) = save_rows(&pool, &rows, wanted.as_deref());
    if saved_names.is_empty() {
        return json!({ "ok": false, "error": "nothing matched the requested rows" });
    }
    update(|s| {
        s["saved"] = json!(saved_names.len());
        s["savedNames"] = json!(saved_names);
    });
    json!({ "ok": true, "saved": saved_names.len(), "names": saved_names, "serial": serial })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize, start: f64, step: f64) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let c = start + step * i as f64;
                Candle {
                    time: i as i64 * 86_400,
                    open: c - step,
                    high: c + step.abs(),
                    low: c - step.abs(),
                    close: c,
                    volume: 1000.0 + i as f64,
                }
            })
            .collect()
    }

    fn inst(universe: &str) -> Instrument {
        Instrument {
            security_id: 13,
            name: "NIFTY 50".into(),
            segment: "IDX_I".into(),
            instrument: "INDEX".into(),
            lot: 1.0,
            universe: universe.into(),
        }
    }

    #[test]
    fn combos_has_singles_and_pairs() {
        let c = combos(&["A", "B", "C"]);
        assert!(c.iter().any(|(k, _)| k == &vec!["A".to_string()]));
        assert!(c.iter().any(|(k, _)| k == &vec!["A".to_string(), "B".to_string()]));
        assert_eq!(c.len(), 3 + 3);
    }

    #[test]
    fn strict_and_settings_tick_every_filter() {
        let keys = vec!["IncUp".to_string(), "BullEma9_21".to_string()];
        let s = settings_for(&keys, EXIT_VARIANTS[0]);
        assert!(s.require_condition);
        assert!(s.all_in_one);
        assert_eq!(s.filters.get("IncUp"), Some(&true));
        assert_eq!(s.filters.get("BullEma9_21"), Some(&true));
    }

    #[test]
    fn simulate_buy_on_uptrend_produces_trades() {
        let candles = ramp(400, 100.0, 0.5);
        let keys = vec!["IncUp".to_string()];
        let s = settings_for(&keys, EXIT_VARIANTS[0]);
        let st = strategy_for(&inst("INDICES"), true, "NIFTY IncUp");
        let r = simulate(&candles, &s, &st, true);
        assert!(r.trades > 0, "uptrend must produce trades");
        assert!(r.pnl > 0.0, "an always-up series must be profitable long");
    }

    #[test]
    fn simulate_returns_none_on_short_series() {
        let candles = ramp(10, 100.0, 1.0);
        let keys = vec!["IncUp".to_string()];
        let s = settings_for(&keys, EXIT_VARIANTS[0]);
        let st = strategy_for(&inst("INDICES"), true, "x");
        let r = simulate(&candles, &s, &st, true);
        assert_eq!(r.trades, 0);
    }

    #[test]
    fn sanitize_strips_symbol_noise() {
        assert_eq!(sanitize("GOLD 28AUG26/TEST"), "GOLD 28AUG26+TEST");
        assert_eq!(sanitize("AE-X 001 A/B (C)+D"), "AE-X 001 A+B C+D");
        assert_eq!(sanitize("  a   b  "), "a b");
    }

    #[test]
    fn template_embeds_strategy_and_settings() {
        let row = json!({
            "securityId": 13,
            "symbol": "NIFTY 50",
            "universe": "INDICES",
            "segment": "IDX_I",
            "instrument": "INDEX",
            "lot": 1.0,
            "side": "BULLISH",
            "filters": ["IncUp"],
            "combo": "IncUp",
            "variant": "SL2 RR2",
            "slPct": 2.0,
            "rr": 2.0,
            "trailPct": 0.0
        });
        let (name, tpl) = build_template(&row, 1);
        assert!(name.starts_with("AE-INDICES-001"));
        assert!(name.contains("BULLISH"));
        assert!(name.contains("IncUp"));
        assert_eq!(tpl["strategies"][0]["securityId"], 13);
        assert_eq!(tpl["side"], "bullish");
        assert_eq!(tpl["settings"]["filters"]["IncUp"], true);
    }

    #[test]
    fn side_lists_split_and_rank_bull_and_bear() {
        let rows = vec![
            json!({"side": "BULLISH", "score": 10.0, "symbol": "A"}),
            json!({"side": "BEARISH", "score": 20.0, "symbol": "B"}),
            json!({"side": "BULLISH", "score": 30.0, "symbol": "C"}),
            json!({"side": "BEARISH", "score": 5.0, "symbol": "D"}),
        ];
        let (bull, bear) = side_lists(&rows);
        assert_eq!(bull.len(), 2);
        assert_eq!(bear.len(), 2);
        assert_eq!(bull[0]["symbol"], "C");
        assert_eq!(bear[0]["symbol"], "B");
    }

    #[test]
    fn best_for_side_returns_a_row_on_a_trend() {
        let candles = ramp(420, 100.0, 0.4);
        let row = best_for_side(&candles, &inst("INDICES"), true, 1);
        // An always-up series should clear the minimum on the bullish side.
        assert!(row.is_some(), "bullish side should produce a winner");
        let row = row.unwrap();
        assert_eq!(row["side"], "BULLISH");
        assert!(row["trades"].as_u64().unwrap() > 0);
    }
}
