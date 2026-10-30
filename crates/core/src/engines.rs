//! Shared structural-analysis engines used by the advanced indicator set.
//! These are fresh Rust implementations of the algorithms the chart needs
//! (swing structure, Elliott-wave zigzag, S/R clustering, pane consensus,
//! volume-liquidity trend, RSI divergence).

use std::collections::BTreeMap;

use crate::math::*;
use crate::model::*;

#[derive(Clone, Copy, Debug)]
pub struct Pivot {
    pub is_high: bool,
    pub price: f64,
    pub idx: usize,
    pub at: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ZigPivot {
    pub is_high: bool,
    pub price: f64,
    pub idx: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Level {
    pub price: f64,
    pub n: f64,
    pub first: usize,
    pub last: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct AutoLine {
    pub a: f64,
    pub b: f64,
    pub p1_idx: usize,
    pub p2_idx: usize,
    pub is_support: bool,
    pub score: f64,
}

fn rnd(v: f64) -> f64 {
    v.round()
}

fn pos_len(v: f64, def: usize) -> usize {
    let r = rnd(v) as i64;
    if r <= 0 {
        def
    } else {
        r as usize
    }
}

/// Ramer-Douglas-Peucker simplification used to render "straight" line variants.
pub fn straighten_line(data: &[Point], tol_frac: f64) -> Vec<Point> {
    if data.len() < 3 {
        return data.to_vec();
    }
    let pts: Vec<Point> = data.iter().filter(|p| p.value.is_finite()).cloned().collect();
    let m = pts.len();
    if m < 3 {
        return pts;
    }
    let mut vmin = f64::INFINITY;
    let mut vmax = f64::NEG_INFINITY;
    for p in &pts {
        if p.value < vmin {
            vmin = p.value;
        }
        if p.value > vmax {
            vmax = p.value;
        }
    }
    let vr = if vmax - vmin == 0.0 { 1.0 } else { vmax - vmin };
    let xr = if m > 1 { (m - 1) as f64 } else { 1.0 };
    let eps = if tol_frac > 0.0 { tol_frac } else { 0.08 };
    let mut keep = vec![false; m];
    keep[0] = true;
    keep[m - 1] = true;
    let mut stack = vec![(0usize, m - 1usize)];
    while let Some((a, b)) = stack.pop() {
        if b - a < 2 {
            continue;
        }
        let x1 = a as f64 / xr;
        let y1 = (pts[a].value - vmin) / vr;
        let x2 = b as f64 / xr;
        let y2 = (pts[b].value - vmin) / vr;
        let dx = x2 - x1;
        let dy = y2 - y1;
        let l = (dx * dx + dy * dy).sqrt();
        let mut maxd = -1.0f64;
        let mut idx = a + 1;
        for i in (a + 1)..b {
            let x = i as f64 / xr;
            let y = (pts[i].value - vmin) / vr;
            let d = if l > 1e-12 {
                ((dy * x - dx * y + x2 * y1 - y2 * x1) / l).abs()
            } else {
                let ddx = x - x1;
                let ddy = y - y1;
                (ddx * ddx + ddy * ddy).sqrt()
            };
            if d > maxd {
                maxd = d;
                idx = i;
            }
        }
        if maxd > eps {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    let mut out = Vec::new();
    for i in 0..m {
        if keep[i] {
            out.push(pts[i].clone());
        }
    }
    out
}

pub fn fractal_pivots(c: &[Candle], strength: f64) -> Vec<Pivot> {
    let s = strength.round() as i64;
    let s = if s <= 0 { 3usize } else { s as usize };
    let n = c.len();
    let mut out = Vec::new();
    if n < 2 * s + 1 {
        return out;
    }
    for i in s..(n - s) {
        let mut is_h = true;
        let mut is_l = true;
        for k in 1..=s {
            if !(c[i].high >= c[i - k].high && c[i].high >= c[i + k].high) {
                is_h = false;
            }
            if !(c[i].low <= c[i - k].low && c[i].low <= c[i + k].low) {
                is_l = false;
            }
            if !is_h && !is_l {
                break;
            }
        }
        if is_h {
            out.push(Pivot { is_high: true, price: c[i].high, idx: i, at: i + s });
        } else if is_l {
            out.push(Pivot { is_high: false, price: c[i].low, idx: i, at: i + s });
        }
    }
    out
}

pub fn cluster_levels(piv: &[Pivot], tol: f64) -> Vec<Level> {
    let mut arr: Vec<(f64, usize)> = piv.iter().map(|p| (p.price, p.idx)).collect();
    arr.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cl: Vec<Level> = Vec::new();
    for (price, idx) in arr {
        let mut placed = false;
        for g in cl.iter_mut() {
            if (price - g.price).abs() <= tol {
                g.price = (g.price * g.n + price) / (g.n + 1.0);
                g.n += 1.0;
                if idx < g.first {
                    g.first = idx;
                }
                if idx > g.last {
                    g.last = idx;
                }
                placed = true;
                break;
            }
        }
        if !placed {
            cl.push(Level { price, n: 1.0, first: idx, last: idx });
        }
    }
    cl
}

pub fn auto_trend_line(c: &[Candle], piv: &[Pivot], look: f64, tol: f64) -> Option<AutoLine> {
    let l0 = rnd(look);
    let l0 = if l0 == 0.0 { 12.0 } else { l0 };
    let look = l0.max(3.0) as usize;
    let lows: Vec<Pivot> = piv.iter().filter(|p| !p.is_high).copied().collect();
    let highs: Vec<Pivot> = piv.iter().filter(|p| p.is_high).copied().collect();
    let eval = |p1: &Pivot, p2: &Pivot, kind_support: bool| -> Option<AutoLine> {
        let di = p2.idx as i64 - p1.idx as i64;
        if di <= 0 {
            return None;
        }
        let a = (p2.price - p1.price) / di as f64;
        let b = p2.price - a * p2.idx as f64;
        let mut touches = 0.0f64;
        let mut viol = 0.0f64;
        for k in p1.idx..c.len() {
            let v = a * k as f64 + b;
            let in_range = v <= c[k].high + tol && v >= c[k].low - tol;
            if in_range {
                touches += 1.0;
            }
            if kind_support && c[k].close < v - tol {
                viol += 1.0;
            } else if !kind_support && c[k].close > v + tol {
                viol += 1.0;
            }
        }
        let span = di as f64;
        let score = touches * 2.0 - viol * 4.0 + span / (c.len().max(1) as f64);
        Some(AutoLine { a, b, p1_idx: p1.idx, p2_idx: p2.idx, is_support: kind_support, score })
    };
    let mut best: Option<AutoLine> = None;
    let lz: Vec<Pivot> = if look <= lows.len() { lows[lows.len() - look..].to_vec() } else { lows.clone() };
    let hz: Vec<Pivot> = if look <= highs.len() { highs[highs.len() - look..].to_vec() } else { highs.clone() };
    for i in 0..lz.len() {
        for j in (i + 1)..lz.len() {
            if !(lz[j].price > lz[i].price) {
                continue;
            }
            if let Some(sc) = eval(&lz[i], &lz[j], true) {
                if best.is_none() || sc.score > best.unwrap().score {
                    best = Some(sc);
                }
            }
        }
    }
    for i in 0..hz.len() {
        for j in (i + 1)..hz.len() {
            if !(hz[j].price < hz[i].price) {
                continue;
            }
            if let Some(sc) = eval(&hz[i], &hz[j], false) {
                if best.is_none() || sc.score > best.unwrap().score {
                    best = Some(sc);
                }
            }
        }
    }
    best
}

/// Best straight line through one side's swing pivots (support = lows,
/// resistance = highs). The line is scored on the pivots themselves: it is
/// rewarded for passing through (touching) as many same-side pivots as possible
/// and heavily penalised for cutting through one (a support low below the line /
/// a resistance high above it). A mild proximity term keeps it by the current
/// price, and a wrong-side line (support above price / resistance below) is
/// rejected, so the fit never drifts away from the chart.
pub fn side_trend_line(c: &[Candle], side: &[Pivot], look: f64, tol: f64, support: bool) -> Option<AutoLine> {
    let l0 = rnd(look);
    let l0 = if l0 == 0.0 { 12.0 } else { l0 };
    let look = l0.max(2.0) as usize;
    if side.len() < 2 {
        return None;
    }
    let z: Vec<Pivot> = if look <= side.len() { side[side.len() - look..].to_vec() } else { side.to_vec() };
    let last_idx = c.len() - 1;
    let last_close = c[last_idx].close;
    let mut best: Option<AutoLine> = None;
    for i in 0..z.len() {
        for j in (i + 1)..z.len() {
            let p1 = &z[i];
            let p2 = &z[j];
            let di = p2.idx as i64 - p1.idx as i64;
            if di <= 0 {
                continue;
            }
            let a = (p2.price - p1.price) / di as f64;
            let b = p2.price - a * p2.idx as f64;
            // Score on the swing pivots: touches = pivots the line passes
            // through, breaks = pivots it cuts through the wrong way.
            let mut touches = 0.0f64;
            let mut breaks = 0.0f64;
            for q in &z {
                if q.idx < p1.idx {
                    continue;
                }
                let v = a * q.idx as f64 + b;
                if (q.price - v).abs() <= tol {
                    touches += 1.0;
                } else if support && q.price < v - tol {
                    breaks += 1.0;
                } else if !support && q.price > v + tol {
                    breaks += 1.0;
                }
            }
            let span = di as f64;
            let last_v = a * last_idx as f64 + b;
            let prox = if tol > 0.0 { (last_v - last_close).abs() / tol } else { (last_v - last_close).abs() };
            let wrong_side = if support { last_v > last_close + tol } else { last_v < last_close - tol };
            let score = touches * 3.0 - breaks * 6.0 + span / (c.len().max(1) as f64)
                - prox * 0.5
                - if wrong_side { 100.0 } else { 0.0 };
            if best.is_none() || score > best.unwrap().score {
                best = Some(AutoLine { a, b, p1_idx: p1.idx, p2_idx: p2.idx, is_support: support, score });
            }
        }
    }
    best
}

pub fn last_alternating_pivots(piv: &[Pivot], count: usize) -> Option<Vec<Pivot>> {
    if piv.len() < count || count == 0 {
        return None;
    }
    let mut start = piv.len() as i64 - count as i64;
    while start >= 0 {
        let s = start as usize;
        let mut ok = true;
        for k in (s + 1)..(s + count) {
            if piv[k].is_high == piv[k - 1].is_high || piv[k].idx <= piv[k - 1].idx {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(piv[s..s + count].to_vec());
        }
        start -= 1;
    }
    None
}

/// ATR-scaled ZigZag (no confirmation `at` bar). Used by projection / S-D / rails.
pub fn atr_zigzag(c: &[Candle], atr_period: usize, atr_mult: f64, min_pct: f64) -> Vec<ZigPivot> {
    let n = c.len();
    let mut piv = Vec::new();
    if n == 0 {
        return piv;
    }
    let atr = wilder_arr(&tr_arr(c), atr_period as i64);
    let th = |i: usize, refp: f64| -> f64 {
        let a = match atr[i] {
            Some(v) if v.is_finite() => v * atr_mult,
            _ => 0.0,
        };
        a.max(refp.abs() * (min_pct / 100.0))
    };
    let mut dir = 1i32;
    let mut ext = c[0].high;
    let mut ext_idx = 0usize;
    for i in 1..n {
        let t = th(i, c[i].close);
        if dir >= 0 {
            if c[i].high > ext {
                ext = c[i].high;
                ext_idx = i;
            }
            if c[i].low <= ext - t {
                piv.push(ZigPivot { is_high: true, price: ext, idx: ext_idx });
                dir = -1;
                ext = c[i].low;
                ext_idx = i;
            }
        } else {
            if c[i].low < ext {
                ext = c[i].low;
                ext_idx = i;
            }
            if c[i].high >= ext + t {
                piv.push(ZigPivot { is_high: false, price: ext, idx: ext_idx });
                dir = 1;
                ext = c[i].high;
                ext_idx = i;
            }
        }
    }
    piv
}

// ---------------------------------------------------------------------------
// Elliott wave engine
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct EwPivot {
    pub is_high: bool,
    pub price: f64,
    pub idx: usize,
    pub at: usize,
}

impl From<&EwPivot> for Pivot {
    fn from(p: &EwPivot) -> Self {
        Pivot { is_high: p.is_high, price: p.price, idx: p.idx, at: p.at }
    }
}

#[derive(Clone, Debug)]
pub struct EwLabel {
    pub idx: usize,
    pub is_high: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct EwAnalysis {
    pub c: Vec<Candle>,
    pub piv: Vec<EwPivot>,
    pub trend_state: Vec<i32>,
    pub labels: Vec<EwLabel>,
    pub fib: Option<(EwPivot, EwPivot)>,
}

pub fn ew_analyze(candles: &[Candle], atr_period: f64, atr_mult: f64, min_pct: f64) -> EwAnalysis {
    let mut cc: Vec<Candle> = Vec::new();
    let mut prev_t: i64 = 0;
    for x in candles {
        if x.time > prev_t
            && x.time as f64 != 0.0
            && x.high.is_finite()
            && x.low.is_finite()
            && x.close.is_finite()
        {
            cc.push(*x);
            prev_t = x.time;
        }
    }
    let n = cc.len();
    let mut out = EwAnalysis { c: cc, ..Default::default() };
    if n < 3 {
        return out;
    }
    let atr_per = if atr_period.round() < 2.0 { 14.0 } else { atr_period.round() };
    let atr_per = atr_per as i64;
    let atr_mult = if atr_mult > 0.0 { atr_mult } else { 2.0 };
    let min_pct = if min_pct >= 0.0 { min_pct } else { 0.15 };
    let atr = wilder_arr(&tr_arr(&out.c), atr_per);
    let th = |i: usize, refp: f64| -> f64 {
        let a = match atr[i] {
            Some(v) if v.is_finite() => v * atr_mult,
            _ => 0.0,
        };
        a.max(refp.abs() * (min_pct / 100.0))
    };
    let mut piv: Vec<EwPivot> = Vec::new();
    let mut dir = 1i32;
    let mut ext = out.c[0].high;
    let mut ext_idx = 0usize;
    for i in 1..n {
        let t = th(i, out.c[i].close);
        if dir >= 0 {
            if out.c[i].high > ext {
                ext = out.c[i].high;
                ext_idx = i;
            }
            if out.c[i].low <= ext - t {
                piv.push(EwPivot { is_high: true, price: ext, idx: ext_idx, at: i });
                dir = -1;
                ext = out.c[i].low;
                ext_idx = i;
            }
        } else {
            if out.c[i].low < ext {
                ext = out.c[i].low;
                ext_idx = i;
            }
            if out.c[i].high >= ext + t {
                piv.push(EwPivot { is_high: false, price: ext, idx: ext_idx, at: i });
                dir = 1;
                ext = out.c[i].high;
                ext_idx = i;
            }
        }
    }
    out.piv = piv;
    if out.piv.len() < 2 {
        return out;
    }
    let mut last_h: Option<f64> = None;
    let mut prev_h: Option<f64> = None;
    let mut last_l: Option<f64> = None;
    let mut prev_l: Option<f64> = None;
    let mut trend = 0i32;
    let mut pi = 0usize;
    let mut trend_state = vec![0i32; n];
    for i in 0..n {
        while pi < out.piv.len() && out.piv[pi].at <= i {
            let p = &out.piv[pi];
            if p.is_high {
                prev_h = last_h;
                last_h = Some(p.price);
            } else {
                prev_l = last_l;
                last_l = Some(p.price);
            }
            if let (Some(lh), Some(ph), Some(ll), Some(pl)) = (last_h, prev_h, last_l, prev_l) {
                if lh > ph && ll > pl {
                    trend = 1;
                } else if lh < ph && ll < pl {
                    trend = -1;
                }
            }
            pi += 1;
        }
        trend_state[i] = trend;
    }
    out.trend_state = trend_state;
    // Wave labels: most recent clean 5-wave impulse, else A-B-C.
    let alt = |a: &[EwPivot]| -> bool {
        for k in 1..a.len() {
            if a[k].is_high == a[k - 1].is_high {
                return false;
            }
        }
        true
    };
    let plen = out.piv.len() as i64;
    let min_start = (plen - 12).max(0);
    let mut imp: Option<Vec<EwPivot>> = None;
    let mut start = plen - 6;
    while start >= min_start {
        if start >= 0 {
            let s = start as usize;
            if s + 6 <= out.piv.len() {
                let w: Vec<EwPivot> = out.piv[s..s + 6].to_vec();
                if alt(&w) {
                    let up_w = !w[0].is_high && w[5].is_high;
                    let dn_w = w[0].is_high && !w[5].is_high;
                    if (trend == 1 && up_w) || (trend == -1 && dn_w) || (trend == 0 && (up_w || dn_w)) {
                        imp = Some(w);
                        break;
                    }
                }
            }
        }
        start -= 1;
    }
    let mut labels: Vec<EwLabel> = Vec::new();
    if let Some(w) = &imp {
        let names = ["1", "2", "3", "4", "5"];
        for k in 1..=5 {
            labels.push(EwLabel { idx: w[k].idx, is_high: w[k].is_high, text: names[k - 1].to_string() });
        }
    } else if out.piv.len() >= 3 {
        let s3: Vec<EwPivot> = out.piv[out.piv.len() - 3..].to_vec();
        if alt(&s3) {
            let names = ["A", "B", "C"];
            for k in 0..3 {
                labels.push(EwLabel { idx: s3[k].idx, is_high: s3[k].is_high, text: names[k].to_string() });
            }
        }
    }
    let mut seen: BTreeMap<i64, bool> = BTreeMap::new();
    let mut filtered = Vec::new();
    for l in labels {
        if let Some(c) = out.c.get(l.idx) {
            if seen.insert(c.time, true).is_none() {
                filtered.push(l);
            }
        }
    }
    out.labels = filtered;
    // Last completed impulse leg for the Fib grid.
    let mut a_sel: Option<EwPivot> = None;
    let mut b_sel: Option<EwPivot> = None;
    let plen = out.piv.len();
    if plen >= 2 {
        let mut k = plen as i64 - 1;
        while k >= 1 {
            let b = &out.piv[k as usize];
            let a = &out.piv[(k - 1) as usize];
            if a.idx >= b.idx {
                k -= 1;
                continue;
            }
            let up2 = b.price > a.price;
            if (trend == 1 && up2 && !a.is_high) || (trend == -1 && !up2 && a.is_high) {
                a_sel = Some(a.clone());
                b_sel = Some(b.clone());
                break;
            }
            k -= 1;
        }
        if a_sel.is_none() {
            a_sel = Some(out.piv[plen - 2].clone());
            b_sel = Some(out.piv[plen - 1].clone());
        }
    }
    if let (Some(a), Some(b)) = (a_sel, b_sel) {
        if b.idx != a.idx && (b.price - a.price).abs() > 0.0 {
            out.fib = Some((a, b));
        }
    }
    out
}

/// Causal swing direction score for the fan overlays' hidden companions.
pub fn fan_swing_trend(candles: &[Candle], atr_period: f64, atr_mult: f64, min_pct: f64, color: &str) -> Vec<SeriesOut> {
    let a = ew_analyze(candles, atr_period, atr_mult, min_pct);
    let empty = vec![SeriesOut::line(color, 1.0)];
    if a.c.len() < 3 || a.piv.len() < 2 {
        return empty;
    }
    let mut dir = 0i32;
    let mut pi = 1usize;
    let mut score = 0i64;
    let mut data = Vec::new();
    for i in 0..a.c.len() {
        while pi < a.piv.len() && a.piv[pi].at <= i {
            dir = if a.piv[pi].price >= a.piv[pi - 1].price { 1 } else { -1 };
            pi += 1;
        }
        score += dir as i64;
        data.push(Point { time: a.c[i].time, value: score as f64, color: None });
    }
    let mut s = SeriesOut::line(color, 1.0);
    s.data = data;
    vec![s]
}

// ---------------------------------------------------------------------------
// Price Action Structure engine
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PaStruct {
    pub line: Vec<Point>,
    pub zig: Vec<Point>,
    pub markers: Vec<Marker>,
    pub trend_state: Vec<i32>,
}

pub fn pa_structure(
    c: &[Candle],
    pivot_len: f64,
    atr_len: f64,
    atr_mult: f64,
    show_markers: bool,
    markers_only: bool,
    up: &str,
    dn: &str,
) -> PaStruct {
    let n = c.len();
    let mut out = PaStruct { trend_state: vec![0; n], ..Default::default() };
    let pl0 = pivot_len.round() as i64;
    let pl = if pl0 < 1 { 3usize } else { pl0 as usize };
    if n < pl * 2 + 2 {
        return out;
    }
    let atr = wilder_arr(&tr_arr(c), (atr_len.round() as i64).max(2));
    let mult = if atr_mult >= 0.0 { atr_mult } else { 0.25 };
    let mut piv: Vec<Pivot> = Vec::new();
    for i in pl..(n - pl) {
        let hi = c[i].high;
        let lo = c[i].low;
        let mut is_h = true;
        let mut is_l = true;
        for j in (i - pl)..=(i + pl) {
            if j == i {
                continue;
            }
            if c[j].high >= hi {
                is_h = false;
            }
            if c[j].low <= lo {
                is_l = false;
            }
            if !is_h && !is_l {
                break;
            }
        }
        if is_h {
            piv.push(Pivot { is_high: true, price: hi, idx: i, at: i + pl });
        }
        if is_l {
            piv.push(Pivot { is_high: false, price: lo, idx: i, at: i + pl });
        }
    }
    piv.sort_by(|a, b| a.at.cmp(&b.at).then(a.idx.cmp(&b.idx)));
    let mut pi = 0usize;
    let mut last_h: Option<f64> = None;
    let mut prev_h: Option<f64> = None;
    let mut last_l: Option<f64> = None;
    let mut prev_l: Option<f64> = None;
    let mut trend = 0i32;
    let mut mk_map: BTreeMap<i64, Marker> = BTreeMap::new();
    for i in 0..n {
        while pi < piv.len() && piv[pi].at <= i {
            let p = piv[pi];
            pi += 1;
            if p.is_high {
                prev_h = last_h;
                last_h = Some(p.price);
            } else {
                prev_l = last_l;
                last_l = Some(p.price);
            }
            if show_markers {
                mk_map.entry(c[p.idx].time).or_insert(Marker {
                    time: c[p.idx].time,
                    position: if p.is_high { "aboveBar" } else { "belowBar" }.into(),
                    color: if p.is_high { dn } else { up }.into(),
                    shape: if p.is_high { "arrowDown" } else { "arrowUp" }.into(),
                    text: if p.is_high { "H" } else { "L" }.into(),
                    size: 1.0,
                });
            }
        }
        let close = c[i].close;
        let buf = match atr[i] {
            Some(a) if a.is_finite() => a * mult,
            _ => 0.0,
        };
        let prev_trend = trend;
        if let Some(lh) = last_h {
            if close > lh + buf {
                trend = 1;
            } else if let Some(ll) = last_l {
                if close < ll - buf {
                    trend = -1;
                } else {
                    trend = structure_flip(last_h, prev_h, last_l, prev_l, trend);
                }
            } else {
                trend = structure_flip(last_h, prev_h, last_l, prev_l, trend);
            }
        } else if let Some(ll) = last_l {
            if close < ll - buf {
                trend = -1;
            } else {
                trend = structure_flip(last_h, prev_h, last_l, prev_l, trend);
            }
        } else {
            trend = structure_flip(last_h, prev_h, last_l, prev_l, trend);
        }
        let mut lvl: Option<f64> = None;
        if trend == 1 {
            lvl = last_l.or(Some(close));
        } else if trend == -1 {
            lvl = last_h.or(Some(close));
        }
        if let Some(v) = lvl {
            if !markers_only {
                out.line.push(Point {
                    time: c[i].time,
                    value: v,
                    color: Some(if trend == 1 { up } else { dn }.to_string()),
                });
            }
        }
        if trend != prev_trend && trend != 0 && show_markers {
            mk_map.entry(c[i].time).or_insert(Marker {
                time: c[i].time,
                position: if trend == 1 { "belowBar" } else { "aboveBar" }.into(),
                color: if trend == 1 { up } else { dn }.into(),
                shape: if trend == 1 { "arrowUp" } else { "arrowDown" }.into(),
                text: if prev_trend == 0 { "BOS" } else { "CHoCH" }.into(),
                size: 1.0,
            });
        }
        out.trend_state[i] = trend;
    }
    if !markers_only {
        let mut p_idx: i64 = -1;
        for p in piv.iter().filter(|p| p.at <= n - 1) {
            if p.idx as i64 == p_idx {
                continue;
            }
            let is_up = out.zig.last().map(|z| p.price >= z.value).unwrap_or(true);
            out.zig.push(Point {
                time: c[p.idx].time,
                value: p.price,
                color: Some(if is_up { up } else { dn }.to_string()),
            });
            p_idx = p.idx as i64;
        }
        if let Some(last_z) = out.zig.last().cloned() {
            let last_c = c[n - 1];
            if last_c.time > last_z.time {
                let is_up = last_c.close >= last_z.value;
                out.zig.push(Point {
                    time: last_c.time,
                    value: last_c.close,
                    color: Some(if is_up { up } else { dn }.to_string()),
                });
            }
        }
    }
    out.markers = mk_map.into_values().collect();
    out
}

fn structure_flip(
    last_h: Option<f64>,
    prev_h: Option<f64>,
    last_l: Option<f64>,
    prev_l: Option<f64>,
    trend: i32,
) -> i32 {
    if let (Some(lh), Some(ph), Some(ll), Some(pl)) = (last_h, prev_h, last_l, prev_l) {
        if lh > ph && ll > pl {
            return 1;
        }
        if lh < ph && ll < pl {
            return -1;
        }
    }
    trend
}

// ---------------------------------------------------------------------------
// S/R + EMA reversal engine
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct SremaRes {
    pub c: Vec<Candle>,
    pub state: Vec<i32>,
    pub anchor_idx: Vec<i64>,
    pub anchor_price: Vec<Option<f64>>,
    pub line: Vec<Option<f64>>,
}

#[allow(clippy::too_many_arguments)]
pub fn srema_analyze(
    candles: &[Candle],
    atr_period: f64,
    atr_mult: f64,
    min_pct: f64,
    ema_period: f64,
    touch_mult: f64,
    touch_window: f64,
    fwd: f64,
) -> SremaRes {
    let a = ew_analyze(candles, atr_period, atr_mult, min_pct);
    let cc = a.c.clone();
    let piv = a.piv.clone();
    let n = cc.len();
    let mut res = SremaRes {
        c: cc.clone(),
        state: vec![0; n],
        anchor_idx: vec![-1; n],
        anchor_price: vec![None; n],
        line: vec![None; n],
    };
    if n < 3 {
        return res;
    }
    let atr = wilder_arr(&tr_arr(&cc), (atr_period.round() as i64).max(2));
    let ema_per = (ema_period.round() as i64).max(1) as usize;
    let closes: Vec<f64> = cc.iter().map(|x| x.close).collect();
    let ema: Vec<Option<f64>> = if ema_per <= 1 {
        closes.iter().map(|v| Some(*v)).collect()
    } else {
        ema_arr(&closes, ema_per as i64)
    };
    let touch_mult = if touch_mult > 0.0 { touch_mult } else { 0.5 };
    let min_pct = if min_pct >= 0.0 { min_pct } else { 0.15 };
    let touch_window = (touch_window.round() as i64).max(1);
    let fwd = (fwd.round() as i64).max(1) as f64;
    let tol_at = |i: usize, lvl: f64| -> f64 {
        let at = match atr[i] {
            Some(v) if v.is_finite() => v * touch_mult,
            _ => 0.0,
        };
        at.max(lvl.abs() * (min_pct / 100.0))
    };
    let mut pi = 0usize;
    let mut last_low: Option<EwPivot> = None;
    let mut last_high: Option<EwPivot> = None;
    let mut last_touch_low: Option<(usize, f64, f64)> = None;
    let mut last_touch_high: Option<(usize, f64, f64)> = None;
    let mut state = 0i32;
    let mut anchor_idx: i64 = -1;
    let mut anchor_price: Option<f64> = None;
    let mut slope = 0f64;
    let g = |idx: usize| -> f64 { ema[idx].unwrap_or(f64::NAN) };
    for i in 0..n {
        while pi < piv.len() && piv[pi].at <= i {
            let p = &piv[pi];
            if p.is_high {
                last_high = Some(p.clone());
            } else {
                last_low = Some(p.clone());
            }
            pi += 1;
        }
        let up_turn = i >= 1 && g(i) > g(i - 1) && (i < 2 || g(i - 1) <= g(i - 2));
        let dn_turn = i >= 1 && g(i) < g(i - 1) && (i < 2 || g(i - 1) >= g(i - 2));
        if let Some(ll) = &last_low {
            if ll.price.is_finite() {
                let tol = tol_at(i, ll.price);
                if cc[i].low <= ll.price + tol && cc[i].close >= ll.price - tol {
                    last_touch_low = Some((i, cc[i].low, ll.price));
                }
            }
        }
        if let Some(lh) = &last_high {
            if lh.price.is_finite() {
                let tol = tol_at(i, lh.price);
                if cc[i].high >= lh.price - tol && cc[i].close <= lh.price + tol {
                    last_touch_high = Some((i, cc[i].high, lh.price));
                }
            }
        }
        let bull = last_touch_low
            .map(|(idx, _, lvl)| up_turn && (i as i64 - idx as i64) <= touch_window && cc[i].close > lvl)
            .unwrap_or(false);
        let bear = last_touch_high
            .map(|(idx, _, lvl)| dn_turn && (i as i64 - idx as i64) <= touch_window && cc[i].close < lvl)
            .unwrap_or(false);
        if bull && !bear {
            let (aidx, aprice, _) = last_touch_low.unwrap();
            state = 1;
            anchor_idx = aidx as i64;
            anchor_price = Some(aprice);
            let tgt = last_high.as_ref().filter(|h| h.price > aprice).map(|h| h.price);
            if let Some(t) = tgt {
                slope = (t - aprice) / fwd;
            } else {
                let at = match atr[aidx] {
                    Some(v) if v.is_finite() => v,
                    _ => 0.0,
                };
                let base = if at > 0.0 {
                    at
                } else {
                    let x = aprice.abs() * (min_pct / 100.0);
                    if x == 0.0 { 1.0 } else { x }
                };
                slope = base * 0.25;
            }
            if !(slope > 0.0) {
                let s = slope.abs();
                slope = if s != 0.0 { s } else { aprice.abs() * 1e-4 };
            }
        } else if bear && !bull {
            let (aidx, aprice, _) = last_touch_high.unwrap();
            state = -1;
            anchor_idx = aidx as i64;
            anchor_price = Some(aprice);
            let tgt = last_low.as_ref().filter(|l| l.price < aprice).map(|l| l.price);
            if let Some(t) = tgt {
                slope = (t - aprice) / fwd;
            } else {
                let at = match atr[aidx] {
                    Some(v) if v.is_finite() => v,
                    _ => 0.0,
                };
                let base = if at > 0.0 {
                    at
                } else {
                    let x = aprice.abs() * (min_pct / 100.0);
                    if x == 0.0 { 1.0 } else { x }
                };
                slope = -base * 0.25;
            }
            if !(slope < 0.0) {
                let s = slope.abs();
                slope = if s != 0.0 { -s } else { -aprice.abs() * 1e-4 };
            }
        } else if state == 1 && last_low.as_ref().map(|l| cc[i].close < l.price).unwrap_or(false) {
            state = 0;
            anchor_idx = -1;
            anchor_price = None;
            slope = 0.0;
        } else if state == -1 && last_high.as_ref().map(|h| cc[i].close > h.price).unwrap_or(false) {
            state = 0;
            anchor_idx = -1;
            anchor_price = None;
            slope = 0.0;
        }
        res.state[i] = state;
        if state != 0 && anchor_idx >= 0 && anchor_price.is_some() {
            res.anchor_idx[i] = anchor_idx;
            res.anchor_price[i] = anchor_price;
            res.line[i] = Some(anchor_price.unwrap() + slope * (i as f64 - anchor_idx as f64));
        }
    }
    res
}

pub fn srema_run(a: &SremaRes, s: i32) -> Vec<Point> {
    let n = a.c.len();
    let mut end: i64 = -1;
    for i in (0..n).rev() {
        if a.state[i] == s {
            end = i as i64;
            break;
        }
    }
    if end < 0 {
        return vec![];
    }
    let mut start = end as usize;
    while start >= 1 && a.state[start - 1] == s {
        start -= 1;
    }
    let mut d = Vec::new();
    for i in start..=(end as usize) {
        if let Some(v) = a.line[i] {
            d.push(Point { time: a.c[i].time, value: v, color: None });
        }
    }
    d
}

// ---------------------------------------------------------------------------
// Pane consensus engine
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PaneSignal {
    pub cc: Vec<Candle>,
    pub regime: Vec<i32>,
    pub data: Vec<Point>,
    pub fit_data: Vec<Point>,
    pub cur_reg: i32,
    pub rsi: Vec<Option<f64>>,
    pub bb_pct: Vec<Option<f64>>,
    pub st_k: Vec<Option<f64>>,
    pub cci: Vec<Option<f64>>,
    pub will_r: Vec<Option<f64>>,
    pub mfi: Vec<Option<f64>>,
}

fn clamp1(v: f64) -> f64 {
    if v > 1.0 {
        1.0
    } else if v < -1.0 {
        -1.0
    } else {
        v
    }
}

pub fn pane_signal(candles: &[Candle], o: &Settings) -> PaneSignal {
    let cc: Vec<Candle> = candles
        .iter()
        .filter(|x| x.time as f64 != 0.0 && x.high.is_finite() && x.low.is_finite() && x.close.is_finite())
        .cloned()
        .collect();
    let n = cc.len();
    let mut res = PaneSignal { cc: cc.clone(), regime: vec![0; n], ..Default::default() };
    if n < 35 {
        return res;
    }
    let closes: Vec<f64> = cc.iter().map(|x| x.close).collect();
    let highs: Vec<f64> = cc.iter().map(|x| x.high).collect();
    let lows: Vec<f64> = cc.iter().map(|x| x.low).collect();
    let atr_period = (num(o, "atrPeriod", 14.0).round() as i64).max(2);
    let atr = wilder_arr(&tr_arr(&cc), atr_period);

    // RSI (Wilder)
    let rsi_p = (num(o, "rsiLength", 14.0).round() as i64).max(2);
    let mut rsi: Vec<Option<f64>> = vec![None; n];
    if n >= (rsi_p as usize) + 1 {
        let p = rsi_p as usize;
        let mut g = 0.0;
        let mut l = 0.0;
        for i in 1..=p {
            let ch = closes[i] - closes[i - 1];
            if ch >= 0.0 {
                g += ch;
            } else {
                l -= ch;
            }
        }
        let mut ag = g / p as f64;
        let mut al = l / p as f64;
        rsi[p] = Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) });
        for i in (p + 1)..n {
            let ch = closes[i] - closes[i - 1];
            ag = (ag * (p as f64 - 1.0) + if ch > 0.0 { ch } else { 0.0 }) / p as f64;
            al = (al * (p as f64 - 1.0) + if ch < 0.0 { -ch } else { 0.0 }) / p as f64;
            rsi[i] = Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) });
        }
    }

    // MACD histogram
    let e12 = ema_arr(&closes, 12);
    let e26 = ema_arr(&closes, 26);
    let mut macd_line: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(a), Some(b)) = (e12[i], e26[i]) {
            macd_line[i] = Some(a - b);
        }
    }
    let sg = ema_skip(&macd_line, 9);
    let mut macd_hist: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(m), Some(s)) = (macd_line[i], sg[i]) {
            macd_hist[i] = Some(m - s);
        }
    }

    // Bollinger %B
    let bb_p = (num(o, "bbLength", 20.0).round() as i64).max(2);
    let mid = sma_arr(&closes, bb_p);
    let sd = stdev_arr(&closes, bb_p);
    let mut bb_pct: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(m), Some(s)) = (mid[i], sd[i]) {
            let u = m + 2.0 * s;
            let lo = m - 2.0 * s;
            let rng = u - lo;
            bb_pct[i] = Some(if rng > 0.0 { (closes[i] - lo) / rng } else { 0.5 });
        }
    }

    // Stochastic %K
    let st_p = (num(o, "stochLength", 14.0).round() as i64).max(2);
    let hh = highest_arr(&highs, st_p);
    let ll = lowest_arr(&lows, st_p);
    let mut st_k: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(h), Some(l)) = (hh[i], ll[i]) {
            let rng = h - l;
            st_k[i] = Some(if rng > 0.0 { 100.0 * (closes[i] - l) / rng } else { 50.0 });
        }
    }

    // CCI
    let cci_p = (num(o, "cciLength", 20.0).round() as i64).max(2);
    let tp: Vec<f64> = cc.iter().map(|x| (x.high + x.low + x.close) / 3.0).collect();
    let sm = sma_arr(&tp, cci_p);
    let mut cci: Vec<Option<f64>> = vec![None; n];
    for i in (cci_p as usize - 1)..n {
        let mut md = 0.0;
        for j in (i + 1 - cci_p as usize)..=i {
            md += (tp[j] - sm[i].unwrap_or(f64::NAN)).abs();
        }
        md /= cci_p as f64;
        cci[i] = Some(if md > 0.0 { (tp[i] - sm[i].unwrap_or(0.0)) / (0.015 * md) } else { 0.0 });
    }

    // Williams %R
    let wr_p = (num(o, "willrLength", 14.0).round() as i64).max(2);
    let whh = highest_arr(&highs, wr_p);
    let wll = lowest_arr(&lows, wr_p);
    let mut will_r: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(h), Some(l)) = (whh[i], wll[i]) {
            let rng = h - l;
            will_r[i] = Some(if rng > 0.0 { -100.0 * (h - closes[i]) / rng } else { -50.0 });
        }
    }

    // MFI
    let mf_p = (num(o, "mfiLength", 14.0).round() as i64).max(2);
    let p: usize = mf_p as usize;
    let mut mfi: Vec<Option<f64>> = vec![None; n];
    {
        let mut pv = vec![0.0f64; n];
        let mut nv = vec![0.0f64; n];
        for i in 1..n {
            let t0 = (cc[i - 1].high + cc[i - 1].low + cc[i - 1].close) / 3.0;
            let t1 = (cc[i].high + cc[i].low + cc[i].close) / 3.0;
            let raw = (if cc[i].volume.is_finite() { cc[i].volume } else { 0.0 }) * t1;
            if t1 > t0 {
                pv[i] = raw;
            } else if t1 < t0 {
                nv[i] = raw;
            }
        }
        let mut sp = 0.0;
        let mut sn = 0.0;
        for i in p..n {
            if i == p {
                for j in (i + 1 - p)..=i {
                    sp += pv[j];
                    sn += nv[j];
                }
            } else {
                sp += pv[i] - pv[i - p];
                sn += nv[i] - nv[i - p];
            }
            mfi[i] = Some(if (sp + sn) > 0.0 { 100.0 * sp / (sp + sn) } else { 50.0 });
        }
    }

    // RSI divergence vote
    let mut div_vote = vec![0.0f64; n];
    if boolv(o, "useDiv", true) {
        let fp = fractal_pivots(&cc, 3.0);
        let hold = 8usize;
        let mut prev_low: Option<Pivot> = None;
        let mut prev_high: Option<Pivot> = None;
        for pt in &fp {
            if rsi[pt.idx].is_none() {
                continue;
            }
            let rv = rsi[pt.idx].unwrap_or(f64::NAN);
            if !pt.is_high {
                if let Some(pl) = prev_low {
                    let prs = rsi[pl.idx].unwrap_or(f64::NAN);
                    if pt.price < pl.price && rv > prs {
                        for i in pt.at..n.min(pt.at + hold + 1) {
                            div_vote[i] = 1.0;
                        }
                    }
                }
                prev_low = Some(*pt);
            } else {
                if let Some(ph) = prev_high {
                    let prs = rsi[ph.idx].unwrap_or(f64::NAN);
                    if pt.price > ph.price && rv < prs {
                        for i in pt.at..n.min(pt.at + hold + 1) {
                            div_vote[i] = -1.0;
                        }
                    }
                }
                prev_high = Some(*pt);
            }
        }
    }

    // Composite vote + overbought/oversold dampening
    let use_rsi = boolv(o, "useRSI", true);
    let use_macd = boolv(o, "useMACD", true);
    let use_bb = boolv(o, "useBB", true);
    let use_stoch = boolv(o, "useStoch", true);
    let use_cci = boolv(o, "useCCI", true);
    let use_willr = boolv(o, "useWillR", true);
    let use_mfi = boolv(o, "useMFI", true);
    let use_div = boolv(o, "useDiv", true);
    let mut comp = vec![0.0f64; n];
    for i in 0..n {
        let mut s = 0.0;
        let mut w = 0.0;
        let mut add = |v: Option<f64>, wt: f64, s: &mut f64, w: &mut f64| {
            if let Some(v) = v {
                if v.is_finite() {
                    *s += v * wt;
                    *w += wt;
                }
            }
        };
        if use_rsi {
            add(rsi[i].map(|v| clamp1((v - 50.0) / 20.0)), 1.0, &mut s, &mut w);
        }
        if use_macd {
            if let (Some(mh), Some(a)) = (macd_hist[i], atr[i]) {
                add(Some(clamp1(mh / (a.max(1e-9) * 0.5))), 1.0, &mut s, &mut w);
            }
        }
        if use_bb {
            add(bb_pct[i].map(|v| clamp1((v - 0.5) * 2.0)), 1.0, &mut s, &mut w);
        }
        if use_stoch {
            add(st_k[i].map(|v| clamp1((v - 50.0) / 40.0)), 1.0, &mut s, &mut w);
        }
        if use_cci {
            add(cci[i].map(|v| clamp1(v / 150.0)), 1.0, &mut s, &mut w);
        }
        if use_willr {
            add(will_r[i].map(|v| clamp1((v + 50.0) / 40.0)), 1.0, &mut s, &mut w);
        }
        if use_mfi {
            add(mfi[i].map(|v| clamp1((v - 50.0) / 30.0)), 1.0, &mut s, &mut w);
        }
        if use_div && div_vote[i] != 0.0 {
            add(Some(div_vote[i]), 1.5, &mut s, &mut w);
        }
        let mut c0 = if w > 0.0 { s / w } else { 0.0 };
        let mut ob = 0;
        let mut os = 0;
        if let Some(v) = rsi[i] {
            if v > 70.0 {
                ob += 1;
            } else if v < 30.0 {
                os += 1;
            }
        }
        if let Some(v) = bb_pct[i] {
            if v > 1.0 {
                ob += 1;
            } else if v < 0.0 {
                os += 1;
            }
        }
        if let Some(v) = st_k[i] {
            if v > 80.0 {
                ob += 1;
            } else if v < 20.0 {
                os += 1;
            }
        }
        if let Some(v) = cci[i] {
            if v > 100.0 {
                ob += 1;
            } else if v < -100.0 {
                os += 1;
            }
        }
        if let Some(v) = will_r[i] {
            if v > -20.0 {
                ob += 1;
            } else if v < -80.0 {
                os += 1;
            }
        }
        if let Some(v) = mfi[i] {
            if v > 80.0 {
                ob += 1;
            } else if v < 20.0 {
                os += 1;
            }
        }
        if c0 > 0.0 && ob >= 3 {
            c0 *= 0.5;
        } else if c0 < 0.0 && os >= 3 {
            c0 *= 0.5;
        }
        comp[i] = c0;
    }
    let smooth_len = (num(o, "smooth", 3.0).round() as i64).max(1);
    let mut sm: Vec<f64> = if smooth_len > 1 { let e = ema_arr(&comp, smooth_len); (0..n).map(|i| e[i].unwrap_or(comp[i])).collect() } else { comp.clone() };
    for i in 0..n {
        if !sm[i].is_finite() {
            sm[i] = comp[i];
        }
    }
    let mut bull_th = num(o, "bullTh", 0.12);
    if bull_th <= 0.0 {
        bull_th = 0.12;
    }
    let mut bear_th = num(o, "bearTh", 0.12);
    if bear_th <= 0.0 {
        bear_th = 0.12;
    }
    let mut rg = 0i32;
    let mut started = false;
    for i in 0..n {
        let ready = rsi[i].is_some() && bb_pct[i].is_some() && st_k[i].is_some();
        if !started {
            if !ready {
                res.regime[i] = 0;
                continue;
            }
            started = true;
        }
        let v = sm[i];
        if v >= bull_th {
            rg = 1;
        } else if v <= -bear_th {
            rg = -1;
        } else if rg == 0 {
            rg = if v >= 0.0 { 1 } else { -1 };
        }
        res.regime[i] = rg;
    }
    let min_seg = (num(o, "minSeg", 5.0).round() as i64).max(1) as usize;
    let regime = &mut res.regime;
    for _ in 0..2 {
        let cp = regime.clone();
        for i in 0..n {
            let mut acc = 0;
            let lo = i.saturating_sub(min_seg);
            let hi = (n - 1).min(i + min_seg);
            for j in lo..=hi {
                acc += cp[j];
            }
            regime[i] = if acc > 0 { 1 } else if acc < 0 { -1 } else { cp[i] };
        }
    }
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let mut st = 0usize;
    for i in 1..=n {
        if i == n || regime[i] != regime[i - 1] {
            let r = regime[st];
            if r != 0 {
                let a = st;
                let b = i - 1;
                let len = b - a + 1;
                if len >= 1 {
                    let mut sx = 0.0;
                    let mut sy = 0.0;
                    let mut sxx = 0.0;
                    let mut sxy = 0.0;
                    for k in a..=b {
                        let kf = k as f64;
                        sx += kf;
                        sy += closes[k];
                        sxx += kf * kf;
                        sxy += kf * closes[k];
                    }
                    let den = len as f64 * sxx - sx * sx;
                    let m = if den != 0.0 { (len as f64 * sxy - sx * sy) / den } else { 0.0 };
                    let q = (sy - m * sx) / len as f64;
                    let col = if r > 0 { up.clone() } else { dn.clone() };
                    for k in a..=b {
                        let mut val = m * k as f64 + q;
                        if !val.is_finite() {
                            val = closes[k];
                        }
                        res.data.push(Point { time: cc[k].time, value: val, color: Some(col.clone()) });
                    }
                }
            }
            st = i;
        }
    }
    let fit_look = (num(o, "fitLook", 60.0).round() as i64).max(10) as usize;
    let f0 = n.saturating_sub(fit_look);
    let flen = n - f0;
    let mut fsx = 0.0;
    let mut fsy = 0.0;
    let mut fsxx = 0.0;
    let mut fsxy = 0.0;
    for i in f0..n {
        let x = i as f64;
        fsx += x;
        fsy += closes[i];
        fsxx += x * x;
        fsxy += x * closes[i];
    }
    let fden = flen as f64 * fsxx - fsx * fsx;
    let fm = if fden != 0.0 {
        (flen as f64 * fsxy - fsx * fsy) / fden
    } else {
        0.0
    };
    let fq = (fsy - fm * fsx) / flen as f64;
    for i in f0..n {
        let v = fm * i as f64 + fq;
        if v.is_finite() {
            res.fit_data.push(Point { time: cc[i].time, value: v, color: None });
        }
    }
    res.cur_reg = if regime[n - 1] != 0 {
        regime[n - 1]
    } else if sm[n - 1] >= 0.0 {
        1
    } else {
        -1
    };
    res.rsi = rsi;
    res.bb_pct = bb_pct;
    res.st_k = st_k;
    res.cci = cci;
    res.will_r = will_r;
    res.mfi = mfi;
    res
}

