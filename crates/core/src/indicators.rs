use serde_json::json;
use std::collections::BTreeMap;

use crate::engines::*;
use crate::math::*;
use crate::model::*;

// ---------------------------------------------------------------------------
// Builder helpers for catalog definitions
// ---------------------------------------------------------------------------

fn num_in(key: &str, label: &str, def: f64, min: f64, max: f64, step: f64) -> InputDef {
    InputDef {
        key: key.into(),
        label: label.into(),
        kind: "number".into(),
        def: json!(def),
        min: Some(min),
        max: Some(max),
        step: Some(step),
        options: vec![],
    }
}

fn source_in(def: &str) -> InputDef {
    let opts: Vec<InputOption> = [
        ("close", "Close"),
        ("open", "Open"),
        ("high", "High"),
        ("low", "Low"),
        ("hl2", "HL2"),
        ("hlc3", "HLC3"),
        ("hlcc4", "HLCC4"),
    ]
    .iter()
    .map(|(v, l)| InputOption { value: (*v).into(), label: (*l).into() })
    .collect();
    InputDef {
        key: "source".into(),
        label: "Source".into(),
        kind: "source".into(),
        def: json!(def),
        min: None,
        max: None,
        step: None,
        options: opts,
    }
}

fn check_in(key: &str, label: &str, def: bool) -> InputDef {
    InputDef {
        key: key.into(),
        label: label.into(),
        kind: "bool".into(),
        def: json!(def),
        min: None,
        max: None,
        step: None,
        options: vec![],
    }
}

fn enum_in(key: &str, label: &str, def: &str, opts: &[(&str, &str)]) -> InputDef {
    InputDef {
        key: key.into(),
        label: label.into(),
        kind: "enum".into(),
        def: json!(def),
        min: None,
        max: None,
        step: None,
        options: opts
            .iter()
            .map(|(v, l)| InputOption { value: (*v).into(), label: (*l).into() })
            .collect(),
    }
}

fn color_st(key: &str, label: &str, def: &str) -> StyleDef {
    StyleDef {
        key: key.into(),
        label: label.into(),
        kind: "color".into(),
        def: json!(def),
        min: None,
        max: None,
        step: None,
    }
}

fn num_st(key: &str, label: &str, def: f64, min: f64, max: f64, step: f64) -> StyleDef {
    StyleDef {
        key: key.into(),
        label: label.into(),
        kind: "number".into(),
        def: json!(def),
        min: Some(min),
        max: Some(max),
        step: Some(step),
    }
}

fn bool_st(key: &str, label: &str, def: bool) -> StyleDef {
    StyleDef {
        key: key.into(),
        label: label.into(),
        kind: "bool".into(),
        def: json!(def),
        min: None,
        max: None,
        step: None,
    }
}

fn def(
    id: &str,
    name: &str,
    full: &str,
    cat: &str,
    kind: IndType,
    format: Option<&str>,
    inputs: Vec<InputDef>,
    style: Vec<StyleDef>,
) -> IndicatorDef {
    IndicatorDef {
        id: id.into(),
        name: name.into(),
        full_name: full.into(),
        cat: cat.into(),
        kind,
        format: format.map(|s| s.into()),
        hidden: false,
        inputs,
        style,
    }
}

fn def_hidden(
    id: &str,
    name: &str,
    full: &str,
    cat: &str,
    kind: IndType,
    format: Option<&str>,
    inputs: Vec<InputDef>,
    style: Vec<StyleDef>,
) -> IndicatorDef {
    let mut d = def(id, name, full, cat, kind, format, inputs, style);
    d.hidden = true;
    d
}

macro_rules! e {
    ($def:expr, $compute:expr, $reading:expr) => {
        IndicatorEntry { def: $def, compute: $compute, markers: None, reading: $reading }
    };
    ($def:expr, $compute:expr, $markers:expr, $reading:expr) => {
        IndicatorEntry { def: $def, compute: $compute, markers: Some($markers), reading: $reading }
    };
}

// ---------------------------------------------------------------------------
// Standard indicator computes
// ---------------------------------------------------------------------------

fn compute_ema(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let color = strv(o, "color", "#2962ff");
    let lw = num(o, "lineWidth", 1.0);
    let src = src_arr(c, &strv(o, "source", "close"));
    let arr = ema_arr(&src, int(o, "length", 9));
    vec![build_series(c, &arr, &color, SeriesKind::Line, lw)]
}

fn compute_ma(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let color = strv(o, "color", "#ff6d00");
    let lw = num(o, "lineWidth", 1.0);
    let src = src_arr(c, &strv(o, "source", "close"));
    let arr = sma_arr(&src, int(o, "length", 20));
    vec![build_series(c, &arr, &color, SeriesKind::Line, lw)]
}

fn compute_smma(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let color = strv(o, "color", "#ffca28");
    let lw = num(o, "lineWidth", 1.0);
    let src = src_arr(c, &strv(o, "source", "close"));
    let arr = wilder_arr(&src, int(o, "length", 20));
    vec![build_series(c, &arr, &color, SeriesKind::Line, lw)]
}

fn compute_hma(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let color = strv(o, "color", "#00bcd4");
    let lw = num(o, "lineWidth", 1.0);
    let len = int(o, "length", 20).max(1);
    let src = src_arr(c, &strv(o, "source", "close"));
    let half = (len / 2).max(1);
    let sq = (len as f64).sqrt().round() as i64;
    let wma = |vals: &[f64], p: i64| -> Vec<Option<f64>> {
        let n = vals.len();
        let mut out = vec![None; n];
        if p <= 0 {
            return out;
        }
        let p = p as usize;
        let denom = (p * (p + 1) / 2) as f64;
        for i in (p - 1)..n {
            let mut s = 0.0;
            for j in 0..p {
                s += vals[i - j] * (p - j) as f64;
            }
            out[i] = Some(s / denom);
        }
        out
    };
    let w1 = wma(&src, half);
    let w2 = wma(&src, len);
    let raw: Vec<Option<f64>> = (0..src.len())
        .map(|i| match (w1[i], w2[i]) {
            (Some(a), Some(b)) => Some(2.0 * a - b),
            _ => None,
        })
        .collect();
    let mut out = vec![None; src.len()];
    let valid: Vec<f64> = raw.iter().map(|v| v.unwrap_or(0.0)).collect();
    let sm = wma(&valid, sq);
    for i in 0..src.len() {
        if raw[i].is_some() {
            out[i] = sm[i];
        }
    }
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_ao(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let med: Vec<f64> = c.iter().map(|x| (x.high + x.low) / 2.0).collect();
    let f = sma_arr(&med, int(o, "fast", 5));
    let s = sma_arr(&med, int(o, "slow", 34));
    let up = strv(o, "upColor", "#26a69a");
    let down = strv(o, "downColor", "#ef5350");
    let mut data = Vec::new();
    let mut prev: Option<f64> = None;
    for i in 0..c.len() {
        if let (Some(a), Some(b)) = (f[i], s[i]) {
            let v = a - b;
            let is_up = prev.map(|p| v >= p).unwrap_or(true);
            data.push(Point { time: c[i].time, value: v, color: Some(if is_up { up.clone() } else { down.clone() }) });
            prev = Some(v);
        }
    }
    let mut s = SeriesOut::hist(&up);
    s.data = data;
    vec![s]
}

fn compute_atr(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let color = strv(o, "color", "#7e57c2");
    let lw = num(o, "lineWidth", 1.0);
    let arr = wilder_arr(&tr_arr(c), int(o, "length", 14));
    vec![build_series(c, &arr, &color, SeriesKind::Line, lw)]
}

fn compute_adx(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let len = int(o, "length", 14).max(1);
    let tr = tr_arr(c);
    let mut up = vec![0.0; n];
    let mut dn = vec![0.0; n];
    for i in 1..n {
        let um = c[i].high - c[i - 1].high;
        let dm = c[i - 1].low - c[i].low;
        up[i] = if um > dm && um > 0.0 { um } else { 0.0 };
        dn[i] = if dm > um && dm > 0.0 { dm } else { 0.0 };
    }
    let sutr = wilder_arr(&tr, len);
    let sup = wilder_arr(&up, len);
    let sdn = wilder_arr(&dn, len);
    let mut di_p = vec![None; n];
    let mut di_m = vec![None; n];
    let mut dx = vec![None; n];
    for i in 0..n {
        if let (Some(t), Some(p), Some(m)) = (sutr[i], sup[i], sdn[i]) {
            if t != 0.0 {
                let dp = 100.0 * p / t;
                let dm = 100.0 * m / t;
                di_p[i] = Some(dp);
                di_m[i] = Some(dm);
                let sum = dp + dm;
                dx[i] = Some(if sum != 0.0 { 100.0 * (dp - dm).abs() / sum } else { 0.0 });
            }
        }
    }
    let dx_filled: Vec<f64> = dx.iter().map(|v| v.unwrap_or(0.0)).collect();
    let adx_arr = wilder_arr(&dx_filled, len);
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &adx_arr, &strv(o, "adxColor", "#e040fb"), SeriesKind::Line, lw),
        build_series(c, &di_p, &strv(o, "diPlusColor", "#26a69a"), SeriesKind::Line, lw),
        build_series(c, &di_m, &strv(o, "diMinusColor", "#ef5350"), SeriesKind::Line, lw),
    ]
}

fn compute_bollinger_b(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20);
    let mult = num(o, "mult", 2.0);
    let cl = src_arr(c, "close");
    let mid = sma_arr(&cl, len);
    let sd = stdev_arr(&cl, len);
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    let mut data = Vec::new();
    for i in 0..c.len() {
        if let (Some(m), Some(s)) = (mid[i], sd[i]) {
            let up = m + mult * s;
            let lo = m - mult * s;
            let range = up - lo;
            data.push(Point { time: c[i].time, value: if range != 0.0 { (cl[i] - lo) / range } else { 0.0 }, color: None });
        }
    }
    let mut s = SeriesOut::line(&color, lw);
    s.data = data;
    vec![s]
}

fn bb_pct_raw(c: &[Candle], len: usize, mult: f64, source: &str) -> Vec<Option<f64>> {
    // Rolling mean/stdev -> Bollinger %B. Shared by the normal pane indicator and
    // the straight-line variant so both stay numerically identical.
    let len = len.max(1);
    let src = src_arr(c, source);
    let n = c.len();
    let mut b = vec![None; n];
    if n >= len {
        let mut sum = 0.0;
        let mut sumsq = 0.0;
        for i in 0..n {
            let v = src[i];
            sum += v;
            sumsq += v * v;
            if i >= len {
                let old = src[i - len];
                sum -= old;
                sumsq -= old * old;
            }
            if i + 1 >= len {
                let mean = sum / len as f64;
                let var0 = (sumsq / len as f64 - mean * mean).max(0.0);
                let sd = var0.sqrt();
                let up = mean + mult * sd;
                let lo = mean - mult * sd;
                let range = up - lo;
                b[i] = Some(if range > 0.0 { (v - lo) / range } else { 0.5 });
            }
        }
    }
    b
}

fn compute_bbpct(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // Matches the old app: rolling mean/stdev, %B, then SMA smoothing of the valid tail.
    let len = int(o, "length", 20).max(1) as usize;
    let mult = if num(o, "mult", 2.0) > 0.0 { num(o, "mult", 2.0) } else { 2.0 };
    let smooth = int(o, "smooth", 1).max(1) as usize;
    let b = bb_pct_raw(c, len, mult, &strv(o, "source", "close"));
    let n = c.len();
    let start = (len - 1).min(n);
    let valid: Vec<f64> = b[start..].iter().map(|v| v.unwrap_or(f64::NAN)).collect();
    let valid_times: Vec<i64> = c[start..].iter().map(|x| x.time).collect();
    let sm = sma_arr_tolerant(&valid, smooth);
    let color = strv(o, "color", "#ffb300");
    let lw = num(o, "lineWidth", 1.0);
    let mut data = Vec::new();
    for i in 0..valid.len() {
        if let Some(v) = sm[i] {
            data.push(Point { time: valid_times[i], value: v, color: None });
        }
    }
    let mut s = SeriesOut::line(&color, lw);
    s.data = data;
    vec![s]
}

/// Per-bar BB%b regime: +1 while %B sits at/above the bull threshold, -1 while
/// below, 0 before %B warms up. A flip is only committed once the old regime has
/// lasted `min_leg` bars, so the trend never flickers through the noise. Shared
/// by the overlay and pane straight-line variants so they turn on the same bar.
fn bbpct_sl_state(b: &[Option<f64>], thr: f64, min_leg: usize) -> Vec<i32> {
    let n = b.len();
    let mut state = vec![0i32; n];
    let min_leg = min_leg.max(1);
    let mut dir = 0i32;
    let mut leg_start = 0usize;
    for i in 0..n {
        if let Some(v) = b[i] {
            if v.is_finite() {
                let s = if v >= thr { 1 } else { -1 };
                if dir == 0 {
                    dir = s;
                    leg_start = i;
                } else if s != dir && i.saturating_sub(leg_start) >= min_leg {
                    dir = s;
                    leg_start = i;
                }
            }
        }
        state[i] = dir;
    }
    state
}

fn compute_bbpctsl(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // BB%b as a single price-trend line: price is held to one straight segment per
    // %B regime (bullish while %B >= threshold, bearish while below). A turn is only
    // committed after `minLeg` bars, so the line shows one clean trend direction and
    // never oscillates through the noise. Its colour comes from its own slope via
    // `color_straight_line` (rising = green, falling = red).
    let len = int(o, "length", 20).max(2) as usize;
    let mult = if num(o, "mult", 2.0) > 0.0 { num(o, "mult", 2.0) } else { 2.0 };
    let thr = num(o, "threshold", 0.5);
    let min_leg = int(o, "minLeg", 5).max(1) as usize;
    let lw = num(o, "lineWidth", 2.0);
    let flat = strv(o, "flatColor", "#6b6b88");
    let b = bb_pct_raw(c, len, mult, &strv(o, "source", "close"));
    let state = bbpct_sl_state(&b, thr, min_leg);
    let n = c.len();
    let mut data: Vec<Point> = Vec::new();
    let mut prev = 0i32;
    for i in 0..n {
        if state[i] != 0 && state[i] != prev {
            data.push(Point { time: c[i].time, value: c[i].close, color: None });
            prev = state[i];
        }
    }
    if let Some(last) = c.last() {
        if data.last().map(|p| p.time) != Some(last.time) {
            data.push(Point { time: last.time, value: last.close, color: None });
        }
    }
    let mut s = SeriesOut::line(&flat, lw);
    s.data = data;
    vec![s]
}

fn compute_bbpctslp(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // Pane twin of `bbpctsl`: the same %B regime held as ONE straight pane trend
    // line - flat until %B warms up, then a constant positive slope while %B is
    // bullish and a constant negative slope while bearish. Because the slope only
    // changes on a committed regime turn, the pane shows a clean up/down trend with
    // no oscillator chop. Green rising / red falling.
    let len = int(o, "length", 20).max(2) as usize;
    let mult = if num(o, "mult", 2.0) > 0.0 { num(o, "mult", 2.0) } else { 2.0 };
    let thr = num(o, "threshold", 0.5);
    let min_leg = int(o, "minLeg", 5).max(1) as usize;
    let step = num(o, "step", 0.02).max(0.0);
    let lw = num(o, "lineWidth", 1.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let flat = strv(o, "flatColor", "#6b6b88");
    let b = bb_pct_raw(c, len, mult, &strv(o, "source", "close"));
    let state = bbpct_sl_state(&b, thr, min_leg);
    let n = c.len();
    let mut level = 0.0f64;
    let mut data: Vec<Point> = Vec::new();
    for i in 0..n {
        if state[i] == 0 {
            continue;
        }
        level += state[i] as f64 * step;
        data.push(Point { time: c[i].time, value: level, color: None });
    }
    color_line_by_slope(&mut data, &up, &dn);
    let mut s = SeriesOut::line(&flat, lw);
    s.data = data;
    vec![s]
}

// ---------------------------------------------------------------------------
// Price Action Diversionless
// ---------------------------------------------------------------------------

/// Pivot-less divergence read at bar `i`: `+1` when price prints a lower low
/// than the recent window while RSI prints a higher low (bullish divergence),
/// `-1` for the mirror bearish case, `0` when there is none. Rolling extremes
/// are used instead of swing pivots, so nothing repaints while a candle forms.
fn padiv_divergence(c: &[Candle], rsi: &[Option<f64>], i: usize, win: usize) -> i32 {
    if i == 0 || win == 0 {
        return 0;
    }
    let start = i.saturating_sub(win);
    let mut lo_p = f64::INFINITY;
    let mut lo_r = f64::INFINITY;
    let mut hi_p = f64::NEG_INFINITY;
    let mut hi_r = f64::NEG_INFINITY;
    let mut have = false;
    for j in start..i {
        if let Some(r) = rsi[j] {
            if r.is_finite() {
                have = true;
                if c[j].low < lo_p {
                    lo_p = c[j].low;
                    lo_r = r;
                }
                if c[j].high > hi_p {
                    hi_p = c[j].high;
                    hi_r = r;
                }
            }
        }
    }
    if !have {
        return 0;
    }
    let r = match rsi[i] {
        Some(r) if r.is_finite() => r,
        _ => return 0,
    };
    if c[i].low <= lo_p && r > lo_r + 1.0 {
        return 1;
    }
    if c[i].high >= hi_p && r < hi_r - 1.0 {
        return -1;
    }
    0
}

/// Per-bar regime of the Price Action Diversionless line: `+1` bull / `-1` bear
/// / `0` unresolved. A flip needs the crossover consensus (EMA fast/slow + slow
/// Supertrend + fast Supertrend) to agree for `confirm` bars, the running leg to
/// have lasted `minLeg` bars, and NO divergence pushing the other way. In-trend
/// divergence and Bollinger %B band pokes therefore never flip the line - only a
/// real reversal does.
fn padiv_state(c: &[Candle], o: &Settings) -> Vec<i32> {
    let n = c.len();
    let mut state = vec![0i32; n];
    if n == 0 {
        return state;
    }
    let source = strv(o, "source", "close");
    let src = src_arr(c, &source);
    let ema_f = ema_arr(&src, int(o, "emaFast", 9).max(1));
    let ema_s = ema_arr(&src, int(o, "emaSlow", 21).max(2));
    let st_period = int(o, "atrPeriod", 10).max(1);
    let st_fast = supertrend_series(c, st_period, num(o, "factorFast", 1.0)).1;
    let st_slow = supertrend_series(c, st_period, num(o, "factorSlow", 2.0)).1;
    let rsi = rsi_wilder(&src, int(o, "rsiLen", 14).max(2));
    let bb = bb_pct_raw(c, int(o, "bbLen", 20).max(2) as usize, num(o, "bbMult", 2.0), &source);
    let confirm = int(o, "confirm", 3).max(1) as usize;
    let min_leg = int(o, "minLeg", 5).max(1) as usize;
    let win = int(o, "divWin", 14).max(2) as usize;

    let mut dir = 0i32;
    let mut leg_start = 0usize;
    let mut pend = 0i32;
    let mut pend_n = 0usize;
    for i in 0..n {
        let mut raw = 0i32;
        if let (Some(f), Some(s)) = (ema_f[i], ema_s[i]) {
            if st_slow[i] != 0 {
                if f > s && st_slow[i] > 0 && st_fast[i] >= 0 {
                    raw = 1;
                } else if f < s && st_slow[i] < 0 && st_fast[i] <= 0 {
                    raw = -1;
                }
            }
        }
        if raw != 0 && raw == pend {
            pend_n += 1;
        } else if raw != 0 {
            pend = raw;
            pend_n = 1;
        } else {
            pend = 0;
            pend_n = 0;
        }

        let div = padiv_divergence(c, &rsi, i, win);
        let fake_up = matches!(bb[i], Some(v) if v > 1.0) && raw <= 0;
        let fake_dn = matches!(bb[i], Some(v) if v < 0.0) && raw >= 0;

        if dir == 0 {
            if pend_n >= confirm {
                dir = pend;
                leg_start = i;
            }
        } else if pend != 0
            && pend != dir
            && pend_n >= confirm
            && i.saturating_sub(leg_start) >= min_leg
        {
            let veto = (pend < 0 && div > 0)
                || (pend > 0 && div < 0)
                || (pend > 0 && fake_up)
                || (pend < 0 && fake_dn);
            if !veto {
                dir = pend;
                leg_start = i;
            }
        }
        state[i] = dir;
    }
    state
}

fn compute_padiv(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // Pane straight line: a constant slope per committed regime, turning only on
    // a confirmed real reversal. Colour is applied by the shared straight-line
    // rule from the same regime, so line and colour can never disagree.
    let step = num(o, "step", 0.02).max(0.0);
    let lw = num(o, "lineWidth", 1.0);
    let flat = strv(o, "flatColor", "#6b6b88");
    let state = padiv_state(c, o);
    let n = c.len();
    let mut level = 0.0f64;
    let mut data: Vec<Point> = Vec::new();
    for i in 0..n {
        if state[i] == 0 {
            continue;
        }
        level += state[i] as f64 * step;
        data.push(Point { time: c[i].time, value: level, color: None });
    }
    color_line_by_slope(&mut data, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"));
    let mut s = SeriesOut::line(&flat, lw);
    s.data = data;
    vec![s]
}

fn compute_padivo(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // Overlay twin: the same confirmed regime held as one straight price line.
    let lw = num(o, "lineWidth", 2.0);
    let flat = strv(o, "flatColor", "#6b6b88");
    let state = padiv_state(c, o);
    let n = c.len();
    let mut data: Vec<Point> = Vec::new();
    let mut prev = 0i32;
    for i in 0..n {
        if state[i] != 0 && state[i] != prev {
            data.push(Point { time: c[i].time, value: c[i].close, color: None });
            prev = state[i];
        }
    }
    if let Some(last) = c.last() {
        if data.last().map(|p| p.time) != Some(last.time) {
            data.push(Point { time: last.time, value: last.close, color: None });
        }
    }
    let mut s = SeriesOut::line(&flat, lw);
    s.data = data;
    vec![s]
}

fn compute_macd(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let cl = src_arr(c, "close");
    let f = ema_arr(&cl, int(o, "fast", 12));
    let s = ema_arr(&cl, int(o, "slow", 26));
    let n = c.len();
    let mut macd = vec![None; n];
    for i in 0..n {
        if let (Some(a), Some(b)) = (f[i], s[i]) {
            macd[i] = Some(a - b);
        }
    }
    let macd_filled: Vec<f64> = macd.iter().map(|v| v.unwrap_or(0.0)).collect();
    let signal = ema_arr(&macd_filled, int(o, "signal", 9));
    let hist_up = strv(o, "histUpColor", "#26a69a");
    let hist_dn = strv(o, "histDownColor", "#ef5350");
    let mut hist = Vec::new();
    let mut prev: Option<f64> = None;
    for i in 0..n {
        if let (Some(m), Some(sig)) = (macd[i], signal[i]) {
            let h = m - sig;
            let up = prev.map(|p| h >= p).unwrap_or(true);
            hist.push(Point { time: c[i].time, value: h, color: Some(if up { hist_up.clone() } else { hist_dn.clone() }) });
            prev = Some(h);
        }
    }
    let macd_color = strv(o, "macdColor", "#2962ff");
    let sig_color = strv(o, "signalColor", "#ff6d00");
    let mut hs = SeriesOut::hist(&hist_up);
    hs.data = hist;
    vec![
        build_series(c, &macd, &macd_color, SeriesKind::Line, 1.0),
        build_series(c, &signal, &sig_color, SeriesKind::Line, 1.0),
        hs,
    ]
}

/// Supertrend line value and trend direction per bar. `None`/`0` until the ATR
/// has warmed up. Shared by the plain Supertrend overlay and the two-Supertrend
/// zigzag, so both use exactly the same band-flip logic.
fn supertrend_series(c: &[Candle], atr_period: i64, factor: f64) -> (Vec<Option<f64>>, Vec<i32>) {
    let atr = wilder_arr(&tr_arr(c), atr_period);
    let n = c.len();
    let mut vals: Vec<Option<f64>> = vec![None; n];
    let mut dirs: Vec<i32> = vec![0; n];
    let mut fu = 0.0f64;
    let mut fl = 0.0f64;
    let mut trend = 1i32;
    let mut started = false;
    for i in 0..n {
        let a = match atr[i] {
            Some(a) => a,
            None => continue,
        };
        let hl2 = (c[i].high + c[i].low) / 2.0;
        let bu = hl2 + factor * a;
        let bl = hl2 - factor * a;
        if !started {
            fu = bu;
            fl = bl;
            started = true;
        } else {
            fu = if bu < fu || c[i - 1].close > fu { bu } else { fu };
            fl = if bl > fl || c[i - 1].close < fl { bl } else { fl };
            if trend == 1 && c[i].close < fl {
                trend = -1;
            } else if trend == -1 && c[i].close > fu {
                trend = 1;
            }
        }
        vals[i] = Some(if trend == 1 { fl } else { fu });
        dirs[i] = trend;
    }
    (vals, dirs)
}

fn compute_supertrend(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let up = strv(o, "upColor", "#26a69a");
    let down = strv(o, "downColor", "#ef5350");
    let (vals, dirs) = supertrend_series(c, int(o, "atrPeriod", 10), num(o, "factor", 3.0));
    let mut data = Vec::new();
    for i in 0..c.len() {
        if let Some(v) = vals[i] {
            data.push(Point {
                time: c[i].time,
                value: v,
                color: Some(if dirs[i] == 1 { up.clone() } else { down.clone() }),
            });
        }
    }
    let mut s = SeriesOut::line(&up, num(o, "lineWidth", 1.0));
    s.data = data;
    vec![s]
}

/// Two Supertrends (ATR 10 factor 1 and factor 0.8 by default) averaged into a
/// single line, then joined by straight zigzag legs through its swing turns:
/// green while the combined trend is rising, red while falling. Drawn as one
/// line - the two Supertrends are never plotted separately.
fn compute_stzigzag(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let flat = strv(o, "flatColor", "#6b6b88");
    let lw = num(o, "lineWidth", 2.0);
    let period = int(o, "atrPeriod", 10).max(1);
    let (fast, _) = supertrend_series(c, period, num(o, "factorFast", 1.0));
    let (slow, _) = supertrend_series(c, period, num(o, "factorSlow", 2.0));
    let n = c.len();
    let agg: Vec<Option<f64>> = (0..n)
        .map(|i| match (fast[i], slow[i]) {
            (Some(a), Some(b)) => Some((a + b) * 0.5),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        })
        .collect();
    let half = int(o, "swing", 5).max(1) as usize;
    let min_leg = int(o, "minLeg", 5).max(1) as usize;
    let vts = zigzag_vertices(&agg, &[], half, min_leg, 0);
    let vals: Vec<f64> = vts
        .iter()
        .map(|&i| agg.get(i).copied().flatten().unwrap_or(f64::NAN))
        .collect();
    vec![pivot_zigzag_line(c, &vts, &vals, &up, &dn, &flat, lw)]
}

fn compute_obv(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let color = strv(o, "color", "#26a69a");
    let lw = num(o, "lineWidth", 1.0);
    let n = c.len();
    let mut obv = vec![Some(0.0); n];
    for i in 1..n {
        let prev = obv[i - 1].unwrap();
        let v = if c[i].close > c[i - 1].close {
            prev + c[i].volume
        } else if c[i].close < c[i - 1].close {
            prev - c[i].volume
        } else {
            prev
        };
        obv[i] = Some(v);
    }
    vec![build_series(c, &obv, &color, SeriesKind::Line, lw)]
}

fn compute_bb(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20);
    let mult = num(o, "mult", 2.0);
    let cl = src_arr(c, "close");
    let mid = sma_arr(&cl, len);
    let sd = stdev_arr(&cl, len);
    let n = c.len();
    let mut up = vec![None; n];
    let mut lo = vec![None; n];
    for i in 0..n {
        if let (Some(m), Some(s)) = (mid[i], sd[i]) {
            up[i] = Some(m + mult * s);
            lo[i] = Some(m - mult * s);
        }
    }
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &up, &color, SeriesKind::Line, lw),
        build_series(c, &mid, &color, SeriesKind::Line, lw),
        build_series(c, &lo, &color, SeriesKind::Line, lw),
    ]
}

fn compute_bbw(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20);
    let mult = num(o, "mult", 2.0);
    let cl = src_arr(c, "close");
    let mid = sma_arr(&cl, len);
    let sd = stdev_arr(&cl, len);
    let n = c.len();
    let mut out = vec![None; n];
    for i in 0..n {
        if let (Some(m), Some(s)) = (mid[i], sd[i]) {
            if m != 0.0 {
                out[i] = Some((2.0 * mult * s) / m * 100.0);
            }
        }
    }
    let color = strv(o, "color", "#ffb300");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_volosc(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let fast = int(o, "fast", 5);
    let slow = int(o, "slow", 20);
    let vol: Vec<f64> = c.iter().map(|x| x.volume).collect();
    let f = sma_arr(&vol, fast);
    let s = sma_arr(&vol, slow);
    let n = c.len();
    let mut out = vec![None; n];
    for i in 0..n {
        if let (Some(a), Some(b)) = (f[i], s[i]) {
            if b != 0.0 {
                out[i] = Some((a - b) / b * 100.0);
            }
        }
    }
    let color = strv(o, "color", "#26a69a");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_ad(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let mut ad = vec![Some(0.0); n];
    for i in 0..n {
        let range = c[i].high - c[i].low;
        let mfm = if range != 0.0 {
            ((c[i].close - c[i].low) - (c[i].high - c[i].close)) / range
        } else {
            0.0
        };
        let prev = if i == 0 { 0.0 } else { ad[i - 1].unwrap() };
        ad[i] = Some(prev + mfm * c[i].volume);
    }
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &ad, &color, SeriesKind::Line, lw)]
}

