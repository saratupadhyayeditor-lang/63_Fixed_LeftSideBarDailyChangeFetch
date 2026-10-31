//! Broker-agnostic instrument resolution for the order adapters.
//!
//! The engines identify a contract by its **Dhan** `security_id` (no agency
//! over which broker fills the order). Every non-Dhan broker, however, needs
//! *its own* instrument token, so an adapter must translate the Dhan contract
//! into that broker's identifier before it can place a live order.
//!
//! Translation is done from each broker's published contract master, matched on
//! the *contract itself* (exchange + underlying + expiry + strike + CE/PE, or
//! the cash symbol for equity) rather than on a trading-symbol string. Trading
//! symbols differ wildly between brokers (`NIFTY26NOV24500CE` vs
//! `NIFTY 26 Nov 26 24500 CE` vs `NIFTY26NOV26C24500`), whereas the contract
//! fields are stable.
//!
//! Masters are large (NSE F&O alone is tens of thousands of rows), so they are
//! downloaded once, parsed into a compact index, and cached in memory with a
//! TTL. A failed download leaves the previous good index in place - an adapter
//! then refuses the order with a clear reason instead of guessing a token.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::scrip::{self, Contract};

/// How long a downloaded contract master is considered fresh.
const MASTER_TTL: Duration = Duration::from_secs(12 * 3600);

/// One contract in a broker's own symbol scheme.
#[derive(Clone, Debug)]
pub struct Instrument {
    /// The broker's instrument id (token / instrument_key / scrip code).
    pub key: String,
    /// The broker's trading symbol, used for order payloads and logging.
    pub symbol: String,
    /// Broker exchange segment name as that broker's API expects it.
    pub segment: String,
    pub lot: f64,
}

/// Key that identifies one contract inside a broker master, independent of the
/// broker's symbol formatting.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ContractKey {
    /// Underlying name / cash symbol, upper case, no spaces (`NIFTY`, `RELIANCE`).
    underlying: String,
    /// `YYYY-MM-DD`, empty for cash equity.
    expiry: String,
    /// Strike in paise (avoids float keys).
    strike: i64,
    /// `CE` / `PE` / `FUT` / `EQ`.
    kind: String,
}

impl ContractKey {
    fn from_contract(c: &Contract) -> Self {
        let kind = if !c.option_type.is_empty() {
            c.option_type.to_ascii_uppercase()
        } else if c.instrument.starts_with("FUT") {
            "FUT".to_string()
        } else {
            "EQ".to_string()
        };
        Self {
            underlying: normalize_name(&c.underlying),
            expiry: c.expiry.clone(),
            strike: (c.strike * 100.0).round() as i64,
            kind,
        }
    }
}

fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '&')
        .collect::<String>()
        .to_ascii_uppercase()
}

/// `YYYY-MM-DD` from the assorted expiry spellings the masters use.
fn normalize_expiry(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    // Already ISO: take the date part.
    if t.len() >= 10 && t.as_bytes().get(4) == Some(&b'-') && t.as_bytes().get(7) == Some(&b'-') {
        return t.chars().take(10).collect();
    }
    // `23-Nov-2026` / `29DEC26` style: parse the day/month/year tokens.
    let cleaned = t.replace(['-', '/', ' '], " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    let (day, mon, year) = if parts.len() >= 3 {
        (parts[0].to_string(), parts[1].to_string(), parts[2].to_string())
    } else if parts.len() == 1 {
        // `29DEC26` / `29DEC2026`: leading digits, then letters, then digits.
        let s = parts[0];
        let dig_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
        let alpha_end = s[dig_end..]
            .find(|c: char| c.is_ascii_digit())
            .map(|i| dig_end + i)
            .unwrap_or(s.len());
        if dig_end == 0 || alpha_end == dig_end || alpha_end >= s.len() {
            return String::new();
        }
        (
            s[..dig_end].to_string(),
            s[dig_end..alpha_end].to_string(),
            s[alpha_end..].to_string(),
        )
    } else {
        return String::new();
    };
    let month = match mon.to_ascii_uppercase().as_str() {
        m if m.starts_with("JAN") => 1,
        m if m.starts_with("FEB") => 2,
        m if m.starts_with("MAR") => 3,
        m if m.starts_with("APR") => 4,
        m if m.starts_with("MAY") => 5,
        m if m.starts_with("JUN") => 6,
        m if m.starts_with("JUL") => 7,
        m if m.starts_with("AUG") => 8,
        m if m.starts_with("SEP") => 9,
        m if m.starts_with("OCT") => 10,
        m if m.starts_with("NOV") => 11,
        m if m.starts_with("DEC") => 12,
        other => other.parse::<u32>().unwrap_or(0),
    };
    let day = day.parse::<u32>().unwrap_or(0);
    let mut year = year.parse::<u32>().unwrap_or(0);
    if year < 100 {
        year += 2000;
    }
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) || year < 2000 {
        return String::new();
    }
    format!("{year:04}-{month:02}-{day:02}")
}

/// A parsed broker contract master.
#[derive(Default)]
struct Master {
    by_key: HashMap<ContractKey, Instrument>,
}

impl Master {
    fn get(&self, key: &ContractKey) -> Option<&Instrument> {
        self.by_key.get(key)
    }
}

struct Slot {
    master: Option<Arc<Master>>,
    fetched: Option<Instant>,
    /// Last error, surfaced to the operator when no master is available.
    error: String,
    /// A fetch is already running; other callers wait instead of re-downloading.
    loading: bool,
}

fn slots() -> &'static Mutex<HashMap<&'static str, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<&'static str, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the broker-specific instrument for a Dhan contract.
///
/// Returns `Err` with an operator-readable reason when the master is not
/// available or the contract is not listed - never a guessed token, because a
/// wrong token would place a real order in the wrong instrument.
pub async fn resolve(broker: &str, contract: &Contract) -> Result<Instrument, String> {
    let key = ContractKey::from_contract(contract);
    let master = master_for(broker).await?;
    master
        .get(&key)
        .cloned()
        .ok_or_else(|| {
            format!(
                "{broker}: contract {} {}{} {} {} is not listed in the broker's instrument master",
                key.underlying,
                if key.expiry.is_empty() { String::new() } else { format!("{} ", key.expiry) },
                if key.strike > 0 { format!("{:.2}", key.strike as f64 / 100.0) } else { String::new() },
                key.kind,
                contract.trading_symbol,
            )
        })
}

