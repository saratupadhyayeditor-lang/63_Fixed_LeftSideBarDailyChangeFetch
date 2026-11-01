//! Multi-broker registry + the single Broker Settings store.
//!
//! One tab ("Broker Settings") owns the API credentials for EVERY supported
//! broker (Dhan, ICICI Breeze, Angel One SmartAPI, Alice Blue ANT, 5paisa,
//! Finvasia Shoonya, Fyers v3, Kotak Neo, Zerodha Kite Connect, Upstox) and the
//! per-broker
//! enable switch. The trading engines then keep their own routing map: which of
//! the enabled brokers THIS engine is allowed to send orders to.
//!
//! Two invariants are enforced here, not in the UI:
//!
//!   * secret values are never echoed back to the browser - `GET /api/brokers`
//!     returns a mask, and a POST that carries the mask back never overwrites
//!     the stored secret;
//!   * a broker only counts as *usable* when its switch is ON and every
//!     required field is filled in, so ticking a broker with missing keys can
//!     never silently swallow an order.
//!
//! Order delivery for Dhan uses the existing `dhan-hq` client. The remaining
//! brokers are registered with their credential fields and trailing
//! capabilities so the settings + routing layer is complete, but their order
//! adapters are marked `adapter_pending` until a live account can verify them:
//! silently placing real money orders through an unverified adapter is worse
//! than refusing, so the router skips them and logs the reason.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use axum::response::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STATE_FILE: &str = "algodhan_brokers.json";
/// What a stored secret looks like to the browser. A POST that sends this exact
/// value back means "leave the stored secret alone".
pub const SECRET_MASK: &str = "********";

/// One credential box in the Broker Settings tab.
#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    /// `text` | `password` | `number`
    pub kind: &'static str,
    pub secret: bool,
    pub required: bool,
    /// Shape the value must have, checked before any network call so an
    /// obviously wrong paste (a mangled key, a token from another broker) is
    /// reported instead of silently "connecting".
    /// `""` (any) | `uuid` | `base32` | `digits` | `alnum` | `token`
    pub format: &'static str,
    /// Consumed only while minting a session (OAuth auth-code / checksum /
    /// password), never presented on an authenticated call. Such a field can
    /// never be proven by a session probe, so a Test must say so rather than
    /// claim the whole account is connected.
    pub login_only: bool,
    pub help: &'static str,
}

const fn f(key: &'static str, label: &'static str, kind: &'static str, secret: bool, required: bool, help: &'static str) -> FieldSpec {
    FieldSpec { key, label, kind, secret, required, format: "", login_only: false, help }
}

/// `f` plus the shape/semantics a Test needs to judge the value.
const fn ff(
    key: &'static str,
    label: &'static str,
    kind: &'static str,
    secret: bool,
    required: bool,
    format: &'static str,
    login_only: bool,
    help: &'static str,
) -> FieldSpec {
    FieldSpec { key, label, kind, secret, required, format, login_only, help }
}

/// One supported broker: credential schema + what the broker can do natively.
#[derive(Clone, Copy, Debug)]
pub struct BrokerSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// Broker runs the trailing stop in its own OMS (BO / Super Order / TSL),
    /// so the app must NOT also ratchet a stop for the same position.
    pub native_trail: bool,
    /// Real order API wired into this build. `false` = registered + configurable
    /// but the router refuses to send a live order through it.
    pub adapter_ready: bool,
    pub note: &'static str,
    /// The credentials for this broker are entered in the app header (the Dhan
    /// Connect box) rather than in Broker Settings. The card still shows the
    /// broker + its enable switch and reports whether the header is connected,
    /// but it must not render duplicate credential inputs.
    pub credentials_in_header: bool,
    /// Fields the broker's authenticated probe genuinely exercises (it sends
    /// them on the profile/limits call). Any `login_only` field NOT listed here
    /// stays "unverified" in a Test result, so a token-only check never implies
    /// the whole account was validated.
    pub probe_proves: &'static [&'static str],
    pub fields: &'static [FieldSpec],
}

/// Broker order methods that carry a native broker-side trailing stop, per the
/// broker's published API. Used to decide who owns the trail for a position.
const DHAN_NATIVE_TRAIL_METHODS: &[&str] = &["super"];
const BO_NATIVE_TRAIL_METHODS: &[&str] = &["super"];