fn compute_mfi(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let p = int(o, "length", 14).max(1) as usize;
    let n = c.len();
    let tp: Vec<f64> = c.iter().map(|x| (x.high + x.low + x.close) / 3.0).collect();
    let mut out = vec![None; n];
    for i in 0..n {
        if i + 1 < p {
            continue;
        }
        let mut pos = 0.0;
        let mut neg = 0.0;
        for j in (i + 1 - p)..=i {
            if j == 0 {
                continue;
            }
            let mf = tp[j] * c[j].volume;
            if tp[j] > tp[j - 1] {
                pos += mf;
            } else if tp[j] < tp[j - 1] {
                neg += mf;
            }
        }
        out[i] = Some(if neg != 0.0 { 100.0 - 100.0 / (1.0 + pos / neg) } else { 100.0 });
    }
    let color = strv(o, "color", "#ff6d00");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_pvt(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let mut out = vec![Some(0.0); n];
    for i in 1..n {
        let prev = out[i - 1].unwrap();
        let change = if c[i - 1].close != 0.0 {
            (c[i].close - c[i - 1].close) / c[i - 1].close
        } else {
            0.0
        };
        out[i] = Some(prev + change * c[i].volume);
    }
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_dpo(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 21);
    let cl = src_arr(c, "close");
    let sm = sma_arr(&cl, len);
    let shift = len / 2 + 1;
    let n = c.len();
    let mut out = vec![None; n];
    for i in 0..n {
        let j = i as i64 - shift;
        if j >= 0 {
            if let Some(s) = sm[j as usize] {
                out[i] = Some(cl[i] - s);
            }
        }
    }
    let color = strv(o, "color", "#7e57c2");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_ppo(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let cl = src_arr(c, "close");
    let f = ema_arr(&cl, int(o, "fast", 12));
    let s = ema_arr(&cl, int(o, "slow", 26));
    let n = c.len();
    let mut ppo = vec![None; n];
    for i in 0..n {
        if let (Some(a), Some(b)) = (f[i], s[i]) {
            if b != 0.0 {
                ppo[i] = Some((a - b) / b * 100.0);
            }
        }
    }
    let ppo_filled: Vec<f64> = ppo.iter().map(|v| v.unwrap_or(0.0)).collect();
    let signal = ema_arr(&ppo_filled, int(o, "signal", 9));
    let n2 = c.len();
    let mut hist = Vec::new();
    let mut prev: Option<f64> = None;
    let hist_up = strv(o, "histUpColor", "#26a69a");
    let hist_dn = strv(o, "histDownColor", "#ef5350");
    for i in 0..n2 {
        if let (Some(p), Some(sig)) = (ppo[i], signal[i]) {
            let h = p - sig;
            let up = prev.map(|x| h >= x).unwrap_or(true);
            hist.push(Point { time: c[i].time, value: h, color: Some(if up { hist_up.clone() } else { hist_dn.clone() }) });
            prev = Some(h);
        }
    }
    let ppo_color = strv(o, "ppoColor", "#2962ff");
    let sig_color = strv(o, "signalColor", "#ff6d00");
    let mut hs = SeriesOut::hist(&hist_up);
    hs.data = hist;
    vec![
        build_series(c, &ppo, &ppo_color, SeriesKind::Line, 1.0),
        build_series(c, &signal, &sig_color, SeriesKind::Line, 1.0),
        hs,
    ]
}

fn compute_williams_r(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 14);
    let n = c.len();
    let high: Vec<f64> = c.iter().map(|x| x.high).collect();
    let low: Vec<f64> = c.iter().map(|x| x.low).collect();
    let hh = highest_arr(&high, len);
    let ll = lowest_arr(&low, len);
    let mut out = vec![None; n];
    for i in 0..n {
        if let (Some(h), Some(l)) = (hh[i], ll[i]) {
            if h != l {
                out[i] = Some((h - c[i].close) / (h - l) * -100.0);
            }
        }
    }
    let color = strv(o, "color", "#e040fb");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_rsi(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 14).max(1) as usize;
    let src = src_arr(c, &strv(o, "source", "close"));
    let n = src.len();
    let mut out = vec![None; n];
    if n > len {
        let mut gain = 0.0;
        let mut loss = 0.0;
        for i in 1..=len {
            let d = src[i] - src[i - 1];
            if d >= 0.0 {
                gain += d;
            } else {
                loss -= d;
            }
        }
        let mut ag = gain / len as f64;
        let mut al = loss / len as f64;
        out[len] = Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) });
        for i in (len + 1)..n {
            let d = src[i] - src[i - 1];
            let g = if d > 0.0 { d } else { 0.0 };
            let l = if d < 0.0 { -d } else { 0.0 };
            ag = (ag * (len as f64 - 1.0) + g) / len as f64;
            al = (al * (len as f64 - 1.0) + l) / len as f64;
            out[i] = Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) });
        }
    }
    let color = strv(o, "color", "#7e57c2");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_uo(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let p1 = int(o, "fast", 7) as usize;
    let p2 = int(o, "mid", 14) as usize;
    let p3 = int(o, "slow", 28) as usize;
    let n = c.len();
    let mut bp = vec![0.0; n];
    let mut tr = vec![0.0; n];
    for i in 0..n {
        if i == 0 {
            continue;
        }
        let true_low = c[i].low.min(c[i - 1].close);
        let true_high = c[i].high.max(c[i - 1].close);
        bp[i] = c[i].close - true_low;
        tr[i] = true_high - true_low;
    }
    let sum = |arr: &[f64], i: usize, p: usize| -> f64 {
        let mut s = 0.0;
        for j in (i + 1 - p)..=i {
            s += arr[j];
        }
        s
    };
    let mut out = vec![None; n];
    for i in 0..n {
        if i + 1 < p3 {
            continue;
        }
        let a1 = sum(&bp, i, p1) / sum(&tr, i, p1).max(f64::EPSILON);
        let a2 = sum(&bp, i, p2) / sum(&tr, i, p2).max(f64::EPSILON);
        let a3 = sum(&bp, i, p3) / sum(&tr, i, p3).max(f64::EPSILON);
        out[i] = Some(100.0 * (4.0 * a1 + 2.0 * a2 + a3) / 7.0);
    }
    let color = strv(o, "color", "#ff6d00");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

/// Rolling window (bars) of the VWAP. A cumulative average (whole window or a
/// single session) sits far above/below price for hours on a trending day; the
/// 20-bar rolling VWAP stays next to price so it hugs the trend and its
/// crossovers are meaningful on a 1-minute chart.
const VWAP_WINDOW: usize = 20;

/// Volume-weighted average of the last `window` bars (typical price x volume).
/// Bars with no usable volume are skipped; when the whole window has none the
/// bar's typical price is used instead. Shared by the chart `vwap` overlay, the
/// Master Line level bias and the server VWAP filters so all three agree.
fn rolling_vwap(cc: &[Candle], window: usize) -> Vec<f64> {
    let window = window.max(1);
    let mut out = vec![0.0f64; cc.len()];
    for i in 0..cc.len() {
        let lo = (i + 1).saturating_sub(window);
        let mut pv = 0.0f64;
        let mut vv = 0.0f64;
        for c in &cc[lo..=i] {
            let tp = (c.high + c.low + c.close) / 3.0;
            let v = if c.volume.is_finite() && c.volume > 0.0 { c.volume } else { 0.0 };
            pv += tp * v;
            vv += v;
        }
        let c = &cc[i];
        out[i] = if vv > 0.0 { pv / vv } else { (c.high + c.low + c.close) / 3.0 };
    }
    out
}

fn compute_vwap(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let vals = rolling_vwap(c, VWAP_WINDOW);
    let out: Vec<Option<f64>> = vals.into_iter().map(Some).collect();
    let color = strv(o, "color", "#ffb300");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_pc(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20);
    let high: Vec<f64> = c.iter().map(|x| x.high).collect();
    let low: Vec<f64> = c.iter().map(|x| x.low).collect();
    let hh = highest_arr(&high, len);
    let ll = lowest_arr(&low, len);
    let color = strv(o, "color", "#26a69a");
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &hh, &color, SeriesKind::Line, lw),
        build_series(c, &ll, &color, SeriesKind::Line, lw),
    ]
}

fn compute_cmf(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20).max(1) as usize;
    let n = c.len();
    let mut mfv = vec![0.0; n];
    for i in 0..n {
        let range = c[i].high - c[i].low;
        let mfm = if range != 0.0 {
            ((c[i].close - c[i].low) - (c[i].high - c[i].close)) / range
        } else {
            0.0
        };
        mfv[i] = mfm * c[i].volume;
    }
    let mut out = vec![None; n];
    for i in 0..n {
        if i + 1 < len {
            continue;
        }
        let mut sum_mfv = 0.0;
        let mut sum_v = 0.0;
        for j in (i + 1 - len)..=i {
            sum_mfv += mfv[j];
            sum_v += c[j].volume;
        }
        out[i] = Some(if sum_v != 0.0 { sum_mfv / sum_v } else { 0.0 });
    }
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_cci(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20).max(1) as usize;
    let n = c.len();
    let tp: Vec<f64> = c.iter().map(|x| (x.high + x.low + x.close) / 3.0).collect();
    let ma = sma_arr(&tp, len as i64);
    let mut out = vec![None; n];
    for i in 0..n {
        if let Some(m) = ma[i] {
            let mut dev = 0.0;
            for j in (i + 1 - len)..=i {
                dev += (tp[j] - m).abs();
            }
            dev /= len as f64;
            out[i] = Some(if dev != 0.0 { (tp[i] - m) / (0.015 * dev) } else { 0.0 });
        }
    }
    let color = strv(o, "color", "#e040fb");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_aroon(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 14).max(1) as usize;
    let n = c.len();
    let mut up = vec![None; n];
    let mut down = vec![None; n];
    for i in 0..n {
        if i + 1 < len {
            continue;
        }
        let start = i + 1 - len;
        let mut hi_idx = start;
        let mut lo_idx = start;
        for j in start..=i {
            if c[j].high >= c[hi_idx].high {
                hi_idx = j;
            }
            if c[j].low <= c[lo_idx].low {
                lo_idx = j;
            }
        }
        up[i] = Some((len as f64 - (i - hi_idx) as f64) / len as f64 * 100.0);
        down[i] = Some((len as f64 - (i - lo_idx) as f64) / len as f64 * 100.0);
    }
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &up, &strv(o, "upColor", "#26a69a"), SeriesKind::Line, lw),
        build_series(c, &down, &strv(o, "downColor", "#ef5350"), SeriesKind::Line, lw),
    ]
}

fn compute_vortex(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 14).max(1) as usize;
    let n = c.len();
    let mut vm_plus = vec![0.0; n];
    let mut vm_minus = vec![0.0; n];
    let mut tr = vec![0.0; n];
    for i in 1..n {
        vm_plus[i] = (c[i].high - c[i - 1].low).abs();
        vm_minus[i] = (c[i].low - c[i - 1].high).abs();
        tr[i] = (c[i].high - c[i].low)
            .max((c[i].high - c[i - 1].close).abs())
            .max((c[i].low - c[i - 1].close).abs());
    }
    let mut vi_p = vec![None; n];
    let mut vi_m = vec![None; n];
    for i in 0..n {
        if i + 1 < len {
            continue;
        }
        let mut sp = 0.0;
        let mut sm = 0.0;
        let mut st = 0.0;
        for j in (i + 1 - len)..=i {
            sp += vm_plus[j];
            sm += vm_minus[j];
            st += tr[j];
        }
        if st != 0.0 {
            vi_p[i] = Some(sp / st);
            vi_m[i] = Some(sm / st);
        }
    }
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &vi_p, &strv(o, "plusColor", "#26a69a"), SeriesKind::Line, lw),
        build_series(c, &vi_m, &strv(o, "minusColor", "#ef5350"), SeriesKind::Line, lw),
    ]
}

fn compute_tsi(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let long = int(o, "long", 25);
    let short = int(o, "short", 13);
    let n = c.len();
    let mut mom = vec![None; n];
    let mut abs_mom = vec![None; n];
    for i in 1..n {
        let m = c[i].close - c[i - 1].close;
        mom[i] = Some(m);
        abs_mom[i] = Some(m.abs());
    }
    let mom_filled: Vec<f64> = mom.iter().map(|v| v.unwrap_or(0.0)).collect();
    let abs_filled: Vec<f64> = abs_mom.iter().map(|v| v.unwrap_or(0.0)).collect();
    let e1 = ema_arr(&mom_filled, long);
    let e2 = ema_arr(&abs_filled, long);
    let s1 = ema_arr(&e1.iter().map(|v| v.unwrap_or(0.0)).collect::<Vec<_>>(), short);
    let s2 = ema_arr(&e2.iter().map(|v| v.unwrap_or(0.0)).collect::<Vec<_>>(), short);
    let mut out = vec![None; n];
    for i in 0..n {
        if e1[i].is_some() && e2[i].is_some() {
            if let (Some(a), Some(b)) = (s1[i], s2[i]) {
                if b != 0.0 {
                    out[i] = Some(100.0 * a / b);
                }
            }
        }
    }
    let color = strv(o, "color", "#2962ff");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_donchian(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20);
    let high: Vec<f64> = c.iter().map(|x| x.high).collect();
    let low: Vec<f64> = c.iter().map(|x| x.low).collect();
    let hh = highest_arr(&high, len);
    let ll = lowest_arr(&low, len);
    let n = c.len();
    let mut mid = vec![None; n];
    for i in 0..n {
        if let (Some(h), Some(l)) = (hh[i], ll[i]) {
            mid[i] = Some((h + l) / 2.0);
        }
    }
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &hh, &color, SeriesKind::Line, lw),
        build_series(c, &mid, &color, SeriesKind::Line, lw),
        build_series(c, &ll, &color, SeriesKind::Line, lw),
    ]
}

fn compute_stochrsi(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let rsi_len = int(o, "rsiLength", 14).max(1) as usize;
    let stoch_len = int(o, "stochLength", 14).max(1) as usize;
    let k_len = int(o, "k", 3).max(1) as usize;
    let d_len = int(o, "d", 3).max(1) as usize;
    let cl = src_arr(c, "close");
    let n = cl.len();
    // RSI
    let mut rsi = vec![None; n];
    if n > rsi_len {
        let mut gain = 0.0;
        let mut loss = 0.0;
        for i in 1..=rsi_len {
            let d = cl[i] - cl[i - 1];
            if d >= 0.0 {
                gain += d;
            } else {
                loss -= d;
            }
        }
        let mut ag = gain / rsi_len as f64;
        let mut al = loss / rsi_len as f64;
        rsi[rsi_len] = Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) });
        for i in (rsi_len + 1)..n {
            let d = cl[i] - cl[i - 1];
            let g = if d > 0.0 { d } else { 0.0 };
            let l = if d < 0.0 { -d } else { 0.0 };
            ag = (ag * (rsi_len as f64 - 1.0) + g) / rsi_len as f64;
            al = (al * (rsi_len as f64 - 1.0) + l) / rsi_len as f64;
            rsi[i] = Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) });
        }
    }
    let rsi_filled: Vec<f64> = rsi.iter().map(|v| v.unwrap_or(0.0)).collect();
    let hh = highest_arr(&rsi_filled, stoch_len as i64);
    let ll = lowest_arr(&rsi_filled, stoch_len as i64);
    let mut raw_k = vec![None; n];
    for i in 0..n {
        if rsi[i].is_some() {
            if let (Some(h), Some(l)) = (hh[i], ll[i]) {
                raw_k[i] = Some(if h != l { (rsi_filled[i] - l) / (h - l) * 100.0 } else { 0.0 });
            }
        }
    }
    let raw_k_filled: Vec<f64> = raw_k.iter().map(|v| v.unwrap_or(0.0)).collect();
    let k_arr = sma_arr(&raw_k_filled, k_len as i64);
    let k_filled: Vec<f64> = k_arr.iter().map(|v| v.unwrap_or(0.0)).collect();
    let d_arr = sma_arr(&k_filled, d_len as i64);
    let mut k_out = vec![None; n];
    let mut d_out = vec![None; n];
    for i in 0..n {
        if raw_k[i].is_some() {
            k_out[i] = k_arr[i];
        }
        if k_arr[i].is_some() {
            d_out[i] = d_arr[i];
        }
    }
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &k_out, &strv(o, "kColor", "#2962ff"), SeriesKind::Line, lw),
        build_series(c, &d_out, &strv(o, "dColor", "#ff6d00"), SeriesKind::Line, lw),
    ]
}

fn compute_elderforce(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 13).max(1);
    let n = c.len();
    let mut raw = vec![None; n];
    for i in 1..n {
        raw[i] = Some((c[i].close - c[i - 1].close) * c[i].volume);
    }
    let filled: Vec<f64> = raw.iter().map(|v| v.unwrap_or(0.0)).collect();
    let ema = ema_arr(&filled, len);
    let mut out = vec![None; n];
    for i in 0..n {
        if raw[i].is_some() {
            out[i] = ema[i];
        }
    }
    let color = strv(o, "color", "#7e57c2");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_keltner(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20);
    let mult = num(o, "mult", 2.0);
    let cl = src_arr(c, "close");
    let mid = ema_arr(&cl, len);
    let atr = wilder_arr(&tr_arr(c), len);
    let n = c.len();
    let mut up = vec![None; n];
    let mut lo = vec![None; n];
    for i in 0..n {
        if let (Some(m), Some(a)) = (mid[i], atr[i]) {
            up[i] = Some(m + mult * a);
            lo[i] = Some(m - mult * a);
        }
    }
    let color = strv(o, "color", "#42a5f5");
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &up, &color, SeriesKind::Line, lw),
        build_series(c, &mid, &color, SeriesKind::Line, lw),
        build_series(c, &lo, &color, SeriesKind::Line, lw),
    ]
}

fn compute_chandelier(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 22);
    let mult = num(o, "mult", 3.0);
    let high: Vec<f64> = c.iter().map(|x| x.high).collect();
    let low: Vec<f64> = c.iter().map(|x| x.low).collect();
    let hh = highest_arr(&high, len);
    let ll = lowest_arr(&low, len);
    let atr = wilder_arr(&tr_arr(c), len);
    let n = c.len();
    let mut long = vec![None; n];
    let mut short = vec![None; n];
    for i in 0..n {
        if let (Some(h), Some(l), Some(a)) = (hh[i], ll[i], atr[i]) {
            long[i] = Some(h - mult * a);
            short[i] = Some(l + mult * a);
        }
    }
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &long, &strv(o, "longColor", "#26a69a"), SeriesKind::Line, lw),
        build_series(c, &short, &strv(o, "shortColor", "#ef5350"), SeriesKind::Line, lw),
    ]
}

fn compute_sqzmom(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 20) as usize;
    let mult = num(o, "mult", 2.0);
    let cl = src_arr(c, "close");
    let n = c.len();
    let basis = sma_arr(&cl, len as i64);
    let dev = stdev_arr(&cl, len as i64);
    let atr = wilder_arr(&tr_arr(c), len as i64);
    let mut upper = vec![None; n];
    let mut lower = vec![None; n];
    let mut sqz = vec![false; n];
    for i in 0..n {
        if let (Some(b), Some(d), Some(a)) = (basis[i], dev[i], atr[i]) {
            upper[i] = Some(b + mult * d);
            lower[i] = Some(b - mult * d);
            let bb_up = b + mult * d;
            let bb_lo = b - mult * d;
            let kc_up = b + mult * a;
            let kc_lo = b - mult * a;
            sqz[i] = bb_up < kc_up && bb_lo > kc_lo;
        }
    }
    // momentum = linreg of (close - avg of donchian mid & sma)
    let high: Vec<f64> = c.iter().map(|x| x.high).collect();
    let low: Vec<f64> = c.iter().map(|x| x.low).collect();
    let hh = highest_arr(&high, len as i64);
    let ll = lowest_arr(&low, len as i64);
    let mut out = vec![None; n];
    for i in 0..n {
        if let (Some(b), Some(h), Some(l)) = (basis[i], hh[i], ll[i]) {
            let avg = (h + l) / 2.0;
            let src = cl[i] - (avg + b) / 2.0;
            // linear regression value at end over `len` window
            let start = i + 1 - len;
            let mut sum_x = 0.0;
            let mut sum_y = 0.0;
            let mut sum_xy = 0.0;
            let mut sum_xx = 0.0;
            for (k, j) in (start..=i).enumerate() {
                let x = k as f64;
                let y = cl[j] - {
                    let bj = basis[j].unwrap_or(b);
                    let hj = hh[j].unwrap_or(h);
                    let lj = ll[j].unwrap_or(l);
                    ((hj + lj) / 2.0 + bj) / 2.0
                };
                sum_x += x;
                sum_y += y;
                sum_xy += x * y;
                sum_xx += x * x;
            }
            let _ = src;
            let denom = len as f64 * sum_xx - sum_x * sum_x;
            if denom != 0.0 {
                let slope = (len as f64 * sum_xy - sum_x * sum_y) / denom;
                let intercept = (sum_y - slope * sum_x) / len as f64;
                out[i] = Some(intercept + slope * (len as f64 - 1.0));
            }
        }
    }
    let _ = sqz;
    let color = strv(o, "color", "#7e57c2");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_fisher(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let len = int(o, "length", 9).max(1) as usize;
    let n = c.len();
    let mut out = vec![None; n];
    let mut value = 0.0f64;
    let mut fish = 0.0f64;
    let mut started = false;
    for i in 0..n {
        if i + 1 < len {
            continue;
        }
        let start = i + 1 - len;
        let mut hi = f64::NEG_INFINITY;
        let mut lo = f64::INFINITY;
        for j in start..=i {
            hi = hi.max((c[j].high + c[j].low) / 2.0);
            lo = lo.min((c[j].high + c[j].low) / 2.0);
        }
        let hl2 = (c[i].high + c[i].low) / 2.0;
        let mut v = if hi != lo { 0.66 * ((hl2 - lo) / (hi - lo) - 0.5) + 0.67 * value } else { 0.0 };
        v = v.clamp(-0.999, 0.999);
        value = v;
        if !started {
            fish = 0.5 * (1.0 + v).ln() + 0.5 * fish;
            started = true;
        } else {
            fish = 0.5 * (1.0 + v).ln() + 0.5 * fish;
        }
        out[i] = Some(fish);
    }
    let color = strv(o, "color", "#2962ff");
    let lw = num(o, "lineWidth", 1.0);
    vec![build_series(c, &out, &color, SeriesKind::Line, lw)]
}

fn compute_ichimoku(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let tenkan = int(o, "tenkan", 9);
    let kijun = int(o, "kijun", 26);
    let senkou = int(o, "senkou", 52);
    let high: Vec<f64> = c.iter().map(|x| x.high).collect();
    let low: Vec<f64> = c.iter().map(|x| x.low).collect();
    let n = c.len();
    let hh_t = highest_arr(&high, tenkan);
    let ll_t = lowest_arr(&low, tenkan);
    let hh_k = highest_arr(&high, kijun);
    let ll_k = lowest_arr(&low, kijun);
    let hh_s = highest_arr(&high, senkou);
    let ll_s = lowest_arr(&low, senkou);
    let mut tenkan_arr = vec![None; n];
    let mut kijun_arr = vec![None; n];
    let mut span_a = vec![None; n];
    let mut span_b = vec![None; n];
    let disp = kijun as usize;
    for i in 0..n {
        if let (Some(h), Some(l)) = (hh_t[i], ll_t[i]) {
            tenkan_arr[i] = Some((h + l) / 2.0);
        }
        if let (Some(h), Some(l)) = (hh_k[i], ll_k[i]) {
            kijun_arr[i] = Some((h + l) / 2.0);
        }
        if i >= disp {
            if let (Some(a), Some(b)) = (tenkan_arr[i - disp], kijun_arr[i - disp]) {
                span_a[i] = Some((a + b) / 2.0);
            }
            if let (Some(h), Some(l)) = (hh_s[i - disp], ll_s[i - disp]) {
                span_b[i] = Some((h + l) / 2.0);
            }
        }
    }
    let lw = num(o, "lineWidth", 1.0);
    vec![
        build_series(c, &tenkan_arr, &strv(o, "tenkanColor", "#2962ff"), SeriesKind::Line, lw),
        build_series(c, &kijun_arr, &strv(o, "kijunColor", "#ef5350"), SeriesKind::Line, lw),
        build_series(c, &span_a, &strv(o, "spanAColor", "#26a69a"), SeriesKind::Line, lw),
        build_series(c, &span_b, &strv(o, "spanBColor", "#ff6d00"), SeriesKind::Line, lw),
    ]
}

/// SMA that ignores NaN warmup values (used by BB%b).
fn sma_arr_tolerant(vals: &[f64], p: usize) -> Vec<Option<f64>> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p == 0 {
        return out;
    }
    let mut sum = 0.0;
    let mut cnt = 0usize;
    for i in 0..n {
        if vals[i].is_finite() {
            sum += vals[i];
            cnt += 1;
        }
        if i >= p {
            if vals[i - p].is_finite() {
                sum -= vals[i - p];
                cnt -= 1;
            }
        }
        if cnt == p {
            out[i] = Some(sum / p as f64);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Advanced structural / overlay indicators (fresh engines)
// ---------------------------------------------------------------------------

fn opt_to_f64(vals: &[Option<f64>]) -> Vec<f64> {
    vals.iter()
        .map(|v| match v {
            Some(x) if x.is_finite() => *x,
            _ => 0.0,
        })
        .collect()
}

fn last_finite(arr: &[Option<f64>]) -> f64 {
    for v in arr.iter().rev() {
        if let Some(x) = v {
            if x.is_finite() {
                return *x;
            }
        }
    }
    0.0
}

fn ew_to_pivots(p: &[EwPivot]) -> Vec<Pivot> {
    p.iter().map(Pivot::from).collect()
}

fn clean_candles(c: &[Candle]) -> Vec<Candle> {
    let mut out = Vec::new();
    let mut prev_t: i64 = 0;
    for x in c {
        if x.time > prev_t
            && x.time as f64 != 0.0
            && x.high.is_finite()
            && x.low.is_finite()
            && x.close.is_finite()
            && x.open.is_finite()
        {
            out.push(*x);
            prev_t = x.time;
        }
    }
    out
}

fn dense_seg(c: &[Candle], a_idx: usize, a_val: f64, b_idx: usize, b_val: f64) -> Vec<Point> {
    let mut d = Vec::new();
    if b_idx < a_idx || c.is_empty() {
        return d;
    }
    if a_idx == b_idx {
        if let Some(x) = c.get(a_idx) {
            d.push(Point { time: x.time, value: a_val, color: None });
        }
        return d;
    }
    let span = (b_idx - a_idx) as f64;
    for i in a_idx..=b_idx {
        if let Some(x) = c.get(i) {
            let f = (i - a_idx) as f64 / span;
            d.push(Point { time: x.time, value: a_val + (b_val - a_val) * f, color: None });
        }
    }
    d
}

fn mk_line(color: &str, lw: f64, data: Vec<Point>, style: Option<i32>) -> SeriesOut {
    let mut s = SeriesOut::line(color, lw);
    s.data = data;
    s.line_style = style;
    s
}

fn live_trend_dir(cc: &[Candle], len: usize) -> Vec<i32> {
    let n = cc.len();
    let mut out = vec![0i32; n];
    if n == 0 {
        return out;
    }
    let closes: Vec<f64> = cc.iter().map(|x| x.close).collect();
    let slow = ema_arr(&closes, len.max(2) as i64);
    let mut last = 0i32;
    for i in 0..n {
        match slow[i] {
            Some(e) if e.is_finite() => {
                if closes[i] > e {
                    last = 1;
                } else if closes[i] < e {
                    last = -1;
                } else if last == 0 {
                    last = 1;
                }
            }
            _ => {
                if i > 0 {
                    last = if closes[i] >= closes[i - 1] { 1 } else { -1 };
                }
            }
        }
        out[i] = last;
    }
    out
}

fn color_points_by_dir(cc: &[Candle], data: &mut [Point], dir: &[i32], up: &str, dn: &str) {
    for p in data.iter_mut() {
        let idx = match cc.binary_search_by_key(&p.time, |x| x.time) {
            Ok(i) => i,
            Err(_) => continue,
        };
        if idx < dir.len() {
            p.color = Some(if dir[idx] >= 0 { up.to_string() } else { dn.to_string() });
        }
    }
}

/// Indicator ids drawn as straight lines. Every one of these is repainted by
/// [`color_straight_line`] so the line is green while it rises (bullish) and red
/// while it falls (bearish), matching the direction the filters read.
pub fn is_straight_line(id: &str) -> bool {
    matches!(
        id,
        "pastruct"
            | "wavefib"
            | "autotrend"
            | "zzline"
            | "trendmaster"
            | "panemaster"
            | "projline"
            | "srema"
            | "slema"
            | "stzigzag"
            | "slconsensus"
            | "supline"
            | "resline"
            | "pitchfork"
            | "fibfan"
            | "gannfan"
            | "vlcore"
            | "bbpctsl"
            | "padiv"
            | "padivo"
            | "slallconsensus"
            | "slallconsensuso"
    )
}

/// Candles a straight-line / zigzag indicator must be evaluated on so its line
/// cannot move while the trailing candle is still forming. When `forming` is
/// true the newest (forming) bar is dropped, so the line is built only up to the
/// last CLOSED candle and holds still until that candle closes. Every other
/// indicator - and every straight-line indicator on a static / closed series -
/// sees all candles unchanged.
pub fn straight_line_input<'a>(id: &str, candles: &'a [Candle], forming: bool) -> &'a [Candle] {
    if forming && is_straight_line(id) && candles.len() >= 2 {
        &candles[..candles.len() - 1]
    } else {
        candles
    }
}

/// Repaint a straight-line series from its OWN slope: each point is green when it
/// sits above the previous point (the drawn line is rising / bullish) and red
/// when it sits below it (falling / bearish). Flat or unresolved stretches keep
/// whatever neutral colour the indicator already chose.
fn color_line_by_slope(data: &mut [Point], up: &str, dn: &str) {
    let n = data.len();
    if n < 2 {
        return;
    }
    for i in 0..n {
        let (a, b) = if i == 0 {
            (data[0].value, data[1].value)
        } else {
            (data[i - 1].value, data[i].value)
        };
        if !a.is_finite() || !b.is_finite() {
            continue;
        }
        if b > a {
            data[i].color = Some(up.to_string());
        } else if b < a {
            data[i].color = Some(dn.to_string());
        }
    }
}

/// Per-bar direction of one line's own value: `+1` while the latest non-empty
/// bar sits above the one before it, `-1` while below, carried forward so every
/// bar has a direction. Used to read a dense line's own slope causally (e.g. a
/// straight-line EMA).
fn slope_dir_from_vals(vals: &[Option<f64>]) -> Vec<i32> {
    let mut out = vec![0i32; vals.len()];
    let mut last = 0i32;
    for i in 0..vals.len() {
        if let Some(v) = vals[i] {
            if i > 0 {
                if let Some(pv) = vals[i - 1] {
                    if v > pv {
                        last = 1;
                    } else if v < pv {
                        last = -1;
                    }
                }
            }
        }
        out[i] = last;
    }
    out
}

/// Causal trend direction of a straight-line indicator's OWN rule, per bar
/// (`+1` bull / `-1` bear / `0` unresolved). The colour is taken from this
/// instead of the drawn pivot geometry, so the line - and the engine's colour
/// read - flips the moment the indicator's own trend turns rather than waiting
/// for a later zigzag pivot to confirm. Most ids use their price/line EMA cross
/// (`live_trend_dir`, tuned by their `trendLen`); the ids with a stronger
/// built-in regime use that regime directly.
fn straight_line_dir(id: &str, cc: &[Candle], o: &Settings) -> Vec<i32> {
    match id {
        "stzigzag" => {
            let period = int(o, "atrPeriod", 10).max(1);
            let (_, dirs) = supertrend_series(cc, period, num(o, "factorFast", 1.0));
            dirs
        }
        "bbpctsl" => {
            let len = int(o, "length", 20).max(2) as usize;
            let mult = if num(o, "mult", 2.0) > 0.0 { num(o, "mult", 2.0) } else { 2.0 };
            let b = bb_pct_raw(cc, len, mult, &strv(o, "source", "close"));
            bbpct_sl_state(&b, num(o, "threshold", 0.5), int(o, "minLeg", 5).max(1) as usize)
        }
        "padiv" | "padivo" => padiv_state(cc, o),
        "slallconsensus" | "slallconsensuso" => sl_all_consensus_dir(
            cc,
            num(o, "minAgree", 12.0).round().max(1.0) as i32,
            num(o, "confirm", 10.0).round().max(0.0) as usize,
            num(o, "strength", 5.0).round().max(2.0),
            num(o, "trendLen", 34.0).round().max(2.0) as usize,
        ),
        "slconsensus" => sl_consensus_dir(
            cc,
            num(o, "minAgree", 2.0).round().max(1.0) as i32,
            num(o, "confirm", 5.0).round().max(0.0) as usize,
            num(o, "strength", 5.0).round().max(2.0),
        ),
        "slema" => {
            let src = src_arr(cc, &strv(o, "source", "close"));
            slope_dir_from_vals(&ema_arr(&src, int(o, "length", 9).max(1)))
        }
        _ => live_trend_dir(cc, num(o, "trendLen", 9.0).round().max(2.0) as usize),
    }
}

/// Single entry point that enforces consistent straight-line colouring for every
/// straight-line indicator: the line is green while the indicator's OWN causal
/// trend is up (bullish) and red while it is down (bearish). Because the colour
/// follows that trend rule - not the drawn pivot legs - the line flips on the
/// trend-change bar instead of waiting for a later pivot to confirm. Does nothing
/// for any other indicator id.
pub fn color_straight_line(id: &str, outs: &mut [SeriesOut], settings: &Settings, candles: &[Candle]) {
    if !is_straight_line(id) {
        return;
    }
    let up = strv(settings, "upColor", "#26a69a");
    let dn = strv(settings, "downColor", "#ef5350");
    let cc = clean_candles(candles);
    if cc.is_empty() {
        for s in outs.iter_mut() {
            color_line_by_slope(&mut s.data, &up, &dn);
        }
        return;
    }
    let dir = straight_line_dir(id, &cc, settings);
    let carry = dir.last().copied().unwrap_or(1);
    let last_time = cc[cc.len() - 1].time;
    for s in outs.iter_mut() {
        color_points_by_dir(&cc, &mut s.data, &dir, &up, &dn);
        // Forward projections past the last candle keep the current trend colour.
        for p in s.data.iter_mut() {
            if p.time > last_time {
                p.color = Some(if carry < 0 { dn.clone() } else { up.clone() });
            }
        }
    }
}

