use crate::model::{Candle, Point, SeriesKind, SeriesOut};

pub type Opt = Option<f64>;

pub fn sma_arr(vals: &[f64], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    let mut sum = 0.0f64;
    for i in 0..n {
        sum += vals[i];
        if i >= p {
            sum -= vals[i - p];
        }
        if i + 1 >= p {
            out[i] = Some(sum / p as f64);
        }
    }
    out
}

/// SMAs over an Option series, returning None where any value in the window is None.
pub fn sma_opt(vals: &[Opt], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    let mut sum = 0.0f64;
    let mut cnt = 0usize;
    for i in 0..n {
        match vals[i] {
            Some(v) => {
                sum += v;
                cnt += 1;
            }
            None => {
                // reset window on gap
                sum = 0.0;
                cnt = 0;
            }
        }
        if i >= p {
            if let Some(v) = vals[i - p] {
                sum -= v;
                cnt = cnt.saturating_sub(1);
            }
        }
        if cnt == p {
            out[i] = Some(sum / p as f64);
        }
    }
    out
}

pub fn ema_arr(vals: &[f64], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 || (n as i64) < p {
        return out;
    }
    let p = p as usize;
    let k = 2.0 / (p as f64 + 1.0);
    let seed = sma_arr(vals, p as i64)[p - 1];
    let mut prev = match seed {
        Some(v) => v,
        None => return out,
    };
    out[p - 1] = Some(prev);
    for i in p..n {
        prev = vals[i] * k + prev * (1.0 - k);
        out[i] = Some(prev);
    }
    out
}

/// EMA tolerant of warmup gaps (used by TSI/Fisher second stage).
pub fn ema_skip(src: &[Opt], p: i64) -> Vec<Opt> {
    let n = src.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    let k = 2.0 / (p as f64 + 1.0);
    let mut cnt = 0usize;
    let mut prev: Option<f64> = None;
    let mut seed = 0.0f64;
    for i in 0..n {
        let v = match src[i] {
            Some(v) if v.is_finite() => v,
            _ => continue,
        };
        match prev {
            None => {
                seed += v;
                cnt += 1;
                if cnt == p {
                    prev = Some(seed / p as f64);
                    out[i] = prev;
                }
            }
            Some(pp) => {
                let np = v * k + pp * (1.0 - k);
                prev = Some(np);
                out[i] = Some(np);
            }
        }
    }
    out
}

pub fn highest_arr(vals: &[f64], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    for i in (p - 1)..n {
        let mut m = f64::NEG_INFINITY;
        for j in (i + 1 - p)..=i {
            if vals[j] > m {
                m = vals[j];
            }
        }
        out[i] = Some(m);
    }
    out
}

pub fn lowest_arr(vals: &[f64], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    for i in (p - 1)..n {
        let mut m = f64::INFINITY;
        for j in (i + 1 - p)..=i {
            if vals[j] < m {
                m = vals[j];
            }
        }
        out[i] = Some(m);
    }
    out
}

pub fn stdev_arr(vals: &[f64], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    let mean = sma_arr(vals, p as i64);
    for i in (p - 1)..n {
        let m = match mean[i] {
            Some(m) => m,
            None => continue,
        };
        let mut s = 0.0f64;
        for j in (i + 1 - p)..=i {
            s += (vals[j] - m) * (vals[j] - m);
        }
        out[i] = Some((s / p as f64).sqrt());
    }
    out
}

pub fn wilder_arr(vals: &[f64], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 || (n as i64) < p {
        return out;
    }
    let p = p as usize;
    let mut sum = 0.0f64;
    for v in vals.iter().take(p) {
        sum += *v;
    }
    let mut prev = sum / p as f64;
    out[p - 1] = Some(prev);
    for i in p..n {
        prev = (prev * (p as f64 - 1.0) + vals[i]) / p as f64;
        out[i] = Some(prev);
    }
    out
}

/// Wilder smoothing over an Option series (skips gaps like the raw values).
pub fn wilder_opt(vals: &[Opt], p: i64) -> Vec<Opt> {
    let n = vals.len();
    let mut out = vec![None; n];
    if p <= 0 {
        return out;
    }
    let p = p as usize;
    let mut seed_sum = 0.0f64;
    let mut cnt = 0usize;
    let mut prev: Option<f64> = None;
    for i in 0..n {
        let v = match vals[i] {
            Some(v) if v.is_finite() => v,
            _ => continue,
        };
        match prev {
            None => {
                seed_sum += v;
                cnt += 1;
                if cnt == p {
                    prev = Some(seed_sum / p as f64);
                    out[i] = prev;
                }
            }
            Some(pp) => {
                let np = (pp * (p as f64 - 1.0) + v) / p as f64;
                prev = Some(np);
                out[i] = Some(np);
            }
        }
    }
    out
}

pub fn tr_arr(c: &[Candle]) -> Vec<f64> {
    let n = c.len();
    let mut out = vec![0.0f64; n];
    if n == 0 {
        return out;
    }
    out[0] = c[0].high - c[0].low;
    for i in 1..n {
        out[i] = (c[i].high - c[i].low)
            .max((c[i].high - c[i - 1].close).abs())
            .max((c[i].low - c[i - 1].close).abs());
    }
    out
}

pub fn src_arr(c: &[Candle], key: &str) -> Vec<f64> {
    match key {
        "hl2" => c.iter().map(|x| (x.high + x.low) / 2.0).collect(),
        "hlc3" => c.iter().map(|x| (x.high + x.low + x.close) / 3.0).collect(),
        "hlcc4" => c
            .iter()
            .map(|x| (x.high + x.low + 2.0 * x.close) / 4.0)
            .collect(),
        "open" => c.iter().map(|x| x.open).collect(),
        "high" => c.iter().map(|x| x.high).collect(),
        "low" => c.iter().map(|x| x.low).collect(),
        _ => c.iter().map(|x| x.close).collect(),
    }
}

/// Convert an Option array into series points (drops None/NaN).
pub fn points_from(c: &[Candle], arr: &[Opt]) -> Vec<Point> {
    let mut data = Vec::with_capacity(arr.len());
    for i in 0..arr.len().min(c.len()) {
        if let Some(v) = arr[i] {
            if v.is_finite() {
                data.push(Point {
                    time: c[i].time,
                    value: v,
                    color: None,
                });
            }
        }
    }
    data
}

pub fn build_series(c: &[Candle], arr: &[Opt], color: &str, kind: SeriesKind, line_width: f64) -> SeriesOut {
    let mut s = SeriesOut::line(color, line_width);
    s.kind = kind;
    s.data = points_from(c, arr);
    s
}

pub fn build_hist(c: &[Candle], arr: &[Opt], color: &str) -> SeriesOut {
    let mut s = SeriesOut::hist(color);
    s.data = points_from(c, arr);
    s
}