const DHAN_FIELDS: &[FieldSpec] = &[
    ff("clientId", "Client ID", "text", false, true, "digits", false, "Dhan client id (numeric)."),
    ff("accessToken", "Access Token", "password", true, true, "token", false, "Dhan v2 REST access token (daily)."),
];
const ZERODHA_FIELDS: &[FieldSpec] = &[
    ff("apiKey", "API Key", "text", false, true, "alnum", false, "Kite Connect api_key (sent with the token)."),
    ff("apiSecret", "API Secret", "password", true, true, "token", true, "Kite Connect api_secret (checksum only)."),
    ff("accessToken", "Access Token", "password", true, true, "token", false, "Kite access_token (daily login)."),
    f("userId", "User ID", "text", false, false, "Kite user id (display / order tag)."),
];
const BREEZE_FIELDS: &[FieldSpec] = &[
    ff("apiKey", "API Key", "text", false, true, "alnum", false, "ICICI Breeze app key (sent as X-AppKey)."),
    ff("secretKey", "Secret Key", "password", true, true, "token", true, "Breeze secret key (checksum only)."),
    ff("sessionToken", "Session Token", "password", true, true, "token", false, "Breeze session token (daily)."),
    ff("userId", "User ID", "text", false, true, "alnum", true, "Breeze user id (login / display only)."),
];
const ANGEL_FIELDS: &[FieldSpec] = &[
    ff("apiKey", "API Key", "text", false, true, "alnum", false, "SmartAPI api_key (sent as X-PrivateKey)."),
    ff("clientCode", "Client Code", "text", false, true, "alnum", true, "Angel client code (login only)."),
    ff("pin", "PIN", "password", true, true, "digits", true, "SmartAPI 4-digit PIN (login only)."),
    ff("totpSecret", "TOTP Secret", "password", true, false, "base32", true, "Base32 TOTP seed; leave blank when using a JWT."),
    ff("jwtToken", "JWT Token", "password", true, false, "token", false, "Pre-generated JWT (skips TOTP login)."),
];
const ALICE_FIELDS: &[FieldSpec] = &[
    f("userId", "User ID", "text", false, true, "Alice Blue user id."),
    ff("apiKey", "API Key", "text", false, true, "alnum", true, "ANT api key (login only)."),
    ff("sessionId", "Session ID", "password", true, true, "token", false, "ANT session id (daily)."),
];
const FIVEPAISA_FIELDS: &[FieldSpec] = &[
    ff("userId", "User ID", "text", false, true, "alnum", true, "5paisa login id (login only)."),
    ff("userKey", "User Key", "password", true, true, "token", false, "5paisa UserKey (sent on every call)."),
    ff("encryptionKey", "Encryption Key", "password", true, true, "token", true, "5paisa EncryptionKey (checksum only)."),
    ff("accessToken", "Access Token", "password", true, false, "token", false, "5paisa access token when already logged in."),
];
const FYERS_FIELDS: &[FieldSpec] = &[
    ff("clientId", "Client ID", "text", false, true, "alnum", false, "Fyers app id (e.g. AB12345-100)."),
    ff("secretKey", "Secret Key", "password", true, true, "token", true, "Fyers v3 secret key (checksum only)."),
    ff("accessToken", "Access Token", "password", true, true, "token", false, "Fyers v3 access token (daily)."),
];
const KOTAK_FIELDS: &[FieldSpec] = &[
    ff("consumerKey", "Consumer Key", "text", false, true, "alnum", true, "Kotak Neo consumer key (login only)."),
    ff("consumerSecret", "Consumer Secret", "password", true, true, "token", true, "Kotak Neo consumer secret (login only)."),
    ff("mobileNumber", "Mobile Number", "text", false, true, "digits", true, "Registered mobile number (login only)."),
    ff("ucc", "UCC", "text", false, true, "alnum", false, "Kotak UCC / client code."),
    ff("totp", "TOTP", "password", true, false, "digits", true, "Current TOTP (or set the seed below)."),
    ff("totpSecret", "TOTP Secret", "password", true, false, "base32", true, "Base32 TOTP seed for automatic login."),
    ff("accessToken", "Access Token", "password", true, false, "token", false, "Session token when already logged in."),
];
/// Finvasia Shoonya (Noren OMS). The session token is daily and can either be
/// pasted directly or minted from the OAuth auth code; there is no PIN + TOTP
/// login endpoint (the TOTP step happens inside Shoonya's hosted OAuth page).
const SHOONYA_FIELDS: &[FieldSpec] = &[
    ff("userId", "User ID", "text", false, true, "alnum", false, "Shoonya login id (uid / actid)."),
    f("accountId", "Account ID", "text", false, false, "Shoonya actid when it differs from the user id."),
    ff("clientId", "Client ID", "text", false, false, "alnum", true, "OAuth client id (needed only to mint a token from an auth code)."),
    ff("secretKey", "Secret Key", "password", true, false, "token", true, "OAuth secret key (checksum for the auth-code exchange)."),
    ff("authCode", "Auth Code", "password", true, false, "token", true, "OAuth redirect code; used once to mint a session token."),
    ff("accessToken", "Access Token", "password", true, true, "token", false, "Shoonya session token (susertoken), daily."),
];
const UPSTOX_FIELDS: &[FieldSpec] = &[
    // Upstox client credentials are UUID / opaque strings; the token is a JWT.
    // Only the token is exercised by the profile call, so the pair is marked
    // login-only and shape-checked here instead of being silently trusted.
    ff("apiKey", "API Key", "text", false, true, "uuid", true, "Upstox v2 api key (UUID from the developer console)."),
    ff("apiSecret", "API Secret", "password", true, true, "token", true, "Upstox v2 api secret (only used to mint a token)."),
    ff("accessToken", "Access Token", "password", true, true, "token", false, "Upstox v2 access token (daily)."),
];