// ---------------------------------------------------------------------------
// Volume-Liquidity Trend Core engine
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct VlCoreRes {
    pub dir: Vec<i32>,
    pub conf: Vec<f64>,
    pub val: Vec<Option<f64>>,
    pub s_val: Vec<Option<f64>>,
    pub h_val: Vec<Option<f64>>,
    pub atr: Vec<f64>,
}

fn clipv(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn vl_wma(vals: &[f64], p: usize) -> Vec<Option<f64>> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p == 0 || n < p {
        return out;
    }
    let denom = (p * (p + 1) / 2) as f64;
    for i in (p - 1)..n {
        let mut s = 0.0;
        for j in 0..p {
            s += (j + 1) as f64 * vals[(i - (p - 1)) + j];
        }
        out[i] = Some(s / denom);
    }
    out
}

fn vl_hma(vals: &[f64], p: usize) -> Vec<Option<f64>> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p == 0 || n < p {
        return out;
    }
    let half = ((p as f64 / 2.0).round() as usize).max(1);
    let w1 = vl_wma(vals, half);
    let w2 = vl_wma(vals, p);
    let f = p - 1;
    let sq = ((p as f64).sqrt().round() as usize).max(1);
    let denom = (sq * (sq + 1) / 2) as f64;
    for i in (f + sq - 1)..n {
        let a = i + 1 - sq;
        let mut s = 0.0;
        for j in a..=i {
            s += (j - a + 1) as f64 * (2.0 * w1[j].unwrap_or(0.0) - w2[j].unwrap_or(0.0));
        }
        out[i] = Some(s / denom);
    }
    out
}