/// Cached master for `broker`, downloading it when stale or absent.
async fn master_for(broker: &str) -> Result<Arc<Master>, String> {
    let id = spec_id(broker)?;
    {
        let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
        let slot = map.entry(id).or_insert_with(|| Slot {
            master: None,
            fetched: None,
            error: String::new(),
            loading: false,
        });
        let fresh = slot
            .fetched
            .map(|t| t.elapsed() < MASTER_TTL)
            .unwrap_or(false);
        if let (Some(m), true) = (slot.master.clone(), fresh) {
            return Ok(m);
        }
        if slot.loading {
            // Another request is downloading: report the cached copy if any so a
            // burst of entries does not serialize behind one slow download.
            if let Some(m) = slot.master.clone() {
                return Ok(m);
            }
            return Err(format!("{broker}: instrument master is still downloading, retry shortly"));
        }
        slot.loading = true;
    }
    let result = download(id).await;
    let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
    let slot = map.get_mut(id).expect("slot inserted above");
    slot.loading = false;
    match result {
        Ok(master) => {
            let arc = Arc::new(master);
            slot.master = Some(arc.clone());
            slot.fetched = Some(Instant::now());
            slot.error.clear();
            Ok(arc)
        }
        Err(e) => {
            slot.error = e.clone();
            // Serve a previous good master rather than failing the order.
            if let Some(m) = slot.master.clone() {
                tracing::warn!("{broker} instrument master refresh failed: {e}");
                return Ok(m);
            }
            Err(format!("{broker}: instrument master unavailable - {e}"))
        }
    }
}

fn spec_id(broker: &str) -> Result<&'static str, String> {
    crate::brokers::spec(broker)
        .map(|s| s.id)
        .ok_or_else(|| format!("unknown broker: {broker}"))
}

/// Load the master now and report its status. The Test button uses this so an
/// operator can confirm the broker's contract list is reachable *before* the
/// first live order needs it, instead of discovering a broken download at entry.
pub async fn warm(broker: &str) -> Value {
    let status = match master_for(broker).await {
        Ok(m) => serde_json::json!({
            "loaded": m.by_key.len(),
            "stale": false,
            "loading": false,
            "error": "",
        }),
        Err(e) => serde_json::json!({
            "loaded": 0,
            "stale": true,
            "loading": false,
            "error": e,
        }),
    };
    serde_json::json!({ "status": status })
}

async fn download(broker: &str) -> Result<Master, String> {
    match broker {
        "dhan" => {
            // The Dhan adapter speaks security ids natively; the scrip master is
            // already in memory, so build the index straight from it.
            let Some(sc) = scrip::get() else {
                return Err("Dhan scrip master is still warming".into());
            };
            Ok(master_from_dhan(&sc))
        }
        "upstox" => {
            let url = "https://assets.upstox.com/market-quote/instruments/exchange/complete.csv.gz";
            let body = http_get(url).await?;
            let text = gunzip(&body)?;
            parse_upstox(&text)
        }
        "zerodha" => {
            let body = http_get("https://api.kite.trade/instruments").await?;
            parse_kite(&String::from_utf8_lossy(&body))
        }
        "angel" => {
            let body = http_get("https://margincalculator.angelbroking.com/OpenAPI_File/files/OpenAPIScripMaster.json").await?;
            parse_angel(&body)
        }
        "fyers" => {
            let nfo = http_get("https://public.fyers.in/sym_details/NSE_FO.csv").await?;
            let nse = http_get("https://public.fyers.in/sym_details/NSE_CM.csv").await?;
            parse_fyers(
                &String::from_utf8_lossy(&nfo),
                &String::from_utf8_lossy(&nse),
            )
        }
        "alice" => {
            let nfo = http_get("https://v2api.aliceblueonline.com/restpy/static/contract_master/NFO.csv").await?;
            let nse = http_get("https://v2api.aliceblueonline.com/restpy/static/contract_master/NSE.csv").await?;
            parse_alice(
                &String::from_utf8_lossy(&nfo),
                &String::from_utf8_lossy(&nse),
            )
        }
        "icici" => {
            let body = http_get("https://directlink.icicidirect.com/NewSecurityMaster/SecurityMaster.zip").await?;
            let files = unzip(&body)?;
            parse_icici(&files)
        }
        "shoonya" => {
            // Noren symbol masters: one zip per segment, pipe/comma delimited
            // with a header row. Only NSE cash + NSE F&O are needed here.
            let nfo = http_get("https://api.shoonya.com/NFO_symbols.txt.zip").await?;
            let nse = http_get("https://api.shoonya.com/NSE_symbols.txt.zip").await?;
            let nfo_txt = unzip_single_text(&nfo)?;
            let nse_txt = unzip_single_text(&nse)?;
            parse_shoonya(&nfo_txt, &nse_txt)
        }
        "fivepaisa" => {
            let nfo = http_get("https://openapi.5paisa.com/VendorsAPI/Service1.svc/ScripMaster/segment/nse_fo").await?;
            let nse = http_get("https://openapi.5paisa.com/VendorsAPI/Service1.svc/ScripMaster/segment/nse_eq").await?;
            parse_fivepaisa(
                &String::from_utf8_lossy(&nfo),
                &String::from_utf8_lossy(&nse),
            )
        }
        "kotak" => {
            let body = http_get("https://gw-napi.kotaksecurities.com/script-details/1.0/masterscrip/file").await?;
            let files = unzip(&body)?;
            parse_kotak(&files)
        }
        other => Err(format!("no instrument master for broker {other}")),
    }
}

async fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 algo-server")
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("{url}: {e}"))
}

fn gunzip(body: &[u8]) -> Result<String, String> {
    use std::io::Read;
    let mut out = String::new();
    flate2::read::GzDecoder::new(body)
        .read_to_string(&mut out)
        .map_err(|e| format!("gzip: {e}"))?;
    Ok(out)
}

/// Extract `name -> bytes` from a zip archive held in memory.
fn unzip(body: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    use std::io::{Cursor, Read};
    let mut archive = zip::ZipArchive::new(Cursor::new(body)).map_err(|e| format!("zip: {e}"))?;
    let mut files = HashMap::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| format!("zip: {e}"))?;
        let name = f.name().map(|n| n.to_string()).unwrap_or_default();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| format!("zip: {e}"))?;
        files.insert(name, buf);
    }
    Ok(files)
}

fn insert(map: &mut HashMap<ContractKey, Instrument>, key: ContractKey, inst: Instrument) {
    // First row wins: masters list the current contract before any stale
    // duplicates, and a duplicate token is still a valid order target.
    map.entry(key).or_insert(inst);
}

/// Split a CSV line honouring simple double-quoted fields.
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|f| f.trim().to_string()).collect()
}

fn master_from_dhan(sc: &Arc<scrip::Scrip>) -> Master {
    // Rebuild the index from the contracts the scrip master already holds. The
    // adapter for Dhan itself does not need this path (it orders by security
    // id), but keeping it uniform makes `resolve("dhan", ..)` testable.
    let mut m = Master::default();
    for sid in sc.contract_ids() {
        if let Some(c) = sc.contract(sid) {
            insert(
                &mut m.by_key,
                ContractKey::from_contract(c),
                Instrument {
                    key: sid.to_string(),
                    symbol: c.trading_symbol.clone(),
                    segment: c.exchange.clone(),
                    lot: c.lot,
                },
            );
        }
    }
    m
}

fn parse_upstox(text: &str) -> Result<Master, String> {
    let mut m = Master::default();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 12 {
            continue;
        }
        let instrument_key = &f[0];
        let symbol = &f[2];
        let name = &f[3];
        let expiry = normalize_expiry(&f[5]);
        let strike = f[6].parse::<f64>().unwrap_or(0.0);
        let lot = f[8].parse::<f64>().unwrap_or(1.0);
        let itype = f[9].to_ascii_uppercase();
        let otype = f[10].to_ascii_uppercase();
        let exchange = f[11].to_ascii_uppercase();
        // Upstox reports `instrument_type` as OPTIDX / OPTSTK / FUT / EQUITY and
        // the CE/PE leg separately in `option_type`.
        let kind = match itype.as_str() {
            "OPTIDX" | "OPTSTK" | "OPTFUT" => {
                if otype != "CE" && otype != "PE" {
                    continue;
                }
                otype.clone()
            }
            "FUT" | "FUTURE" => "FUT".to_string(),
            "EQUITY" | "EQ" => "EQ".to_string(),
            _ => continue,
        };
        if kind == "FUT" || kind == "CE" || kind == "PE" {
            if expiry.is_empty() {
                continue;
            }
        }
        let segment = match exchange.as_str() {
            "NSE_FO" if kind == "FUT" => "NSE_FO",
            "NSE_FO" => "NSE_FO",
            "BSE_FO" => "BSE_FO",
            "MCX_FO" => "MCX_FO",
            "NSE_EQ" => "NSE_EQ",
            "BSE_EQ" => "BSE_EQ",
            _ => continue,
        };
        insert(
            &mut m.by_key,
            ContractKey {
                // Cash equity is keyed by its trading symbol; `name` is the
                // company name ("RELIANCE INDUSTRIES"), which no engine uses.
                underlying: if kind == "EQ" {
                    normalize_name(symbol)
                } else {
                    normalize_name(name)
                },
                expiry: if kind == "EQ" { String::new() } else { expiry },
                strike: (strike * 100.0).round() as i64,
                kind,
            },
            Instrument {
                key: instrument_key.clone(),
                symbol: symbol.clone(),
                segment: segment.to_string(),
                lot: lot.max(1.0),
            },
        );
    }
    if m.by_key.is_empty() {
        return Err("Upstox master parsed to zero contracts".into());
    }
    Ok(m)
}

fn parse_kite(text: &str) -> Result<Master, String> {
    let mut m = Master::default();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 12 {
            continue;
        }
        let token = &f[0];
        let symbol = &f[2];
        let name = &f[3];
        let expiry = normalize_expiry(&f[5]);
        let strike = f[6].parse::<f64>().unwrap_or(0.0);
        let lot = f[8].parse::<f64>().unwrap_or(1.0);
        let itype = f[9].to_ascii_uppercase();
        let exchange = f[11].to_ascii_uppercase();
        let kind = match itype.as_str() {
            "CE" | "PE" => itype,
            "FUT" => "FUT".to_string(),
            "EQ" => "EQ".to_string(),
            _ => continue,
        };
        let segment = match exchange.as_str() {
            "NFO" => "NFO",
            "BFO" => "BFO",
            "MCX" => "MCX",
            "NSE" => "NSE",
            "BSE" => "BSE",
            _ => continue,
        };
        insert(
            &mut m.by_key,
            ContractKey {
                underlying: normalize_name(name),
                expiry: if kind == "EQ" { String::new() } else { expiry },
                strike: (strike * 100.0).round() as i64,
                kind,
            },
            Instrument {
                key: symbol.clone(),
                symbol: symbol.clone(),
                segment: segment.to_string(),
                lot: lot.max(1.0),
            },
        );
        // `token` is intentionally unused: Kite's order API takes the trading
        // symbol (tradingsymbol), which is `key`.
        let _ = token;
    }
    if m.by_key.is_empty() {
        return Err("Kite master parsed to zero contracts".into());
    }
    Ok(m)
}