/// Build the double-line (main + signal) render for a straight-line indicator.
///
/// `base` is the base indicator's already-computed series, its main straight
/// line first. The returned pair is:
///   * the base main STRAIGHT line unchanged, and
///   * a SIGNAL line that rides a smooth, zero-lag band around it - above the
///     line while its own trend is up, below it while the trend is down - and
///     crosses the main line exactly at each trend flip.
///
/// The band is driven by the line's OWN trend (its per-point colour), so unlike
/// an EMA - which lags and sits on the wrong side for the whole first trend at
/// the open and the last trend into the close - the signal is already on the
/// correct side from the very first bar to the very last. The crossover itself
/// is smoothed over `dlEmaLen` bars (a smoothstep fan), so it looks like an EMA
/// crossover but without the lag. The main line is interpolated onto the chart's
/// bar grid first, so even a pivot-only line gets a smooth signal. Any extra
/// base series (channel parallels / fan rays) are dropped; the family is a
/// "double line".
pub fn double_line_series(base: Vec<SeriesOut>, o: &Settings, candles: &[Candle]) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0).max(1.0);
    let mut it = base.into_iter();
    let mut main = match it.next() {
        Some(m) => m,
        None => SeriesOut::line(&strv(o, "dlSignalColor", "#ffffff"), lw),
    };
    main.line_width = Some(lw);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let sig_color = strv(o, "dlSignalColor", "#ffffff");
    let gap_points = strv(o, "dlGapUnit", "percent").eq_ignore_ascii_case("points");
    let gap = num(o, "dlGap", 0.1).max(0.0);
    // Bars over which the crossover fans in and out (the smooth transition).
    let width = num(o, "dlEmaLen", 9.0).round().max(2.0) as usize;

    // Main-line vertices with their own trend side (from the line's colour).
    let mut vt: Vec<i64> = Vec::new();
    let mut vv: Vec<f64> = Vec::new();
    let mut vs: Vec<i32> = Vec::new();
    let mut dir = 1i32;
    for p in main.data.iter() {
        if !p.value.is_finite() {
            continue;
        }
        if let Some(c) = p.color.as_deref() {
            dir = if c.eq_ignore_ascii_case(&up) {
                1
            } else if c.eq_ignore_ascii_case(&dn) {
                -1
            } else {
                dir
            };
        }
        vt.push(p.time);
        vv.push(p.value);
        vs.push(dir);
    }

    let mut sig = SeriesOut::line(&sig_color, lw);
    sig.last_value_visible = false;
    sig.price_line_visible = false;
    if !vt.is_empty() {
        let (t0, tn) = (vt[0], vt[vt.len() - 1]);
        // Interpolate the main line onto the bar grid, carrying its trend side.
        let mut times: Vec<i64> = Vec::new();
        let mut vals: Vec<f64> = Vec::new();
        let mut side: Vec<f64> = Vec::new();
        for c in candles {
            let t = c.time;
            if t < t0 || t > tn {
                continue;
            }
            let idx = match vt.binary_search(&t) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            let idx = idx.min(vt.len() - 1);
            let (a_t, a_v) = (vt[idx], vv[idx]);
            let val = match vt.get(idx + 1) {
                Some(&b_t) if b_t > t && b_t != a_t => {
                    a_v + (vv[idx + 1] - a_v) * ((t - a_t) as f64 / (b_t - a_t) as f64)
                }
                _ => a_v,
            };
            times.push(t);
            vals.push(val);
            side.push(vs[idx] as f64);
        }
        // Smooth the side into a continuous -1..1 lean over `width` bars,
        // centred on each flip, so the crossover passes through zero exactly at
        // the flip. Edge padding keeps the ends at the full side, so the first
        // trend (open) and the last trend (into the close) are never left on the
        // wrong side the way a lagging EMA would leave them.
        let n = vals.len();
        let m = (width / 2).max(1);
        for i in 0..n {
            let mut sum = 0.0;
            let mut cnt = 0.0;
            for j in i as isize - m as isize..=i as isize + m as isize {
                let v = if j < 0 {
                    side[0]
                } else if j as usize >= n {
                    side[n - 1]
                } else {
                    side[j as usize]
                };
                sum += v;
                cnt += 1.0;
            }
            let lean = sum / cnt;
            // Smoothstep: keeps the sign and the zero-crossing, curves the fan.
            let lean = (3.0 * lean - lean * lean * lean) / 2.0;
            let unit = if gap_points { gap } else { vals[i].abs() * gap / 100.0 };
            sig.data.push(Point {
                time: times[i],
                value: vals[i] + lean * unit,
                color: Some(sig_color.clone()),
            });
        }
    }
    vec![main, sig]
}

/// Median spacing of a bar-time series, in the series' own time units, used to
/// turn an elapsed time into a number of bars. Falls back to 1.0 for short or
/// non-increasing inputs.
pub fn bar_interval(times: &[i64]) -> f64 {
    let mut d: Vec<i64> = times.windows(2).map(|w| w[1] - w[0]).filter(|x| *x > 0).collect();
    if d.is_empty() {
        return 1.0;
    }
    d.sort_unstable();
    d[d.len() / 2] as f64
}

/// The "Double Line" family's registry entries are computed by the chart layer
/// (which resolves the base indicator and calls [`double_line_series`]); this
/// stub keeps the catalog entry computable on its own.
fn compute_dl_stub(_c: &[Candle], _o: &Settings) -> Vec<SeriesOut> {
    Vec::new()
}

fn compute_pastruct(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let lw = num(o, "lineWidth", 2.0);
    let s = pa_structure(
        c,
        num(o, "pivotLen", 3.0),
        num(o, "atrLen", 14.0),
        num(o, "atrMult", 0.25),
        boolv(o, "showMarkers", true),
        boolv(o, "markersOnly", false),
        &up,
        &dn,
    );
    let data = if strv(o, "lineMode", "zigzag") == "trail" { s.line } else { s.zig };
    vec![mk_line(&up, lw, data, None)]
}

fn markers_pastruct(c: &[Candle], o: &Settings) -> Vec<Marker> {
    pa_structure(
        c,
        num(o, "pivotLen", 3.0),
        num(o, "atrLen", 14.0),
        num(o, "atrMult", 0.25),
        true,
        boolv(o, "markersOnly", false),
        &strv(o, "upColor", "#26a69a"),
        &strv(o, "downColor", "#ef5350"),
    )
    .markers
}

/// Live-data placeholders: the old app reads window.OITrend / option-chain
/// snapshots for these. In the Rust build the chain feed is not wired yet, so
/// they render the same fixed (empty) series shape and fill in once the feed
/// lands.
fn compute_pcr(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let lw = num(o, "lineWidth", 1.0);
    let mut raw = SeriesOut::line(&strv(o, "pcrColor", "#9e9e9e"), lw);
    raw.point_markers = true;
    raw.last_value_visible = true;
    raw.price_scale_id = Some("pcr".into());
    raw.price_lines = vec![PriceLine {
        price: 1.0,
        color: "#607d8b".into(),
        line_width: 1.0,
        line_style: 2,
        title: "PCR 1.0".into(),
    }];
    let mut fast = SeriesOut::line(&strv(o, "fastColor", "#00d4aa"), lw);
    fast.last_value_visible = false;
    fast.price_scale_id = Some("pcr".into());
    let mut slow = SeriesOut::line(&strv(o, "slowColor", "#ff9800"), lw);
    slow.last_value_visible = false;
    slow.price_scale_id = Some("pcr".into());
    let _ = n;
    vec![raw, fast, slow]
}

fn compute_pcrrail(_c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 1.0);
    let cols = ["#ff5252", "#00d4aa", "#b39ddb", "#4fc3f7", "#4fc3f7"];
    cols.iter().map(|col| SeriesOut::line(col, lw)).collect()
}

fn compute_iv(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let lw = num(o, "lineWidth", 2.0);
    let mut iv = SeriesOut::line(&strv(o, "color", "#e040fb"), lw);
    let mut delta = SeriesOut::line(&strv(o, "deltaColor", "#00bcd4"), lw);
    delta.last_value_visible = false;
    let mut vega = SeriesOut::line(&strv(o, "vegaColor", "#ff6d00"), lw);
    vega.last_value_visible = false;
    let _ = n;
    iv.data = Vec::new();
    vec![iv, delta, vega]
}

fn compute_projline(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 1.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let flat = strv(o, "histColor", "#7ee0ff");
    let empty = || vec![SeriesOut::line(&flat, lw)];
    let cc = clean_candles(c);
    let n = cc.len();
    if n < 3 {
        return empty();
    }
    let atr_per = (num(o, "atrPeriod", 14.0).round() as i64).max(2) as usize;
    let atr_mult = if num(o, "atrMult", 2.0) > 0.0 { num(o, "atrMult", 2.0) } else { 2.0 };
    let min_pct = if num(o, "minPct", 0.15) >= 0.0 { num(o, "minPct", 0.15) } else { 0.15 };
    let piv = atr_zigzag(&cc, atr_per, atr_mult, min_pct);
    if piv.len() < 2 {
        return empty();
    }
    // Segment-trend-wise single zigzag history through the ATR swings.
    let mut vts: Vec<usize> = piv.iter().map(|p| p.idx).collect();
    let mut vals: Vec<f64> = piv.iter().map(|p| p.price).collect();
    // Carry the zigzag to the live bar at the current price (so it never shoots
    // away from the candles).
    let base = (cc[n - 1].high + cc[n - 1].low) * 0.5;
    if *vts.last().unwrap() < n - 1 {
        vts.push(n - 1);
        vals.push(base);
    }
    let mut hist = pivot_zigzag_line(&cc, &vts, &vals, &up, &dn, &flat, lw);
    let last_t = cc[n - 1].time;
    // The carried right-edge point reflects the live trend so its colour matches
    // the bar the trader is looking at, not the (possibly stale) last pivot leg.
    let dir = live_trend_dir(&cc, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    if let Some(p) = hist.data.last_mut() {
        if p.time == last_t {
            p.color = Some(if dir.last().copied().unwrap_or(0) < 0 { dn.clone() } else { up.clone() });
        }
    }
    vec![hist]
}

fn compute_wavefib(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let rg = strv(o, "rangeColor", "#9e9e9e");
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    if a.c.len() < 3 || a.piv.len() < 2 {
        return vec![SeriesOut::line(&rg, lw)];
    }
    let mut zig: Vec<Point> = Vec::new();
    if boolv(o, "showTrend", true) {
        for k in 0..a.piv.len() {
            let nx = a.piv.get(k + 1).unwrap_or(&a.piv[k]);
            let col = if nx.price >= a.piv[k].price { &up } else { &dn };
            zig.push(Point { time: a.c[a.piv[k].idx].time, value: a.piv[k].price, color: Some(col.clone()) });
        }
        if let (Some(last_z), Some(last_c)) = (zig.last(), a.c.last()) {
            if last_c.time > last_z.time {
                let last_p = &a.piv[a.piv.len() - 1];
                let col = if last_c.close >= last_p.price { &up } else { &dn };
                zig.push(Point { time: last_c.time, value: last_c.close, color: Some(col.clone()) });
            }
        }
    }
    let mut price_lines: Vec<PriceLine> = Vec::new();
    if let Some((a2, b2)) = &a.fib {
        if boolv(o, "showFib", false) || boolv(o, "showExt", false) {
            let lvl = |pct: f64| a2.price + (b2.price - a2.price) * (pct / 100.0);
            let fib_color = strv(o, "fibColor", "#ffd54f");
            let ext_color = strv(o, "fibExtColor", "#ff8a65");
            if boolv(o, "showFib", false) {
                for pct in [23.6, 38.2, 50.0, 61.8, 78.6] {
                    price_lines.push(PriceLine {
                        price: lvl(pct),
                        color: fib_color.clone(),
                        line_width: 1.0,
                        line_style: 3,
                        title: format!("{:.1}%", pct),
                    });
                }
            }
            if boolv(o, "showExt", false) {
                for pct in [100.0, 127.2, 161.8, 200.0] {
                    price_lines.push(PriceLine {
                        price: lvl(pct),
                        color: ext_color.clone(),
                        line_width: 1.0,
                        line_style: 2,
                        title: format!("{:.1}%", pct),
                    });
                }
            }
        }
    }
    let mut s = mk_line(&up, lw, zig, None);
    s.price_lines = price_lines;
    vec![s]
}

fn markers_wavefib(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    if !boolv(o, "showLabels", true) {
        return Vec::new();
    }
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    a.labels
        .iter()
        .filter_map(|l| a.c.get(l.idx).map(|x| Marker {
            time: x.time,
            position: if l.is_high { "aboveBar" } else { "belowBar" }.into(),
            color: if l.is_high { dn.clone() } else { up.clone() },
            shape: "circle".into(),
            text: l.text.clone(),
            size: 1.0,
        }))
        .collect()
}

fn score_line(c: &[Candle], state: &[i32], color: &str) -> Vec<SeriesOut> {
    let mut score = 0i64;
    let mut data = Vec::new();
    for (k, st) in state.iter().enumerate() {
        score += *st as i64;
        if let Some(x) = c.get(k) {
            data.push(Point { time: x.time, value: score as f64, color: None });
        }
    }
    vec![mk_line(color, 1.0, data, None)]
}

fn compute_ewtrend(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 6.0), num(o, "minPct", 0.15));
    score_line(&a.c, &a.trend_state, "#ffb300")
}

fn compute_patrend(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let s = pa_structure(
        c,
        num(o, "pivotLen", 10.0),
        num(o, "atrLen", 14.0),
        num(o, "atrMult", 0.25),
        false,
        false,
        "#26a69a",
        "#ef5350",
    );
    score_line(c, &s.trend_state, "#26a69a")
}

fn compute_keylevel(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let cc = clean_candles(c);
    if cc.len() < 10 {
        return vec![SeriesOut::line("#000000", lw)];
    }
    let piv = fractal_pivots(&cc, num(o, "strength", 5.0));
    if piv.len() < 3 {
        return vec![SeriesOut::line("#000000", lw)];
    }
    let atr_per = (num(o, "atrPeriod", 14.0).round() as i64).max(2);
    let atr = wilder_arr(&tr_arr(&cc), atr_per);
    let atr_last = last_finite(&atr);
    let last_close = cc[cc.len() - 1].close;
    let min_pct = if num(o, "minPct", 0.08) >= 0.0 { num(o, "minPct", 0.08) } else { 0.08 };
    let mut tol = (atr_last * if num(o, "tolMult", 0.6) > 0.0 { num(o, "tolMult", 0.6) } else { 0.6 })
        .max(last_close.abs() * (min_pct / 100.0));
    if !(tol > 0.0) {
        tol = if last_close.abs() * 0.001 != 0.0 { last_close.abs() * 0.001 } else { 1.0 };
    }
    let mut cl = cluster_levels(&piv, tol);
    cl.sort_by(|a, b| b.n.partial_cmp(&a.n).unwrap_or(std::cmp::Ordering::Equal).then(b.last.cmp(&a.last)));
    let want = (num(o, "zones", 5.0).round() as i64).max(1) as usize;
    let mut pmin = f64::INFINITY;
    let mut pmax = f64::NEG_INFINITY;
    for l in &cl {
        if l.price < pmin {
            pmin = l.price;
        }
        if l.price > pmax {
            pmax = l.price;
        }
    }
    let min_sep = (tol * 2.5).max((pmax - pmin) * 0.04);
    let mut shown: Vec<Level> = Vec::new();
    for l in &cl {
        if shown.len() >= want {
            break;
        }
        if shown.iter().all(|p| (p.price - l.price).abs() >= min_sep) {
            shown.push(*l);
        }
    }
    let max_n = shown.first().map(|l| l.n).unwrap_or(1.0).max(1.0);
    let cols = [
        strv(o, "c1", "#2962ff"),
        strv(o, "c2", "#ff9800"),
        strv(o, "c3", "#ef5350"),
        strv(o, "c4", "#26a69a"),
        strv(o, "c5", "#ab47bc"),
    ];
    let i1 = cc.len() - 1;
    let mut out = Vec::new();
    for (k, lvl) in shown.iter().enumerate() {
        let mut i0 = lvl.first.min(i1);
        if i0 >= i1 {
            i0 = i1.saturating_sub(1);
        }
        let data = vec![
            Point { time: cc[i0].time, value: lvl.price, color: None },
            Point { time: cc[i1].time, value: lvl.price, color: None },
        ];
        let mut s = mk_line(&cols[k % cols.len()], lw, data, None);
        s.title = Some(format!("{}%", (lvl.n / max_n * 100.0).round() as i64));
        s.price_line_visible = false;
        out.push(s);
    }
    out
}

fn compute_autotrend(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let flat = "#888888";
    let cc = clean_candles(c);
    let n = cc.len();
    if n < 12 {
        return vec![SeriesOut::line(flat, lw)];
    }
    // Segment-trend-wise zigzag through the causal swing chain: turns confirm as
    // soon as price retraces the threshold and never repaint, and the live leg is
    // carried to the last bar, so the line tracks the trend instead of snapping
    // into place only after a reversal has fully formed.
    let strength = num(o, "strength", 5.0).round().max(1.0) as usize;
    let mid: Vec<Option<f64>> = cc.iter().map(|x| Some((x.high + x.low) * 0.5)).collect();
    let vts = zigzag_vertices(&mid, &[], strength, strength, 0);
    if vts.len() < 2 {
        return vec![SeriesOut::line(flat, lw)];
    }
    let vals: Vec<f64> = vts.iter().map(|&i| mid[i].unwrap_or(f64::NAN)).collect();
    vec![pivot_zigzag_line(&cc, &vts, &vals, &up, &dn, flat, lw)]
}

fn compute_level_trend(c: &[Candle], o: &Settings, support: bool) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let cc = clean_candles(c);
    let n = cc.len();
    if n < 12 {
        return vec![SeriesOut::line(&up, lw)];
    }
    let piv = fractal_pivots(&cc, num(o, "strength", 5.0));
    let side: Vec<Pivot> = piv.iter().filter(|p| p.is_high != support).copied().collect();
    if side.len() < 2 {
        return vec![SeriesOut::line(&up, lw)];
    }
    let atr_per = (num(o, "atrPeriod", 14.0).round() as i64).max(2);
    let atr = wilder_arr(&tr_arr(&cc), atr_per);
    let atr_last = last_finite(&atr);
    let last_close = cc[n - 1].close;
    let min_pct = if num(o, "minPct", 0.05) >= 0.0 { num(o, "minPct", 0.05) } else { 0.05 };
    let mut tol = (atr_last * num(o, "tolMult", 0.5).max(0.0))
        .max(last_close.abs() * (min_pct / 100.0));
    if !(tol > 0.0) {
        tol = if last_close.abs() * 0.001 != 0.0 { last_close.abs() * 0.001 } else { 1.0 };
    }
    let best = match side_trend_line(&cc, &side, num(o, "look", 12.0), tol, support) {
        Some(b) => b,
        None => return vec![SeriesOut::line(&up, lw)],
    };
    // Rising line green, falling line red, so the colour tracks the slope the
    // trader sees rather than the support/resistance label.
    let col = if best.a >= 0.0 { up } else { dn };
    let last_t = cc[n - 1].time;
    let interval = if n >= 2 && last_t - cc[n - 2].time > 0 {
        last_t - cc[n - 2].time
    } else {
        60
    };
    let (plo, phi) = {
        let lo = cc.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let hi = cc.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let pad = (hi - lo) * 0.25;
        (lo - pad, hi + pad)
    };
    let fwd = (num(o, "fwd", 10.0).round() as i64).max(0);
    let start = if boolv(o, "fullSpan", false) { 0 } else { best.p1_idx };
    let mut data: Vec<Point> = Vec::new();
    for i in start..n {
        let v = best.a * i as f64 + best.b;
        if v.is_finite() {
            data.push(Point { time: cc[i].time, value: v, color: None });
        }
    }
    // Extend forward, but never draw the projection outside the price band:
    // a steep fit must not shoot off the top/bottom of the chart.
    for k in 1..=fwd {
        let v = best.a * (n as f64 - 1.0 + k as f64) + best.b;
        if !v.is_finite() || v < plo || v > phi {
            break;
        }
        data.push(Point { time: last_t + k * interval, value: v, color: None });
    }
    vec![mk_line(&col, lw, data, None)]
}

fn compute_supline(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    compute_level_trend(c, o, true)
}

fn compute_resline(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    compute_level_trend(c, o, false)
}

fn markers_supline(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"))
}

fn markers_resline(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"))
}

fn compute_smiio(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let short = int(o, "shortlen", 13).max(1);
    let long = int(o, "longlen", 25).max(2);
    let sig = int(o, "siglen", 9).max(1);
    let lw = num(o, "lineWidth", 1.0);
    let mut pc = vec![0.0; n];
    let mut ap = vec![0.0; n];
    for i in 1..n {
        pc[i] = c[i].close - c[i - 1].close;
        ap[i] = pc[i].abs();
    }
    let p1 = ema_arr(&pc, short);
    let p2 = ema_arr(&opt_to_f64(&p1), long);
    let a1 = ema_arr(&ap, short);
    let a2 = ema_arr(&opt_to_f64(&a1), long);
    let mut smi: Vec<Opt> = vec![None; n];
    for i in 0..n {
        if let Some(a) = a2[i] {
            if a != 0.0 {
                smi[i] = Some(100.0 * p2[i].unwrap_or(0.0) / a);
            }
        }
    }
    let signal = ema_arr(&opt_to_f64(&smi), sig);
    let mut hist: Vec<Opt> = vec![None; n];
    for i in 0..n {
        if let (Some(s), Some(sg)) = (smi[i], signal[i]) {
            hist[i] = Some(s - sg);
        }
    }
    vec![
        build_series(c, &smi, &strv(o, "color", "#7e57c2"), SeriesKind::Line, lw),
        build_series(c, &signal, &strv(o, "signalColor", "#ff6d00"), SeriesKind::Line, lw),
        build_series(c, &hist, &strv(o, "histColor", "#42a5f5"), SeriesKind::Line, lw),
    ]
}

fn compute_smf(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let look = (num(o, "length", 14.0).round() as i64).max(1) as usize;
    let sig = int(o, "signalLen", 9).max(1);
    let vol_len = (num(o, "volLen", 20.0).round() as i64).max(1) as usize;
    let cap = if num(o, "pulseCap", 3.0) > 0.0 { num(o, "pulseCap", 3.0) } else { 3.0 };
    let lw = num(o, "lineWidth", 1.0);
    let mut clv = vec![0.0; n];
    let mut wgt = vec![0.0; n];
    let mut vol_sum = 0.0;
    for i in 0..n {
        let b = c[i];
        let vol = b.volume;
        vol_sum += vol;
        if i >= vol_len {
            vol_sum -= c[i - vol_len].volume;
        }
        let range = b.high - b.low;
        let cl = if range > 0.0 { ((b.close - b.low) - (b.high - b.close)) / range } else { 0.0 };
        clv[i] = cl;
        let avg = if i + 1 >= vol_len { vol_sum / vol_len as f64 } else { vol_sum / (i + 1) as f64 };
        let pulse = if avg > 0.0 { (vol / avg).min(cap) } else { 1.0 };
        wgt[i] = vol * pulse;
    }
    let mut smf_arr = vec![0.0; n];
    let mut sn = 0.0;
    let mut sd = 0.0;
    for i in 0..n {
        sn += clv[i] * wgt[i];
        sd += wgt[i];
        if i >= look {
            sn -= clv[i - look] * wgt[i - look];
            sd -= wgt[i - look];
        }
        if i + 1 >= look {
            smf_arr[i] = if sd > 0.0 { 100.0 * sn / sd } else { 0.0 };
        }
    }
    let signal = ema_arr(&smf_arr, sig);
    let mut main_data = Vec::new();
    let mut sig_data = Vec::new();
    let mut hist_data = Vec::new();
    let mut prev_h: Option<f64> = None;
    let up_col = strv(o, "histUpColor", "#26a69a");
    let dn_col = strv(o, "histDownColor", "#ef5350");
    for i in (look - 1)..n {
        let v = smf_arr[i];
        if v.is_nan() {
            continue;
        }
        main_data.push(Point { time: c[i].time, value: v, color: None });
        let s = match signal[i] {
            Some(s) if !s.is_nan() => s,
            _ => continue,
        };
        sig_data.push(Point { time: c[i].time, value: s, color: None });
        let h = v - s;
        let up = prev_h.map(|p| h >= p).unwrap_or(h >= 0.0);
        hist_data.push(Point { time: c[i].time, value: h, color: Some(if up { up_col.clone() } else { dn_col.clone() }) });
        prev_h = Some(h);
    }
    let mut hs = SeriesOut::hist(&up_col);
    hs.data = hist_data;
    vec![
        mk_line(&strv(o, "color", "#26c6da"), lw, main_data, None),
        mk_line(&strv(o, "signalColor", "#ff6d00"), lw, sig_data, None),
        hs,
    ]
}

fn compute_autosr(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    if n == 0 {
        return Vec::new();
    }
    let lw = num(o, "lineWidth", 1.0);
    let res_color = strv(o, "resColor", "#26a69a");
    let sup_color = strv(o, "supColor", "#ef5350");
    let atr_per = (num(o, "atrPeriod", 14.0).round() as i64).max(2);
    let atr_mult = if num(o, "atrMult", 2.0) > 0.0 { num(o, "atrMult", 2.0) } else { 2.0 };
    let min_pct = if num(o, "minPct", 0.15) >= 0.0 { num(o, "minPct", 0.15) } else { 0.15 };
    let atr = wilder_arr(&tr_arr(c), atr_per);
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
    let mut last_pivot: Option<(bool, f64, usize)> = None;
    for i in 1..n {
        let t = th(i, c[i].close);
        if dir >= 0 {
            if c[i].high > ext {
                ext = c[i].high;
                ext_idx = i;
            }
            if c[i].low <= ext - t {
                last_pivot = Some((true, ext, ext_idx));
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
                last_pivot = Some((false, ext, ext_idx));
                dir = 1;
                ext = c[i].high;
                ext_idx = i;
            }
        }
    }
    let (mut res, mut sup, trend_up) = if dir >= 0 {
        (Some(ext), last_pivot.map(|p| p.1), true)
    } else {
        (last_pivot.map(|p| p.1), Some(ext), false)
    };
    if sup.map(|v| !v.is_finite()).unwrap_or(true) {
        sup = Some(c.iter().map(|x| x.low).fold(f64::INFINITY, f64::min));
    }
    if res.map(|v| !v.is_finite()).unwrap_or(true) {
        res = Some(c.iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max));
    }
    let res = res.unwrap();
    let sup = sup.unwrap();
    let fmt = |v: f64| if v < 100.0 { format!("{:.3}", v) } else { format!("{:.2}", v) };
    let mk_out = |price: f64, color: &str, active: bool, tag: &str| -> SeriesOut {
        let w = if active { lw + 1.0 } else { lw };
        let mut s = SeriesOut::line(color, w);
        s.data = c.iter().map(|x| Point { time: x.time, value: price, color: None }).collect();
        s.price_lines = vec![PriceLine {
            price,
            color: color.into(),
            line_width: w,
            line_style: if active { 0 } else { 2 },
            title: format!("{} {}", tag, fmt(price)),
        }];
        s
    };
    let mut out = vec![
        mk_out(res, &res_color, trend_up, "RES"),
        mk_out(sup, &sup_color, !trend_up, "SUP"),
    ];
    if boolv(o, "touchLine", true) && n >= 2 {
        let up = trend_up;
        let level = if up { sup } else { res };
        let mut k: i64 = -1;
        if let Some((is_high, _, idx)) = last_pivot {
            if idx + 1 < n && ((up && !is_high) || (!up && is_high)) {
                k = idx as i64;
            }
        }
        if k < 0 {
            let tol = level.abs() * 0.0005;
            let mut i = n as i64 - 2;
            while i >= 0 {
                if c[i as usize].low - tol <= level && c[i as usize].high + tol >= level {
                    k = i;
                    break;
                }
                i -= 1;
            }
        }
        if k >= 0 && (k as usize) < n - 1 {
            let color = if up { strv(o, "touchUpColor", "#26a69a") } else { strv(o, "touchDownColor", "#ef5350") };
            let data = vec![
                Point { time: c[k as usize].time, value: level, color: None },
                Point { time: c[n - 1].time, value: c[n - 1].close, color: None },
            ];
            let mut s = mk_line(&color, lw + 1.0, data, Some(0));
            s.last_value_visible = false;
            s.price_line_visible = false;
            out.push(s);
        }
    }
    out
}

fn break_markers(cc: &[Candle], piv: &[EwPivot], up: &str, dn: &str, require3: bool) -> Vec<Marker> {
    if piv.len() < 3 && require3 {
        return Vec::new();
    }
    let mut last_h: Option<f64> = None;
    let mut prev_h: Option<f64> = None;
    let mut last_l: Option<f64> = None;
    let mut prev_l: Option<f64> = None;
    let mut trend = 0i32;
    let mut out = Vec::new();
    let mut seen: BTreeMap<i64, bool> = BTreeMap::new();
    for p in piv {
        if p.is_high {
            prev_h = last_h;
            last_h = Some(p.price);
        } else {
            prev_l = last_l;
            last_l = Some(p.price);
        }
        let mut nt = trend;
        if let (Some(lh), Some(ph), Some(ll), Some(pl)) = (last_h, prev_h, last_l, prev_l) {
            if lh > ph && ll > pl {
                nt = 1;
            } else if lh < ph && ll < pl {
                nt = -1;
            }
        }
        if nt != trend && nt != 0 {
            if let Some(x) = cc.get(p.idx) {
                if seen.insert(x.time, true).is_none() {
                    out.push(Marker {
                        time: x.time,
                        position: if nt == 1 { "belowBar" } else { "aboveBar" }.into(),
                        color: if nt == 1 { up.into() } else { dn.into() },
                        shape: if nt == 1 { "arrowUp" } else { "arrowDown" }.into(),
                        text: if trend == 0 { "BOS" } else { "CHoCH" }.into(),
                        size: 1.0,
                    });
                }
            }
        }
        trend = nt;
    }
    out
}

/// Trend-start arrows derived from a line's own direction: `arrowUp` prints on
/// the bar where the line turns from falling/flat to rising, `arrowDown` where
/// it turns from rising/flat to falling. This is the exact direction the AST
/// "Straight Line" trend filters read, so the chart arrow and the arrow
/// detection filter always agree.
fn slope_markers(series: &[SeriesOut], idx: usize, up: &str, dn: &str) -> Vec<Marker> {
    let s = match series.get(idx) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut prev = 0i32;
    for i in 1..s.data.len() {
        let d = s.data[i].value - s.data[i - 1].value;
        let dir = if d > 0.0 {
            1
        } else if d < 0.0 {
            -1
        } else {
            prev
        };
        if dir != 0 && dir != prev {
            out.push(Marker {
                time: s.data[i].time,
                position: if dir == 1 { "belowBar" } else { "aboveBar" }.into(),
                color: if dir == 1 { up.into() } else { dn.into() },
                shape: if dir == 1 { "arrowUp" } else { "arrowDown" }.into(),
                text: String::new(),
                size: 1.0,
            });
        }
        prev = dir;
    }
    out
}

// Per-indicator arrow markers for the straight-line / overlay family. Each one
// derives arrows from the very line the AST trend filter reads, so the chart
// arrows and the "Arrow detection" filters share one definition.
fn markers_vlcore(c: &[Candle], o: &Settings) -> Vec<Marker> {
    slope_markers(&compute_vlcore(c, o), 0, &strv(o, "upColor", "#00e676"), &strv(o, "downColor", "#ff5252"))
}
fn markers_vl(c: &[Candle], o: &Settings) -> Vec<Marker> {
    slope_markers(&compute_vl(c, o), 0, &strv(o, "color", "#26c6da"), &strv(o, "signalColor", "#ff6d00"))
}
// Straight-line fits (Auto Trendline, Pitchfork, Trend Projection, Gann/Fib Fan,
// S/R EMA Reversal) have a constant slope, so a per-bar slope change never fires
// and the chart shows no arrows. Derive their trend-start arrows from the ATR
// zigzag structure instead - an up arrow where the structure turns to
// higher-highs + higher-lows, a down arrow where it turns lower. This is the
// same structure the trendline is anchored to.
fn markers_pivot_trend(c: &[Candle], o: &Settings, up: &str, dn: &str) -> Vec<Marker> {
    let cc = clean_candles(c);
    let piv = fractal_pivots(&cc, num(o, "strength", 5.0));
    structural_markers(&cc, &piv, up, dn)
}

fn structural_markers(cc: &[Candle], piv: &[Pivot], up: &str, dn: &str) -> Vec<Marker> {
    // Emit an arrow every time the swing structure confirms a direction: an
    // up swing makes a higher-high together with a higher-low, a down swing a
    // lower-high together with a lower-low. Unlike a regime-change detector
    // this keeps firing while a trend runs, so a fresh arrow is always near the
    // right edge and the "Arrow detection" filter has something to trigger on.
    let mut last_h: Option<f64> = None;
    let mut prev_h: Option<f64> = None;
    let mut last_l: Option<f64> = None;
    let mut prev_l: Option<f64> = None;
    let mut out = Vec::new();
    let mut seen: BTreeMap<i64, bool> = BTreeMap::new();
    for p in piv {
        if p.is_high {
            prev_h = last_h;
            last_h = Some(p.price);
        } else {
            prev_l = last_l;
            last_l = Some(p.price);
        }
        let dir = match (last_h, prev_h, last_l, prev_l) {
            (Some(lh), Some(ph), Some(ll), Some(pl)) => {
                if lh > ph && ll > pl {
                    1
                } else if lh < ph && ll < pl {
                    -1
                } else {
                    0
                }
            }
            _ => 0,
        };
        if dir != 0 {
            if let Some(x) = cc.get(p.idx) {
                if seen.insert(x.time, true).is_none() {
                    out.push(Marker {
                        time: x.time,
                        position: if dir == 1 { "belowBar" } else { "aboveBar" }.into(),
                        color: if dir == 1 { up.into() } else { dn.into() },
                        shape: if dir == 1 { "arrowUp" } else { "arrowDown" }.into(),
                        text: "".into(),
                        size: 1.0,
                    });
                }
            }
        }
    }
    out
}
fn markers_srema(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, &strv(o, "bullColor", "#26a69a"), &strv(o, "bearColor", "#ef5350"))
}
fn markers_slema(c: &[Candle], o: &Settings) -> Vec<Marker> {
    slope_markers(&compute_slema(c, o), 0, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"))
}
fn markers_autotrend(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"))
}
fn markers_pitchfork(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, "#26a69a", "#ef5350")
}
fn markers_projline(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, "#26a69a", "#ef5350")
}
fn markers_fibfan(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, "#26a69a", "#ef5350")
}
fn markers_gannfan(c: &[Candle], o: &Settings) -> Vec<Marker> {
    markers_pivot_trend(c, o, "#26a69a", "#ef5350")
}
fn markers_supplydemand(c: &[Candle], o: &Settings) -> Vec<Marker> {
    slope_markers(&compute_supplydemand(c, o), 0, "#26a69a", "#ef5350")
}