fn vl_lsma(vals: &[f64], p: usize) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let n = vals.len();
    let mut out_v = vec![None; n];
    let mut out_s = vec![None; n];
    if p < 2 || n < p {
        return (out_v, out_s);
    }
    let mid = (p as f64 - 1.0) / 2.0;
    let mut varx = 0.0;
    for x in 0..p {
        let d = x as f64 - mid;
        varx += d * d;
    }
    for i in (p - 1)..n {
        let a = i + 1 - p;
        let mut sum = 0.0;
        for j in a..=i {
            sum += vals[j];
        }
        let mean = sum / p as f64;
        let mut cov = 0.0;
        for j in a..=i {
            cov += ((j - a) as f64 - mid) * (vals[j] - mean);
        }
        if varx <= 0.0 {
            out_v[i] = Some(mean);
            out_s[i] = Some(0.0);
            continue;
        }
        let slope = cov / varx;
        out_s[i] = Some(slope);
        out_v[i] = Some(mean + slope * (p as f64 - 1.0 - mid));
    }
    (out_v, out_s)
}

fn vl_atr(c: &[Candle], p: usize) -> Vec<Option<f64>> {
    let n = c.len();
    let mut out = vec![None; n];
    if n < p || p == 0 {
        return out;
    }
    let mut s = 0.0;
    for i in 0..p.min(n) {
        let h = c[i].high;
        let l = c[i].low;
        let pc = if i > 0 { c[i - 1].close } else { l };
        s += (h - l).max((h - pc).abs()).max((l - pc).abs());
    }
    out[p - 1] = Some(s / p as f64);
    for i in p..n {
        let h = c[i].high;
        let l = c[i].low;
        let pc = c[i - 1].close;
        let tr = (h - l).max((h - pc).abs()).max((l - pc).abs());
        out[i] = Some((out[i - 1].unwrap_or(0.0) * (p as f64 - 1.0) + tr) / p as f64);
    }
    out
}

