//! Option Chain tab, implemented in Rust/WASM to mirror the old Flask app's
//! option-chain screen: expiry picker, Refresh / Auto / All-Expiries controls,
//! instrument + lot info, an OI/IV chart and the 15-column CE/PE chain table
//! with ATM highlighting and "Go to ATM".
//!
//! Data comes from the Rust server routes added in `crates/server/src/optionchain.rs`
//! (`/api/expiries`, `/api/option_chain`, `/api/option_chain_all`). The tab is
//! rendered and drawn entirely from Rust; clicking a row's chart button opens
//! that strike on the candlestick chart through the shared WASM chart engine.

use std::cell::RefCell;

use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{CanvasRenderingContext2d, Element, HtmlCanvasElement, HtmlSelectElement};

use algo_core::oi_trend::OiRecord;

fn window() -> web_sys::Window {
    web_sys::window().expect("no window")
}
fn document() -> web_sys::Document {
    window().document().expect("no document")
}
fn el(id: &str) -> Option<Element> {
    document().get_element_by_id(id)
}
fn set_html(id: &str, html: &str) {
    if let Some(e) = el(id) {
        e.set_inner_html(html);
    }
}
fn set_text(id: &str, txt: &str) {
    if let Some(e) = el(id) {
        e.set_text_content(Some(txt));
    }
}
fn show(id: &str, visible: bool) {
    if let Some(e) = el(id) {
        let list = e.class_list();
        let _ = if visible { list.remove_1("hidden") } else { list.add_1("hidden") };
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
struct OcState {
    // Chart-symbol identity (security id / segment used by the candle route).
    sec_id: i64,
    exch: String,
    inst: String,
    name: String,
    // Option-chain underlying identity.
    oc_id: i64,
    oc_exch: String,
    expiries: Vec<String>,
    expiry: String,
    rows: Vec<Value>,
    spot: f64,
    lot: f64,
    trading_symbol: String,
    all_chains: Vec<Value>,
    loaded: bool,
    all_mode: bool,
    auto_on: bool,
    // True while the option-premium candlestick chart (opened from a chain row)
    // is on screen, so the option-chain fast refresh keeps running for that
    // strike even though the Option Chain tab itself is not active.
    chart_open: bool,
    // Strike of the option-premium chart currently open (0 = none). Lets the OI
    // strip mark the selected strike, matching the old app's `gSelSymbol()`.
    sel_strike: f64,
    busy: bool,
    gen: u64,
    error: String,
    // Live / retry / cache bookkeeping.
    partial: bool,
    partial_polls: u32,
    load_retries: u32,
    status: String,
    last_update: f64,
    // Short client-side cache for the current expiry.
    cache: std::collections::HashMap<String, (f64, Value)>,
    // Per-symbol expiry-ladder cache (old app's `cacheSet("expiries", ...)`,
    // 5-minute TTL) so re-opening a symbol paints the dropdown instantly.
    exp_cache: std::collections::HashMap<String, (f64, Vec<String>)>,
    // Chart geometry for hover/crosshair.
    chart_rows: Vec<Value>,
    chart_geom: Option<(f64, f64, f64, f64, f64, f64)>,
}

thread_local! {
    static OC: RefCell<OcState> = RefCell::new(OcState::default());
}

fn with_oc<F: FnOnce(&mut OcState) -> R, R>(f: F) -> R {
    OC.with(|s| f(&mut s.borrow_mut()))
}
fn read_oc<F: FnOnce(&OcState) -> R, R>(f: F) -> R {
    OC.with(|s| f(&s.borrow()))
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

fn fget(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_i64().map(|i| i as f64))
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0)
}

fn opt_fget(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_i64().map(|i| i as f64))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
}

fn fmt2(v: f64) -> String {
    if v.is_finite() {
        format!("{:.2}", v)
    } else {
        "-".into()
    }
}

fn signed2(v: f64) -> String {
    if v.is_finite() {
        format!("{}{:.2}", if v >= 0.0 { "+" } else { "" }.to_string(), v)
    } else {
        "-".into()
    }
}

fn fmt_compact(v: f64) -> String {
    let a = v.abs();
    if !v.is_finite() {
        return "-".into();
    }
    if a >= 1e7 {
        format!("{:.2}Cr", v / 1e7)
    } else if a >= 1e5 {
        format!("{:.2}L", v / 1e5)
    } else if a >= 1e3 {
        format!("{:.2}K", v / 1e3)
    } else {
        format!("{:.0}", v)
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------

async fn post_json(url: &str, body: &Value) -> Option<Value> {
    post_json_status(url, body).await.and_then(|(_, v)| v)
}

/// POST returning `(http_status, body)` so callers can react to 429/503 and
/// partial-response conditions.
async fn post_json_status(url: &str, body: &Value) -> Option<(u16, Option<Value>)> {
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&body.to_string()));
    if let Ok(headers) = web_sys::Headers::new() {
        let _ = headers.set("Content-Type", "application/json");
        init.set_headers(&headers);
    }
    let promise = window().fetch_with_str_and_init(url, &init);
    let resp = JsFuture::from(promise).await.ok()?;
    let resp: web_sys::Response = resp.dyn_into().ok()?;
    let status = resp.status();
    let text_p = resp.text().ok()?;
    let text = JsFuture::from(text_p).await.ok()?;
    let s = text.as_string();
    let value = s.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Some((status, value))
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Promise-based sleep built on `setTimeout`.
async fn sleep_ms(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _reject| {
        let cb: &js_sys::Function = resolve.unchecked_ref();
        let _ = window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(cb, ms);
    });
    let _ = JsFuture::from(p).await;
}

fn storage_get(key: &str) -> Option<String> {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(key).ok().flatten())
}

fn storage_set(key: &str, val: &str) {
    if let Ok(Some(s)) = window().local_storage() {
        let _ = s.set_item(key, val);
    }
}

/// Exchange segment for an option strike, derived from the underlying segment.
fn oc_option_seg(exch: &str) -> &'static str {
    let u = exch.to_uppercase();
    if u.contains("BSE") {
        "BSE_FNO"
    } else if u.contains("MCX") {
        "MCX_COMM"
    } else {
        "NSE_FNO"
    }
}

fn quote_for<'a>(qm: &'a Value, sid: i64) -> Option<&'a Value> {
    qm.get(sid.to_string())
        .or_else(|| qm.get(format!("IDX_I:{}", sid)))
}

/// Publish the current chain's option strikes to the sidebar quote engine so
/// they ride along with the `/api/quotes` poll and `/ws` stream, then register
/// them for live server-side subscription.
fn publish_oc_securities() {
    let secs = read_oc(|s| {
        let seg = oc_option_seg(&s.oc_exch).to_string();
        let mut v: Vec<(i64, String)> = Vec::new();
        for r in &s.rows {
            for k in ["CE SID", "PE SID"] {
                if let Some(sid) = r.get(k).and_then(|x| x.as_i64()) {
                    if sid != 0 {
                        v.push((sid, seg.clone()));
                    }
                }
            }
        }
        v
    });
    let arr = js_sys::Array::new();
    let mut payload: Vec<Value> = Vec::new();
    for (sid, seg) in &secs {
        let o = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("security_id"), &JsValue::from_f64(*sid as f64));
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("exchange_segment"), &JsValue::from_str(seg));
        arr.push(&o);
        payload.push(serde_json::json!({ "security_id": sid, "exchange_segment": seg }));
    }
    let _ = js_sys::Reflect::set(
        &JsValue::from(window()),
        &JsValue::from_str("__ocSecurities"),
        arr.as_ref(),
    );
    if !payload.is_empty() {
        let body = serde_json::json!({ "securities": payload });
        spawn_local(async move {
            let _ = post_json("/api/oc/subscribe", &body).await;
        });
    }
}

fn req_body(oc_id: i64, exch: &str, name: &str, expiry: Option<&str>) -> Value {
    let mut m = serde_json::json!({
        "security_id": oc_id,
        "exchange_segment": exch,
        "symbol_name": name,
    });
    if let Some(e) = expiry {
        m["expiry"] = Value::String(e.to_string());
    }
    m
}

// ---------------------------------------------------------------------------
// Symbol selection
// ---------------------------------------------------------------------------

struct Symbol {
    sec_id: i64,
    exch: String,
    inst: String,
    name: String,
    oc_id: i64,
    oc_exch: String,
}

fn selected_symbol() -> Symbol {
    let opt = document()
        .query_selector("#symbolSelect option:checked")
        .ok()
        .flatten();
    let get = |o: &Element, a: &str| o.get_attribute(a).unwrap_or_default();
    match opt {
        Some(o) => {
            let sec_id = o
                .get_attribute("value")
                .and_then(|v| v.parse().ok())
                .unwrap_or(13);
            let oc_id = get(&o, "data-oc-id").parse().unwrap_or(sec_id);
            let oc_exch = {
                let e = get(&o, "data-oc-exch");
                if e.is_empty() {
                    get(&o, "data-exch")
                } else {
                    e
                }
            };
            Symbol {
                sec_id,
                exch: get(&o, "data-exch"),
                inst: get(&o, "data-inst"),
                name: {
                    let n = get(&o, "data-symbol-name");
                    if n.is_empty() {
                        o.text_content().unwrap_or_default()
                    } else {
                        n
                    }
                },
                oc_id,
                oc_exch,
            }
        }
        None => Symbol {
            sec_id: 13,
            exch: "IDX_I".into(),
            inst: "INDEX".into(),
            name: "NIFTY 50".into(),
            oc_id: 13,
            oc_exch: "IDX_I".into(),
        },
    }
}