const CATALOG: &[BrokerSpec] = &[
    BrokerSpec {
        id: "dhan",
        name: "Dhan",
        native_trail: true,
        adapter_ready: true,
        note: "Super Order trailingJump + Forever/GTT. Fully wired in this build. Credentials header ke Dhan Connect se aate hain.",
        credentials_in_header: true,
        probe_proves: &["clientId", "accessToken"],
        fields: DHAN_FIELDS,
    },
    BrokerSpec {
        id: "icici",
        name: "ICICI Breeze",
        native_trail: false,
        adapter_ready: true,
        note: "Breeze order / cancel API. No native trailing jump is sent, so the engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["apiKey", "sessionToken", "userId"],
        fields: BREEZE_FIELDS,
    },
    BrokerSpec {
        id: "angel",
        name: "Angel One SmartAPI",
        native_trail: false,
        adapter_ready: true,
        note: "SmartAPI order / cancel (PIN + TOTP or JWT login). Engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["apiKey"],
        fields: ANGEL_FIELDS,
    },
    BrokerSpec {
        id: "alice",
        name: "Alice Blue ANT",
        native_trail: false,
        adapter_ready: true,
        note: "ANT order / cancel API. Engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["userId", "sessionId"],
        fields: ALICE_FIELDS,
    },
    BrokerSpec {
        id: "fivepaisa",
        name: "5paisa",
        native_trail: false,
        adapter_ready: true,
        note: "5paisa order / cancel API. Engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["userId", "userKey"],
        fields: FIVEPAISA_FIELDS,
    },
    BrokerSpec {
        id: "shoonya",
        name: "Finvasia Shoonya",
        native_trail: false,
        adapter_ready: true,
        note: "Shoonya Noren order / cancel API (LMT + SL-LMT only). Engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["userId", "accessToken"],
        fields: SHOONYA_FIELDS,
    },
    BrokerSpec {
        id: "fyers",
        name: "Fyers v3",
        native_trail: false,
        adapter_ready: true,
        note: "Fyers v3 order / cancel API. Engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["clientId", "accessToken"],
        fields: FYERS_FIELDS,
    },
    BrokerSpec {
        id: "kotak",
        name: "Kotak Neo",
        native_trail: false,
        adapter_ready: true,
        note: "Kotak Neo order / cancel API. Engine owns the trail.",
        credentials_in_header: false,
        probe_proves: &["ucc", "accessToken"],
        fields: KOTAK_FIELDS,
    },
    BrokerSpec {
        id: "zerodha",
        name: "Zerodha Kite Connect",
        native_trail: false,
        adapter_ready: true,
        note: "Kite Connect order / cancel API. No native TSL, so the engine runs its own trail.",
        credentials_in_header: false,
        probe_proves: &["apiKey", "accessToken"],
        fields: ZERODHA_FIELDS,
    },
    BrokerSpec {
        id: "upstox",
        name: "Upstox",
        native_trail: false,
        adapter_ready: true,
        note: "Upstox v2 order / cancel API. No native TSL, so the engine runs its own trail.",
        credentials_in_header: false,
        probe_proves: &["apiKey", "apiSecret", "accessToken"],
        fields: UPSTOX_FIELDS,
    },
];

pub fn catalog() -> &'static [BrokerSpec] {
    CATALOG
}

pub fn spec(id: &str) -> Option<&'static BrokerSpec> {
    CATALOG.iter().find(|b| b.id.eq_ignore_ascii_case(id))
}

/// One stored broker account.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrokerAccount {
    pub enabled: bool,
    pub fields: BTreeMap<String, String>,
}

/// Whole Broker Settings document (one file, shared by every engine tab).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrokerStore {
    pub accounts: BTreeMap<String, BrokerAccount>,
    pub updated_at: i64,
}

impl BrokerAccount {
    fn field(&self, key: &str) -> &str {
        self.fields.get(key).map(|s| s.trim()).unwrap_or("")
    }

    /// Required fields still empty for this broker (empty when complete).
    fn missing(&self, spec: &BrokerSpec) -> Vec<&'static str> {
        spec.fields
            .iter()
            .filter(|f| f.required && self.field(f.key).is_empty())
            .map(|f| f.key)
            .collect()
    }

    /// Switch ON and every required credential present.
    fn usable(&self, spec: &BrokerSpec) -> bool {
        self.enabled && self.missing(spec).is_empty()
    }
}