fn rolling_ext(c: &[Candle], high: bool, w: usize) -> Vec<Option<f64>> {
    let n = c.len();
    let mut out = vec![None; n];
    if w == 0 {
        return out;
    }
    for i in (w - 1)..n {
        let mut m = if high { f64::NEG_INFINITY } else { f64::INFINITY };
        for j in (i + 1 - w)..=i {
            let v = if high { c[j].high } else { c[j].low };
            if high {
                if v > m {
                    m = v;
                }
            } else if v < m {
                m = v;
            }
        }
        out[i] = Some(m);
    }
    out
}

fn vl_pivots(c: &[Candle], w: usize) -> (Vec<bool>, Vec<bool>) {
    let n = c.len();
    let mut is_hi = vec![false; n];
    let mut is_lo = vec![false; n];
    if w == 0 || n < 2 * w + 1 {
        return (is_hi, is_lo);
    }
    for c2 in w..(n - w) {
        let mut ok_hi = true;
        let mut ok_lo = true;
        let base = c[c2];
        for k in 0..=(2 * w) {
            if k == w {
                continue;
            }
            let idx = c2 + k - w;
            let v = c[idx];
            if base.high <= v.high {
                ok_hi = false;
            }
            if base.low >= v.low {
                ok_lo = false;
            }
            if !ok_hi && !ok_lo {
                break;
            }
        }
        if ok_hi {
            is_hi[c2] = true;
        }
        if ok_lo {
            is_lo[c2] = true;
        }
    }
    (is_hi, is_lo)
}

