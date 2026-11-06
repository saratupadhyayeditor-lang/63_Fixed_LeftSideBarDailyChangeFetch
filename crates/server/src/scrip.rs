//! Dhan scrip-master: downloads and parses the exchange contract master so
//! option security ids, expiry ladders, lot sizes and exact trading symbols
//! resolve locally, without a broker session.
//!
//! Mirrors the old Flask app's `_get_scrip_master` / `_build_scrip_lookups` /
//! `_resolve_option_security` / `_oc_bucket` / `_scrip_expiries`. The CSV is
//! ~26 MB, so it is cached on disk for an hour (same `/tmp` path the old app
//! used) and served from an in-memory `Arc` afterwards.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use algo_core::option as bs;

const SCRIP_URL: &str = "https://images.dhan.co/api-data/api-scrip-master.csv";
const SCRIP_TTL: Duration = Duration::from_secs(3600);

/// `0` = still warming, `1` = ready, `2` = warm failed (never coming).
const PENDING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

static STATUS: AtomicU8 = AtomicU8::new(PENDING);

/// CE/PE legs of one strike: `(security_id, trading_symbol)`.
#[derive(Clone, Debug, Default)]
pub struct StrikeEntry {
    pub lot: f64,
    pub ce: Option<(i64, String)>,
    pub pe: Option<(i64, String)>,
}

/// Fully resolved option contract.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub security_id: i64,
    pub lot: f64,
    pub trading_symbol: String,
    pub expiry: String,
}

/// One futures row (FUTSTK / FUTIDX / FUTCOM) used to resolve a derivative
/// underlying for Dhan's `/optionchain` call (old `_pick_fut_row`) and the
/// authoritative contract lot size / trading symbol (old `_resolve_lot_size`).
#[derive(Clone, Debug)]
pub struct FutRow {
    pub exchange: String,
    pub expiry: String,
    pub security_id: i64,
    pub lot: f64,
    pub trading_symbol: String,
}

/// One tradeable MCX near-month commodity future, derived live from the Dhan
/// scrip master (old app built this from `FUTCOM` rows; contract ids roll every
/// expiry so a static list would never place a real order).
#[derive(Clone, Debug)]
pub struct CommodityRow {
    pub name: String,
    pub trading_symbol: String,
    pub security_id: i64,
    pub lot: f64,
    pub expiry: String,
    pub has_options: bool,
}

/// Curated MCX contract lots (old `_MCX_LOT_BY_PREFIX`); 0 = unknown.
fn mcx_lot_override(trading_symbol: &str) -> f64 {
    let sym = trading_symbol.to_uppercase();
    let prefix = match sym.split('-').next() {
        Some(p) if sym.contains('-') => p,
        _ => return 0.0,
    };
    match prefix {
        "GOLD" => 100.0,
        "GOLDM" => 10.0,
        "GOLDGUINEA" => 0.8,
        "GOLDPETAL" => 0.1,
        "GOLDTEN" => 1.0,
        "SILVER" => 30.0,
        "SILVERM" => 5.0,
        "SILVERMIC" => 1.0,
        "SILVER100" => 100.0,
        "CRUDEOIL" => 100.0,
        "CRUDEOILM" => 10.0,
        "NATURALGAS" => 250.0,
        "NATGASMINI" => 125.0,
        "COPPER" => 2500.0,
        "ALUMINIUM" => 5000.0,
        "ALUMINI" => 1000.0,
        "LEAD" => 5000.0,
        "LEADMINI" => 1000.0,
        "ZINC" => 5000.0,
        "ZINCMINI" => 1000.0,
        "NICKEL" => 250.0,
        "MENTHAOIL" => 960.0,
        "COTTON" => 25.0,
        "COTTONOIL" => 10000.0,
        "KAPAS" => 1.0,
        "CARDAMOM" => 120.0,
        "STEELREBAR" => 10000.0,
        "ELECDMBL" => 10000.0,
        _ => 0.0,
    }
}

/// One resolved contract, independent of any broker's own symbol scheme.
///
/// The engines speak Dhan `security_id`; every non-Dhan adapter has to translate
/// that identity into its own instrument key/token. Keeping the full row here
/// (underlying, expiry, strike, option type, exchange, lot) means an adapter can
/// look up its own master by contract fields instead of guessing from a
/// broker-specific trading symbol.
#[derive(Clone, Debug)]
pub struct Contract {
    /// Dhan security id (the engine's identity for this contract).
    pub security_id: i64,
    /// `NSE` / `BSE` / `MCX` (scrip-master exchange id).
    pub exchange: String,
    /// Raw scrip-master instrument name: `EQUITY`, `OPTIDX`, `OPTSTK`, `OPTFUT`,
    /// `FUTIDX`, `FUTSTK`, `FUTCOM`.
    pub instrument: String,
    /// F&O underlying prefix (e.g. `NIFTY`) or the cash symbol for equities.
    pub underlying: String,
    /// `YYYY-MM-DD` expiry (empty for cash equity).
    pub expiry: String,
    pub strike: f64,
    /// `CE` / `PE` / empty for futures + equity.
    pub option_type: String,
    pub lot: f64,
    /// Dhan trading symbol (kept for logging / fallback matching).
    pub trading_symbol: String,
}

/// Parsed scrip master. Only OPT*/FUT* rows plus the NSE cash-equity id map are
/// retained (the rest is never looked up), which keeps the footprint small.
#[derive(Default)]
pub struct Scrip {
    /// Every resolved contract keyed by Dhan security id, used by the non-Dhan
    /// order adapters to translate a position's contract into their own token.
    contracts: HashMap<i64, Contract>,
    /// `(exchange, prefix, expiry_date)` -> `strike_cents` -> legs.
    oc: HashMap<(String, String, String), BTreeMap<i64, StrikeEntry>>,
    /// `(exchange, prefix)` -> sorted expiries that carry option strikes.
    expiries: HashMap<(String, String), Vec<String>>,
    /// `(exchange, prefix)` -> sorted expiries for genuine **stock** options
    /// (`OPTSTK`) only. Kept separate from `expiries` because the latter also
    /// holds index options (`OPTIDX`) and commodity options-on-futures
    /// (`OPTFUT`, e.g. NSE CRUDEOIL/GOLD/SILVERM/ZINC/COPPER/NATURALGAS): a
    /// commodity OPTFUT expiry must never be mistaken for a stock F&O expiry by
    /// the Realtime / Paper Trade 1 expiry-day guard.
    stock_expiries: HashMap<(String, String), Vec<String>>,
    /// `(prefix, instrument)` -> sorted futures expiries (fallback ladder for
    /// commodities whose OPTFUT coverage is partial).
    fo_expiries: HashMap<(String, String), Vec<String>>,
    /// `(prefix, instrument)` -> futures rows (exchange / expiry / security id)
    /// so an equity spot can be mapped to its derivative underlying.
    fo: HashMap<(String, String), Vec<FutRow>>,
    /// Every OPTIDX / OPTSTK / OPTFUT security id. Lets the quote backfill tell
    /// an option apart from a future by id alone (scanner-discovered options are
    /// never registered in the option-chain map), so it never fetches an option
    /// with a futures instrument type and corrupts its previous close / LTP.
    opt_ids: HashSet<i64>,
    /// NSE cash-equity symbol -> Dhan security id (series EQ only). Used by the
    /// equity-research module to locate a stock's option chain via
    /// `resolve_underlying`.
    eq_sid: HashMap<String, i64>,
}