// ---------------------------------------------------------------------------
// Data flow
// ---------------------------------------------------------------------------

/// Expiry ladder with the old app's 15-attempt / 7s retry. A 404 (commodity
/// futures without listed options) is terminal and reported via the bool.
async fn fetch_expiries(oc_id: i64, exch: &str, name: &str) -> (Vec<String>, bool) {
    let body = req_body(oc_id, exch, name, None);
    for attempt in 0..15 {
        if attempt > 0 {
            sleep_ms(7000).await;
        }
        if let Some((code, v)) = post_json_status("/api/expiries", &body).await {
            if code == 404 {
                return (Vec::new(), true);
            }
            if let Some(v) = v {
                if v.get("status").and_then(|x| x.as_str()) == Some("success") {
                    let list: Vec<String> = v
                        .get("data")
                        .and_then(|d| d.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !list.is_empty() {
                        return (list, false);
                    }
                }
            }
        }
    }
    (Vec::new(), false)
}

async fn fetch_all(oc_id: i64, exch: &str, name: &str) -> Option<Value> {
    let body = req_body(oc_id, exch, name, None);
    post_json("/api/option_chain_all", &body).await
}

/// Full (re)load: resolve the selected symbol, refresh the expiry ladder and
/// load the nearest expiry's chain.
pub(crate) fn refresh_full() {
    let sym = selected_symbol();
    let changed = read_oc(|s| {
        s.oc_id != sym.oc_id || s.oc_exch != sym.oc_exch || !s.loaded
    });
    with_oc(|s| {
        s.sec_id = sym.sec_id;
        s.exch = sym.exch.clone();
        s.inst = sym.inst.clone();
        s.name = sym.name.clone();
        s.oc_id = sym.oc_id;
        s.oc_exch = sym.oc_exch.clone();
        s.gen += 1;
        s.busy = true;
        s.error.clear();
        s.partial = false;
        s.partial_polls = 0;
        if changed {
            s.loaded = false;
            s.rows.clear();
            s.expiries.clear();
            s.all_chains.clear();
            s.cache.clear();
        }
    });
    set_loading(true, "Loading option chain...");
    let g = read_oc(|s| s.gen);
    let (oc_id, exch, name) = (sym.oc_id, sym.oc_exch.clone(), sym.name.clone());
    let exp_key = format!("{}|{}", oc_id, exch.to_uppercase());
    // Serve the per-symbol expiry ladder from the 5-minute client cache first so
    // re-opening a symbol paints the dropdown without waiting on the network
    // (old app's `cacheGet("expiries", ocKey)`).
    if let Some((at, list)) = read_oc(|s| s.exp_cache.get(&exp_key).cloned()) {
        if now_ms() - at < 300000.0 && !list.is_empty() {
            with_oc(|s| {
                s.expiries = merge_expiries(&s.expiries, &list);
                if s.expiry.is_empty() || !s.expiries.contains(&s.expiry) {
                    s.expiry = s.expiries.first().cloned().unwrap_or_default();
                }
            });
            render_expiry_options();
        }
    }

    set_html("oc-table-container", "<div class='oc-empty'>Loading option chain...</div>");
    spawn_local(async move {
        // Always revalidate the ladder in the background; merge so a transient
        // empty response never wipes a good list (old app's mergeOcExpiries).
        let (list, no_options) = fetch_expiries(oc_id, &exch, &name).await;
        if read_oc(|s| s.gen) != g {
            return;
        }
        if no_options {
            with_oc(|s| {
                s.busy = false;
                s.loaded = true;
                s.load_retries = 0;
                s.error = format!("No options listed for {}", name);
            });
            set_loading(false, "");
            render_table();
            return;
        }
        with_oc(|s| {
            if !list.is_empty() {
                s.expiries = merge_expiries(&s.expiries, &list);
                s.exp_cache
                    .insert(exp_key.clone(), (now_ms(), s.expiries.clone()));
                // Restore the expiry the user last used for this symbol.
                if s.expiry.is_empty() {
                    if let Some(saved) = storage_get("savedOcExpiry") {
                        if s.expiries.contains(&saved) {
                            s.expiry = saved;
                        }
                    }
                }
                if !s.expiries.contains(&s.expiry) {
                    s.expiry = s.expiries.first().cloned().unwrap_or_default();
                }
            }
        });
        render_expiry_options();
        let expiry = read_oc(|s| s.expiry.clone());
        if !expiry.is_empty() {
            storage_set("savedOcExpiry", &expiry);
        }
        if expiry.is_empty() {
            with_oc(|s| {
                s.busy = false;
                s.loaded = true;
                s.error = format!("No options listed for {}", name);
            });
            set_loading(false, "");
            render_table();
            return;
        }
        let resp = fetch_chain_cached(oc_id, &exch, &name, &expiry).await;
        if read_oc(|s| s.gen) != g {
            return;
        }
        match resp {
            Some((_status, Some(v))) if v.get("status").and_then(|x| x.as_str()) == Some("error") => {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Failed to load option chain")
                    .to_string();
                with_oc(|s| {
                    s.busy = false;
                    s.loaded = true;
                    s.error = msg;
                });
                schedule_load_retry();
            }
            Some((_status, Some(v))) if v.get("status").and_then(|x| x.as_str()) == Some("loading") => {
                // Server is still assembling the chain (old app's
                // `status:"loading"` -> retry with the 1.5s -> 5s backoff).
                with_oc(|s| {
                    s.busy = false;
                    s.error.clear();
                });
                set_status_kind("Loading option chain...", "warn");
                schedule_load_retry();
            }
            Some((status, Some(v))) => {
                apply_chain(&v);
                with_oc(|s| {
                    s.loaded = true;
                    s.busy = false;
                });
                if status == 503 || status == 429 {
                    schedule_partial_retry(g);
                } else {
                    schedule_partial_retry(g);
                }
            }
            _ => {
                with_oc(|s| {
                    s.busy = false;
                    s.loaded = true;
                    s.error = "Failed to load option chain".into();
                });
                schedule_load_retry();
            }
        }
        set_loading(false, "");
        render_all_oc();
        scroll_atm();
    });
}

/// Called when the option-chain tab is shown. (Re)loads only when the symbol
/// changed, nothing is loaded yet, or the last load failed. Otherwise the live
/// columns are already being pushed over the websocket, so just re-render -
/// this avoids blanking a healthy chain to "Loading option chain..." on every
/// tab switch.
pub(crate) fn ensure_oc_loaded() {
    let sym = selected_symbol();
    let (need, all_mode) = read_oc(|s| {
        let need = !s.loaded
            || s.oc_id != sym.oc_id
            || s.oc_exch != sym.oc_exch
            || !s.error.is_empty();
        (need, s.all_mode)
    });
    if need {
        refresh_full();
    } else {
        render_all_oc();
        if !all_mode {
            render_chart(&read_oc(|s| s.rows.clone()));
        }
    }
}

/// Refresh the current expiry only (Auto / ambient refresh).
fn refresh_chain() {
    if read_oc(|s| s.busy || s.expiry.is_empty() || !s.loaded) {
        return;
    }
    let g = with_oc(|s| {
        s.gen += 1;
        s.gen
    });
    let (oc_id, exch, name, expiry) =
        read_oc(|s| (s.oc_id, s.oc_exch.clone(), s.name.clone(), s.expiry.clone()));
    with_oc(|s| s.busy = true);
    let refresh = !read_oc(|s| s.all_mode);
    if refresh {
        set_loading(true, "Refreshing...");
    }
    spawn_local(async move {
        if let Some((status, Some(v))) = fetch_chain_cached(oc_id, &exch, &name, &expiry).await {
            if read_oc(|s| s.gen) != g {
                return;
            }
            if v.get("status").and_then(|x| x.as_str()) == Some("error") {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Failed to load option chain")
                    .to_string();
                with_oc(|s| {
                    s.error = msg;
                    s.loaded = true;
                });
            } else if v.get("status").and_then(|x| x.as_str()) == Some("loading") {
                set_status_kind("Loading option chain...", "warn");
                schedule_load_retry();
            } else {
                apply_chain(&v);
                if status == 503 || status == 429 {
                    schedule_partial_retry(g);
                } else {
                    schedule_partial_retry(g);
                }
            }
        }
        with_oc(|s| s.busy = false);
        if !read_oc(|s| s.all_mode) {
            set_loading(false, "");
            render_table();
        }
    });
}

/// All-Expiries mode.
fn refresh_all() {
    if read_oc(|s| s.busy) {
        return;
    }
    let g = with_oc(|s| {
        s.gen += 1;
        s.gen
    });
    let (oc_id, exch, name) = read_oc(|s| (s.oc_id, s.oc_exch.clone(), s.name.clone()));
    with_oc(|s| s.busy = true);
    set_loading(true, "Loading all expiries...");
    set_html("oc-all-container", "<div class='oc-empty'>Loading all expiries...</div>");
    spawn_local(async move {
        if let Some(resp) = fetch_all(oc_id, &exch, &name).await {
            if read_oc(|s| s.gen) != g {
                return;
            }
            let chains = resp
                .get("chains")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let expiries: Vec<String> = resp
                .get("expiries")
                .and_then(|c| c.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            // Instrument + spot info from the first chain (old app behaviour).
            let first = chains.first();
            let lot = first
                .and_then(|c| c.get("instrument"))
                .map(|i| fget(i, "lot_size"))
                .unwrap_or(0.0);
            let ts = first
                .and_then(|c| c.get("instrument"))
                .and_then(|i| i.get("trading_symbol"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            with_oc(|s| {
                s.all_chains = chains;
                if !expiries.is_empty() {
                    s.expiries = merge_expiries(&s.expiries, &expiries);
                    // The All-Expiries view fetches the authoritative full ladder;
                    // make it the source of truth for the dropdown + cache.
                    let key = format!("{}|{}", oc_id, exch.to_uppercase());
                    s.exp_cache.insert(key, (now_ms(), s.expiries.clone()));
                }
                if lot > 0.0 {
                    s.lot = lot;
                }
                if !ts.is_empty() {
                    s.trading_symbol = ts;
                }
                s.busy = false;
            });
            render_expiry_options();
        } else {
            with_oc(|s| s.busy = false);
        }
        set_loading(false, "");
        render_all_oc();
    });
}

fn merge_expiries(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut v = existing.to_vec();
    for e in incoming {
        if !v.contains(e) {
            v.push(e.clone());
        }
    }
    v.sort();
    v
}

/// Fetch a chain through a short-lived client cache so tab switches and rapid
/// re-renders do not hammer the server.
async fn fetch_chain_cached(
    oc_id: i64,
    exch: &str,
    name: &str,
    expiry: &str,
) -> Option<(u16, Option<Value>)> {
    let key = format!("{}|{}|{}", oc_id, exch, expiry);
    if let Some((at, v)) = read_oc(|s| s.cache.get(&key).cloned()) {
        if now_ms() - at < 3000.0 {
            return Some((200, Some(v)));
        }
    }
    let body = req_body(oc_id, exch, name, Some(expiry));
    let resp = post_json_status("/api/option_chain", &body).await;
    if let Some((code, Some(v))) = &resp {
        let partial = v.get("partial").and_then(|p| p.as_bool()).unwrap_or(false);
        // Never cache a partial chain: the old app keeps polling until the full
        // (greeks/IV/OI) view lands, and a cached partial would mask the upgrade.
        if *code == 200 && v.get("status").and_then(|x| x.as_str()) == Some("success") && !partial {
            with_oc(|s| s.cache.insert(key.clone(), (now_ms(), v.clone())));
        }
    }
    resp
}

/// When the server marks a chain `partial` (still assembling / rate-limited),
/// poll it again until it is complete or we give up (old app: up to 30 polls at
/// 2.5s, widening to 5s).
fn schedule_partial_retry(g: u64) {
    let (partial, polls) = read_oc(|s| (s.partial, s.partial_polls));
    if !partial || polls >= 30 {
        return;
    }
    with_oc(|s| s.partial_polls += 1);
    let delay = (2500 + (polls as i32 / 6) * 500).min(5000);
    spawn_local(async move {
        sleep_ms(delay).await;
        if read_oc(|s| s.gen) != g {
            return;
        }
        if read_oc(|s| s.partial) {
            refresh_chain();
        }
    });
}

/// Retry the initial chain load when the server is briefly unavailable, up to 40
/// attempts with the old app's 1.5s -> 5s widening backoff.
fn schedule_load_retry() {
    let n = with_oc(|s| {
        s.load_retries = s.load_retries.saturating_add(1);
        s.load_retries
    });
    if n > 40 {
        return;
    }
    let delay = (1500 + (n as i32 - 1) * 125).min(5000);
    spawn_local(async move {
        sleep_ms(delay).await;
        refresh_full();
    });
}

fn apply_chain(resp: &Value) {
    let rows = resp
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let spot = fget(resp, "spot_price");
    let lot = resp
        .get("instrument")
        .map(|i| fget(i, "lot_size"))
        .unwrap_or(0.0);
    let ts = resp
        .get("instrument")
        .and_then(|i| i.get("trading_symbol"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let partial = resp
        .get("partial")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let count = rows.len();
    with_oc(|s| {
        s.rows = rows;
        s.spot = spot;
        s.lot = lot;
        s.trading_symbol = ts;
        s.error.clear();
        s.partial = partial;
        s.load_retries = 0;
        if !partial {
            s.partial_polls = 0;
        }
        s.last_update = now_ms();
    });
    publish_oc_securities();
    sync_oc_analytics_from(&read_oc(|s| s.rows.clone()), spot);
    let stamp = fmt_clock();
    with_oc(|s| s.status = format!("Updated {}", stamp));
    let partial_now = partial;
    let cooldown = resp.get("cooldown").and_then(|v| v.as_bool()).unwrap_or(false);
    if cooldown {
        // Old app's 503 "Rate limited" surface: show the message and keep the
        // partial-poll retry running until the 30s cooldown lifts.
        let msg = resp
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Rate limited - option chain temporarily unavailable");
        set_status_kind(&format!("OC: {msg} - retrying..."), "warn");
    } else if !partial_now {
        set_status_kind(&format!("Loaded {} strikes | {}", count, stamp), "ok");
    } else {
        set_status_kind("Updating...", "");
    }
}

fn fmt_clock() -> String {
    // IST (UTC+05:30) wall clock, matching the candle convention everywhere else
    // in the app - not the browser's own timezone.
    let sec = (js_sys::Date::now() / 1000.0) as i64 + 19_800;
    let rem = sec.rem_euclid(86_400);
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let s = rem % 60;
    format!("{:02}:{:02}:{:02}", h, mi, s)
}

fn set_loading(on: bool, msg: &str) {
    if let Some(b) = el("ocRefreshBtn") {
        b.set_text_content(Some(if on { "..." } else { "Refresh" }));
        let cl = b.class_list();
        let _ = if on { cl.add_1("loading") } else { cl.remove_1("loading") };
    }
    if on && !msg.is_empty() {
        set_status_kind(msg, "");
    }
}

/// Centre a row inside its scroll container (old app's `scrollOCToRow`), which
/// is truer than `scroll_into_view` for tall chains.
fn scroll_oc_to_row(row: &Element, container: &Element) {
    let row_top = row.get_bounding_client_rect().top()
        - container.get_bounding_client_rect().top()
        + container.scroll_top() as f64;
    let target = (row_top - container.client_height() as f64 / 2.0
        + row.client_height() as f64 / 2.0)
        .max(0.0);
    container.set_scroll_top(target as i32);
}

/// Scroll the ATM row into view (old app's scrollOCToRow / goToATM).
fn scroll_atm() {
    let all = read_oc(|s| s.all_mode);
    let (container_sel, row_sel) = if all {
        ("#oc-all-container", "#oc-all-container .atm")
    } else {
        ("#oc-table-container", "#oc-table-container .atm")
    };
    if let (Ok(Some(container)), Ok(Some(row))) = (
        document().query_selector(container_sel),
        document().query_selector(row_sel),
    ) {
        scroll_oc_to_row(&row, &container);
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_expiry_options() {
    let (expiries, current) = read_oc(|s| (s.expiries.clone(), s.expiry.clone()));
    let mut html = String::new();
    if expiries.is_empty() {
        html.push_str("<option value=''>No expiries</option>");
    }
    for e in &expiries {
        let sel = if *e == current { " selected" } else { "" };
        html.push_str(&format!("<option value='{0}'{1}>{0}</option>", esc(e), sel));
    }
    set_html("ocExpirySelect", &html);
}

fn window_bounds(rows: &[Value], spot: f64, each: usize) -> (usize, usize) {
    let strikes: Vec<f64> = rows.iter().map(|r| fget(r, "Strike")).collect();
    let atm = {
        let mut best = 0usize;
        let mut bd = f64::INFINITY;
        for (i, s) in strikes.iter().enumerate() {
            let d = (s - spot).abs();
            if d < bd {
                bd = d;
                best = i;
            }
        }
        best
    };
    (atm.saturating_sub(each), (atm + each + 1).min(rows.len()))
}

fn row_html(r: &Value, is_atm: bool, expiry: &str) -> String {
    let strike = fget(r, "Strike");
    let ce_sid = r.get("CE SID").and_then(|v| v.as_i64()).unwrap_or(0);
    let pe_sid = r.get("PE SID").and_then(|v| v.as_i64()).unwrap_or(0);

    let leg = |side: &str, sid_key: &str| -> String {
        let ltp = fget(r, &format!("{side} LTP"));
        let chg = fget(r, &format!("{side} Chg"));
        let chg_pct = fget(r, &format!("{side} Chg%"));
        let iv = opt_fget(r, &format!("{side} IV"));
        let oi = fget(r, &format!("{side} OI"));
        let vol = fget(r, &format!("{side} Volume"));
        let bid = fget(r, &format!("{side} Bid"));
        let ask = fget(r, &format!("{side} Ask"));
        let delta = opt_fget(r, &format!("{side} Delta"));
        let cls = if side == "CE" { "ce" } else { "pe" };
        let dir = if chg >= 0.0 { "up" } else { "down" };
        let ba = if bid > 0.0 && ask > 0.0 {
            format!("{}/{}", fmt2(bid), fmt2(ask))
        } else {
            "-".into()
        };
        let delta_s = delta.map(|d| format!("{:.4}", d)).unwrap_or_else(|| "-".into());
        let iv_s = iv.map(|v| format!("{:.2}%", v)).unwrap_or_else(|| "-".into());
        let sid = r.get(sid_key).and_then(|v| v.as_i64()).unwrap_or(0);
        format!(
            "<td class='{cls} ltp-cell'><span class='oc-chart-btn' data-side='{side_l}' data-sid='{sid}' data-strike='{strike_v}'>&#9654;</span> \
             <span class='{cls}-ltp'>{ltp}</span></td>\
             <td class='{cls} chg {dir}'><span class='chg-val'>{cv}</span><span class='chg-pct'>({cp}%)</span></td>\
             <td class='{cls}-iv'>{iv_s}</td>\
             <td class='{cls}-oi'>{oi}</td>\
             <td class='{cls}-vol'>{vol}</td>\
             <td class='{cls}-ba'>{ba}</td>\
             <td class='{cls}-delta'>{delta_s}</td>",
            cls = cls,
            side_l = side.to_lowercase(),
            sid = sid,
            strike_v = strike,
            ltp = fmt2(ltp),
            dir = dir,
            cv = signed2(chg),
            cp = signed2(chg_pct),
            iv_s = iv_s,
            oi = fmt_compact(oi),
            vol = fmt_compact(vol),
            ba = ba,
            delta_s = delta_s,
        )
    };

    let strike_cls = if is_atm { "strike atm-strike" } else { "strike" };
    format!(
        "<tr class='{row_cls}' data-ce-sid='{ce_sid}' data-pe-sid='{pe_sid}' data-expiry='{expiry}' data-strike='{strike}'>\
         <td class='{strike_cls}'>{strike}</td>{ce}{pe}</tr>",
        row_cls = if is_atm { "atm" } else { "" },
        strike_cls = strike_cls,
        ce_sid = ce_sid,
        pe_sid = pe_sid,
        expiry = esc(expiry),
        strike = strike,
        ce = leg("CE", "CE SID"),
        pe = leg("PE", "PE SID"),
    )
}

const TABLE_HEAD: &str = "<thead><tr>\
<th>Strike</th><th>CE LTP</th><th>CE Chg</th><th>CE IV%</th><th>CE OI</th><th>CE Vol</th><th>CE B/A</th><th>CE Delta</th>\
<th>PE LTP</th><th>PE Chg</th><th>PE IV%</th><th>PE OI</th><th>PE Vol</th><th>PE B/A</th><th>PE Delta</th>\
</tr></thead>";

fn render_table() {
    let (rows, spot, expiry, all_mode, error, loaded) =
        read_oc(|s| (s.rows.clone(), s.spot, s.expiry.clone(), s.all_mode, s.error.clone(), s.loaded));
    if all_mode {
        return;
    }
    if !error.is_empty() {
        set_html(
            "oc-table-container",
            &format!("<div class='oc-empty err'>{}</div>", esc(&error)),
        );
        render_chart(&[]);
        return;
    }
    if rows.is_empty() {
        let msg = if loaded {
            "No options available for this symbol".to_string()
        } else {
            "Loading option chain...".to_string()
        };
        set_html("oc-table-container", &format!("<div class='oc-empty'>{msg}</div>"));
        render_chart(&[]);
        return;
    }
    let (start, end) = window_bounds(&rows, spot, 10);
    let strikes: Vec<f64> = rows.iter().map(|r| fget(r, "Strike")).collect();
    let atm_idx = {
        let mut best = start;
        let mut bd = f64::INFINITY;
        for i in start..end {
            let d = (strikes[i] - spot).abs();
            if d < bd {
                bd = d;
                best = i;
            }
        }
        best
    };
    let mut body = String::from("<tbody>");
    for i in start..end {
        body.push_str(&row_html(&rows[i], i == atm_idx, &expiry));
    }
    body.push_str("</tbody>");
    let table = format!("<table class='oc-table'>{TABLE_HEAD}{body}</table>");
    set_html("oc-table-container", &table);
    render_instrument_info();
    render_chart(&rows[start..end].to_vec());
    update_atm_button(strikes.get(atm_idx).copied().unwrap_or(spot));
    // Old app re-centres the ATM row on every render, not only on refresh.
    scroll_atm();
    // Feed the freshly rendered chain into the OI Trend overlay (no-op unless
    // the toggle is on).
    sync_oi_trend_from(&rows, spot, &expiry);
}

/// Map the tab's chain rows (old column keys) into the normalized `OiRecord`
/// list the OI-trend math expects.
fn oi_records_from_rows(rows: &[Value]) -> Vec<OiRecord> {
    rows.iter()
        .map(|r| OiRecord {
            strike: fget(r, "Strike"),
            ce_oi: fget(r, "CE OI"),
            pe_oi: fget(r, "PE OI"),
            ce_chg: fget(r, "CE Chg OI"),
            pe_chg: fget(r, "PE Chg OI"),
            ce_vol: fget(r, "CE Volume"),
            pe_vol: fget(r, "PE Volume"),
            ce_ltp: fget(r, "CE LTP"),
            pe_ltp: fget(r, "PE LTP"),
            ce_iv: fget(r, "CE IV"),
            pe_iv: fget(r, "PE IV"),
        })
        .collect()
}

/// Days to expiry (IST calendar days) for the level/expected-range math.
fn dte_days(expiry: &str) -> f64 {
    let exp = algo_core::option::parse_ymd(expiry).unwrap_or(0);
    if exp <= 0 {
        return 7.0;
    }
    let ist_days = ((js_sys::Date::now() / 1000.0) + 19800.0) / 86400.0;
    let today = ist_days.floor() as i64;
    ((exp - today).max(1)) as f64
}

fn sync_oi_trend_from(rows: &[Value], spot: f64, expiry: &str) {
    if !crate::oi_trend_is_on() {
        return;
    }
    let recs = oi_records_from_rows(rows);
    if recs.is_empty() {
        return;
    }
    crate::oi_trend_apply(recs, spot, dte_days(expiry));
}

/// Compute live PCR (total put OI / total call OI) and the ATM implied volatility
/// from the loaded chain and push them into the chart's `pcr` / `iv` indicators.
/// Runs on every chain render, independent of the OI-trend toggle.
fn sync_oc_analytics_from(rows: &[Value], spot: f64) {
    let recs = oi_records_from_rows(rows);
    if recs.is_empty() {
        return;
    }
    let call_oi: f64 = recs.iter().map(|r| r.ce_oi.max(0.0)).sum();
    let put_oi: f64 = recs.iter().map(|r| r.pe_oi.max(0.0)).sum();
    let pcr = if call_oi > 0.0 { put_oi / call_oi } else { 0.0 };
    let iv = recs
        .iter()
        .min_by(|a, b| {
            (a.strike - spot)
                .abs()
                .partial_cmp(&(b.strike - spot).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|r| {
            let vals: Vec<f64> = [r.ce_iv, r.pe_iv]
                .into_iter()
                .filter(|v| *v > 0.0)
                .collect();
            if vals.is_empty() {
                0.0
            } else {
                vals.iter().sum::<f64>() / vals.len() as f64
            }
        })
        .unwrap_or(0.0);
    crate::set_oc_analytics(pcr, iv);
}

/// Load the option chain on demand (used when a PCR / IV indicator is added
/// before the Option Chain tab has ever been opened).
pub(crate) fn ensure_loaded() {
    if read_oc(|s| s.loaded) {
        return;
    }
    refresh_full();
}

/// Re-run the OI-trend overlay from the currently loaded chain (called when the
/// toggle is switched on).
pub(crate) fn sync_oi_trend_now() {
    let (rows, spot, expiry) = read_oc(|s| (s.rows.clone(), s.spot, s.expiry.clone()));
    sync_oi_trend_from(&rows, spot, &expiry);
}

fn render_instrument_info() {
    let (name, ts, lot, status, updated) =
        read_oc(|s| (s.name.clone(), s.trading_symbol.clone(), s.lot, s.status.clone(), s.last_update));
    let mut html = format!("<b class='oc-name'>{}</b>", esc(&name));
    if !ts.is_empty() {
        html.push_str(&format!(" &nbsp; Instrument: <span class='oc-ts'>{}</span>", esc(&ts)));
    }
    if lot > 0.0 {
        html.push_str(&format!(" &nbsp; Lot size: <b class='oc-lot'>{:.0}</b>", lot));
    }
    if !status.is_empty() {
        let age = if updated > 0.0 { ((now_ms() - updated) / 1000.0).max(0.0) } else { 0.0 };
        html.push_str(&format!(
            " &nbsp; <span class='oc-upd'>{}</span>",
            esc(&if age > 15.0 { format!("{} ({:.0}s ago)", status, age) } else { status })
        ));
    }
    set_html("ocInstrumentInfo", &html);
}

fn update_atm_button(strike: f64) {
    if let Some(b) = el("goAtmBtn") {
        if strike > 0.0 {
            b.set_text_content(Some(&format!("Go to ATM ({:.0})", strike)));
            b.remove_attribute("disabled").ok();
        } else {
            b.set_text_content(Some("Go to ATM"));
        }
    }
}

fn render_all_oc() {
    let all_mode = read_oc(|s| s.all_mode);
    show("oc-chart-container", !all_mode);
    show("oc-table-container", !all_mode);
    show("oc-all-container", all_mode);
    if all_mode {
        render_all_chains();
    } else {
        render_table();
    }
}

fn render_all_chains() {
    let chains = read_oc(|s| s.all_chains.clone());
    if chains.is_empty() {
        set_html(
            "oc-all-container",
            "<div class='oc-empty'>Loading all expiries...</div>",
        );
        return;
    }
    let mut html = String::new();
    for chain in &chains {
        let expiry = chain.get("expiry").and_then(|v| v.as_str()).unwrap_or("");
        let spot = fget(chain, "spot_price");
        let rows = chain
            .get("records")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        let (start, end) = window_bounds(&rows, spot, 10);
        let strikes: Vec<f64> = rows.iter().map(|r| fget(r, "Strike")).collect();
        let atm_idx = {
            let mut best = start;
            let mut bd = f64::INFINITY;
            for i in start..end {
                let d = (strikes[i] - spot).abs();
                if d < bd {
                    bd = d;
                    best = i;
                }
            }
            best
        };
        let mut body = String::from("<tbody>");
        for i in start..end {
            body.push_str(&row_html(&rows[i], i == atm_idx, expiry));
        }
        body.push_str("</tbody>");
        html.push_str(&format!(
            "<div class='oc-all-block'><div class='oc-all-head'>Expiry: <b>{}</b> &nbsp; Spot: <b>{}</b></div>\
             <table class='oc-table oc-all-tbl'>{}{}</table></div>",
            esc(expiry),
            fmt2(spot),
            TABLE_HEAD,
            body
        ));
    }
    if html.is_empty() {
        html = "<div class='oc-empty'>No options available</div>".into();
    }
    set_html("oc-all-container", &html);
    scroll_atm();
}

// ---------------------------------------------------------------------------
// OI / IV chart (canvas)
// ---------------------------------------------------------------------------

fn render_chart(rows: &[Value]) {
    let Some(e) = el("ocChart") else { return };
    let Ok(canvas) = e.dyn_into::<HtmlCanvasElement>() else {
        return;
    };
    let css_w = canvas.client_width().max(320) as f64;
    let h = 260.0;
    canvas.set_width((css_w) as u32);
    canvas.set_height(h as u32);
    let Ok(Some(ctx)) = canvas.get_context("2d") else {
        return;
    };
    let Ok(ctx) = ctx.dyn_into::<CanvasRenderingContext2d>() else {
        return;
    };
    ctx.set_fill_style_str("#0f0f23");
    ctx.fill_rect(0.0, 0.0, css_w, h);
    ctx.set_font("10px sans-serif");

    if rows.is_empty() {
        ctx.set_fill_style_str("#666");
        ctx.fill_text("No option data", 12.0, 24.0).ok();
        return;
    }
    let strikes: Vec<f64> = rows.iter().map(|r| fget(r, "Strike")).collect();
    let ce_oi: Vec<f64> = rows.iter().map(|r| fget(r, "CE OI")).collect();
    let pe_oi: Vec<f64> = rows.iter().map(|r| fget(r, "PE OI")).collect();
    let ce_iv: Vec<f64> = rows.iter().map(|r| opt_fget(r, "CE IV").unwrap_or(0.0)).collect();
    let pe_iv: Vec<f64> = rows.iter().map(|r| opt_fget(r, "PE IV").unwrap_or(0.0)).collect();

    let left = 6.0;
    let right = css_w - 46.0;
    let top = 12.0;
    let bottom = h - 22.0;
    let plot_w = (right - left).max(10.0);
    let plot_h = (bottom - top).max(10.0);
    let n = rows.len();

    let max_oi = ce_oi
        .iter()
        .chain(pe_oi.iter())
        .fold(0.0f64, |a, b| a.max(*b))
        .max(1.0);
    let max_iv = ce_iv
        .iter()
        .chain(pe_iv.iter())
        .fold(0.0f64, |a, b| a.max(*b))
        .max(1.0);

    let slot = plot_w / n as f64;
    let bar_w = (slot * 0.34).max(1.0);
    let y_oi = |v: f64| bottom - (v / max_oi) * plot_h;
    let y_iv = |v: f64| bottom - (v / (max_iv * 1.15)) * plot_h;

    with_oc(|s| {
        s.chart_rows = rows.to_vec();
        s.chart_geom = Some((left, slot, top, bottom, max_oi, max_iv));
    });

    // legend
    ctx.set_fill_style_str("rgba(0,212,170,0.85)");
    ctx.fill_rect(left + 2.0, top + 2.0, 8.0, 8.0);
    ctx.set_fill_style_str("#00d4aa");
    ctx.fill_text("CE OI", left + 13.0, top + 10.0).ok();
    ctx.set_fill_style_str("rgba(255,82,82,0.85)");
    ctx.fill_rect(left + 58.0, top + 2.0, 8.0, 8.0);
    ctx.set_fill_style_str("#ff5252");
    ctx.fill_text("PE OI", left + 69.0, top + 10.0).ok();
    ctx.set_stroke_style_str("#00d4aa");
    ctx.begin_path();
    ctx.move_to(left + 116.0, top + 6.0);
    ctx.line_to(left + 132.0, top + 6.0);
    ctx.stroke();
    ctx.set_fill_style_str("#00d4aa");
    ctx.fill_text("CE IV", left + 135.0, top + 10.0).ok();
    ctx.set_stroke_style_str("#ff5252");
    ctx.begin_path();
    ctx.move_to(left + 180.0, top + 6.0);
    ctx.line_to(left + 196.0, top + 6.0);
    ctx.stroke();
    ctx.set_fill_style_str("#ff5252");
    ctx.fill_text("PE IV", left + 199.0, top + 10.0).ok();

    // grid
    ctx.set_stroke_style_str("#1e1e40");
    ctx.set_line_width(1.0);
    for g in 0..=4 {
        let y = top + plot_h * g as f64 / 4.0;
        ctx.begin_path();
        ctx.move_to(left, y);
        ctx.line_to(right, y);
        ctx.stroke();
    }

    // OI bars: PE (red) left of centre, CE (green) right of centre.
    for i in 0..n {
        let cx = left + slot * (i as f64 + 0.5);
        ctx.set_fill_style_str("rgba(255,82,82,0.65)");
        let pe_h = bottom - y_oi(pe_oi[i]);
        ctx.fill_rect(cx - bar_w, y_oi(pe_oi[i]), bar_w, pe_h);
        ctx.set_fill_style_str("rgba(0,212,170,0.65)");
        let ce_h = bottom - y_oi(ce_oi[i]);
        ctx.fill_rect(cx, y_oi(ce_oi[i]), bar_w, ce_h);
    }

    // IV lines on the right axis.
    draw_line(&ctx, left, slot, &ce_iv, &y_iv, "#00d4aa");
    draw_line(&ctx, left, slot, &pe_iv, &y_iv, "#ff5252");

    // ATM vertical dashed gold line.
    let spot = read_oc(|s| s.spot);
    let atm = {
        let mut best = 0usize;
        let mut bd = f64::INFINITY;
        for (i, s) in strikes.iter().enumerate() {
            let d = (s - spot).abs();
            if d < bd {
                bd = d;
                best = i;
            }
        }
        best
    };
    let ax = left + slot * (atm as f64 + 0.5);
    ctx.set_stroke_style_str("#ffd700");
    ctx.set_line_width(1.0);
    let dash = JsValue::from(js_sys::Array::of2(&JsValue::from(4), &JsValue::from(4)));
    ctx.set_line_dash(&dash).ok();
    ctx.begin_path();
    ctx.move_to(ax, top);
    ctx.line_to(ax, bottom);
    ctx.stroke();
    let nodash = JsValue::from(js_sys::Array::new());
    ctx.set_line_dash(&nodash).ok();
    ctx.set_fill_style_str("#ffd700");
    ctx.fill_text(&format!("ATM {}", strikes[atm] as i64), (ax + 3.0).min(right - 40.0), top + 9.0).ok();

    // axis labels
    ctx.set_fill_style_str("#888");
    ctx.fill_text("OI", left, top - 2.0).ok();
    ctx.fill_text("IV%", right - 18.0, top - 2.0).ok();
    ctx.fill_text(&fmt_compact(max_oi), left, top + 9.0).ok();
    ctx.fill_text(&format!("{:.0}%", max_iv * 1.15), right + 1.0, top + 9.0).ok();
    // X-axis strike labels, capped at 20 (Chart.js `maxTicksLimit: 20`).
    let tick_step = ((n as f64 / 20.0).ceil() as usize).max(1);
    ctx.set_fill_style_str("#888");
    let mut i = 0usize;
    while i < n {
        let label = format!("{}", strikes[i] as i64);
        let cx = left + slot * (i as f64 + 0.5);
        let w = label.len() as f64 * 5.0;
        let tx = (cx - w / 2.0).clamp(left, right - w);
        ctx.fill_text(&label, tx, bottom + 12.0).ok();
        i += tick_step;
    }
    // Always show the last strike when the step skipped it.
    if n > 0 && (n - 1) % tick_step != 0 {
        let label = format!("{}", strikes[n - 1] as i64);
        let w = label.len() as f64 * 5.0;
        ctx.fill_text(&label, (right - w).max(left), bottom + 12.0).ok();
    }
}

fn draw_line(
    ctx: &CanvasRenderingContext2d,
    left: f64,
    slot: f64,
    vals: &[f64],
    y: &dyn Fn(f64) -> f64,
    color: &str,
) {
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.5);
    ctx.begin_path();
    for (i, v) in vals.iter().enumerate() {
        let x = left + slot * (i as f64 + 0.5);
        let yy = y(*v);
        if i == 0 {
            ctx.move_to(x, yy);
        } else {
            ctx.line_to(x, yy);
        }
    }
    ctx.stroke();
}

/// Hover crosshair + tooltip drawn on top of the OI/IV chart.
fn draw_hover(idx: usize) {
    let rows = read_oc(|s| s.chart_rows.clone());
    if idx >= rows.len() || rows.is_empty() {
        return;
    }
    render_chart(&rows);
    let Some((left, slot, top, bottom, _, _)) = read_oc(|s| s.chart_geom) else {
        return;
    };
    let Some(e) = el("ocChart") else { return };
    let Ok(canvas) = e.dyn_into::<HtmlCanvasElement>() else {
        return;
    };
    let Ok(Some(ctx)) = canvas.get_context("2d") else {
        return;
    };
    let Ok(ctx) = ctx.dyn_into::<CanvasRenderingContext2d>() else {
        return;
    };
    let cx = left + slot * (idx as f64 + 0.5);
    ctx.set_stroke_style_str("rgba(255,255,255,0.30)");
    ctx.set_line_width(1.0);
    ctx.begin_path();
    ctx.move_to(cx, top);
    ctx.line_to(cx, bottom);
    ctx.stroke();

    let r = &rows[idx];
    let strike = fget(r, "Strike");
    let ce = fget(r, "CE LTP");
    let pe = fget(r, "PE LTP");
    let ce_oi = fget(r, "CE OI");
    let pe_oi = fget(r, "PE OI");
    let ce_iv = opt_fget(r, "CE IV").unwrap_or(0.0);
    let pe_iv = opt_fget(r, "PE IV").unwrap_or(0.0);
    let ce_chg = fget(r, "CE Chg");
    let pe_chg = fget(r, "PE Chg");
    let ce_vol = fget(r, "CE Volume");
    let pe_vol = fget(r, "PE Volume");
    let lines = [
        format!("Strike {strike}"),
        format!("CE {ce:.2} ({ce_chg:+.2})   PE {pe:.2} ({pe_chg:+.2})"),
        format!("CE OI {}  PE OI {}", fmt_compact(ce_oi), fmt_compact(pe_oi)),
        format!("CE Vol {}  PE Vol {}", fmt_compact(ce_vol), fmt_compact(pe_vol)),
        format!("CE IV {ce_iv:.1}%  PE IV {pe_iv:.1}%"),
    ];
    let w = 230.0;
    let h = 14.0 * lines.len() as f64 + 8.0;
    let cw = canvas.client_width() as f64;
    let tx = (cx + 8.0).min(cw - w - 4.0).max(left);
    let ty = top + 16.0;
    ctx.set_fill_style_str("rgba(10,10,30,0.92)");
    ctx.fill_rect(tx, ty, w, h);
    ctx.set_stroke_style_str("#33336b");
    ctx.set_line_width(1.0);
    ctx.stroke_rect(tx, ty, w, h);
    ctx.set_fill_style_str("#ddd");
    for (i, l) in lines.iter().enumerate() {
        ctx.fill_text(l, tx + 6.0, ty + 12.0 + i as f64 * 14.0).ok();
    }
}

/// Map a canvas x-coordinate to a strike index using the stored geometry.
fn hover_index(px: f64) -> Option<usize> {
    let (left, slot, _, _, _, _) = read_oc(|s| s.chart_geom)?;
    let n = read_oc(|s| s.chart_rows.len());
    if n == 0 || slot <= 0.0 {
        return None;
    }
    let i = ((px - left) / slot).floor();
    if i < 0.0 {
        return Some(0);
    }
    let i = i as usize;
    Some(i.min(n - 1))
}

// ---------------------------------------------------------------------------
// Interaction
// ---------------------------------------------------------------------------

fn oc_active() -> bool {
    el("tab-optionchain")
        .map(|e| e.class_list().contains("active"))
        .unwrap_or(false)
}

fn switch_tab(name: &str) {
    if let Ok(list) = document().query_selector_all(".tab-btn[data-tab]") {
        for i in 0..list.length() {
            if let Some(n) = list.item(i).and_then(|x| x.dyn_into::<Element>().ok()) {
                let mine = n.get_attribute("data-tab").as_deref() == Some(name);
                let cl = n.class_list();
                let _ = if mine { cl.add_1("active") } else { cl.remove_1("active") };
            }
        }
    }
    if let Ok(list) = document().query_selector_all(".tab-content") {
        for i in 0..list.length() {
            if let Some(n) = list.item(i).and_then(|x| x.dyn_into::<Element>().ok()) {
                let id = n.get_attribute("id").unwrap_or_default();
                let mine = id == format!("tab-{name}");
                let cl = n.class_list();
                let _ = if mine { cl.add_1("active") } else { cl.remove_1("active") };
            }
        }
    }
    if name != "chart" {
        with_oc(|s| s.chart_open = false);
    }
}

/// Called by the shared chart symbol switch (`select_symbol`) so a plain sidebar
/// selection cancels any option-strike fast refresh started from a chain row.
pub fn on_oc_symbol_change() {
    with_oc(|s| {
        s.chart_open = false;
        s.sel_strike = 0.0;
    });
    crate::oc_analytics_reset();
}

/// True while an option-premium chart is the active chart (old app `_chartKind
/// === 'opt'`). Drives the OI strip vs. the level/direction overlay split.
pub(crate) fn chart_open_now() -> bool {
    read_oc(|s| s.chart_open)
}

/// Strike of the option chart on screen (0 when none), for the strip marker.
pub(crate) fn selected_strike_now() -> f64 {
    read_oc(|s| s.sel_strike)
}

fn select_expiry(value: &str) {
    with_oc(|s| {
        s.expiry = value.to_string();
        s.partial = false;
        s.partial_polls = 0;
    });
    storage_set("savedOcExpiry", value);
    crate::oc_analytics_reset();
    let g = with_oc(|s| {
        s.gen += 1;
        s.gen
    });
    let (oc_id, exch, name, expiry) =
        read_oc(|s| (s.oc_id, s.oc_exch.clone(), s.name.clone(), s.expiry.clone()));
    set_loading(true, "Loading option chain...");
    set_html("oc-table-container", "<div class='oc-empty'>Loading option chain...</div>");
    spawn_local(async move {
        if let Some((status, Some(v))) = fetch_chain_cached(oc_id, &exch, &name, &expiry).await {
            if read_oc(|s| s.gen) != g {
                return;
            }
            if v.get("status").and_then(|x| x.as_str()) == Some("error") {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Failed to load option chain")
                    .to_string();
                with_oc(|s| {
                    s.error = msg;
                    s.loaded = true;
                });
            } else {
                apply_chain(&v);
                with_oc(|s| s.loaded = true);
                if status == 503 || status == 429 {
                    schedule_partial_retry(g);
                } else {
                    schedule_partial_retry(g);
                }
            }
        }
        set_loading(false, "");
        render_all_oc();
        scroll_atm();
    });
}

fn toggle_all() {
    let all = with_oc(|s| {
        s.all_mode = !s.all_mode;
        s.all_mode
    });
    if let Some(b) = el("ocAllBtn") {
        b.set_text_content(Some(if all { "All ON" } else { "All Expiries" }));
        let cl = b.class_list();
        let _ = if all { cl.add_1("warn") } else { cl.remove_1("warn") };
    }
    render_all_oc();
    if all {
        refresh_all();
    } else {
        // Old toggleOCAll re-fetches the single chain on exit.
        refresh_chain();
    }
}

fn toggle_auto() {
    let want = !read_oc(|s| s.auto_on);
    if want && read_oc(|s| s.expiry.is_empty() || !s.loaded) {
        set_text("ocStatus", "Select a symbol and expiry first");
        return;
    }
    with_oc(|s| s.auto_on = want);
    if let Some(b) = el("ocAutoBtn") {
        b.set_text_content(Some(if want { "Auto ON" } else { "Auto" }));
        let cl = b.class_list();
        let _ = if want { cl.add_1("warn") } else { cl.remove_1("warn") };
    }
    if want {
        // Live columns are pushed over the websocket, so there is no REST
        // refresh to arm here - just reflect the always-live state.
        set_text("ocStatus", "Live websocket feed on");
    }
}

fn go_to_atm() {
    let all = read_oc(|s| s.all_mode);
    let (container_sel, row_sel) = if all {
        ("#oc-all-container", "#oc-all-container .atm")
    } else {
        ("#oc-table-container", "#oc-table-container .atm")
    };
    let container = document().query_selector(container_sel).ok().flatten();
    let row = document().query_selector(row_sel).ok().flatten();
    match (row, container) {
        (Some(row), Some(container)) => {
            scroll_oc_to_row(&row, &container);
            let cl = row.class_list();
            let _ = cl.add_1("flash-atm");
            let cb = Closure::<dyn FnMut()>::new(move || {
                let _ = row.class_list().remove_1("flash-atm");
            });
            window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    800,
                )
                .ok();
            cb.forget();
        }
        _ => alert("No ATM row found. Load option chain first."),
    }
}

fn is_index_name(name: &str) -> bool {
    let u = name.to_uppercase();
    u.contains("NIFTY") || u.contains("SENSEX") || u.contains("VIX")
}

/// Cycle between two identical animation classes so re-adding restarts the
/// flash without dangling `setTimeout` closures.
fn flash(el: &Element, up: bool) {
    let cl = el.class_list();
    let (a, b) = if up { ("tick-up", "tick-up2") } else { ("tick-down", "tick-down2") };
    if cl.contains(a) {
        let _ = cl.remove_1(a);
        let _ = cl.add_1(b);
    } else {
        let _ = cl.remove_1(b);
        let _ = cl.add_1(a);
    }
}

fn set_cell_text(tr: &Element, sel: &str, txt: &str, flash_up: Option<bool>) {
    if let Ok(Some(el)) = tr.query_selector(sel) {
        if el.text_content().as_deref() != Some(txt) {
            el.set_text_content(Some(txt));
            if let Some(up) = flash_up {
                flash(&el, up);
            }
        }
    }
}

fn update_leg(tr: &Element, side: &str, q: &Value) {
    let ltp = fget(q, "ltp");
    if ltp <= 0.0 {
        return;
    }
    let chg = fget(q, "change");
    let pct = fget(q, "change_pct");
    let side_l = side.to_lowercase();
    set_cell_text(tr, &format!(".{side_l}-ltp"), &fmt2(ltp), Some(chg >= 0.0));
    if let Ok(Some(td)) = tr.query_selector(&format!(".{side_l}.chg")) {
        let cl = td.class_list();
        let _ = cl.remove_1("up");
        let _ = cl.remove_1("down");
        let _ = if chg >= 0.0 { cl.add_1("up") } else { cl.add_1("down") };
        if let Ok(Some(v)) = td.query_selector(".chg-val") {
            let txt = signed2(chg);
            if v.text_content().as_deref() != Some(txt.as_str()) {
                v.set_text_content(Some(&txt));
                flash(&v, chg >= 0.0);
            }
        }
        if let Ok(Some(p)) = td.query_selector(".chg-pct") {
            p.set_text_content(Some(&format!("({}%)", signed2(pct))));
        }
    }
    // Full-mode extras (old app's `ocApplyExtra`): IV / OI / Volume / Bid-Ask /
    // Delta tick live alongside the premium when the feed publishes them.
    if let Some(iv) = q.get("iv").and_then(|v| v.as_f64()) {
        if iv > 0.0 {
            set_cell_text(tr, &format!(".{side_l}-iv"), &format!("{:.2}%", iv), None);
        }
    }
    if let Some(oi) = q.get("oi").and_then(|v| v.as_f64()) {
        if oi > 0.0 {
            set_cell_text(tr, &format!(".{side_l}-oi"), &fmt_compact(oi), None);
        }
    }
    if let Some(vol) = q.get("volume").and_then(|v| v.as_f64()) {
        if vol > 0.0 {
            set_cell_text(tr, &format!(".{side_l}-vol"), &fmt_compact(vol), None);
        }
    }
    let bid = q.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ask = q.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if bid > 0.0 && ask > 0.0 {
        set_cell_text(
            tr,
            &format!(".{side_l}-ba"),
            &format!("{}/{}", fmt2(bid), fmt2(ask)),
            None,
        );
    }
    if let Some(delta) = q.get("delta").and_then(|v| v.as_f64()) {
        set_cell_text(tr, &format!(".{side_l}-delta"), &format!("{:.4}", delta), None);
    }
}

/// Merge a live quote into the cached chain record for one strike/side so the
/// next full render keeps the freshest OI/IV/Volume/Bid-Ask (old app's
/// `updateOCLive` wrote back into `ocRecords`, not only the DOM).
fn apply_extra(rows: &mut [Value], sid: i64, q: &Value) {
    for r in rows.iter_mut() {
        let side = if r.get("CE SID").and_then(|v| v.as_i64()) == Some(sid) {
            "CE"
        } else if r.get("PE SID").and_then(|v| v.as_i64()) == Some(sid) {
            "PE"
        } else {
            continue;
        };
        let Some(map) = r.as_object_mut() else { continue };
        let mut set = |k: String, v: f64| {
            map.insert(k, serde_json::json!(v));
        };
        if let Some(v) = q.get("ltp").and_then(|v| v.as_f64()) {
            if v > 0.0 {
                set(format!("{side} LTP"), v);
            }
        }
        if let Some(v) = q.get("change").and_then(|v| v.as_f64()) {
            set(format!("{side} Chg"), v);
        }
        if let Some(v) = q.get("change_pct").and_then(|v| v.as_f64()) {
            set(format!("{side} Chg%"), v);
        }
        for (key, field) in [
            ("iv", "IV"),
            ("oi", "OI"),
            ("volume", "Volume"),
            ("bid", "Bid"),
            ("ask", "Ask"),
            ("delta", "Delta"),
        ] {
            if let Some(v) = q.get(key).and_then(|v| v.as_f64()) {
                if v > 0.0 || field == "Delta" {
                    set(format!("{side} {field}"), v);
                }
            }
        }
        return;
    }
}

/// Merge a live quote into every cached record set (current expiry + all-expiry
/// chains) so re-renders never regress to stale extras.
fn apply_extra_all(sid: i64, q: &Value) {
    with_oc(|s| {
        apply_extra(&mut s.rows, sid, q);
        for ch in s.all_chains.iter_mut() {
            if let Some(recs) = ch.get_mut("records").and_then(|v| v.as_array_mut()) {
                apply_extra(recs, sid, q);
            }
        }
    });
}

/// Live quote ingest: the sidebar quote engine pushes its merged map here on
/// every `/ws` frame / `/api/quotes` poll so option LTPs tick and flash without
/// a full chain re-render (old app's `updateOCLive`).
#[wasm_bindgen]
pub fn update_oc_quotes(json: &str) {
    if !oc_active() {
        return;
    }
    let qm: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return,
    };
    for container in ["oc-table-container", "oc-all-container"] {
        let sel = format!("#{container} tr[data-ce-sid]");
        let Ok(list) = document().query_selector_all(&sel) else {
            continue;
        };
        for i in 0..list.length() {
            let Some(tr) = list.item(i).and_then(|n| n.dyn_into::<Element>().ok()) else {
                continue;
            };
            if let Some(sid) = tr.get_attribute("data-ce-sid").and_then(|v| v.parse::<i64>().ok()) {
                if let Some(q) = quote_for(&qm, sid) {
                    update_leg(&tr, "CE", q);
                    apply_extra_all(sid, q);
                }
            }
            if let Some(sid) = tr.get_attribute("data-pe-sid").and_then(|v| v.parse::<i64>().ok()) {
                if let Some(q) = quote_for(&qm, sid) {
                    update_leg(&tr, "PE", q);
                    apply_extra_all(sid, q);
                }
            }
        }
    }
}

fn alert(msg: &str) {
    let _ = window().alert_with_message(msg);
}

/// Write a message to both the local option-chain status and the app's global
/// `#status` bar (the old app had a single global status span, so OC messages
/// appeared there too). `kind` is "", "ok" or "warn".
fn set_status_kind(msg: &str, kind: &str) {
    for id in ["ocStatus", "status"] {
        if let Some(e) = el(id) {
            e.set_text_content(Some(msg));
            if !kind.is_empty() {
                let _ = e.set_attribute("class", kind);
            }
        }
    }
}

fn oc_status(msg: &str) {
    set_status_kind(msg, "");
}

fn open_strike(sid: i64, side: &str, expiry: &str, strike: f64) {
    // Old app guards the open on a loaded expiry (its `ocExpirySelect` shows
    // "--"/"Loading..." until the chain lands).
    if expiry.is_empty() || expiry.starts_with("--") || expiry.starts_with("Loading") {
        alert("Please load the option chain first");
        return;
    }
    let (oc_id, oc_exch, name) = read_oc(|s| (s.oc_id, s.oc_exch.clone(), s.name.clone()));
    // Stale-symbol guard: abort if the selection changed since render.
    if selected_symbol().oc_id != oc_id {
        return;
    }
    let side = side.to_string();
    let expiry = expiry.to_string();
    let label = format!("{} {} {}", name, side, expiry);
    let seg = oc_option_seg(&oc_exch).to_string();
    let inst = if seg == "MCX_COMM" {
        "OPTFUT"
    } else if is_index_name(&name) {
        "OPTIDX"
    } else {
        "OPTSTK"
    };
    switch_tab("chart");
    spawn_local(async move {
        let mut use_sid = sid;
        let mut use_inst = inst.to_string();
        let mut use_seg = seg.to_string();
        if use_sid == 0 {
            // Resolve the contract through the server (real Dhan chain lookup,
            // synthetic fallback) — old app's /api/option_security fallback.
            let body = serde_json::json!({
                "security_id": oc_id,
                "symbol_name": name,
                "expiry": expiry,
                "strike": strike,
                "option_type": side,
                "exchange_segment": oc_exch,
            });
            if let Some((_, Some(v))) = post_json_status("/api/option_security", &body).await {
                if let Some(d) = v.get("data") {
                    use_sid = d.get("security_id").and_then(|x| x.as_i64()).unwrap_or(0);
                    if let Some(it) = d.get("instrument_type").and_then(|x| x.as_str()) {
                        use_inst = it.to_string();
                    }
                    if let Some(es) = d.get("exchange_segment").and_then(|x| x.as_str()) {
                        use_seg = es.to_string();
                    }
                }
            }
        }
        if use_sid == 0 {
            oc_status(&format!(
                "Option security not found for {} {}",
                strike, side
            ));
            return;
        }
        // Re-check the selection right before committing the symbol.
        if selected_symbol().oc_id != oc_id {
            return;
        }
        oc_status(&format!("Loading {} option candles...", label));
        // Old app's unified open path: always route through the shared chart
        // loader (`select_symbol` -> `load_chart`), which now owns the MCX
        // intraday -> daily fallback.
        crate::select_symbol(use_sid as f64, &use_seg, &use_inst, &label);
        // Keep the option-chain fast refresh running while this strike chart is
        // open (old app kept its OC interval alive on the chart tab), and record
        // the strike so the OI strip marks it.
        with_oc(|s| {
            s.chart_open = true;
            s.sel_strike = strike;
        });
    });
}

/// Open the option-premium chart directly from a CE/PE security id (old app's
/// `openOptionChartBySid`), without needing the option chain to be loaded.
/// Public WASM export kept for parity with the old Auto-Experiment detail
/// button (the new UI currently has no caller, but JS may invoke it).
#[wasm_bindgen]
pub fn open_option_chart_by_sid(
    sid: f64,
    exchange_segment: &str,
    instrument_type: &str,
    label: &str,
) {
    if sid <= 0.0 {
        return;
    }
    let seg = if exchange_segment.trim().is_empty() {
        "NSE_FNO".to_string()
    } else {
        exchange_segment.to_string()
    };
    let inst = if instrument_type.trim().is_empty() {
        if seg.to_uppercase().contains("MCX") {
            "OPTFUT".to_string()
        } else {
            "OPTIDX".to_string()
        }
    } else {
        instrument_type.to_string()
    };
    let name = if label.trim().is_empty() {
        format!("{} {}", inst, sid as i64)
    } else {
        label.to_string()
    };
    switch_tab("chart");
    set_status_kind(&format!("Loading {} option candles...", name), "");
    crate::select_symbol(sid, &seg, &inst, &name);
    with_oc(|s| s.chart_open = true);
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

pub fn boot_option_chain() {
    // Tab buttons.
    if let Ok(list) = document().query_selector_all(".tab-btn[data-tab]") {
        for i in 0..list.length() {
            let Some(node) = list.item(i).and_then(|x| x.dyn_into::<Element>().ok()) else { continue };
            let name = node.get_attribute("data-tab").unwrap_or_default();
            let is_oc = name == "optionchain";
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                switch_tab(&name);
                if is_oc {
                    ensure_oc_loaded();
                }
            });
            node.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
                .ok();
            cb.forget();
        }
    }

    // Expiry picker.
    if let Some(sel) = el("ocExpirySelect").and_then(|e| e.dyn_into::<HtmlSelectElement>().ok()) {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            let val = el("ocExpirySelect")
                .and_then(|e| e.dyn_into::<HtmlSelectElement>().ok())
                .map(|s| s.value())
                .unwrap_or_default();
            if !val.is_empty() {
                select_expiry(&val);
            }
        });
        sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .ok();
        cb.forget();
    }

    // Refresh / Auto / All / Go to ATM.
    if let Some(b) = el("ocRefreshBtn") {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| refresh_full());
        b.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref()).ok();
        cb.forget();
    }
    if let Some(b) = el("ocAutoBtn") {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| toggle_auto());
        b.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref()).ok();
        cb.forget();
    }
    if let Some(b) = el("ocAllBtn") {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| toggle_all());
        b.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref()).ok();
        cb.forget();
    }
    if let Some(b) = el("goAtmBtn") {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| go_to_atm());
        b.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref()).ok();
        cb.forget();
    }

    // Row chart buttons (event delegation on both containers).
    for container in ["oc-table-container", "oc-all-container"] {
        if let Some(c) = el(container) {
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
                let Some(t) = e.target() else { return };
                let Ok(mut node) = t.dyn_into::<Element>() else { return };
                loop {
                    if node.class_list().contains("oc-chart-btn") {
                        let side = node.get_attribute("data-side").unwrap_or_default();
                        let sid = node
                            .get_attribute("data-sid")
                            .and_then(|v| v.parse::<i64>().ok())
                            .unwrap_or(0);
                        let strike = node
                            .get_attribute("data-strike")
                            .and_then(|v| v.parse::<f64>().ok())
                            .unwrap_or(0.0);
                        let expiry = node
                            .parent_element()
                            .and_then(|td| td.parent_element())
                            .and_then(|tr| tr.get_attribute("data-expiry"))
                            .unwrap_or_default();
                        open_strike(sid, &side.to_uppercase(), &expiry, strike);
                        return;
                    }
                    match node.parent_element() {
                        Some(p) => node = p,
                        None => return,
                    }
                }
            });
            c.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
                .ok();
            cb.forget();
        }
    }

    // Chart hover: crosshair + tooltip; leave clears it.
    if let Some(e) = el("ocChart") {
        let c = e.clone();
        let mm = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
            let rect = c.get_bounding_client_rect();
            let x = ev.client_x() as f64 - rect.left();
            if let Some(i) = hover_index(x) {
                draw_hover(i);
            }
        });
        e.add_event_listener_with_callback("mousemove", mm.as_ref().unchecked_ref())
            .ok();
        mm.forget();
        let c2 = e.clone();
        let ml = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |_ev: web_sys::MouseEvent| {
            let _ = c2;
            let rows = read_oc(|s| s.chart_rows.clone());
            if !rows.is_empty() {
                render_chart(&rows);
            }
        });
        e.add_event_listener_with_callback("mouseleave", ml.as_ref().unchecked_ref())
            .ok();
        ml.forget();
    }

    // Re-render the canvas when the viewport changes while the tab is visible,
    // and whenever the tab is (re)shown, so the chart always fits its width.
    let rz = Closure::<dyn FnMut()>::new(move || {
        if !oc_active() {
            return;
        }
        let rows = read_oc(|s| s.chart_rows.clone());
        if !rows.is_empty() {
            render_chart(&rows);
        }
    });
    window()
        .add_event_listener_with_callback("resize", rz.as_ref().unchecked_ref())
        .ok();
    rz.forget();

    // Live option-chain columns (LTP/Chg/Chg%/OI/Chg OI/Volume/IV/Bid/Ask and
    // all Greeks) are pushed over the websocket feed and merged in place by
    // `update_oc_quotes`, so there is no REST heartbeat polling the chain here.

    // Best-effort: if the option-chain tab is already the active one at boot,
    // load it.
    if oc_active() {
        refresh_full();
    }
}
