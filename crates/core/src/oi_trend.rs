//! OI Trend + Levels: a fresh Rust port of the old web app's `oitrend.js`
//! (the "OI Trend" overlay on the candlestick chart).
//!
//! Two halves, both pure and unit-tested here so the server and the WASM chart
//! share one source of truth:
//!   * [`level_data`] / [`oi_rows`] turn an option-chain snapshot into PCR,
//!     Max Pain, CE/PE OI walls, expected range and the per-strike CE/PE
//!     interplay around spot.
//!   * [`regime_line`] builds the EMA-like trend-state line from candles, and
//!     [`context_of`] + [`classify`] fuse the price methods, volume trend, PCR
//!     and per-strike OI into the arrow / label / reversal / consolidation
//!     state shown next to the line.
//!
//! Numeric conventions deliberately mirror the JavaScript original (EMA seeded
//! on the first value, Wilder ATR seeded on the first true range, `|| 0` on
//! non-finite chain fields) so the drawn line and the engine reading stay
//! identical to the old app.

use serde::{Deserialize, Serialize};

use crate::model::Candle;

// ---------------------------------------------------------------------------
// Normalized option-chain row
// ---------------------------------------------------------------------------

/// One strike of an option-chain snapshot. Field names are canonical; the
/// server is responsible for mapping Dhan's response into this shape.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct OiRecord {
    pub strike: f64,
    #[serde(default)]
    pub ce_oi: f64,
    #[serde(default)]
    pub pe_oi: f64,
    #[serde(default)]
    pub ce_chg: f64,
    #[serde(default)]
    pub pe_chg: f64,
    #[serde(default)]
    pub ce_vol: f64,
    #[serde(default)]
    pub pe_vol: f64,
    #[serde(default)]
    pub ce_ltp: f64,
    #[serde(default)]
    pub pe_ltp: f64,
    #[serde(default)]
    pub ce_iv: f64,
    #[serde(default)]
    pub pe_iv: f64,
}

fn fin(x: f64) -> f64 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Basic series helpers (JS-parity seeding)
// ---------------------------------------------------------------------------

/// `emaArr(vals, n)`: EMA seeded on the FIRST value (not an SMA seed), NaN
/// values forward-fill the previous value.
pub fn ema_first(vals: &[f64], n: usize) -> Vec<f64> {
    let len = vals.len();
    let mut out = vec![0.0; len];
    if len == 0 || n == 0 {
        return out;
    }
    let k = 2.0 / (n as f64 + 1.0);
    let mut p = vals[0];
    for i in 0..len {
        let v = vals[i];
        if v.is_nan() {
            out[i] = p;
            continue;
        }
        p = if i == 0 { v } else { v * k + p * (1.0 - k) };
        out[i] = p;
    }
    out
}

/// `smaArr(vals, n)`: NaN/absent values count as 0; missing warmup = NaN.
pub fn sma_fill0(vals: &[f64], n: usize) -> Vec<f64> {
    let len = vals.len();
    let mut out = vec![f64::NAN; len];
    if n == 0 {
        return out;
    }
    let mut sum = 0.0f64;
    for i in 0..len {
        let v = if vals[i].is_nan() { 0.0 } else { vals[i] };
        sum += v;
        if i >= n {
            let old = if vals[i - n].is_nan() { 0.0 } else { vals[i - n] };
            sum -= old;
        }
        if i + 1 >= n {
            out[i] = sum / n as f64;
        }
    }
    out
}

fn true_range_at(c: &Candle, prev: Option<&Candle>) -> f64 {
    match prev {
        None => c.high - c.low,
        Some(p) => (c.high - c.low)
            .max((c.high - p.close).abs())
            .max((c.low - p.close).abs()),
    }
}

/// `atrArr(candles, n)`: Wilder smoothing seeded on the first candle's TR.
pub fn atr_first(candles: &[Candle], n: usize) -> Vec<f64> {
    let len = candles.len();
    let mut out = vec![f64::NAN; len];
    if len == 0 || n == 0 {
        return out;
    }
    let mut tr = true_range_at(&candles[0], None);
    out[0] = tr;
    for i in 1..len {
        tr = true_range_at(&candles[i], Some(&candles[i - 1]));
        let prev = out[i - 1];
        out[i] = if prev.is_nan() {
            tr
        } else {
            (prev * (n as f64 - 1.0) + tr) / n as f64
        };
    }
    out
}

// ---------------------------------------------------------------------------
// Normal distribution + ATM greeks
// ---------------------------------------------------------------------------

/// Abramowitz-Stegun 7.1.26 normal CDF (matches the JS client).
pub fn norm_cdf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.2316419 * x.abs());
    let d = 0.3989422804014327 * (-x * x / 2.0).exp();
    let p = d
        * t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));
    if x >= 0.0 {
        1.0 - p
    } else {
        p
    }
}

pub fn norm_pdf(x: f64) -> f64 {
    0.3989422804014327 * (-x * x / 2.0).exp()
}

/// ATM greeks for the call: `(delta, vega)`; `None` on invalid inputs.
pub fn atm_greeks(spot: f64, strike: f64, iv_pct: f64, dte_days: f64) -> Option<(f64, f64)> {
    let s = spot;
    let k = strike;
    let sig = iv_pct / 100.0;
    let t = if dte_days.is_finite() && dte_days != 0.0 {
        (dte_days.max(0.5)) / 365.0
    } else {
        7.0f64.max(0.5) / 365.0
    };
    if !(s > 0.0 && k > 0.0 && sig > 0.0 && t > 0.0) {
        return None;
    }
    let root = sig * t.sqrt();
    let d1 = ((s / k).ln() + (0.06 + 0.5 * sig * sig) * t) / root;
    Some((norm_cdf(d1), s * norm_pdf(d1) * t.sqrt()))
}

