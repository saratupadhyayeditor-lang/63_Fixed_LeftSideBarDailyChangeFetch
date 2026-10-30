//! "Market Analysis" tab backend.
//!
//! Serves the four blocks the operator wants in one place:
//!   * FII / DII cash-market buy/sell/net values (NSE provisional report).
//!   * Participant-wise F&O open interest (NSE EOD archive CSV) - the FII / DII
//!     / Pro / Client long/short positions in index & stock futures and options.
//!   * Global equity indices via Yahoo Finance (delayed quotes).
//!   * India indices (NIFTY / BANK NIFTY / FINNIFTY / INDIA VIX / GIFT NIFTY /
//!     SENSEX) from the live Dhan quote feed.
//!
//! NSE has no official keyless API, so its endpoints are called with a browser
//! User-Agent and a session cookie harvested from the homepage. Every source is
//! cached behind a short TTL so a busy tab never hammers the upstreams and a
//! single upstream outage never blanks the whole tab (each section carries its
//! own `ok` flag).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use futures_util::future::join_all;
use serde_json::{json, Value};

use crate::broker::DhanState;
use crate::market;
use algo_core::model::Candle;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// Shared keep-alive client (one connection pool for all three upstreams).
fn client() -> Option<reqwest::Client> {
    static HTTP: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(UA)
            .build()
            .ok()
    })
    .clone()
}

// ---------------------------------------------------------------------------
// Tiny TTL cache
// ---------------------------------------------------------------------------

struct Entry {
    at: Instant,
    data: Value,
}

static CACHE: OnceLock<Mutex<HashMap<&'static str, Entry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<&'static str, Entry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(key: &str, ttl: Duration) -> Option<Value> {
    let g = cache().lock().ok()?;
    let e = g.get(key)?;
    if e.at.elapsed() < ttl {
        Some(e.data.clone())
    } else {
        None
    }
}

fn cache_put(key: &'static str, data: Value) {
    if let Ok(mut g) = cache().lock() {
        g.insert(key, Entry { at: Instant::now(), data });
    }
}

/// Return the cached section when fresh, otherwise run `f` and cache an
/// `ok:true` result (failures are never cached, so the next tick retries).
async fn cached_or<F, Fut>(
    key: &'static str,
    ttl: Duration,
    cl: &Option<reqwest::Client>,
    f: F,
) -> Value
where
    F: FnOnce(reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = Value>,
{
    if let Some(v) = cache_get(key, ttl) {
        return v;
    }
    let Some(c) = cl.clone() else {
        return json!({ "ok": false, "error": "http client unavailable" });
    };
    let v = f(c).await;
    if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
        cache_put(key, v.clone());
    }
    v
}

// ---------------------------------------------------------------------------
// NSE plumbing
// ---------------------------------------------------------------------------

/// GET an NSE endpoint with a warm session cookie. The homepage often answers
/// 403 (bot wall) but still hands out `Set-Cookie`, and the JSON endpoints then
/// accept that cookie. Returns the body text on success.
async fn nse_get(cl: &reqwest::Client, url: &str) -> Option<String> {
    let mut cookies = String::new();
    if let Ok(resp) = cl
        .get("https://www.nseindia.com/")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        for c in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = c.to_str() {
                if let Some(pair) = s.split(';').next() {
                    if !pair.is_empty() {
                        if !cookies.is_empty() {
                            cookies.push_str("; ");
                        }
                        cookies.push_str(pair);
                    }
                }
            }
        }
    }
    let mut req = cl
        .get(url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", "https://www.nseindia.com/");
    if !cookies.is_empty() {
        req = req.header(reqwest::header::COOKIE, cookies);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
}

// ---------------------------------------------------------------------------
// 1. FII / DII cash flows
// ---------------------------------------------------------------------------

async fn fetch_fiidii(cl: reqwest::Client) -> Value {
    let Some(txt) = nse_get(&cl, "https://www.nseindia.com/api/fiidiiTradeReact").await else {
        return json!({ "ok": false, "error": "NSE FII/DII fetch failed" });
    };
    // [{category, date, buyValue, sellValue, netValue}, ...]
    let rows: Vec<Value> = match serde_json::from_str(&txt) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": format!("parse: {e}") }),
    };
    let num = |v: &Value, k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| v.get(k).and_then(|x| x.as_f64()))
            .unwrap_or(0.0)
    };
    let mut fii = Value::Null;
    let mut dii = Value::Null;
    let mut date = String::new();
    for r in &rows {
        let cat = r.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let entry = json!({
            "category": cat,
            "buy": num(r, "buyValue"),
            "sell": num(r, "sellValue"),
            "net": num(r, "netValue"),
        });
        if date.is_empty() {
            date = r.get("date").and_then(|v| v.as_str()).unwrap_or("").to_string();
        }
        if cat.contains("FII") || cat.contains("FPI") {
            fii = entry;
        } else if cat.eq_ignore_ascii_case("DII") {
            dii = entry;
        }
    }
    json!({ "ok": true, "date": date, "fii": fii, "dii": dii, "rows": rows })
}

// ---------------------------------------------------------------------------
// 2. Participant-wise F&O open interest (EOD archive)
// ---------------------------------------------------------------------------

/// Howard Hinnant civil_from_days: epoch day -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// IST calendar date for a Unix timestamp.
fn ist_ymd(secs: i64) -> (i64, u32, u32) {
    civil_from_days((secs + 19_800).div_euclid(86_400))
}

fn month_name(m: u32) -> &'static str {
    const M: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    M.get((m as usize).saturating_sub(1)).copied().unwrap_or("")
}

fn split_csv(line: &str) -> Vec<&str> {
    line.split(',').collect()
}

/// Parse the NSE participant-OI CSV into a header list plus `{name, values}` rows.
fn parse_participant_csv(txt: &str) -> Option<(Vec<String>, Vec<Value>)> {
    let mut lines = txt.lines().filter(|l| !l.trim().is_empty());
    let _title = lines.next()?;
    let header_line = lines.next()?;
    let headers: Vec<String> = split_csv(header_line)
        .iter()
        .map(|s| s.trim().to_string())
        .collect();
    if headers.len() < 2 {
        return None;
    }
    let mut rows = Vec::new();
    for line in lines {
        let cells = split_csv(line);
        if cells.len() < 2 {
            continue;
        }
        let name = cells[0].trim().to_string();
        if name.is_empty() {
            continue;
        }
        let values: Vec<Option<f64>> = cells[1..]
            .iter()
            .map(|c| c.trim().parse::<f64>().ok())
            .collect();
        rows.push(json!({ "name": name, "values": values }));
    }
    if rows.is_empty() {
        return None;
    }
    Some((headers, rows))
}

async fn fetch_participant(cl: reqwest::Client) -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Walk back up to 9 days so weekends/holidays land on the last session.
    for back in 0..9i64 {
        let (y, m, d) = ist_ymd(now - back * 86_400);
        let fname = format!("fao_participant_oi_{:02}{:02}{:04}.csv", d, m, y);
        let url = format!("https://nsearchives.nseindia.com/content/nsccl/{fname}");
        let resp = cl
            .get(&url)
            .header("Referer", "https://www.nseindia.com/")
            .send()
            .await;
        let Ok(r) = resp else { continue };
        if !r.status().is_success() {
            continue;
        }
        let Ok(txt) = r.text().await else { continue };
        if let Some((columns, rows)) = parse_participant_csv(&txt) {
            return json!({
                "ok": true,
                "date": format!("{:02}-{}-{:04}", d, month_name(m), y),
                "file": fname,
                "columns": columns,
                "rows": rows,
            });
        }
    }
    json!({ "ok": false, "error": "no recent participant OI file found" })
}

// ---------------------------------------------------------------------------
// 3. Global indices (Yahoo Finance)
// ---------------------------------------------------------------------------

const GLOBAL: &[(&str, &str)] = &[
    ("S&P 500", "%5EGSPC"),
    ("Dow Jones", "%5EDJI"),
    ("Nasdaq", "%5EIXIC"),
    ("Nikkei 225", "%5EN225"),
    ("Hang Seng", "%5EHSI"),
    ("FTSE 100", "%5EFTSE"),
    ("DAX", "%5EGDAXI"),
    ("US Dollar Index", "DX-Y.NYB"),
    ("Crude Oil (WTI)", "CL%3DF"),
    ("Brent Crude", "BZ%3DF"),
    ("US 10Y Yield", "%5ETNX"),
];

/// Global rows that are NOT equity indices, so they must not enter the
/// "global breadth" average (a rising dollar or crude is not a risk-on signal).
fn is_non_equity(name: &str) -> bool {
    name.contains("Dollar") || name.contains("Crude") || name.contains("Brent") || name.contains("Yield")
}