fn first_ge(arr: &[usize], idx: i64) -> i64 {
    let mut lo = 0i64;
    let mut hi = arr.len() as i64 - 1;
    let mut ans = -1i64;
    while lo <= hi {
        let m = (lo + hi) >> 1;
        if arr[m as usize] as i64 >= idx {
            ans = m;
            hi = m - 1;
        } else {
            lo = m + 1;
        }
    }
    ans
}

#[derive(Clone, Copy, Default)]
struct Ls {
    n: f64,
    sx: f64,
    sy: f64,
    sxx: f64,
    sxy: f64,
}

impl Ls {
    fn reset(&mut self) {
        *self = Ls::default();
    }
    fn feed(&mut self, i: f64, y: f64) {
        self.sx += i;
        self.sy += y;
        self.sxx += i * i;
        self.sxy += i * y;
        self.n += 1.0;
    }
    fn slope(&self) -> f64 {
        if self.n < 2.0 {
            return 0.0;
        }
        let den = self.n * self.sxx - self.sx * self.sx;
        if den.abs() < 1e-12 {
            return 0.0;
        }
        (self.n * self.sxy - self.sx * self.sy) / den
    }
    fn end(&self, i: f64) -> Option<f64> {
        if self.n < 1.0 {
            return None;
        }
        let sl = self.slope();
        Some((self.sy - sl * self.sx) / self.n + sl * i)
    }
}