// ---------------------------------------------------------------------------
// Level geometry
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Wall {
    /// "res" (call wall above spot) or "sup" (put wall below spot).
    pub kind: String,
    pub strike: f64,
    pub oi: f64,
    pub chg: f64,
    pub vol: f64,
    pub fresh: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NearStrike {
    pub strike: f64,
    pub oi: f64,
    pub chg: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelData {
    pub rows: usize,
    pub pcr: Option<f64>,
    pub pcr_chg: Option<f64>,
    pub total_ce_oi: f64,
    pub total_pe_oi: f64,
    pub total_ce_chg: f64,
    pub total_pe_chg: f64,
    pub max_pain: Option<f64>,
    pub walls: Vec<Wall>,
    pub atm_strike: Option<f64>,
    pub exp_move: Option<f64>,
    pub exp_hi: Option<f64>,
    pub exp_lo: Option<f64>,
    pub iv: Option<f64>,
    pub spot: Option<f64>,
    pub delta: Option<f64>,
    pub vega: Option<f64>,
    pub res_overhead: f64,
    pub sup_under: f64,
    pub net_oi: f64,
    pub box_oi: f64,
    pub near_ce: Option<NearStrike>,
    pub near_pe: Option<NearStrike>,
    pub pcr_band: Option<f64>,
    pub ce_oi_band: f64,
    pub pe_oi_band: f64,
}

impl Default for LevelData {
    fn default() -> Self {
        LevelData {
            rows: 0,
            pcr: None,
            pcr_chg: None,
            total_ce_oi: 0.0,
            total_pe_oi: 0.0,
            total_ce_chg: 0.0,
            total_pe_chg: 0.0,
            max_pain: None,
            walls: Vec::new(),
            atm_strike: None,
            exp_move: None,
            exp_hi: None,
            exp_lo: None,
            iv: None,
            spot: None,
            delta: None,
            vega: None,
            res_overhead: 0.0,
            sup_under: 0.0,
            net_oi: 0.0,
            box_oi: 0.0,
            near_ce: None,
            near_pe: None,
            pcr_band: None,
            ce_oi_band: 0.0,
            pe_oi_band: 0.0,
        }
    }
}

/// Tunables for [`level_data`]; defaults match the JS original.
#[derive(Clone, Copy, Debug)]
pub struct LevelOpts {
    pub cluster_pct: f64,
    pub oi_thresh_pct: f64,
    pub max_walls: usize,
    pub dte_days: f64,
    pub oi_near_pct: f64,
}

impl Default for LevelOpts {
    fn default() -> Self {
        LevelOpts {
            cluster_pct: 0.004,
            oi_thresh_pct: 0.05,
            max_walls: 10,
            dte_days: 7.0,
            oi_near_pct: 0.006,
        }
    }
}

/// Chain snapshot -> level geometry. Mirrors `levelData()` in `oitrend.js`.
pub fn level_data(records: &[OiRecord], spot: f64, opts: &LevelOpts) -> LevelData {
    let mut res = LevelData::default();
    res.spot = if spot > 0.0 { Some(spot) } else { None };
    if records.is_empty() {
        return res;
    }

    struct Row {
        r: OiRecord,
    }
    let mut list: Vec<Row> = Vec::new();
    let (mut ce_oi, mut pe_oi, mut ce_chg, mut pe_chg) = (0.0, 0.0, 0.0, 0.0);
    let (mut iv_sum, mut iv_n) = (0.0, 0.0);
    let mut atm_dist = f64::INFINITY;
    let mut atm_idx: Option<usize> = None;

    for rec in records {
        let s = fin(rec.strike);
        if !(s > 0.0) {
            continue;
        }
        let c_oi = fin(rec.ce_oi.max(0.0));
        let p_oi = fin(rec.pe_oi.max(0.0));
        let c_chg = fin(rec.ce_chg);
        let p_chg = fin(rec.pe_chg);
        let idx = list.len();
        list.push(Row {
            r: OiRecord {
                strike: s,
                ce_oi: c_oi,
                pe_oi: p_oi,
                ce_chg: c_chg,
                pe_chg: p_chg,
                ce_vol: fin(rec.ce_vol),
                pe_vol: fin(rec.pe_vol),
                ce_ltp: fin(rec.ce_ltp),
                pe_ltp: fin(rec.pe_ltp),
                ce_iv: fin(rec.ce_iv),
                pe_iv: fin(rec.pe_iv),
            },
        });
        ce_oi += c_oi;
        pe_oi += p_oi;
        ce_chg += c_chg;
        pe_chg += p_chg;
        if spot > 0.0 {
            let d = (s - spot).abs();
            if d < atm_dist {
                atm_dist = d;
                atm_idx = Some(idx);
            }
            let a = fin(rec.ce_iv);
            let b = fin(rec.pe_iv);
            if a > 0.0 && b > 0.0 {
                iv_sum += (a + b) / 2.0;
                iv_n += 1.0;
            }
        }
    }

    res.rows = list.len();
    res.total_ce_oi = ce_oi;
    res.total_pe_oi = pe_oi;
    res.total_ce_chg = ce_chg;
    res.total_pe_chg = pe_chg;
    res.pcr = if ce_oi > 0.0 { Some(pe_oi / ce_oi) } else { None };
    res.pcr_chg = if ce_chg != 0.0 {
        Some(pe_chg / ce_chg)
    } else if pe_chg != 0.0 {
        Some(if pe_chg > 0.0 { 99.0 } else { -99.0 })
    } else {
        None
    };

    if spot > 0.0 {
        if let Some(ai) = atm_idx {
            let atm = list[ai].r;
            res.atm_strike = Some(atm.strike);
            let iv = if iv_n > 0.0 {
                Some(iv_sum / iv_n)
            } else {
                let v = (atm.ce_iv + atm.pe_iv) / 2.0;
                if v != 0.0 {
                    Some(v)
                } else {
                    None
                }
            };
            res.iv = iv;
            let dte = opts.dte_days;
            let mut mv: Option<f64> = None;
            if let Some(ivv) = iv {
                let cand = spot * (ivv / 100.0) * (dte.max(0.5) / 365.0).sqrt();
                if cand.is_finite() {
                    mv = Some(cand);
                }
                if let Some((d, v)) = atm_greeks(spot, atm.strike, ivv, dte) {
                    res.delta = Some(d);
                    res.vega = Some(v);
                }
            }
            if !(mv.unwrap_or(0.0) > 0.0) {
                let st = atm.ce_ltp + atm.pe_ltp;
                if st > 0.0 {
                    mv = Some(st * 1.25);
                }
            }
            if let Some(m) = mv {
                if m > 0.0 {
                    res.exp_move = Some(m);
                    res.exp_hi = Some(spot + m);
                    res.exp_lo = Some(spot - m);
                }
            }
        }
    }

    // Max pain: strike minimizing total ITM option payout.
    if spot > 0.0 && !list.is_empty() {
        let mut best: Option<f64> = None;
        let mut best_pain = f64::INFINITY;
        for cand in &list {
            if cand.r.ce_oi + cand.r.pe_oi <= 0.0 {
                continue;
            }
            let mut pain = 0.0;
            for r in &list {
                if r.r.strike >= cand.r.strike {
                    pain += r.r.ce_oi * (r.r.strike - cand.r.strike);
                }
                if r.r.strike <= cand.r.strike {
                    pain += r.r.pe_oi * (cand.r.strike - r.r.strike);
                }
            }
            if pain < best_pain {
                best_pain = pain;
                best = Some(cand.r.strike);
            }
        }
        res.max_pain = best;
    }

    // CE walls above spot / PE walls below spot, clustered then strongest-first.
    if spot > 0.0 && !list.is_empty() {
        #[derive(Clone, Copy)]
        struct SideRow {
            strike: f64,
            oi: f64,
            chg: f64,
            vol: f64,
        }
        let cluster = |mut rows: Vec<SideRow>| -> Vec<(f64, f64, f64, f64)> {
            if rows.is_empty() {
                return Vec::new();
            }
            rows.sort_by(|a, b| a.strike.partial_cmp(&b.strike).unwrap());
            let mut groups: Vec<Vec<SideRow>> = Vec::new();
            let mut cur: Vec<SideRow> = Vec::new();
            let mut lim = f64::NEG_INFINITY;
            for r in rows {
                if cur.is_empty() || r.strike > lim {
                    if !cur.is_empty() {
                        groups.push(std::mem::take(&mut cur));
                    }
                    cur = Vec::new();
                }
                cur.push(r);
                lim = r.strike * (1.0 + opts.cluster_pct);
            }
            if !cur.is_empty() {
                groups.push(cur);
            }
            let mut out: Vec<(f64, f64, f64, f64)> = groups
                .into_iter()
                .map(|g| {
                    let (mut oi, mut chg, mut vol, mut w_sum, mut w_oi) = (0.0, 0.0, 0.0, 0.0, 0.0);
                    for r in &g {
                        oi += r.oi;
                        chg += r.chg;
                        vol += r.vol;
                        w_sum += r.oi * r.strike;
                        w_oi += r.oi;
                    }
                    let strike = if w_oi != 0.0 { w_sum / w_oi } else { g[0].strike };
                    (strike, oi, chg, vol)
                })
                .collect();
            out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            out
        };
        let c_rows: Vec<SideRow> = list
            .iter()
            .filter(|r| r.r.strike > spot && r.r.ce_oi > 0.0)
            .map(|r| SideRow {
                strike: r.r.strike,
                oi: r.r.ce_oi,
                chg: r.r.ce_chg,
                vol: r.r.ce_vol,
            })
            .collect();
        let p_rows: Vec<SideRow> = list
            .iter()
            .filter(|r| r.r.strike < spot && r.r.pe_oi > 0.0)
            .map(|r| SideRow {
                strike: r.r.strike,
                oi: r.r.pe_oi,
                chg: r.r.pe_chg,
                vol: r.r.pe_vol,
            })
            .collect();
        let t_ce = if ce_oi != 0.0 { ce_oi } else { 1.0 };
        let t_pe = if pe_oi != 0.0 { pe_oi } else { 1.0 };
        let mut walls: Vec<Wall> = Vec::new();
        for (strike, oi, chg, vol) in cluster(c_rows) {
            if oi >= t_ce * opts.oi_thresh_pct {
                walls.push(Wall {
                    kind: "res".into(),
                    strike: strike.round(),
                    oi,
                    chg,
                    vol,
                    fresh: chg > 0.0,
                });
            }
        }
        for (strike, oi, chg, vol) in cluster(p_rows) {
            if oi >= t_pe * opts.oi_thresh_pct {
                walls.push(Wall {
                    kind: "sup".into(),
                    strike: strike.round(),
                    oi,
                    chg,
                    vol,
                    fresh: chg > 0.0,
                });
            }
        }
        walls.sort_by(|a, b| b.oi.partial_cmp(&a.oi).unwrap());
        walls.truncate(opts.max_walls);
        res.walls = walls;
    }

    // Per-strike CE/PE interplay right around spot.
    if spot > 0.0 && !list.is_empty() {
        let near = opts.oi_near_pct;
        let lo_b = spot * (1.0 - near);
        let hi_b = spot * (1.0 + near);
        let mut near_ce = NearStrike::default();
        let mut near_pe = NearStrike::default();
        let (mut ce_b, mut pe_b) = (0.0, 0.0);
        for r in &list {
            if r.r.strike > spot && r.r.strike <= hi_b && r.r.ce_oi > near_ce.oi {
                near_ce = NearStrike {
                    strike: r.r.strike,
                    oi: r.r.ce_oi,
                    chg: r.r.ce_chg,
                };
            }
            if r.r.strike < spot && r.r.strike >= lo_b && r.r.pe_oi > near_pe.oi {
                near_pe = NearStrike {
                    strike: r.r.strike,
                    oi: r.r.pe_oi,
                    chg: r.r.pe_chg,
                };
            }
            if r.r.strike >= lo_b && r.r.strike <= hi_b {
                ce_b += r.r.ce_oi;
                pe_b += r.r.pe_oi;
            }
        }
        let tot = near_ce.oi + near_pe.oi;
        res.res_overhead = if tot > 0.0 { near_ce.oi / tot } else { 0.0 };
        res.sup_under = if tot > 0.0 { near_pe.oi / tot } else { 0.0 };
        res.net_oi = if tot > 0.0 {
            (near_pe.oi - near_ce.oi) / tot
        } else {
            0.0
        };
        res.box_oi = res.res_overhead.min(res.sup_under);
        res.near_ce = if near_ce.oi > 0.0 { Some(near_ce) } else { None };
        res.near_pe = if near_pe.oi > 0.0 { Some(near_pe) } else { None };
        res.pcr_band = if ce_b > 0.0 {
            Some(pe_b / ce_b)
        } else if pe_b > 0.0 {
            Some(3.0)
        } else {
            None
        };
        res.ce_oi_band = ce_b;
        res.pe_oi_band = pe_b;
    }

    res
}

/// Active-OI rows (ascending strikes) for the ordered premium-chart strip.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OiRow {
    pub strike: f64,
    pub ce_oi: f64,
    pub pe_oi: f64,
    pub ce_chg: f64,
    pub pe_chg: f64,
}

/// Mirrors `oiRows()` in `oitrend.js`.
pub fn oi_rows(records: &[OiRecord], spot: f64, range_pct: f64) -> Vec<OiRow> {
    let mut out: Vec<OiRow> = Vec::new();
    for r in records {
        let s = fin(r.strike);
        if !(s > 0.0) {
            continue;
        }
        if spot > 0.0 && (s - spot).abs() / spot > range_pct {
            continue;
        }
        let ce_oi = fin(r.ce_oi.max(0.0));
        let pe_oi = fin(r.pe_oi.max(0.0));
        if ce_oi + pe_oi <= 0.0 {
            continue;
        }
        out.push(OiRow {
            strike: s,
            ce_oi,
            pe_oi,
            ce_chg: fin(r.ce_chg),
            pe_chg: fin(r.pe_chg),
        });
    }
    out.sort_by(|a, b| a.strike.partial_cmp(&b.strike).unwrap());
    out
}

// ---------------------------------------------------------------------------
// Regime trend line
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Regime {
    Up,
    Down,
    Flat,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TrendPoint {
    pub time: i64,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegimeLast {
    pub regime: Regime,
    pub ema_f: f64,
    pub ema_s: f64,
    pub atr: f64,
    pub vol: f64,
    pub vol_avg: f64,
    pub close: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegimeResult {
    pub data: Vec<TrendPoint>,
    pub regs: Vec<Regime>,
    pub last: RegimeLast,
}

#[derive(Clone, Copy, Debug)]
pub struct RegimeOpts {
    pub fast: usize,
    pub slow: usize,
    pub enter_k: f64,
    pub exit_k: f64,
    pub atr_len: usize,
    pub vol_avg_len: usize,
}

impl Default for RegimeOpts {
    fn default() -> Self {
        RegimeOpts {
            fast: 9,
            slow: 21,
            enter_k: 0.45,
            exit_k: 0.12,
            atr_len: 14,
            vol_avg_len: 20,
        }
    }
}

/// Trend-state line with hysteresis, drawn as STRAIGHT segments. The regime is
/// still decided the same way as `regimeLine()` (fast/slow EMA + ATR band), but
/// the plotted line does not follow the EMA tick-by-tick: every regime run is a
/// single straight line from its first to its last value. This means a run that
/// is classified flat but whose fast EMA still drifts (common while the EMAs
/// are converging during a move) is drawn as a sloped straight line following
/// that drift, and only a genuinely unchanged run stays horizontal.
pub fn regime_line(candles: &[Candle], opts: &RegimeOpts) -> RegimeResult {
    let n = candles.len();
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let vols: Vec<f64> = candles.iter().map(|c| c.volume).collect();
    let e_f = ema_first(&closes, opts.fast);
    let e_s = ema_first(&closes, opts.slow);
    let atrs = atr_first(candles, opts.atr_len);
    let vol_avg = sma_fill0(&vols, opts.vol_avg_len);

    let mut regs = vec![Regime::Flat; n];
    let mut regime = Regime::Flat;
    for i in 0..n {
        let at = if atrs[i].is_nan() { 0.0 } else { atrs[i] };
        if at > 0.0 {
            let d = e_f[i] - e_s[i];
            match regime {
                Regime::Flat => {
                    if d > at * opts.enter_k {
                        regime = Regime::Up;
                    } else if d < -at * opts.enter_k {
                        regime = Regime::Down;
                    }
                }
                Regime::Up => {
                    if d < -at * opts.exit_k {
                        regime = Regime::Flat;
                    }
                }
                Regime::Down => {
                    if d > at * opts.exit_k {
                        regime = Regime::Flat;
                    }
                }
            }
        }
        regs[i] = regime;
    }

    // Straight trend segments: each run of the same regime is drawn as ONE
    // straight line from its first to its last fast-EMA value instead of
    // following the EMA tick-by-tick, so the overlay reads as clean straight
    // trend lines (not an EMA-like curve). Every run is interpolated, including
    // flat runs: if the fast EMA drifted while the regime stayed flat, the
    // segment slopes with that drift (so a bullish/bearish move never shows as
    // a horizontal plate); a run with no change stays horizontal. Consecutive
    // runs share the boundary EMA value, so the segments join without steps.
    let mut data = Vec::with_capacity(n);
    let mut i = 0usize;
    while i < n {
        let r = regs[i];
        let mut j = i;
        while j + 1 < n && regs[j + 1] == r {
            j += 1;
        }
        let t0 = candles[i].time as f64;
        let t1 = candles[j].time as f64;
        let v0 = e_f[i];
        let v1 = e_f[j];
        let dt = t1 - t0;
        for k in i..=j {
            let value = if dt.abs() > 0.0 {
                let w = (candles[k].time as f64 - t0) / dt;
                v0 + (v1 - v0) * w
            } else {
                e_f[k]
            };
            data.push(TrendPoint {
                time: candles[k].time,
                value: (value * 100.0).round() / 100.0,
            });
        }
        i = j + 1;
    }

    let li = n.saturating_sub(1);
    let last = if n == 0 {
        RegimeLast {
            regime: Regime::Flat,
            ema_f: 0.0,
            ema_s: 0.0,
            atr: 0.0,
            vol: 0.0,
            vol_avg: 0.0,
            close: 0.0,
        }
    } else {
        RegimeLast {
            regime: regs[li],
            ema_f: e_f[li],
            ema_s: e_s[li],
            atr: if atrs[li].is_nan() { 0.0 } else { atrs[li] },
            vol: vols[li],
            vol_avg: if vol_avg[li].is_nan() { 0.0 } else { vol_avg[li] },
            close: closes[li],
        }
    };

    RegimeResult { data, regs, last }
}

// ---------------------------------------------------------------------------
// Supertrend series (used by the context methods)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StPoint {
    pub dir: i32,
    pub band: f64,
}

/// Direction-aware Supertrend band. Mirrors `stSeries()`.
pub fn st_series(candles: &[Candle], period: usize, factor: f64) -> Vec<StPoint> {
    let n = candles.len();
    let atrs = atr_first(candles, period);
    let mut out = Vec::with_capacity(n);
    let mut prev_f: Option<f64> = None;
    let mut prev_fu: Option<f64> = None;
    let mut prev_fd: Option<f64> = None;
    let mut dir = 1i32;
    for i in 0..n {
        let atr = if atrs[i].is_nan() { 0.0 } else { atrs[i] };
        let hl2 = (candles[i].high + candles[i].low) / 2.0;
        let up = hl2 + factor * atr;
        let dn = hl2 - factor * atr;
        let fu = match prev_fu {
            None => up,
            Some(p) => up.min(p),
        };
        let fd = match prev_fd {
            None => dn,
            Some(p) => dn.max(p),
        };
        if prev_f.is_none() {
            dir = 1;
        } else if candles[i].close > prev_fu.unwrap_or(up) {
            dir = 1;
        } else if candles[i].close < prev_fd.unwrap_or(dn) {
            dir = -1;
        }
        let band = if dir == 1 { fd } else { fu };
        prev_fu = Some(fu);
        prev_fd = Some(fd);
        prev_f = Some(band);
        out.push(StPoint { dir, band });
    }
    out
}

// ---------------------------------------------------------------------------
// Volume trend
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
pub struct VolTrend {
    pub dir: i32,
    pub slope: f64,
    pub rel: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct VolTrendOpts {
    pub win: usize,
    pub dead: f64,
    pub scale: f64,
}

impl Default for VolTrendOpts {
    fn default() -> Self {
        VolTrendOpts {
            win: 6,
            dead: 0.03,
            scale: 0.12,
        }
    }
}

/// Mirrors `volTrendOf()`.
pub fn vol_trend_of(vols: &[f64], cfg: &VolTrendOpts) -> VolTrend {
    let win = cfg.win;
    let n = vols.len();
    if n < win * 5 + 2 {
        return VolTrend::default();
    }
    let avg = |seg: &[f64]| -> f64 {
        let (mut s, mut c) = (0.0, 0usize);
        for &v in seg {
            if v > 0.0 {
                s += v;
                c += 1;
            }
        }
        if c > 0 {
            s / c as f64
        } else {
            0.0
        }
    };
    let a = avg(&vols[n - win..n]);
    let b = avg(&vols[n - win * 4..n - win]);
    if !(a > 0.0) {
        return VolTrend::default();
    }
    let rel = (a - b) / a;
    let m = if a != 0.0 { a } else { 1.0 };
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for i in 0..win {
        let x = i as f64 - (win as f64 - 1.0) / 2.0;
        let v = vols[n - win + i];
        sxy += x * v;
        sxx += x * x;
    }
    let slope = if sxx != 0.0 {
        (sxy / sxx) / m.max(1e-9)
    } else {
        0.0
    };
    let dir = if rel > cfg.dead {
        1
    } else if rel < -cfg.dead {
        -1
    } else {
        0
    };
    VolTrend {
        dir,
        slope,
        rel: (rel / cfg.scale).clamp(-1.0, 1.0),
    }
}

// ---------------------------------------------------------------------------
// Trend-movement context + classification
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OiContext {
    pub res: f64,
    pub sup: f64,
    pub net: f64,
    #[serde(rename = "box")]
    pub box_oi: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub spot: f64,
    pub agreement: f64,
    pub method_n: usize,
    pub vol: VolTrend,
    pub pcr: Option<f64>,
    pub pcr_chg: Option<f64>,
    pub pcr_dir: f64,
    pub pcr_level: f64,
    pub walls: Vec<Wall>,
    pub oi: Option<OiContext>,
}

#[derive(Clone, Copy, Debug)]
pub struct ContextOpts {
    pub pcr_dead: f64,
    pub pcr_scale: f64,
    pub pcr_hi: f64,
    pub pcr_lo: f64,
    pub vol: VolTrendOpts,
}

impl Default for ContextOpts {
    fn default() -> Self {
        ContextOpts {
            pcr_dead: 0.02,
            pcr_scale: 0.08,
            pcr_hi: 1.3,
            pcr_lo: 0.75,
            vol: VolTrendOpts::default(),
        }
    }
}

/// Fold every price method + volume + PCR into one agreement context.
/// Mirrors `contextOf()`.
pub fn context_of(candles: &[Candle], level: &LevelData, cfg: &ContextOpts) -> Context {
    let n = candles.len();
    let li = n.saturating_sub(1);
    let spot = if let Some(s) = level.spot {
        if s > 0.0 {
            s
        } else if n > 0 {
            candles[li].close
        } else {
            0.0
        }
    } else if n > 0 {
        candles[li].close
    } else {
        0.0
    };

    let mut ctx = Context {
        spot,
        agreement: 0.0,
        method_n: 0,
        vol: VolTrend::default(),
        pcr: level.pcr,
        pcr_chg: level.pcr_chg,
        pcr_dir: 0.0,
        pcr_level: 0.0,
        walls: level.walls.clone(),
        oi: if !level.walls.is_empty() || level.net_oi != 0.0 || level.box_oi != 0.0 {
            Some(OiContext {
                res: level.res_overhead,
                sup: level.sup_under,
                net: level.net_oi,
                box_oi: level.box_oi,
            })
        } else {
            None
        },
    };
    if n < 5 {
        return ctx;
    }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let vols: Vec<f64> = candles.iter().map(|c| c.volume).collect();

    // Price methods: EMA ladder + close-vs-EMA21 + two Supertrends.
    let mut agg = 0i32;
    let mut m_n = 0usize;
    let mut vote = |ok: bool, bull: bool, agg: &mut i32, m_n: &mut usize| {
        if !ok {
            return;
        }
        *agg += if bull { 1 } else { -1 };
        *m_n += 1;
    };
    let need = |len: usize| n >= len;
    let ema = |len: usize| ema_first(&closes, len);
    let e9 = need(9).then(|| ema(9));
    let e21 = need(21).then(|| ema(21));
    let e35 = need(35).then(|| ema(35));
    let e50 = need(50).then(|| ema(50));
    let e100 = need(100).then(|| ema(100));
    let e200 = need(200).then(|| ema(200));

    vote(
        e9.is_some() && e21.is_some(),
        e9.as_ref().unwrap()[li] > e21.as_ref().unwrap()[li],
        &mut agg,
        &mut m_n,
    );
    vote(
        e21.is_some() && e35.is_some(),
        e21.as_ref().unwrap()[li] > e35.as_ref().unwrap()[li],
        &mut agg,
        &mut m_n,
    );
    vote(
        e35.is_some() && e50.is_some(),
        e35.as_ref().unwrap()[li] > e50.as_ref().unwrap()[li],
        &mut agg,
        &mut m_n,
    );
    vote(
        e50.is_some() && e100.is_some(),
        e50.as_ref().unwrap()[li] > e100.as_ref().unwrap()[li],
        &mut agg,
        &mut m_n,
    );
    vote(
        e100.is_some() && e200.is_some(),
        e100.as_ref().unwrap()[li] > e200.as_ref().unwrap()[li],
        &mut agg,
        &mut m_n,
    );
    if need(22) {
        vote(
            true,
            candles[li].close > e21.as_ref().unwrap()[li],
            &mut agg,
            &mut m_n,
        );
        let st1 = st_series(candles, 10, 1.0);
        let st2 = st_series(candles, 10, 2.0);
        vote(
            true,
            st1[li].dir == 1 && candles[li].close >= st1[li].band,
            &mut agg,
            &mut m_n,
        );
        vote(true, st2[li].dir == 1, &mut agg, &mut m_n);
    }
    ctx.method_n = m_n;
    ctx.agreement = if m_n > 0 {
        agg as f64 / m_n as f64
    } else {
        0.0
    };

    ctx.vol = vol_trend_of(&vols, &cfg.vol);

    if let (Some(pcr), Some(pcr_chg)) = (ctx.pcr, ctx.pcr_chg) {
        if pcr > 0.0 && pcr_chg.is_finite() {
            let rel = (pcr_chg / pcr).abs();
            if rel >= cfg.pcr_dead {
                ctx.pcr_dir = -((pcr_chg / pcr) / cfg.pcr_scale).clamp(-1.0, 1.0);
            }
        }
    }
    if let Some(pcr) = ctx.pcr {
        ctx.pcr_level = if pcr <= cfg.pcr_lo {
            1.0
        } else if pcr >= cfg.pcr_hi {
            -1.0
        } else {
            0.0
        };
    }
    ctx
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResult {
    pub kind: String,
    pub arrow: Option<String>,
    pub label: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vol: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oi_dir: Option<f64>,
    pub net: f64,
    pub box_oi: f64,
    pub info: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ClassifyOpts {
    pub near_pct: f64,
    pub oi_box_floor: f64,
    pub oi_bal_max: f64,
    pub cons_score_t: f64,
    pub oi_break_th: f64,
    pub w_agr: f64,
    pub w_vol: f64,
    pub w_pcr: f64,
    pub w_oi: f64,
    pub strong_t: f64,
    pub weak_t: f64,
}

impl Default for ClassifyOpts {
    fn default() -> Self {
        ClassifyOpts {
            near_pct: 0.006,
            oi_box_floor: 0.30,
            oi_bal_max: 0.45,
            cons_score_t: 0.22,
            oi_break_th: 0.5,
            w_agr: 0.40,
            w_vol: 0.25,
            w_pcr: 0.20,
            w_oi: 0.15,
            strong_t: 0.18,
            weak_t: -0.15,
        }
    }
}

fn info_text(ctx: &Context, agr: Option<f64>, score: Option<f64>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pcr) = ctx.pcr {
        let mut p = format!("PCR {:.2}", pcr);
        if let Some(chg) = ctx.pcr_chg {
            if chg.is_finite() {
                p += &format!(" chg {}{:.2}", if chg < 0.0 { "" } else { "+" }, chg);
            }
        }
        parts.push(p);
    }
    parts.push(format!(
        "Vol {}",
        match ctx.vol.dir {
            1 => "rising",
            -1 => "falling",
            _ => "flat",
        }
    ));
    if let Some(a) = agr {
        parts.push(format!("Price {}{:.2}", if a > 0.0 { "+" } else { "" }, a));
    }
    if let Some(s) = score {
        parts.push(format!("Score {}{:.2}", if s > 0.0 { "+" } else { "" }, s));
    }
    if let Some(oi) = &ctx.oi {
        if oi.net.abs() >= 0.05 || oi.box_oi >= 0.2 {
            let f = |v: f64| if v >= 0.99 { "1".to_string() } else { format!("{:.2}", v) };
            parts.push(format!("OI sup {}/res {}", f(oi.sup), f(oi.res)));
        }
    }
    parts.join(" | ")
}

/// Fuse regime + context into the arrow / label / color. Mirrors `classify()`.
pub fn classify(last: &RegimeLast, ctx: &Context, cfg: &ClassifyOpts) -> ClassifyResult {
    let rev_color = "#ff9100";
    let oi = ctx.oi.clone().unwrap_or_default();
    let net = oi.net;
    let box_oi = oi.box_oi;
    let squeeze = box_oi >= cfg.oi_box_floor && net.abs() <= cfg.oi_bal_max;

    let plain_cons = ClassifyResult {
        kind: "consolidation".into(),
        arrow: None,
        label: "Consolidation Liquidity Grabbing Phase".into(),
        color: "#ffc107".into(),
        strength: None,
        score: None,
        agr: None,
        vol: None,
        pcr: None,
        oi_dir: None,
        net,
        box_oi: box_oi,
        info: info_text(ctx, None, None),
    };
    let pin_cons = ClassifyResult {
        kind: "consolidation".into(),
        arrow: None,
        label: "Consolidation · OI squeeze (CE & PE walls both sides)".into(),
        color: "#ffc107".into(),
        ..plain_cons.clone()
    };
    let bias = |up: bool| ClassifyResult {
        kind: "continue".into(),
        arrow: Some(if up { "up" } else { "down" }.into()),
        label: if up {
            "OI Bias UP (put OI heavy below)".into()
        } else {
            "OI Bias DOWN (call OI heavy above)".into()
        },
        color: if up { "#26c6da".into() } else { "#ff7043".into() },
        strength: Some("weak".into()),
        score: None,
        agr: None,
        vol: None,
        pcr: None,
        oi_dir: None,
        net,
        box_oi: box_oi,
        info: info_text(ctx, None, None),
    };

    if last.regime == Regime::Flat {
        if !squeeze && net >= cfg.oi_break_th {
            return bias(true);
        }
        if !squeeze && net <= -cfg.oi_break_th {
            return bias(false);
        }
        return if squeeze { pin_cons } else { plain_cons };
    }

    let up = last.regime == Regime::Up;
    let reg_sign = if up { 1.0 } else { -1.0 };
    let spot = if ctx.spot > 0.0 {
        ctx.spot
    } else {
        last.close
    };

    let agr = ctx.agreement * reg_sign;
    let vol = ctx.vol.dir as f64 * reg_sign;
    let mut pcr = 0.0;
    pcr += ctx.pcr_dir * reg_sign;
    let oi_dir = net * reg_sign;
    let score = cfg.w_agr * agr + cfg.w_vol * vol + cfg.w_pcr * pcr + cfg.w_oi * oi_dir;
    let strong = score >= cfg.strong_t;
    let weak = score <= cfg.weak_t;
    let dir_color = if up { "#00e676" } else { "#ff5252" };
    let dim_color = if up {
        "rgba(0,230,118,0.55)"
    } else {
        "rgba(255,82,82,0.55)"
    };

    // Reversal near a fresh/heavy wall on the trend's side.
    if spot > 0.0 && !ctx.walls.is_empty() {
        let side = if up { "res" } else { "sup" };
        let hit = ctx.walls.iter().find(|w| {
            w.kind == side
                && (if up { w.strike > spot } else { w.strike < spot })
                && (w.strike - spot).abs() / spot <= cfg.near_pct * 2.5
        });
        if let Some(hit) = hit {
            let wall_arg = hit.chg > 0.0;
            let extreme = if up {
                ctx.pcr_level == 1.0
            } else {
                ctx.pcr_level == -1.0
            };
            let pcr_trigger = if up {
                ctx.pcr_dir < -0.4
            } else {
                ctx.pcr_dir > 0.4
            };
            if wall_arg || extreme || score <= cfg.weak_t || pcr_trigger {
                let strike_txt = (hit.strike * 100.0).round() / 100.0;
                let label = format!("Reversal @ {}", strike_txt);
                return ClassifyResult {
                    kind: "reversal".into(),
                    arrow: Some(if up { "down" } else { "up" }.into()),
                    label,
                    color: rev_color.into(),
                    strength: None,
                    score: Some(score),
                    agr: Some(agr),
                    vol: Some(vol),
                    pcr: Some(pcr),
                    oi_dir: Some(oi_dir),
                    net,
                    box_oi: box_oi,
                    info: info_text(ctx, Some(agr), Some(score)),
                };
            }
        }
    }

    if squeeze && score.abs() < cfg.cons_score_t {
        return pin_cons;
    }

    let label = if strong {
        "Trend Continue"
    } else if weak {
        "Trend Continue (weak)"
    } else {
        "Trend Continue"
    };
    let color = if strong {
        dir_color.to_string()
    } else if weak {
        "rgba(255,255,255,0.35)".to_string()
    } else {
        dim_color.to_string()
    };
    ClassifyResult {
        kind: "continue".into(),
        arrow: Some(if up { "up" } else { "down" }.into()),
        label: label.into(),
        color,
        strength: Some(if strong {
            "strong".into()
        } else if weak {
            "weak".into()
        } else {
            "normal".into()
        }),
        score: Some(score),
        agr: Some(agr),
        vol: Some(vol),
        pcr: Some(pcr),
        oi_dir: Some(oi_dir),
        net,
        box_oi: box_oi,
        info: info_text(ctx, Some(agr), Some(score)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn c(time: i64, o: f64, h: f64, l: f64, cl: f64, v: f64) -> Candle {
        Candle {
            time,
            open: o,
            high: h,
            low: l,
            close: cl,
            volume: v,
        }
    }

    #[test]
    fn ema_first_seeds_on_first_value() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let e = ema_first(&v, 3); // k = 0.5
        assert_eq!(e[0], 1.0);
        assert_eq!(e[1], 1.0 * 0.5 + 2.0 * 0.5);
        let expected2 = 3.0 * 0.5 + e[1] * 0.5;
        assert!((e[2] - expected2).abs() < 1e-12);
    }

    #[test]
    fn atr_seeds_on_first_true_range() {
        let cs = vec![
            c(0, 10.0, 12.0, 9.0, 11.0, 0.0),
            c(1, 11.0, 13.0, 10.5, 12.0, 0.0),
        ];
        let a = atr_first(&cs, 14);
        assert_eq!(a[0], 3.0); // 12 - 9
        let tr = 2.5f64; // max(2.5, |13-11|=2, |10.5-11|=0.5)
        let expected = (3.0 * 13.0 + tr) / 14.0;
        assert!((a[1] - expected).abs() < 1e-12);
    }

    #[test]
    fn regime_line_tracks_uptrend() {
        let mut cs = Vec::new();
        for i in 0..60 {
            let price = 100.0 + i as f64;
            cs.push(c(i, price, price + 1.0, price - 1.0, price, 1000.0 + i as f64 * 10.0));
        }
        let r = regime_line(&cs, &RegimeOpts::default());
        assert_eq!(r.regs[59], Regime::Up);
        // trending line follows the fast EMA (rounded to 2dp)
        let li = 59;
        assert!((r.data[li].value - (r.last.ema_f * 100.0).round() / 100.0).abs() < 1e-9);
    }

    #[test]
    fn regime_line_draws_straight_segments() {
        // A zig-zag so both trending and flat runs appear.
        let mut cs = Vec::new();
        let mut price = 100.0;
        for i in 0..140 {
            let drift = if (i / 20) % 2 == 0 { 0.6 } else { -0.35 };
            price += drift;
            cs.push(c(i, price, price + 1.0, price - 1.0, price, 1000.0 + i as f64 * 10.0));
        }
        let r = regime_line(&cs, &RegimeOpts::default());
        let n = r.data.len();
        let mut i = 0usize;
        while i < n {
            let reg = r.regs[i];
            let mut j = i;
            while j + 1 < n && r.regs[j + 1] == reg {
                j += 1;
            }
            let t = |k: usize| r.data[k].time as f64;
            let v = |k: usize| r.data[k].value;
            if j > i + 1 {
                // Every run is ONE straight line (collinear), including flat
                // runs whose fast EMA still drifted.
                let s0 = (v(i + 1) - v(i)) / (t(i + 1) - t(i));
                for k in (i + 1)..j {
                    let s = (v(k + 1) - v(k)) / (t(k + 1) - t(k));
                    assert!((s - s0).abs() < 0.02, "run not straight at {k}: {s} vs {s0}");
                }
            }
            // A flat run must not be forced horizontal when its value moved.
            if reg == Regime::Flat && j > i && (v(j) - v(i)).abs() > 1.0 {
                assert!((v(i + 1) - v(i)).abs() > 0.0, "drifting flat run is horizontal");
            }
            i = j + 1;
        }
    }

    #[test]
    fn level_data_pcr_maxpain_walls() {
        let recs = vec![
            OiRecord { strike: 25000.0, ce_oi: 100.0, pe_oi: 500.0, ce_chg: 10.0, pe_chg: 50.0, ..Default::default() },
            OiRecord { strike: 25100.0, ce_oi: 400.0, pe_oi: 300.0, ce_chg: 40.0, pe_chg: 20.0, ..Default::default() },
            OiRecord { strike: 25200.0, ce_oi: 900.0, pe_oi: 100.0, ce_chg: 90.0, pe_chg: 5.0, ..Default::default() },
        ];
        let l = level_data(&recs, 25100.0, &LevelOpts::default());
        assert_eq!(l.rows, 3);
        assert!((l.pcr.unwrap() - 900.0 / 1400.0).abs() < 1e-12);
        // call wall above spot is 25200, put wall below is 25000
        assert_eq!(l.walls[0].kind, "res");
        assert_eq!(l.walls[0].strike, 25200.0);
        assert!(l.max_pain.is_some());
    }

    #[test]
    fn oi_rows_sorted_and_filtered() {
        let recs = vec![
            OiRecord { strike: 30000.0, ce_oi: 1.0, pe_oi: 1.0, ..Default::default() },
            OiRecord { strike: 25000.0, ce_oi: 10.0, pe_oi: 20.0, ..Default::default() },
            OiRecord { strike: 25100.0, ce_oi: 10.0, pe_oi: 0.0, ..Default::default() },
        ];
        let rows = oi_rows(&recs, 25000.0, 0.12);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].strike, 25000.0);
        assert_eq!(rows[1].strike, 25100.0);
    }

    #[test]
    fn classify_flat_strong_oi_bias() {
        let last = RegimeLast {
            regime: Regime::Flat,
            ema_f: 100.0,
            ema_s: 100.0,
            atr: 1.0,
            vol: 100.0,
            vol_avg: 100.0,
            close: 100.0,
        };
        let ctx = Context {
            spot: 100.0,
            agreement: 0.0,
            method_n: 0,
            vol: VolTrend::default(),
            pcr: Some(1.0),
            pcr_chg: Some(0.0),
            pcr_dir: 0.0,
            pcr_level: 0.0,
            walls: vec![],
            oi: Some(OiContext { res: 0.1, sup: 0.9, net: 0.8, box_oi: 0.1 }),
        };
        let st = classify(&last, &ctx, &ClassifyOpts::default());
        assert_eq!(st.kind, "continue");
        assert_eq!(st.arrow.as_deref(), Some("up"));
    }

    #[test]
    fn classify_reversal_at_fresh_call_wall() {
        let last = RegimeLast {
            regime: Regime::Up,
            ema_f: 100.5,
            ema_s: 100.0,
            atr: 1.0,
            vol: 100.0,
            vol_avg: 100.0,
            close: 100.5,
        };
        let wall = Wall {
            kind: "res".into(),
            strike: 101.0,
            oi: 500.0,
            chg: 50.0,
            vol: 10.0,
            fresh: true,
        };
        let ctx = Context {
            spot: 100.5,
            agreement: 0.0,
            method_n: 0,
            vol: VolTrend::default(),
            pcr: Some(1.0),
            pcr_chg: Some(0.0),
            pcr_dir: 0.0,
            pcr_level: 0.0,
            walls: vec![wall],
            oi: Some(OiContext { res: 0.5, sup: 0.5, net: 0.0, box_oi: 0.5 }),
        };
        let st = classify(&last, &ctx, &ClassifyOpts::default());
        assert_eq!(st.kind, "reversal");
        assert_eq!(st.arrow.as_deref(), Some("down"));
        assert!(st.label.starts_with("Reversal @ 101"));
    }

    #[test]
    fn vol_trend_rising_vs_falling() {
        let mut rising = vec![100.0; 40];
        for i in 0..6 {
            rising[34 + i] = 100.0 + (i as f64 + 1.0) * 50.0;
        }
        assert_eq!(vol_trend_of(&rising, &VolTrendOpts::default()).dir, 1);

        let mut falling = vec![100.0; 40];
        for i in 0..6 {
            falling[34 + i] = 100.0 - (i as f64 + 1.0) * 10.0;
        }
        assert_eq!(vol_trend_of(&falling, &VolTrendOpts::default()).dir, -1);
    }
}