async fn fetch_global_one(cl: reqwest::Client, name: &'static str, sym: &'static str) -> Value {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{sym}?interval=1d&range=5d"
    );
    let resp = cl.get(&url).header("Accept", "application/json").send().await;
    if let Ok(r) = resp {
        if r.status().is_success() {
            if let Ok(v) = r.json::<Value>().await {
                if let Some(m) = v.pointer("/chart/result/0/meta") {
                    // Daily closes let us derive the previous close reliably; some
                    // symbols (notably futures like CL=F) report chartPreviousClose
                    // equal to the live price on a 1-day range, which zeroes the change.
                    let closes: Vec<f64> = v
                        .pointer("/chart/result/0/indicators/quote/0/close")
                        .and_then(|c| c.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
                        .unwrap_or_default();
                    let last_close = closes.last().copied().unwrap_or(0.0);
                    let mut price = jsonf(m, "regularMarketPrice");
                    if price <= 0.0 {
                        price = last_close;
                    }
                    let prev = {
                        let arr_prev = if closes.len() >= 2 {
                            closes[closes.len() - 2]
                        } else {
                            0.0
                        };
                        let meta_prev = {
                            let p = jsonf(m, "chartPreviousClose");
                            if p > 0.0 { p } else { jsonf(m, "previousClose") }
                        };
                        // Prefer the second-to-last daily close; fall back to meta
                        // only when it is a genuine prior value (not equal to price).
                        if arr_prev > 0.0 {
                            arr_prev
                        } else if meta_prev > 0.0 && (meta_prev - price).abs() > 1e-9 {
                            meta_prev
                        } else {
                            0.0
                        }
                    };
                    let change = if prev > 0.0 { price - prev } else { 0.0 };
                    let pct = if prev > 0.0 { change / prev * 100.0 } else { 0.0 };
                    // A yield is quoted in percent, not a currency.
                    let currency = if name.contains("Yield") {
                        "%"
                    } else {
                        m.get("currency").and_then(|c| c.as_str()).unwrap_or("")
                    };
                    return json!({
                        "name": name,
                        "ok": price > 0.0,
                        "price": price,
                        "prevClose": prev,
                        "change": change,
                        "changePct": pct,
                        "currency": currency,
                    });
                }
            }
        }
    }
    json!({ "name": name, "ok": false })
}

fn jsonf(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

async fn fetch_global(cl: reqwest::Client) -> Value {
    let futs = GLOBAL.iter().map(|(name, sym)| {
        fetch_global_one(cl.clone(), name, sym)
    });
    let rows: Vec<Value> = join_all(futs).await;
    let any = rows
        .iter()
        .any(|r| r.get("ok").and_then(|x| x.as_bool()).unwrap_or(false));
    json!({ "ok": any, "rows": rows })
}

// ---------------------------------------------------------------------------
// 4. India indices (live Dhan feed)
// ---------------------------------------------------------------------------

const INDIA: &[(i64, &str, &str)] = &[
    (13, "NIFTY 50", "IDX_I"),
    (25, "BANK NIFTY", "IDX_I"),
    (27, "FINNIFTY", "IDX_I"),
    (21, "INDIA VIX", "IDX_I"),
    (5024, "GIFT NIFTY", "IDX_I"),
    (51, "SENSEX", "IDX_I"),
];

async fn fetch_india(dhan: &DhanState) -> Value {
    let Some(cl) = dhan.session_client().await else {
        return json!({ "ok": false, "error": "Dhan session not connected" });
    };
    dhan.dhan_throttle().await;
    let mut req: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (sid, _name, seg) in INDIA {
        req.entry((*seg).to_string()).or_default().push(*sid);
    }
    match cl.market_feed_quote(&req).await {
        Ok(resp) => {
            let mut map: HashMap<i64, &dhan_hq::QuoteEntry> = HashMap::new();
            for (_seg, legs) in resp.iter() {
                for (sid, e) in legs {
                    if let Ok(id) = sid.parse::<i64>() {
                        map.insert(id, e);
                    }
                }
            }
            // Dhan's quote feed is frozen for the GIFT NIFTY index (its candle
            // series is correct), so mirror the latest candle close below.
            let gift_ltp = dhan
                .fetch_candles(5024, "IDX_I", "INDEX", "1min")
                .await
                .ok()
                .and_then(|c| c.last().map(|x| x.close))
                .filter(|v| *v > 0.0);
            // The REST snapshot reports no previous close outside market hours
            // (and GIFT NIFTY's is frozen), so seed the prior-session close from
            // the daily candle series for every index that lacks one.
            let mut daily_prev: HashMap<i64, f64> = HashMap::new();
            for (id, _name, _seg) in INDIA {
                let needs = map
                    .get(id)
                    .map(|e| e.previous_close_price <= 0.0)
                    .unwrap_or(true);
                if !needs {
                    continue;
                }
                if let Ok(c) = dhan.fetch_candles(*id, "IDX_I", "INDEX", "1day").await {
                    if c.len() >= 2 {
                        let v = c[c.len() - 2].close;
                        if v > 0.0 {
                            daily_prev.insert(*id, v);
                        }
                    }
                }
            }
            let rows: Vec<Value> = INDIA
                .iter()
                .map(|(id, name, _seg)| {
                    let e = map.get(id);
                    let mut price = e.map(|x| x.last_price).unwrap_or(0.0);
                    let net = e.map(|x| x.net_change).unwrap_or(0.0);
                    // Dhan's authoritative previous close (never `ohlc.close`,
                    // which equals the day close and would zero the change).
                    let mut prev = e.map(|x| x.previous_close_price).unwrap_or(0.0);
                    if prev <= 0.0 && net != 0.0 {
                        prev = price - net;
                    }
                    if let Some(p) = daily_prev.get(id) {
                        prev = *p;
                    }
                    if *id == 5024 {
                        if let Some(g) = gift_ltp {
                            price = g;
                        }
                    }
                    let change = if prev > 0.0 {
                        price - prev
                    } else if net != 0.0 {
                        net
                    } else {
                        0.0
                    };
                    let pct = if prev > 0.0 { change / prev * 100.0 } else { 0.0 };
                    json!({
                        "name": name,
                        "securityId": id,
                        "ok": price > 0.0,
                        "price": price,
                        "change": change,
                        "changePct": pct,
                    })
                })
                .collect();
            json!({ "ok": true, "rows": rows })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// 5. Next-session NIFTY bias (gap + trend)
// ---------------------------------------------------------------------------

fn row<'a>(section: &'a Value, name: &str) -> Option<&'a Value> {
    section
        .get("rows")?
        .as_array()?
        .iter()
        .find(|r| r.get("name").and_then(|n| n.as_str()) == Some(name))
}

fn fnum(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Read a participant-OI cell. `columns[0]` is the participant label, so the
/// value for column `i` sits at `values[i-1]`.
fn pcell(section: &Value, participant: &str, col: &str) -> f64 {
    let Some(cols) = section.get("columns").and_then(|c| c.as_array()) else {
        return 0.0;
    };
    let Some(ci) = cols.iter().position(|c| c.as_str() == Some(col)) else {
        return 0.0;
    };
    if ci == 0 {
        return 0.0;
    }
    let Some(rows) = section.get("rows").and_then(|r| r.as_array()) else {
        return 0.0;
    };
    let Some(r) = rows
        .iter()
        .find(|r| r.get("name").and_then(|n| n.as_str()) == Some(participant))
    else {
        return 0.0;
    };
    r.get("values")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(ci - 1))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0)
}

#[allow(clippy::too_many_arguments)]
fn addf(
    factors: &mut Vec<Value>,
    group: &str,
    name: &str,
    value: String,
    points: f64,
    note: &str,
) {
    let p = (points * 100.0).round() / 100.0;
    let impact = if p > 0.05 {
        "bull"
    } else if p < -0.05 {
        "bear"
    } else {
        "neutral"
    };
    factors.push(json!({
        "group": group,
        "name": name,
        "value": value,
        "points": p,
        "impact": impact,
        "note": note,
    }));
}

fn verdict_label(direction: &str) -> &'static str {
    match direction {
        "up" => "GAP UP",
        "down" => "GAP DOWN",
        "flat" => "FLAT / NO CLEAR GAP",
        "bull" => "BULLISH",
        "bear" => "BEARISH",
        _ => "NEUTRAL",
    }
}