/// Gate helper for the straight-line arrow filters: did the zigzag structure
/// turn to the requested side exactly at the bar `offset` bars back? Mirrors
/// `markers_pivot_trend`, so the arrow a trader sees on the chart is the arrow
/// this filter detects. `None` when the structure is not resolvable yet
/// (warm-up / fewer than three pivots), treated as non-blocking by the gate.
pub fn trend_flip_at(
    candles: &[Candle],
    offset: usize,
    bull: bool,
    strength: f64,
) -> Option<bool> {
    let n = candles.len();
    if offset >= n {
        return None;
    }
    let target = candles[n - 1 - offset].time;
    let cc = clean_candles(candles);
    let piv = fractal_pivots(&cc, strength);
    if piv.len() < 3 {
        return None;
    }
    let mk = structural_markers(&cc, &piv, "#26a69a", "#ef5350");
    let want = if bull { "arrowUp" } else { "arrowDown" };
    Some(mk.iter().any(|m| m.time == target && m.shape == want))
}

fn compute_zzline(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    if a.c.len() < 3 || a.piv.len() < 2 {
        return vec![SeriesOut::line(&strv(o, "trendColor", "#2962ff"), lw)];
    }
    let mut line = Vec::new();
    let mut line_color = strv(o, "trendColor", "#2962ff");
    if boolv(o, "showLine", true) {
        let atr_per = (num(o, "atrPeriod", 14.0).round() as i64).max(2);
        let atr = wilder_arr(&tr_arr(&a.c), atr_per);
        let atr_last = last_finite(&atr);
        let last_close = a.c[a.c.len() - 1].close;
        let min_pct = if num(o, "minPct", 0.15) >= 0.0 { num(o, "minPct", 0.15) } else { 0.15 };
        let mut tol = (atr_last * 0.5).max(last_close.abs() * (min_pct / 100.0));
        if !(tol > 0.0) {
            tol = if last_close.abs() * 0.001 != 0.0 { last_close.abs() * 0.001 } else { 1.0 };
        }
        if let Some(best) = auto_trend_line(&a.c, &ew_to_pivots(&a.piv), num(o, "pivotLook", 8.0), tol) {
            line_color = if best.is_support { up.clone() } else { dn.clone() };
            let start = if boolv(o, "fullSpan", true) { 0 } else { best.p1_idx };
            for i in start..a.c.len() {
                let v = best.a * i as f64 + best.b;
                if v.is_finite() {
                    line.push(Point { time: a.c[i].time, value: v, color: None });
                }
            }
        }
    }
    let dir = live_trend_dir(&a.c, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    let mut s1 = mk_line(&line_color, lw, line, None);
    color_points_by_dir(&a.c, &mut s1.data, &dir, &up, &dn);
    vec![s1]
}

fn markers_zzline(c: &[Candle], o: &Settings) -> Vec<Marker> {
    if !boolv(o, "showBreaks", true) {
        return Vec::new();
    }
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    break_markers(&a.c, &a.piv, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"), false)
}

fn compute_trendmaster(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    if a.c.len() < 3 || a.piv.len() < 2 {
        return vec![SeriesOut::line(&strv(o, "trendColor", "#2962ff"), lw)];
    }
    let atr_per = (num(o, "atrPeriod", 14.0).round() as i64).max(2);
    let atr = wilder_arr(&tr_arr(&a.c), atr_per);
    let atr_last = last_finite(&atr);
    let last_close = a.c[a.c.len() - 1].close;
    let min_pct = if num(o, "minPct", 0.15) >= 0.0 { num(o, "minPct", 0.15) } else { 0.15 };
    let mut tol = (atr_last * 0.5).max(last_close.abs() * (min_pct / 100.0));
    if !(tol > 0.0) {
        tol = if last_close.abs() * 0.001 != 0.0 { last_close.abs() * 0.001 } else { 1.0 };
    }
    let mut line = Vec::new();
    let mut line_color = strv(o, "trendColor", "#2962ff");
    if let Some(best) = auto_trend_line(&a.c, &ew_to_pivots(&a.piv), num(o, "trendLook", 12.0), tol) {
        line_color = if best.is_support { up.clone() } else { dn.clone() };
        let start = if boolv(o, "fullSpan", true) { 0 } else { best.p1_idx };
        for i in start..a.c.len() {
            let v = best.a * i as f64 + best.b;
            if v.is_finite() {
                line.push(Point { time: a.c[i].time, value: v, color: None });
            }
        }
    }
    let dir = live_trend_dir(&a.c, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    let mut s1 = mk_line(&line_color, lw, line, None);
    color_points_by_dir(&a.c, &mut s1.data, &dir, &up, &dn);
    vec![s1]
}

fn markers_trendmaster(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    if a.c.len() < 3 || a.piv.len() < 2 {
        return Vec::new();
    }
    let mut mk: Vec<Marker> = Vec::new();
    let mut seen: BTreeMap<i64, bool> = BTreeMap::new();
    if boolv(o, "showBreaks", true) {
        mk = break_markers(&a.c, &a.piv, &up, &dn, true);
        for m in &mk {
            seen.insert(m.time, true);
        }
    }
    if boolv(o, "showVol", true) && !a.c.is_empty() {
        let vlen = (num(o, "volLength", 21.0).round() as i64).max(2) as usize;
        let vmult = if num(o, "volMult", 1.5) > 0.0 { num(o, "volMult", 1.5) } else { 1.5 };
        let vols: Vec<f64> = a.c.iter().map(|x| if x.volume.is_finite() { x.volume } else { 0.0 }).collect();
        let mut bias = vec![0i32; a.c.len()];
        let mut b = 0i32;
        let mut pi = 0usize;
        let (mut lh, mut ph, mut ll, mut pl): (Option<f64>, Option<f64>, Option<f64>, Option<f64>) = (None, None, None, None);
        for i in 0..a.c.len() {
            while pi < a.piv.len() && a.piv[pi].at <= i {
                let p = &a.piv[pi];
                pi += 1;
                if p.is_high {
                    ph = lh;
                    lh = Some(p.price);
                } else {
                    pl = ll;
                    ll = Some(p.price);
                }
                if let (Some(a1), Some(b1), Some(c1), Some(d1)) = (lh, ph, ll, pl) {
                    if a1 > b1 && c1 > d1 {
                        b = 1;
                    } else if a1 < b1 && c1 < d1 {
                        b = -1;
                    }
                }
            }
            bias[i] = b;
        }
        let vol_color = strv(o, "volColor", "#ffd54f");
        let mut vsum = 0.0;
        for i in 0..a.c.len() {
            if i >= vlen + 1 {
                vsum -= vols[i - vlen - 1];
            }
            if i >= 1 {
                vsum += vols[i - 1];
            }
            let cnt = i.min(vlen);
            if cnt < vlen {
                continue;
            }
            let avg = vsum / cnt as f64;
            if bias[i] == 0 || avg <= 0.0 || !(vols[i] > vmult * avg) {
                continue;
            }
            let body_up = a.c[i].close >= a.c[i].open;
            if (bias[i] == 1) != body_up {
                continue;
            }
            let t = a.c[i].time;
            if seen.insert(t, true).is_some() {
                continue;
            }
            mk.push(Marker {
                time: t,
                position: "inBar".into(),
                color: vol_color.clone(),
                shape: "circle".into(),
                text: String::new(),
                size: 1.0,
            });
        }
    }
    mk
}

fn compute_panemaster(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let flat = strv(o, "trendColor", "#2962ff");
    let sig = pane_signal(c, o);
    if sig.cc.len() < 35 {
        return vec![SeriesOut::line(&flat, lw)];
    }
    // Segment-trend-wise zigzag over the fused pane score: turn at every real
    // swing instead of drawing one least-squares line across a whole regime
    // (which cut through opposite trends). The six oscillators must move with
    // the leg for a turn to be kept.
    let strength = num(o, "minSeg", 3.0).round().max(1.0) as usize;
    let matrix: Vec<Vec<Option<f64>>> = vec![
        sig.rsi.clone(),
        sig.bb_pct.clone(),
        sig.st_k.clone(),
        sig.cci.clone(),
        sig.will_r.clone(),
        sig.mfi.clone(),
    ];
    let mid = price_mid_series(&sig.cc);
    let vts = zigzag_vertices(&mid, &matrix, strength, strength, 2);
    vec![zigzag_series_line(&sig.cc, &vts, &up, &dn, &flat, lw)]
}

fn markers_panemaster(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let sig = pane_signal(c, o);
    if sig.cc.len() < 35 {
        return Vec::new();
    }
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let mut mk: Vec<Marker> = Vec::new();
    let mut seen: BTreeMap<i64, bool> = BTreeMap::new();
    for i in 1..sig.cc.len() {
        if sig.regime[i] != sig.regime[i - 1] && sig.regime[i] != 0 {
            let t = sig.cc[i].time;
            if seen.insert(t, true).is_none() {
                let bull = sig.regime[i] > 0;
                mk.push(Marker {
                    time: t,
                    position: if bull { "belowBar" } else { "aboveBar" }.into(),
                    color: if bull { up.clone() } else { dn.clone() },
                    shape: if bull { "arrowUp" } else { "arrowDown" }.into(),
                    text: if bull { "BULL" } else { "BEAR" }.into(),
                    size: 1.0,
                });
            }
        }
    }
    if boolv(o, "showExhaustion", true) {
        let ex = strv(o, "exhaustColor", "#ffb300");
        let (mut prev_ob, mut prev_os) = (false, false);
        for i in 0..sig.cc.len() {
            if sig.regime[i] == 0 {
                prev_ob = false;
                prev_os = false;
                continue;
            }
            let mut ob = 0;
            let mut os = 0;
            if let Some(Some(a)) = sig.rsi.get(i).copied() {
                if a > 70.0 { ob += 1; } else if a < 30.0 { os += 1; }
            }
            if let Some(Some(b)) = sig.bb_pct.get(i).copied() {
                if b > 1.0 { ob += 1; } else if b < 0.0 { os += 1; }
            }
            if let Some(Some(k)) = sig.st_k.get(i).copied() {
                if k > 80.0 { ob += 1; } else if k < 20.0 { os += 1; }
            }
            if let Some(Some(cc)) = sig.cci.get(i).copied() {
                if cc > 100.0 { ob += 1; } else if cc < -100.0 { os += 1; }
            }
            if let Some(Some(w)) = sig.will_r.get(i).copied() {
                if w > -20.0 { ob += 1; } else if w < -80.0 { os += 1; }
            }
            if let Some(Some(mf)) = sig.mfi.get(i).copied() {
                if mf > 80.0 { ob += 1; } else if mf < 20.0 { os += 1; }
            }
            let ob_now = sig.regime[i] > 0 && ob >= 4;
            let os_now = sig.regime[i] < 0 && os >= 4;
            let t = sig.cc[i].time;
            if ob_now && !prev_ob && seen.insert(t, true).is_none() {
                mk.push(Marker { time: t, position: "aboveBar".into(), color: ex.clone(), shape: "circle".into(), text: "OB".into(), size: 1.0 });
            } else if os_now && !prev_os && seen.insert(t, true).is_none() {
                mk.push(Marker { time: t, position: "belowBar".into(), color: ex.clone(), shape: "circle".into(), text: "OS".into(), size: 1.0 });
            }
            prev_ob = ob_now;
            prev_os = os_now;
        }
    }
    mk
}

// ---------------------------------------------------------------------------
// Straight Line Consensus
// ---------------------------------------------------------------------------

/// Per-bar direction (+1 rising / -1 falling / 0 no resolved line yet) of one
/// output series (`idx`) of a straight-line indicator (`id`), aligned to `c`.
fn sl_line_dir(c: &[Candle], id: &str, idx: usize) -> Vec<i32> {
    let o: Settings = Settings::new();
    let outs = match id {
        "ewtrend" => compute_ewtrend(c, &o),
        "patrend" => compute_patrend(c, &o),
        "zzline" => compute_zzline(c, &o),
        "trendmaster" => compute_trendmaster(c, &o),
        "panemaster" => compute_panemaster(c, &o),
        "autotrend" => compute_autotrend(c, &o),
        "pitchfork" => compute_pitchfork(c, &o),
        "projline" => compute_projline(c, &o),
        "gant" => compute_gant(c, &o),
        "fibt" => compute_fibt(c, &o),
        "sremat" => compute_sremat(c, &o),
        _ => return vec![0; c.len()],
    };
    let mut by_time: BTreeMap<i64, f64> = BTreeMap::new();
    if let Some(s) = outs.get(idx) {
        for p in &s.data {
            if p.value.is_finite() {
                by_time.insert(p.time, p.value);
            }
        }
    }
    let vals: Vec<Option<f64>> = c.iter().map(|x| by_time.get(&x.time).copied()).collect();
    let mut out = vec![0i32; c.len()];
    let mut last = 0i32;
    for i in 0..vals.len() {
        if let Some(v) = vals[i] {
            if i > 0 {
                if let Some(pv) = vals[i - 1] {
                    if v > pv {
                        last = 1;
                    } else if v < pv {
                        last = -1;
                    }
                }
            }
        }
        out[i] = last;
    }
    out
}

/// Direction of the "Straight Line Consensus" trendline.
///
/// The twelve straight-line indicators vote a direction, but a raw vote is
/// leading and flips well before the candles turn. To make the line follow the
/// market like the support/resistance trendline (and flip only on a real
/// reversal) the consensus direction is locked to the swing structure:
/// a new direction must clear `min_agree` votes for `confirm` bars and then
/// price must actually break structure (close beyond the most recent confirmed
/// swing high for a bull flip, below the most recent confirmed swing low for a
/// bear flip). Until structure breaks the old trend is carried forward, so the
/// line stays with the candles right up to the reversal.
pub fn sl_consensus_dir(c: &[Candle], min_agree: i32, confirm: usize, strength: f64) -> Vec<i32> {
    let n = c.len();
    if n == 0 {
        return Vec::new();
    }
    let need = min_agree.max(1);
    let voters: [(&str, usize); 12] = [
        ("ewtrend", 0),
        ("ewtrend", 0),
        ("patrend", 0),
        ("zzline", 0),
        ("trendmaster", 0),
        ("panemaster", 0),
        ("autotrend", 0),
        ("pitchfork", 0),
        ("projline", 0),
        ("gant", 0),
        ("fibt", 0),
        ("sremat", 0),
    ];
    let mut votes = vec![0i32; n];
    for (id, idx) in voters {
        let d = sl_line_dir(c, id, idx);
        for i in 0..n {
            votes[i] += d.get(i).copied().unwrap_or(0);
        }
    }
    consensus_gate(c, &votes, need, confirm, strength)
}

/// Per-bar direction (+1 rising / -1 falling) of the aggregate line of one
/// overlay indicator (`id`): the mean of all of its finite output series at each
/// bar (for bands/channels that is the channel mid; for single-line indicators
/// it is the line itself). Used as one vote by the overlay consensus.
fn ovl_line_dir(c: &[Candle], id: &str) -> Vec<i32> {
    let o: Settings = Settings::new();
    let outs = match id {
        "ema" => compute_ema(c, &o),
        "ma" => compute_ma(c, &o),
        "smma" => compute_smma(c, &o),
        "hma" => compute_hma(c, &o),
        "bb" => compute_bb(c, &o),
        "keltner" => compute_keltner(c, &o),
        "pc" => compute_pc(c, &o),
        "donchian" => compute_donchian(c, &o),
        "chandelier" => compute_chandelier(c, &o),
        "ichimoku" => compute_ichimoku(c, &o),
        "supertrend" => compute_supertrend(c, &o),
        "vwap" => compute_vwap(c, &o),
        _ => return vec![0; c.len()],
    };
    let n = c.len();
    let mut sum = vec![0.0f64; n];
    let mut cnt = vec![0u32; n];
    let mut by_time: BTreeMap<i64, usize> = BTreeMap::new();
    for (i, x) in c.iter().enumerate() {
        by_time.insert(x.time, i);
    }
    for s in &outs {
        for p in &s.data {
            if p.value.is_finite() {
                if let Some(&i) = by_time.get(&p.time) {
                    sum[i] += p.value;
                    cnt[i] += 1;
                }
            }
        }
    }
    let mut out = vec![0i32; n];
    let mut last = 0i32;
    let mut prev: Option<f64> = None;
    for i in 0..n {
        if cnt[i] > 0 {
            let v = sum[i] / cnt[i] as f64;
            if let Some(pv) = prev {
                if v > pv {
                    last = 1;
                } else if v < pv {
                    last = -1;
                }
            }
            prev = Some(v);
        }
        out[i] = last;
    }
    out
}

/// Direction of the "Overlay Consensus" trendline. The twelve classic overlay
/// indicators (EMA, MA, SMMA, HMA, Bollinger, Keltner, Price Channel, Donchian,
/// Chandelier, Ichimoku, Supertrend, VWAP) each vote their own slope; the
/// majority is gated exactly like the straight-line consensus, so the fused line
/// flips only on a real, price-confirmed reversal.
pub fn ovl_consensus_dir(c: &[Candle], min_agree: i32, confirm: usize, strength: f64) -> Vec<i32> {
    let n = c.len();
    if n == 0 {
        return Vec::new();
    }
    let voters = [
        "ema",
        "ma",
        "smma",
        "hma",
        "bb",
        "keltner",
        "pc",
        "donchian",
        "chandelier",
        "ichimoku",
        "supertrend",
        "vwap",
    ];
    let mut votes = vec![0i32; n];
    for id in voters {
        let d = ovl_line_dir(c, id);
        for i in 0..n {
            votes[i] += d.get(i).copied().unwrap_or(0);
        }
    }
    consensus_gate(c, &votes, min_agree, confirm, strength)
}

/// Apply the vote-threshold + confirmation + swing-structure gate to a raw
/// per-bar vote tally. Shared by every consensus indicator (straight-line and
/// overlay) so they all flip only on a real, price-confirmed reversal.
fn consensus_gate(
    c: &[Candle],
    votes: &[i32],
    min_agree: i32,
    confirm: usize,
    strength: f64,
) -> Vec<i32> {
    let n = c.len();
    if n == 0 {
        return Vec::new();
    }
    let need = min_agree.max(1);
    // Most recent *confirmed* swing high/low available at each bar (a fractal
    // pivot is only known `strength` bars after it printed, so it is causal).
    // `strength < 2` disables structural gating entirely: the consensus then
    // follows the raw vote (used by the engine indicator-filter, whose settings
    // default to 0 = flip the instant the vote flips).
    let use_structure = strength >= 2.0;
    let piv = if use_structure { fractal_pivots(c, strength) } else { Vec::new() };
    let mut ph: Vec<Option<f64>> = vec![None; n];
    let mut pl: Vec<Option<f64>> = vec![None; n];
    let mut last_ph: Option<f64> = None;
    let mut last_pl: Option<f64> = None;
    let mut pi = 0usize;
    for i in 0..n {
        while pi < piv.len() && piv[pi].at <= i {
            if piv[pi].is_high {
                last_ph = Some(piv[pi].price);
            } else {
                last_pl = Some(piv[pi].price);
            }
            pi += 1;
        }
        ph[i] = last_ph;
        pl[i] = last_pl;
    }
    let mut out = vec![0i32; n];
    let mut cur = 0i32;
    let mut pending = 0i32;
    let mut streak = 0usize;
    for i in 0..n {
        let desired = if votes[i] >= need {
            1
        } else if votes[i] <= -need {
            -1
        } else {
            0
        };
        if desired == 0 || desired == cur {
            streak = 0;
            pending = 0;
        } else if desired == pending {
            streak += 1;
        } else {
            pending = desired;
            streak = 1;
        }
        if pending != 0 && streak >= confirm.max(1) {
            // Structure break required to flip when a confirmed swing exists; if
            // no swing pivot is known yet (e.g. a monotonic run with no pullback)
            // the vote+confirm gate alone governs so the line can never freeze.
            let price_ok = if !use_structure || cur == 0 {
                true
            } else if pending > 0 {
                ph[i].map_or(true, |h| c[i].close > h)
            } else {
                pl[i].map_or(true, |l| c[i].close < l)
            };
            if price_ok {
                cur = pending;
                streak = 0;
                pending = 0;
            }
        }
        out[i] = cur;
    }
    out
}

/// Candle midpoints as a value series: the price axis the consensus zigzags are
/// anchored to, so their legs always cut through the candle bodies and every
/// real price swing is a zigzag candidate.
fn price_mid_series(cc: &[Candle]) -> Vec<Option<f64>> {
    cc.iter().map(|c| Some((c.high + c.low) * 0.5)).collect()
}

/// Align the twelve straight-line indicators (one chosen output series each,
/// same voters as `sl_consensus_dir`) and return their per-bar mean. Used as the
/// value axis of the zigzag drawn by the Straight Line Consensus.
fn sl_series_matrix(c: &[Candle]) -> (Vec<Option<f64>>, Vec<Vec<Option<f64>>>) {
    let o: Settings = Settings::new();
    let voters: [(&str, usize); 12] = [
        ("ewtrend", 0),
        ("ewtrend", 0),
        ("patrend", 0),
        ("zzline", 0),
        ("trendmaster", 0),
        ("panemaster", 0),
        ("autotrend", 0),
        ("pitchfork", 0),
        ("projline", 0),
        ("gant", 0),
        ("fibt", 0),
        ("sremat", 0),
    ];
    let n = c.len();
    let mut by_time: BTreeMap<i64, usize> = BTreeMap::new();
    for (i, x) in c.iter().enumerate() {
        by_time.insert(x.time, i);
    }
    let mut matrix: Vec<Vec<Option<f64>>> = Vec::with_capacity(voters.len());
    let mut sum = vec![0.0f64; n];
    let mut cnt = vec![0u32; n];
    for (id, idx) in voters {
        let outs = match id {
            "ewtrend" => compute_ewtrend(c, &o),
            "patrend" => compute_patrend(c, &o),
            "zzline" => compute_zzline(c, &o),
            "trendmaster" => compute_trendmaster(c, &o),
            "panemaster" => compute_panemaster(c, &o),
            "autotrend" => compute_autotrend(c, &o),
            "pitchfork" => compute_pitchfork(c, &o),
            "projline" => compute_projline(c, &o),
            "gant" => compute_gant(c, &o),
            "fibt" => compute_fibt(c, &o),
            "sremat" => compute_sremat(c, &o),
            _ => Vec::new(),
        };
        let mut col = vec![None; n];
        if let Some(s) = outs.get(idx) {
            for p in &s.data {
                if p.value.is_finite() {
                    if let Some(&i) = by_time.get(&p.time) {
                        if col[i].is_none() {
                            col[i] = Some(p.value);
                        }
                    }
                }
            }
        }
        for i in 0..n {
            if let Some(v) = col[i] {
                sum[i] += v;
                cnt[i] += 1;
            }
        }
        matrix.push(col);
    }
    let agg = (0..n)
        .map(|i| if cnt[i] > 0 { Some(sum[i] / cnt[i] as f64) } else { None })
        .collect();
    (agg, matrix)
}

fn compute_slconsensus(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let flat = strv(o, "flatColor", "#6b6b88");
    let need = num(o, "minAgree", 2.0).round().max(1.0) as usize;
    let confirm = num(o, "confirm", 5.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0);
    let cc = clean_candles(c);
    if cc.len() < 2 {
        return vec![SeriesOut::line(&flat, lw)];
    }
    // Segment-trend-wise zigzag over the twelve straight-line indicators'
    // aggregate: one straight leg per swing, turning at every real reversal
    // instead of a single long line that would cut across other trends.
    let (_, matrix) = sl_series_matrix(&cc);
    let mid = price_mid_series(&cc);
    let vts = zigzag_vertices(&mid, &matrix, strength as usize, confirm, need);
    vec![zigzag_series_line(&cc, &vts, &up, &dn, &flat, lw)]
}

/// Every straight-line indicator id that votes in the Overall Straight Line
/// Consensus. The consensus ids are excluded (they would recurse / overlap) and
/// the price twin `padivo` is dropped so `padiv` is not counted twice.
fn sl_all_voter_ids() -> [&'static str; 18] {
    [
        "pastruct", "wavefib", "autotrend", "zzline", "trendmaster",
        "panemaster", "projline", "srema", "slema", "stzigzag",
        "supline", "resline", "pitchfork", "fibfan", "gannfan",
        "vlcore", "bbpctsl", "padiv",
    ]
}

/// Direction of the Overall Straight Line Consensus line. Every straight-line
/// indicator votes the SAME causal trend that colours its own line (its regime
/// for the reversal-aware ids, a slow price trend for the rest); the strong
/// majority is then gated by the shared consensus rule (vote threshold +
/// confirmation bars + a real swing-structure break), so the fused line follows
/// the OVERALL trend and flips only on a MAJOR trend change - never on the minor
/// divergences / pullbacks that make a zigzag.
fn sl_all_consensus_dir(
    c: &[Candle],
    min_agree: i32,
    confirm: usize,
    strength: f64,
    trend_len: usize,
) -> Vec<i32> {
    let n = c.len();
    if n == 0 {
        return Vec::new();
    }
    // Voters that fall back to a live price trend read use this (slow) length, so
    // a short counter-trend blip cannot sway the overall direction.
    let mut o: Settings = Settings::new();
    o.insert("trendLen".to_string(), serde_json::Value::from(trend_len as f64));
    let mut votes = vec![0i32; n];
    for id in sl_all_voter_ids() {
        let d = straight_line_dir(id, c, &o);
        for i in 0..n {
            votes[i] += d.get(i).copied().unwrap_or(0);
        }
    }
    consensus_gate(c, &votes, min_agree, confirm, strength)
}

fn compute_slallconsensus(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // Pane straight line: one constant slope per committed overall-trend regime,
    // turning only on a confirmed reversal of the whole straight-line family.
    let step = num(o, "step", 0.02).max(0.0);
    let lw = num(o, "lineWidth", 1.0);
    let flat = strv(o, "flatColor", "#6b6b88");
    let need = num(o, "minAgree", 12.0).round().max(1.0) as i32;
    let confirm = num(o, "confirm", 10.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0);
    let trend_len = num(o, "trendLen", 34.0).round().max(2.0) as usize;
    let cc = clean_candles(c);
    let state = sl_all_consensus_dir(&cc, need, confirm, strength, trend_len);
    let mut level = 0.0f64;
    let mut data: Vec<Point> = Vec::new();
    for i in 0..cc.len() {
        let s = state.get(i).copied().unwrap_or(0);
        if s == 0 {
            continue;
        }
        level += s as f64 * step;
        data.push(Point { time: cc[i].time, value: level, color: None });
    }
    color_line_by_slope(&mut data, &strv(o, "upColor", "#26a69a"), &strv(o, "downColor", "#ef5350"));
    let mut s = SeriesOut::line(&flat, lw);
    s.data = data;
    vec![s]
}

fn compute_slallconsensuso(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    // Overlay trendline: one straight segment per overall-trend leg, drawn
    // between the leg's real turning prices. Because both ends are actual closes
    // the line always stays inside the price range, so it hugs the trend and can
    // never run off the chart or squeeze the candles. A tiny direction nudge on a
    // perfectly flat leg keeps it from ever being a flatline. The series is also
    // excluded from the price auto-scale, so the candles are never compressed.
    let lw = num(o, "lineWidth", 2.0);
    let flat = strv(o, "flatColor", "#6b6b88");
    let need = num(o, "minAgree", 12.0).round().max(1.0) as i32;
    let confirm = num(o, "confirm", 10.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0);
    let trend_len = num(o, "trendLen", 34.0).round().max(2.0) as usize;
    let min_pct = num(o, "minSlopePct", 0.0005).max(0.0);
    let cc = clean_candles(c);
    let state = sl_all_consensus_dir(&cc, need, confirm, strength, trend_len);
    let n = cc.len();
    let mut data: Vec<Point> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let s = state.get(i).copied().unwrap_or(0);
        if s == 0 {
            i += 1;
            continue;
        }
        // Extent of this committed leg (the final one runs to the last bar).
        let a = i;
        let mut b = i;
        while b + 1 < n && state.get(b + 1).copied().unwrap_or(0) == s {
            b += 1;
        }
        let pa = cc[a].close;
        let pb = cc[b].close;
        let bars = (b - a).max(1) as f64;
        let lo = pa.min(pb);
        let hi = pa.max(pb);
        let mut slope = (pb - pa) / bars;
        if slope == 0.0 {
            // Extremely rare: never emit a perfectly flat leg.
            slope = s as f64 * pa.abs().max(1.0) * min_pct;
        }
        for k in a..=b {
            let v = (pa + slope * (k - a) as f64).clamp(lo, hi);
            data.push(Point { time: cc[k].time, value: v, color: None });
        }
        i = b + 1;
    }
    let mut s = SeriesOut::line(&flat, lw);
    s.exclude_autoscale = true;
    s.data = data;
    vec![s]
}

fn markers_slconsensus(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let need = num(o, "minAgree", 2.0).round().max(1.0) as usize;
    let confirm = num(o, "confirm", 5.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0);
    let cc = clean_candles(c);
    if cc.len() < 2 {
        return Vec::new();
    }
    let (_, matrix) = sl_series_matrix(&cc);
    let mid = price_mid_series(&cc);
    let vts = zigzag_vertices(&mid, &matrix, strength as usize, confirm, need);
    zigzag_turn_markers(&cc, &vts, &up, &dn)
}

/// Align the twelve overlay indicators to the candles as one column each and
/// return their per-bar mean (`agg`). For bands/channels the mean of all output
/// lines is the channel mid; for single-line indicators it is the line itself.
fn ovl_series_matrix(c: &[Candle]) -> (Vec<Option<f64>>, Vec<Vec<Option<f64>>>) {
    let o: Settings = Settings::new();
    let ids = [
        "ema", "ma", "smma", "hma", "bb", "keltner", "pc", "donchian", "chandelier",
        "ichimoku", "supertrend", "vwap",
    ];
    let n = c.len();
    let mut by_time: BTreeMap<i64, usize> = BTreeMap::new();
    for (i, x) in c.iter().enumerate() {
        by_time.insert(x.time, i);
    }
    let mut matrix: Vec<Vec<Option<f64>>> = Vec::with_capacity(ids.len());
    let mut sum = vec![0.0f64; n];
    let mut cnt = vec![0u32; n];
    for id in ids {
        let outs = match id {
            "ema" => compute_ema(c, &o),
            "ma" => compute_ma(c, &o),
            "smma" => compute_smma(c, &o),
            "hma" => compute_hma(c, &o),
            "bb" => compute_bb(c, &o),
            "keltner" => compute_keltner(c, &o),
            "pc" => compute_pc(c, &o),
            "donchian" => compute_donchian(c, &o),
            "chandelier" => compute_chandelier(c, &o),
            "ichimoku" => compute_ichimoku(c, &o),
            "supertrend" => compute_supertrend(c, &o),
            "vwap" => compute_vwap(c, &o),
            _ => Vec::new(),
        };
        let mut col = vec![None; n];
        for s in &outs {
            for p in &s.data {
                if p.value.is_finite() {
                    if let Some(&i) = by_time.get(&p.time) {
                        if col[i].is_none() {
                            col[i] = Some(p.value);
                        }
                    }
                }
            }
        }
        for i in 0..n {
            if let Some(v) = col[i] {
                sum[i] += v;
                cnt[i] += 1;
            }
        }
        matrix.push(col);
    }
    let agg = (0..n)
        .map(|i| if cnt[i] > 0 { Some(sum[i] / cnt[i] as f64) } else { None })
        .collect();
    (agg, matrix)
}

fn first_valid_idx(v: &[Option<f64>]) -> Option<usize> {
    v.iter().position(|x| x.is_some())
}

fn last_valid_idx(v: &[Option<f64>]) -> Option<usize> {
    v.iter().rposition(|x| x.is_some())
}

/// ZigZag turning points of the aggregate overlay line: alternate swing
/// highs/lows with at least `half` bars on each side, legs at least `min_leg`
/// bars long, and each leg backed by at least `min_agree` of the twelve overlay
/// indicators moving the same way. Endpoints are always included so the line
/// spans the whole series.
fn zigzag_vertices(
    agg: &[Option<f64>],
    matrix: &[Vec<Option<f64>>],
    half: usize,
    min_leg: usize,
    min_agree: usize,
) -> Vec<usize> {
    let n = agg.len();
    if n == 0 {
        return Vec::new();
    }
    let half = half.max(1);
    let min_leg = min_leg.max(1);
    // Causal, non-repainting swing detection. A swing is confirmed - and frozen -
    // the moment price retraces `half` typical steps from the running extreme,
    // instead of waiting for `half` FUTURE bars to agree. The old forward-looking
    // scan could only classify the trailing `half` bars once bars after them
    // existed, so through a reversal the turn bar kept being re-picked and the
    // line only settled at the end. Here every committed turn is permanent and
    // the caller carries the live leg to the last bar.
    let mut step = vec![0.0f64; n];
    {
        let mut prev: Option<f64> = None;
        let mut ema = 0.0f64;
        for i in 0..n {
            if let Some(v) = agg[i] {
                if let Some(p) = prev {
                    let d = (v - p).abs();
                    ema = if ema == 0.0 { d } else { ema + (d - ema) / 14.0 };
                }
                prev = Some(v);
            }
            step[i] = ema;
        }
    }
    let mut cand: Vec<(usize, bool)> = Vec::new();
    let first = match first_valid_idx(agg) {
        Some(f) => f,
        None => return Vec::new(),
    };
    {
        let mut dir = 1i32;
        let mut ext = agg[first].unwrap();
        let mut ext_idx = first;
        for i in (first + 1)..n {
            let v = match agg[i] {
                Some(v) => v,
                None => continue,
            };
            let t = step[i] * half as f64;
            if !(t > 0.0) {
                continue;
            }
            if dir >= 0 {
                if v > ext {
                    ext = v;
                    ext_idx = i;
                } else if ext - v >= t {
                    cand.push((ext_idx, true));
                    dir = -1;
                    ext = v;
                    ext_idx = i;
                }
            } else if v < ext {
                ext = v;
                ext_idx = i;
            } else if v - ext >= t {
                cand.push((ext_idx, false));
                dir = 1;
                ext = v;
                ext_idx = i;
            }
        }
        if cand.first().map(|c| c.0) != Some(first) {
            let side = cand.first().map(|c| !c.1).unwrap_or(false);
            cand.insert(0, (first, side));
        }
    }
    let mut vts: Vec<usize> = Vec::new();
    let mut highs: Vec<bool> = Vec::new();
    for (i, is_high) in cand {
        let v = agg[i].unwrap();
        if vts.is_empty() {
            vts.push(i);
            highs.push(is_high);
            continue;
        }
        let last = *vts.last().unwrap();
        let last_high = *highs.last().unwrap();
        if i.saturating_sub(last) < min_leg {
            continue;
        }
        if is_high == last_high {
            let lv = agg[last].unwrap();
            let better = if is_high { v > lv } else { v < lv };
            if better {
                *vts.last_mut().unwrap() = i;
            }
            continue;
        }
        // A low -> high leg is an up move (and high -> low a down move). Keep
        // the turn only when enough of the twelve overlays agree with the leg.
        let go_up = !last_high;
        let mut oks = 0usize;
        for col in matrix {
            if let (Some(va), Some(vb)) = (col[last], col[i]) {
                if (go_up && vb > va) || (!go_up && vb < va) {
                    oks += 1;
                }
            }
        }
        if oks < min_agree {
            continue;
        }
        vts.push(i);
        highs.push(is_high);
    }
    if vts.is_empty() {
        if let (Some(f), Some(l)) = (first_valid_idx(agg), last_valid_idx(agg)) {
            vts.push(f);
            if l != f {
                vts.push(l);
            }
        }
        return vts;
    }
    if let Some(f) = first_valid_idx(agg) {
        if f < vts[0] {
            vts.insert(0, f);
        }
    }
    if let Some(l) = last_valid_idx(agg) {
        if l > *vts.last().unwrap() {
            vts.push(l);
        }
    }
    vts
}

/// Build the zigzag line from a set of vertex indices: one straight leg between
/// consecutive vertices. Values are anchored to the candle midpoint so the line
/// always cuts through the candle bodies rather than floating on an averaged
/// indicator scale; each leg is coloured by its own visible slope.
fn zigzag_series_line(
    cc: &[Candle],
    vts: &[usize],
    up: &str,
    dn: &str,
    flat: &str,
    lw: f64,
) -> SeriesOut {
    let mid = |i: usize| (cc[i].high + cc[i].low) * 0.5;
    let mut data: Vec<Point> = Vec::new();
    for (k, &i) in vts.iter().enumerate() {
        let v = mid(i);
        let color = if k == 0 {
            flat.to_string()
        } else {
            let pv = mid(vts[k - 1]);
            if v > pv {
                up.to_string()
            } else if v < pv {
                dn.to_string()
            } else {
                flat.to_string()
            }
        };
        data.push(Point { time: cc[i].time, value: (v * 100.0).round() / 100.0, color: Some(color) });
    }
    mk_line(flat, lw, data, None)
}

/// Flip arrows at the internal zigzag turns: a low turn is a bull start, a high
/// turn a bear start.
fn zigzag_turn_markers(cc: &[Candle], vts: &[usize], up: &str, dn: &str) -> Vec<Marker> {
    let mid = |i: usize| (cc[i].high + cc[i].low) * 0.5;
    let mut mk: Vec<Marker> = Vec::new();
    for k in 1..vts.len().saturating_sub(1) {
        let cur = mid(vts[k]);
        let prev = mid(vts[k - 1]);
        let next = mid(vts[k + 1]);
        let is_high = cur >= prev && cur >= next;
        let bull = !is_high;
        mk.push(Marker {
            time: cc[vts[k]].time,
            position: if bull { "belowBar" } else { "aboveBar" }.into(),
            color: if bull { up.to_string() } else { dn.to_string() },
            shape: if bull { "arrowUp" } else { "arrowDown" }.into(),
            text: if bull { "BULL" } else { "BEAR" }.into(),
            size: 1.0,
        });
    }
    mk
}

/// Segment-trend-wise zigzag through confirmed swing pivots: one straight leg
/// between consecutive pivots, each coloured by that leg's direction. Used by
/// the Auto Trendline and Trend Projection so they follow the price swings
/// instead of one constant-slope fit.
fn pivot_zigzag_line(
    cc: &[Candle],
    vts: &[usize],
    vals: &[f64],
    up: &str,
    dn: &str,
    flat: &str,
    lw: f64,
) -> SeriesOut {
    let m = vts.len().min(vals.len());
    let mut data: Vec<Point> = Vec::with_capacity(m);
    for k in 0..m {
        let v = vals[k];
        if !v.is_finite() || vts[k] >= cc.len() {
            continue;
        }
        let color = if k + 1 < m {
            if vals[k + 1] >= v { up } else { dn }
        } else if k > 0 {
            if v >= vals[k - 1] { up } else { dn }
        } else {
            flat
        };
        data.push(Point { time: cc[vts[k]].time, value: v, color: Some(color.into()) });
    }
    mk_line(flat, lw, data, None)
}

fn compute_ovlconsensus(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let flat = strv(o, "flatColor", "#6b6b88");
    let need = num(o, "minAgree", 2.0).round().max(1.0) as usize;
    let confirm = num(o, "confirm", 5.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0);
    let cc = clean_candles(c);
    if cc.len() < 2 {
        return vec![SeriesOut::line(&flat, lw)];
    }
    // Segment-trend-wise zigzag: draw a straight leg between every aggregate
    // swing turn instead of one long line across a whole consensus regime, so
    // the line never cuts through an opposite trend on its way.
    let (agg, matrix) = ovl_series_matrix(&cc);
    let vts = zigzag_vertices(&agg, &matrix, strength as usize, confirm, need);
    let mut data: Vec<Point> = Vec::new();
    for (k, &i) in vts.iter().enumerate() {
        let v = match agg[i] {
            Some(v) => v,
            None => continue,
        };
        let color = if k == 0 {
            flat.clone()
        } else {
            let pv = agg[vts[k - 1]].unwrap_or(v);
            if v > pv {
                up.clone()
            } else if v < pv {
                dn.clone()
            } else {
                flat.clone()
            }
        };
        data.push(Point { time: cc[i].time, value: (v * 100.0).round() / 100.0, color: Some(color) });
    }
    vec![mk_line(&flat, lw, data, None)]
}

fn markers_ovlconsensus(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let need = num(o, "minAgree", 2.0).round().max(1.0) as usize;
    let confirm = num(o, "confirm", 5.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0);
    let cc = clean_candles(c);
    if cc.len() < 2 {
        return Vec::new();
    }
    let (agg, matrix) = ovl_series_matrix(&cc);
    let vts = zigzag_vertices(&agg, &matrix, strength as usize, confirm, need);
    let mut mk: Vec<Marker> = Vec::new();
    for k in 1..vts.len().saturating_sub(1) {
        let cur = agg[vts[k]].unwrap_or(0.0);
        let prev = agg[vts[k - 1]].unwrap_or(cur);
        let next = agg[vts[k + 1]].unwrap_or(cur);
        let is_high = cur >= prev && cur >= next;
        let bull = !is_high;
        mk.push(Marker {
            time: cc[vts[k]].time,
            position: if bull { "belowBar" } else { "aboveBar" }.into(),
            color: if bull { up.clone() } else { dn.clone() },
            shape: if bull { "arrowUp" } else { "arrowDown" }.into(),
            text: if bull { "BULL" } else { "BEAR" }.into(),
            size: 1.0,
        });
    }
    mk
}

fn compute_pitchfork(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 1.0);
    let med = strv(o, "medianColor", "#2962ff");
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    if a.c.len() < 3 {
        return vec![SeriesOut::line(&med, lw)];
    }
    let min_span = (num(o, "minSpan", 12.0).round() as i64).max(3) as f64;
    let mut rng = 0.0;
    {
        let hi = a.c.iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max);
        let lo = a.c.iter().map(|x| x.low).fold(f64::INFINITY, f64::min);
        rng = hi - lo;
    }
    let mut sel: Option<(EwPivot, EwPivot, EwPivot, f64)> = None;
    let piv = &a.piv;
    if piv.len() >= 3 {
        let mut end = piv.len() as i64 - 1;
        while end >= 2 {
            let c3 = &piv[end as usize];
            let b2 = &piv[(end - 1) as usize];
            let a1 = &piv[(end - 2) as usize];
            if a1.is_high == b2.is_high || b2.is_high == c3.is_high {
                end -= 1;
                continue;
            }
            if !(a1.idx < b2.idx && b2.idx < c3.idx) {
                end -= 1;
                continue;
            }
            let span = c3.idx as f64 - a1.idx as f64;
            if span < min_span {
                end -= 1;
                continue;
            }
            let mx0 = (a1.idx + c3.idx) as f64 / 2.0;
            if (mx0 - b2.idx as f64).abs() < (2.0f64).max(span * 0.15) {
                end -= 1;
                continue;
            }
            let my0 = (a1.price + c3.price) / 2.0;
            let sl = (my0 - b2.price) / (mx0 - b2.idx as f64);
            if !sl.is_finite() || sl.abs() * span > rng * 0.8 {
                end -= 1;
                continue;
            }
            sel = Some((a1.clone(), b2.clone(), c3.clone(), sl));
            break;
        }
    }
    let (a1, b2, c3, slope) = match sel {
        Some(v) => v,
        None => return vec![SeriesOut::line(&med, lw)],
    };
    let start_idx = if boolv(o, "fullSpan", false) { 0 } else { a1.idx.min(b2.idx).min(c3.idx) };
    let mk = |base: &EwPivot| -> Vec<Point> {
        let mut d = Vec::new();
        for i in start_idx..a.c.len() {
            let v = base.price + slope * (i as f64 - base.idx as f64);
            if v.is_finite() {
                d.push(Point { time: a.c[i].time, value: v, color: None });
            }
        }
        d
    };
    let mut s0 = mk_line(&med, lw, mk(&b2), None);
    s0.exclude_autoscale = true;
    let dir = live_trend_dir(&a.c, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    color_points_by_dir(&a.c, &mut s0.data, &dir, &strv(o, "lowerColor", "#26a69a"), &strv(o, "upperColor", "#ef5350"));
    vec![s0]
}

fn fan_rays(c: &[Candle], o: &Settings, ratios: &[f64], color_key: &str, default_color: &str) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 1.0);
    let col = strv(o, color_key, default_color);
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    let empty = || ratios.iter().map(|_| SeriesOut::line(&col, lw)).collect::<Vec<_>>();
    if a.c.len() < 3 {
        return empty();
    }
    let t = match last_alternating_pivots(&ew_to_pivots(&a.piv), 2) {
        Some(v) => v,
        None => return empty(),
    };
    let p0 = t[0];
    let p1 = t[1];
    let dx = p1.idx as i64 - p0.idx as i64;
    if dx <= 0 {
        return empty();
    }
    let dy = p1.price - p0.price;
    let dir = live_trend_dir(&a.c, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    let mut out: Vec<SeriesOut> = ratios
        .iter()
        .map(|r| {
            let slope = (r * dy) / dx as f64;
            let mut d = Vec::new();
            for i in p0.idx..a.c.len() {
                let v = p0.price + slope * (i as f64 - p0.idx as f64);
                if v.is_finite() {
                    d.push(Point { time: a.c[i].time, value: v, color: None });
                }
            }
            let mut s = mk_line(&col, lw, d, None);
            s.exclude_autoscale = true;
            s
        })
        .collect();
    for s in out.iter_mut() {
        color_points_by_dir(&a.c, &mut s.data, &dir, "#26a69a", "#ef5350");
    }
    out
}