static SCRIP: OnceLock<Arc<Scrip>> = OnceLock::new();

/// The loaded scrip master, or `None` while warming / after a failed warm.
pub fn get() -> Option<Arc<Scrip>> {
    SCRIP.get().cloned()
}

/// True until the warm task finishes (success or permanent failure). Used by
/// `/api/option_chain` to answer `status:"loading"` instead of guessing.
pub fn pending() -> bool {
    STATUS.load(Ordering::Relaxed) == PENDING
}

/// Warm-state label for diagnostics: "loading", "ready" or "failed". A failed
/// master load is a silent killer for entries (no security id resolves), so the
/// support bundle must be able to state it explicitly.
pub fn warm_status() -> &'static str {
    match STATUS.load(Ordering::Relaxed) {
        PENDING => "loading",
        READY => "ready",
        _ => "failed",
    }
}

/// Number of resolved contracts in the loaded master (0 while loading/failed).
pub fn contract_count() -> usize {
    get().map(|s| s.contract_ids().len()).unwrap_or(0)
}

pub fn strike_key(strike: f64) -> i64 {
    (strike * 100.0).round() as i64
}

fn normalize_expiry(expiry: &str) -> String {
    expiry.chars().take(10).collect()
}

/// Scrip-master exchange id for an API derivative segment.
pub fn scrip_exch(segment: &str) -> &'static str {
    let u = segment.to_uppercase();
    if u.contains("BSE") {
        "BSE"
    } else if u.contains("NCDEX") {
        "NCDEX"
    } else if u.contains("MCX") {
        "MCX"
    } else {
        "NSE"
    }
}

/// True when an F&O underlying prefix is a cash index (Dhan `OPTIDX`) rather
/// than a single stock (`OPTSTK`). Mirrors the old app's `isIndex()` helper.
/// Sending the wrong instrument type makes Dhan's `/charts/intraday` answer an
/// empty payload or a single junk bar, so every option resolution must get this
/// right.
pub fn is_index_prefix(prefix: &str) -> bool {
    matches!(
        prefix.trim().to_uppercase().as_str(),
        "NIFTY"
            | "BANKNIFTY"
            | "FINNIFTY"
            | "MIDCPNIFTY"
            | "NIFTYNXT50"
            | "SENSEX"
            | "SENSEX50"
            | "BANKEX"
            | "GIFTNIFTY"
    )
}

/// Map a UI symbol name to its F&O trading-symbol prefix (old `_fno_underlying`).
pub fn fno_underlying(symbol_name: &str) -> String {
    let name = symbol_name.trim().to_uppercase();
    match name.as_str() {
        "NIFTY 50" => "NIFTY".to_string(),
        "BANK NIFTY" => "BANKNIFTY".to_string(),
        "FINNIFTY" => "FINNIFTY".to_string(),
        "SENSEX" => "SENSEX".to_string(),
        "MIDCPNIFTY" => "MIDCPNIFTY".to_string(),
        "GIFT NIFTY" => "GIFTNIFTY".to_string(),
        "BAJAJ-AUTO" => "BAJAJ".to_string(),
        "NAM-INDIA" => "NAM".to_string(),
        "TATACOMM" => "TATACOMM".to_string(),
        _ => name.replace(' ', ""),
    }
}