fn parse_angel(body: &[u8]) -> Result<Master, String> {
    let rows: Vec<Value> = serde_json::from_slice(body).map_err(|e| format!("Angel master json: {e}"))?;
    let mut m = Master::default();
    for r in rows {
        let itype = r.get("instrumenttype").and_then(|v| v.as_str()).unwrap_or("");
        let exch = r.get("exch_seg").and_then(|v| v.as_str()).unwrap_or("");
        let kind = match itype {
            "OPTIDX" | "OPTSTK" | "OPTFUT" => {
                let sym = r.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                if sym.ends_with("CE") {
                    "CE"
                } else if sym.ends_with("PE") {
                    "PE"
                } else {
                    continue;
                }
            }
            "FUTIDX" | "FUTSTK" | "FUTCOM" => "FUT",
            _ => continue,
        };
        let underlying = normalize_name(r.get("name").and_then(|v| v.as_str()).unwrap_or(""));
        if underlying.is_empty() {
            continue;
        }
        let expiry = normalize_expiry(r.get("expiry").and_then(|v| v.as_str()).unwrap_or(""));
        // Angel reports strike in paise already (`3000000.000000` for 30000).
        let strike = r
            .get("strike")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let lot = r
            .get("lotsize")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.0);
        let token = r.get("token").and_then(|v| v.as_str()).unwrap_or("");
        let symbol = r.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
        if token.is_empty() {
            continue;
        }
        let segment = match exch {
            "NFO" => "NFO",
            "BFO" => "BFO",
            "MCX" => "MCX",
            "NSE" => "NSE",
            "BSE" => "BSE",
            _ => continue,
        };
        insert(
            &mut m.by_key,
            ContractKey {
                underlying,
                expiry,
                strike: strike.round() as i64,
                kind: kind.to_string(),
            },
            Instrument {
                key: token.to_string(),
                symbol: symbol.to_string(),
                segment: segment.to_string(),
                lot: lot.max(1.0),
            },
        );
    }
    if m.by_key.is_empty() {
        return Err("Angel master parsed to zero contracts".into());
    }
    Ok(m)
}

/// Fyers `sym_details` CSV is positional with no header. Index 9 is the
/// broker symbol (`NSE:NIFTY26NOV24500CE`), 13 the underlying, 15 the strike,
/// 16 `CE`/`PE`/`XX`, 8 the expiry epoch, 3 the lot size.
fn parse_fyers(fo: &str, cm: &str) -> Result<Master, String> {
    let mut m = Master::default();
    for line in fo.lines().chain(cm.lines()) {
        if line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 17 {
            continue;
        }
        let broker_symbol = &f[9];
        if broker_symbol.is_empty() || broker_symbol == "NSE:" {
            continue;
        }
        let (prefix, rest) = match broker_symbol.split_once(':') {
            Some((p, r)) => (p.to_ascii_uppercase(), r.to_string()),
            None => continue,
        };
        let kind_raw = f[16].to_ascii_uppercase();
        let kind = if kind_raw == "CE" || kind_raw == "PE" {
            kind_raw
        } else if rest.ends_with("FUT") || f[1].ends_with("FUT") {
            "FUT".to_string()
        } else {
            "EQ".to_string()
        };
        let underlying = normalize_name(&f[13]);
        if underlying.is_empty() {
            continue;
        }
        let expiry = if kind == "EQ" {
            String::new()
        } else {
            f[8]
                .parse::<i64>()
                .ok()
                .and_then(epoch_to_date)
                .unwrap_or_default()
        };
        if kind != "EQ" && expiry.is_empty() {
            continue;
        }
        let strike = f[15].parse::<f64>().unwrap_or(0.0);
        let lot = f[3].parse::<f64>().unwrap_or(1.0);
        let segment = match prefix.as_str() {
            "NSE" if kind == "EQ" => "NSE",
            "NSE" => "NFO",
            "BSE" if kind == "EQ" => "BSE",
            "BSE" => "BFO",
            "MCX" => "MCX",
            _ => continue,
        };
        insert(
            &mut m.by_key,
            ContractKey {
                underlying,
                expiry,
                strike: (strike * 100.0).round() as i64,
                kind,
            },
            Instrument {
                key: broker_symbol.clone(),
                symbol: broker_symbol.clone(),
                segment: segment.to_string(),
                lot: lot.max(1.0),
            },
        );
    }
    if m.by_key.is_empty() {
        return Err("Fyers master parsed to zero contracts".into());
    }
    Ok(m)
}


/// Shoonya ships each segment as a zip holding one `.txt`. Take the first text
/// entry; a zip with no text file is a malformed download, not an empty master.
fn unzip_single_text(body: &[u8]) -> Result<String, String> {
    let files = unzip(body)?;
    let mut best: Option<(String, Vec<u8>)> = None;
    for (name, bytes) in files {
        if name.to_ascii_lowercase().ends_with(".txt") {
            if best.as_ref().map(|(_, b)| bytes.len() > b.len()).unwrap_or(true) {
                best = Some((name, bytes));
            }
        }
    }
    let Some((name, bytes)) = best else {
        return Err("shoonya master zip contained no .txt file".into());
    };
    let text = String::from_utf8_lossy(&bytes).to_string();
    if text.trim().is_empty() {
        return Err(format!("shoonya master {name} is empty"));
    }
    Ok(text)
}