pub fn vlcore_engine(candles: &[Candle], o: &Settings) -> VlCoreRes {
    let n = candles.len();
    let mut res = VlCoreRes {
        dir: vec![0; n],
        conf: vec![0.0; n],
        val: vec![None; n],
        s_val: vec![None; n],
        h_val: vec![None; n],
        atr: vec![0.0; n],
    };
    if n < 10 {
        return res;
    }
    let l0 = num(o, "length", 21.0).round() as i64;
    let l = l0.max(5) as usize;
    let atr_p = (num(o, "atrLength", 14.0).round() as i64).max(2) as usize;
    let gap = num(o, "gap", 1.0);
    let _ = gap;
    let use_vol = boolv(o, "useVolume", true);
    let straight = boolv(o, "straightLine", true);
    let wick = (num(o, "wickLen", 3.0).round() as i64).max(1) as usize;
    let confirm = (num(o, "confirm", 2.0).round() as i64).max(1) as i32;
    let strong = {
        let s = num(o, "strongThr", 0.0);
        if s >= 0.1 { s } else { 1.15 }
    };

    let mut hi = vec![0.0; n];
    let mut lo = vec![0.0; n];
    let mut cl = vec![0.0; n];
    let mut op = vec![0.0; n];
    let mut hl = vec![0.0; n];
    let mut vol = vec![0.0; n];
    for i in 0..n {
        hi[i] = candles[i].high;
        lo[i] = candles[i].low;
        cl[i] = candles[i].close;
        op[i] = candles[i].open;
        hl[i] = (hi[i] + lo[i]) / 2.0;
        vol[i] = candles[i].volume;
    }

    let atr = vl_atr(candles, atr_p);
    let e_fast = ema_arr(&cl, ((l as f64 * 0.4).round() as i64).max(2));
    let e_slow = ema_arr(&cl, l as i64);
    let hma = vl_hma(&hl, l);
    let lr = vl_lsma(&cl, l);

    let avg_k = ((l as f64 / 3.0).round() as usize).max(2);
    let rail_w = ((l as f64 / 2.0).round() as usize).max(3);

    let roll_hi = rolling_ext(candles, true, rail_w);
    let roll_lo = rolling_ext(candles, false, rail_w);

    let vma = sma_arr(&vol, 20);
    let mut vr = vec![1.0f64; n];
    for i in 0..n {
        if let Some(v) = vma[i] {
            if v > 0.0 && vol[i] > 0.0 {
                vr[i] = vol[i] / v;
            }
        }
    }

    let mut atr_safe = vec![0.0f64; n];
    let mut last_good = 0.0f64;
    for i in 0..n {
        let mut a = atr[i].unwrap_or(f64::NAN);
        if !a.is_finite() || a == 0.0 {
            a = last_good;
        }
        if a <= 0.0 {
            a = (cl[i] * 0.001).max(1e-9);
        }
        atr_safe[i] = a;
        last_good = a;
    }
    res.atr = atr_safe.clone();

    let mut s_arr = vec![0.0f64; n];
    for i in 1..n {
        let a = atr_safe[i];
        let mut f_vel = 0.0;
        let mut f_pos = 0.0;
        let mut f_ema = 0.0;
        if let Some(s) = lr.1[i] {
            f_vel = clipv(s * l as f64 / a, -2.0, 2.0) / 2.0;
        }
        if let Some(h) = hma[i] {
            f_pos = clipv((cl[i] - h) / (a * 1.2), -2.0, 2.0) / 2.0;
        }
        if let (Some(f), Some(s)) = (e_fast[i], e_slow[i]) {
            f_ema = clipv((f - s) / (a * 0.9), -2.0, 2.0) / 2.0;
        }
        let mut f_pat = 0.0;
        let c = candles[i];
        let pc = candles[i - 1];
        let rng = (c.high - c.low).max(1e-9);
        let up_wick = c.high - c.open.max(c.close);
        let dn_wick = c.open.min(c.close) - c.low;
        if up_wick / rng > 0.6 && dn_wick < rng * 0.25 && c.close < c.open {
            f_pat -= 0.25;
        }
        if dn_wick / rng > 0.6 && up_wick < rng * 0.25 && c.close > c.open {
            f_pat += 0.25;
        }
        if c.close > c.open && pc.close < pc.open && c.low <= pc.low && c.close >= (pc.open + pc.close) / 2.0 {
            f_pat += 0.4;
        }
        if c.close < c.open && pc.close > pc.open && c.high >= pc.high && c.close <= (pc.open + pc.close) / 2.0 {
            f_pat -= 0.4;
        }
        let mut vol_k = 1.0;
        if use_vol {
            let r = vr[i];
            if r >= 1.1 {
                vol_k = (0.8 + (r - 1.0) * 0.6).min(1.6);
            } else if r < 0.75 {
                vol_k = 0.55;
            }
        }
        let s = 0.42 * f_vel + 0.30 * f_pos + 0.27 * f_ema + clipv(f_pat, -1.0, 1.0) * 0.5 * vol_k;
        s_arr[i] = clipv(s, -2.0, 2.0);
    }

    let (is_hi, is_lo) = vl_pivots(candles, wick);
    let mut pk_hi: Vec<usize> = Vec::new();
    let mut pk_lo: Vec<usize> = Vec::new();
    for i in 0..n {
        if is_hi[i] {
            pk_hi.push(i);
        }
        if is_lo[i] {
            pk_lo.push(i);
        }
    }

    let mut dir = 0i32;
    let mut st_low: Option<f64> = None;
    let mut st_high: Option<f64> = None;
    let mut pending_up = 0i32;
    let mut pending_down = 0i32;
    let mut grab_until: i64 = -1;
    let mut prev_close_ref: f64 = -1.0;
    let mut processed_lo: i64 = -1;
    let mut processed_hi: i64 = -1;
    let mut prev_out_dir = 0i32;
    let mut glide_rem = 0i32;
    let mut drawn_val: Option<f64> = None;
    let mut d_dir = 0i32;
    let mut yv: Option<f64> = None;
    let mut m_s = 0.0f64;
    let mut hold = false;
    let mut hold_why = ' ';
    let mut last_ref: i64 = 0;
    let mut need_ref = false;
    let mut ls = Ls::default();
    let mut acc_store = vec![0.0f64; avg_k];

    for i in (rail_w.max(2))..n {
        let c = candles[i];
        let a = atr_safe[i];
        let idx = i % avg_k;
        acc_store[idx] = s_arr[i];
        let mut acc = 0.0;
        for t in 0..avg_k {
            acc += acc_store[t];
        }
        acc /= avg_k as f64;

        let mut threat = 0i32;

        loop {
            let k = first_ge(&pk_lo, processed_lo + 1);
            if k < 0 {
                break;
            }
            let plo = pk_lo[k as usize];
            if plo as i64 <= i as i64 - wick as i64 {
                processed_lo = plo as i64;
                let lp = candles[plo].low;
                if dir == 1 && (st_low.is_none() || lp >= st_low.unwrap()) {
                    st_low = Some(lp);
                }
            } else {
                break;
            }
        }
        loop {
            let k = first_ge(&pk_hi, processed_hi + 1);
            if k < 0 {
                break;
            }
            let phi = pk_hi[k as usize];
            if phi as i64 <= i as i64 - wick as i64 {
                processed_hi = phi as i64;
                let hp = candles[phi].high;
                if dir == -1 && (st_high.is_none() || hp <= st_high.unwrap()) {
                    st_high = Some(hp);
                }
            } else {
                break;
            }
        }

        let rl_prev = roll_lo[i - 1].or(roll_lo[i]).unwrap_or(c.low);
        let rh_prev = roll_hi[i - 1].or(roll_hi[i]).unwrap_or(c.high);

        if dir == 1 {
            let local = st_low.unwrap_or(f64::NEG_INFINITY).max(rl_prev);
            let is_grab = c.low < local && c.close > local;
            if is_grab {
                grab_until = i as i64 + wick.max(2) as i64;
            }
            let under = c.close <= local;
            threat = if is_grab || under { 1 } else { 0 };
            if i as i64 > grab_until && !is_grab {
                if under {
                    pending_down += 1;
                    let crash = c.close < c.open
                        && acc <= -strong
                        && prev_close_ref >= 0.0
                        && c.close <= prev_close_ref - a * 0.6;
                    if pending_down >= confirm || crash {
                        dir = -1;
                        st_high = Some((roll_hi[i].unwrap_or(cl[i])).max(cl[i]));
                        st_low = None;
                        pending_up = 0;
                        pending_down = 0;
                        grab_until = i as i64 + wick.max(2) as i64;
                    }
                } else {
                    pending_down = 0;
                }
            }
            res.conf[i] = acc.abs().min(1.0);
        } else if dir == -1 {
            let local = st_high.unwrap_or(f64::INFINITY).min(rh_prev);
            let is_grab = c.high > local && c.close < local;
            if is_grab {
                grab_until = i as i64 + wick.max(2) as i64;
            }
            let over = c.close >= local;
            threat = if is_grab || over { 1 } else { 0 };
            if i as i64 > grab_until && !is_grab {
                if over {
                    pending_up += 1;
                    let crash = c.close > c.open
                        && acc >= strong
                        && prev_close_ref >= 0.0
                        && c.close >= prev_close_ref + a * 0.6;
                    if pending_up >= confirm || crash {
                        dir = 1;
                        st_low = Some((roll_lo[i].unwrap_or(cl[i])).min(cl[i]));
                        st_high = None;
                        pending_up = 0;
                        pending_down = 0;
                        grab_until = i as i64 + wick.max(2) as i64;
                    }
                } else {
                    pending_up = 0;
                }
            }
            res.conf[i] = acc.abs().min(1.0);
        } else {
            if acc >= 0.18 {
                if c.close > rh_prev || acc >= strong {
                    dir = 1;
                    st_low = Some((roll_lo[i].unwrap_or(cl[i])).min(cl[i]));
                    st_high = None;
                    pending_up = 0;
                    pending_down = 0;
                    grab_until = i as i64 + wick.max(2) as i64;
                }
            } else if acc <= -0.18 && (c.close < rl_prev || acc <= -strong) {
                dir = -1;
                st_high = Some((roll_hi[i].unwrap_or(cl[i])).max(cl[i]));
                st_low = None;
                pending_up = 0;
                pending_down = 0;
                grab_until = i as i64 + wick.max(2) as i64;
            }
        }

        let mut leg_damp = 1.0;
        if dir == 1 {
            if let Some(rh) = roll_hi[i] {
                let ext = ((rh - cl[i]) / a).max(0.0);
                if ext > 6.0 {
                    leg_damp = (1.0 - (ext - 6.0) * 0.05).max(0.45);
                }
            }
        } else if dir == -1 {
            if let Some(rl) = roll_lo[i] {
                let ext = ((cl[i] - rl) / a).max(0.0);
                if ext > 6.0 {
                    leg_damp = (1.0 - (ext - 6.0) * 0.05).max(0.45);
                }
            }
        }

        res.dir[i] = dir;
        res.conf[i] = res.conf[i].max(acc.abs()) * leg_damp;

        let mut target: Option<f64> = None;
        let mut anchor: Option<f64> = None;
        if straight {
            if dir != 0 {
                if dir != d_dir {
                    ls.reset();
                    d_dir = dir;
                    last_ref = i as i64;
                    if dir == 1 {
                        let mut j = i;
                        let mut jv = c.low;
                        let start = rail_w.max(i.saturating_sub(rail_w));
                        for t in start..=i {
                            if candles[t].low <= jv {
                                jv = candles[t].low;
                                j = t;
                            }
                        }
                        yv = Some(jv);
                        let dx = i as i64 - j as i64;
                        m_s = if dx > 0 { ((c.close - jv) / dx as f64).max(0.0) } else { 0.0 };
                    } else {
                        let mut j = i;
                        let mut jv = c.high;
                        let start = rail_w.max(i.saturating_sub(rail_w));
                        for t in start..=i {
                            if candles[t].high >= jv {
                                jv = candles[t].high;
                                j = t;
                            }
                        }
                        yv = Some(jv);
                        let dx = i as i64 - j as i64;
                        m_s = if dx > 0 { ((c.close - jv) / dx as f64).min(0.0) } else { 0.0 };
                    }
                    ls.feed(i as f64, c.close);
                    target = yv;
                } else {
                    ls.feed(i as f64, c.close);
                    if dir == 1 {
                        if hold {
                            if hold_why == 'g' {
                                if threat == 0 {
                                    hold = false;
                                    need_ref = true;
                                }
                            } else if c.close >= yv.unwrap_or(f64::NEG_INFINITY) {
                                hold = false;
                                need_ref = true;
                            }
                        } else if threat != 0 {
                            hold = true;
                            hold_why = 'g';
                        }
                        if !hold {
                            if yv.is_none() || !yv.unwrap().is_finite() {
                                yv = Some(c.close);
                            }
                            if ls.n >= 4.0 {
                                let se = ls.end(i as f64);
                                let sl = ls.slope();
                                let drift_slope = (m_s - sl).abs() * ((i as i64 - last_ref).max(1) as f64);
                                let drift_level = (yv.unwrap() - se.unwrap_or(yv.unwrap())).abs();
                                if need_ref || drift_slope > a * 0.35 || drift_level > a * 1.5 {
                                    m_s = sl.max(0.0);
                                    if let Some(se) = se {
                                        let mut d = se - yv.unwrap();
                                        d = clipv(d, -a * 0.5, a * 0.5);
                                        yv = Some(yv.unwrap() + d);
                                    }
                                    last_ref = i as i64;
                                    need_ref = false;
                                }
                            }
                            if c.close >= yv.unwrap() {
                                yv = Some(yv.unwrap() + m_s);
                            } else {
                                hold = true;
                                hold_why = 'o';
                            }
                        }
                    } else {
                        if hold {
                            if hold_why == 'g' {
                                if threat == 0 {
                                    hold = false;
                                    need_ref = true;
                                }
                            } else if c.close <= yv.unwrap_or(f64::INFINITY) {
                                hold = false;
                                need_ref = true;
                            }
                        } else if threat != 0 {
                            hold = true;
                            hold_why = 'g';
                        }
                        if !hold {
                            if yv.is_none() || !yv.unwrap().is_finite() {
                                yv = Some(c.close);
                            }
                            if ls.n >= 4.0 {
                                let se = ls.end(i as f64);
                                let sl = ls.slope();
                                let drift_slope = (m_s - sl).abs() * ((i as i64 - last_ref).max(1) as f64);
                                let drift_level = (yv.unwrap() - se.unwrap_or(yv.unwrap())).abs();
                                if need_ref || drift_slope > a * 0.35 || drift_level > a * 1.5 {
                                    m_s = sl.min(0.0);
                                    if let Some(se) = se {
                                        let mut d = se - yv.unwrap();
                                        d = clipv(d, -a * 0.5, a * 0.5);
                                        yv = Some(yv.unwrap() + d);
                                    }
                                    last_ref = i as i64;
                                    need_ref = false;
                                }
                            }
                            if c.close <= yv.unwrap() {
                                yv = Some(yv.unwrap() + m_s);
                            } else {
                                hold = true;
                                hold_why = 'o';
                            }
                        }
                    }
                    target = yv;
                }
            } else {
                if d_dir != 0 {
                    ls.reset();
                }
                let mut r = yv.filter(|v| v.is_finite());
                if r.is_none() {
                    let fallback = if let Some(h) = hma[i] {
                        if h.is_finite() {
                            Some(h)
                        } else if roll_hi[i].is_some() && roll_lo[i].is_some() {
                            Some((roll_hi[i].unwrap() + roll_lo[i].unwrap()) / 2.0)
                        } else {
                            None
                        }
                    } else if roll_hi[i].is_some() && roll_lo[i].is_some() {
                        Some((roll_hi[i].unwrap() + roll_lo[i].unwrap()) / 2.0)
                    } else {
                        None
                    };
                    r = Some(fallback.unwrap_or(cl[i]));
                    yv = r;
                    last_ref = i as i64;
                } else {
                    let a2 = atr_safe[i];
                    if let Some(h) = hma[i] {
                        if h.is_finite() && (cl[i] < yv.unwrap() - a2 * 1.5 || cl[i] > yv.unwrap() + a2 * 1.5) {
                            let mut d = h - yv.unwrap();
                            d = clipv(d, -a2 * 0.4, a2 * 0.4);
                            yv = Some(yv.unwrap() + d);
                        }
                    }
                }
                target = yv;
            }
        } else {
            let lr_val = lr.0[i];
            let lr_sl = lr.1[i].filter(|v| v.is_finite()).unwrap_or(0.0);
            anchor = if let Some(h) = hma[i] {
                if h.is_finite() {
                    Some(h + lr_sl)
                } else {
                    lr_val.filter(|v| v.is_finite())
                }
            } else {
                lr_val.filter(|v| v.is_finite())
            };
            if anchor.is_none() {
                anchor = Some(cl[i]);
            }
            target = anchor;
        }
        if target.is_none() || !target.unwrap().is_finite() {
            target = Some(cl[i]);
        }
        res.s_val[i] = target;
        res.h_val[i] = anchor.or(target);

        if dir != prev_out_dir {
            glide_rem = 4;
            prev_out_dir = dir;
        }
        match drawn_val {
            None => {
                drawn_val = target;
                glide_rem = 0;
            }
            Some(dv) if !dv.is_finite() => {
                drawn_val = target;
                glide_rem = 0;
            }
            Some(dv) => {
                if glide_rem > 0 {
                    drawn_val = Some(dv + (target.unwrap() - dv) * 0.5);
                    glide_rem -= 1;
                } else {
                    drawn_val = target;
                }
            }
        }
        res.val[i] = drawn_val;
        prev_close_ref = c.close;
    }

    res
}