impl Scrip {
    /// Exchange whose scrip-master option ladder actually holds `prefix`,
    /// starting from the segment's default exchange. A cash index (`IDX_I`) maps
    /// to NSE by default, but BSE-only index derivatives (SENSEX, BANKEX, ...)
    /// list their options exclusively on BSE. Binding such a lookup to NSE finds
    /// no bucket and silently resolves no option ("no PE option premium"), so
    /// re-home the prefix onto the exchange that demonstrably lists it. Explicit
    /// derivative segments (BSE_FNO / MCX_COMM / NCD_FNO) always win.
    pub fn fno_exch(&self, segment: &str, prefix: &str) -> &'static str {
        let base = scrip_exch(segment);
        let seg = segment.to_uppercase();
        if seg.contains("BSE") || seg.contains("MCX") || seg.contains("NCDEX") {
            return base;
        }
        let p = prefix.to_uppercase();
        if p.is_empty() {
            return base;
        }
        // Direct ladder lookups (O(1)) instead of `has_options`' full key scan:
        // this runs inside the 100 ms scan loop, once per strategy per tick.
        let lists_options = |x: &str| {
            self.expiries
                .get(&(x.to_string(), p.clone()))
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        };
        if lists_options(base) {
            return base;
        }
        for cand in ["NSE", "BSE", "MCX", "NCDEX"] {
            if cand != base && lists_options(cand) {
                return cand;
            }
        }
        base
    }

    pub fn bucket(&self, exch: &str, prefix: &str, expiry: &str) -> Option<&BTreeMap<i64, StrikeEntry>> {
        let key = (
            exch.to_uppercase(),
            prefix.to_uppercase(),
            normalize_expiry(expiry),
        );
        self.oc.get(&key)
    }

    /// True when `sid` is an option contract (OPTIDX / OPTSTK / OPTFUT) in the
    /// scrip master. Used by the quote backfill so an option is never fetched
    /// with a futures instrument type (which returns the underlying's candles
    /// and corrupts the option's previous close / LTP).
    pub fn is_option(&self, sid: i64) -> bool {
        sid > 0 && self.opt_ids.contains(&sid)
    }

    /// NSE cash-equity Dhan security id for a symbol (series EQ only).
    pub fn equity_sid(&self, symbol: &str) -> Option<i64> {
        self.eq_sid.get(&symbol.trim().to_uppercase()).copied()
    }

    /// Full contract row for a Dhan security id. This is what the non-Dhan order
    /// adapters read: the engines only know the Dhan id, so every other broker's
    /// instrument key is resolved from these contract fields.
    pub fn contract(&self, sid: i64) -> Option<&Contract> {
        self.contracts.get(&sid)
    }

    /// Every security id that has a resolved contract row.
    pub fn contract_ids(&self) -> Vec<i64> {
        self.contracts.keys().copied().collect()
    }

    /// True when any option strikes exist for this prefix on this exchange.
    /// `None` when the master is empty (unverifiable).
    pub fn has_options(&self, prefix: &str, exch: &str) -> Option<bool> {
        if self.expiries.is_empty() {
            return None;
        }
        let p = prefix.to_uppercase();
        Some(
            self.expiries
                .keys()
                .any(|(x, pp)| x == exch && *pp == p),
        )
    }

    /// Local expiry ladder (old `_scrip_expiries`). Falls back to a cross-listed
    /// exchange only when the resolved exchange has no options at all, then to
    /// the futures ladder.
    pub fn expiries_for(&self, prefix: &str, exch: &str) -> Option<Vec<String>> {
        let p = prefix.to_uppercase();
        if let Some(v) = self.expiries.get(&(exch.to_uppercase(), p.clone())) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
        if self.has_options(&p, exch) == Some(true) {
            return None;
        }
        let mut cross: Vec<String> = Vec::new();
        for ((x, pp), v) in &self.expiries {
            if *pp == p && matches!(x.as_str(), "NSE" | "BSE" | "MCX" | "NCDEX") {
                cross.extend(v.iter().cloned());
            }
        }
        cross.sort();
        cross.dedup();
        if !cross.is_empty() {
            return Some(cross);
        }
        let mut out: Vec<String> = Vec::new();
        for ((pp, inst), v) in &self.fo_expiries {
            if *pp == p && matches!(inst.as_str(), "FUTIDX" | "FUTSTK" | "FUTCOM") {
                out.extend(v.iter().cloned());
            }
        }
        out.sort();
        out.dedup();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// True when `day` (epoch-day, IST) is an F&O **stock** option expiry day:
    /// at least one NSE / BSE `OPTSTK` underlying lists that exact expiry.
    /// Indices (`OPTIDX`) and commodities (MCX / NCDEX, or NSE `OPTFUT` like
    /// CRUDEOIL/GOLD) never count. Used by the Realtime / Paper Trade 1
    /// F&O-stock expiry guard, so it short-circuits on the first match and is
    /// cheap enough to call on every engine pass.
    pub fn is_stock_option_expiry_day(&self, day: i64) -> bool {
        self.stock_expiries
            .values()
            .any(|list| list.iter().any(|e| bs::parse_ymd(e) == Some(day)))
    }

    /// Upcoming F&O **stock** option expiry dates (`YYYY-MM-DD`) on or after
    /// `from_day` (epoch-day, IST), nearest first, de-duplicated across every
    /// NSE / BSE `OPTSTK` underlying and capped at `limit`. When `from_day` is
    /// itself an expiry day the first entry is "today" (the current expiry).
    /// Indices and commodities are excluded. Backs the UI expiry-date readout.
    pub fn stock_option_expiries_from(&self, from_day: i64, limit: usize) -> Vec<String> {
        let mut rows: Vec<(i64, String)> = Vec::new();
        for list in self.stock_expiries.values() {
            for e in list {
                if let Some(d) = bs::parse_ymd(e) {
                    if d >= from_day {
                        rows.push((d, e.clone()));
                    }
                }
            }
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        rows.dedup_by(|a, b| a.0 == b.0);
        rows.into_iter().take(limit).map(|(_, s)| s).collect()
    }

    fn leg_of(&self, exch: &str, prefix: &str, expiry: &str, strike: f64, ot: &str) -> Option<(i64, String, f64)> {
        let b = self.bucket(exch, prefix, expiry)?;
        let e = b.get(&strike_key(strike))?;
        let leg = if ot == "CE" { e.ce.clone() } else { e.pe.clone() }?;
        Some((leg.0, leg.1, e.lot))
    }

    fn nearest_expiry(&self, exch: &str, prefix: &str, expiry: &str, strike: f64, ot: &str) -> Option<String> {
        let req_days = bs::parse_ymd(&normalize_expiry(expiry))?;
        let p = prefix.to_uppercase();
        let mut best: Option<(i64, String)> = None;
        for ((x, pp, e), b) in &self.oc {
            if x != exch || *pp != p {
                continue;
            }
            let has = b
                .get(&strike_key(strike))
                .map(|en| {
                    if ot == "CE" {
                        en.ce.is_some()
                    } else {
                        en.pe.is_some()
                    }
                })
                .unwrap_or(false);
            if !has {
                continue;
            }
            if let Some(d) = bs::parse_ymd(e) {
                let diff = (d - req_days).abs();
                if best.as_ref().map(|(bd, _)| diff < *bd).unwrap_or(true) {
                    best = Some((diff, e.clone()));
                }
            }
        }
        best.map(|(_, e)| e)
    }

    /// Resolve an option contract (old `_resolve_option_security`): exact
    /// exchange+prefix+expiry+strike first, then the nearest expiry, then a
    /// cross-listed exchange.
    pub fn resolve(
        &self,
        symbol_name: &str,
        expiry: &str,
        strike: f64,
        option_type: &str,
        segment: &str,
    ) -> Option<Resolved> {
        let prefix = fno_underlying(symbol_name);
        if prefix.is_empty() || strike <= 0.0 {
            return None;
        }
        let ot = option_type.trim().to_uppercase();
        if ot != "CE" && ot != "PE" {
            return None;
        }
        let exp = normalize_expiry(expiry);
        let exch = self.fno_exch(segment, &prefix);

        if let Some((sid, sym, lot)) = self.leg_of(exch, &prefix, &exp, strike, &ot) {
            return Some(Resolved { security_id: sid, lot, trading_symbol: sym, expiry: exp });
        }
        // Relaxed match: nearest expiry on the resolved exchange.
        if let Some(e) = self.nearest_expiry(exch, &prefix, &exp, strike, &ot) {
            if let Some((sid, sym, lot)) = self.leg_of(exch, &prefix, &e, strike, &ot) {
                return Some(Resolved { security_id: sid, lot, trading_symbol: sym, expiry: e });
            }
        }
        // Cross-listed exchange fallback (only when resolved exch has none).
        if self.has_options(&prefix, exch) != Some(true) {
            for cand in ["NSE", "BSE", "MCX", "NCDEX"] {
                if cand == exch {
                    continue;
                }
                if let Some((sid, sym, lot)) = self.leg_of(cand, &prefix, &exp, strike, &ot) {
                    return Some(Resolved { security_id: sid, lot, trading_symbol: sym, expiry: exp });
                }
            }
        }
        None
    }

    fn fo_rows(&self, prefix: &str, instr: &str) -> &[FutRow] {
        self.fo
            .get(&(prefix.to_uppercase(), instr.to_string()))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Old `_pick_fut_row`: exact expiry match, else nearest, else first.
    fn pick_fut_row<'a>(rows: &'a [FutRow], expiry: &str) -> Option<&'a FutRow> {
        if rows.is_empty() {
            return None;
        }
        let exp = normalize_expiry(expiry);
        if !exp.is_empty() {
            if let Some(r) = rows.iter().find(|r| r.expiry == exp) {
                return Some(r);
            }
            if let Some(req) = bs::parse_ymd(&exp) {
                if let Some(r) = rows
                    .iter()
                    .filter_map(|r| bs::parse_ymd(&r.expiry).map(|d| ((d - req).abs(), r)))
                    .min_by_key(|(d, _)| *d)
                    .map(|(_, r)| r)
                {
                    return Some(r);
                }
            }
        }
        rows.first()
    }

    /// Resolve the derivative underlying `(security_id, exchange_segment)` for
    /// Dhan's `/optionchain` call (old `_resolve_fno_underlying`, expiry-aware).
    /// Returns the original id/segment when the scrip master cannot help.
    pub fn resolve_underlying(
        &self,
        symbol_name: &str,
        security_id: i64,
        segment: &str,
        expiry: &str,
    ) -> (i64, String) {
        let seg = segment.to_uppercase();
        if seg == "IDX_I" {
            // Indices whose derivatives trade on BSE (SENSEX) must resolve to
            // their BSE_FNO FUTIDX underlying; NSE indices keep IDX_I.
            let prefix = fno_underlying(symbol_name);
            let rows: Vec<FutRow> = self
                .fo_rows(&prefix, "FUTIDX")
                .iter()
                .filter(|r| r.exchange == "BSE")
                .cloned()
                .collect();
            if let Some(r) = Self::pick_fut_row(&rows, expiry) {
                return (r.security_id, "BSE_FNO".to_string());
            }
            return (security_id, seg);
        }
        if matches!(seg.as_str(), "NSE_FNO" | "BSE_FNO" | "MCX_COMM" | "NCD_FNO") {
            return (security_id, seg);
        }
        // Equity segment: map to the derivative underlying via the scrip master.
        let prefix = fno_underlying(symbol_name);
        let exch = scrip_exch(&seg);
        for instr in ["FUTSTK", "FUTIDX"] {
            let all = self.fo_rows(&prefix, instr);
            if all.is_empty() {
                continue;
            }
            let rows: Vec<FutRow> = if exch.is_empty() {
                all.to_vec()
            } else {
                let seg_rows: Vec<FutRow> =
                    all.iter().filter(|r| r.exchange == exch).cloned().collect();
                if !seg_rows.is_empty() {
                    seg_rows
                } else {
                    all.to_vec()
                }
            };
            if let Some(r) = Self::pick_fut_row(&rows, expiry) {
                let fno_seg = if exch == "BSE" { "BSE_FNO" } else { "NSE_FNO" };
                return (r.security_id, fno_seg.to_string());
            }
        }
        (security_id, seg)
    }

    /// Cold-cache guard (old `_expiry_matches_underlying`): true when the scrip
    /// master carries the requested expiry for the resolved derivative exchange.
    /// Only enforced for F&O equity requests; indices/commodities are exempt.
    pub fn expiry_matches_underlying(&self, symbol_name: &str, segment: &str, expiry: &str) -> bool {
        let seg = segment.to_uppercase();
        if seg != "NSE_EQ" && seg != "BSE_EQ" {
            return true;
        }
        if self.fo.is_empty() {
            return true;
        }
        let prefix = fno_underlying(symbol_name);
        let exch = scrip_exch(&seg);
        if prefix.is_empty() || exch.is_empty() {
            return true;
        }
        let exp = normalize_expiry(expiry);
        for instr in ["FUTSTK", "FUTIDX"] {
            let all = self.fo_rows(&prefix, instr);
            if all.is_empty() {
                continue;
            }
            let rows: Vec<&FutRow> = all.iter().filter(|r| r.exchange == exch).collect();
            let rows: Vec<&FutRow> = if rows.is_empty() {
                all.iter().collect()
            } else {
                rows
            };
            if rows.is_empty() {
                continue;
            }
            if rows.iter().any(|r| r.expiry == exp) {
                return true;
            }
            // Foreign date: reject only when this prefix lists options here.
            return self.has_options(&prefix, exch) != Some(true);
        }
        true
    }

    /// Authoritative contract lot size + trading symbol for a symbol (old
    /// `_resolve_lot_size` underlying-prefix path / `_oc_instrument_meta`).
    /// Futures are canonical; the first row for the exchange is used.
    pub fn lot_for(&self, symbol_name: &str, segment: &str) -> Option<(f64, String)> {
        // Candidate underlying prefixes, most specific first. A plain underlying
        // or UI symbol ("ADANIGREEN") resolves directly, but a fully-resolved
        // option / future trading symbol ("ADANIGREEN-Oct2026-1320-PE") must fall
        // back to the prefix before its first '-' / ' '. Without that fallback the
        // lookup misses and every F&O entry silently books lot 1, i.e. a
        // ~600x-too-small quantity (3 units instead of 1800) - tiny rupee PnL
        // swallowed by broker charges. Pure prefix derivation, no extra IO.
        let mut prefixes: Vec<String> = Vec::new();
        let direct = fno_underlying(symbol_name);
        if !direct.is_empty() {
            prefixes.push(direct);
        }
        if let Some(short) = symbol_name.split(['-', ' ']).next() {
            let short = fno_underlying(short);
            if !short.is_empty() && !prefixes.iter().any(|p| p.eq_ignore_ascii_case(&short)) {
                prefixes.push(short);
            }
        }
        let exch = scrip_exch(segment);
        for prefix in prefixes {
            for instr in ["FUTIDX", "FUTSTK", "FUTCOM"] {
                let all = self.fo_rows(&prefix, instr);
                if all.is_empty() {
                    continue;
                }
                let filtered: Vec<&FutRow> = if exch.is_empty() {
                    all.iter().collect()
                } else {
                    let seg: Vec<&FutRow> = all.iter().filter(|r| r.exchange == exch).collect();
                    if seg.is_empty() {
                        all.iter().collect()
                    } else {
                        seg
                    }
                };
                let row = filtered.first()?;
                if !(row.lot > 0.0) {
                    continue;
                }
                let lot = mcx_lot_override(&row.trading_symbol);
                let lot = if lot > 0.0 { lot } else { row.lot };
                let sym = if row.trading_symbol.is_empty() {
                    prefix.clone()
                } else {
                    row.trading_symbol.clone()
                };
                return Some((lot, sym));
            }
        }
        None
    }

    /// Lot-size lookup for every F&O underlying and index (old
    /// `_build_lot_size_map`), built from the scrip master. Returns
    /// `(by_prefix, by_name)` where `by_name` covers the UI index names.
    pub fn lot_map(&self) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
        let mut by_prefix: BTreeMap<String, f64> = BTreeMap::new();
        for instr in ["FUTIDX", "FUTSTK", "FUTCOM"] {
            for ((prefix, i), rows) in &self.fo {
                if i != instr {
                    continue;
                }
                if by_prefix.contains_key(prefix) {
                    continue;
                }
                if let Some(row) = rows.first() {
                    if !(row.lot > 0.0) {
                        continue;
                    }
                    let lot = mcx_lot_override(&row.trading_symbol);
                    by_prefix.insert(
                        prefix.clone(),
                        if lot > 0.0 { lot } else { row.lot },
                    );
                }
            }
        }
        let mut by_name: BTreeMap<String, f64> = BTreeMap::new();
        for (ui_name, prefix) in [
            ("NIFTY 50", "NIFTY"),
            ("BANK NIFTY", "BANKNIFTY"),
            ("FINNIFTY", "FINNIFTY"),
            ("SENSEX", "SENSEX"),
            ("MIDCPNIFTY", "MIDCPNIFTY"),
            ("GIFT NIFTY", "GIFTNIFTY"),
        ] {
            if let Some(lot) = by_prefix.get(prefix) {
                by_name.insert(ui_name.to_string(), *lot);
            }
        }
        (by_prefix, by_name)
    }

    /// Live MCX near-month future for every commodity prefix in the master
    /// (real security id / trading symbol / lot / expiry). Falls back to the
    /// earliest listed expiry when the whole ladder is in the past.
    pub fn commodity_futures(&self) -> Vec<CommodityRow> {
        let today = today_days();
        let mut out: Vec<CommodityRow> = Vec::new();
        for ((prefix, inst), rows) in &self.fo {
            if inst != "FUTCOM" {
                continue;
            }
            let mcx: Vec<&FutRow> = rows.iter().filter(|r| r.exchange == "MCX").collect();
            if mcx.is_empty() {
                continue;
            }
            let pick = mcx
                .iter()
                .filter(|r| bs::parse_ymd(&r.expiry).map(|d| d >= today).unwrap_or(false))
                .min_by(|a, b| a.expiry.cmp(&b.expiry))
                .copied()
                .or_else(|| mcx.iter().min_by(|a, b| a.expiry.cmp(&b.expiry)).copied());
            let Some(r) = pick else { continue };
            let override_lot = mcx_lot_override(&r.trading_symbol);
            let lot = if override_lot > 0.0 { override_lot } else { r.lot };
            out.push(CommodityRow {
                name: prefix.clone(),
                trading_symbol: r.trading_symbol.clone(),
                security_id: r.security_id,
                lot,
                expiry: r.expiry.clone(),
                has_options: self.has_options(prefix, "MCX") == Some(true),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// One commodity future by security id (scanner universe lookup).
    pub fn commodity_by_id(&self, security_id: i64) -> Option<CommodityRow> {
        self.commodity_futures()
            .into_iter()
            .find(|c| c.security_id == security_id)
    }

    /// Lot size + trading symbol + has_options for an MCX commodity id.
    pub fn commodity_meta(&self, security_id: i64) -> Option<(f64, String, bool)> {
        self.commodity_by_id(security_id)
            .map(|c| (c.lot, c.trading_symbol, c.has_options))
    }
}

fn today_days() -> i64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0)
}

fn cache_path() -> PathBuf {
    std::env::temp_dir().join("algodhan_scrip_master.csv")
}

// ---------------------------------------------------------------------------
// Exchange freeze quantity (Dhan detailed scrip master, SM_FREEZE_QTY)
// ---------------------------------------------------------------------------
//
// The realtime engine slices an oversized Super Order into exchange-legal
// chunks. Dhan enforces the per-order quantity cap (the "freeze quantity"),
// which only lives in the *detailed* contract master. Mirrors the old app's
// `_get_freeze_maps`: download once, cache on disk for an hour, look up by
// security id first and by underlying symbol as a fallback.

const FREEZE_URL: &str = "https://images.dhan.co/api-data/api-scrip-master-detailed.csv";
const FREEZE_TTL: Duration = Duration::from_secs(3600);

/// Fallback freeze quantities per underlying (used until the detailed master
/// lands, or when it is unavailable). Values match the old `FREEZE_BY_UNDER`.
const FREEZE_FALLBACK: &[(&str, f64)] = &[
    ("BANKNIFTY", 601.0),
    ("NIFTYNXT50", 601.0),
    ("NIFTY", 1756.0),
    ("FINNIFTY", 1801.0),
    ("MIDCPNIFTY", 2761.0),
    ("SENSEX", 1001.0),
    ("BANKEX", 901.0),
    ("SENSEX50", 1801.0),
];

#[derive(Default)]
pub struct FreezeMap {
    by_id: HashMap<i64, f64>,
    by_under: HashMap<String, f64>,
}

static FREEZE: OnceLock<Arc<FreezeMap>> = OnceLock::new();
static FREEZE_STATUS: AtomicU8 = AtomicU8::new(PENDING);

/// Fallback freeze quantity for an underlying name (longest-prefix match).
pub fn freeze_fallback(underlying: &str) -> f64 {
    let u: String = underlying
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut keys: Vec<&(&str, f64)> = FREEZE_FALLBACK.iter().collect();
    keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (k, v) in keys {
        if u.contains(k) {
            return *v;
        }
    }
    0.0
}

/// Resolve the freeze quantity for a security id / underlying symbol. Falls
/// back to the static per-underlying map when the detailed master is missing.
pub fn freeze_qty(security_id: i64, underlying: &str) -> f64 {
    if let Some(m) = FREEZE.get() {
        if security_id > 0 {
            if let Some(v) = m.by_id.get(&security_id) {
                return *v;
            }
        }
        if !underlying.is_empty() {
            if let Some(v) = m.by_under.get(&underlying.to_uppercase()) {
                return *v;
            }
        }
    }
    freeze_fallback(underlying)
}

/// Parse the detailed master for `SECURITY_ID`, `UNDERLYING_SYMBOL` and
/// `SM_FREEZE_QTY`. Column positions are read from the header so a column
/// re-order upstream cannot silently corrupt the map.
fn parse_freeze_file(path: &Path) -> Option<FreezeMap> {
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut map = FreezeMap::default();
    let mut idx: Option<(usize, usize, usize)> = None;
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if idx.is_none() {
            let find = |name: &str| f.iter().position(|c| c.trim().eq_ignore_ascii_case(name));
            idx = find("SECURITY_ID")
                .zip(find("UNDERLYING_SYMBOL"))
                .map(|(a, b)| (a, b, find("SM_FREEZE_QTY").unwrap_or(usize::MAX)));
            continue;
        }
        let (si, ui, fi) = idx?;
        if fi == usize::MAX || f.len() <= fi {
            continue;
        }
        let Ok(sid) = f[si].trim().parse::<i64>() else { continue };
        let Ok(q) = f[fi].trim().parse::<f64>() else { continue };
        if q > 0.0 {
            map.by_id.insert(sid, q);
        }
        let under = f[ui].trim().to_uppercase();
        if !under.is_empty() {
            map.by_under.entry(under).or_insert(q);
        }
    }
    if map.by_id.is_empty() {
        return None;
    }
    Some(map)
}