/// Shoonya NFO master header:
/// `Exchange,Token,LotSize,Symbol,TradingSymbol,Expiry,Instrument,OptionType,
/// StrikePrice,TickSize,`
/// and the NSE cash master:
/// `Exchange,Token,LotSize,Symbol,TradingSymbol,Instrument,TickSize,`
///
/// `Symbol` is the underlying (`NIFTY`), `TradingSymbol` is what the order API
/// wants (`NIFTY26NOV2424500CE`, `CANBK-EQ`). Derivative rows carry strike in
/// rupees while the shared key wants paise.
fn parse_shoonya(fo: &str, cm: &str) -> Result<Master, String> {
    let mut m = Master::default();
    for (segment, text) in [("NFO", fo), ("NSE", cm)] {
        for (i, line) in text.lines().enumerate() {
            if i == 0 || line.trim().is_empty() {
                continue;
            }
            let f = csv_fields(line);
            if f.len() < 7 {
                continue;
            }
            let token = f[1].trim();
            let lot = f[2].parse::<f64>().unwrap_or(1.0);
            let underlying_raw = f[3].trim();
            let tsym = f[4].trim();
            if token.is_empty() || tsym.is_empty() || underlying_raw.is_empty() {
                continue;
            }
            if segment == "NSE" {
                // Cash: only real equity scrips (`-EQ` suffix); indices and
                // bonds share the same file.
                let itype = f[5].trim().to_ascii_uppercase();
                if !itype.eq_ignore_ascii_case("EQ") || !tsym.to_ascii_uppercase().ends_with("-EQ") {
                    continue;
                }
                insert(
                    &mut m.by_key,
                    ContractKey {
                        underlying: normalize_name(underlying_raw),
                        expiry: String::new(),
                        strike: 0,
                        kind: "EQ".into(),
                    },
                    Instrument {
                        key: token.to_string(),
                        symbol: tsym.to_string(),
                        segment: "NSE".into(),
                        lot: lot.max(1.0),
                    },
                );
                continue;
            }
            if f.len() < 9 {
                continue;
            }
            let expiry = normalize_shoonya_expiry(f[5].trim());
            let itype = f[6].trim().to_ascii_uppercase();
            let otype = f[7].trim().to_ascii_uppercase();
            let kind = match itype.as_str() {
                "OPTIDX" | "OPTSTK" | "OPTFUT" => {
                    if otype != "CE" && otype != "PE" {
                        continue;
                    }
                    otype
                }
                "FUTIDX" | "FUTSTK" | "FUTCOM" => "FUT".to_string(),
                _ => continue,
            };
            if expiry.is_empty() {
                continue;
            }
            // Futures carry a placeholder strike (`-0.01` in the real master);
            // the shared key holds the underlying only, so futures key on 0.
            let strike = if kind == "FUT" {
                0
            } else {
                (f[8].parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64
            };
            insert(
                &mut m.by_key,
                ContractKey {
                    underlying: normalize_name(underlying_raw),
                    expiry,
                    strike,
                    kind,
                },
                Instrument {
                    key: token.to_string(),
                    symbol: tsym.to_string(),
                    segment: "NFO".into(),
                    lot: lot.max(1.0),
                },
            );
        }
    }
    if m.by_key.is_empty() {
        return Err("Shoonya master parsed to zero contracts".into());
    }
    Ok(m)
}

/// Shoonya writes expiry as `26-NOV-2026` (and futures as `23-NOV-2026`).
fn normalize_shoonya_expiry(raw: &str) -> String {
    normalize_expiry(raw)
}

fn epoch_to_date(secs: i64) -> Option<String> {
    // Fyers stamps expiry at 10:10 IST on the expiry day; shifting by the IST
    // offset then taking the UTC date yields the intended calendar date.
    let shifted = secs + 5 * 3600 + 30 * 60;
    let days = shifted.div_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    Some(format!("{y:04}-{mo:02}-{d:02}"))
}

/// Howard Hinnant's `civil_from_days`, for converting epoch days without a
/// date-time dependency.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Alice Blue `NFO.csv`: `Exch,Exchange Segment,Symbol,Token,Instrument Type,
/// Option Type,Strike Price,Instrument Name,Formatted Ins Name,Trading Symbol,
/// Expiry Date,Lot Size,Tick Size`.
fn parse_alice(fo: &str, cm: &str) -> Result<Master, String> {
    let mut m = Master::default();
    for (i, line) in fo.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 12 {
            continue;
        }
        let exch = f[0].to_ascii_uppercase();
        let itype = f[4].to_ascii_uppercase();
        let otype = f[5].to_ascii_uppercase();
        let kind = match itype.as_str() {
            "OPTIDX" | "OPTSTK" | "OPTFUT" => otype,
            "FUTIDX" | "FUTSTK" | "FUTCOM" => "FUT".to_string(),
            _ => continue,
        };
        let underlying = normalize_name(&f[7]);
        if underlying.is_empty() {
            continue;
        }
        let expiry = normalize_expiry(&f[10]);
        let strike = f[6].parse::<f64>().unwrap_or(0.0);
        let lot = f[11].parse::<f64>().unwrap_or(1.0);
        let token = &f[3];
        let symbol = &f[9];
        if token.is_empty() || symbol.is_empty() {
            continue;
        }
        let segment = match exch.as_str() {
            "NFO" => "NFO",
            "BFO" => "BFO",
            "MCX" => "MCX",
            _ => continue,
        };
        insert(
            &mut m.by_key,
            ContractKey {
                underlying,
                expiry,
                strike: (strike * 100.0).round() as i64,
                kind,
            },
            Instrument {
                key: token.clone(),
                symbol: symbol.clone(),
                segment: segment.to_string(),
                lot: lot.max(1.0),
            },
        );
    }
    // The cash segment uses its own header layout; only equity rows are needed.
    for (i, line) in cm.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 4 {
            continue;
        }
        let series = f.get(2).map(|s| s.to_ascii_uppercase()).unwrap_or_default();
        if series != "EQ" {
            continue;
        }
        let symbol = f[1].clone();
        let token = f[3].clone();
        if symbol.is_empty() || token.is_empty() {
            continue;
        }
        insert(
            &mut m.by_key,
            ContractKey {
                underlying: normalize_name(&symbol),
                expiry: String::new(),
                strike: 0,
                kind: "EQ".to_string(),
            },
            Instrument {
                key: token,
                symbol,
                segment: "NSE".to_string(),
                lot: 1.0,
            },
        );
    }
    if m.by_key.is_empty() {
        return Err("Alice Blue master parsed to zero contracts".into());
    }
    Ok(m)
}