fn compute_fibfan(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    fan_rays(c, o, &[0.5], "fanColor", "#ab47bc")
}

fn compute_gannfan(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let mults = [1.0];
    let lw = num(o, "lineWidth", 1.0);
    let col = strv(o, "fanColor", "#607d8b");
    let a = ew_analyze(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15));
    let empty = || mults.iter().map(|_| SeriesOut::line(&col, lw)).collect::<Vec<_>>();
    if a.c.len() < 3 {
        return empty();
    }
    let t = match last_alternating_pivots(&ew_to_pivots(&a.piv), 2) {
        Some(v) => v,
        None => return empty(),
    };
    let p0 = t[0];
    let p1 = t[1];
    let dx = p1.idx as i64 - p0.idx as i64;
    if dx <= 0 {
        return empty();
    }
    let base = (p1.price - p0.price) / dx as f64;
    let dir = live_trend_dir(&a.c, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    let mut out: Vec<SeriesOut> = mults
        .iter()
        .map(|m| {
            let slope = m * base;
            let mut d = Vec::new();
            for i in p0.idx..a.c.len() {
                let v = p0.price + slope * (i as f64 - p0.idx as f64);
                if v.is_finite() {
                    d.push(Point { time: a.c[i].time, value: v, color: None });
                }
            }
            let mut s = mk_line(&col, lw, d, None);
            s.exclude_autoscale = true;
            s
        })
        .collect();
    for s in out.iter_mut() {
        color_points_by_dir(&a.c, &mut s.data, &dir, "#26a69a", "#ef5350");
    }
    out
}

fn compute_fibt(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    fan_swing_trend(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15), "#ab47bc")
}

fn compute_gant(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    fan_swing_trend(c, num(o, "atrPeriod", 14.0), num(o, "atrMult", 2.0), num(o, "minPct", 0.15), "#607d8b")
}

fn compute_slema(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let flat = strv(o, "flatColor", "#6b6b88");
    let lw = num(o, "lineWidth", 2.0);
    let src = src_arr(c, &strv(o, "source", "close"));
    let arr = ema_arr(&src, int(o, "length", 9).max(1));
    // Draw the EMA as a zigzag through its own swing turns instead of one smooth
    // curve: each leg is a straight line between an EMA high and the next EMA
    // low (and back), so the line visibly zigzags with the trend. The shared
    // straight-line colour rule repaints each rising leg green and each falling
    // leg red, exactly like the other straight-line indicators.
    let half = int(o, "swing", 5).max(1) as usize;
    let min_leg = int(o, "minLeg", 5).max(1) as usize;
    let vts = zigzag_vertices(&arr, &[], half, min_leg, 0);
    let vals: Vec<f64> = vts
        .iter()
        .map(|&i| arr.get(i).copied().flatten().unwrap_or(f64::NAN))
        .collect();
    vec![pivot_zigzag_line(c, &vts, &vals, &up, &dn, &flat, lw)]
}

fn compute_srema(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let bull = strv(o, "bullColor", "#26a69a");
    let bear = strv(o, "bearColor", "#ef5350");
    let a = srema_analyze(
        c,
        num(o, "atrPeriod", 14.0),
        num(o, "atrMult", 2.0),
        num(o, "minPct", 0.15),
        num(o, "emaPeriod", 1.0),
        num(o, "touchMult", 0.5),
        num(o, "touchWindow", 3.0),
        num(o, "fwd", 20.0),
    );
    if a.c.len() < 3 {
        return vec![SeriesOut::line(&bull, lw)];
    }
    let cc = &a.c;
    let n = cc.len();
    let dir = live_trend_dir(cc, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    let want_bull = dir[n - 1] >= 0;
    let piv = atr_zigzag(
        cc,
        (num(o, "atrPeriod", 14.0).round() as i64).max(2) as usize,
        if num(o, "atrMult", 2.0) > 0.0 { num(o, "atrMult", 2.0) } else { 2.0 },
        if num(o, "minPct", 0.15) >= 0.0 { num(o, "minPct", 0.15) } else { 0.15 },
    );
    let mut anchor: Option<(usize, f64)> = None;
    for p in piv.iter().rev() {
        if p.idx < n && ((want_bull && !p.is_high) || (!want_bull && p.is_high)) {
            anchor = Some((p.idx, p.price));
            break;
        }
    }
    let mut data = Vec::new();
    if let Some((ai, ap)) = anchor {
        let span = (n - 1).saturating_sub(ai);
        if span > 0 {
            let slope = (cc[n - 1].close - ap) / span as f64;
            for i in ai..n {
                let v = ap + slope * (i - ai) as f64;
                if v.is_finite() {
                    data.push(Point {
                        time: cc[i].time,
                        value: v,
                        color: Some((if want_bull { &bull } else { &bear }).clone()),
                    });
                }
            }
        }
    }
    let mut s = SeriesOut::line(if want_bull { &bull } else { &bear }, lw);
    s.last_value_visible = false;
    s.price_line_visible = false;
    s.data = data;
    vec![s]
}

fn compute_sremat(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let a = srema_analyze(
        c,
        num(o, "atrPeriod", 14.0),
        num(o, "atrMult", 2.0),
        num(o, "minPct", 0.15),
        num(o, "emaPeriod", 1.0),
        num(o, "touchMult", 0.5),
        num(o, "touchWindow", 3.0),
        num(o, "fwd", 20.0),
    );
    if a.c.len() < 3 {
        return vec![SeriesOut::line("#ffb300", 1.0)];
    }
    score_line(&a.c, &a.state, "#ffb300")
}

/// One completed swing leg of the price structure: the bar span and the signed
/// price amplitude between two consecutive confirmed pivots.
#[derive(Clone, Copy, Debug)]
struct SdLeg {
    start_idx: usize,
    end_idx: usize,
    start_px: f64,
    end_px: f64,
}

impl SdLeg {
    fn bars(&self) -> f64 {
        self.end_idx.saturating_sub(self.start_idx) as f64
    }
    fn amp(&self) -> f64 {
        self.end_px - self.start_px
    }
}

/// Causal "short-trend" structure legs (no ATR-zigzag / pivot confirmation).
/// A leg runs while the causal trend keeps its sign, so the boundary appears on
/// the bar the trend actually flips - the chain never waits for a swing to be
/// confirmed. Each leg records its bar length and its real amplitude (the
/// extreme-to-extreme move) - the two numbers the repeat projection draws.
fn structure_legs(cc: &[Candle], trend_len: usize) -> Vec<SdLeg> {
    let n = cc.len();
    let mut legs = Vec::new();
    if n < 3 {
        return legs;
    }
    let dir = live_trend_dir(cc, trend_len.max(2));
    let mut start = 0usize;
    for i in 1..=n {
        if i < n && dir[i] == dir[start] {
            continue;
        }
        let (s, e) = (start, i - 1);
        let mut hi_i = s;
        let mut lo_i = s;
        for k in s..=e {
            if cc[k].high > cc[hi_i].high {
                hi_i = k;
            }
            if cc[k].low < cc[lo_i].low {
                lo_i = k;
            }
        }
        let up = hi_i >= lo_i;
        let (start_idx, start_px, end_idx, end_px) = if up {
            (lo_i, cc[lo_i].low, hi_i, cc[hi_i].high)
        } else {
            (hi_i, cc[hi_i].high, lo_i, cc[lo_i].low)
        };
        if end_idx != start_idx {
            legs.push(SdLeg { start_idx, end_idx, start_px, end_px });
        }
        start = i;
    }
    legs
}

/// Measure the recent completed "short-trend" structure and forecast the next
/// one. Public so a backtest can evaluate the exact rule the indicator draws.
#[derive(Clone, Copy, Debug)]
pub struct SdForecast {
    pub bars: f64,
    pub amp: f64,
    pub dir: i32,
    pub score: f64,
}

/// `dir` is +1 (up) / -1 (down); `score` is the fraction of the recent legs that
/// match the template within `eq_tol` (0..1) - the "structure repeat" confidence.
pub fn sd_forecast(
    cc: &[Candle],
    trend_len: usize,
    sym_k: usize,
    eq_tol: f64,
) -> Option<SdForecast> {
    let legs = structure_legs(cc, trend_len);
    if legs.len() < 2 {
        return None;
    }
    // Only completed structures form the template; the live leg is still forming.
    let completed = &legs[..legs.len() - 1];
    let take = sym_k.max(1).min(completed.len());
    let recent = &completed[completed.len() - take..];
    let bars = recent.iter().map(|l| l.bars()).sum::<f64>() / take as f64;
    let amp = recent.iter().map(|l| l.amp().abs()).sum::<f64>() / take as f64;
    if !(bars > 0.0) || !(amp > 0.0) {
        return None;
    }
    let mut hits = 0usize;
    for l in recent {
        let bo = (l.bars() - bars).abs() / bars;
        let ao = (l.amp().abs() - amp).abs() / amp;
        if bo <= eq_tol && ao <= eq_tol {
            hits += 1;
        }
    }
    let score = hits as f64 / take as f64;
    let last_dir = if legs[legs.len() - 1].amp() >= 0.0 { 1 } else { -1 };
    Some(SdForecast { bars, amp, dir: -last_dir, score })
}

/// Structure-Based Supply/Demand - causal, length-repeat.
///
/// Every short-trend structure is measured (bar length + amplitude) WITHOUT a
/// pivot: a leg ends on the bar the causal trend flips, so nothing waits for a
/// swing to confirm. Because a structure price formed on one side tends to
/// repeat with the same length and size on the other, the recent completed legs
/// are averaged into a template and the next leg is drawn FORWARD - dashed,
/// before its candles exist - with the measured direction and length, so the
/// trader sees where and when the next structure is expected to land.
fn compute_supplydemand(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 1.0);
    let up = strv(o, "upColor", "#26a69a");
    let dn = strv(o, "downColor", "#ef5350");
    let proj_c = strv(o, "projColor", "#ffb74d");
    let cc = clean_candles(c);
    let n = cc.len();
    let empty = || SeriesOut::line(&up, lw);
    if n < 3 {
        return vec![empty()];
    }
    let trend_len = (num(o, "structLen", 5.0).round() as i64).max(2) as usize;
    let eq_tol = num(o, "eqTol", 25.0).max(0.0) / 100.0;
    let sym_k = int(o, "symLegs", 3).max(1) as usize;
    let proj_n = int(o, "projCount", 1).max(0) as usize;
    let show_proj = boolv(o, "showProjection", true);

    let legs = structure_legs(&cc, trend_len);
    if legs.is_empty() {
        return vec![empty()];
    }

    // Confirmed structure path: one straight leg per measured short-trend
    // structure, coloured by that leg's own direction (demand up / supply down).
    let mut data: Vec<Point> = Vec::new();
    for (k, l) in legs.iter().enumerate() {
        let col = if l.amp() >= 0.0 { up.clone() } else { dn.clone() };
        let mut seg = dense_seg(&cc, l.start_idx, l.start_px, l.end_idx, l.end_px);
        if k > 0 && !seg.is_empty() {
            seg.remove(0);
        }
        for p in seg.iter_mut() {
            p.color = Some(col.clone());
        }
        data.extend(seg);
    }
    // Live leg to the current bar so the line never floats away from price.
    if let Some(last) = legs.last() {
        if last.end_idx < n - 1 {
            let col = if cc[n - 1].close >= last.end_px { up.clone() } else { dn.clone() };
            let mut live = dense_seg(&cc, last.end_idx, last.end_px, n - 1, cc[n - 1].close);
            if !live.is_empty() {
                live.remove(0);
            }
            for p in live.iter_mut() {
                p.color = Some(col.clone());
            }
            data.extend(live);
        }
    }
    let s0 = mk_line(&up, lw, data, None);

    // Forecast: repeat the measured structure forward, before its candles exist.
    // Drawn dashed and excluded from autoscale so the hypothetical leg never
    // rescales the real chart.
    let mut s1 = mk_line(&proj_c, lw, Vec::new(), Some(2));
    s1.exclude_autoscale = true;
    if show_proj && proj_n > 0 {
        if let Some(f) = sd_forecast(&cc, trend_len, sym_k, eq_tol) {
            let interval = if n >= 2 && cc[n - 1].time > cc[n - 2].time {
                cc[n - 1].time - cc[n - 2].time
            } else {
                60
            };
            let bars = f.bars.round().max(1.0) as i64;
            let mut t = cc[n - 1].time;
            let mut px = cc[n - 1].close;
            let mut dir = if f.dir == 0 { 1 } else { f.dir };
            s1.data.push(Point { time: t, value: px, color: Some(proj_c.clone()) });
            for _ in 0..proj_n {
                let end_px = px + dir as f64 * f.amp;
                for s in 1..=bars {
                    let frac = s as f64 / bars as f64;
                    let v = px + (end_px - px) * frac;
                    s1.data.push(Point { time: t + s * interval, value: v, color: Some(proj_c.clone()) });
                }
                t += bars * interval;
                px = end_px;
                dir = -dir;
            }
            let side = if dir >= 0 { "bullish" } else { "bearish" };
            s1.title = Some(format!(
                "Next {side} structure · {bars} bars · {:.2} pts · repeat {:.0}%",
                f.amp,
                f.score * 100.0
            ));
        }
    }
    vec![s0, s1]
}

// ---------------------------------------------------------------------------
// Master Line - one fused, causal, non-repainting overlay
// ---------------------------------------------------------------------------

/// VWAP used by the Master Line's level bias and level line. Same rolling
/// window as the chart `vwap` overlay so both always agree.
fn running_vwap(cc: &[Candle]) -> Vec<f64> {
    rolling_vwap(cc, VWAP_WINDOW)
}

/// Previous completed session's high/low. Times are IST-naive wall clock, so the
/// session boundary is the wall-clock day (`time / 86400`).
fn prev_day_high_low(cc: &[Candle]) -> (Option<f64>, Option<f64>) {
    if cc.is_empty() {
        return (None, None);
    }
    let cur_day = cc[cc.len() - 1].time.div_euclid(86_400);
    let mut target: Option<i64> = None;
    let mut hi = f64::NEG_INFINITY;
    let mut lo = f64::INFINITY;
    for c in cc.iter().rev() {
        let d = c.time.div_euclid(86_400);
        if d == cur_day {
            continue;
        }
        if target.is_none() {
            target = Some(d);
        }
        if Some(d) != target {
            break;
        }
        if c.high > hi {
            hi = c.high;
        }
        if c.low < lo {
            lo = c.low;
        }
    }
    if target.is_some() && hi.is_finite() && lo.is_finite() {
        (Some(hi), Some(lo))
    } else {
        (None, None)
    }
}

/// Regime (trend vs range) per bar via the shared OI-trend hysteresis engine
/// (fast/slow EMA + ATR band). Returns (dir, is_flat).
fn regime_state(cc: &[Candle], fast: usize, slow: usize, k: f64, atr_len: usize) -> (Vec<i32>, Vec<bool>) {
    let n = cc.len();
    let mut rdir = vec![0i32; n];
    let mut rflat = vec![false; n];
    let opts = crate::oi_trend::RegimeOpts {
        fast,
        slow,
        enter_k: k,
        exit_k: k.min(0.12),
        atr_len,
        vol_avg_len: 20,
    };
    let rr = crate::oi_trend::regime_line(cc, &opts);
    for i in 0..n {
        match rr.regs.get(i).copied().unwrap_or(crate::oi_trend::Regime::Flat) {
            crate::oi_trend::Regime::Up => rdir[i] = 1,
            crate::oi_trend::Regime::Down => rdir[i] = -1,
            crate::oi_trend::Regime::Flat => {
                rdir[i] = 0;
                rflat[i] = true;
            }
        }
    }
    (rdir, rflat)
}

/// Fuse the six factors into one causal per-bar direction (+1/-1, carried):
/// trend/consensus confluence, level bias (VWAP + PDH/PDL + zones), regime
/// (trend-follow vs range mean-reversion), and the option-chain OI bias. No
/// pivot confirmation is waited on, so the line flips the bar the fused state
/// turns instead of lagging a swing.
fn master_fused_dir(cc: &[Candle], o: &Settings, rdir: &[i32], rflat: &[bool]) -> Vec<i32> {
    let n = cc.len();
    let vwap = running_vwap(cc);
    let (pdh, pdl) = prev_day_high_low(cc);
    let oi_bias = num(o, "oiBias", 0.0);
    let oi_dir = if oi_bias > 5.0 { 1.0 } else if oi_bias < -5.0 { -1.0 } else { 0.0 };
    let use_levels = boolv(o, "useLevels", true);
    let use_regime = boolv(o, "useRegime", true);
    let use_oi = boolv(o, "useOi", true);
    let mut dir = vec![0i32; n];
    let mut last = 0i32;
    for i in 0..n {
        let lvl = if use_levels {
            let mut b = if cc[i].close >= vwap[i] { 1i32 } else { -1 };
            if let Some(h) = pdh {
                if cc[i].close > h {
                    b += 1;
                }
            }
            if let Some(l) = pdl {
                if cc[i].close < l {
                    b -= 1;
                }
            }
            (b as f64).signum() as i32
        } else {
            0
        };
        let trend = rdir[i];
        let in_range = use_regime && rflat[i];
        let mut score = if in_range {
            -(lvl as f64) + 0.5 * (rdir[i] as f64)
        } else {
            (trend as f64) + (lvl as f64)
        };
        if use_oi {
            score += 0.5 * oi_dir;
        }
        let d = if score >= 0.5 {
            1
        } else if score <= -0.5 {
            -1
        } else {
            0
        };
        if d != 0 {
            last = d;
        }
        dir[i] = if d != 0 { d } else { last };
    }
    dir
}

/// Causal price zigzag: turns confirm the moment price retraces the threshold,
/// with no consensus/overlay gating, so the line always breaks into readable
/// legs instead of collapsing onto one long straight segment.
fn master_zigzag_vertices(cc: &[Candle], strength: usize, min_leg: usize) -> Vec<usize> {
    let mid: Vec<Option<f64>> = cc.iter().map(|x| Some((x.high + x.low) * 0.5)).collect();
    zigzag_vertices(&mid, &[], strength.max(2), min_leg.max(1), 0)
}