fn freeze_cache_path() -> PathBuf {
    std::env::temp_dir().join("algodhan_scrip_master_detailed.csv")
}

/// Warm the freeze-quantity map in the background (idempotent).
pub fn spawn_freeze_warm() {
    if FREEZE.get().is_some() || FREEZE_STATUS.load(Ordering::Relaxed) != PENDING {
        return;
    }
    if FREEZE_STATUS
        .compare_exchange(PENDING, READY, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    tokio::spawn(async move {
        let path = freeze_cache_path();
        let fresh = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| d < FREEZE_TTL)
            .unwrap_or(false);
        if !fresh {
            if let Some(bytes) = download_freeze().await {
                if let Err(e) = std::fs::write(&path, &bytes) {
                    tracing::warn!("freeze master cache write failed: {e}");
                }
            }
        }
        let p = path.clone();
        let map = tokio::task::spawn_blocking(move || parse_freeze_file(&p))
            .await
            .ok()
            .flatten();
        match map {
            Some(m) => {
                let _ = FREEZE.set(Arc::new(m));
                tracing::info!("freeze qty master ready");
            }
            None => {
                FREEZE_STATUS.store(FAILED, Ordering::Relaxed);
                tracing::warn!("freeze qty master unavailable; using fallback map");
            }
        }
    });
}