// ---------------------------------------------------------------------------
// RSI divergence core
// ---------------------------------------------------------------------------

pub fn rsi_wilder(closes: &[f64], len: i64) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if len < 1 || n < (len as usize + 1) {
        return out;
    }
    let p = len as usize;
    let mut g = 0.0;
    let mut l = 0.0;
    for i in 1..=p {
        let d = closes[i] - closes[i - 1];
        if d > 0.0 {
            g += d;
        } else {
            l -= d;
        }
    }
    let mut ag = g / p as f64;
    let mut al = l / p as f64;
    out[p] = Some(if al == 0.0 {
        if ag == 0.0 { 50.0 } else { 100.0 }
    } else {
        100.0 - 100.0 / (1.0 + ag / al)
    });
    for i in (p + 1)..n {
        let d = closes[i] - closes[i - 1];
        let gain = if d > 0.0 { d } else { 0.0 };
        let loss = if d < 0.0 { -d } else { 0.0 };
        ag = (ag * (p as f64 - 1.0) + gain) / p as f64;
        al = (al * (p as f64 - 1.0) + loss) / p as f64;
        out[i] = Some(if al == 0.0 {
            if ag == 0.0 { 50.0 } else { 100.0 }
        } else {
            100.0 - 100.0 / (1.0 + ag / al)
        });
    }
    out
}