/// Transparent, weighted bias engine. Every input is EOD (FII/DII cash,
/// participant F&O OI) or live overnight (GIFT NIFTY, global indices, USD index,
/// India VIX). GIFT NIFTY dominates the gap estimate because it trades almost
/// round the clock and leads the NIFTY open; global/USD are secondary and
/// partly overlapping. This is an indicative read, not investment advice.
fn predict(fiidii: &Value, participant: &Value, global: &Value, india: &Value) -> Value {
    let mut factors: Vec<Value> = Vec::new();

    // --- live India inputs ---
    let gift = row(india, "GIFT NIFTY");
    let gift_ok = gift
        .and_then(|r| r.get("ok"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let gift_pct = gift.map(|r| fnum(r, "changePct")).unwrap_or(0.0);
    let nifty_pct = row(india, "NIFTY 50")
        .map(|r| fnum(r, "changePct"))
        .unwrap_or(0.0);
    let vix = row(india, "INDIA VIX")
        .map(|r| fnum(r, "price"))
        .unwrap_or(0.0);

    // --- global inputs ---
    let gv = |n: &str| row(global, n).map(|r| fnum(r, "changePct")).unwrap_or(0.0);
    let us_avg = (gv("S&P 500") + gv("Dow Jones") + gv("Nasdaq")) / 3.0;
    let dxy = gv("US Dollar Index");
    let glist: Vec<f64> = global
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter(|r| r.get("ok").and_then(|b| b.as_bool()).unwrap_or(false))
                .filter(|r| !is_non_equity(r.get("name").and_then(|n| n.as_str()).unwrap_or("")))
                .map(|r| fnum(r, "changePct"))
                .collect()
        })
        .unwrap_or_default();
    let global_avg = if glist.is_empty() {
        0.0
    } else {
        glist.iter().sum::<f64>() / glist.len() as f64
    };

    // --- FII / DII cash ---
    let fiidii_ok = fiidii.get("ok").and_then(|b| b.as_bool()).unwrap_or(false);
    let fii_net = fiidii.get("fii").map(|f| fnum(f, "net")).unwrap_or(0.0);
    let dii_net = fiidii.get("dii").map(|d| fnum(d, "net")).unwrap_or(0.0);

    // --- participant F&O OI (FII) ---
    let part_ok = participant
        .get("ok")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let fii_fl = pcell(participant, "FII", "Future Index Long");
    let fii_fs = pcell(participant, "FII", "Future Index Short");
    let fii_cl = pcell(participant, "FII", "Option Index Call Long");
    let fii_pl = pcell(participant, "FII", "Option Index Put Long");
    let fii_fut_ratio = if fii_fl + fii_fs > 0.0 {
        (fii_fl - fii_fs) / (fii_fl + fii_fs)
    } else {
        0.0
    };
    let fii_opt_ratio = if fii_cl + fii_pl > 0.0 {
        (fii_cl - fii_pl) / (fii_cl + fii_pl)
    } else {
        0.0
    };

    // ---- GAP score (next-session open) ----
    let mut gap_score = 0.0;
    if gift_ok {
        let p = clamp(gift_pct, -2.5, 2.5) * 3.0;
        gap_score += p;
        addf(
            &mut factors,
            "gap",
            "GIFT NIFTY (overnight)",
            format!("{gift_pct:+.2}%"),
            p,
            "leads the NIFTY open",
        );
    }
    let p_us = clamp(us_avg, -2.5, 2.5) * 1.5;
    gap_score += p_us;
    addf(
        &mut factors,
        "gap",
        "US markets (S&P/Dow/Nasdaq)",
        format!("{us_avg:+.2}%"),
        p_us,
        "overnight risk tone",
    );
    let p_glob = clamp(global_avg, -2.5, 2.5) * 1.0;
    gap_score += p_glob;
    addf(
        &mut factors,
        "gap",
        "Global average",
        format!("{global_avg:+.2}%"),
        p_glob,
        "Europe + Asia breadth",
    );
    let p_dxy = clamp(-dxy, -2.5, 2.5) * 1.0;
    gap_score += p_dxy;
    addf(
        &mut factors,
        "gap",
        "US Dollar Index",
        format!("{dxy:+.2}%"),
        p_dxy,
        "strong USD pressures FII flows",
    );
    let p_vix = if vix > 0.0 {
        clamp((15.0 - vix) * 0.2, -1.0, 1.0)
    } else {
        0.0
    };
    gap_score += p_vix;
    addf(
        &mut factors,
        "gap",
        "India VIX level",
        if vix > 0.0 { format!("{vix:.2}") } else { "n/a".to_string() },
        p_vix,
        "high VIX = risk-off, wider moves",
    );

    // Crude is the biggest imported-inflation input for India, so an overnight
    // spike is a gap-down risk even when GIFT/global equities look calm. Average
    // WTI and Brent when both are live; keep the factor out when neither is.
    let crude_row = |name: &str| {
        row(global, name).filter(|r| {
            r.get("ok").and_then(|b| b.as_bool()).unwrap_or(false)
        })
    };
    let crude_pcts: Vec<f64> = [
        crude_row("Crude Oil (WTI)").map(|r| fnum(r, "changePct")),
        crude_row("Brent Crude").map(|r| fnum(r, "changePct")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let crude_gap_pct = if crude_pcts.is_empty() {
        0.0
    } else {
        crude_pcts.iter().sum::<f64>() / crude_pcts.len() as f64
    };
    if !crude_pcts.is_empty() {
        let p_cr = clamp(-crude_gap_pct, -2.5, 2.5) * 0.6;
        gap_score += p_cr;
        addf(
            &mut factors,
            "gap",
            "Crude oil (WTI/Brent)",
            format!("{crude_gap_pct:+.2}%"),
            p_cr,
            "higher crude raises India's import bill / inflation risk",
        );
    }

    let gap_dir = if gap_score >= 1.0 {
        "up"
    } else if gap_score <= -1.0 {
        "down"
    } else {
        "flat"
    };
    let est_pct = 0.7 * gift_pct + 0.2 * us_avg - 0.1 * dxy;
    // GIFT NIFTY is the anchor for the open; without it a gap call is tentative.
    let (gap_conf, gap_label) = if gift_ok {
        (
            clamp(50.0 + gap_score.abs() * 12.0, 50.0, 95.0),
            verdict_label(gap_dir).to_string(),
        )
    } else {
        (
            clamp(25.0 + gap_score.abs() * 5.0, 25.0, 45.0),
            format!("{} (tentative)", verdict_label(gap_dir)),
        )
    };
    let gap_conf = gap_conf.round() as i64;

    // ---- TREND score (next-session direction) ----
    let mut trend_score = 0.0;
    if fiidii_ok {
        let p_fii = clamp(fii_net / 1500.0, -2.5, 2.5);
        trend_score += p_fii;
        addf(
            &mut factors,
            "trend",
            "FII cash net",
            format!("{fii_net:+.0} cr"),
            p_fii,
            "primary institutional flow",
        );
        let p_dii = clamp(dii_net / 2000.0, -2.0, 2.0);
        trend_score += p_dii;
        addf(
            &mut factors,
            "trend",
            "DII cash net",
            format!("{dii_net:+.0} cr"),
            p_dii,
            "domestic support / cushion",
        );
    }
    if part_ok {
        let p_fut = clamp(fii_fut_ratio * 3.0, -3.0, 3.0);
        trend_score += p_fut;
        addf(
            &mut factors,
            "trend",
            "FII index-futures net",
            format!("{:+.0}%", fii_fut_ratio * 100.0),
            p_fut,
            "net long (+) / short (-) positioning",
        );
        let p_opt = clamp(fii_opt_ratio * 3.0, -3.0, 3.0);
        trend_score += p_opt;
        addf(
            &mut factors,
            "trend",
            "FII index-options bias",
            format!("{:+.0}%", fii_opt_ratio * 100.0),
            p_opt,
            "call-long vs put-long skew",
        );
    }
    // --- participant F&O OI (Pro / proprietary desks) ---
    if part_ok {
        let pro_fl = pcell(participant, "Pro", "Future Index Long");
        let pro_fs = pcell(participant, "Pro", "Future Index Short");
        let pro_cl = pcell(participant, "Pro", "Option Index Call Long");
        let pro_pl = pcell(participant, "Pro", "Option Index Put Long");
        if pro_fl + pro_fs > 0.0 {
            let ratio = (pro_fl - pro_fs) / (pro_fl + pro_fs);
            let p = clamp(ratio * 2.0, -2.0, 2.0);
            trend_score += p;
            addf(
                &mut factors,
                "trend",
                "Pro index-futures net",
                format!("{:+.0}%", ratio * 100.0),
                p,
                "proprietary desks net positioning",
            );
        }
        if pro_cl + pro_pl > 0.0 {
            let ratio = (pro_cl - pro_pl) / (pro_cl + pro_pl);
            let p = clamp(ratio * 1.5, -1.5, 1.5);
            trend_score += p;
            addf(
                &mut factors,
                "trend",
                "Pro index-options bias",
                format!("{:+.0}%", ratio * 100.0),
                p,
                "pro call-long vs put-long skew",
            );
        }
    }
    let p_mom = clamp(nifty_pct, -2.5, 2.5) * 0.5;
    trend_score += p_mom;
    addf(
        &mut factors,
        "trend",
        "NIFTY momentum",
        format!("{nifty_pct:+.2}%"),
        p_mom,
        "last-session price action",
    );
    let p_gt = clamp(global_avg, -2.5, 2.5) * 1.0;
    trend_score += p_gt;
    addf(
        &mut factors,
        "trend",
        "Global risk tone",
        format!("{global_avg:+.2}%"),
        p_gt,
        "external direction",
    );
    if !crude_pcts.is_empty() {
        let p_cr = clamp(-crude_gap_pct * 0.4, -0.5, 0.5);
        trend_score += p_cr;
        addf(
            &mut factors,
            "trend",
            "Crude oil (import cost)",
            format!("{crude_gap_pct:+.2}%"),
            p_cr,
            "higher crude pressures India (net importer)",
        );
    }
    let p_vt = if vix > 0.0 {
        clamp((15.0 - vix) * 0.2, -1.0, 1.0)
    } else {
        0.0
    };
    trend_score += p_vt;
    addf(
        &mut factors,
        "trend",
        "India VIX trend",
        if vix > 0.0 { format!("{vix:.2}") } else { "n/a".to_string() },
        p_vt,
        "falling VIX supports bulls",
    );

    let trend_dir = if trend_score >= 1.5 {
        "bull"
    } else if trend_score <= -1.5 {
        "bear"
    } else {
        "neutral"
    };
    let trend_conf = clamp(50.0 + trend_score.abs() * 7.0, 50.0, 90.0).round() as i64;

    json!({
        "ok": true,
        "gap": {
            "direction": gap_dir,
            "label": gap_label,
            "estPct": (est_pct * 100.0).round() / 100.0,
            "score": (gap_score * 100.0).round() / 100.0,
            "confidence": gap_conf,
        },
        "trend": {
            "direction": trend_dir,
            "label": verdict_label(trend_dir),
            "score": (trend_score * 100.0).round() / 100.0,
            "confidence": trend_conf,
        },
        "factors": factors,
        "disclaimer": "Indicative bias from GIFT NIFTY, global markets, crude oil, USD index, India VIX, \
FII/DII cash flows and FII/Pro participant F&O open interest. Not investment advice. Best read after \
the India close, for the next session.",
    })
}

// ---------------------------------------------------------------------------
// 5b. Gap-up / gap-down statistics + trend-prediction accuracy (backtest)
// ---------------------------------------------------------------------------

/// Epoch day -> `YYYY-MM-DD` (candles are already shifted to IST wall-clock).
fn day_label(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_ymd(s: &str) -> Option<i64> {
    let mut it = s.trim().split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Monday of the week containing `day` (epoch days).
fn week_start(day: i64) -> i64 {
    day - (day + 3).rem_euclid(7)
}

fn ma_pct(ok: i64, total: i64) -> f64 {
    if total <= 0 {
        0.0
    } else {
        (ok as f64 * 1000.0 / total as f64).round() / 10.0
    }
}

#[derive(Default)]
struct GapBucket {
    sessions: i64,
    gap_ups: i64,
    gap_up_ok: i64,
    gap_downs: i64,
    gap_down_ok: i64,
    trend_preds: i64,
    trend_ok: i64,
}

fn bucket_label(day: i64, bucket: &str) -> String {
    match bucket {
        "week" => day_label(week_start(day)),
        "month" => day_label(day)[..7].to_string(),
        "year" => day_label(day)[..4].to_string(),
        _ => day_label(day),
    }
}

/// Pure gap engine: walk the daily series once, classify every session's open
/// against the previous close and score the price-action trend prediction
/// (green candle -> next session up, red candle -> next session down) against
/// the next session's close. Kept separate from the HTTP handler so the
/// classification rules are unit-testable without a Dhan session.
fn gap_stats_from(candles: &[Candle], from_day: i64, to_day: i64, bucket: &str) -> Value {
    // Normalise to one row per IST day (Dhan can repeat a day on the daily
    // endpoint); the last row of the day wins.
    let mut days: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for c in candles {
        if c.open > 0.0 && c.close > 0.0 {
            days.insert(c.time.div_euclid(86400), (c.open, c.close));
        }
    }
    let rows: Vec<(i64, f64, f64)> = days
        .into_iter()
        .map(|(d, (o, c))| (d, o, c))
        .collect();

    let mut buckets: BTreeMap<String, GapBucket> = BTreeMap::new();
    let mut detail: Vec<Value> = Vec::new();
    let mut sessions = 0i64;
    let mut gap_ups = 0i64;
    let mut gap_up_ok = 0i64;
    let mut gap_downs = 0i64;
    let mut gap_down_ok = 0i64;
    let mut trend_preds = 0i64;
    let mut trend_ok = 0i64;

    for i in 1..rows.len() {
        let (day, open, close) = rows[i];
        if day < from_day || day > to_day {
            continue;
        }
        let prev_close = rows[i - 1].2;
        if prev_close <= 0.0 {
            continue;
        }
        sessions += 1;
        let gap_pct = (open - prev_close) / prev_close * 100.0;
        let direction = if gap_pct > 0.0 {
            "up"
        } else if gap_pct < 0.0 {
            "down"
        } else {
            "flat"
        };
        // A gap-up "succeeds" when the session holds above the gap-open
        // (close > open); a gap-down succeeds when it holds below (close < open).
        let success = match direction {
            "up" => close > open,
            "down" => close < open,
            _ => false,
        };

        // Trend prediction made on THIS session for the NEXT session.
        let next = rows.get(i + 1);
        let pred = if close > open {
            "bull"
        } else if close < open {
            "bear"
        } else {
            "flat"
        };
        let (actual, correct) = match next {
            Some((_, _nopen, nclose)) if pred != "flat" => {
                let actual = if *nclose > close {
                    "bull"
                } else if *nclose < close {
                    "bear"
                } else {
                    "flat"
                };
                (actual, actual == pred)
            }
            _ => ("", false),
        };

        let key = bucket_label(day, bucket);
        let b = buckets.entry(key).or_default();
        b.sessions += 1;
        match direction {
            "up" => {
                gap_ups += 1;
                b.gap_ups += 1;
                if success {
                    gap_up_ok += 1;
                    b.gap_up_ok += 1;
                }
            }
            "down" => {
                gap_downs += 1;
                b.gap_downs += 1;
                if success {
                    gap_down_ok += 1;
                    b.gap_down_ok += 1;
                }
            }
            _ => {}
        }
        if pred != "flat" && !actual.is_empty() {
            trend_preds += 1;
            b.trend_preds += 1;
            if correct {
                trend_ok += 1;
                b.trend_ok += 1;
            }
        }
        if direction != "flat" {
            detail.push(json!({
                "date": day_label(day),
                "prevClose": (prev_close * 100.0).round() / 100.0,
                "open": (open * 100.0).round() / 100.0,
                "close": (close * 100.0).round() / 100.0,
                "gapPct": (gap_pct * 100.0).round() / 100.0,
                "direction": direction,
                "success": success,
                "prediction": pred,
                "actual": actual,
                "correct": correct,
            }));
        }
    }

    let bucket_rows: Vec<Value> = buckets
        .iter()
        .map(|(label, b)| {
            json!({
                "label": label,
                "sessions": b.sessions,
                "gapUps": b.gap_ups,
                "gapUpSuccess": b.gap_up_ok,
                "gapUpRate": ma_pct(b.gap_up_ok, b.gap_ups),
                "gapDowns": b.gap_downs,
                "gapDownSuccess": b.gap_down_ok,
                "gapDownRate": ma_pct(b.gap_down_ok, b.gap_downs),
                "trendPredictions": b.trend_preds,
                "trendCorrect": b.trend_ok,
                "trendAccuracy": ma_pct(b.trend_ok, b.trend_preds),
            })
        })
        .collect();

    json!({
        "ok": true,
        "from": day_label(from_day),
        "to": day_label(to_day),
        "bucket": bucket,
        "source": "NIFTY 50 daily candles",
        "sessions": sessions,
        "gapUps": gap_ups,
        "gapUpSuccess": gap_up_ok,
        "gapUpFail": gap_ups - gap_up_ok,
        "gapUpRate": ma_pct(gap_up_ok, gap_ups),
        "gapDowns": gap_downs,
        "gapDownSuccess": gap_down_ok,
        "gapDownFail": gap_downs - gap_down_ok,
        "gapDownRate": ma_pct(gap_down_ok, gap_downs),
        "trendPredictions": trend_preds,
        "trendCorrect": trend_ok,
        "trendAccuracy": ma_pct(trend_ok, trend_preds),
        "buckets": bucket_rows,
        "rows": detail,
        "note": "Gap-up = today's open above yesterday's close; successful when the \
session holds above its open (close > open). Gap-down mirrors it. Trend accuracy is a \
backtest of the price-action signal (green/red session -> next-session direction).",
    })
}

/// NIFTY 50 daily OHLC from Yahoo Finance (keyless, works while the Dhan session
/// is disconnected). The analysis tab already trusts Yahoo for the global
/// indices; using it here keeps the gap statistics usable after the India close.
async fn fetch_nifty_daily(cl: reqwest::Client) -> Option<Vec<Candle>> {
    let url = "https://query1.finance.yahoo.com/v8/finance/chart/%5ENSEI?interval=1d&range=2y";
    let resp = cl.get(url).header("Accept", "application/json").send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let res = v.pointer("/chart/result/0")?;
    let ts = res.get("timestamp")?.as_array()?;
    let q = res.pointer("/indicators/quote/0")?;
    let opens = q.get("open")?.as_array()?;
    let closes = q.get("close")?.as_array()?;
    let highs = q.get("high").and_then(|x| x.as_array());
    let lows = q.get("low").and_then(|x| x.as_array());
    let mut out: Vec<Candle> = Vec::new();
    for i in 0..ts.len().min(opens.len()).min(closes.len()) {
        let Some(t) = ts[i]
            .as_i64()
            .or_else(|| ts[i].as_f64().map(|v| v as i64))
        else {
            continue;
        };
        let (Some(o), Some(c)) = (opens[i].as_f64(), closes[i].as_f64()) else {
            continue;
        };
        if o <= 0.0 || c <= 0.0 {
            continue;
        }
        let h = highs
            .and_then(|a| a.get(i))
            .and_then(|x| x.as_f64())
            .filter(|x| *x > 0.0)
            .unwrap_or(o.max(c));
        let l = lows
            .and_then(|a| a.get(i))
            .and_then(|x| x.as_f64())
            .filter(|x| *x > 0.0)
            .unwrap_or(o.min(c));
        out.push(Candle {
            time: t + 19800,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 0.0,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// `GET /api/market/gap-stats?from=YYYY-MM-DD&to=YYYY-MM-DD&bucket=day|week|month|year`
pub async fn gap_stats(
    State(dhan): State<DhanState>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let today = (market::now_secs() + 19800).div_euclid(86400);
    let mut from_day = q
        .get("from")
        .and_then(|s| parse_ymd(s))
        .unwrap_or(today - 90);
    let mut to_day = q.get("to").and_then(|s| parse_ymd(s)).unwrap_or(today);
    if from_day > to_day {
        std::mem::swap(&mut from_day, &mut to_day);
    }
    // The daily endpoint carries ~2 years; clamp so a typo cannot request the
    // whole calendar.
    if to_day - from_day > 730 {
        from_day = to_day - 730;
    }
    let bucket = match q.get("bucket").map(|s| s.as_str()) {
        Some("week") => "week",
        Some("month") => "month",
        Some("year") => "year",
        _ => "day",
    };
    let yahoo = match client() {
        Some(cl) => fetch_nifty_daily(cl).await,
        None => None,
    };
    let (candles, source) = match yahoo {
        Some(c) => (c, "NIFTY 50 daily candles (Yahoo)"),
        None => match dhan.fetch_candles(13, "IDX_I", "INDEX", "1day").await {
            Ok(c) => (c, "NIFTY 50 daily candles (Dhan)"),
            Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
        },
    };
    let mut out = gap_stats_from(&candles, from_day, to_day, bucket);
    out["source"] = json!(source);
    Json(out)
}



/// `GET /api/market/analysis` - every section in one payload.
pub async fn analysis(State(dhan): State<DhanState>) -> impl IntoResponse {
    let cl = client();
    let fiidii = cached_or("fiidii", Duration::from_secs(300), &cl, fetch_fiidii).await;
    let participant =
        cached_or("participant", Duration::from_secs(3600), &cl, fetch_participant).await;
    let global = cached_or("global", Duration::from_secs(60), &cl, fetch_global).await;
    let india = if let Some(v) = cache_get("india", Duration::from_secs(20)) {
        v
    } else {
        let v = fetch_india(&dhan).await;
        if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            cache_put("india", v.clone());
        }
        v
    };
    let prediction = predict(&fiidii, &participant, &global, &india);
    let news = cached_or("news", Duration::from_secs(600), &cl, fetch_news).await;
    let news_prediction = news_prediction(&news);
    let overall = overall_prediction(&prediction, &news_prediction, &news, &global);
    Json(json!({
        "ok": true,
        "pulledAt": market::now_secs(),
        "fiidii": fiidii,
        "participant": participant,
        "global": global,
        "india": india,
        "prediction": prediction,
        "news": news,
        "newsPrediction": news_prediction,
        "overall": overall,
    }))
}

// ---------------------------------------------------------------------------
// 5. News feed (keyless RSS: Google News queries + official/publisher feeds)
// ---------------------------------------------------------------------------

/// (region, category, full RSS url). Google News search queries cover the
/// requested topics without any API key; the extra publisher/official feeds add
/// authoritative sources. Parsed server-side and cached behind a TTL.
const NEWS_FEEDS: &[(&str, &str, &str)] = &[
    ("India", "Markets", "https://news.google.com/rss/search?q=Nifty%20OR%20Sensex%20OR%20stock%20market&hl=en-IN&gl=IN&ceid=IN:en"),
    ("India", "RBI / Policy", "https://news.google.com/rss/search?q=RBI%20monetary%20policy%20OR%20repo%20rate%20OR%20SEBI&hl=en-IN&gl=IN&ceid=IN:en"),
    ("India", "Economy / Data", "https://news.google.com/rss/search?q=India%20inflation%20OR%20GDP%20OR%20IIP%20OR%20rupee&hl=en-IN&gl=IN&ceid=IN:en"),
    ("India", "Budget / GST", "https://news.google.com/rss/search?q=India%20budget%20OR%20GST&hl=en-IN&gl=IN&ceid=IN:en"),
    ("India", "Earnings", "https://news.google.com/rss/search?q=India%20quarterly%20results%20earnings&hl=en-IN&gl=IN&ceid=IN:en"),
    ("Global", "US Fed", "https://news.google.com/rss/search?q=Federal%20Reserve%20rate%20decision%20OR%20FOMC&hl=en-IN&gl=IN&ceid=IN:en"),
    ("Global", "US Data", "https://news.google.com/rss/search?q=US%20CPI%20inflation%20OR%20nonfarm%20payrolls&hl=en-IN&gl=IN&ceid=IN:en"),
    ("Global", "Global Markets", "https://news.google.com/rss/search?q=global%20stock%20markets%20OR%20Wall%20Street&hl=en-IN&gl=IN&ceid=IN:en"),
    ("Global", "Crude Oil", "https://news.google.com/rss/search?q=crude%20oil%20price%20OR%20Brent%20OR%20OPEC&hl=en-IN&gl=IN&ceid=IN:en"),
    ("Global", "Geopolitics", "https://news.google.com/rss/search?q=geopolitics%20OR%20sanctions%20OR%20war%20OR%20Middle%20East&hl=en-IN&gl=IN&ceid=IN:en"),
    ("India", "Markets", "https://economictimes.indiatimes.com/markets/rssfeeds/1977021501.cms"),
    ("Global", "US Fed", "https://www.federalreserve.gov/feeds/press_all.xml"),
    ("India", "Policy", "https://pib.gov.in/RssMain.aspx?ModId=6&Lang=1&Regid=3"),
];

const BULL_WORDS: &[&str] = &[
    "surge", "surges", "rally", "rallies", "jump", "jumps", "gain", "gains", "rise", "rises",
    "rising", "high", "highs", "record", "boost", "boosts", "strong", "stronger", "strength",
    "growth", "optimism", "dovish", "stimulus", "recovery", "recover", "profit", "profits", "beat",
    "beats", "upgrade", "upgrades", "easing", "inflow", "inflows", "rebound", "rebounds", "soar",
    "soars", "bull", "bullish", "higher", "climb", "climbs", "upside", "cheer", "support",
    "outperform", "outperforms", "expands", "expansion", "resilient", "sanguine",
];

const BEAR_WORDS: &[&str] = &[
    "fall", "falls", "drop", "drops", "plunge", "plunges", "slump", "slumps", "crash", "crashes",
    "decline", "declines", "low", "lows", "loss", "losses", "weak", "weaker", "weakness",
    "recession", "hike", "hikes", "hawkish", "fear", "fears", "war", "sanction", "sanctions",
    "crisis", "default", "scam", "fraud", "selloff", "outflow", "outflows", "downgrade",
    "downgrades", "tariff", "tariffs", "slowdown", "unemployment", "panic", "warning", "warns",
    "bear", "bearish", "lower", "slide", "slides", "tumble", "tumbles", "pressure", "pressures",
    "risk", "risks", "conflict", "tension", "tensions", "sell", "sells", "selling", "fears",
];

/// Decode the XML entities that appear in RSS titles (numeric + the common five).
fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'&' {
            if let Some(rel) = s[i..].find(';') {
                let semi = i + rel;
                let ent = &s[i + 1..semi];
                let named = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => None,
                };
                let decoded = named.or_else(|| {
                    let num = ent.strip_prefix('#')?;
                    let cp = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    cp.and_then(char::from_u32)
                });
                if let Some(c) = decoded {
                    out.push(c);
                    i = semi + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn strip_cdata(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("<![CDATA[") {
        if let Some(end) = rest.strip_suffix("]]>") {
            return end.trim();
        }
    }
    t
}

/// Content of the first `<tag ...>...</tag>` in `block` (CDATA-aware).
fn tag_content(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = block.find(&open)?;
    let after = &block[start..];
    let gt = after.find('>')?;
    if after[..gt].ends_with('/') {
        return None;
    }
    let content = &after[gt + 1..];
    let close = format!("</{tag}>");
    let end = content.find(&close)?;
    Some(xml_unescape(strip_cdata(&content[..end])).trim().to_string())
}

fn month_num(m: &str) -> i64 {
    match m.get(..3).unwrap_or("").to_ascii_lowercase().as_str() {
        "jan" => 1, "feb" => 2, "mar" => 3, "apr" => 4, "may" => 5, "jun" => 6,
        "jul" => 7, "aug" => 8, "sep" => 9, "oct" => 10, "nov" => 11, "dec" => 12, _ => 0,
    }
}

fn parse_tz(z: &str) -> i64 {
    let (sign, body) = if let Some(b) = z.strip_prefix('+') {
        (1, b)
    } else if let Some(b) = z.strip_prefix('-') {
        (-1, b)
    } else {
        (0, z)
    };
    if sign != 0 && body.len() >= 4 && body.chars().all(|c| c.is_ascii_digit()) {
        let h: i64 = body[..2].parse().unwrap_or(0);
        let m: i64 = body[2..4].parse().unwrap_or(0);
        return sign * (h * 3600 + m * 60);
    }
    match z {
        "EST" => -5 * 3600, "EDT" => -4 * 3600,
        "CST" => -6 * 3600, "CDT" => -5 * 3600,
        "MST" => -7 * 3600, "MDT" => -6 * 3600,
        "PST" => -8 * 3600, "PDT" => -7 * 3600,
        "IST" => 5 * 3600 + 1800,
        _ => 0,
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// RFC-822 date (RSS pubDate) to epoch seconds; 0 when it cannot be parsed.
fn parse_rfc822(s: &str) -> i64 {
    let clean = s.replace(',', " ");
    let toks: Vec<&str> = clean.split_whitespace().collect();
    let mut i = 0;
    if toks
        .first()
        .and_then(|t| t.chars().next())
        .map(|c| !c.is_ascii_digit())
        .unwrap_or(false)
    {
        i = 1;
    }
    if toks.len() < i + 4 {
        return 0;
    }
    let day: i64 = toks[i].parse().unwrap_or(0);
    let mon = month_num(toks[i + 1]);
    let year: i64 = toks[i + 2].parse().unwrap_or(0);
    let tp: Vec<i64> = toks[i + 3].split(':').map(|p| p.parse().unwrap_or(0)).collect();
    let hh = tp.first().copied().unwrap_or(0);
    let mm = tp.get(1).copied().unwrap_or(0);
    let ss = tp.get(2).copied().unwrap_or(0);
    let tz = toks.get(i + 4).map(|z| parse_tz(z)).unwrap_or(0);
    if day == 0 || mon == 0 || year == 0 {
        return 0;
    }
    days_from_civil(year, mon, day) * 86400 + hh * 3600 + mm * 60 + ss - tz
}

fn title_sentiment(title: &str) -> &'static str {
    let lower = title.to_lowercase();
    let words: std::collections::HashSet<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let bull = words.iter().filter(|w| BULL_WORDS.contains(w)).count();
    let bear = words.iter().filter(|w| BEAR_WORDS.contains(w)).count();
    if bull > bear {
        "bull"
    } else if bear > bull {
        "bear"
    } else {
        "neutral"
    }
}

/// Google News titles read "Headline - Source"; a `<source>` tag carries it too.
fn split_title_source(title: &str, source_tag: Option<String>) -> (String, String) {
    if let Some(src) = source_tag {
        if !src.is_empty() {
            let suffix = format!(" - {src}");
            if let Some(h) = title.strip_suffix(&suffix) {
                return (h.trim().to_string(), src);
            }
            return (title.to_string(), src);
        }
    }
    if let Some(idx) = title.rfind(" - ") {
        let (h, s) = title.split_at(idx);
        let src = s.trim_start_matches(" - ").trim();
        if !src.is_empty() && src.len() < 60 {
            return (h.trim().to_string(), src.to_string());
        }
    }
    (title.to_string(), String::new())
}

fn parse_rss(xml: &str, region: &str, category: &str, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut rest = xml;
    while out.len() < limit {
        let Some(s) = rest.find("<item") else { break };
        let after = &rest[s..];
        let Some(gt) = after.find('>') else { break };
        let block = &after[gt + 1..];
        let Some(end) = block.find("</item>") else { break };
        let item = &block[..end];
        rest = &block[end + 7..];

        let title = tag_content(item, "title").unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let link = tag_content(item, "link").unwrap_or_default();
        let source_tag = tag_content(item, "source");
        let (headline, source) = split_title_source(&title, source_tag);
        let ts = tag_content(item, "pubDate")
            .map(|d| parse_rfc822(&d))
            .unwrap_or(0);
        let sentiment = title_sentiment(&headline);
        out.push(json!({
            "title": headline,
            "source": source,
            "link": link,
            "ts": ts,
            "region": region,
            "category": category,
            "sentiment": sentiment,
        }));
    }
    out
}

async fn fetch_news(cl: reqwest::Client) -> Value {
    let futs = NEWS_FEEDS.iter().map(|(region, category, url)| {
        let c = cl.clone();
        async move {
            let resp = c
                .get(*url)
                .header("Accept", "application/rss+xml, application/xml, text/xml, */*")
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => r
                    .text()
                    .await
                    .ok()
                    .map(|t| parse_rss(&t, region, category, 8))
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        }
    });
    let results = join_all(futs).await;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut items: Vec<Value> = Vec::new();
    for batch in results {
        for it in batch {
            let key = it.get("title").and_then(|t| t.as_str()).unwrap_or("").to_lowercase();
            if key.is_empty() || !seen.insert(key) {
                continue;
            }
            items.push(it);
        }
    }
    items.sort_by(|a, b| {
        b.get("ts")
            .and_then(|t| t.as_i64())
            .unwrap_or(0)
            .cmp(&a.get("ts").and_then(|t| t.as_i64()).unwrap_or(0))
    });
    let ok = !items.is_empty();
    let truncated: Vec<Value> = items.into_iter().take(60).collect();
    json!({ "ok": ok, "updated": market::now_secs(), "items": truncated })
}

fn event_tags(items: &[Value]) -> Vec<String> {
    const EVENTS: &[(&str, &str)] = &[
        ("fed", "US Fed"), ("fomc", "US Fed"), ("cpi", "US CPI"), ("inflation", "Inflation"),
        ("nonfarm", "US jobs (NFP)"), ("payroll", "US jobs (NFP)"), ("rbi", "RBI policy"),
        ("budget", "Union Budget"), ("gst", "GST"), ("election", "Election"), ("gdp", "GDP"),
        ("opec", "OPEC / crude supply"), ("war", "Geopolitics"), ("sanction", "Sanctions"),
        ("tariff", "Tariffs"),
    ];
    let mut found: Vec<String> = Vec::new();
    for it in items {
        let t = it.get("title").and_then(|x| x.as_str()).unwrap_or("").to_lowercase();
        for (kw, tag) in EVENTS {
            if t.contains(kw) && !found.iter().any(|f| f == tag) {
                found.push((*tag).to_string());
            }
        }
    }
    found.truncate(6);
    found
}

/// A SEPARATE, headline-only bias (kept apart from the price/flow prediction in
/// `predict`, because news is noisy and largely already priced in).
fn news_prediction(section: &Value) -> Value {
    let Some(items) = section.get("items").and_then(|i| i.as_array()) else {
        return json!({ "ok": false, "error": "no news" });
    };
    let (mut bull, mut bear, mut neutral) = (0i64, 0i64, 0i64);
    let mut bull_drivers: Vec<String> = Vec::new();
    let mut bear_drivers: Vec<String> = Vec::new();
    for it in items {
        let sent = it.get("sentiment").and_then(|s| s.as_str()).unwrap_or("neutral");
        let title = it.get("title").and_then(|t| t.as_str()).unwrap_or("");
        match sent {
            "bull" => {
                bull += 1;
                if bull_drivers.len() < 3 {
                    bull_drivers.push(title.to_string());
                }
            }
            "bear" => {
                bear += 1;
                if bear_drivers.len() < 3 {
                    bear_drivers.push(title.to_string());
                }
            }
            _ => neutral += 1,
        }
    }
    let total = bull + bear + neutral;
    if total == 0 {
        return json!({ "ok": false, "error": "no news" });
    }
    let index = if bull + bear > 0 {
        (bull - bear) as f64 / (bull + bear) as f64
    } else {
        0.0
    };
    let direction = if index > 0.15 {
        "bull"
    } else if index < -0.15 {
        "bear"
    } else {
        "neutral"
    };
    let confidence = clamp(50.0 + index.abs() * 50.0, 50.0, 90.0).round() as i64;
    let label = match direction {
        "bull" => "NEWS LEANS BULLISH",
        "bear" => "NEWS LEANS BEARISH",
        _ => "NEWS NEUTRAL / MIXED",
    };
    let events = event_tags(items);
    json!({
        "ok": true,
        "label": label,
        "direction": direction,
        "index": (index * 100.0).round() / 100.0,
        "confidence": confidence,
        "counts": { "bull": bull, "bear": bear, "neutral": neutral, "total": total },
        "eventRisk": if events.is_empty() { "Low" } else { "High" },
        "events": events,
        "bullDrivers": bull_drivers,
        "bearDrivers": bear_drivers,
        "disclaimer": "Headline sentiment is keyword-based and indicative only. News is noisy and \
often already priced in - treat as context, not advice.",
    })
}

/// A single combined read: the price/flow trend prediction is the primary
/// driver; headline news sentiment enters with a small, capped weight (news is
/// noisy and largely already priced in). Kept separate from both inputs so the
/// operator can see exactly how the two views reconcile.
fn overall_prediction(pred: &Value, news_pred: &Value, news: &Value, global: &Value) -> Value {
    let trend = pred.get("trend");
    let trend_dir = trend
        .and_then(|t| t.get("direction"))
        .and_then(|d| d.as_str())
        .unwrap_or("neutral");
    let trend_score = trend
        .and_then(|t| t.get("score"))
        .and_then(|s| s.as_f64())
        .unwrap_or(0.0);
    let trend_label = trend
        .and_then(|t| t.get("label"))
        .and_then(|l| l.as_str())
        .unwrap_or("NEUTRAL");

    let news_ok = news_pred.get("ok").and_then(|b| b.as_bool()).unwrap_or(false)
        && news.get("ok").and_then(|b| b.as_bool()).unwrap_or(false);
    let news_index = if news_ok {
        news_pred.get("index").and_then(|i| i.as_f64()).unwrap_or(0.0)
    } else {
        0.0
    };
    let news_dir = news_pred
        .get("direction")
        .and_then(|d| d.as_str())
        .unwrap_or("neutral");
    let news_label = news_pred
        .get("label")
        .and_then(|l| l.as_str())
        .unwrap_or("n/a");

    // News carries a minor, capped weight so a noisy headline can tilt but never
    // dominate the price/flow read.
    let news_pts = if news_ok {
        clamp(news_index * 0.8, -0.8, 0.8)
    } else {
        0.0
    };
    let overall_score = trend_score + news_pts;

    let dir = if overall_score >= 1.5 {
        "bull"
    } else if overall_score <= -1.5 {
        "bear"
    } else {
        "neutral"
    };
    let label = match dir {
        "bull" => "OVERALL BULLISH",
        "bear" => "OVERALL BEARISH",
        _ => "OVERALL NEUTRAL",
    };

    let price_dir = match trend_dir {
        "bull" => 1,
        "bear" => -1,
        _ => 0,
    };
    let nd = match news_dir {
        "bull" => 1,
        "bear" => -1,
        _ => 0,
    };
    let agreement = if price_dir != 0 && nd != 0 {
        if price_dir == nd {
            "aligned"
        } else {
            "conflict"
        }
    } else {
        "partial"
    };

    let mut conf = clamp(50.0 + overall_score.abs() * 7.0, 50.0, 92.0);
    if agreement == "conflict" {
        conf -= 6.0;
    } else if agreement == "aligned" {
        conf += 4.0;
    }
    let conf = clamp(conf, 40.0, 92.0).round() as i64;

    let mut components: Vec<Value> = vec![json!({
        "name": "Price / flow trend",
        "value": trend_label,
        "points": (trend_score * 100.0).round() / 100.0,
        "weight": "primary",
    })];
    if news_ok {
        components.push(json!({
            "name": "News sentiment",
            "value": news_label,
            "points": (news_pts * 100.0).round() / 100.0,
            "weight": "minor",
        }));
    }
    // Crude is already scored inside the price/flow trend (import-cost factor);
    // surface it here as its own line so the combined view shows it, without
    // double-counting it into the score.
    let crude = row(global, "Crude Oil (WTI)")
        .map(|r| fnum(r, "changePct"))
        .unwrap_or(0.0);
    if crude != 0.0 {
        components.push(json!({
            "name": "Crude oil (WTI)",
            "value": format!("{crude:+.2}%"),
            "points": (clamp(-crude * 0.4, -0.5, 0.5) * 100.0).round() / 100.0,
            "weight": "counted in trend",
        }));
    }
    let brent = row(global, "Brent Crude")
        .map(|r| fnum(r, "changePct"))
        .unwrap_or(0.0);
    if brent != 0.0 {
        components.push(json!({
            "name": "Brent crude",
            "value": format!("{brent:+.2}%"),
            "points": 0.0,
            "weight": "reference",
        }));
    }

    let note = match agreement {
        "aligned" => "Price/flow and news agree - higher-conviction read.",
        "conflict" => "Price/flow and news disagree - size small, expect choppiness.",
        _ => "Only one side has a clear lean - modest conviction.",
    };

    json!({
        "ok": true,
        "direction": dir,
        "label": label,
        "score": (overall_score * 100.0).round() / 100.0,
        "confidence": conf,
        "agreement": agreement,
        "components": components,
        "note": note,
        "disclaimer": "Combines the price/flow trend prediction (primary) with headline news \
sentiment (minor, capped). Indicative bias only - not investment advice.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn india(gift: Option<f64>, vix: f64, nifty_pct: f64) -> Value {
        let mut rows = vec![
            json!({"name":"NIFTY 50","ok":true,"changePct":nifty_pct,"price":22500.0}),
            json!({"name":"INDIA VIX","ok":true,"price":vix}),
        ];
        if let Some(g) = gift {
            rows.insert(0, json!({"name":"GIFT NIFTY","ok":true,"changePct":g,"price":22400.0}));
        }
        json!({ "ok": true, "rows": rows })
    }

    fn global(have: bool, sp: f64, dxy: f64) -> Value {
        if !have {
            return json!({ "ok": false, "rows": [] });
        }
        json!({ "ok": true, "rows": [
            {"name":"S&P 500","ok":true,"changePct":sp},
            {"name":"Dow Jones","ok":true,"changePct":sp},
            {"name":"Nasdaq","ok":true,"changePct":sp},
            {"name":"US Dollar Index","ok":true,"changePct":dxy},
        ]})
    }

    fn participant(fii_fut_long: f64, fii_fut_short: f64, call_long: f64, put_long: f64) -> Value {
        json!({
            "ok": true,
            "columns": ["Client Type","Future Index Long","Future Index Short",
                        "Option Index Call Long","Option Index Put Long"],
            "rows": [{"name":"FII","values":[fii_fut_long, fii_fut_short, call_long, put_long]}],
        })
    }

    #[test]
    fn prediction_reads_gap_up_and_bullish_when_data_aligns() {
        let fiidii = json!({"ok":true,"fii":{"net":2000.0},"dii":{"net":500.0}});
        let p = predict(
            &fiidii,
            &participant(900.0, 100.0, 800.0, 400.0),
            &global(true, 1.0, -0.3),
            &india(Some(0.6), 16.0, 0.3),
        );
        assert_eq!(p["gap"]["direction"], "up");
        assert_eq!(p["trend"]["direction"], "bull");
        assert!(p["gap"]["confidence"].as_i64().unwrap() >= 50);
    }

    #[test]
    fn prediction_reads_gap_down_and_bearish_when_data_aligns() {
        let fiidii = json!({"ok":true,"fii":{"net":-3000.0},"dii":{"net":-800.0}});
        let p = predict(
            &fiidii,
            &participant(100.0, 900.0, 300.0, 1200.0),
            &global(true, -1.2, 0.7),
            &india(Some(-0.8), 22.0, -0.5),
        );
        assert_eq!(p["gap"]["direction"], "down");
        assert_eq!(p["trend"]["direction"], "bear");
    }

    #[test]
    fn prediction_labels_tentative_and_caps_confidence_without_gift() {
        let fiidii = json!({"ok":true,"fii":{"net":1500.0},"dii":{"net":0.0}});
        let p = predict(
            &fiidii,
            &participant(800.0, 200.0, 700.0, 400.0),
            &global(true, 1.0, -0.2),
            &india(None, 0.0, 0.0),
        );
        let label = p["gap"]["label"].as_str().unwrap();
        assert!(label.contains("tentative"), "label was {label}");
        assert!(p["gap"]["confidence"].as_i64().unwrap() <= 45);
    }

    fn participant_fii_pro(
        fii_l: f64,
        fii_s: f64,
        fii_cl: f64,
        fii_pl: f64,
        pro_l: f64,
        pro_s: f64,
        pro_cl: f64,
        pro_pl: f64,
    ) -> Value {
        json!({
            "ok": true,
            "columns": ["Client Type","Future Index Long","Future Index Short",
                        "Option Index Call Long","Option Index Put Long"],
            "rows": [
                {"name":"FII","values":[fii_l, fii_s, fii_cl, fii_pl]},
                {"name":"Pro","values":[pro_l, pro_s, pro_cl, pro_pl]},
            ],
        })
    }

    #[test]
    fn prediction_scores_pro_positioning_when_present() {
        let fiidii = json!({"ok":true,"fii":{"net":0.0},"dii":{"net":0.0}});
        let p = predict(
            &fiidii,
            &participant_fii_pro(100.0, 100.0, 100.0, 100.0, 900.0, 100.0, 800.0, 200.0),
            &global(true, 0.0, 0.0),
            &india(Some(0.0), 15.0, 0.0),
        );
        let names: Vec<String> = p["factors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n == "Pro index-futures net"),
            "factors: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "Pro index-options bias"),
            "factors: {names:?}"
        );
        // Net-long Pro futures + call skew push the trend bullish.
        assert_eq!(p["trend"]["direction"], "bull");
    }

    #[test]
    fn rfc822_dates_parse_to_epoch() {
        // Mon, 06 Jan 2025 10:30:00 +0530
        let a = parse_rfc822("Mon, 06 Jan 2025 10:30:00 +0530");
        // Same instant expressed in GMT.
        let b = parse_rfc822("Mon, 06 Jan 2025 05:00:00 GMT");
        assert_eq!(a, b);
        assert!(a > 0);
        // Unparseable dates degrade to 0, never panic.
        assert_eq!(parse_rfc822(""), 0);
        assert_eq!(parse_rfc822("not a date"), 0);
    }

    #[test]
    fn title_sentiment_flags_direction() {
        assert_eq!(title_sentiment("Stocks surge to record high on strong GDP growth"), "bull");
        assert_eq!(title_sentiment("Markets plunge as war fears trigger selloff"), "bear");
        assert_eq!(title_sentiment("Company board meeting scheduled"), "neutral");
    }

    #[test]
    fn split_title_source_uses_source_tag_then_dash() {
        let (h, s) = split_title_source("Nifty ends higher - Reuters", Some("Reuters".into()));
        assert_eq!(h, "Nifty ends higher");
        assert_eq!(s, "Reuters");
        let (h2, s2) = split_title_source("Nifty ends higher - Economic Times", None);
        assert_eq!(h2, "Nifty ends higher");
        assert_eq!(s2, "Economic Times");
        let (h3, s3) = split_title_source("No source here", None);
        assert_eq!(h3, "No source here");
        assert_eq!(s3, "");
    }

    #[test]
    fn parse_rss_reads_items_and_cdata() {
        let xml = r#"<?xml version="1.0"?><rss><channel>
            <item><title><![CDATA[Nifty surges on Fed rate cut hopes - ET]]></title>
              <link>https://x/1</link><pubDate>Mon, 06 Jan 2025 10:30:00 +0530</pubDate></item>
            <item><title>Rupee falls to record low</title><link>https://x/2</link>
              <pubDate>Mon, 06 Jan 2025 09:00:00 +0530</pubDate></item>
        </channel></rss>"#;
        let items = parse_rss(xml, "India", "Markets", 8);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["title"], "Nifty surges on Fed rate cut hopes");
        assert_eq!(items[0]["source"], "ET");
        assert_eq!(items[0]["sentiment"], "bull");
        assert_eq!(items[1]["sentiment"], "bear");
        assert!(items[0]["ts"].as_i64().unwrap() > 0);
    }

    #[test]
    fn news_prediction_is_separate_and_capped() {
        let section = json!({ "ok": true, "items": [
            {"title":"A","sentiment":"bull"},
            {"title":"B","sentiment":"bull"},
            {"title":"C","sentiment":"bear"},
        ]});
        let np = news_prediction(&section);
        assert_eq!(np["direction"], "bull");
        assert!(np["confidence"].as_i64().unwrap() <= 90);
        // Empty feed degrades gracefully.
        assert_eq!(news_prediction(&json!({"ok":false})), json!({"ok":false,"error":"no news"}));
    }

    #[test]
    fn overall_prediction_combines_trend_news_and_crude() {
        let pred = json!({"ok":true,"trend":{"direction":"bull","label":"BULLISH","score":2.0,"confidence":64}});
        let news = json!({"ok":true,"items":[]});
        let global = json!({"ok":true,"rows":[
            {"name":"Crude Oil (WTI)","ok":true,"changePct":2.0},
            {"name":"Brent Crude","ok":true,"changePct":1.5},
        ]});

        let aligned = overall_prediction(
            &pred,
            &json!({"ok":true,"direction":"bull","label":"NEWS LEANS BULLISH","index":0.3,"confidence":65}),
            &news,
            &global,
        );
        assert_eq!(aligned["direction"], "bull");
        assert_eq!(aligned["agreement"], "aligned");
        let names: Vec<&str> = aligned["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(names.iter().any(|n| n.contains("Crude oil")), "components: {names:?}");

        let conflict = overall_prediction(
            &pred,
            &json!({"ok":true,"direction":"bear","label":"NEWS LEANS BEARISH","index":-0.3,"confidence":60}),
            &news,
            &global,
        );
        assert_eq!(conflict["agreement"], "conflict");
        assert!(
            conflict["confidence"].as_i64().unwrap() < aligned["confidence"].as_i64().unwrap(),
            "conflict should lower confidence"
        );
    }

    #[test]
    fn gap_stats_classifies_holds_fills_and_trend_accuracy() {
        let base = days_from_civil(2026, 10, 1);
        let candle = |d: i64, o: f64, c: f64| Candle {
            time: (base + d) * 86400,
            open: o,
            high: o.max(c) + 1.0,
            low: o.min(c) - 1.0,
            close: c,
            volume: 0.0,
        };
        let candles = vec![
            candle(0, 100.0, 100.0),
            candle(1, 102.0, 103.0),  // gap up, held
            candle(2, 104.0, 103.5),  // gap up, filled
            candle(3, 102.0, 101.0),  // gap down, held
            candle(4, 100.0, 101.5),  // gap down, filled
        ];
        let out = gap_stats_from(&candles, base + 1, base + 4, "day");
        assert_eq!(out["sessions"], 4);
        assert_eq!(out["gapUps"], 2);
        assert_eq!(out["gapUpSuccess"], 1);
        assert_eq!(out["gapUpFail"], 1);
        assert_eq!(out["gapUpRate"], 50.0);
        assert_eq!(out["gapDowns"], 2);
        assert_eq!(out["gapDownSuccess"], 1);
        assert_eq!(out["gapDownRate"], 50.0);
        // Day 4 has no next session, so only 3 trend predictions are scored.
        assert_eq!(out["trendPredictions"], 3);
        assert_eq!(out["trendCorrect"], 2);
        assert_eq!(out["trendAccuracy"], 66.7);
        // Month grouping collapses the four sessions into one bucket.
        let month = gap_stats_from(&candles, base + 1, base + 4, "month");
        let buckets = month["buckets"].as_array().unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0]["label"], "2026-10");
        assert_eq!(buckets[0]["gapUps"], 2);
    }

    #[test]
    fn gap_stats_range_only_counts_sessions_inside_the_window() {
        let base = days_from_civil(2026, 10, 1);
        let candle = |d: i64, o: f64, c: f64| Candle {
            time: (base + d) * 86400,
            open: o,
            high: o.max(c) + 1.0,
            low: o.min(c) - 1.0,
            close: c,
            volume: 0.0,
        };
        let candles = vec![
            candle(0, 100.0, 100.0),
            candle(1, 101.0, 102.0),
            candle(2, 103.0, 101.0),
        ];
        let out = gap_stats_from(&candles, base + 2, base + 2, "day");
        assert_eq!(out["sessions"], 1, "only the in-range session is counted");
        assert_eq!(out["gapUps"], 1);
        // Day 2 still compares against day 1's close, which is outside the range.
        assert_eq!(out["rows"][0]["prevClose"], 102.0);
        assert_eq!(out["rows"][0]["direction"], "up");
    }

    #[test]
    fn gap_prediction_reads_overnight_crude_move() {
        let fiidii = json!({"ok":false});
        let part = json!({"ok":false});
        let mut g = global(true, 0.0, 0.0);
        g["rows"].as_array_mut().unwrap().push(
            json!({"name":"Crude Oil (WTI)","ok":true,"changePct":3.0}),
        );
        g["rows"].as_array_mut().unwrap().push(
            json!({"name":"Brent Crude","ok":true,"changePct":3.0}),
        );
        let p = predict(&fiidii, &part, &g, &india(Some(0.0), 15.0, 0.0));
        let names: Vec<&str> = p["factors"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["group"] == "gap")
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("Crude oil")),
            "crude must enter the gap score: {names:?}"
        );
        assert!(
            p["gap"]["score"].as_f64().unwrap() < 0.0,
            "a crude spike must lean gap-down"
        );
    }
}