async fn download_freeze() -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .ok()?;
    let resp = client.get(FREEZE_URL).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.bytes().await.ok().map(|b| b.to_vec())
}

/// Spawn the background warm task (idempotent). Safe to call at startup.
pub fn spawn_warm() {
    spawn_freeze_warm();
    if SCRIP.get().is_some() {
        return;
    }
    tokio::spawn(async move {
        for attempt in 0..3 {
            if let Some(sc) = load().await {
                let _ = SCRIP.set(Arc::new(sc));
                STATUS.store(READY, Ordering::Relaxed);
                tracing::info!("scrip master ready");
                return;
            }
            tracing::warn!("scrip master warm attempt {} failed", attempt + 1);
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        STATUS.store(FAILED, Ordering::Relaxed);
        tracing::warn!("scrip master unavailable; instrument lookup will return no results");
    });
}

async fn load() -> Option<Scrip> {
    let path = cache_path();
    let fresh = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d < SCRIP_TTL)
        .unwrap_or(false);
    if !fresh {
        if let Some(bytes) = download().await {
            if let Err(e) = std::fs::write(&path, &bytes) {
                tracing::warn!("scrip master cache write failed: {e}");
            }
        }
    }
    let p = path.clone();
    tokio::task::spawn_blocking(move || parse_file(&p))
        .await
        .ok()
        .flatten()
}

