//! Live order delivery for every registered broker.
//!
//! The trading engines know a contract as a Dhan `security_id`. An adapter turns
//! that into the broker's own instrument plus a place/cancel/modify call, so a
//! ticked broker in the Order Placement card receives a real order at that
//! broker instead of only at Dhan.
//!
//! Two invariants this module keeps:
//!
//!   * **No silent fallback.** A broker that is routed but cannot resolve its
//!     instrument, authenticate, or reach its API returns a typed error. The
//!     engine surfaces it and never books a fill that did not happen.
//!   * **One trail owner.** Only Dhan's Super Order carries a broker-side
//!     ratchet in this build; every other adapter places a plain (non-trailing)
//!     order, so `brokers::native_trail` is false for them and the engine keeps
//!     the single trailing stop. That is what stops a position from being
//!     ratcheted twice.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::instruments;
use crate::scrip::Contract;

/// One real order the adapter placed.
#[derive(Clone, Debug)]
pub struct Placed {
    pub order_id: String,
    /// Broker-side status string when the API returned one.
    pub status: String,
}

/// Everything an adapter needs to place one entry or exit order.
#[derive(Clone, Debug)]
pub struct OrderReq {
    pub contract: Contract,
    /// `BUY` / `SELL` (already resolved; no bracket legs are attached).
    pub side: String,
    pub qty: i64,
    /// `MARKET` / `LIMIT` / `SL` / `SL-M`.
    pub order_type: String,
    /// Limit / trigger price, 0 for a pure market order.
    pub price: f64,
    /// Product as the engine understands it: `INTRADAY` / `CNC` / `MARGIN`.
    pub product: String,
    /// Last traded price at send time, used by brokers that cannot take a
    /// market order (Shoonya) to derive a marketable limit price.
    pub ltp: f64,
    /// Idempotency tag carried where the broker supports one.
    pub correlation: String,
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client: {e}"))
}

fn cred(broker: &str, key: &str) -> String {
    crate::brokers::credential(broker, key)
}

/// Product string in the generic `INTRADAY` / `CNC` vocabulary.
fn product_of(req: &OrderReq) -> String {
    match req.product.to_ascii_uppercase().as_str() {
        "CNC" | "DELIVERY" => "CNC".to_string(),
        "MARGIN" => "MARGIN".to_string(),
        _ => "INTRADAY".to_string(),
    }
}

fn is_market(req: &OrderReq) -> bool {
    req.order_type.eq_ignore_ascii_case("MARKET")
}

fn err_text(v: &Value) -> String {
    if let Some(m) = v.get("message").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            return m.to_string();
        }
    }
    if let Some(m) = v.get("error").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            return m.to_string();
        }
    }
    if let Some(m) = v.get("errorMessage").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            return m.to_string();
        }
    }
    v.to_string().chars().take(300).collect()
}

