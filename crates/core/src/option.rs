//! Option-chain numerics and the canonical chain-row model shared by the server
//! (which fills it from Dhan or from the synthetic generator) and the WASM
//! option-chain tab (which renders it).
//!
//! The Black-Scholes helpers mirror the old Python app's `_bs_*` routines
//! (`_BS_RATE = 0.06`, bisection IV over `[0.001, 4.0]`, expiry valued at
//! 15:30 IST) so greeks and implied vols read identically after the port.

use serde_json::json;

pub const BS_RATE: f64 = 0.06;

// ---------------------------------------------------------------------------
// Normal distribution
// ---------------------------------------------------------------------------

/// Abramowitz-Stegun 7.1.26 normal CDF (same approximation the old app used).
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

fn d1_d2(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> Option<(f64, f64)> {
    if !(s > 0.0 && k > 0.0 && t > 0.0 && sigma > 0.0) {
        return None;
    }
    let root = sigma * t.sqrt();
    let d1 = ((s / k).ln() + (r + 0.5 * sigma * sigma) * t) / root;
    Some((d1, d1 - root))
}

/// Standard Black-Scholes price. Returns 0 on invalid inputs.
pub fn bs_price(s: f64, k: f64, t: f64, r: f64, sigma: f64, is_call: bool) -> f64 {
    let Some((d1, d2)) = d1_d2(s, k, t, r, sigma) else {
        return 0.0;
    };
    let disc = (-r * t).exp();
    if is_call {
        s * norm_cdf(d1) - k * disc * norm_cdf(d2)
    } else {
        k * disc * norm_cdf(-d2) - s * norm_cdf(-d1)
    }
}

/// Bisection implied vol over `[0.001, 4.0]`, 50 iterations. `None` when the
/// market price is below intrinsic or outside the bracket (old app behaviour).
pub fn implied_vol(s: f64, k: f64, t: f64, r: f64, price: f64, is_call: bool) -> Option<f64> {
    if !(s > 0.0 && k > 0.0 && t > 0.0 && price > 0.0) {
        return None;
    }
    let intrinsic = if is_call {
        (s - k).max(0.0)
    } else {
        (k - s).max(0.0)
    };
    if price < intrinsic {
        return None;
    }
    if bs_price(s, k, t, r, 0.001, is_call) > price {
        return None;
    }
    let (mut lo, mut hi) = (0.001f64, 4.0f64);
    for _ in 0..50 {
        let mid = 0.5 * (lo + hi);
        let p = bs_price(s, k, t, r, mid, is_call);
        if p > price {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some((0.5 * (lo + hi) * 10000.0).round() / 10000.0)
}

pub fn bs_delta(s: f64, k: f64, t: f64, r: f64, sigma: f64, is_call: bool) -> f64 {
    let Some((d1, _)) = d1_d2(s, k, t, r, sigma) else {
        return 0.0;
    };
    if is_call {
        norm_cdf(d1)
    } else {
        norm_cdf(d1) - 1.0
    }
}

pub fn bs_vega(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    let Some((d1, _)) = d1_d2(s, k, t, r, sigma) else {
        return 0.0;
    };
    s * norm_pdf(d1) * t.sqrt()
}

pub fn bs_gamma(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    let Some((d1, _)) = d1_d2(s, k, t, r, sigma) else {
        return 0.0;
    };
    norm_pdf(d1) / (s * sigma * t.sqrt())
}

/// Per-day theta (old app reports Dhan's REST theta; this is the analytic one).
pub fn bs_theta(s: f64, k: f64, t: f64, r: f64, sigma: f64, is_call: bool) -> f64 {
    let Some((d1, d2)) = d1_d2(s, k, t, r, sigma) else {
        return 0.0;
    };
    let first = -(s * norm_pdf(d1) * sigma) / (2.0 * t.sqrt());
    let disc = (-r * t).exp();
    if is_call {
        first - r * k * disc * norm_cdf(d2)
    } else {
        first + r * k * disc * norm_cdf(-d2)
    }
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

/// Days since the Unix epoch for a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`].
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn format_ymd(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Parse `YYYY-MM-DD` into days since the epoch.
pub fn parse_ymd(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let y: i64 = it.next()?.trim().parse().ok()?;
    let m: i64 = it.next()?.trim().parse().ok()?;
    let d: i64 = it.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Years to expiry, valuing the contract at 15:30 IST on the expiry date.
pub fn ttm_years(expiry: &str, now_secs: i64) -> f64 {
    let days = parse_ymd(expiry).unwrap_or_else(|| now_secs / 86400);
    let exp_epoch = days * 86400 + 55800; // 15:30 IST == UTC 10:00
    let now_ist = now_secs + 19800;
    let t = (exp_epoch - now_ist) as f64 / (365.25 * 86400.0);
    t.max(0.001)
}

// ---------------------------------------------------------------------------
// Chain model
// ---------------------------------------------------------------------------

/// One side (CE or PE) of a strike.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ChainLeg {
    pub sid: i64,
    pub ltp: f64,
    pub chg: f64,
    pub chg_pct: f64,
    pub oi: f64,
    pub chg_oi: f64,
    pub vol: f64,
    pub iv: f64,
    pub bid: f64,
    pub ask: f64,
    pub delta: f64,
    pub theta: f64,
    pub gamma: f64,
    pub vega: f64,
}

/// One strike row of the option chain.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ChainRow {
    pub strike: f64,
    pub ce: ChainLeg,
    pub pe: ChainLeg,
}

fn num(v: f64) -> serde_json::Value {
    if v.is_finite() {
        json!(v)
    } else {
        serde_json::Value::Null
    }
}

fn round(v: f64, dp: i32) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    let f = 10f64.powi(dp);
    (v * f).round() / f
}

impl ChainLeg {
    fn into_map(&self, side: &str) -> Vec<(String, serde_json::Value)> {
        vec![
            (format!("{side} SID"), json!(self.sid)),
            (format!("{side} LTP"), num(round(self.ltp, 2))),
            (format!("{side} Chg"), num(round(self.chg, 2))),
            (format!("{side} Chg%"), num(round(self.chg_pct, 2))),
            (format!("{side} OI"), num(round(self.oi, 0))),
            (format!("{side} Chg OI"), num(round(self.chg_oi, 0))),
            (format!("{side} Volume"), num(round(self.vol, 0))),
            (format!("{side} IV"), num(round(self.iv, 2))),
            (format!("{side} Bid"), num(round(self.bid, 2))),
            (format!("{side} Ask"), num(round(self.ask, 2))),
            (format!("{side} Delta"), num(round(self.delta, 4))),
            (format!("{side} Theta"), num(round(self.theta, 4))),
            (format!("{side} Gamma"), num(round(self.gamma, 4))),
            (format!("{side} Vega"), num(round(self.vega, 4))),
        ]
    }
}

impl ChainRow {
    /// Serialise with the old app's exact column keys ("CE LTP", "PE Chg", ...).
    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        m.insert("Strike".into(), num(round(self.strike, 2)));
        for (k, v) in self.ce.into_map("CE") {
            m.insert(k, v);
        }
        for (k, v) in self.pe.into_map("PE") {
            m.insert(k, v);
        }
        serde_json::Value::Object(m)
    }

    /// Normalised snapshot for the OI Trend / level engine.
    pub fn oi_record(&self) -> crate::oi_trend::OiRecord {
        crate::oi_trend::OiRecord {
            strike: self.strike,
            ce_oi: self.ce.oi,
            pe_oi: self.pe.oi,
            ce_chg: self.ce.chg_oi,
            pe_chg: self.pe.chg_oi,
            ce_vol: self.ce.vol,
            pe_vol: self.pe.vol,
            ce_ltp: self.ce.ltp,
            pe_ltp: self.pe.ltp,
            ce_iv: self.ce.iv,
            pe_iv: self.pe.iv,
        }
    }
}

/// Smallest positive gap between consecutive strikes (the chain's interval).
pub fn strike_interval(strikes: &[f64]) -> f64 {
    let mut best = f64::INFINITY;
    for w in strikes.windows(2) {
        let d = w[1] - w[0];
        if d > 0.0 && d < best {
            best = d;
        }
    }
    if best.is_finite() {
        best
    } else {
        50.0
    }
}

/// Index of the strike closest to `spot` (0 when the chain is empty).
pub fn atm_index(strikes: &[f64], spot: f64) -> usize {
    if strikes.is_empty() {
        return 0;
    }
    let mut best = 0usize;
    let mut best_d = f64::INFINITY;
    for (i, s) in strikes.iter().enumerate() {
        let d = (s - spot).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}