async fn download() -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .ok()?;
    let resp = client.get(SCRIP_URL).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.bytes().await.ok().map(|b| b.to_vec())
}

fn parse_file(path: &Path) -> Option<Scrip> {
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut sc = Scrip::default();
    let mut expiry_sets: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut stock_sets: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut fo_sets: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut header = true;
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if header {
            header = false;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 11 {
            continue;
        }
        let inst = f[3];
        if inst == "EQUITY" {
            let exch = f[0].trim().to_uppercase();
            let series = f.get(14).map(|s| s.trim().to_uppercase()).unwrap_or_default();
            if exch == "NSE" && series == "EQ" {
                if let Ok(sid) = f[2].trim().parse::<i64>() {
                    if sid > 0 {
                        let symbol = f[5].trim().to_uppercase();
                        sc.eq_sid.insert(symbol.clone(), sid);
                        sc.contracts.insert(
                            sid,
                            Contract {
                                security_id: sid,
                                exchange: exch,
                                instrument: inst.to_string(),
                                underlying: symbol,
                                expiry: String::new(),
                                strike: 0.0,
                                option_type: String::new(),
                                lot: f.get(6).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(1.0).max(1.0),
                                trading_symbol: f[5].trim().to_string(),
                            },
                        );
                    }
                }
            }
            continue;
        }
        let is_opt = matches!(inst, "OPTIDX" | "OPTSTK" | "OPTFUT");
        let is_fut = matches!(inst, "FUTIDX" | "FUTSTK" | "FUTCOM");
        if !is_opt && !is_fut {
            continue;
        }
        let symbol = f[5];
        let Some(prefix) = symbol.split('-').next() else { continue };
        if prefix.is_empty() {
            continue;
        }
        let prefix = prefix.to_uppercase();
        let exch = f[0].to_uppercase();
        let expiry = normalize_expiry(f[8]);
        if expiry.is_empty() {
            continue;
        }
        if is_fut {
            fo_sets.entry((prefix.clone(), inst.to_string())).or_default().push(expiry.clone());
            if let Ok(sid) = f[2].trim().parse::<i64>() {
                let lot = f[6].trim().parse::<f64>().unwrap_or(0.0);
                sc.fo
                    .entry((prefix.clone(), inst.to_string()))
                    .or_default()
                    .push(FutRow {
                        exchange: exch.clone(),
                        expiry: expiry.clone(),
                        security_id: sid,
                        lot,
                        trading_symbol: symbol.to_string(),
                    });
                sc.contracts.insert(
                    sid,
                    Contract {
                        security_id: sid,
                        exchange: exch.clone(),
                        instrument: inst.to_string(),
                        underlying: prefix.clone(),
                        expiry,
                        strike: 0.0,
                        option_type: String::new(),
                        lot: lot.max(1.0),
                        trading_symbol: symbol.to_string(),
                    },
                );
            }
            continue;
        }
        let otype = f[10].trim().to_uppercase();
        if otype != "CE" && otype != "PE" {
            continue;
        }
        let Ok(sid) = f[2].trim().parse::<i64>() else { continue };
        sc.opt_ids.insert(sid);
        let Ok(strike) = f[9].trim().parse::<f64>() else { continue };
        if strike <= 0.0 {
            continue;
        }
        let lot = f[6].trim().parse::<f64>().unwrap_or(1.0);
        let bucket = sc
            .oc
            .entry((exch.clone(), prefix.clone(), expiry.clone()))
            .or_default();
        let entry = bucket.entry(strike_key(strike)).or_default();
        entry.lot = lot;
        if otype == "CE" {
            entry.ce = Some((sid, symbol.to_string()));
        } else {
            entry.pe = Some((sid, symbol.to_string()));
        }
        sc.contracts.insert(
            sid,
            Contract {
                security_id: sid,
                exchange: exch.clone(),
                instrument: inst.to_string(),
                underlying: prefix.clone(),
                expiry: expiry.clone(),
                strike,
                option_type: otype.to_string(),
                lot: lot.max(1.0),
                trading_symbol: symbol.to_string(),
            },
        );
        if inst == "OPTSTK" {
            stock_sets
                .entry((exch.clone(), prefix.clone()))
                .or_default()
                .push(expiry.clone());
        }
        expiry_sets
            .entry((exch, prefix))
            .or_default()
            .push(expiry);
    }
    for (k, mut v) in expiry_sets {
        v.sort();
        v.dedup();
        sc.expiries.insert(k, v);
    }
    for (k, mut v) in stock_sets {
        v.sort();
        v.dedup();
        sc.stock_expiries.insert(k, v);
    }
    for (k, mut v) in fo_sets {
        v.sort();
        v.dedup();
        sc.fo_expiries.insert(k, v);
    }
    if sc.oc.is_empty() {
        return None;
    }
    Some(sc)
}