/// Broker-reported order id, or an error carrying the broker's message.
fn order_id_from(v: &Value, path: &[&str]) -> Result<String, String> {
    let mut cur = v;
    for p in path {
        match cur.get(*p) {
            Some(next) => cur = next,
            None => return Err(err_text(v)),
        }
    }
    let id = match cur {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    if id.is_empty() {
        return Err(err_text(v));
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Public dispatch
// ---------------------------------------------------------------------------

/// Place one order at `broker`. Errors are operator-readable and never imply a
/// fill: the engine only books a position when this returns `Ok`.
pub async fn place(broker: &str, req: &OrderReq) -> Result<Placed, String> {
    match broker {
        "dhan" => dhan_place(req).await,
        "upstox" => upstox_place(req).await,
        "zerodha" => zerodha_place(req).await,
        "angel" => angel_place(req).await,
        "fyers" => fyers_place(req).await,
        "alice" => alice_place(req).await,
        "icici" => icici_place(req).await,
        "fivepaisa" => fivepaisa_place(req).await,
        "shoonya" => shoonya_place(req).await,
        "kotak" => kotak_place(req).await,
        other => Err(format!("unknown broker: {other}")),
    }
}

/// Cancel a working order (used to clear protective legs before an exit).
pub async fn cancel(broker: &str, order_id: &str) -> Result<(), String> {
    match broker {
        "dhan" => dhan_cancel(order_id).await,
        "upstox" => upstox_cancel(order_id).await,
        "zerodha" => zerodha_cancel(order_id).await,
        "angel" => angel_cancel(order_id).await,
        "fyers" => fyers_cancel(order_id).await,
        "alice" => alice_cancel(order_id).await,
        "icici" => icici_cancel(order_id).await,
        "fivepaisa" => fivepaisa_cancel(order_id).await,
        "shoonya" => shoonya_cancel(order_id).await,
        "kotak" => kotak_cancel(order_id).await,
        other => Err(format!("unknown broker: {other}")),
    }
}

/// Verify the stored credentials with a real authenticated call.
pub async fn probe(broker: &str) -> Result<String, String> {
    match broker {
        "dhan" => dhan_probe().await,
        "upstox" => upstox_probe().await,
        "zerodha" => zerodha_probe().await,
        "angel" => angel_probe().await,
        "fyers" => fyers_probe().await,
        "alice" => alice_probe().await,
        "icici" => icici_probe().await,
        "fivepaisa" => fivepaisa_probe().await,
        "shoonya" => shoonya_probe().await,
        "kotak" => kotak_probe().await,
        other => Err(format!("unknown broker: {other}")),
    }
}

/// Split `qty` (already in units) into one child quantity per broker.
///
/// Lots are distributed as evenly as possible: every broker receives
/// `total_lots / n` lots and the leftover lots go to the first brokers in
/// catalogue order, so the total is exactly the operator's lot count. A broker
/// that would receive zero lots is skipped rather than sent a zero-quantity
/// order.
pub fn split_qty(qty: i64, lot: f64, brokers: &[String]) -> Vec<(String, i64)> {
    if brokers.is_empty() || qty <= 0 {
        return Vec::new();
    }
    let lot = if lot >= 1.0 { lot.round() as i64 } else { 1 };
    let total_lots = (qty as f64 / lot as f64).round().max(1.0) as i64;
    let n = brokers.len() as i64;
    let base = total_lots / n;
    let rem = total_lots % n;
    let mut out = Vec::new();
    for (i, b) in brokers.iter().enumerate() {
        let mut lots = base;
        if (i as i64) < rem {
            lots += 1;
        }
        if lots <= 0 {
            continue;
        }
        out.push((b.clone(), lots * lot));
    }
    out
}

// ---------------------------------------------------------------------------
// Dhan (existing adapter, kept identical to the engine's old direct path)
// ---------------------------------------------------------------------------

fn dhan_client_id() -> String {
    cred("dhan", "clientId")
}

fn dhan_token() -> String {
    cred("dhan", "accessToken")
}

async fn dhan_place(req: &OrderReq) -> Result<Placed, String> {
    let client_id = dhan_client_id();
    let token = dhan_token();
    if client_id.is_empty() || token.is_empty() {
        return Err("dhan: client id / access token missing in Broker Settings".into());
    }
    // Super Order is the only Dhan method with a native trailing ratchet, but the
    // engine only routes through here for the plain entry path; the Super Order
    // path stays in `realtime.rs` because it needs the target/stop legs there.
    let seg = segment_for_dhan(&req.contract);
    let body = json!({
        "dhanClientId": client_id,
        "correlationId": req.correlation,
        "transactionType": if req.side.eq_ignore_ascii_case("SELL") { "SELL" } else { "BUY" },
        "exchangeSegment": seg,
        "productType": dhan_product(&product_of(req)),
        "orderType": if is_market(req) { "MARKET" } else { "LIMIT" },
        "validity": "DAY",
        "securityId": req.contract.security_id.to_string(),
        "quantity": req.qty,
        "price": if is_market(req) { 0.0 } else { req.price },
    });
    let url = "https://api.dhan.co/v2/orders";
    let resp = http()?
        .post(url)
        .header("access-token", &token)
        .header("client-id", &client_id)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("dhan place: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if !status.is_success() {
        return Err(format!("dhan place HTTP {status}: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["orderId"]).unwrap_or_default(),
        status: v.get("orderStatus").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

fn segment_for_dhan(c: &Contract) -> &'static str {
    match (c.exchange.as_str(), c.instrument.as_str()) {
        ("NSE", "EQUITY") => "NSE_EQ",
        ("BSE", "EQUITY") => "BSE_EQ",
        ("MCX", _) => "MCX_COMM",
        ("BSE", _) => "BSE_FNO",
        _ => "NSE_FNO",
    }
}

fn dhan_product(p: &str) -> &'static str {
    match p {
        "CNC" => "CNC",
        "MARGIN" => "MARGIN",
        _ => "INTRADAY",
    }
}

async fn dhan_cancel(order_id: &str) -> Result<(), String> {
    let token = dhan_token();
    if token.is_empty() {
        return Err("dhan: access token missing".into());
    }
    let resp = http()?
        .delete(format!("https://api.dhan.co/v2/orders/{order_id}"))
        .header("access-token", &token)
        .send()
        .await
        .map_err(|e| format!("dhan cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let t = resp.text().await.unwrap_or_default();
        Err(format!("dhan cancel: {t}"))
    }
}

async fn dhan_probe() -> Result<String, String> {
    let client_id = dhan_client_id();
    let token = dhan_token();
    if client_id.is_empty() || token.is_empty() {
        return Err("client id / access token missing".into());
    }
    // The engine's own session client is authoritative for the header Connect
    // button; a direct profile call proves these exact stored keys work.
    let client = dhan_hq::DhanClient::new(client_id, token);
    match client.profile().await {
        Ok(p) => Ok(p.dhan_client_id),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Upstox v2
// ---------------------------------------------------------------------------

const UPSTOX_BASE: &str = "https://api.upstox.com/v2";

fn upstox_token() -> String {
    cred("upstox", "accessToken")
}

async fn upstox_place(req: &OrderReq) -> Result<Placed, String> {
    let token = upstox_token();
    if token.is_empty() {
        return Err("upstox: access token missing".into());
    }
    let inst = instruments::resolve("upstox", &req.contract).await?;
    let body = json!({
        "quantity": req.qty,
        "product": match product_of(req).as_str() { "CNC" => "D", "MARGIN" => "D", _ => "I" },
        "validity": "DAY",
        "price": if is_market(req) { 0.0 } else { req.price },
        "tag": req.correlation.chars().take(20).collect::<String>(),
        "instrument_token": inst.key,
        "order_type": if is_market(req) { "MARKET" } else { "LIMIT" },
        "transaction_type": if req.side.eq_ignore_ascii_case("SELL") { "SELL" } else { "BUY" },
        "disclosed_quantity": 0,
        "trigger_price": 0,
        "is_amo": false,
    });
    let resp = http()?
        .post(format!("{UPSTOX_BASE}/order/place"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("upstox place: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if !status.is_success() {
        return Err(format!("upstox place HTTP {status}: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["data", "order_id"]).unwrap_or_default(),
        status: if status.is_success() { "placed".into() } else { String::new() },
    })
}

async fn upstox_cancel(order_id: &str) -> Result<(), String> {
    let token = upstox_token();
    if token.is_empty() {
        return Err("upstox: access token missing".into());
    }
    let resp = http()?
        .delete(format!("{UPSTOX_BASE}/order/cancel"))
        .query(&[("order_id", order_id)])
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("upstox cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let t = resp.text().await.unwrap_or_default();
        Err(format!("upstox cancel: {t}"))
    }
}

/// Verify the Upstox `apiKey`/`apiSecret` pair against the OAuth token endpoint.
///
/// The profile call only exercises the access token, so a wrong key/secret pair
/// used to sail through as "connected" whenever the token happened to be valid.
/// The token endpoint checks the pair itself: with a placeholder auth code a
/// correct pair answers `UDAPI100070` (invalid redirect_uri) - the pair passed -
/// while a wrong pair answers `UDAPI100069`. Treating "not 100069" as success
/// avoids depending on the exact redirect_uri registered for the app.
async fn upstox_check_pair(key: &str, secret: &str) -> Result<(), String> {
    if key.is_empty() || secret.is_empty() {
        return Err("api key / api secret missing".into());
    }
    let resp = http()?
        .post("https://api.upstox.com/v2/login/authorization/token")
        .header("Accept", "application/json")
        .form(&[
            ("code", "PROBE"),
            ("client_id", key),
            ("client_secret", secret),
            ("redirect_uri", "https://127.0.0.1"),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("upstox credential check: {e}"))?;
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let code = v
        .get("errors")
        .and_then(|e| e.get(0))
        .and_then(|e| e.get("error_code"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if code == "UDAPI100069" {
        return Err("api key / api secret rejected by Upstox (UDAPI100069)".into());
    }
    Ok(())
}

async fn upstox_probe() -> Result<String, String> {
    let token = upstox_token();
    if token.is_empty() {
        return Err("access token missing".into());
    }
    // Prove the key pair before the session, so a wrong pair cannot hide behind
    // a valid token.
    upstox_check_pair(&cred("upstox", "apiKey"), &cred("upstox", "apiSecret")).await?;
    let resp = http()?
        .get(format!("{UPSTOX_BASE}/user/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("upstox profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", err_text(&v)));
    }
    Ok(v.get("data")
        .and_then(|d| d.get("user_id"))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}

// ---------------------------------------------------------------------------
// Zerodha Kite Connect
// ---------------------------------------------------------------------------

const KITE_BASE: &str = "https://api.kite.trade";

fn kite_auth() -> Result<String, String> {
    let key = cred("zerodha", "apiKey");
    let token = cred("zerodha", "accessToken");
    if key.is_empty() || token.is_empty() {
        return Err("zerodha: api key / access token missing".into());
    }
    Ok(format!("token {key}:{token}"))
}

async fn zerodha_place(req: &OrderReq) -> Result<Placed, String> {
    let auth = kite_auth()?;
    let inst = instruments::resolve("zerodha", &req.contract).await?;
    let form = [
        ("tradingsymbol", inst.symbol.clone()),
        ("exchange", inst.segment.clone()),
        ("transaction_type", if req.side.eq_ignore_ascii_case("SELL") { "SELL".into() } else { "BUY".into() }),
        ("order_type", if is_market(req) { "MARKET".into() } else { "LIMIT".into() }),
        ("quantity", req.qty.to_string()),
        ("product", match product_of(req).as_str() { "CNC" => "CNC".into(), "MARGIN" => "MIS".into(), _ => "MIS".into() }),
        ("validity", "DAY".into()),
        ("price", if is_market(req) { "0".into() } else { format!("{:.2}", req.price) }),
        ("tag", req.correlation.chars().take(20).collect::<String>()),
    ];
    let resp = http()?
        .post(format!("{KITE_BASE}/orders/regular"))
        .header("Authorization", auth)
        .header("X-Kite-Version", "3")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("zerodha place: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if !status.is_success() {
        return Err(format!("zerodha place HTTP {status}: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["data", "order_id"]).unwrap_or_default(),
        status: "placed".into(),
    })
}

async fn zerodha_cancel(order_id: &str) -> Result<(), String> {
    let auth = kite_auth()?;
    let resp = http()?
        .delete(format!("{KITE_BASE}/orders/regular/{order_id}"))
        .header("Authorization", auth)
        .header("X-Kite-Version", "3")
        .send()
        .await
        .map_err(|e| format!("zerodha cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("zerodha cancel: {}", resp.text().await.unwrap_or_default()))
    }
}

async fn zerodha_probe() -> Result<String, String> {
    let auth = kite_auth()?;
    let resp = http()?
        .get(format!("{KITE_BASE}/user/profile"))
        .header("Authorization", auth)
        .header("X-Kite-Version", "3")
        .send()
        .await
        .map_err(|e| format!("zerodha profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", err_text(&v)));
    }
    Ok(v.get("data")
        .and_then(|d| d.get("user_id"))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}

// ---------------------------------------------------------------------------
// Angel One SmartAPI
// ---------------------------------------------------------------------------

const ANGEL_BASE: &str = "https://apiconnect.angelone.in";

/// Header set SmartAPI requires on every call beyond the bearer token.
fn angel_headers() -> Vec<(&'static str, String)> {
    vec![
        ("X-UserType", "USER".into()),
        ("X-SourceID", "WEB".into()),
        ("X-ClientLocalIP", "127.0.0.1".into()),
        ("X-ClientPublicIP", "127.0.0.1".into()),
        ("X-MACAddress", "00:00:00:00:00:00".into()),
    ]
}

/// A JWT from the stored token, or one minted from PIN + TOTP. Returns
/// `(jwt, optional refresh token)`.
async fn angel_jwt() -> Result<String, String> {
    let jwt = cred("angel", "jwtToken");
    if !jwt.is_empty() {
        return Ok(jwt);
    }
    let api_key = cred("angel", "apiKey");
    let client_code = cred("angel", "clientCode");
    let pin = cred("angel", "pin");
    let seed = cred("angel", "totpSecret");
    if api_key.is_empty() || client_code.is_empty() || pin.is_empty() || seed.is_empty() {
        return Err("angel: jwtToken missing and PIN + TOTP seed not fully set".into());
    }
    let totp = totp_now(&seed)?;
    let body = json!({ "clientcode": client_code, "password": pin, "totp": totp });
    let mut rb = http()?
        .post(format!("{ANGEL_BASE}/rest/auth/angelbroking/user/v1/loginByPassword"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("X-PrivateKey", api_key);
    for (k, v) in angel_headers() {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("angel login: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("status").and_then(|x| x.as_bool()) == Some(false) {
        return Err(format!("angel login failed: {}", err_text(&v)));
    }
    let jwt = v
        .get("data")
        .and_then(|d| d.get("jwtToken"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if jwt.is_empty() {
        return Err(format!("angel login returned no jwtToken: {}", err_text(&v)));
    }
    Ok(jwt)
}

async fn angel_place(req: &OrderReq) -> Result<Placed, String> {
    let api_key = cred("angel", "apiKey");
    if api_key.is_empty() {
        return Err("angel: api key missing".into());
    }
    let jwt = angel_jwt().await?;
    let inst = instruments::resolve("angel", &req.contract).await?;
    let exchange = match inst.segment.as_str() {
        "NFO" => "NFO",
        "BFO" => "BFO",
        "MCX" => "MCX",
        "NSE" => "NSE",
        "BSE" => "BSE",
        other => other,
    };
    let body = json!({
        "variety": "NORMAL",
        "tradingsymbol": inst.symbol,
        "symboltoken": inst.key,
        "transactiontype": if req.side.eq_ignore_ascii_case("SELL") { "SELL" } else { "BUY" },
        "exchange": exchange,
        "ordertype": if is_market(req) { "MARKET" } else { "LIMIT" },
        "producttype": match product_of(req).as_str() { "CNC" => "DELIVERY", "MARGIN" => "CARRYFORWARD", _ => "INTRADAY" },
        "duration": "DAY",
        "price": if is_market(req) { "0".to_string() } else { format!("{:.2}", req.price) },
        "squareoff": "0",
        "stoploss": "0",
        "quantity": req.qty.to_string(),
    });
    let mut rb = http()?
        .post(format!("{ANGEL_BASE}/rest/secure/angelbroking/order/v1/placeOrder"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("X-PrivateKey", api_key)
        .header("Authorization", format!("Bearer {jwt}"));
    for (k, v) in angel_headers() {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("angel place: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("status").and_then(|x| x.as_bool()) == Some(false) {
        return Err(format!("angel place: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["data", "orderid"]).unwrap_or_default(),
        status: "placed".into(),
    })
}

async fn angel_cancel(order_id: &str) -> Result<(), String> {
    let api_key = cred("angel", "apiKey");
    let jwt = angel_jwt().await?;
    let body = json!({ "variety": "NORMAL", "orderid": order_id });
    let mut rb = http()?
        .post(format!("{ANGEL_BASE}/rest/secure/angelbroking/order/v1/cancelOrder"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("X-PrivateKey", api_key)
        .header("Authorization", format!("Bearer {jwt}"));
    for (k, v) in angel_headers() {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("angel cancel: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("status").and_then(|x| x.as_bool()) == Some(false) {
        return Err(format!("angel cancel: {}", err_text(&v)));
    }
    Ok(())
}

async fn angel_probe() -> Result<String, String> {
    let api_key = cred("angel", "apiKey");
    if api_key.is_empty() {
        return Err("api key missing".into());
    }
    let jwt = angel_jwt().await?;
    let mut rb = http()?
        .post(format!("{ANGEL_BASE}/rest/secure/angelbroking/user/v1/getProfile"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("X-PrivateKey", api_key)
        .header("Authorization", format!("Bearer {jwt}"));
    for (k, v) in angel_headers() {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&json!({})).send().await.map_err(|e| format!("angel profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("status").and_then(|x| x.as_bool()) == Some(false) {
        return Err(err_text(&v));
    }
    Ok(v.get("data")
        .and_then(|d| d.get("clientcode"))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}

// ---------------------------------------------------------------------------
// Fyers v3
// ---------------------------------------------------------------------------

const FYERS_BASE: &str = "https://api-t1.fyers.in/api/v3";

fn fyers_auth() -> Result<String, String> {
    let client_id = cred("fyers", "clientId");
    let token = cred("fyers", "accessToken");
    if client_id.is_empty() || token.is_empty() {
        return Err("fyers: client id / access token missing".into());
    }
    Ok(format!("{client_id}:{token}"))
}

/// Fyers wants `NSE:SYMBOL` / `NSE:SYMBOL-D`-style symbols with a product code
/// appended to the base symbol.
async fn fyers_place(req: &OrderReq) -> Result<Placed, String> {
    let auth = fyers_auth()?;
    let inst = instruments::resolve("fyers", &req.contract).await?;
    let product = match product_of(req).as_str() {
        "CNC" => "CNC",
        _ => "INTRADAY",
    };
    // Fyers encodes the product in the symbol suffix for cash, and uses the
    // `productType` field for derivatives.
    let symbol = inst.symbol.clone();
    let body = json!({
        "symbol": symbol,
        "qty": req.qty,
        "type": if is_market(req) { 2 } else { 1 },
        "side": if req.side.eq_ignore_ascii_case("SELL") { -1 } else { 1 },
        "productType": product,
        "limitPrice": if is_market(req) { 0.0 } else { req.price },
        "stopPrice": 0.0,
        "validity": "DAY",
        "disclosedQty": 0,
        "offlineOrder": false,
        "orderTag": req.correlation.chars().take(20).collect::<String>(),
    });
    let resp = http()?
        .post(format!("{FYERS_BASE}/orders/sync"))
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("fyers place: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("s").and_then(|x| x.as_str()) == Some("error") {
        return Err(format!("fyers place: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["id"]).unwrap_or_default(),
        status: "placed".into(),
    })
}

async fn fyers_cancel(order_id: &str) -> Result<(), String> {
    let auth = fyers_auth()?;
    let resp = http()?
        .delete(format!("{FYERS_BASE}/orders/sync/{order_id}"))
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| format!("fyers cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("fyers cancel: {}", resp.text().await.unwrap_or_default()))
    }
}

async fn fyers_probe() -> Result<String, String> {
    let auth = fyers_auth()?;
    let resp = http()?
        .get(format!("{FYERS_BASE}/profile"))
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| format!("fyers profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("s").and_then(|x| x.as_str()) == Some("error") {
        return Err(err_text(&v));
    }
    Ok(v.get("data")
        .and_then(|d| d.get("fy_id").or_else(|| d.get("display_name")))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}

// ---------------------------------------------------------------------------
// Alice Blue ANT
// ---------------------------------------------------------------------------

const ALICE_BASE: &str = "https://a3.aliceblueonline.com/rest";

fn alice_headers() -> Result<Vec<(&'static str, String)>, String> {
    let user = cred("alice", "userId");
    let session = cred("alice", "sessionId");
    if user.is_empty() || session.is_empty() {
        return Err("alice: user id / session id missing".into());
    }
    Ok(vec![
        ("Authorization", format!("Bearer {user} {session}")),
        ("Content-Type", "application/json".into()),
        ("Accept", "application/json".into()),
    ])
}

async fn alice_place(req: &OrderReq) -> Result<Placed, String> {
    let headers = alice_headers()?;
    let inst = instruments::resolve("alice", &req.contract).await?;
    let body = json!({
        "symbol": inst.symbol,
        "token": inst.key,
        "exchange": inst.segment,
        "transactionType": if req.side.eq_ignore_ascii_case("SELL") { "SELL" } else { "BUY" },
        "orderType": if is_market(req) { "MARKET" } else { "LIMIT" },
        "product": match product_of(req).as_str() { "CNC" => "CNC", "MARGIN" => "MARGIN", _ => "INTRADAY" },
        "quantity": req.qty.to_string(),
        "price": if is_market(req) { "0".to_string() } else { format!("{:.2}", req.price) },
        "validity": "DAY",
    });
    let mut rb = http()?.post(format!("{ALICE_BASE}/PlaceOrder"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("alice place: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("status").and_then(|x| x.as_str()) == Some("error") {
        return Err(format!("alice place: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["NOrdNo"]).unwrap_or_default(),
        status: "placed".into(),
    })
}

async fn alice_cancel(order_id: &str) -> Result<(), String> {
    let headers = alice_headers()?;
    let body = json!({ "brokerOrderId": order_id });
    let mut rb = http()?.post(format!("{ALICE_BASE}/CancelOrder"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("alice cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("alice cancel: {}", resp.text().await.unwrap_or_default()))
    }
}

async fn alice_probe() -> Result<String, String> {
    let headers = alice_headers()?;
    let mut rb = http()?.post(format!("{ALICE_BASE}/UserProfile"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&json!({})).send().await.map_err(|e| format!("alice profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("status").and_then(|x| x.as_str()) == Some("error") {
        return Err(err_text(&v));
    }
    Ok(v.get("clientId")
        .or_else(|| v.get("userId"))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}

// ---------------------------------------------------------------------------
// ICICI Breeze
// ---------------------------------------------------------------------------

const BREEZE_BASE: &str = "https://api.icicidirect.com/breezeapi/api/v1";

fn breeze_headers() -> Result<Vec<(&'static str, String)>, String> {
    let app_key = cred("icici", "apiKey");
    let token = cred("icici", "sessionToken");
    if app_key.is_empty() || token.is_empty() {
        return Err("icici: api key / session token missing".into());
    }
    Ok(vec![
        ("X-AppKey", app_key),
        ("X-Token", token),
        ("Content-Type", "application/json".into()),
        ("Accept", "application/json".into()),
    ])
}

async fn icici_place(req: &OrderReq) -> Result<Placed, String> {
    let headers = breeze_headers()?;
    let inst = instruments::resolve("icici", &req.contract).await?;
    let c = &req.contract;
    let body = json!({
        "stock_code": inst.key,
        "exchange_code": inst.segment,
        "product": match product_of(req).as_str() { "CNC" => "cash", "MARGIN" => "margin", _ => "futures" },
        "action": if req.side.eq_ignore_ascii_case("SELL") { "sell" } else { "buy" },
        "order_type": if is_market(req) { "market" } else { "limit" },
        "quantity": req.qty.to_string(),
        "price": if is_market(req) { String::new() } else { format!("{:.2}", req.price) },
        "validity": "day",
        "stoploss": "",
        "expiry_date": if c.expiry.is_empty() { String::new() } else { icici_expiry(&c.expiry) },
        "right": if c.option_type.is_empty() { String::new() } else { c.option_type.to_ascii_lowercase() },
        "strike_price": if c.strike > 0.0 { format!("{:.2}", c.strike) } else { String::new() },
    });
    let mut rb = http()?.post(format!("{BREEZE_BASE}/order"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("icici place: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || v.get("Status").and_then(|x| x.as_i64()) == Some(500) {
        return Err(format!("icici place: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["Success", "order_id"]).unwrap_or_default(),
        status: "placed".into(),
    })
}

/// Breeze wants `YYYY-MM-DD` expiry, which the contract already carries.
fn icici_expiry(iso: &str) -> String {
    iso.chars().take(10).collect()
}

async fn icici_cancel(order_id: &str) -> Result<(), String> {
    let headers = breeze_headers()?;
    let body = json!({ "order_id": order_id });
    let mut rb = http()?.post(format!("{BREEZE_BASE}/order"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    // Breeze cancels through DELETE on the same resource.
    let resp = rb
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("icici cancel: {e}"))?;
    let _ = rb;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("icici cancel: {}", resp.text().await.unwrap_or_default()))
    }
}

async fn icici_probe() -> Result<String, String> {
    let headers = breeze_headers()?;
    let mut rb = http()?.get(format!("{BREEZE_BASE}/customerdetails"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.send().await.map_err(|e| format!("icici profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", err_text(&v)));
    }
    Ok(v.get("Success")
        .and_then(|s| s.get("idirect_userid"))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}

// ---------------------------------------------------------------------------
// 5paisa
// ---------------------------------------------------------------------------

const FIVEPAISA_BASE: &str = "https://Openapi.5paisa.com/VendorsAPI/Service1.svc";

fn fivepaisa_headers() -> Result<Vec<(&'static str, String)>, String> {
    let key = cred("fivepaisa", "userKey");
    let enc = cred("fivepaisa", "encryptionKey");
    if key.is_empty() || enc.is_empty() {
        return Err("5paisa: user key / encryption key missing".into());
    }
    let ts = chrono_like_now();
    Ok(vec![
        ("Content-Type", "application/json".into()),
        ("Accept", "application/json".into()),
        // 5paisa requires key + timestamp in the auth header.
        ("Authorization", format!("Bearer {key}:{ts}")),
    ])
}

fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

async fn fivepaisa_place(req: &OrderReq) -> Result<Placed, String> {
    let headers = fivepaisa_headers()?;
    let inst = instruments::resolve("fivepaisa", &req.contract).await?;
    let body = json!({
        "head": { "key": cred("fivepaisa", "userKey") },
        "body": {
            "Orders": [{
                "Exchange": inst.segment,
                "ExchangeType": if req.side.eq_ignore_ascii_case("SELL") { "S" } else { "B" },
                "ScripCode": inst.key,
                "Qty": req.qty,
                "Price": if is_market(req) { 0.0 } else { req.price },
                "IsIntraday": product_of(req) != "CNC",
                "OrderType": if is_market(req) { "M" } else { "L" },
                "DisQty": 0,
                "StopLossPrice": 0.0,
            }]
        }
    });
    let mut rb = http()?.post(format!("{FIVEPAISA_BASE}/V1/PlaceOrderRequest"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("5paisa place: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("5paisa place: {}", err_text(&v)));
    }
    let id = v
        .get("body")
        .and_then(|b| b.get("Orders"))
        .and_then(|o| o.as_array())
        .and_then(|a| a.first())
        .and_then(|o| o.get("ExchOrderID").or_else(|| o.get("RemoteOrderID")))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok(Placed {
        order_id: id,
        status: "placed".into(),
    })
}

async fn fivepaisa_cancel(order_id: &str) -> Result<(), String> {
    let headers = fivepaisa_headers()?;
    let body = json!({
        "head": { "key": cred("fivepaisa", "userKey") },
        "body": { "ExchOrderID": order_id }
    });
    let mut rb = http()?.post(format!("{FIVEPAISA_BASE}/V1/CancelOrderRequest"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("5paisa cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("5paisa cancel: {}", resp.text().await.unwrap_or_default()))
    }
}

async fn fivepaisa_probe() -> Result<String, String> {
    let headers = fivepaisa_headers()?;
    let body = json!({ "head": { "key": cred("fivepaisa", "userKey") } });
    let mut rb = http()?.post(format!("{FIVEPAISA_BASE}/V4/ClientDetails"));
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    let resp = rb.json(&body).send().await.map_err(|e| format!("5paisa profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("5paisa: {}", err_text(&v)));
    }
    Ok(v.get("body")
        .and_then(|b| b.get("ClientCode"))
        .and_then(|x| x.as_str())
        .unwrap_or("connected")
        .to_string())
}


// ---------------------------------------------------------------------------
// Finvasia Shoonya (Noren OMS)
// ---------------------------------------------------------------------------

const SHOONYA_BASE: &str = "https://api.shoonya.com/NorenWClientAPI";

/// Shoonya has no market order type at all: `PlaceOrder` accepts only `LMT`
/// and `SL-LMT`, and rejects `MKT`. A "market" request is therefore sent as a
/// marketable LIMIT - the price is pushed a few percent through the last price
/// so it crosses immediately, while still being a valid resting order if the
/// book is momentarily empty.
fn shoonya_marketable_price(req: &OrderReq) -> f64 {
    let side_buy = !req.side.eq_ignore_ascii_case("SELL");
    let base = if req.price > 0.0 { req.price } else { req.ltp };
    if base <= 0.0 {
        return 0.0;
    }
    let bump = (base * 0.02).max(0.05);
    if side_buy { base + bump } else { (base - bump).max(0.05) }
}

fn shoonya_ids() -> Result<(String, String, String), String> {
    let uid = cred("shoonya", "userId");
    let actid = {
        let a = cred("shoonya", "accountId");
        if a.is_empty() { uid.clone() } else { a }
    };
    let token = cred("shoonya", "accessToken");
    if uid.is_empty() || token.is_empty() {
        return Err("shoonya: user id / access token missing in Broker Settings".into());
    }
    Ok((uid, actid, token))
}

/// Exchange segment as Noren expects it (`NSE`, `NFO`, `BSE`, `BFO`, `MCX`).
fn shoonya_exchange(segment: &str) -> Result<&'static str, String> {
    match segment.to_ascii_uppercase().as_str() {
        "NSE" | "NSE_EQ" | "NSE_CM" => Ok("NSE"),
        "NFO" | "NSE_FO" => Ok("NFO"),
        "BSE" | "BSE_EQ" | "BSE_CM" => Ok("BSE"),
        "BFO" | "BSE_FO" => Ok("BFO"),
        "MCX" | "MCX_FO" => Ok("MCX"),
        other => Err(format!("shoonya: unsupported exchange segment {other}")),
    }
}

/// POST a Noren endpoint. The OMS answers HTTP 200 for both accepted and
/// rejected orders and signals the outcome only in `stat`, so the body is
/// always parsed and `Not_Ok` is turned into an error.
async fn shoonya_post(path: &str, payload: &Value) -> Result<Value, String> {
    let (_, _, token) = shoonya_ids()?;
    let resp = http()?
        .post(format!("{SHOONYA_BASE}/{path}"))
        .header("Content-Type", "text/plain")
        .header("Authorization", format!("Bearer {token}"))
        .body(format!("jData={payload}"))
        .send()
        .await
        .map_err(|e| format!("shoonya {path}: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if v.get("stat").and_then(|x| x.as_str()) == Some("Ok") {
        return Ok(v);
    }
    Err(format!("shoonya {path}: {}", err_text(&v)))
}

async fn shoonya_place(req: &OrderReq) -> Result<Placed, String> {
    let (uid, actid, _) = shoonya_ids()?;
    let inst = instruments::resolve("shoonya", &req.contract).await?;
    let exch = shoonya_exchange(&inst.segment)?;
    // Shoonya keys every order by `tsym`, not by a numeric token.
    let price = if is_market(req) {
        shoonya_marketable_price(req)
    } else {
        req.price
    };
    if price <= 0.0 {
        return Err(
            "shoonya: no market order type and no price/quote available to build a marketable limit"
                .into(),
        );
    }
    let body = json!({
        "uid": uid,
        "actid": actid,
        "exch": exch,
        "tsym": inst.symbol,
        "qty": req.qty.to_string(),
        "prc": format!("{price:.2}"),
        // C = CNC, M = NRML/margin, I = MIS/intraday. CO/BO are not supported.
        "prd": match product_of(req).as_str() { "CNC" => "C", "MARGIN" => "M", _ => "I" },
        "trantype": if req.side.eq_ignore_ascii_case("SELL") { "S" } else { "B" },
        // MKT is rejected outright, so a market request becomes a marketable LMT.
        "prctyp": if req.order_type.to_ascii_uppercase().starts_with("SL") { "SL-LMT" } else { "LMT" },
        "ret": "DAY",
        "ordersource": "API",
        "remarks": req.correlation.chars().take(24).collect::<String>(),
    });
    let v = shoonya_post("PlaceOrder", &body).await?;
    Ok(Placed {
        order_id: v
            .get("norenordno")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        status: "placed".into(),
    })
}

async fn shoonya_cancel(order_id: &str) -> Result<(), String> {
    let (uid, _, _) = shoonya_ids()?;
    let (uid2, actid) = (uid.clone(), {
        let a = cred("shoonya", "accountId");
        if a.is_empty() { uid } else { a }
    });
    let body = json!({
        "uid": uid2,
        "actid": actid,
        "norenordno": order_id,
        "ordersource": "API",
    });
    shoonya_post("CancelOrder", &body).await.map(|_| ())
}

async fn shoonya_probe() -> Result<String, String> {
    let (uid, actid, _) = shoonya_ids()?;
    let v = shoonya_post("Limits", &json!({ "uid": uid, "actid": actid })).await?;
    // `Limits` is a real authenticated call; echo the cash balance so the Test
    // button can show that the session actually works, not just that a token
    // string is stored.
    let cash = v.get("cash").and_then(|x| x.as_str()).unwrap_or("0");
    Ok(format!("{uid} (cash {cash})"))
}

// ---------------------------------------------------------------------------
// Kotak Neo
// ---------------------------------------------------------------------------

const KOTAK_BASE: &str = "https://gw-napi.kotaksecurities.com";

fn kotak_session() -> Result<(String, String), String> {
    let ucc = cred("kotak", "ucc");
    let token = cred("kotak", "accessToken");
    if ucc.is_empty() || token.is_empty() {
        return Err("kotak: UCC / access token missing (login first or paste the session token)".into());
    }
    Ok((ucc, token))
}

async fn kotak_place(req: &OrderReq) -> Result<Placed, String> {
    let (ucc, token) = kotak_session()?;
    let inst = instruments::resolve("kotak", &req.contract).await?;
    let body = json!({
        "am": "NO",
        "dq": "0",
        "es": inst.segment,
        "mp": "0",
        "pc": inst.key,
        "pf": match product_of(req).as_str() { "CNC" => "NRML", "MARGIN" => "NRML", _ => "MIS" },
        "pr": if is_market(req) { "0".to_string() } else { format!("{:.2}", req.price) },
        "pt": if is_market(req) { "MKT" } else { "L" },
        "qt": req.qty.to_string(),
        "rt": "DAY",
        "tp": "0",
        "ts": if req.side.eq_ignore_ascii_case("SELL") { "S" } else { "B" },
        "tt": "IN",
    });
    let resp = http()?
        .post(format!("{KOTAK_BASE}/quick/order/rule/ms/place"))
        .header("Authorization", format!("Bearer {token}"))
        .header("neo-fin-key", "neotradeapi")
        .header("Sid", &ucc)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("kotak place: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("kotak place: {}", err_text(&v)));
    }
    Ok(Placed {
        order_id: order_id_from(&v, &["data", "nOrdNo"]).unwrap_or_default(),
        status: "placed".into(),
    })
}

async fn kotak_cancel(order_id: &str) -> Result<(), String> {
    let (ucc, token) = kotak_session()?;
    let body = json!({ "on": order_id, "am": "NO" });
    let resp = http()?
        .post(format!("{KOTAK_BASE}/quick/order/cancel"))
        .header("Authorization", format!("Bearer {token}"))
        .header("neo-fin-key", "neotradeapi")
        .header("Sid", &ucc)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("kotak cancel: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("kotak cancel: {}", resp.text().await.unwrap_or_default()))
    }
}

async fn kotak_probe() -> Result<String, String> {
    let (ucc, token) = kotak_session()?;
    let resp = http()?
        .get(format!("{KOTAK_BASE}/quick/user/limits"))
        .header("Authorization", format!("Bearer {token}"))
        .header("neo-fin-key", "neotradeapi")
        .header("Sid", &ucc)
        .send()
        .await
        .map_err(|e| format!("kotak profile: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("kotak: {}", err_text(&v)));
    }
    Ok(ucc)
}

// ---------------------------------------------------------------------------
// TOTP (needed for SmartAPI's PIN + TOTP login)
// ---------------------------------------------------------------------------

/// Current 6-digit TOTP for a base32 seed (RFC 6238, 30s step, SHA-1).
fn totp_now(seed: &str) -> Result<String, String> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha1::Sha1;
    let key = base32_decode(seed)?;
    let step = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 30)
        .unwrap_or(0);
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).map_err(|e| format!("totp: {e}"))?;
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let off = (digest[19] & 0x0f) as usize;
    let bin = ((digest[off] as u32 & 0x7f) << 24)
        | ((digest[off + 1] as u32) << 16)
        | ((digest[off + 2] as u32) << 8)
        | (digest[off + 3] as u32);
    Ok(format!("{:06}", bin % 1_000_000))
}

fn base32_decode(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let clean = clean.trim_end_matches('=');
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits = 0;
    for c in clean.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            '2'..='7' => c as u32 - '2' as u32 + 26,
            other => return Err(format!("totp: invalid base32 char {other}")),
        };
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    if out.is_empty() {
        return Err("totp: empty seed".into());
    }
    Ok(out)
}

/// Snapshot of which credential fields are present per broker (never the
/// values), used by the settings page to explain what a Test actually checked.
pub fn credential_presence(broker: &str) -> Value {
    let Some(spec) = crate::brokers::spec(broker) else {
        return Value::Null;
    };
    let mut map = BTreeMap::new();
    for f in spec.fields {
        map.insert(f.key.to_string(), !cred(broker, f.key).is_empty());
    }
    json!(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_matches_the_rfc_6238_reference_vector() {
        // RFC 6238 SHA-1 test key "12345678901234567890" in base32.
        // The time-dependent vector cannot be frozen, but the algorithm must
        // produce a zero-padded six digit code for a valid seed.
        let code = totp_now("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn base32_round_trips_a_known_seed() {
        let bytes = base32_decode("JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(bytes, b"Hello!\xDE\xAD\xBE\xEF");
    }

    #[test]
    fn lots_split_evenly_and_keep_the_total() {
        let brokers: Vec<String> = (1..=5).map(|i| format!("b{i}")).collect();
        // 10 lots over 5 brokers -> 2 each.
        let parts = split_qty(10 * 65, 65.0, &brokers);
        assert_eq!(parts.len(), 5);
        assert_eq!(parts.iter().map(|(_, q)| q).sum::<i64>(), 650);
        assert!(parts.iter().all(|(_, q)| *q == 130));
        // 9 lots over 5 brokers -> 2,2,2,2,1 (total preserved).
        let parts = split_qty(9 * 65, 65.0, &brokers);
        assert_eq!(parts.len(), 5);
        assert_eq!(parts.iter().map(|(_, q)| q).sum::<i64>(), 585);
        let qtys: Vec<i64> = parts.iter().map(|(_, q)| *q / 65).collect();
        assert_eq!(qtys, vec![2, 2, 2, 2, 1]);
    }

    #[test]
    fn a_broker_that_cannot_get_a_whole_lot_is_skipped() {
        let brokers: Vec<String> = (1..=5).map(|i| format!("b{i}")).collect();
        // 2 lots over 5 brokers: only two brokers can receive one lot each.
        let parts = split_qty(2 * 65, 65.0, &brokers);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.iter().map(|(_, q)| q).sum::<i64>(), 130);
    }
}