#[derive(Clone, Debug)]
pub struct DivSignal {
    pub index: usize,
    pub time: i64,
    pub kind_bull: bool,
    pub hidden: bool,
}

pub fn divergence(candles: &[Candle], length: i64, pivot: i64, lookback: i64) -> Vec<DivSignal> {
    let len = length.max(1);
    let p = pivot.max(1) as usize;
    let lookback = lookback.max((p * 2) as i64);
    let n = candles.len();
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows: Vec<f64> = candles.iter().map(|c| c.low).collect();
    let rsi = rsi_wilder(&closes, len);
    // strict-fractal swing points on the RSI series
    let mut lows_idx: Vec<usize> = Vec::new();
    let mut highs_idx: Vec<usize> = Vec::new();
    if n >= 2 * p + 1 {
        for i in p..(n - p) {
            let v = match rsi[i] {
                Some(v) if v.is_finite() => v,
                _ => continue,
            };
            let mut l_ok = true;
            let mut h_ok = true;
            for j in (i - p)..i {
                match rsi[j] {
                    Some(w) if w.is_finite() => {
                        if !(w > v) {
                            l_ok = false;
                        }
                        if !(w < v) {
                            h_ok = false;
                        }
                    }
                    _ => {
                        l_ok = false;
                        h_ok = false;
                    }
                }
            }
            for j in (i + 1)..=(i + p) {
                match rsi[j] {
                    Some(w) if w.is_finite() => {
                        if !(w > v) {
                            l_ok = false;
                        }
                        if !(w < v) {
                            h_ok = false;
                        }
                    }
                    _ => {
                        l_ok = false;
                        h_ok = false;
                    }
                }
            }
            if h_ok {
                highs_idx.push(i);
            }
            if l_ok {
                lows_idx.push(i);
            }
        }
    }
    let mut sig: Vec<DivSignal> = Vec::new();
    for hi in 0..highs_idx.len() {
        let i = highs_idx[hi];
        let rv = rsi[i].unwrap_or(f64::NAN);
        let pv = highs[i];
        let mut k = hi as i64 - 1;
        while k >= 0 {
            let j = highs_idx[k as usize];
            if i as i64 - j as i64 > lookback {
                break;
            }
            let r_ref = rsi[j].unwrap_or(f64::NAN);
            let p_ref = highs[j];
            if r_ref.is_nan() || r_ref == rv {
                k -= 1;
                continue;
            }
            if pv > p_ref && rv < r_ref {
                sig.push(DivSignal { index: i, time: candles[i].time, kind_bull: false, hidden: false });
                break;
            }
            if pv < p_ref && rv > r_ref {
                sig.push(DivSignal { index: i, time: candles[i].time, kind_bull: false, hidden: true });
                break;
            }
            k -= 1;
        }
    }
    for lo in 0..lows_idx.len() {
        let i = lows_idx[lo];
        let rv = rsi[i].unwrap_or(f64::NAN);
        let pv = lows[i];
        let mut k = lo as i64 - 1;
        while k >= 0 {
            let j = lows_idx[k as usize];
            if i as i64 - j as i64 > lookback {
                break;
            }
            let r_ref = rsi[j].unwrap_or(f64::NAN);
            let p_ref = lows[j];
            if r_ref.is_nan() || r_ref == rv {
                k -= 1;
                continue;
            }
            if pv < p_ref && rv > r_ref {
                sig.push(DivSignal { index: i, time: candles[i].time, kind_bull: true, hidden: false });
                break;
            }
            if pv > p_ref && rv < r_ref {
                sig.push(DivSignal { index: i, time: candles[i].time, kind_bull: true, hidden: true });
                break;
            }
            k -= 1;
        }
    }
    sig.sort_by_key(|s| s.index);
    sig
}