/// Master Line - one fused overlay that combines causal trend/consensus, level
/// supply/demand (VWAP, PDH/PDL, swing zones), regime (trend vs range), the live
/// option-chain OI bias (injected as the `oiBias` setting) and an ATR-based
/// expected-range projection. The line is non-repainting: swing turns freeze the
/// moment price retraces the threshold, and the live leg runs to the last bar.
fn compute_masterline(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 2.0);
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let flat = strv(o, "flatColor", "#6b6b88");
    let level_c = strv(o, "levelColor", "#7e57c2");
    let sup_c = strv(o, "supColor", "#26a69a");
    let res_c = strv(o, "resColor", "#ef5350");
    let proj_c = strv(o, "projColor", "#ffb74d");

    let cc = clean_candles(c);
    let n = cc.len();
    if n < 5 {
        return vec![SeriesOut::line(&flat, lw)];
    }
    let confirm = num(o, "confirm", 3.0).round().max(0.0) as usize;
    let strength = num(o, "strength", 5.0).round().max(2.0) as usize;
    let atr_len = (num(o, "atrLen", 14.0).round() as i64).max(2);
    let proj_mult = num(o, "projMult", 1.5).max(0.0);
    let proj_bars = (num(o, "projBars", 12.0).round() as i64).max(1) as usize;
    let zone_tol = num(o, "zoneTol", 0.5).max(0.0) / 100.0;
    let zone_n = (num(o, "zoneCount", 3.0).round() as i64).max(0) as usize;
    let fast = (num(o, "regimeFast", 9.0).round() as i64).max(2) as usize;
    let slow = (num(o, "regimeSlow", 21.0).round() as i64).max(3) as usize;
    let rk = num(o, "regimeK", 0.45).max(0.01);
    let use_proj = boolv(o, "useProjection", true);

    let (rdir, rflat) = regime_state(&cc, fast, slow, rk, atr_len as usize);
    let dir = master_fused_dir(&cc, o, &rdir, &rflat);
    let atr = last_finite(&wilder_arr(&tr_arr(&cc), atr_len));

    // Main fused zigzag: a causal price zigzag broken into readable legs (no
    // consensus gating), each leg coloured by its own slope.
    let vts = master_zigzag_vertices(&cc, strength, confirm);
    let mid: Vec<Option<f64>> = cc.iter().map(|x| Some((x.high + x.low) * 0.5)).collect();
    let mut s0 = if vts.len() >= 2 {
        let vals: Vec<f64> = vts.iter().map(|&i| mid[i].unwrap_or(f64::NAN)).collect();
        pivot_zigzag_line(&cc, &vts, &vals, &up, &dn, &flat, lw)
    } else {
        SeriesOut::line(&flat, lw)
    };

    // Level supply/demand reference lines.
    let cur = cc[n - 1].close;
    let vwap = running_vwap(&cc);
    let mut pls: Vec<PriceLine> = vec![PriceLine {
        price: (vwap[n - 1] * 100.0).round() / 100.0,
        color: level_c.clone(),
        line_width: 1.0,
        line_style: 2,
        title: "VWAP".into(),
    }];
    let (pdh, pdl) = prev_day_high_low(&cc);
    if let Some(h) = pdh {
        pls.push(PriceLine { price: (h * 100.0).round() / 100.0, color: res_c.clone(), line_width: 1.0, line_style: 2, title: "PDH".into() });
    }
    if let Some(l) = pdl {
        pls.push(PriceLine { price: (l * 100.0).round() / 100.0, color: sup_c.clone(), line_width: 1.0, line_style: 2, title: "PDL".into() });
    }
    if zone_n > 0 {
        let piv = fractal_pivots(&cc, strength as f64);
        let mut tol = (cur.abs() * zone_tol).max(atr * 0.6);
        if !(tol > 0.0) {
            tol = 1e-9;
        }
        let mut cl = cluster_levels(&piv, tol);
        cl.sort_by(|a, b| b.n.partial_cmp(&a.n).unwrap_or(std::cmp::Ordering::Equal));
        for l in cl.into_iter().take(zone_n) {
            let is_sup = l.price <= cur;
            pls.push(PriceLine {
                price: (l.price * 100.0).round() / 100.0,
                color: if is_sup { sup_c.clone() } else { res_c.clone() },
                line_width: 1.0,
                line_style: 2,
                title: if is_sup { "Demand".into() } else { "Supply".into() },
            });
        }
    }
    s0.price_lines = pls;
    // The Master Line is an annotation overlay, not a scale driver. Its level
    // lines (rolling VWAP, previous-day H/L, swing zones) can sit far from
    // the live price on a trending series; letting them into the autoscale
    // crushed the candles into a flat band and zoomed the chart out. Excluding
    // the whole series keeps the candles' own range as the scale while the
    // zigzag still draws (it lives inside the candle range) and any level line
    // that is actually in view still renders.
    s0.exclude_autoscale = true;

    // ATR volatility-normalised projection: expected range the next `projBars`.
    let mut s1 = mk_line(&proj_c, lw, Vec::new(), Some(2));
    s1.exclude_autoscale = true;
    if use_proj && atr > 0.0 && n >= 2 {
        let interval = if cc[n - 1].time > cc[n - 2].time { cc[n - 1].time - cc[n - 2].time } else { 60 };
        let d0 = if dir[n - 1] != 0 { dir[n - 1] } else { 1 };
        let end = cur + d0 as f64 * proj_mult * atr;
        s1.data.push(Point { time: cc[n - 1].time, value: (cur * 100.0).round() / 100.0, color: Some(proj_c.clone()) });
        for s in 1..=proj_bars {
            let v = cur + (end - cur) * (s as f64 / proj_bars as f64);
            s1.data.push(Point {
                time: cc[n - 1].time + s as i64 * interval,
                value: (v * 100.0).round() / 100.0,
                color: Some(proj_c.clone()),
            });
        }
        s1.title = Some(format!("ATR path {:+.1} / {} bars (±{:.1})", end - cur, proj_bars, proj_mult * atr));
    }

    vec![s0, s1]
}

fn markers_masterline(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let strength = num(o, "strength", 5.0).round().max(2.0) as usize;
    let confirm = num(o, "confirm", 3.0).round().max(0.0) as usize;
    let cc = clean_candles(c);
    if cc.len() < 5 {
        return Vec::new();
    }
    let vts = master_zigzag_vertices(&cc, strength, confirm);
    zigzag_turn_markers(&cc, &vts, &up, &dn)
}

fn compute_vl(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let n = c.len();
    let l = (num(o, "length", 14.0).round() as i64).max(1) as usize;
    let vol_len = (num(o, "volLen", 20.0).round() as i64).max(1) as usize;
    let lw = num(o, "lineWidth", 1.0);
    let base_color = strv(o, "color", "#26c6da");
    let mut vol_avg = vec![0.0; n];
    let mut v_sum = 0.0;
    for i in 0..n {
        let vol = c[i].volume;
        v_sum += vol;
        if i >= vol_len {
            v_sum -= c[i - vol_len].volume;
        }
        if i + 1 >= vol_len {
            vol_avg[i] = v_sum / vol_len as f64;
        }
    }
    let mut vw = vec![0.0; n];
    let mut pv = 0.0;
    let mut vsum = 0.0;
    for i in 0..n {
        let vol = c[i].volume;
        pv += c[i].close * vol;
        vsum += vol;
        if i >= l {
            let oo = c[i - l];
            let ov = oo.volume;
            pv -= oo.close * ov;
            vsum -= ov;
        }
        if i + 1 >= l && vsum > 0.0 {
            vw[i] = pv / vsum;
        }
    }
    let hx = base_color.trim_start_matches('#').to_string();
    let hx = if hx.len() == 3 { hx.chars().flat_map(|ch| [ch, ch]).collect::<String>() } else { hx };
    let (cr, cg, cb) = if hx.len() >= 6 {
        (
            u8::from_str_radix(&hx[0..2], 16).unwrap_or(0),
            u8::from_str_radix(&hx[2..4], 16).unwrap_or(0),
            u8::from_str_radix(&hx[4..6], 16).unwrap_or(0),
        )
    } else {
        (0x26, 0xc6, 0xda)
    };
    let mut main_data = Vec::new();
    for i in (l - 1)..n {
        let v = vw[i];
        if v == 0.0 && i + 1 < l {
            continue;
        }
        let vol = c[i].volume;
        let avg = if vol_avg[i] > 0.0 { vol_avg[i] } else { vol };
        let ratio = if avg > 0.0 { vol / avg } else { 1.0 };
        let mut alpha = 0.18 + ratio * 0.82;
        if alpha < 0.15 {
            alpha = 0.15;
        } else if alpha > 1.0 {
            alpha = 1.0;
        }
        main_data.push(Point {
            time: c[i].time,
            value: v,
            color: Some(format!("rgba({},{},{},{})", cr, cg, cb, alpha)),
        });
    }
    if boolv(o, "straight", true) {
        main_data = straighten_line(&main_data, num(o, "straightTol", 0.08));
    }
    let dir = live_trend_dir(c, num(o, "trendLen", 9.0).round().max(2.0) as usize);
    let mut m0 = mk_line(&base_color, lw, main_data, None);
    color_points_by_dir(c, &mut m0.data, &dir, "#26a69a", "#ef5350");
    vec![m0]
}

fn compute_rsidiv(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let lw = num(o, "lineWidth", 1.0);
    let len = int(o, "length", 14);
    let closes: Vec<f64> = c.iter().map(|x| x.close).collect();
    let rsi = rsi_wilder(&closes, len);
    let data = points_from(c, &rsi);
    vec![mk_line(&strv(o, "color", "#b39ddb"), lw, data, None)]
}

fn markers_rsidiv(c: &[Candle], o: &Settings) -> Vec<Marker> {
    let res = divergence(c, int(o, "length", 14), int(o, "pivot", 5), int(o, "lookback", 200));
    let show_reg = boolv(o, "showRegular", true);
    let show_hid = boolv(o, "showHidden", true);
    let bull_c = strv(o, "bullColor", "#00d4aa");
    let bear_c = strv(o, "bearColor", "#ff5252");
    let mut mk = Vec::new();
    for s in res {
        if s.hidden && !show_hid {
            continue;
        }
        if !s.hidden && !show_reg {
            continue;
        }
        let bull = s.kind_bull;
        let (shape, position, text) = if bull {
            (if s.hidden { "circle" } else { "arrowUp" }, "belowBar", if s.hidden { "hidden bull" } else { "bull" })
        } else {
            (if s.hidden { "circle" } else { "arrowDown" }, "aboveBar", if s.hidden { "hidden bear" } else { "bear" })
        };
        mk.push(Marker {
            time: s.time,
            position: position.into(),
            color: if bull { bull_c.clone() } else { bear_c.clone() },
            shape: shape.into(),
            text: text.into(),
            size: 1.0,
        });
    }
    mk
}