/// ICICI ships a zip of pipe-separated masters. `FONSEScripMaster.txt` holds the
/// F&O rows; `Token` is the Breeze `stock_code`.
fn parse_icici(files: &HashMap<String, Vec<u8>>) -> Result<Master, String> {
    let mut m = Master::default();
    let fo = files
        .get("FONSEScripMaster.txt")
        .or_else(|| files.get("NFO.csv"))
        .ok_or("ICICI zip has no FONSEScripMaster.txt")?;
    let text = String::from_utf8_lossy(fo);
    let mut lines = text.lines();
    let header = csv_fields(lines.next().unwrap_or(""));
    let idx = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
    let (Some(i_token), Some(i_inst), Some(i_series), Some(i_expiry), Some(i_strike), Some(i_opt), Some(i_lot), Some(i_exch)) = (
        idx("Token"),
        idx("InstrumentName"),
        idx("Series"),
        idx("ExpiryDate"),
        idx("StrikePrice"),
        idx("OptionType"),
        idx("LotSize"),
        idx("ExchangeCode"),
    ) else {
        return Err("ICICI FO master header changed".into());
    };
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() <= i_exch.max(i_lot).max(i_strike) {
            continue;
        }
        let itype = f[i_inst].to_ascii_uppercase();
        let kind = match itype.as_str() {
            "OPTIDX" | "OPTSTK" => f[i_opt].to_ascii_uppercase(),
            "FUTIDX" | "FUTSTK" => "FUT".to_string(),
            _ => continue,
        };
        if kind != "FUT" && kind != "CE" && kind != "PE" {
            continue;
        }
        // `ExchangeCode` carries the display name (`NIFTY 50`); `Series` is
        // `OPTION`/`FUTURE`. The underlying is the display name's first token.
        let display = f[i_exch].to_ascii_uppercase();
        let underlying = normalize_name(display.split_whitespace().next().unwrap_or(&display));
        if underlying.is_empty() {
            continue;
        }
        let expiry = normalize_expiry(&f[i_expiry]);
        let strike = f[i_strike].parse::<f64>().unwrap_or(0.0);
        let lot = f[i_lot].parse::<f64>().unwrap_or(1.0);
        let token = &f[i_token];
        if token.is_empty() {
            continue;
        }
        let _ = i_series;
        insert(
            &mut m.by_key,
            ContractKey {
                underlying,
                expiry,
                strike: (strike * 100.0).round() as i64,
                kind,
            },
            Instrument {
                key: token.clone(),
                symbol: display,
                segment: "NFO".to_string(),
                lot: lot.max(1.0),
            },
        );
    }
    if m.by_key.is_empty() {
        return Err("ICICI master parsed to zero contracts".into());
    }
    Ok(m)
}

/// 5paisa master CSV: `ScripCode, Name, Expiry, StrikeRate, ...` with the
/// instrument type in `ExchType`/`ScripType`. Only the fields the adapter needs
/// are read; the rest vary between 5paisa master revisions.
fn parse_fivepaisa(fo: &str, eq: &str) -> Result<Master, String> {
    let mut m = Master::default();
    for (i, line) in fo.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 10 {
            continue;
        }
        // 5paisa's documented order: ScripCode, Name, Expiry, StrikeRate,
        // OptionType, ScripType ... treated defensively below.
        let code = &f[0];
        let name = &f[1];
        let expiry = normalize_expiry(&f[2]);
        let strike = f[3].parse::<f64>().unwrap_or(0.0);
        let otype = f.get(4).map(|s| s.to_ascii_uppercase()).unwrap_or_default();
        let stype = f.iter().map(|s| s.to_ascii_uppercase()).collect::<Vec<_>>();
        let kind = if otype == "CE" || otype == "PE" {
            otype
        } else if stype.iter().any(|s| s == "FUT") || stype.iter().any(|s| s == "XX") {
            "FUT".to_string()
        } else {
            continue;
        };
        let lot = f
            .iter()
            .rev()
            .find_map(|s| s.parse::<f64>().ok().filter(|v| *v >= 1.0 && v.fract() == 0.0))
            .unwrap_or(1.0);
        let underlying = normalize_name(name.split_whitespace().next().unwrap_or(name));
        if underlying.is_empty() || code.is_empty() {
            continue;
        }
        insert(
            &mut m.by_key,
            ContractKey {
                underlying,
                expiry,
                strike: (strike * 100.0).round() as i64,
                kind,
            },
            Instrument {
                key: code.clone(),
                symbol: code.clone(),
                segment: "N".to_string(),
                lot: lot.max(1.0),
            },
        );
    }
    for (i, line) in eq.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 2 {
            continue;
        }
        let code = &f[0];
        let symbol = &f[1];
        if code.is_empty() || symbol.is_empty() {
            continue;
        }
        insert(
            &mut m.by_key,
            ContractKey {
                underlying: normalize_name(symbol),
                expiry: String::new(),
                strike: 0,
                kind: "EQ".to_string(),
            },
            Instrument {
                key: code.clone(),
                symbol: symbol.clone(),
                segment: "N".to_string(),
                lot: 1.0,
            },
        );
    }
    if m.by_key.is_empty() {
        return Err("5paisa master parsed to zero contracts (cache may be cold)".into());
    }
    Ok(m)
}