/// Read-only view of one broker's stored state (secrets already masked).
fn account_json(spec: &BrokerSpec, acct: Option<&BrokerAccount>) -> Value {
    let empty = BrokerAccount::default();
    let a = acct.unwrap_or(&empty);
    let mut fields = serde_json::Map::new();
    for f in spec.fields {
        let raw = a.field(f.key);
        let shown = if f.secret && !raw.is_empty() { SECRET_MASK.to_string() } else { raw.to_string() };
        fields.insert(f.key.to_string(), json!(shown));
    }
    let missing: Vec<&str> = a.missing(spec);
    json!({
        "id": spec.id,
        "enabled": a.enabled,
        "fields": fields,
        "configured": missing.is_empty() && spec.fields.iter().any(|f| f.required),
        "missing": missing,
        "usable": a.usable(spec),
    })
}

pub fn catalog_json() -> Value {
    let s = state();
    let list: Vec<Value> = CATALOG
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "name": b.name,
                "nativeTrail": b.native_trail,
                "adapterReady": b.adapter_ready,
                "credentialsInHeader": b.credentials_in_header,
                "note": b.note,
                "fields": b.fields.iter().map(|f| json!({
                    "key": f.key,
                    "label": f.label,
                    "kind": f.kind,
                    "secret": f.secret,
                    "required": f.required,
                    "help": f.help,
                })).collect::<Vec<Value>>(),
                "account": account_json(b, s.accounts.get(b.id)),
            })
        })
        .collect();
    json!({
        "ok": true,
        "secretMask": SECRET_MASK,
        "updatedAt": s.updated_at,
        "brokers": list,
    })
}

/// Broker ids that are switched ON and fully configured, in catalogue order.
pub fn enabled_configured() -> Vec<&'static str> {
    let s = state();
    CATALOG
        .iter()
        .filter(|b| s.accounts.get(b.id).map(|a| a.usable(b)).unwrap_or(false))
        .map(|b| b.id)
        .collect()
}

/// True once the operator has entered anything at all in Broker Settings. Used
/// to keep an untouched install on its legacy Dhan-only routing instead of
/// silently routing nothing.
pub fn any_configured() -> bool {
    let s = state();
    s.accounts
        .values()
        .any(|a| a.enabled || a.fields.values().any(|v| !v.trim().is_empty()))
}

/// Whether this build can actually deliver a real order to `id`.
pub fn adapter_ready(id: &str) -> bool {
    spec(id).map(|b| b.adapter_ready).unwrap_or(false)
}

/// Raw credential value for one broker field (trimmed, empty when unset).
///
/// Only the order adapters call this: the value must never be echoed to the
/// browser or written to a log.
pub fn credential(id: &str, key: &str) -> String {
    let s = state();
    s.accounts
        .get(id)
        .and_then(|a| a.fields.get(key))
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

/// Resolve the brokers one engine should send this entry to.
///
///   * an engine that has NEVER been routed (empty map) sends to every broker
///     that is switched ON and fully configured in Broker Settings; on an
///     untouched install that is exactly the legacy `["dhan"]`, so nothing
///     changes for an existing book merely because the tab was opened;
///   * a non-empty map sends ONLY to the ticked brokers that are also switched
///     ON + fully configured (an explicit untick is stored as `false`, so "all
///     unticked" is distinguishable from "never routed" and refuses to trade);
///   * a ticked broker that is not usable is dropped here and reported by the
///     caller, never silently traded.
pub fn resolve_route(map: &BTreeMap<String, bool>, usable: &[&str], settings_touched: bool) -> Vec<String> {
    if map.is_empty() {
        // Never routed: honour every usable broker (legacy install = Dhan only).
        if usable.is_empty() && !settings_touched {
            return vec!["dhan".to_string()];
        }
        return usable.iter().map(|s| s.to_ascii_lowercase()).collect();
    }
    map.iter()
        .filter(|(_, on)| **on)
        .map(|(k, _)| k.to_ascii_lowercase())
        .filter(|id| usable.iter().any(|u| u.eq_ignore_ascii_case(id)))
        .collect()
}

/// Merge a settings POST into the store. Only known brokers/fields are
/// accepted, and a masked secret is treated as "unchanged", so a round-tripped
/// form can never wipe credentials.
pub fn update_account(id: &str, enabled: Option<bool>, fields: Option<&serde_json::Map<String, Value>>) -> Result<(), String> {
    let spec = spec(id).ok_or_else(|| format!("unknown broker: {id}"))?;
    let mut s = state();
    let acct = s.accounts.entry(spec.id.to_string()).or_default();
    if let Some(on) = enabled {
        acct.enabled = on;
    }
    if let Some(map) = fields {
        for f in spec.fields {
            if let Some(v) = map.get(f.key) {
                let val = v.as_str().map(|x| x.trim().to_string()).unwrap_or_default();
                if f.secret && (val == SECRET_MASK || val.is_empty()) {
                    // Keep whatever is already stored - an empty box in the UI
                    // means "not editing this secret", never "erase it".
                    continue;
                }
                acct.fields.insert(f.key.to_string(), val);
            }
        }
    }
    s.updated_at = now_secs();
    persist(&s);
    Ok(())
}

/// Trail owner for a position routed to `broker` with `method`:
///   * `"broker"` - the broker's own OMS ratchets the stop (native BO/TSL/
///     Super Order), so the engine must not run a second trail;
///   * `"engine"` - the app owns the trail.
pub fn trail_owner(broker: &str, method: &str) -> &'static str {
    let native_methods = if broker.eq_ignore_ascii_case("dhan") {
        DHAN_NATIVE_TRAIL_METHODS
    } else {
        BO_NATIVE_TRAIL_METHODS
    };
    let native = spec(broker).map(|b| b.native_trail).unwrap_or(false)
        && native_methods.iter().any(|m| m.eq_ignore_ascii_case(method));
    if native {
        "broker"
    } else {
        "engine"
    }
}