fn compute_vlcore(c: &[Candle], o: &Settings) -> Vec<SeriesOut> {
    let up = strv(o, "upColor", "#00e676");
    let dn = strv(o, "downColor", "#ff5252");
    let flat = strv(o, "flatColor", "#6b6b88");
    let lw = num(o, "lineWidth", 2.0);
    let cc = clean_candles(c);
    let n = cc.len();
    if n < 5 {
        return vec![SeriesOut::line(&flat, lw)];
    }
    // Trend Core is a causal price zigzag: straight legs between confirmed
    // swing turns, with no consensus/score gating, so it is pure structure and
    // never repaints. `length` is the retrace strength, `confirm` the minimum
    // bars between turns.
    let strength = num(o, "length", 5.0).round().clamp(2.0, 200.0) as usize;
    let min_leg = num(o, "confirm", 3.0).round().max(1.0) as usize;
    let vts = master_zigzag_vertices(&cc, strength, min_leg);
    if vts.len() < 2 {
        return vec![SeriesOut::line(&flat, lw)];
    }
    // A dense render: one point per bar along each leg (collinear on a straight
    // leg) so the line still draws as a zigzag while staying bar-aligned for the
    // indicator gates that read a value at a bar offset.
    let mid: Vec<Option<f64>> = cc.iter().map(|x| Some((x.high + x.low) * 0.5)).collect();
    let mut data = Vec::with_capacity(n);
    for k in 0..vts.len() - 1 {
        let (a, b) = (vts[k], vts[k + 1]);
        if b <= a {
            continue;
        }
        let (va, vb) = (mid[a].unwrap_or(f64::NAN), mid[b].unwrap_or(f64::NAN));
        if !va.is_finite() || !vb.is_finite() {
            continue;
        }
        let color = if vb >= va { &up } else { &dn };
        let span = (b - a) as f64;
        for i in a..b {
            let v = va + (vb - va) * ((i - a) as f64 / span);
            data.push(Point { time: cc[i].time, value: (v * 100.0).round() / 100.0, color: Some(color.clone()) });
        }
    }
    if let Some(&last) = vts.last() {
        if let Some(v) = mid[last] {
            data.push(Point { time: cc[last].time, value: (v * 100.0).round() / 100.0, color: None });
        }
    }
    vec![mk_line(&flat, lw, data, None)]
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------
pub fn registry() -> Vec<IndicatorEntry> {
    let mut v: Vec<IndicatorEntry> = Vec::new();

    v.push(e!(
        def("ema", "EMA", "Exponential Moving Average", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Length", 9.0, 1.0, 500.0, 1.0), source_in("close")],
            vec![color_st("color", "Color", "#2962ff"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_ema, "price"
    ));
    v.push(e!(
        def("ma", "MA", "Moving Average", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 500.0, 1.0), source_in("close")],
            vec![color_st("color", "Color", "#ff6d00"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_ma, "price"
    ));
    v.push(e!(
        def("smma", "Smoothed MA", "Smoothed Moving Average", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 500.0, 1.0), source_in("close")],
            vec![color_st("color", "Color", "#ffca28"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_smma, "price"
    ));
    v.push(e!(
        def("hma", "HMA", "Hull Moving Average", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 500.0, 1.0), source_in("close")],
            vec![color_st("color", "Color", "#00bcd4"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_hma, "price"
    ));
    v.push(e!(
        def("ao", "Awesome Oscillator", "Awesome Oscillator", "Momentum", IndType::Pane, None,
            vec![num_in("fast", "Fast length", 5.0, 1.0, 200.0, 1.0), num_in("slow", "Slow length", 34.0, 2.0, 500.0, 1.0)],
            vec![color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350")]),
        compute_ao, "number"
    ));
    v.push(e!(
        def("atr", "ATR", "Average True Range", "Volatility", IndType::Pane, None,
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#7e57c2"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_atr, "price"
    ));
    v.push(e!(
        def("adx", "ADX", "Average Directional Index", "Volatility", IndType::Pane, Some("percent"),
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0)],
            vec![color_st("adxColor", "ADX color", "#e040fb"), color_st("diPlusColor", "+DI color", "#26a69a"),
                 color_st("diMinusColor", "-DI color", "#ef5350"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_adx, "percent"
    ));
    v.push(e!(
        def("bollingerB", "Bollinger Bands %B", "Bollinger Bands %B", "Volatility", IndType::Pane, Some("decimal"),
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0), num_in("mult", "Mult", 2.0, 0.1, 10.0, 0.1)],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_bollinger_b, "decimal"
    ));
    v.push(e!(
        def("bbpct", "BB%b", "Bollinger %B", "Trend", IndType::Pane, Some("decimal"),
            vec![num_in("length", "Length", 20.0, 2.0, 200.0, 1.0), num_in("mult", "Std.dev mult", 2.0, 0.1, 5.0, 0.1)],
            vec![color_st("color", "Color", "#ffb300"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_bbpct, "decimal"
    ));
    v.push(e!(
        def("bbpctsl", "BB%b Straight Line", "BB%b Straight Line (Bollinger %B held as one price-trend line: green while %B stays above the bull threshold, red while below, turning only after a minimum leg)", "Trend", IndType::Overlay, None,
            vec![num_in("length", "BB length", 20.0, 2.0, 200.0, 1.0), num_in("mult", "Std.dev mult", 2.0, 0.1, 5.0, 0.1),
                 num_in("threshold", "%B bull threshold", 0.5, 0.0, 1.0, 0.05), num_in("minLeg", "Min leg", 5.0, 1.0, 200.0, 1.0),
                 source_in("close")],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_bbpctsl, "price"
    ));
    v.push(e!(
        def("bbpctslp", "BB%b Straightline Pane", "BB%b Straightline Pane (Bollinger %B held as one straight pane trend line: constant rising slope while %B stays above the bull threshold, constant falling slope while below, turning only after a minimum leg)", "Trend", IndType::Pane, Some("decimal"),
            vec![num_in("length", "BB length", 20.0, 2.0, 200.0, 1.0), num_in("mult", "Std.dev mult", 2.0, 0.1, 5.0, 0.1),
                 num_in("threshold", "%B bull threshold", 0.5, 0.0, 1.0, 0.05), num_in("minLeg", "Min leg", 5.0, 1.0, 200.0, 1.0),
                 num_in("step", "Trend step", 0.02, 0.0, 1.0, 0.01), source_in("close")],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_bbpctslp, "decimal"
    ));
    v.push(e!(
        def("padiv", "Price Action Diversionless", "Price Action Diversionless (Combines RSI divergence, Bollinger %B, EMA 9/21 and Supertrend(10,1)/(10,2) into ONE straight trend line. It flips only on a confirmed real reversal, and never on an in-trend divergence or fake band break)", "Trend", IndType::Pane, Some("decimal"),
            vec![num_in("rsiLen", "RSI length", 14.0, 2.0, 100.0, 1.0),
                 num_in("bbLen", "BB length", 20.0, 2.0, 200.0, 1.0), num_in("bbMult", "BB std.dev mult", 2.0, 0.1, 5.0, 0.1),
                 num_in("emaFast", "EMA fast", 9.0, 1.0, 200.0, 1.0), num_in("emaSlow", "EMA slow", 21.0, 2.0, 400.0, 1.0),
                 num_in("atrPeriod", "Supertrend ATR", 10.0, 1.0, 100.0, 1.0),
                 num_in("factorFast", "Supertrend fast factor", 1.0, 0.1, 10.0, 0.1), num_in("factorSlow", "Supertrend slow factor", 2.0, 0.1, 10.0, 0.1),
                 num_in("divWin", "Divergence window", 14.0, 2.0, 100.0, 1.0),
                 num_in("confirm", "Confirm bars", 3.0, 1.0, 50.0, 1.0), num_in("minLeg", "Min leg", 5.0, 1.0, 200.0, 1.0),
                 num_in("step", "Trend step", 0.02, 0.0, 1.0, 0.01), source_in("close")],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_padiv, "decimal"
    ));
    v.push(e!(
        def("padivo", "Price Action Diversionless (Price)", "Price Action Diversionless (Overlay) (The same confirmed real-reversal trend held as one straight price line; in-trend divergences and fake breaks are ignored)", "Trend", IndType::Overlay, None,
            vec![num_in("rsiLen", "RSI length", 14.0, 2.0, 100.0, 1.0),
                 num_in("bbLen", "BB length", 20.0, 2.0, 200.0, 1.0), num_in("bbMult", "BB std.dev mult", 2.0, 0.1, 5.0, 0.1),
                 num_in("emaFast", "EMA fast", 9.0, 1.0, 200.0, 1.0), num_in("emaSlow", "EMA slow", 21.0, 2.0, 400.0, 1.0),
                 num_in("atrPeriod", "Supertrend ATR", 10.0, 1.0, 100.0, 1.0),
                 num_in("factorFast", "Supertrend fast factor", 1.0, 0.1, 10.0, 0.1), num_in("factorSlow", "Supertrend slow factor", 2.0, 0.1, 10.0, 0.1),
                 num_in("divWin", "Divergence window", 14.0, 2.0, 100.0, 1.0),
                 num_in("confirm", "Confirm bars", 3.0, 1.0, 50.0, 1.0), num_in("minLeg", "Min leg", 5.0, 1.0, 200.0, 1.0),
                 source_in("close")],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_padivo, "price"
    ));
    v.push(e!(
        def("macd", "MACD", "MACD", "Momentum", IndType::Pane, None,
            vec![num_in("fast", "Fast length", 12.0, 1.0, 200.0, 1.0), num_in("slow", "Slow length", 26.0, 2.0, 500.0, 1.0),
                 num_in("signal", "Signal length", 9.0, 1.0, 200.0, 1.0)],
            vec![color_st("macdColor", "MACD line", "#2962ff"), color_st("signalColor", "Signal line", "#ff6d00"),
                 color_st("histUpColor", "Histogram up", "#26a69a"), color_st("histDownColor", "Histogram down", "#ef5350")]),
        compute_macd, "number"
    ));
    v.push(e!(
        def("supertrend", "Supertrend", "Supertrend", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 10.0, 1.0, 200.0, 1.0), num_in("factor", "Factor", 3.0, 0.1, 10.0, 0.1)],
            vec![color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_supertrend, "price"
    ));
    v.push(e!(
        def("stzigzag", "Supertrend ZigZag", "Supertrend ZigZag (Supertrend ATR 10 factor 1 and factor 0.8 averaged into one line and drawn as a straight-line zigzag through its swing turns; green rising / red falling)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 10.0, 1.0, 200.0, 1.0),
                 num_in("factorFast", "Fast factor", 1.0, 0.1, 10.0, 0.1),
                 num_in("factorSlow", "Slow factor", 0.8, 0.1, 10.0, 0.1),
                 num_in("swing", "Swing", 5.0, 1.0, 100.0, 1.0),
                 num_in("minLeg", "Min leg", 5.0, 1.0, 200.0, 1.0)],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "ZigZag width", 2.0, 1.0, 5.0, 1.0)]),
        compute_stzigzag, "price"
    ));
    v.push(e!(
        def("obv", "OBV", "On-Balance Volume", "Volume", IndType::Pane, None,
            vec![],
            vec![color_st("color", "Color", "#26a69a"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_obv, "number"
    ));
    v.push(e!(
        def("bb", "Bollinger Bands", "Bollinger Bands", "Volatility", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0), num_in("mult", "Mult", 2.0, 0.1, 10.0, 0.1)],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_bb, "price"
    ));
    v.push(e!(
        def("bbw", "BBW", "Bollinger Band Width", "Volume", IndType::Pane, Some("decimal"),
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0), num_in("mult", "Mult", 2.0, 0.1, 10.0, 0.1)],
            vec![color_st("color", "Color", "#ffb300"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_bbw, "number"
    ));
    v.push(e!(
        def("volosc", "Volume Oscillator", "Volume Oscillator", "Volume", IndType::Pane, Some("percent"),
            vec![num_in("fast", "Fast length", 5.0, 1.0, 200.0, 1.0), num_in("slow", "Slow length", 20.0, 2.0, 500.0, 1.0)],
            vec![color_st("color", "Color", "#26a69a"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_volosc, "percent"
    ));
    v.push(e!(
        def("ad", "Accumulation/Distribution", "Accumulation/Distribution Index", "Volume", IndType::Pane, None,
            vec![],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_ad, "number"
    ));
    v.push(e!(
        def("mfi", "MFI", "Money Flow Index", "Volume", IndType::Pane, Some("percent"),
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#ff6d00"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_mfi, "number"
    ));
    v.push(e!(
        def("pvt", "PVT", "Price Volume Trend", "Volume", IndType::Pane, None,
            vec![],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_pvt, "number"
    ));
    v.push(e!(
        def("dpo", "DPO", "Detrended Price Oscillator", "Momentum", IndType::Pane, None,
            vec![num_in("length", "Length", 21.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#7e57c2"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_dpo, "number"
    ));
    v.push(e!(
        def("ppo", "PPO", "Percentage Price Oscillator", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("fast", "Fast length", 12.0, 1.0, 200.0, 1.0), num_in("slow", "Slow length", 26.0, 2.0, 500.0, 1.0),
                 num_in("signal", "Signal length", 9.0, 1.0, 200.0, 1.0)],
            vec![color_st("ppoColor", "PPO line", "#2962ff"), color_st("signalColor", "Signal line", "#ff6d00"),
                 color_st("histUpColor", "Histogram up", "#26a69a"), color_st("histDownColor", "Histogram down", "#ef5350")]),
        compute_ppo, "percent"
    ));
    v.push(e!(
        def("williamsR", "Williams %R", "Williams Percent Range", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#e040fb"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_williams_r, "number"
    ));
    v.push(e!(
        def("rsi", "RSI", "Relative Strength Index", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0), source_in("close")],
            vec![color_st("color", "Color", "#7e57c2"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_rsi, "number"
    ));
    v.push(e!(
        def("uo", "UO", "Ultimate Oscillator", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("fast", "Fast length", 7.0, 1.0, 100.0, 1.0), num_in("mid", "Mid length", 14.0, 2.0, 200.0, 1.0),
                 num_in("slow", "Slow length", 28.0, 3.0, 500.0, 1.0)],
            vec![color_st("color", "Color", "#ff6d00"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_uo, "number"
    ));
    v.push(e!(
        def("vwap", "VWAP", "Volume Weighted Average Price", "Overlay", IndType::Overlay, None,
            vec![],
            vec![color_st("color", "Color", "#ffb300"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_vwap, "price"
    ));
    v.push(e!(
        def("pc", "Price Channel", "Price Channel", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 500.0, 1.0)],
            vec![color_st("color", "Color", "#26a69a"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_pc, "price"
    ));
    v.push(e!(
        def("cmf", "CMF", "Chaikin Money Flow", "Volume", IndType::Pane, None,
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_cmf, "number"
    ));
    v.push(e!(
        def("cci", "CCI", "Commodity Channel Index", "Momentum", IndType::Pane, None,
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#e040fb"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_cci, "number"
    ));
    v.push(e!(
        def("aroon", "Aroon", "Aroon Up/Down", "Trend", IndType::Pane, Some("percent"),
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0)],
            vec![color_st("upColor", "Aroon Up", "#26a69a"), color_st("downColor", "Aroon Down", "#ef5350"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_aroon, "number"
    ));
    v.push(e!(
        def("vortex", "Vortex", "Vortex Indicator (VI+ / VI-)", "Trend", IndType::Pane, None,
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0)],
            vec![color_st("plusColor", "VI+ color", "#26a69a"), color_st("minusColor", "VI- color", "#ef5350"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_vortex, "number"
    ));
    v.push(e!(
        def("tsi", "TSI", "True Strength Index", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("long", "Long length", 25.0, 1.0, 200.0, 1.0), num_in("short", "Short length", 13.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#2962ff"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_tsi, "number"
    ));
    v.push(e!(
        def("donchian", "Donchian Channel", "Donchian Channel", "Trend", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 500.0, 1.0)],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_donchian, "price"
    ));
    v.push(e!(
        def("stochrsi", "Stoch RSI", "Stochastic RSI (K/D)", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("rsiLength", "RSI length", 14.0, 1.0, 200.0, 1.0), num_in("stochLength", "Stoch length", 14.0, 1.0, 200.0, 1.0),
                 num_in("k", "K smoothing", 3.0, 1.0, 50.0, 1.0), num_in("d", "D smoothing", 3.0, 1.0, 50.0, 1.0)],
            vec![color_st("kColor", "K color", "#2962ff"), color_st("dColor", "D color", "#ff6d00"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_stochrsi, "number"
    ));
    v.push(e!(
        def("elderforce", "Force Index", "Elder Force Index", "Momentum", IndType::Pane, None,
            vec![num_in("length", "Length", 13.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#7e57c2"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_elderforce, "number"
    ));
    v.push(e!(
        def("keltner", "Keltner Channels", "Keltner Channels (EMA + ATR)", "Volatility", IndType::Overlay, None,
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0), num_in("mult", "Mult", 2.0, 0.1, 10.0, 0.1)],
            vec![color_st("color", "Color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_keltner, "price"
    ));
    v.push(e!(
        def("chandelier", "Chandelier Exit", "Chandelier Exit (ATR trailing)", "Volatility", IndType::Overlay, None,
            vec![num_in("length", "Length", 22.0, 1.0, 200.0, 1.0), num_in("mult", "Mult", 3.0, 0.1, 10.0, 0.1)],
            vec![color_st("longColor", "Long color", "#26a69a"), color_st("shortColor", "Short color", "#ef5350"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_chandelier, "price"
    ));
    v.push(e!(
        def("sqzmom", "Squeeze Momentum", "TTM Squeeze Momentum", "Volatility", IndType::Pane, None,
            vec![num_in("length", "Length", 20.0, 1.0, 200.0, 1.0), num_in("mult", "Mult", 2.0, 0.1, 10.0, 0.1)],
            vec![color_st("color", "Color", "#7e57c2"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_sqzmom, "number"
    ));
    v.push(e!(
        def("fisher", "Fisher Transform", "Fisher Transform (MESA)", "Momentum", IndType::Pane, None,
            vec![num_in("length", "Length", 9.0, 1.0, 200.0, 1.0)],
            vec![color_st("color", "Color", "#2962ff"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_fisher, "number"
    ));
    v.push(e!(
        def("ichimoku", "Ichimoku", "Ichimoku Cloud", "Trend", IndType::Overlay, None,
            vec![num_in("tenkan", "Tenkan", 9.0, 1.0, 200.0, 1.0), num_in("kijun", "Kijun", 26.0, 1.0, 200.0, 1.0),
                 num_in("senkou", "Senkou B", 52.0, 1.0, 500.0, 1.0)],
            vec![color_st("tenkanColor", "Tenkan color", "#2962ff"), color_st("kijunColor", "Kijun color", "#ef5350"),
                 color_st("spanAColor", "Span A color", "#26a69a"), color_st("spanBColor", "Span B color", "#ff6d00"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_ichimoku, "price"
    ));

    v.push(e!(
        def("pastruct", "Price Action Trend", "Price Action Structure (swing HH/HL + BOS/CHoCH, non-repaint)", "Trend", IndType::Overlay, None,
            vec![num_in("pivotLen", "Pivot length", 3.0, 1.0, 50.0, 1.0),
                 num_in("atrLen", "ATR length", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 0.25, 0.05, 5.0, 0.05),
                 enum_in("lineMode", "Line mode", "zigzag", &[("zigzag", "ZigZag"), ("trail", "Trailing")]),
                 check_in("markersOnly", "Markers only", false)],
            vec![bool_st("showMarkers", "Show markers", true),
                 color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_pastruct, markers_pastruct, "price"
    ));
    v.push(e!(
        def("pcr", "PCR EMA", "Put-Call Ratio + EMA overlay (pinned to a bottom band)", "Overlay", IndType::Overlay, Some("decimal"),
            vec![],
            vec![color_st("pcrColor", "PCR color", "#9e9e9e"), color_st("fastColor", "Fast color", "#00d4aa"),
                 color_st("slowColor", "Slow color", "#ff9800"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_pcr, "number"
    ));
    v.push(e!(
        def("pcrrail", "OI Rails", "OI Support/Resistance Rails + PCR (right-extended)", "Overlay", IndType::Overlay, None,
            vec![],
            vec![num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_pcrrail, "number"
    ));
    v.push(e!(
        def("iv", "IV", "Implied Volatility (IV) + Delta + Vega - live option greeks scaled onto the price axis as straight intersecting lines", "Volatility", IndType::Overlay, Some("decimal"),
            vec![],
            vec![color_st("color", "IV color", "#e040fb"), color_st("deltaColor", "Delta color", "#00bcd4"),
                 color_st("vegaColor", "Vega color", "#ff6d00"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_iv, "number"
    ));
    v.push(e!(
        def("projline", "Trend Projection", "Trend Projection (segment-trend-wise zigzag through the ATR swings, last leg projected into the future)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05),
                 num_in("fwd", "Project bars", 30.0, 1.0, 500.0, 1.0)],
            vec![color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 color_st("histColor", "History color", "#7ee0ff"), color_st("projColor", "Projection color", "#b388ff"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_projline, markers_projline, "price"
    ));
    v.push(e!(
        def("wavefib", "Elliott Wave Trend", "Elliott wave structure: 1-2-3-4-5 / A-B-C labels + a single zigzag wave line", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05)],
            vec![bool_st("showTrend", "Show trend", true), bool_st("showFib", "Show fib", false),
                 bool_st("showExt", "Show extensions", false), bool_st("showLabels", "Show labels", true),
                 color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 color_st("rangeColor", "Range color", "#9e9e9e"), color_st("fibColor", "Fib color", "#ffd54f"),
                 color_st("fibExtColor", "Fib ext color", "#ff8a65"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_wavefib, markers_wavefib, "price"
    ));
    v.push(e!(
        def_hidden("ewtrend", "Elliott Wave Trend (state)", "Elliott wave confirmed trend state (hidden; drives the AST filter)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 6.0, 0.1, 20.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05)],
            vec![]),
        compute_ewtrend, "number"
    ));
    v.push(e!(
        def_hidden("patrend", "Price Action Trend (state)", "Price Action structure trend state (hidden; drives the AST filter)", "Trend", IndType::Overlay, None,
            vec![num_in("pivotLen", "Pivot length", 10.0, 1.0, 50.0, 1.0),
                 num_in("atrLen", "ATR length", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 0.25, 0.05, 5.0, 0.05)],
            vec![]),
        compute_patrend, "number"
    ));
    v.push(e!(
        def("keylevel", "Key Levels", "Key Levels (support/resistance zones scored by how often price respected them)", "Overlay", IndType::Overlay, None,
            vec![num_in("strength", "Pivot strength", 5.0, 1.0, 50.0, 1.0),
                 num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("minPct", "Min level %", 0.08, 0.0, 5.0, 0.01),
                 num_in("tolMult", "Cluster tol mult", 0.6, 0.1, 5.0, 0.1),
                 num_in("zones", "Zones", 5.0, 1.0, 10.0, 1.0)],
            vec![color_st("c1", "Level 1", "#2962ff"), color_st("c2", "Level 2", "#ff9800"),
                 color_st("c3", "Level 3", "#ef5350"), color_st("c4", "Level 4", "#26a69a"),
                 color_st("c5", "Level 5", "#ab47bc"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_keylevel, "price"
    ));
    v.push(e!(
        def("autotrend", "Auto Trendline", "Auto Trendline (segment-trend-wise zigzag through the confirmed swing pivots)", "Overlay", IndType::Overlay, None,
            vec![num_in("strength", "Pivot strength", 5.0, 1.0, 50.0, 1.0),
                 num_in("look", "Look back", 60.0, 3.0, 500.0, 1.0),
                 num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("minPct", "Min line %", 0.05, 0.0, 5.0, 0.01),
                 num_in("tolMult", "Tol mult", 0.5, 0.05, 5.0, 0.05),
                 check_in("fullSpan", "Full span", true)],
            vec![color_st("upColor", "Support color", "#26a69a"), color_st("downColor", "Resistance color", "#ef5350"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_autotrend, markers_autotrend, "price"
    ));
    v.push(e!(
        def("smiio", "SMI Ergodic Oscillator", "SMI Ergodic Oscillator", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("shortlen", "Short length", 13.0, 1.0, 100.0, 1.0),
                 num_in("longlen", "Long length", 25.0, 2.0, 200.0, 1.0),
                 num_in("siglen", "Signal length", 9.0, 1.0, 100.0, 1.0)],
            vec![color_st("color", "SMI color", "#7e57c2"), color_st("signalColor", "Signal color", "#ff6d00"),
                 color_st("histColor", "Histogram color", "#42a5f5"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_smiio, "number"
    ));
    v.push(e!(
        def("smf", "SMF", "Smart Money Flow (volume-pulse accumulation/distribution)", "Volume", IndType::Pane, Some("percent"),
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0),
                 num_in("signalLen", "Signal length", 9.0, 1.0, 100.0, 1.0),
                 num_in("volLen", "Volume length", 20.0, 1.0, 200.0, 1.0),
                 num_in("pulseCap", "Pulse cap", 3.0, 1.0, 10.0, 0.1)],
            vec![color_st("color", "SMF color", "#26c6da"), color_st("signalColor", "Signal color", "#ff6d00"),
                 color_st("histUpColor", "Histogram up", "#26a69a"), color_st("histDownColor", "Histogram down", "#ef5350"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_smf, "number"
    ));
    v.push(e!(
        def("autosr", "Auto Support Resistance", "Auto Support Resistance (structure high/low lines)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min move %", 0.15, 0.0, 5.0, 0.05),
                 check_in("touchLine", "Touch line", true)],
            vec![color_st("resColor", "Resistance color", "#26a69a"), color_st("supColor", "Support color", "#ef5350"),
                 color_st("touchUpColor", "Touch up color", "#26a69a"), color_st("touchDownColor", "Touch down color", "#ef5350"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_autosr, "price"
    ));
    v.push(e!(
        def("zzline", "ZigZag Trendline", "ATR ZigZag structure + straight pivot-fit intersection trendline (non-repaint)", "Trend", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05),
                 num_in("pivotLook", "Pivot look", 8.0, 2.0, 100.0, 1.0),
                 check_in("showZig", "Show zigzag", true), check_in("showLine", "Show trend line", true),
                 check_in("showBreaks", "Show breaks", true), check_in("fullSpan", "Full span", true)],
            vec![color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 color_st("trendColor", "Trend color", "#2962ff"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_zzline, markers_zzline, "price"
    ));
    v.push(e!(
        def("trendmaster", "Combo Master", "Combo Master: structure zigzag + straight intersection trendline + BOS/CHoCH + volume confirmation (non-repaint)", "Trend", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05),
                 num_in("trendLook", "Trend look", 12.0, 2.0, 100.0, 1.0),
                 check_in("lineOnly", "Line only", false), check_in("showLevel", "Show level", false),
                 check_in("showBreaks", "Show breaks", true), check_in("fullSpan", "Full span", true)],
            vec![color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 color_st("trendColor", "Trend color", "#2962ff"), color_st("levelColor", "Level color", "#ffb300"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_trendmaster, markers_trendmaster, "price"
    ));
    v.push(e!(
        def("panemaster", "Pane Consensus Signal", "Pane Consensus Signal (all pane oscillators fused into one segment-trend-wise zigzag line: one straight leg per swing, flipping at every real turn)", "Trend", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("rsiLength", "RSI length", 14.0, 2.0, 100.0, 1.0),
                 num_in("bbLength", "BB length", 20.0, 2.0, 200.0, 1.0),
                 num_in("cciLength", "CCI length", 20.0, 2.0, 200.0, 1.0),
                 num_in("mfiLength", "MFI length", 14.0, 2.0, 200.0, 1.0),
                 num_in("stochLength", "Stoch length", 14.0, 2.0, 200.0, 1.0),
                 num_in("willrLength", "Williams length", 14.0, 2.0, 200.0, 1.0),
                 num_in("smooth", "Smooth", 3.0, 1.0, 50.0, 1.0),
                 num_in("fitLook", "Fit look", 60.0, 5.0, 300.0, 1.0),
                 num_in("minSeg", "Min segment", 3.0, 1.0, 50.0, 1.0),
                 num_in("bullTh", "Bull threshold", 3.0, 1.0, 8.0, 1.0),
                 num_in("bearTh", "Bear threshold", 3.0, 1.0, 8.0, 1.0),
                 check_in("useRSI", "Use RSI", true), check_in("useBB", "Use BB", true),
                 check_in("useStoch", "Use Stoch", true), check_in("useCCI", "Use CCI", true),
                 check_in("useWillR", "Use Williams", true), check_in("useMFI", "Use MFI", true),
                 check_in("useMACD", "Use MACD", true), check_in("useDiv", "Use divergence", true)],
            vec![color_st("upColor", "Up color", "#26a69a"), color_st("downColor", "Down color", "#ef5350"),
                 color_st("trendColor", "Trend color", "#2962ff"), color_st("exhaustColor", "Exhaustion color", "#ffb300"),
                 bool_st("showExhaustion", "Show exhaustion", true), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_panemaster, markers_panemaster, "number"
    ));
    v.push(e!(
        def("pitchfork", "Pitchfork", "Andrews Pitchfork (median + parallel channel from 3 confirmed swings)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05),
                 num_in("minSpan", "Min span", 12.0, 3.0, 200.0, 1.0)],
            vec![color_st("medianColor", "Median color", "#2962ff"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_pitchfork, markers_pitchfork, "price"
    ));
    v.push(e!(
        def("fibfan", "Fibonacci Fan", "Fibonacci Fan (rays from a confirmed swing through fib ratios)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05)],
            vec![color_st("fanColor", "Fan color", "#ab47bc"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_fibfan, markers_fibfan, "price"
    ));
    v.push(e!(
        def("gannfan", "Gann Fan", "Gann Fan (1x8 ... 8x1 rays scaled to the confirmed swing)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05)],
            vec![color_st("fanColor", "Fan color", "#607d8b"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_gannfan, markers_gannfan, "price"
    ));
    v.push(e!(
        def_hidden("fibt", "Fibonacci Fan (state)", "Fibonacci Fan swing direction state (hidden; drives the AST filter)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05)],
            vec![]),
        compute_fibt, "price"
    ));
    v.push(e!(
        def_hidden("gant", "Gann Fan (state)", "Gann Fan swing direction state (hidden; drives the AST filter)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05)],
            vec![]),
        compute_gant, "price"
    ));
    v.push(e!(
        def("srema", "S/R EMA Reversal", "Support Resistance EMA(1) Reversal (straight line)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05),
                 num_in("emaPeriod", "EMA period", 1.0, 1.0, 100.0, 1.0),
                 num_in("touchMult", "Touch mult", 0.5, 0.05, 5.0, 0.05),
                 num_in("touchWindow", "Touch window", 3.0, 1.0, 50.0, 1.0),
                 num_in("fwd", "Forward bars", 20.0, 1.0, 200.0, 1.0)],
            vec![color_st("bullColor", "Bull color", "#26a69a"), color_st("bearColor", "Bear color", "#ef5350"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_srema, markers_srema, "price"
    ));
    v.push(e!(
        def("slema", "ZigZag EMA", "ZigZag EMA (exponential moving average joined by straight zigzag legs through its swing turns; green rising / red falling)", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Length", 9.0, 1.0, 500.0, 1.0), source_in("close"),
                 num_in("swing", "Swing", 5.0, 1.0, 100.0, 1.0),
                 num_in("minLeg", "Min leg", 5.0, 1.0, 200.0, 1.0)],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_slema, markers_slema, "price"
    ));
    v.push(e!(
        def_hidden("sremat", "S/R EMA Reversal (state)", "Support Resistance EMA(1) reversal direction state (hidden; drives the AST filter)", "Overlay", IndType::Overlay, None,
            vec![num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("atrMult", "ATR mult", 2.0, 0.1, 10.0, 0.1),
                 num_in("minPct", "Min pivot %", 0.15, 0.0, 5.0, 0.05),
                 num_in("emaPeriod", "EMA period", 1.0, 1.0, 100.0, 1.0),
                 num_in("touchMult", "Touch mult", 0.5, 0.05, 5.0, 0.05),
                 num_in("touchWindow", "Touch window", 3.0, 1.0, 50.0, 1.0),
                 num_in("fwd", "Forward bars", 20.0, 1.0, 200.0, 1.0)],
            vec![]),
        compute_sremat, "number"
    ));
    v.push(e!(
        def("supplydemand", "Structure Based Supply Demand", "Structure Based Supply Demand (measures each short-trend structure's bar length and amplitude causally - a leg ends the bar the trend flips, no pivot confirmation lag - then repeats the measured length forward to project the next structure before its candles form; dashed forecast + repeat-confidence shows where and when the next structure lands)", "Overlay", IndType::Overlay, None,
            vec![num_in("structLen", "Structure trend", 5.0, 2.0, 100.0, 1.0),
                 num_in("symLegs", "Structure legs", 3.0, 1.0, 12.0, 1.0),
                 num_in("eqTol", "Equal tol %", 25.0, 0.0, 100.0, 1.0),
                 num_in("projCount", "Project structures", 1.0, 0.0, 6.0, 1.0),
                 check_in("showProjection", "Show forecast", true)],
            vec![color_st("upColor", "Demand color", "#26a69a"), color_st("downColor", "Supply color", "#ef5350"),
                 color_st("liveColor", "Live color", "#7ee0ff"), color_st("projColor", "Forecast color", "#ffb74d"),
                 num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_supplydemand, markers_supplydemand, "price"
    ));
    v.push(e!(
        def("masterline", "Master Line", "Master Line (one fused, causal, readable overlay: a non-repainting price zigzag whose legs flip the bar price retraces the threshold + level supply/demand from VWAP, previous-day high/low and swing zones + regime-aware trend-vs-range logic + live option-chain OI bias + an ATR volatility-normalised expected-range projection)", "Overlay", IndType::Overlay, None,
            vec![num_in("confirm", "Confirm bars", 3.0, 0.0, 30.0, 1.0),
                 num_in("strength", "Swing strength", 5.0, 2.0, 50.0, 1.0),
                 num_in("regimeFast", "Regime fast EMA", 9.0, 2.0, 100.0, 1.0),
                 num_in("regimeSlow", "Regime slow EMA", 21.0, 3.0, 200.0, 1.0),
                 num_in("regimeK", "Regime enter k", 0.45, 0.05, 3.0, 0.05),
                 num_in("atrLen", "ATR length", 14.0, 2.0, 200.0, 1.0),
                 num_in("projMult", "Projection ATR mult", 1.5, 0.1, 10.0, 0.1),
                 num_in("projBars", "Projection bars", 12.0, 1.0, 200.0, 1.0),
                 num_in("zoneTol", "Zone tol %", 0.5, 0.05, 5.0, 0.05),
                 num_in("zoneCount", "Zones", 3.0, 0.0, 10.0, 1.0),
                 num_in("oiBias", "OI bias (-100..100)", 0.0, -100.0, 100.0, 1.0),
                 check_in("useLevels", "Levels / zones", true),
                 check_in("useRegime", "Regime aware", true),
                 check_in("useOi", "OI bias", true),
                 check_in("useProjection", "ATR projection", true)],
            vec![color_st("upColor", "Bull color", "#00e676"), color_st("downColor", "Bear color", "#ff5252"),
                 color_st("flatColor", "Flat color", "#6b6b88"), color_st("levelColor", "VWAP color", "#7e57c2"),
                 color_st("supColor", "Demand color", "#26a69a"), color_st("resColor", "Supply color", "#ef5350"),
                 color_st("projColor", "Projection color", "#ffb74d"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_masterline, markers_masterline, "price"
    ));
    v.push(e!(
        def("slconsensus", "Straight Line Consensus", "Straight Line Consensus (majority vote of the twelve straight-line indicators drawn as a segment-trend-wise zigzag: one straight leg per aggregate swing, flipping at every real turn)", "Overlay", IndType::Overlay, None,
            vec![num_in("minAgree", "Min net votes", 2.0, 1.0, 12.0, 1.0),
                 num_in("confirm", "Confirm bars", 5.0, 0.0, 30.0, 1.0),
                 num_in("strength", "Swing strength", 5.0, 2.0, 50.0, 1.0)],
            vec![color_st("upColor", "Bull color", "#00e676"), color_st("downColor", "Bear color", "#ff5252"),
                 color_st("flatColor", "Flat color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_slconsensus, markers_slconsensus, "price"
    ));
    v.push(e!(
        def("ovlconsensus", "Overlay Consensus", "Overlay Consensus (the twelve classic overlay indicators - EMA, MA, SMMA, HMA, Bollinger, Keltner, Price Channel, Donchian, Chandelier, Ichimoku, Supertrend, VWAP - averaged and drawn as a segment-trend-wise zigzag: one straight leg per aggregate swing, flipping at every real turn)", "Overlay", IndType::Overlay, None,
            vec![num_in("minAgree", "Min agree", 2.0, 1.0, 12.0, 1.0),
                 num_in("confirm", "Min leg bars", 5.0, 0.0, 30.0, 1.0),
                 num_in("strength", "Pivot strength", 5.0, 2.0, 50.0, 1.0)],
            vec![color_st("upColor", "Bull color", "#00e676"), color_st("downColor", "Bear color", "#ff5252"),
                 color_st("flatColor", "Flat color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_ovlconsensus, markers_ovlconsensus, "price"
    ));
    v.push(e!(
        def("slallconsensus", "Overall Straight Line Consensus", "Overall Straight Line Consensus (every straight-line indicator votes its own slope; the strong majority, confirmed for several bars and gated on a real swing-structure break, is held as ONE straight trend line that follows the overall trend and flips only on a major reversal - not a zigzag)", "Trend", IndType::Pane, Some("decimal"),
            vec![num_in("minAgree", "Min net votes", 12.0, 1.0, 18.0, 1.0),
                 num_in("confirm", "Confirm bars", 10.0, 0.0, 60.0, 1.0),
                 num_in("strength", "Swing strength", 5.0, 2.0, 50.0, 1.0),
                 num_in("trendLen", "Voter trend length", 34.0, 2.0, 400.0, 1.0),
                 num_in("step", "Trend step", 0.02, 0.0, 1.0, 0.01)],
            vec![color_st("upColor", "Bull color", "#26a69a"), color_st("downColor", "Bear color", "#ef5350"),
                 color_st("flatColor", "Flat color", "#6b6b88"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_slallconsensus, "decimal"
    ));
    v.push(e!(
        def("slallconsensuso", "Overall Straight Line Consensus (Price)", "Overall Straight Line Consensus (Overlay) (the same overall-trend consensus of every straight-line indicator held as one straight price line per trend leg - rising on a bull leg, falling on a bear leg, never flat - reversing only at a major confirmed flip)", "Trend", IndType::Overlay, None,
            vec![num_in("minAgree", "Min net votes", 12.0, 1.0, 18.0, 1.0),
                 num_in("confirm", "Confirm bars", 10.0, 0.0, 60.0, 1.0),
                 num_in("strength", "Swing strength", 5.0, 2.0, 50.0, 1.0),
                 num_in("trendLen", "Voter trend length", 34.0, 2.0, 400.0, 1.0),
                 num_in("minSlopePct", "Min slope %/bar", 0.0005, 0.0, 0.02, 0.0001)],
            vec![color_st("upColor", "Bull color", "#26a69a"), color_st("downColor", "Bear color", "#ef5350"),
                 color_st("flatColor", "Flat color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_slallconsensuso, "price"
    ));
    v.push(e!(
        def("supline", "Support Trendline", "Support Trendline (single straight line through multiple swing lows, extended forward)", "Overlay", IndType::Overlay, None,
            vec![num_in("strength", "Pivot strength", 5.0, 2.0, 50.0, 1.0),
                 num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("minPct", "Min tol %", 0.05, 0.0, 5.0, 0.05),
                 num_in("tolMult", "Tol ATR mult", 0.5, 0.0, 10.0, 0.1),
                 num_in("look", "Pivots to scan", 12.0, 3.0, 400.0, 1.0),
                 num_in("fwd", "Forward bars", 10.0, 0.0, 200.0, 1.0),
                 check_in("fullSpan", "Full span", false)],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_supline, markers_supline, "price"
    ));
    v.push(e!(
        def("resline", "Resistance Trendline", "Resistance Trendline (single straight line through multiple swing highs, extended forward)", "Overlay", IndType::Overlay, None,
            vec![num_in("strength", "Pivot strength", 5.0, 2.0, 50.0, 1.0),
                 num_in("atrPeriod", "ATR period", 14.0, 2.0, 200.0, 1.0),
                 num_in("minPct", "Min tol %", 0.05, 0.0, 5.0, 0.05),
                 num_in("tolMult", "Tol ATR mult", 0.5, 0.0, 10.0, 0.1),
                 num_in("look", "Pivots to scan", 12.0, 3.0, 400.0, 1.0),
                 num_in("fwd", "Forward bars", 10.0, 0.0, 200.0, 1.0),
                 check_in("fullSpan", "Full span", false)],
            vec![color_st("upColor", "Rising color", "#26a69a"), color_st("downColor", "Falling color", "#ef5350"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_resline, markers_resline, "price"
    ));
    v.push(e!(
        def("vl", "VL", "Volume Line (volume-weighted trend, battery fade)", "Volume", IndType::Overlay, None,
            vec![num_in("length", "Length", 14.0, 1.0, 200.0, 1.0),
                 num_in("signalLen", "Signal length", 9.0, 1.0, 100.0, 1.0),
                 num_in("volLen", "Volume length", 20.0, 1.0, 200.0, 1.0),
                 check_in("straight", "Straighten", true)],
            vec![color_st("color", "Color", "#26c6da"), color_st("signalColor", "Signal color", "#ff6d00"),
                 num_st("straightTol", "Straighten tol", 0.08, 0.0, 1.0, 0.01), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_vl, markers_vl, "price"
    ));
    v.push(e!(
        def("rsidiv", "RSI Divergence", "RSI Divergence (Regular + Hidden)", "Momentum", IndType::Pane, Some("percent"),
            vec![num_in("length", "RSI length", 14.0, 2.0, 100.0, 1.0),
                 num_in("pivot", "Pivot strength", 5.0, 1.0, 50.0, 1.0),
                 num_in("lookback", "Look back", 200.0, 10.0, 1000.0, 1.0),
                 check_in("showRegular", "Show regular", true), check_in("showHidden", "Show hidden", true)],
            vec![color_st("color", "RSI color", "#b39ddb"), color_st("bullColor", "Bull color", "#00d4aa"),
                 color_st("bearColor", "Bear color", "#ff5252"), num_st("lineWidth", "Line width", 1.0, 1.0, 5.0, 1.0)]),
        compute_rsidiv, markers_rsidiv, "number"
    ));
    v.push(e!(
        def("vlcore", "Trend Core", "Trend Core (causal price zigzag - straight legs between confirmed swing turns, non-repainting)", "Overlay", IndType::Overlay, None,
            vec![num_in("length", "Swing strength", 5.0, 1.0, 200.0, 1.0),
                 num_in("confirm", "Min leg", 3.0, 1.0, 50.0, 1.0)],
            vec![color_st("upColor", "Rising color", "#00e676"), color_st("downColor", "Falling color", "#ff5252"),
                 color_st("flatColor", "Neutral color", "#6b6b88"), num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_vlcore, markers_vlcore, "price"
    ));

    // "Double Line" family: every straight-line overlay indicator (the EMA one
    // excluded) plus Trend Core, each rendered as its main straight line + a
    // signal line that is a normal EMA of that line (see `double_line_series`).
    // The registry entry only carries the catalog definition - the base
    // indicator's inputs/style cloned plus the signal-length/colour controls -
    // while the actual line maths runs in the chart layer, so the base
    // indicator is never duplicated here.
    const DL_BASES: &[&str] = &[
        "bbpctsl", "padivo", "stzigzag", "pastruct", "projline", "wavefib", "autotrend",
        "zzline", "trendmaster", "panemaster", "pitchfork", "fibfan", "gannfan", "srema",
        "supplydemand", "slconsensus", "ovlconsensus", "slallconsensuso", "supline", "resline",
        "vl", "vlcore",
    ];
    let base_defs: Vec<(IndicatorDef, &'static str)> = DL_BASES
        .iter()
        .filter_map(|b| {
            v.iter()
                .find(|e| &e.def.id == *b)
                .map(|e| (e.def.clone(), e.reading))
        })
        .collect();
    for (bdef, reading) in base_defs {
        let base_name = bdef.name.clone();
        let mut d = bdef;
        let base_id = d.id.clone();
        d.id = format!("dl_{base_id}");
        d.name = format!("Double Line {base_name}");
        d.full_name = format!(
            "Double Line {base_name} (the {base_name} straight line kept as-is, plus a smooth zero-lag signal line that rides above/below it by its own trend and crosses exactly at each trend flip)"
        );
        d.cat = "Straight Line Indicator".into();
        d.inputs.push(num_in("dlEmaLen", "Crossover width", 9.0, 2.0, 200.0, 1.0));
        d.inputs.push(num_in("dlGap", "Signal gap", 0.1, 0.0, 100.0, 0.05));
        d.inputs.push(enum_in(
            "dlGapUnit",
            "Signal gap unit",
            "percent",
            &[("percent", "% of price"), ("points", "Points")],
        ));
        d.style.push(color_st("dlSignalColor", "Signal line color", "#ffffff"));
        v.push(IndicatorEntry {
            def: d,
            compute: compute_dl_stub,
            markers: None,
            reading,
        });
    }
    // Double Line OI Trend: the OI direction-state line is owned by the live
    // option chain (not a candle-only compute), so the chart layer layers its
    // series in from the OI direction overlay; this entry keeps it in the menu.
    v.push(e!(
        def("dl_oit", "Double Line OI Trend", "Double Line OI Trend (the OI direction-state line kept as-is, plus a smooth zero-lag signal line that crosses exactly at each regime flip; needs the live option chain)", "Straight Line Indicator", IndType::Overlay, None,
            vec![num_in("dlEmaLen", "Crossover width", 9.0, 2.0, 200.0, 1.0),
                 num_in("dlGap", "Signal gap", 0.1, 0.0, 100.0, 0.05),
                 enum_in("dlGapUnit", "Signal gap unit", "percent", &[("percent", "% of price"), ("points", "Points")])],
            vec![color_st("dlSignalColor", "Signal line color", "#ffffff"),
                 num_st("lineWidth", "Line width", 2.0, 1.0, 5.0, 1.0)]),
        compute_dl_stub, "price"
    ));

    crate::patterns::registry_into(&mut v);

    v
}

pub fn find(id: &str) -> Option<IndicatorEntry> {
    registry().into_iter().find(|x| x.def.id == id)
}

pub fn catalog_json() -> serde_json::Value {
    let list: Vec<IndicatorDef> = registry().into_iter().map(|x| x.def).collect();
    serde_json::to_value(list).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(n: usize) -> Vec<Candle> {
        let mut out = Vec::with_capacity(n);
        let mut price = 100.0_f64;
        let mut t = 1_700_000_000_i64;
        for i in 0..n {
            let wave = ((i as f64) * 0.13).sin() * 2.5 + ((i as f64) * 0.041).cos() * 0.9;
            let open = price;
            let close = (price + wave).max(1.0);
            let high = open.max(close) + (i % 5) as f64 * 0.3 + 0.2;
            let low = open.min(close) - (i % 3) as f64 * 0.25 - 0.15;
            out.push(Candle { time: t, open, high, low, close, volume: 1000.0 + (i % 17) as f64 * 87.0 });
            price = close;
            t += 300;
        }
        out
    }

    fn defaults(entry: &IndicatorEntry) -> Settings {
        let mut s = Settings::new();
        for inp in &entry.def.inputs {
            s.insert(inp.key.clone(), inp.def.clone());
        }
        for st in &entry.def.style {
            s.insert(st.key.clone(), st.def.clone());
        }
        s
    }

    #[test]
    fn vwap_hugs_price_with_a_rolling_window() {
        // 25 heavy-volume bars at 100, then 20 bars at 200. A cumulative VWAP
        // would stay pinned near 100 while price trades at 200; the rolling
        // window must have moved on to the new price within 20 bars.
        let mut candles = Vec::new();
        for i in 0..25 {
            candles.push(Candle { time: i as i64 * 300, open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1_000.0 });
        }
        for i in 0..20 {
            candles.push(Candle { time: (25 + i) as i64 * 300, open: 200.0, high: 200.0, low: 200.0, close: 200.0, volume: 1_000.0 });
        }
        let out = compute_vwap(&candles, &Settings::new());
        let vals: Vec<f64> = out[0].data.iter().map(|p| p.value).collect();
        assert_eq!(vals.len(), 45);
        assert!((vals[24] - 100.0).abs() < 1e-9);
        // Five new bars in: 15 old + 5 new => (15*100 + 5*200) / 20.
        assert!((vals[29] - 125.0).abs() < 1e-9, "window must roll, got {}", vals[29]);
        assert!(vals[29] < 200.0 && vals[29] > 100.0, "must sit between old and new price");
        // Twenty new bars in: the old session is fully out of the window.
        assert!((vals[44] - 200.0).abs() < 1e-9, "window fully rolled, got {}", vals[44]);
    }

    #[test]
    fn vwap_ignores_nan_and_missing_volume() {
        let candles = vec![
            Candle { time: 0, open: 10.0, high: 10.0, low: 10.0, close: 10.0, volume: f64::NAN },
            Candle { time: 300, open: 20.0, high: 20.0, low: 20.0, close: 20.0, volume: 100.0 },
            Candle { time: 600, open: 30.0, high: 30.0, low: 30.0, close: 30.0, volume: 0.0 },
        ];
        let out = compute_vwap(&candles, &Settings::new());
        let vals: Vec<f64> = out[0].data.iter().map(|p| p.value).collect();
        assert!(vals.iter().all(|v| v.is_finite()), "NaN volume must not poison the line: {vals:?}");
        assert!((vals[0] - 10.0).abs() < 1e-9, "no usable volume yet => typical price");
        assert!((vals[1] - 20.0).abs() < 1e-9);
        assert!((vals[2] - 20.0).abs() < 1e-9, "a zero-volume bar keeps the running VWAP");
    }

    #[test]
    fn straight_line_colour_follows_its_own_slope() {
        // Rising then falling then flat: points must be green while the line
        // climbs (bullish) and red while it drops (bearish); a flat step keeps
        // whatever neutral colour was there (None here).
        let mut data: Vec<Point> = [1.0, 2.0, 3.0, 2.0, 1.0, 1.0]
            .iter()
            .enumerate()
            .map(|(i, v)| Point { time: i as i64, value: *v, color: None })
            .collect();
        color_line_by_slope(&mut data, "#26a69a", "#ef5350");
        let c: Vec<Option<String>> = data.iter().map(|p| p.color.clone()).collect();
        assert_eq!(c[0].as_deref(), Some("#26a69a"));
        assert_eq!(c[1].as_deref(), Some("#26a69a"));
        assert_eq!(c[2].as_deref(), Some("#26a69a"));
        assert_eq!(c[3].as_deref(), Some("#ef5350"));
        assert_eq!(c[4].as_deref(), Some("#ef5350"));
        assert_eq!(c[5], None);
    }

    #[test]
    fn straight_line_indicators_get_direction_colours() {
        // The shared rule must paint at least some green (rising) and some red
        // (falling) across the straight-line family, and must not touch a
        // non-straight-line indicator.
        let candles = synth(360);
        let mut green = 0;
        let mut red = 0;
        for e in registry() {
            if !is_straight_line(&e.def.id) {
                continue;
            }
            let mut out = (e.compute)(&candles, &defaults(&e));
            color_straight_line(&e.def.id, &mut out, &defaults(&e), &candles);
            green += out.iter().flat_map(|s| &s.data).filter(|p| p.color.as_deref() == Some("#26a69a")).count();
            red += out.iter().flat_map(|s| &s.data).filter(|p| p.color.as_deref() == Some("#ef5350")).count();
        }
        assert!(green > 0, "expected rising straight-line legs to be green");
        assert!(red > 0, "expected falling straight-line legs to be red");

        // A momentum pane is not a straight line, so its colours are untouched.
        let rsi = registry().into_iter().find(|e| e.def.id == "rsi").unwrap();
        let mut out = (rsi.compute)(&candles, &defaults(&rsi));
        let before: Vec<Option<String>> = out.iter().flat_map(|s| s.data.iter().map(|p| p.color.clone())).collect();
        color_straight_line("rsi", &mut out, &defaults(&rsi), &candles);
        let after: Vec<Option<String>> = out.iter().flat_map(|s| s.data.iter().map(|p| p.color.clone())).collect();
        assert_eq!(before, after, "non-straight-line indicator must be left alone");
    }

    #[test]
    fn double_line_signal_rides_the_correct_side_and_crosses_at_flips() {
        let candles = synth(360);
        let reg = registry();
        // The family registers a `dl_` entry for the straight-line overlays and
        // for Trend Core, but excludes the EMA (ZigZag EMA) one.
        assert!(reg.iter().any(|e| e.def.id == "dl_vlcore"), "Trend Core double line registered");
        assert!(reg.iter().any(|e| e.def.id == "dl_stzigzag"));
        assert!(reg.iter().any(|e| e.def.id == "dl_oit"), "OI Trend double line registered");
        assert!(!reg.iter().any(|e| e.def.id == "dl_slema"), "EMA excluded from the family");

        // Interpolate the (possibly pivot-only) main line at any bar time.
        let main_at = |main: &SeriesOut, t: i64| -> Option<f64> {
            let p = &main.data;
            if p.is_empty() || t < p[0].time || t > p[p.len() - 1].time {
                return None;
            }
            let i = match p.binary_search_by_key(&t, |q| q.time) {
                Ok(i) => return if p[i].value.is_finite() { Some(p[i].value) } else { None },
                Err(i) => i.saturating_sub(1),
            };
            let a = &p[i];
            let b = p.get(i + 1)?;
            if !a.value.is_finite() || !b.value.is_finite() {
                return None;
            }
            if b.time == a.time {
                return Some(a.value);
            }
            Some(a.value + (b.value - a.value) * ((t - a.time) as f64 / (b.time - a.time) as f64))
        };
        // The main line's own trend side at a bar (its colour).
        let side_at = |main: &SeriesOut, t: i64| -> i32 {
            let mut d = 1i32;
            for p in main.data.iter() {
                if p.time > t {
                    break;
                }
                match p.color.as_deref() {
                    Some(c) if c.eq_ignore_ascii_case("#26a69a") => d = 1,
                    Some(c) if c.eq_ignore_ascii_case("#ef5350") => d = -1,
                    _ => {}
                }
            }
            d
        };

        for base_id in ["vlcore", "stzigzag"] {
            let base = reg.iter().find(|e| e.def.id == base_id).unwrap();
            let mut settings = defaults(base);
            settings.insert("dlEmaLen".into(), json!(9));
            settings.insert("dlGap".into(), json!(0.5));
            settings.insert("dlGapUnit".into(), json!("percent"));
            let mut series = (base.compute)(&candles, &settings);
            color_straight_line(base_id, &mut series, &settings, &candles);
            let main_len = series[0].data.len();
            let out = double_line_series(series, &settings, &candles);
            assert_eq!(out.len(), 2, "{base_id}: main + signal");
            let main = &out[0];
            let sig = &out[1];
            assert_eq!(main.data.len(), main_len, "{base_id}: main stays the straight line");
            assert!(!sig.data.is_empty(), "{base_id}: signal drawn on the bar grid");
            assert!(
                sig.data.iter().all(|p| p.color.as_deref() == Some("#ffffff")),
                "{base_id}: signal line is white"
            );

            let mut crossings = 0usize;
            let mut prev = 0i32;
            let mut min_ratio = f64::INFINITY;
            for s in &sig.data {
                let Some(mv) = main_at(main, s.time) else { continue };
                let full = mv.abs() * 0.5 / 100.0;
                let diff = (s.value - mv).abs();
                assert!(
                    diff <= full + 1e-9,
                    "{base_id}: signal never strays wider than the full gap"
                );
                if full > 0.0 {
                    min_ratio = min_ratio.min(diff / full);
                }
                let d = if s.value >= mv { 1 } else { -1 };
                if prev != 0 && d != prev {
                    crossings += 1;
                }
                prev = d;
            }
            assert!(crossings >= 2, "{base_id}: the signal crosses the line, both ways ({crossings})");
            assert!(min_ratio < 0.25, "{base_id}: crossover comes back to the line at a flip ({min_ratio})");

            // The first (open) and last (into the close) bars must already sit on
            // the line's own trend side, at (near) the full gap - never stuck on
            // the wrong side or left hugging/overlapping the line.
            for (tag, s) in [("open", sig.data.first()), ("close", sig.data.last())] {
                let s = s.unwrap();
                let mv = main_at(main, s.time).unwrap();
                let full = mv.abs() * 0.5 / 100.0;
                let diff = (s.value - mv).abs();
                let side = side_at(main, s.time);
                assert_eq!(
                    (s.value - mv).signum() as i32,
                    side,
                    "{base_id}: {tag} signal must sit on the trend side"
                );
                assert!(
                    diff > full * 0.5,
                    "{base_id}: {tag} signal must ride the fan, not overlap the line"
                );
            }
        }
    }

    #[test]
    fn supertrend_zigzag_is_a_single_combined_line() {
        // One line only, ending on the live average of the two Supertrends, and
        // enrolled in the shared straight-line family.
        let candles = synth(360);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "stzigzag").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        assert_eq!(out.len(), 1, "must draw exactly one line");
        let zz = &out[0];
        assert!(zz.data.len() >= 2, "zigzag should draw at least one leg");
        let (fast, _) = supertrend_series(&candles, 10, 1.0);
        let (slow, _) = supertrend_series(&candles, 10, 0.8);
        let last = candles.len() - 1;
        let want = (fast[last].unwrap() + slow[last].unwrap()) * 0.5;
        assert!(
            (zz.data.last().unwrap().value - want).abs() < 1e-6,
            "zigzag must end on the live average of the two Supertrends"
        );
        assert!(zz.data.iter().any(|p| p.color.as_deref() == Some("#26a69a")), "rising legs green");
        assert!(zz.data.iter().any(|p| p.color.as_deref() == Some("#ef5350")), "falling legs red");
        assert!(is_straight_line("stzigzag"), "must be a straight-line indicator");
    }

    #[test]
    fn straight_line_input_drops_the_forming_bar_only() {
        // A straight-line / zigzag indicator evaluated while the trailing candle
        // is still forming must ignore that bar, so its line holds still until the
        // candle closes. A closed series is left whole, and non-straight-line
        // indicators always see every candle.
        let candles = synth(30);
        for id in ["stzigzag", "slema", "zzline", "autotrend", "slconsensus", "bbpctsl", "panemaster"] {
            assert!(is_straight_line(id), "{id} must be a straight-line indicator");
            let got = straight_line_input(id, &candles, true);
            assert_eq!(got.len(), candles.len() - 1, "{id} must drop the forming bar");
            assert_eq!(got.last().unwrap().time, candles[candles.len() - 2].time);
        }
        assert_eq!(straight_line_input("stzigzag", &candles, false).len(), candles.len());
        assert_eq!(straight_line_input("rsi", &candles, true).len(), candles.len());
    }

    /// A clean repeating measured-move structure: `cycles` up legs then `cycles`
    /// down legs, each leg `bars` bars long and `amp` points tall.
    fn structure_wave(cycles: usize, bars: usize, amp: f64) -> Vec<Candle> {
        let mut out = Vec::new();
        let mut price = 100.0_f64;
        let mut t = 1_700_000_000_i64;
        let step = amp / bars as f64;
        for _ in 0..cycles {
            for _ in 0..bars {
                let open = price;
                price += step;
                out.push(Candle { time: t, open, high: open.max(price) + 0.3, low: open.min(price) - 0.3, close: price, volume: 1000.0 });
                t += 300;
            }
            for _ in 0..bars {
                let open = price;
                price -= step;
                out.push(Candle { time: t, open, high: open.max(price) + 0.3, low: open.min(price) - 0.3, close: price, volume: 1000.0 });
                t += 300;
            }
        }
        out
    }

    #[test]
    fn structure_based_supplydemand_projects_before_candles() {
        let candles = structure_wave(8, 20, 50.0);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "supplydemand").unwrap();
        assert_eq!(e.def.name, "Structure Based Supply Demand");
        assert_eq!(e.def.kind, IndType::Overlay);
        let out = (e.compute)(&candles, &defaults(e));
        assert_eq!(out.len(), 2, "confirmed structure + forecast series");
        let last_time = candles.last().unwrap().time;
        assert!(
            out[0].data.iter().any(|p| p.time == last_time),
            "confirmed structure must reach the live candle"
        );
        let forecast = &out[1];
        assert!(forecast.data.len() >= 2, "forecast must draw a leg");
        assert!(
            forecast.data.iter().skip(1).all(|p| p.time > last_time),
            "forecast must be drawn into the future, before its candles exist"
        );
        assert_eq!(forecast.line_style, Some(2), "forecast must be dashed");
    }

    #[test]
    fn structure_based_supplydemand_forecast_measures_the_repeating_structure() {
        let candles = structure_wave(10, 20, 50.0);
        let cc = clean_candles(&candles);
        let f = sd_forecast(&cc, 5, 3, 0.25).expect("forecast");
        assert!((f.bars - 20.0).abs() <= 3.0, "should measure ~20-bar structures, got {}", f.bars);
        assert!((f.amp - 50.0).abs() <= 10.0, "should measure ~50-pt structures, got {}", f.amp);
        assert!(f.score >= 0.66, "repeating structure should score high, got {}", f.score);
        assert!(f.dir == 1 || f.dir == -1, "direction must be resolved");
    }

    #[test]
    fn structure_based_supplydemand_backtest_hits_the_repeat() {
        // Backtest the core claim: the next structure repeats the recent
        // structures' bar length and amplitude, on the opposite side. Every leg
        // is predicted from the trailing legs and scored against what followed.
        let candles = structure_wave(30, 20, 50.0);
        let cc = clean_candles(&candles);
        let legs_raw = structure_legs(&cc, 5);
        let legs: Vec<(f64, f64, i32)> = legs_raw
            .iter()
            .map(|l| (l.bars(), l.amp().abs(), if l.amp() >= 0.0 { 1 } else { -1 }))
            .collect();
        assert!(legs.len() >= 12, "need enough measured structures, got {}", legs.len());
        let k = 3usize;
        let tol = 0.25;
        let mut samples = 0usize;
        let mut hits = 0usize;
        for idx in k..legs.len() {
            let tmpl_bars = legs[idx - k..idx].iter().map(|l| l.0).sum::<f64>() / k as f64;
            let tmpl_amp = legs[idx - k..idx].iter().map(|l| l.1).sum::<f64>() / k as f64;
            let (ab, aa, ad) = legs[idx];
            let bars_ok = (ab - tmpl_bars).abs() / tmpl_bars <= tol;
            let amp_ok = (aa - tmpl_amp).abs() / tmpl_amp <= tol;
            let dir_ok = ad == -legs[idx - 1].2;
            samples += 1;
            if bars_ok && amp_ok && dir_ok {
                hits += 1;
            }
        }
        let rate = hits as f64 / samples as f64;
        assert!(rate >= 0.8, "structure repeat hit rate should be high, got {:.0}%", rate * 100.0);
    }

    #[test]
    fn structure_based_supplydemand_is_robust_on_short_and_noisy_input() {
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "supplydemand").unwrap();
        let short = synth(2);
        assert!(!(e.compute)(&short, &defaults(e)).is_empty());
        let noisy = synth(300);
        let cc = clean_candles(&noisy);
        if let Some(f) = sd_forecast(&cc, 5, 3, 0.25) {
            assert!((0.0..=1.0).contains(&f.score));
            assert!(f.bars >= 1.0 && f.amp > 0.0);
        }
        assert_eq!((e.compute)(&noisy, &defaults(e)).len(), 2);
    }

    #[test]
    fn master_line_draws_fused_line_and_atr_projection() {
        let candles = structure_wave(8, 20, 50.0);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "masterline").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        assert_eq!(out.len(), 2, "fused line + projection");
        let last_time = candles.last().unwrap().time;
        assert!(out[0].data.iter().any(|p| p.time == last_time), "line reaches the live candle");
        assert!(!out[0].price_lines.is_empty(), "S/D level lines present");
        assert!(out[0].exclude_autoscale, "S/D level lines must not drive the candle autoscale");
        let proj = &out[1];
        assert!(proj.data.len() >= 2, "projection must draw");
        assert!(
            proj.data.iter().skip(1).all(|p| p.time > last_time),
            "projection is drawn into the future"
        );
        assert_eq!(proj.line_style, Some(2), "projection must be dashed");
    }

    #[test]
    fn master_line_flips_both_ways_with_the_trend() {
        let candles = trend_then_drop();
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "masterline").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let d = &out[0].data;
        assert!(d.iter().any(|p| p.color.as_deref() == Some("#00e676")), "bull leg");
        assert!(d.iter().any(|p| p.color.as_deref() == Some("#ff5252")), "bear leg");
    }

    #[test]
    fn master_line_draws_a_multi_leg_zigzag_not_one_straight_line() {
        let candles = synth(300);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "masterline").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        assert!(
            out[0].data.len() >= 4,
            "masterline must break into readable zigzag legs, got {}",
            out[0].data.len()
        );
    }

    #[test]
    fn master_line_oi_bias_tilts_the_fused_direction() {
        let candles = structure_wave(2, 10, 20.0);
        let cc = clean_candles(&candles);
        let n = cc.len();
        let rdir = vec![0i32; n];
        let rflat = vec![true; n];
        let mut o = Settings::new();
        o.insert("useLevels".into(), serde_json::json!(false));
        o.insert("useOi".into(), serde_json::json!(true));
        o.insert("oiBias".into(), serde_json::json!(100.0));
        let up = master_fused_dir(&cc, &o, &rdir, &rflat);
        o.insert("oiBias".into(), serde_json::json!(-100.0));
        let dn = master_fused_dir(&cc, &o, &rdir, &rflat);
        assert!(up.iter().all(|&d| d == 1), "positive OI bias -> bullish fuse");
        assert!(dn.iter().all(|&d| d == -1), "negative OI bias -> bearish fuse");
    }

    #[test]
    fn trend_core_is_a_causal_price_zigzag() {
        let candles = synth(400);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "vlcore").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        assert_eq!(out.len(), 1, "one zigzag line");
        let d = &out[0].data;
        assert!(d.len() >= candles.len() - 1, "dense render stays bar-aligned, got {}", d.len());
        let lo = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let hi = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        assert!(
            d.iter().all(|p| p.value >= lo - 0.01 && p.value <= hi + 0.01),
            "zigzag must stay inside the price range"
        );
        let mut flips = 0;
        let mut prev: Option<f64> = None;
        let mut dir = 0i32;
        for p in d {
            if let Some(pr) = prev {
                let s = (p.value - pr).signum() as i32;
                if s != 0 && s != dir {
                    flips += 1;
                    dir = s;
                }
            }
            prev = Some(p.value);
        }
        assert!(flips >= 2, "must break into multiple legs, flips={flips}");
    }

    #[test]
    fn trend_core_does_not_repaint_early_legs() {
        let full = synth(400);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "vlcore").unwrap();
        let o = defaults(e);
        let short = &full[..300];
        let s_short = (e.compute)(short, &o);
        let s_full = (e.compute)(&full, &o);
        for (a, b) in s_short[0].data.iter().zip(s_full[0].data.iter()).take(250) {
            assert_eq!(a.time, b.time);
            assert!((a.value - b.value).abs() < 1e-6, "past leg revised at {}", a.time);
        }
    }

    #[test]
    fn master_line_is_robust_on_short_and_noisy_input() {
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "masterline").unwrap();
        let short = synth(2);
        assert!(!(e.compute)(&short, &defaults(e)).is_empty());
        let noisy = synth(300);
        assert_eq!((e.compute)(&noisy, &defaults(e)).len(), 2);
    }

    #[test]
    fn bbpct_straight_line_is_a_single_colored_trend_line() {
        // The BB%b straight-line is one price line, reaches the live candle, and
        // carries the shared rising/falling colouring.
        let candles = trend_then_drop();
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "bbpctsl").unwrap();
        assert!(is_straight_line("bbpctsl"), "must be a straight-line indicator");
        let mut out = (e.compute)(&candles, &defaults(e));
        assert_eq!(out.len(), 1, "must draw exactly one line");
        assert!(out[0].data.len() >= 2, "must draw at least one leg");
        assert_eq!(
            out[0].data.last().unwrap().time,
            candles.last().unwrap().time,
            "must reach the latest candle"
        );
        color_straight_line("bbpctsl", &mut out, &defaults(e), &candles);
        let data = &out[0].data;
        assert!(data.iter().any(|p| p.color.as_deref() == Some("#26a69a")), "rising legs green");
        assert!(data.iter().any(|p| p.color.as_deref() == Some("#ef5350")), "falling legs red");
    }

    #[test]
    fn bbpct_straightline_pane_is_a_single_colored_pane_line() {
        // The pane twin is a Pane series, one line of constant slope per %B regime.
        let candles = trend_then_drop();
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "bbpctslp").unwrap();
        assert_eq!(e.def.kind, IndType::Pane, "must be a pane indicator");
        assert!(!is_straight_line("bbpctslp"), "pane twin is not enrolled as a price overlay");
        let out = (e.compute)(&candles, &defaults(e));
        assert_eq!(out.len(), 1, "must draw exactly one line");
        let data = &out[0].data;
        assert!(data.len() >= 2, "must draw at least one leg");
        assert_eq!(
            data.last().unwrap().time,
            candles.last().unwrap().time,
            "must reach the latest candle"
        );
        // A bullish regime must raise the line and a bearish one lower it.
        let d: Vec<i32> = data.windows(2).map(|w| if w[1].value > w[0].value { 1 } else if w[1].value < w[0].value { -1 } else { 0 }).collect();
        assert!(d.contains(&1) && d.contains(&-1), "pane must trend both up and down");
        assert!(data.iter().any(|p| p.color.as_deref() == Some("#26a69a")), "rising legs green");
        assert!(data.iter().any(|p| p.color.as_deref() == Some("#ef5350")), "falling legs red");
    }

    #[test]
    fn support_trendline_touches_multiple_lows() {
        // Clean sawtooth whose swing lows sit exactly on y = 100 + 0.2*i. The
        // support line must pass through (touch) at least three of those lows.
        let mut candles = Vec::new();
        for i in 0..300 {
            let base = 100.0 + 0.2 * i as f64;
            let phase = (i % 20) as f64;
            let tri = if phase < 10.0 { phase } else { 20.0 - phase };
            let close = base + tri;
            candles.push(Candle { time: i as i64 * 300, open: close - 0.2, high: close + 0.3, low: close - 0.3, close, volume: 1000.0 });
        }
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "supline").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let line = out.first().unwrap();
        let cc = clean_candles(&candles);
        let start = cc.iter().position(|c| c.time == line.data[0].time).unwrap();
        let slope = line.data[1].value - line.data[0].value;
        let b = line.data[0].value - slope * start as f64;
        let piv = fractal_pivots(&cc, 5.0);
        let touched = piv
            .iter()
            .filter(|p| !p.is_high)
            .filter(|p| {
                let v = slope * p.idx as f64 + b;
                (v - p.price).abs() <= 1.0
            })
            .count();
        assert!(touched >= 3, "support line must touch at least 3 swing lows, got {touched}");
    }

    #[test]
    fn straight_line_arrow_markers_flip_with_direction() {
        // Arrow markers are printed exactly where a line's slope changes sign.
        let mut line = SeriesOut::line("#26a69a", 1.0);
        line.data = vec![
            Point { time: 1, value: 1.0, color: None },
            Point { time: 2, value: 2.0, color: None },
            Point { time: 3, value: 3.0, color: None },
            Point { time: 4, value: 2.5, color: None },
            Point { time: 5, value: 2.0, color: None },
            Point { time: 6, value: 2.5, color: None },
        ];
        let mk = slope_markers(&[line], 0, "#00ff00", "#ff0000");
        let got: Vec<(&str, i64)> = mk.iter().map(|m| (m.shape.as_str(), m.time)).collect();
        assert_eq!(got, vec![("arrowUp", 2), ("arrowDown", 4), ("arrowUp", 6)]);
    }

    #[test]
    fn straight_line_indicators_expose_arrow_markers() {
        let reg = registry();
        for id in ["vlcore", "vl", "srema", "autotrend", "pitchfork", "projline", "fibfan", "gannfan", "supplydemand"] {
            let e = reg.iter().find(|e| e.def.id == id).unwrap();
            assert!(e.markers.is_some(), "{id} must expose up/down arrow markers");
        }
    }

    #[test]
    fn straight_line_markers_are_emitted_for_real_series() {
        let candles = synth(400);
        let reg = registry();
        for id in ["zzline", "trendmaster", "panemaster", "vlcore", "vl", "srema", "autotrend", "pitchfork", "projline", "fibfan", "gannfan", "supplydemand"] {
            let e = reg.iter().find(|e| e.def.id == id).unwrap();
            let mk = (e.markers.unwrap())(&candles, &defaults(e));
            assert!(!mk.is_empty(), "{id} must emit trend-start arrows on a real oscillating series, got 0");
        }
    }

    #[test]
    fn straight_line_consensus_line_stays_within_price_range() {
        // The consensus line is now a segment-trend-wise zigzag: each leg joins
        // two swing turns. The line must run through the candle bodies rather
        // than floating away from them, and must reach the latest candle.
        let mut candles = Vec::new();
        for i in 0..400 {
            let base = 100.0 + 0.5 * i as f64 + ((i as f64) * 0.35).sin() * 0.6;
            candles.push(Candle {
                time: i as i64 * 300,
                open: base - 0.1,
                high: base + 0.9,
                low: base - 0.9,
                close: base,
                volume: 1000.0,
            });
        }
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "slconsensus").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let line = out.first().expect("consensus line series");
        assert!(!line.data.is_empty(), "consensus line must have data on a real series");
        let cc = clean_candles(&candles);
        let by_time: std::collections::BTreeMap<i64, usize> =
            cc.iter().enumerate().map(|(i, c)| (c.time, i)).collect();
        let pts: Vec<(usize, f64)> = line.data.iter().map(|p| (by_time[&p.time], p.value)).collect();
        let inside = pts.iter().filter(|(i, v)| *v >= cc[*i].low - 1e-6 && *v <= cc[*i].high + 1e-6).count();
        let frac = inside as f64 / pts.len().max(1) as f64;
        assert!(frac >= 0.9, "consensus line should cut through candles, only {frac:.2} inside");
        assert!(
            line.data.last().unwrap().value > line.data.first().unwrap().value,
            "consensus line must rise with the up-trend"
        );
        assert_eq!(
            line.data.last().unwrap().time,
            cc.last().unwrap().time,
            "consensus line must reach the most recent candle"
        );
    }

    #[test]
    fn straight_line_consensus_rises_on_a_ramp() {
        // A monotonic up-ramp must draw a rising line (colour/direction match
        // the trend), not an inverted one.
        let mut candles = Vec::new();
        for i in 0..300 {
            let c = 100.0 + i as f64 * 0.5;
            candles.push(Candle { time: i as i64 * 300, open: c - 0.4, high: c + 0.6, low: c - 0.6, close: c, volume: 1000.0 });
        }
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "slconsensus").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let line = out.first().unwrap();
        let first = line.data.first().unwrap().value;
        let last = line.data.last().unwrap().value;
        assert!(last > first, "line must rise with an up-ramp: first={first} last={last}");
        // Segment-trend-wise: a monotonic ramp is one straight leg from first to
        // last vertex, so the plotted vertices must strictly rise (never a flat
        // plate, which the old least-squares anchor could sit on).
        assert!(line.data.len() >= 2, "ramp must draw at least the two end vertices");
        for w in line.data.windows(2) {
            assert!(
                w[1].value >= w[0].value,
                "ramp line must not fall back: {} -> {}",
                w[0].value,
                w[1].value
            );
        }
    }

    #[test]
    fn causal_zigzag_never_repaints_confirmed_turns() {
        // The whole point of the pivot fix: a turn that has been confirmed by bar
        // k must stay exactly where it is no matter what later bars do. The old
        // forward-looking scan re-picked the trailing turns as data arrived, so
        // the line searched during a reversal and only settled at the end.
        let candles = synth(400);
        let cc = clean_candles(&candles);
        let mid: Vec<Option<f64>> = cc.iter().map(|x| Some((x.high + x.low) * 0.5)).collect();
        let full = zigzag_vertices(&mid, &[], 5, 5, 0);
        assert!(full.len() >= 3, "synth series must produce several turns");
        for k in [120usize, 200, 300, 380] {
            let part = zigzag_vertices(&mid[..k], &[], 5, 5, 0);
            // The prefix's last bar is only the carried live anchor, not a
            // confirmed turn, so compare everything before it.
            let confirmed: Vec<usize> = part.iter().copied().filter(|&v| v < k - 1).collect();
            assert!(!confirmed.is_empty(), "k={k} must have confirmed turns");
            for &v in &confirmed {
                assert!(full.contains(&v), "confirmed turn {v} disappeared (repainted) at k={k}");
            }
            assert_eq!(
                &full[..confirmed.len()],
                &confirmed[..],
                "confirmed turns were re-picked or reordered at k={k}"
            );
        }
    }

    #[test]
    fn pane_consensus_draws_a_price_anchored_zigzag() {
        // Pane Consensus now draws a segment-trend-wise zigzag anchored to the
        // candles (not one least-squares line per regime), so every vertex must
        // sit inside the candle body and the line must reach the latest bar.
        let candles = synth(400);
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "panemaster").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let line = out.first().expect("pane consensus series");
        assert!(!line.data.is_empty(), "pane consensus must draw a zigzag");
        let cc = clean_candles(&candles);
        for p in &line.data {
            let c = cc.iter().find(|c| c.time == p.time).unwrap();
            assert!(
                p.value >= c.low - 1e-6 && p.value <= c.high + 1e-6,
                "pane zigzag must cut through candle bodies: {} not in [{}, {}]",
                p.value,
                c.low,
                c.high
            );
        }
        assert_eq!(
            line.data.last().unwrap().time,
            cc.last().unwrap().time,
            "pane zigzag must reach the latest candle"
        );
    }

    #[test]
    fn auto_trendline_and_projection_draw_multi_leg_zigzags() {
        // Both were constant-slope fit lines; they must now be segment-trend-wise
        // zigzags that reach the latest bar.
        let candles = synth(400);
        let cc = clean_candles(&candles);
        let reg = registry();
        for id in ["autotrend", "projline"] {
            let e = reg.iter().find(|e| e.def.id == id).unwrap();
            let out = (e.compute)(&candles, &defaults(e));
            let hist = out.first().unwrap();
            assert!(
                hist.data.len() >= 3,
                "{id} must draw a multi-leg zigzag, got {} points",
                hist.data.len()
            );
            assert_eq!(
                hist.data.last().unwrap().time,
                cc.last().unwrap().time,
                "{id} zigzag must reach the latest candle"
            );
        }
    }

    #[test]
    fn overlay_consensus_fuses_the_twelve_overlays_into_a_rising_line() {
        let mut candles = Vec::new();
        for i in 0..300 {
            let c = 100.0 + i as f64 * 0.5;
            candles.push(Candle { time: i as i64 * 300, open: c - 0.4, high: c + 0.6, low: c - 0.6, close: c, volume: 1000.0 });
        }
        let cc = clean_candles(&candles);
        let dir = ovl_consensus_dir(&cc, 2, 5, 5.0);
        assert_eq!(dir.last().copied(), Some(1), "overlay consensus must resolve bullish on an up-ramp");
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "ovlconsensus").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let line = out.first().unwrap();
        assert!(!line.data.is_empty(), "overlay consensus must draw a line");
        assert!(
            line.data.last().unwrap().value > line.data.first().unwrap().value,
            "overlay consensus line must rise with the up-ramp"
        );
    }

    #[test]
    fn straight_line_consensus_confirm_delays_flips_and_reaches_last_bar() {
        // A larger confirmation window must never add flips (hysteresis can only
        // hold a trend longer), and the drawn line must extend to the latest
        // candle so it tracks the trend right up to the last bar.
        let candles = synth(400);
        let fast = sl_consensus_dir(&candles, 2, 1, 5.0);
        let slow = sl_consensus_dir(&candles, 2, 8, 5.0);
        let flips = |d: &[i32]| d.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(flips(&slow) <= flips(&fast), "confirm must not increase flips");
        let reg = registry();
        let e = reg.iter().find(|e| e.def.id == "slconsensus").unwrap();
        let out = (e.compute)(&candles, &defaults(e));
        let line = out.first().unwrap();
        let cc = clean_candles(&candles);
        assert_eq!(
            line.data.last().unwrap().time,
            cc.last().unwrap().time,
            "consensus line must reach the most recent candle"
        );
    }

    #[test]
    fn support_and_resistance_trendlines_are_separate_lines() {
        // Two independent indicators: one straight line through swing lows, one
        // through swing highs. They must be distinct, with support below price
        // and resistance above it on an oscillating series.
        let candles = synth(500);
        let reg = registry();
        let sup = reg.iter().find(|e| e.def.id == "supline").unwrap();
        let res = reg.iter().find(|e| e.def.id == "resline").unwrap();
        let so = (sup.compute)(&candles, &defaults(sup));
        let ro = (res.compute)(&candles, &defaults(res));
        let s = so.first().expect("support line");
        let r = ro.first().expect("resistance line");
        assert!(s.data.len() > 2, "support line must be drawn");
        assert!(r.data.len() > 2, "resistance line must be drawn");
        let smean = s.data.iter().map(|p| p.value).sum::<f64>() / s.data.len() as f64;
        let rmean = r.data.iter().map(|p| p.value).sum::<f64>() / r.data.len() as f64;
        assert!(smean < rmean, "support line ({smean}) must sit below resistance ({rmean})");
        // The line must be near the current price band, not projected far away
        // from the chart (the old bug that pushed the candles into a sliver).
        let last_t = candles.last().unwrap().time;
        let last_close = candles.last().unwrap().close;
        let lo = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let hi = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let range = hi - lo;
        let s_last = s.data.iter().find(|p| p.time == last_t).expect("support line covers the last bar").value;
        let r_last = r.data.iter().find(|p| p.time == last_t).expect("resistance line covers the last bar").value;
        assert!(s_last <= last_close + range * 0.05, "support line {s_last} must not sit above price {last_close}");
        assert!(r_last >= last_close - range * 0.05, "resistance line {r_last} must not sit below price {last_close}");
        for (name, v) in [("support", s_last), ("resistance", r_last)] {
            assert!(
                v >= lo - range * 0.15 && v <= hi + range * 0.15,
                "{name} line {v} projected outside the price band [{lo}, {hi}]"
            );
        }
        let smk = (sup.markers.unwrap())(&candles, &defaults(sup));
        let rmk = (res.markers.unwrap())(&candles, &defaults(res));
        assert!(!smk.is_empty(), "support trendline must expose arrows");
        assert!(!rmk.is_empty(), "resistance trendline must expose arrows");
    }

    fn trend_then_drop() -> Vec<Candle> {
        let mut c = synth(200);
        let mut price = c.last().unwrap().close;
        let mut t = c.last().unwrap().time;
        for _ in 0..40 {
            let open = price;
            let close = price - 0.9;
            t += 300;
            c.push(Candle {
                time: t,
                open,
                high: open + 0.2,
                low: close - 0.2,
                close,
                volume: 1000.0,
            });
            price = close;
        }
        c
    }

    #[test]
    fn live_trend_dir_flips_same_bar_as_the_turn() {
        let c = trend_then_drop();
        let dir = live_trend_dir(&c, 9);
        assert_eq!(dir[c.len() - 1], -1, "trend must be bearish right after the down leg");
        assert_eq!(dir[210], -1, "trend must be bearish a few bars into the drop");
    }

    #[test]
    fn straight_line_family_colors_follow_live_trend() {
        let c = trend_then_drop();
        let last_time = c.last().unwrap().time;
        let reg = registry();
        for id in [
            "zzline", "trendmaster", "panemaster", "srema", "autotrend", "pitchfork",
            "projline", "fibfan", "gannfan", "supplydemand", "vl",
        ] {
            let e = reg.iter().find(|e| e.def.id == id).unwrap();
            let out = (e.compute)(&c, &defaults(e));
            let last = out
                .iter()
                .find_map(|s| {
                    s.data
                        .last()
                        .filter(|p| p.time == last_time)
                        .and_then(|p| p.color.clone())
                })
                .unwrap_or_else(|| panic!("{id} produced no trend-colored point at the last bar"));
            assert!(
                last.contains("ef5350"),
                "{id} last point must be bearish-colored after the down leg, got {last}"
            );
        }
    }

    #[test]
    fn registry_matches_old_app_catalog() {
        let reg = registry();
        // 71 indicators (69 old-app + the Straightline EMA + Supertrend ZigZag)
        // + 27 candlestick patterns (crate::patterns) + the BB%b straight line
        // and its pane twin + the fused Master Line + Price Action Diversionless
        // (pane + price) + the Overall Straight Line Consensus (pane + price)
        // + the Double Line family (22 straight-line overlays + Trend Core + OI Trend).
        assert_eq!(reg.len(), 128);
        let find = |id: &str| reg.iter().find(|e| e.def.id == id).unwrap();
        for id in ["pcr", "pcrrail", "iv", "panemaster", "ewtrend", "patrend", "sremat"] {
            assert_eq!(find(id).def.kind, IndType::Overlay, "{id} must be an overlay");
        }
        for id in ["ewtrend", "fibt", "gant", "patrend", "sremat"] {
            assert!(find(id).def.hidden, "{id} must be hidden");
        }
    }

    #[test]
    fn all_indicators_compute_without_panic() {
        for n in [5usize, 60, 400] {
            let candles = synth(n);
            for entry in registry() {
                let settings = defaults(&entry);
                let out = (entry.compute)(&candles, &settings);
                for series in &out {
                    assert!(
                        series.data.len() <= n + 600,
                        "{} produced too many points at n={n}",
                        entry.def.id
                    );
                }
                if let Some(mk) = entry.markers {
                    let _ = mk(&candles, &settings);
                }
            }
        }
    }

    #[test]
    fn padiv_flips_only_on_a_real_reversal() {
        // Strong up-trend then a strong down-trend: the confirmed regime must be
        // bullish during the climb and flip to bearish only after the reversal is
        // confirmed (never on the in-trend noise).
        let mut candles = Vec::new();
        let mut price = 100.0_f64;
        let mut t = 1_700_000_000_i64;
        for i in 0..80 {
            let step = if i < 40 { 1.2 } else { -1.2 };
            let open = price;
            let close = price + step;
            let high = open.max(close) + 0.4;
            let low = open.min(close) - 0.4;
            candles.push(Candle { time: t, open, high, low, close, volume: 1000.0 });
            price = close;
            t += 300;
        }
        let entry = registry().into_iter().find(|e| e.def.id == "padiv").unwrap();
        let settings = defaults(&entry);
        let state = padiv_state(&candles, &settings);
        assert_eq!(state.len(), candles.len());
        let first = state.iter().find(|d| **d != 0).copied().unwrap();
        assert_eq!(first, 1, "line must start bullish in the climb");
        assert!(state.iter().any(|d| *d == 1), "expected a bullish leg");
        assert!(state.iter().any(|d| *d == -1), "expected a bearish leg after the reversal");
        assert_ne!(*state.last().unwrap(), 0, "last bar must be committed");
    }

    #[test]
    fn overall_consensus_is_a_straight_line_that_follows_the_trend() {
        // long up-trend then a long down-trend.
        let mut candles = Vec::new();
        let mut price = 100.0_f64;
        let mut t = 1_700_000_000_i64;
        for i in 0..140 {
            let step = if i < 70 { 1.3 } else { -1.3 };
            let open = price;
            let close = price + step;
            let high = open.max(close) + 0.4;
            let low = open.min(close) - 0.4;
            candles.push(Candle { time: t, open, high, low, close, volume: 1000.0 });
            price = close;
            t += 300;
        }
        let state = sl_all_consensus_dir(&candles, 12, 10, 5.0, 34);
        assert_eq!(state.len(), candles.len());
        let first = state.iter().find(|d| **d != 0).copied().unwrap();
        assert_eq!(first, 1, "overall consensus must start bullish on an up-trend");
        assert_eq!(*state.last().unwrap(), -1, "must end bearish after the reversal");
        let flips = state.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(flips <= 4, "consensus must not zigzag (flips={flips})");
        // On a noisy series the major-trend line must stay put, not flip with
        // every wobble / divergence.
        let noisy = sl_all_consensus_dir(&synth(400), 12, 10, 5.0, 34);
        let noisy_flips = noisy.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(noisy_flips <= 8, "noise must not flip the trendline (flips={noisy_flips})");
        let reg = registry();
        for id in ["slallconsensus", "slallconsensuso"] {
            let e = reg.iter().find(|e| e.def.id == id).expect("registered");
            assert!(!e.def.hidden);
        }
        // The overlay trendline must never contain a flat segment: within every
        // leg it moves strictly up (bull) or strictly down (bear).
        let entry = reg.into_iter().find(|e| e.def.id == "slallconsensuso").unwrap();
        let os = defaults(&entry);
        let series = compute_slallconsensuso(&candles, &os);
        let pts = &series[0].data;
        assert!(pts.len() >= 2, "overlay must draw a line");
        let mut up = false;
        let mut down = false;
        for w in pts.windows(2) {
            assert!(
                (w[1].value - w[0].value).abs() > 1e-9,
                "overlay must not contain a flat segment"
            );
            if w[1].value > w[0].value {
                up = true;
            } else {
                down = true;
            }
        }
        assert!(up && down, "overlay must have both a rising and a falling leg");
    }
}