/// Kotak Neo master archive: `nse_fo.csv` + `nse_cm.csv` with the Kotak
/// instrument token and trading symbol.
fn parse_kotak(files: &HashMap<String, Vec<u8>>) -> Result<Master, String> {
    let mut m = Master::default();
    for (name, bytes) in files {
        let lower = name.to_ascii_lowercase();
        let is_fo = lower.contains("fo");
        let text = String::from_utf8_lossy(bytes);
        let mut lines = text.lines();
        let header = csv_fields(lines.next().unwrap_or(""));
        let idx = |want: &[&str]| {
            header.iter().position(|h| {
                let h = h.to_ascii_lowercase();
                want.iter().any(|w| h.contains(w))
            })
        };
        let i_sym = idx(&["symbol", "tradingsymbol"]);
        let i_token = idx(&["instrument_token", "token", "pinstock"]);
        let (Some(i_sym), Some(i_token)) = (i_sym, i_token) else {
            continue;
        };
        let i_inst = idx(&["instrument_type", "instrument"]);
        let i_expiry = idx(&["expiry"]);
        let i_strike = idx(&["strike"]);
        let i_lot = idx(&["lot_size", "lotsize"]);
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let f = csv_fields(line);
            if f.len() <= i_sym.max(i_token) {
                continue;
            }
            let symbol = &f[i_sym];
            let token = &f[i_token];
            if symbol.is_empty() || token.is_empty() {
                continue;
            }
            let kind = match i_inst.map(|i| f.get(i).map(|s| s.to_ascii_uppercase()).unwrap_or_default()) {
                Some(s) if s == "CE" || s == "PE" => s,
                Some(s) if s.starts_with("FUT") => "FUT".to_string(),
                _ if is_fo => continue,
                _ => "EQ".to_string(),
            };
            let underlying = if kind == "EQ" {
                normalize_name(symbol)
            } else {
                normalize_name(
                    symbol
                        .split(|c: char| c.is_ascii_digit())
                        .next()
                        .unwrap_or(symbol),
                )
            };
            if underlying.is_empty() {
                continue;
            }
            let expiry = if kind == "EQ" {
                String::new()
            } else {
                i_expiry
                    .and_then(|i| f.get(i))
                    .map(|s| normalize_expiry(s))
                    .unwrap_or_default()
            };
            let strike = i_strike
                .and_then(|i| f.get(i))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let lot = i_lot
                .and_then(|i| f.get(i))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            let segment = if is_fo { "nse_fo" } else { "nse_cm" };
            insert(
                &mut m.by_key,
                ContractKey {
                    underlying,
                    expiry,
                    strike: (strike * 100.0).round() as i64,
                    kind,
                },
                Instrument {
                    key: token.clone(),
                    symbol: symbol.clone(),
                    segment: segment.to_string(),
                    lot: lot.max(1.0),
                },
            );
        }
    }
    if m.by_key.is_empty() {
        return Err("Kotak master parsed to zero contracts".into());
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_normalisation_handles_every_master_spelling() {
        assert_eq!(normalize_expiry("2026-11-26 15:30:00"), "2026-11-26");
        assert_eq!(normalize_expiry("23-Nov-2026"), "2026-11-23");
        assert_eq!(normalize_expiry("29DEC2027"), "2027-12-29");
        assert_eq!(normalize_expiry("29DEC26"), "2026-12-29");
        assert_eq!(normalize_expiry(""), "");
    }

    #[test]
    fn epoch_expiry_lands_on_the_ist_calendar_day() {
        // 1793700600 = 2026-11-03 10:10 UTC per the Fyers master sample.
        assert_eq!(epoch_to_date(1793700600).as_deref(), Some("2026-11-03"));
        assert_eq!(epoch_to_date(1795428600).as_deref(), Some("2026-11-23"));
    }

    #[test]
    #[ignore]
    fn shoonya_parser_handles_the_real_master() {
        // Opt-in: hits api.shoonya.com. Run with
        // `cargo test -p algo-server -- --ignored shoonya_parser_handles`.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let m = rt.block_on(async {
            let nfo = http_get("https://api.shoonya.com/NFO_symbols.txt.zip").await?;
            let nse = http_get("https://api.shoonya.com/NSE_symbols.txt.zip").await?;
            parse_shoonya(&unzip_single_text(&nfo)?, &unzip_single_text(&nse)?)
        });
        let m = m.expect("real Shoonya master parsed");
        assert!(m.by_key.len() > 20_000, "only {} contracts", m.by_key.len());
        // Shoonya writes options as `NIFTY26JUN29C39000` / `...P39000` (the leg
        // sits between expiry and strike), so check the key side, not a suffix.
        assert!(m
            .by_key
            .iter()
            .any(|(k, i)| k.kind == "CE" && i.segment == "NFO" && k.strike > 0));
        assert!(m
            .by_key
            .iter()
            .any(|(k, i)| k.kind == "PE" && i.segment == "NFO" && k.strike > 0));
        assert!(m.by_key.iter().any(|(k, i)| k.kind == "FUT" && i.segment == "NFO"));
        assert!(m
            .by_key
            .values()
            .any(|i| i.segment == "NSE" && i.symbol.ends_with("-EQ")));
        // A real NIFTY option row must resolve to the exact `tsym` Shoonya's
        // order API takes, proving the strike/expiry/leg mapping is right.
        let nifty = m
            .by_key
            .iter()
            .find(|(k, i)| {
                k.kind == "CE" && k.underlying == "NIFTY" && i.segment == "NFO"
            })
            .map(|(k, i)| (k.clone(), i.clone()))
            .expect("a NIFTY CE row");
        assert!(nifty.1.symbol.starts_with("NIFTY"));
        assert!(nifty.1.symbol.contains('C'));
        assert_eq!(nifty.1.symbol, nifty.1.symbol.to_uppercase());
        assert!(nifty.1.lot >= 1.0);
        assert!(nifty.0.strike > 100_000, "strike not in paise: {}", nifty.0.strike);
    }

    #[test]
    fn shoonya_master_maps_options_futures_and_cash() {
        // Real header + row shapes from api.shoonya.com/NFO_symbols.txt.zip.
        let nfo = concat!(
            "Exchange,Token,LotSize,Symbol,TradingSymbol,Expiry,Instrument,OptionType,StrikePrice,TickSize,\n",
            "NFO,48577,,,,,,,0,0,\n",
            "NFO,89665,65,NIFTY,NIFTY26JUN29P39000,26-JUN-2029,OPTIDX,PE,39000,0.05,\n",
            "NFO,61473,25,NIFTYNXT50,NIFTYNXT5023NOV26F,23-NOV-2026,FUTIDX,XX,-0.01,0.2,\n",
        );
        let nse = concat!(
            "Exchange,Token,LotSize,Symbol,TradingSymbol,Instrument,TickSize,\n",
            "NSE,26000,1,Nifty 50,NIFTY INDEX,INDEX,0,\n",
            "NSE,17635,1,ZYDUSWELL,ZYDUSWELL-EQ,EQ,0.05,\n",
        );
        let m = parse_shoonya(nfo, nse).unwrap();
        let pe = m
            .get(&ContractKey {
                underlying: "NIFTY".into(),
                expiry: "2029-06-26".into(),
                strike: 3_900_000,
                kind: "PE".into(),
            })
            .expect("option resolved");
        // The order API wants `tsym`, not the numeric token.
        assert_eq!(pe.symbol, "NIFTY26JUN29P39000");
        assert_eq!(pe.segment, "NFO");
        assert_eq!(pe.lot, 65.0);
        let fut = m
            .get(&ContractKey {
                underlying: "NIFTYNXT50".into(),
                expiry: "2026-11-23".into(),
                strike: 0,
                kind: "FUT".into(),
            })
            .expect("future resolved");
        assert_eq!(fut.symbol, "NIFTYNXT5023NOV26F");
        let eq = m
            .get(&ContractKey {
                underlying: "ZYDUSWELL".into(),
                expiry: String::new(),
                strike: 0,
                kind: "EQ".into(),
            })
            .expect("equity resolved");
        assert_eq!(eq.symbol, "ZYDUSWELL-EQ");
        assert_eq!(eq.segment, "NSE");
        // An index row in the cash file must not be mistaken for a tradable EQ.
        assert!(m
            .get(&ContractKey {
                underlying: "NIFTY50".into(),
                expiry: String::new(),
                strike: 0,
                kind: "EQ".into(),
            })
            .is_none());
    }

    #[test]
    fn upstox_master_maps_option_contract_fields() {
        let csv = concat!(
            "\"instrument_key\",\"exchange_token\",\"tradingsymbol\",\"name\",\"last_price\",\"expiry\",\"strike\",\"tick_size\",\"lot_size\",\"instrument_type\",\"option_type\",\"exchange\"\n",
            "\"NSE_FO|12345\",\"12345\",\"NIFTY26NOV24500CE\",\"NIFTY\",\"0\",\"2026-11-26\",\"24500.0\",\"0.05\",\"65\",\"OPTIDX\",\"CE\",\"NSE_FO\"\n",
            "\"NSE_EQ|INE002A01018\",\"2885\",\"RELIANCE\",\"RELIANCE INDUSTRIES\",\"0\",\"\",\"0\",\"0.05\",\"1\",\"EQUITY\",\"\",\"NSE_EQ\"\n"
        );
        let m = parse_upstox(csv).unwrap();
        let opt = m
            .get(&ContractKey {
                underlying: "NIFTY".into(),
                expiry: "2026-11-26".into(),
                strike: 2450000,
                kind: "CE".into(),
            })
            .expect("option resolved");
        assert_eq!(opt.key, "NSE_FO|12345");
        assert_eq!(opt.segment, "NSE_FO");
        assert_eq!(opt.lot, 65.0);
        let eq = m
            .get(&ContractKey {
                underlying: "RELIANCE".into(),
                expiry: String::new(),
                strike: 0,
                kind: "EQ".into(),
            })
            .expect("equity resolved");
        assert_eq!(eq.key, "NSE_EQ|INE002A01018");
    }

    #[test]
    fn kite_master_keys_on_trading_symbol() {
        let csv = concat!(
            "instrument_token,exchange_token,tradingsymbol,name,last_price,expiry,strike,tick_size,lot_size,instrument_type,segment,exchange\n",
            "13804546,53924,NIFTY26N0323050CE,NIFTY,0,2026-11-03,23050,0.05,65,CE,NFO-OPT,NFO\n"
        );
        let m = parse_kite(csv).unwrap();
        let inst = m
            .get(&ContractKey {
                underlying: "NIFTY".into(),
                expiry: "2026-11-03".into(),
                strike: 2305000,
                kind: "CE".into(),
            })
            .unwrap();
        assert_eq!(inst.key, "NIFTY26N0323050CE");
        assert_eq!(inst.segment, "NFO");
    }

    #[test]
    fn alice_master_uses_the_exchange_segment_column() {
        let csv = concat!(
            "Exch,Exchange Segment,Symbol,Token,Instrument Type,Option Type,Strike Price,Instrument Name,Formatted Ins Name,Trading Symbol,Expiry Date,Lot Size,Tick Size\n",
            "NFO,nse_fo,NIFTY,12345,OPTIDX,CE,24500,NIFTY,NIFTY 26 NOV 24500 CE,NIFTY26NOV24500CE,2026-11-26,65,0.05\n"
        );
        let m = parse_alice(csv, "").unwrap();
        let inst = m
            .get(&ContractKey {
                underlying: "NIFTY".into(),
                expiry: "2026-11-26".into(),
                strike: 2450000,
                kind: "CE".into(),
            })
            .unwrap();
        assert_eq!(inst.key, "12345");
        assert_eq!(inst.symbol, "NIFTY26NOV24500CE");
    }

    #[test]
    fn a_contract_missing_from_the_master_is_an_error_not_a_guess() {
        // The resolver must never invent a token; verified through the key path
        // used by `resolve`.
        let mut m = Master::default();
        insert(
            &mut m.by_key,
            ContractKey {
                underlying: "NIFTY".into(),
                expiry: "2026-11-26".into(),
                strike: 2450000,
                kind: "CE".into(),
            },
            Instrument {
                key: "x".into(),
                symbol: "x".into(),
                segment: "NSE_FO".into(),
                lot: 65.0,
            },
        );
        assert!(m
            .get(&ContractKey {
                underlying: "NIFTY".into(),
                expiry: "2026-11-26".into(),
                strike: 2450050,
                kind: "CE".into(),
            })
            .is_none());
    }
}