#[cfg(test)]
mod index_prefix_tests {
    use super::is_index_prefix;

    #[test]
    fn recognises_cash_index_underlyings() {
        for p in ["NIFTY", "BANKNIFTY", "FINNIFTY", "MIDCPNIFTY", "SENSEX", "banknifty"] {
            assert!(is_index_prefix(p), "{p} should be an index");
        }
    }

    #[test]
    fn rejects_stock_underlyings() {
        for p in ["RELIANCE", "TATAMOTORS", "BAJAJ", "NAM", ""] {
            assert!(!is_index_prefix(p), "{p} should not be an index");
        }
    }
}

#[cfg(test)]
mod option_id_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn is_option_separates_options_from_futures() {
        let path = std::env::temp_dir().join(format!("scrip_opt_test_{}.csv", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(
                f,
                "SEM_EXM_EXCH_ID,SEM_SEGMENT,SEM_SMST_SECURITY_ID,SEM_INSTRUMENT_NAME,SEM_EXPIRY_CODE,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_CUSTOM_SYMBOL,SEM_EXPIRY_DATE,SEM_STRIKE_PRICE,SEM_OPTION_TYPE"
            )
            .unwrap();
            // One stock option and one stock future.
            writeln!(
                f,
                "NSE,D,123434,OPTSTK,0,LTM-Sep2026-4200-PE,150.0,LTM 29 SEP 4200 PUT,2026-09-29 14:30:00,4200.00000,PE"
            )
            .unwrap();
            writeln!(
                f,
                "NSE,D,99999,FUTSTK,0,LTM-Sep2026-FUT,150.0,LTM 29 SEP FUT,2026-09-29 14:30:00,,"
            )
            .unwrap();
        }
        let sc = parse_file(&path).expect("scrip parses");
        assert!(sc.is_option(123434), "option id must be detected");
        assert!(!sc.is_option(99999), "future id must not be an option");
        assert!(!sc.is_option(0), "invalid id must not be an option");
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod lot_for_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn lot_for_resolves_a_full_option_symbol_to_the_underlying_lot() {
        let path = std::env::temp_dir().join(format!("scrip_lot_test_{}.csv", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(
                f,
                "SEM_EXM_EXCH_ID,SEM_SEGMENT,SEM_SMST_SECURITY_ID,SEM_INSTRUMENT_NAME,SEM_EXPIRY_CODE,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_CUSTOM_SYMBOL,SEM_EXPIRY_DATE,SEM_STRIKE_PRICE,SEM_OPTION_TYPE"
            )
            .unwrap();
            writeln!(
                f,
                "NSE,D,48712,FUTSTK,0,ADANIGREEN-Oct2026-FUT,600.0,ADANIGREEN OCT FUT,2026-10-27 14:30:00,,"
            )
            .unwrap();
            writeln!(
                f,
                "NSE,D,70001,OPTSTK,0,ADANIGREEN-Oct2026-1320-PE,600.0,ADANIGREEN 27 OCT 1320 PUT,2026-10-27 15:30:00,1320.00000,PE"
            )
            .unwrap();
        }
        let sc = parse_file(&path).expect("scrip parses");
        // A plain underlying resolves directly.
        assert_eq!(sc.lot_for("ADANIGREEN", "NSE_FNO").map(|(l, _)| l), Some(600.0));
        // A fully-resolved option symbol (exactly what open_entry passes) must
        // still find the underlying lot instead of falling back to 1.
        assert_eq!(
            sc.lot_for("ADANIGREEN-Oct2026-1320-PE", "NSE_FNO").map(|(l, _)| l),
            Some(600.0)
        );
        // Unknown underlying: no lot, so the caller keeps its own fallback.
        assert!(sc.lot_for("NOSUCHSTOCK-Oct2026-100-CE", "NSE_FNO").is_none());
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod stock_expiry_day_tests {
    use super::*;
    use algo_core::option as bs;
    use std::io::Write;

    #[test]
    fn stock_option_expiry_day_ignores_indices_and_commodities() {
        let path = std::env::temp_dir().join(format!("scrip_expday_test_{}.csv", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(
                f,
                "SEM_EXM_EXCH_ID,SEM_SEGMENT,SEM_SMST_SECURITY_ID,SEM_INSTRUMENT_NAME,SEM_EXPIRY_CODE,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_CUSTOM_SYMBOL,SEM_EXPIRY_DATE,SEM_STRIKE_PRICE,SEM_OPTION_TYPE"
            )
            .unwrap();
            // F&O stock option expiring 2026-10-27.
            writeln!(
                f,
                "NSE,D,111111,OPTSTK,0,RELIANCE-Oct2026-1400-CE,250.0,RELIANCE 27 OCT 1400 CALL,2026-10-27 15:30:00,1400.00000,CE"
            )
            .unwrap();
            // Index option expiring 2026-10-13 (must NOT count as a stock expiry).
            writeln!(
                f,
                "NSE,D,222222,OPTIDX,0,NIFTY-Oct2026-25000-CE,75.0,NIFTY 13 OCT 25000 CALL,2026-10-13 15:30:00,25000.00000,CE"
            )
            .unwrap();
            // MCX commodity option expiring 2026-10-15 (also must NOT count).
            writeln!(
                f,
                "MCX,M,333333,OPTFUT,0,CRUDEOIL-Oct2026-5000-CE,100.0,CRUDEOIL 15 OCT 5000 CALL,2026-10-15 23:30:00,5000.00000,CE"
            )
            .unwrap();
        }
        let sc = parse_file(&path).expect("scrip parses");
        let stock_day = bs::parse_ymd("2026-10-27").unwrap();
        let index_day = bs::parse_ymd("2026-10-13").unwrap();
        let comm_day = bs::parse_ymd("2026-10-15").unwrap();
        assert!(sc.is_stock_option_expiry_day(stock_day), "stock option expiry day must match");
        assert!(!sc.is_stock_option_expiry_day(index_day), "index expiry day must not match");
        assert!(!sc.is_stock_option_expiry_day(comm_day), "commodity expiry day must not match");
        // Upcoming ladder must list the stock expiry only, nearest first.
        let from = bs::parse_ymd("2026-10-01").unwrap();
        let up = sc.stock_option_expiries_from(from, 4);
        assert_eq!(up, vec!["2026-10-27".to_string()], "only the F&O stock expiry is listed");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nse_commodity_optfut_is_not_a_stock_expiry_day() {
        let path = std::env::temp_dir().join(format!("scrip_commoptfut_test_{}.csv", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(
                f,
                "SEM_EXM_EXCH_ID,SEM_SEGMENT,SEM_SMST_SECURITY_ID,SEM_INSTRUMENT_NAME,SEM_EXPIRY_CODE,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_CUSTOM_SYMBOL,SEM_EXPIRY_DATE,SEM_STRIKE_PRICE,SEM_OPTION_TYPE"
            )
            .unwrap();
            // NSE options-on-futures on a commodity (CRUDEOIL) expiring 2026-10-08.
            // This is listed on NSE with a non-index prefix, and previously tripped
            // the F&O-stock expiry guard even though it is not a stock option.
            writeln!(
                f,
                "NSE,M,444444,OPTFUT,0,CRUDEOIL-08Oct2026-5550-CE,100.0,CRUDEOIL 08 OCT 5550 CALL,2026-10-08 15:30:00,5550.00000,CE"
            )
            .unwrap();
            // A genuine NSE stock option expiring 2026-10-27 (must count).
            writeln!(
                f,
                "NSE,D,555555,OPTSTK,0,RELIANCE-Oct2026-1400-CE,250.0,RELIANCE 27 OCT 1400 CALL,2026-10-27 15:30:00,1400.00000,CE"
            )
            .unwrap();
        }
        let sc = parse_file(&path).expect("scrip parses");
        let crude_day = bs::parse_ymd("2026-10-08").unwrap();
        let stock_day = bs::parse_ymd("2026-10-27").unwrap();
        assert!(
            !sc.is_stock_option_expiry_day(crude_day),
            "NSE commodity OPTFUT expiry must NOT count as a stock expiry day"
        );
        assert!(sc.is_stock_option_expiry_day(stock_day), "genuine stock option expiry must count");
        let from = bs::parse_ymd("2026-10-01").unwrap();
        assert_eq!(
            sc.stock_option_expiries_from(from, 4),
            vec!["2026-10-27".to_string()],
            "commodity OPTFUT must not appear in the stock expiry ladder"
        );
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod fno_exch_tests {
    use super::*;
    use std::io::Write;

    const HEADER: &str = "SEM_EXM_EXCH_ID,SEM_SEGMENT,SEM_SMST_SECURITY_ID,SEM_INSTRUMENT_NAME,SEM_EXPIRY_CODE,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_CUSTOM_SYMBOL,SEM_EXPIRY_DATE,SEM_STRIKE_PRICE,SEM_OPTION_TYPE";

    fn write_master(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("scrip_fnoexch_{tag}_{}.csv", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "{HEADER}").unwrap();
            // SENSEX options live ONLY on BSE (real master: 4230 BSE OPTIDX rows).
            writeln!(f, "BSE,D,910001,OPTIDX,0,SENSEX-Nov2026-80000-CE,20.0,SENSEX 80000 CALL,2026-11-12 15:30:00,80000.00000,CE").unwrap();
            writeln!(f, "BSE,D,910002,OPTIDX,0,SENSEX-Nov2026-80000-PE,20.0,SENSEX 80000 PUT,2026-11-12 15:30:00,80000.00000,PE").unwrap();
            // NIFTY options live ONLY on NSE.
            writeln!(f, "NSE,D,910003,OPTIDX,0,NIFTY-Nov2026-24500-CE,75.0,NIFTY 24500 CALL,2026-11-12 15:30:00,24500.00000,CE").unwrap();
            writeln!(f, "NSE,D,910004,OPTIDX,0,NIFTY-Nov2026-24500-PE,75.0,NIFTY 24500 PUT,2026-11-12 15:30:00,24500.00000,PE").unwrap();
            // MCX commodity options stay on MCX.
            writeln!(f, "MCX,M,910005,OPTFUT,0,CRUDEOIL-Nov2026-5000-CE,100.0,CRUDEOIL 5000 CALL,2026-11-19 23:30:00,5000.00000,CE").unwrap();
        }
        path
    }

    #[test]
    fn sensx_idx_i_rehomes_to_bse_but_nifty_stays_nse() {
        let path = write_master("rehome");
        let sc = parse_file(&path).expect("scrip parses");
        assert_eq!(sc.fno_exch("IDX_I", "SENSEX"), "BSE", "SENSEX options are BSE-only");
        assert_eq!(sc.fno_exch("IDX_I", "NIFTY"), "NSE");
        assert_eq!(sc.fno_exch("IDX_I", "BANKNIFTY"), "NSE", "unknown prefix keeps default");
        assert_eq!(sc.fno_exch("MCX_COMM", "CRUDEOIL"), "MCX");
        assert_eq!(sc.fno_exch("BSE_FNO", "SENSEX"), "BSE", "explicit segment wins");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sensx_option_resolves_on_bse_through_idx_i_segment() {
        let path = write_master("resolve");
        let sc = parse_file(&path).expect("scrip parses");
        let r = sc
            .resolve("SENSEX", "2026-11-12", 80000.0, "PE", "IDX_I")
            .expect("SENSEX PE must resolve even though the spot segment is IDX_I");
        assert_eq!(r.security_id, 910002);
        assert_eq!(r.expiry, "2026-11-12");
        // And a NIFTY option under the same IDX_I segment resolves on NSE.
        let n = sc
            .resolve("NIFTY 50", "2026-11-12", 24500.0, "CE", "IDX_I")
            .expect("NIFTY CE must resolve on NSE");
        assert_eq!(n.security_id, 910003);
        let _ = std::fs::remove_file(&path);
    }
}