/// Trail owner for a whole routed entry: broker-side only when EVERY broker the
/// order went to runs a native trail for this method. A mixed set falls back to
/// the engine, which owns the single trail (never two trails on one position).
pub fn trail_owner_for(brokers: &[String], method: &str) -> &'static str {
    if brokers.is_empty() {
        return "engine";
    }
    if brokers.iter().all(|b| trail_owner(b, method) == "broker") {
        "broker"
    } else {
        "engine"
    }
}

/// Who actually ratchets the stop for one routed entry.
///
/// The broker owns it only when EVERY routed broker runs a native trail for this
/// method AND a jump (`point_trail`) is actually configured. A native-trail
/// broker with no jump would trail nothing, so the engine keeps its own trail
/// instead of leaving the position unprotected. Any mixed set (one broker with
/// no native trail) also falls back to the engine, so a position never carries
/// two competing ratchets.
pub fn broker_owns_trail(brokers: &[String], method: &str, point_trail: f64) -> bool {
    point_trail > 0.0 && trail_owner_for(brokers, method) == "broker"
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

/// Presence-only credential report for the diagnostics bundle. NEVER returns a
/// stored value: a secret or field is reported as filled/empty only, so the
/// operator can share the bundle safely.
pub fn presence_json() -> Value {
    let s = state();
    let mut present: Vec<&str> = Vec::new();
    let mut partial: Vec<Value> = Vec::new();
    let mut enabled: Vec<&str> = Vec::new();
    let mut accounts = serde_json::Map::new();
    for b in CATALOG.iter() {
        let Some(a) = s.accounts.get(b.id) else { continue };
        if a.enabled {
            enabled.push(b.id);
        }
        let mut counts = serde_json::Map::new();
        let mut filled = 0usize;
        let mut required_missing = 0usize;
        for f in b.fields.iter() {
            let has = a
                .fields
                .get(f.key)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            counts.insert(
                f.key.to_string(),
                json!({
                    "present": has,
                    "secret": f.secret,
                    "required": f.required,
                }),
            );
            if has {
                filled += 1;
            } else if f.required {
                required_missing += 1;
            }
        }
        if filled > 0 && required_missing == 0 {
            present.push(b.id);
        } else if filled > 0 {
            partial.push(json!({"id": b.id, "missingRequired": required_missing}));
        }
        accounts.insert(b.id.to_string(), Value::Object(counts));
    }
    json!({
        "secretMask": SECRET_MASK,
        "note": "Values are never included - only present/absent.",
        "present": present,
        "enabled": enabled,
        "partial": partial,
        "accounts": Value::Object(accounts),
    })
}

pub async fn catalog_get() -> Json<Value> {
    Json(catalog_json())
}

pub async fn account_post(Json(v): Json<Value>) -> Json<Value> {
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if id.is_empty() {
        return Json(json!({ "ok": false, "error": "id required" }));
    }
    let enabled = v.get("enabled").and_then(|x| x.as_bool());
    let fields = v.get("fields").and_then(|x| x.as_object());
    match update_account(&id, enabled, fields) {
        Ok(()) => Json(catalog_json()),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

/// Test one broker's stored credentials with a live authenticated call.
///
/// Every registered broker now has a real order adapter, so the Test button
/// proves the credentials instead of reporting a build limitation. A configured
/// broker whose login call fails is reported with the broker's own message.

/// Shape test for one credential value. Returns a human-readable reason when
/// the value cannot possibly be right, so a mangled paste is caught before a
/// network call (a token from another broker, a truncated key, a non-numeric
/// client id). Never used to *prove* a value - only to reject obvious mistakes.
fn shape_error(f: &FieldSpec, raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    let ok = match f.format {
        "" => true,
        "uuid" => {
            // 8-4-4-4-12 hex, case-insensitive.
            let parts: Vec<&str> = v.split('-').collect();
            parts.len() == 5
                && parts
                    .iter()
                    .zip([8, 4, 4, 4, 12])
                    .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
        }
        "base32" => {
            let c: String = v.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
            !c.is_empty()
                && c.chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
                && c.chars().all(|ch| !matches!(ch, '0' | '1' | '8' | '9'))
        }
        "digits" => !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()),
        "alnum" => !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric()),
        "token" => {
            // An opaque session token / secret: printable, no spaces, and long
            // enough that a truncated paste or a stray word is rejected.
            v.len() >= 8 && v.chars().all(|c| !c.is_whitespace() && !c.is_control())
        }
        _ => true,
    };
    if ok {
        None
    } else {
        Some(format!(
            "{} looks wrong for {} (expected {})",
            f.key, f.label, f.format
        ))
    }
}

/// Credential report for a Test: which stored fields are present, which fail a
/// shape check, and which can never be proven by a session probe (`login_only`).
///
/// This exists because a session probe only exercises the fields the broker puts
/// on an authenticated call. Upstox's profile call never looks at `apiKey` /
/// `apiSecret`, so before this a wrong key pair + a still-valid token reported
/// "connected" - the exact bug operators hit after "switching" a working account
/// to wrong credentials.
pub fn credential_audit(spec: &BrokerSpec) -> Value {
    let acct = state().accounts.get(spec.id).cloned().unwrap_or_default();
    let mut invalid: Vec<String> = Vec::new();
    let mut unverified: Vec<String> = Vec::new();
    let mut present: BTreeMap<String, bool> = BTreeMap::new();
    for f in spec.fields {
        let raw = acct.field(f.key);
        let has = !raw.is_empty();
        present.insert(f.key.to_string(), has);
        if !has {
            continue;
        }
        if let Some(e) = shape_error(f, raw) {
            invalid.push(e);
        }
        // A login-only field is only "unverified" if the probe does not send
        // it. Upstox's probe validates the key pair, so it proves those fields.
        if f.login_only && !spec.probe_proves.contains(&f.key) {
            unverified.push(f.key.to_string());
        }
    }
    json!({
        "present": present,
        "invalid": invalid,
        "unverified": unverified,
    })
}

pub async fn test_post(Json(v): Json<Value>) -> Json<Value> {
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").trim().to_ascii_lowercase();
    let Some(spec) = spec(&id) else {
        return Json(json!({ "ok": false, "error": format!("unknown broker: {id}") }));
    };
    if spec.credentials_in_header {
        // Dhan's keys live in the header Connect box, not in this form. Report
        // the header's own probe result instead of demanding duplicate inputs.
        return match crate::adapters::probe(spec.id).await {
            Ok(who) => Json(json!({
                "ok": true,
                "status": "connected",
                "message": who,
                "source": "header",
                "credentials": crate::adapters::credential_presence(spec.id),
                "instruments": crate::instruments::warm(spec.id).await["status"].clone(),
            })),
            Err(e) => Json(json!({
                "ok": false,
                "status": "error",
                "source": "header",
                "error": format!("Header Dhan Connect check failed: {e}"),
            })),
        };
    }
    let missing: Vec<&'static str> = state()
        .accounts
        .get(spec.id)
        .map(|a| a.missing(spec))
        .unwrap_or_else(|| BrokerAccount::default().missing(spec));
    if !missing.is_empty() {
        return Json(json!({ "ok": false, "status": "incomplete", "missing": missing }));
    }
    // Reject an obviously wrong value BEFORE the network call. A token from
    // another broker or a truncated key would otherwise ride on a valid session
    // token and be reported as "connected".
    let audit = credential_audit(spec);
    let invalid: Vec<String> = audit["invalid"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    if !invalid.is_empty() {
        return Json(json!({
            "ok": false,
            "status": "invalid",
            "error": invalid.join("; "),
            "credentials": audit,
        }));
    }
    match crate::adapters::probe(spec.id).await {
        Ok(who) => {
            // Only the fields a probe actually sends are proven. If the broker
            // also needs login-only secrets, say so instead of implying the
            // whole account is verified.
            let unverified: Vec<String> = audit["unverified"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            Json(json!({
                "ok": true,
                "status": "connected",
                "message": who,
                "verified": "session",
                "unverified": unverified,
                "credentials": audit,
                "instruments": crate::instruments::warm(spec.id).await["status"].clone(),
            }))
        }
        Err(e) => Json(json!({
            "ok": false,
            "status": "error",
            "error": e,
            "credentials": audit,
        })),
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn data_dir() -> PathBuf {
    let dir = std::env::var("ALGODHAN_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn data_path() -> PathBuf {
    data_dir().join(STATE_FILE)
}

/// Tests all share the process-wide store, so a write from a unit test must not
/// drop a fake account into the operator's real `data/` folder.
#[cfg(test)]
static TEST_NO_PERSIST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn persist(v: &BrokerStore) {
    #[cfg(test)]
    if TEST_NO_PERSIST.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(v) {
        let path = data_path();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

static STORE: OnceLock<Mutex<BrokerStore>> = OnceLock::new();

fn state() -> MutexGuard<'static, BrokerStore> {
    STORE
        .get_or_init(|| Mutex::new(load()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn load() -> BrokerStore {
    let mut s = std::fs::read_to_string(data_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<BrokerStore>(&raw).ok())
        .unwrap_or_default();
    // Drop unknown broker ids/fields from a hand-edited or older file so the
    // stored document always matches the current catalogue.
    s.accounts.retain(|id, _| spec(id).is_some());
    for (id, acct) in s.accounts.iter_mut() {
        if let Some(sp) = spec(id) {
            acct.fields.retain(|k, _| sp.fields.iter().any(|f| f.key == k));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test mutates the same process-wide store, so they must not run
    /// concurrently - otherwise one test's `clear_for_test` wipes another's
    /// fixtures mid-assertion.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn clear_for_test() {
        TEST_NO_PERSIST.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut s = state();
        s.accounts.clear();
        s.updated_at = 0;
    }

    #[test]
    fn catalogue_registers_every_requested_broker() {
        let _g = test_guard();
        for id in [
            "dhan", "icici", "angel", "alice", "fivepaisa", "shoonya", "fyers", "kotak", "zerodha",
            "upstox",
        ] {
            let sp = spec(id).unwrap_or_else(|| panic!("missing broker {id}"));
            assert!(!sp.fields.is_empty(), "{id} has no credential fields");
            assert!(sp.fields.iter().any(|f| f.required), "{id} has no required field");
        }
    }

    #[test]
    fn dhan_credentials_come_from_the_header_not_the_settings_form() {
        let _g = test_guard();
        // The Dhan card must not ask for a second copy of the keys that the
        // header Connect box already owns; if the flag were lost the operator
        // would see two competing credential sources for one account.
        assert!(spec("dhan").unwrap().credentials_in_header);
        for b in CATALOG.iter().filter(|b| b.id != "dhan") {
            assert!(!b.credentials_in_header, "{} must keep its own form", b.id);
        }
    }

    #[test]
    fn every_catalogue_broker_has_a_live_order_adapter() {
        let _g = test_guard();
        // Every registered broker must be deliverable: a broker that can be
        // switched ON in Broker Settings but cannot receive an order would let
        // the operator believe a trade was sent when it was not.
        for b in CATALOG {
            assert!(b.adapter_ready, "{} has no order adapter", b.id);
        }
    }

    #[test]
    fn secrets_are_masked_and_masked_posts_keep_the_stored_value() {
        let _g = test_guard();
        clear_for_test();
        let mut fields = serde_json::Map::new();
        fields.insert("clientId".into(), json!("1000000001"));
        fields.insert("accessToken".into(), json!("real-token"));
        update_account("dhan", Some(true), Some(&fields)).unwrap();

        let view = catalog_json();
        let dhan = view["brokers"].as_array().unwrap().iter().find(|b| b["id"] == "dhan").unwrap();
        assert_eq!(dhan["account"]["fields"]["accessToken"], json!(SECRET_MASK));
        assert_eq!(dhan["account"]["fields"]["clientId"], json!("1000000001"));
        assert_eq!(dhan["account"]["usable"], json!(true));

        // A round-tripped form (mask + unchanged client id) must not erase the
        // secret or disable the account.
        let mut echo = serde_json::Map::new();
        echo.insert("clientId".into(), json!("1000000001"));
        echo.insert("accessToken".into(), json!(SECRET_MASK));
        update_account("dhan", None, Some(&echo)).unwrap();
        let s = state();
        let acct = s.accounts.get("dhan").unwrap();
        assert_eq!(acct.field("accessToken"), "real-token");
        assert!(acct.enabled);
        drop(s);
        clear_for_test();
    }

    #[test]
    fn a_wrong_shaped_field_is_flagged_before_any_network_call() {
        let _g = test_guard();
        clear_for_test();
        // A token pasted from another broker is not a UUID; it can never be a
        // valid Upstox api key, so the audit must say so even though the token
        // itself is syntactically fine.
        let mut fields = serde_json::Map::new();
        fields.insert("apiKey".into(), json!("not-a-uuid"));
        fields.insert("apiSecret".into(), json!("s3cretvalue"));
        fields.insert("accessToken".into(), json!("a-valid-looking-token"));
        update_account("upstox", Some(true), Some(&fields)).unwrap();

        let audit = credential_audit(spec("upstox").unwrap());
        let invalid: Vec<&str> = audit["invalid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(invalid.len(), 1, "only the api key is malformed: {invalid:?}");
        assert!(invalid[0].contains("apiKey"), "{invalid:?}");
        // Upstox's probe validates the key pair through the token endpoint, so
        // no field is left unverified once the shapes are sound.
        let un: Vec<&str> = audit["unverified"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert!(un.is_empty(), "{un:?}");
        clear_for_test();
    }

    #[test]
    fn well_formed_credentials_pass_the_audit() {
        let _g = test_guard();
        clear_for_test();
        let mut fields = serde_json::Map::new();
        fields.insert("apiKey".into(), json!("aedc36be-1579-4cf6-96f9-448cad35ee24"));
        fields.insert("apiSecret".into(), json!("test-secret-value"));
        fields.insert("accessToken".into(), json!("eyJhbGciOiJIUzI1NiJ9.payload.sig"));
        update_account("upstox", Some(true), Some(&fields)).unwrap();
        assert!(credential_audit(spec("upstox").unwrap())["invalid"].as_array().unwrap().is_empty());
        clear_for_test();
    }

    #[test]
    fn non_numeric_dhan_client_id_is_rejected() {
        let _g = test_guard();
        clear_for_test();
        let mut fields = serde_json::Map::new();
        fields.insert("clientId".into(), json!("110929445x"));
        fields.insert("accessToken".into(), json!("kite-token-123456"));
        update_account("dhan", Some(true), Some(&fields)).unwrap();
        let audit = credential_audit(spec("dhan").unwrap());
        assert_eq!(audit["invalid"].as_array().unwrap().len(), 1);
        clear_for_test();
    }

    #[test]
    fn an_enabled_but_incomplete_broker_is_not_usable() {
        let _g = test_guard();
        clear_for_test();
        update_account("zerodha", Some(true), None).unwrap();
        assert!(!enabled_configured().contains(&"zerodha"));
        let view = catalog_json();
        let z = view["brokers"].as_array().unwrap().iter().find(|b| b["id"] == "zerodha").unwrap();
        assert_eq!(z["account"]["usable"], json!(false));
        assert!(z["account"]["missing"].as_array().unwrap().len() >= 3);
        clear_for_test();
    }

    #[test]
    fn untouched_settings_keep_the_legacy_dhan_route() {
        let _g = test_guard();
        let empty = BTreeMap::new();
        assert_eq!(resolve_route(&empty, &[], false), vec!["dhan".to_string()]);
        // A never-routed engine sends to every usable broker, so opening the
        // Broker Settings tab can never silently stop an existing book.
        assert_eq!(
            resolve_route(&empty, &["dhan", "zerodha"], true),
            vec!["dhan".to_string(), "zerodha".to_string()]
        );
        // An explicit all-off map refuses to trade instead of falling back.
        let mut all_off = BTreeMap::new();
        all_off.insert("dhan".to_string(), false);
        assert!(resolve_route(&all_off, &["dhan"], true).is_empty());
    }

    #[test]
    fn a_route_only_keeps_brokers_that_are_enabled_and_fully_configured() {
        let _g = test_guard();
        let mut map = BTreeMap::new();
        map.insert("dhan".to_string(), true);
        map.insert("zerodha".to_string(), true);
        map.insert("angel".to_string(), false);
        let usable = ["dhan", "angel"];
        assert_eq!(resolve_route(&map, &usable, true), vec!["dhan".to_string()]);
        // A ticked broker that lost its credentials is dropped, not misrouted;
        // with nothing left deliverable the entry is refused.
        assert!(resolve_route(&map, &[], true).is_empty());
    }

    #[test]
    fn trail_owner_prefers_the_broker_only_for_native_methods() {
        let _g = test_guard();
        assert_eq!(trail_owner("dhan", "super"), "broker");
        // Forever / GTT carries no trailing leg, so the engine owns the trail.
        assert_eq!(trail_owner("dhan", "forever"), "engine");
        assert_eq!(trail_owner("dhan", "normal"), "engine");
        assert_eq!(trail_owner("dhan", "slice"), "engine");
        // Zerodha / Upstox never trail broker-side.
        assert_eq!(trail_owner("zerodha", "super"), "engine");
        assert_eq!(trail_owner("upstox", "super"), "engine");
        // A mixed fan-out is owned by the engine so only one trail ever runs.
        let mixed = vec!["dhan".to_string(), "zerodha".to_string()];
        assert_eq!(trail_owner_for(&mixed, "super"), "engine");
        // Only Dhan sends a native trailing leg in this build; every other
        // adapter places a plain order, so a Dhan + Fyers fan-out is still
        // engine-owned (never two ratchets on one position).
        let dhan_fyers = vec!["dhan".to_string(), "fyers".to_string()];
        assert_eq!(trail_owner_for(&dhan_fyers, "super"), "engine");
        assert_eq!(trail_owner("fyers", "super"), "engine");
    }

    #[test]
    fn broker_owns_the_trail_only_with_a_configured_jump() {
        let _g = test_guard();
        let dhan = vec!["dhan".to_string()];
        assert!(broker_owns_trail(&dhan, "super", 5.0));
        // Native method but no jump configured -> the broker would trail
        // nothing, so the engine must keep its own trail.
        assert!(!broker_owns_trail(&dhan, "super", 0.0));
        // Non-native method never hands the trail to the broker.
        assert!(!broker_owns_trail(&dhan, "normal", 5.0));
        // Paper never routes to a broker, so the engine always owns the trail.
        assert!(!broker_owns_trail(&[], "super", 5.0));
    }
}
