//! Live-market self-diagnosis: the evidence collector that makes a bug on an
//! INSTALLED Windows machine (which nobody can log into) as debuggable as the
//! same bug reproduced in a preview environment.
//!
//! Three pieces, all local - nothing is ever uploaded:
//!
//!   1. `snapshot` - the same layered read the operator would otherwise do by
//!      hand: is the server alive, is the clock/IST sane, is the engine loop and
//!      the signal scan actually running (heartbeat, NOT the blocker list - a
//!      dead scan records no blocker), is the feed streaming, did the instrument
//!      masters load, is there margin, and what is the newest entry. Every
//!      timestamp is reported as an AGE so "stale" is unambiguous.
//!   2. `capture` - write that snapshot, the blockers and the tail of the server
//!      log into a timestamped file under `<data>/diagnostics/`. The engine loop
//!      calls it BY ITSELF when the engine is armed, the market is open and no
//!      entry has happened for a while - so the frozen moment is already on disk
//!      when the operator notices nothing is trading.
//!   3. `bundle` - one downloadable text file with every capture, the startup
//!      self-check, engine states, margin numbers, broker field PRESENCE (never
//!      a secret value), host facts and the log tail. That single file is what
//!      the operator sends over for analysis.
//!
//! Design rule: this module must never be able to break trading. Every read is
//! best-effort and a failure degrades one field, never the request.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use axum::extract::State;
use axum::response::Json;
use serde_json::{json, Value};

use crate::broker::DhanState;
use crate::realtime::{rt_engine_ids, RealtimeState};
use crate::{equity_engine, instruments, scrip, shared_margin};

/// How long the engine may be armed + market open with no entry before the loop
/// captures a diagnostic on its own.
const NO_TRADE_CAPTURE_MS: i64 = 10 * 60 * 1000;
/// Minimum gap between automatic captures, so a quiet market cannot fill the
/// disk with one file every ten minutes all day.
const AUTO_CAPTURE_GAP_MS: i64 = 30 * 60 * 1000;
/// Maximum number of capture files kept on disk.
const KEEP_CAPTURES: usize = 40;
/// Tail length of the server log written into a capture / bundle.
const LOG_TAIL_LINES: usize = 400;

static LAST_AUTO_CAPTURE: AtomicI64 = AtomicI64::new(0);
static AUTO_CAPTURE_ARMED: AtomicBool = AtomicBool::new(false);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn data_dir() -> PathBuf {
    std::env::var("ALGODHAN_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"))
}

fn capture_dir() -> PathBuf {
    let d = data_dir().join("diagnostics");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// IST `HH:MM:SS` for a wall clock, so a log line can be read without doing the
/// offset in one's head.
fn ist_hms(ms: i64) -> String {
    let secs = ms.div_euclid(1000) + 19_800;
    let s = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3_600, (s % 3_600) / 60, s % 60)
}

fn ist_date(ms: i64) -> String {
    let days = (ms.div_euclid(1000) + 19_800) / 86_400;
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil(z: i64) -> (i64, u32, u32) {
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

/// IST minute-of-day and weekday-open flag, computed here rather than trusted
/// from the engine: a wrong host clock/timezone is exactly the silent killer
/// this report has to expose.
fn ist_clock() -> Value {
    let ms = now_ms();
    let ist = ms.div_euclid(1000) + 19_800;
    let mins = ist.rem_euclid(86_400) / 60;
    let weekday = (ist.div_euclid(86_400) + 4).rem_euclid(7);
    let session = weekday != 0 && weekday != 6 && (9 * 60 + 15..=15 * 60 + 30).contains(&mins);
    json!({
        "nowMs": ms,
        "istDate": ist_date(ms),
        "istTime": ist_hms(ms),
        "istMinuteOfDay": mins,
        "weekday": weekday,
        "isWeekend": weekday == 0 || weekday == 6,
        "nseSessionOpen": session,
        "hostTz": std::env::var("TZ").unwrap_or_else(|_| "(unset)".into()),
    })
}

/// Age in seconds of a millisecond stamp (0 stamp reads as `null`, i.e. "never").
fn age_secs(stamp_ms: i64) -> Value {
    if stamp_ms <= 0 {
        return Value::Null;
    }
    json!(((now_ms() - stamp_ms).max(0) as f64 / 1000.0 * 10.0).round() / 10.0)
}

/// Everything one engine tab can be blocked by, in the order the engine itself
/// evaluates it. `engine` is a realtime engine id ("realtime", "paper", ...).
async fn engine_block(rt: &RealtimeState, dhan: &DhanState, engine: &str) -> Value {
    let (on, armed, doc_state) = rt
        .diagnostic_engine_state()
        .map(|(a, b, c)| (a, b, c))
        .unwrap_or((false, false, Value::Null));
    let connected = dhan.is_connected().await;
    let feed = dhan.feed_diagnostic();
    let cap = shared_margin::cap_for(engine).unwrap_or(0.0);
    let (locked, running) = rt.diagnostic_locked_margin();
    let blockers = rt.blockers_snapshot(30 * 60 * 1000);
    json!({
        "engine": engine,
        "engineOn": on,
        "armed": armed,
        "doc": doc_state,
        "margin": {
            "cap": (cap * 100.0).round() / 100.0,
            "locked": (locked.max(0.0) * 100.0).round() / 100.0,
            "openPositions": running,
            "free": (((cap - locked).max(0.0)) * 100.0).round() / 100.0,
            "capIsZero": cap <= 0.0,
        },
        "runtime": {
            "connected": connected,
            "feed": feed,
            "loopAgeSec": age_secs(rt.diagnostic_loop_heartbeat()),
            "loopPasses": rt.diagnostic_loop_passes(),
            "scanAgeSec": age_secs(rt.diagnostic_scan_heartbeat()),
            "scanPasses": rt.diagnostic_scan_passes(),
            "entriesPlaced": rt.diagnostic_entries_placed(),
            "lastEntryAgeSec": age_secs(rt.diagnostic_last_entry()),
        },
        "blockers": blockers,
    })
}

/// The layered live snapshot. This is the machine-readable form of the manual
/// diagnosis (server alive -> clock -> engine loop -> scan -> feed -> masters ->
/// margin -> entries -> order errors).
pub async fn snapshot_json(dhan: &DhanState, rt: &RealtimeState) -> Value {
    let engines: Vec<Value> = {
        let mut out = Vec::new();
        for id in rt_engine_ids() {
            if let Some(state) = rt.engine_handle(id) {
                out.push(engine_block(&state, dhan, id).await);
            }
        }
        out
    };
    let equity = json!({
        "paper": equity_engine::diagnostic_block("paper"),
        "live": equity_engine::diagnostic_block("live"),
    });
    let brokers = crate::brokers::presence_json();
    let masters = instruments::status_json();
    json!({
        "ok": true,
        "atMs": now_ms(),
        "atIst": format!("{} {}", ist_date(now_ms()), ist_hms(now_ms())),
        "clock": ist_clock(),
        "server": {
            "pid": std::process::id(),
            "uptimeSec": age_secs(boot_ms()),
            "exe": std::env::current_exe().ok().map(|p| p.display().to_string()),
            "dataDir": data_dir().display().to_string(),
            "version": env!("CARGO_PKG_VERSION"),
        },
        "scripMaster": {
            "status": scrip::warm_status(),
            "contracts": scrip::contract_count(),
        },
        "instrumentMasters": masters,
        "sharedMargin": shared_margin::snapshot_json(),
        "brokers": brokers,
        "engines": engines,
        "equity": equity,
    })
}

/// Process start time (ms), memoised on first use. The bundled shell restarts
/// the server often, so uptime is a useful "is this a fresh cold start?" signal.
fn boot_ms() -> i64 {
    static BOOT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *BOOT.get_or_init(now_ms)
}

fn read_log_tail(lines: usize) -> String {
    let path = data_dir().join("server.log");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return format!("(server.log not found at {})", path.display());
    };
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// Write one capture file and return its path. `reason` is a short label that
/// ends up in the file name so the operator can tell an automatic capture from a
/// manual one at a glance.
fn write_capture(snap: &Value, reason: &str) -> Result<PathBuf, String> {
    let dir = capture_dir();
    let stamp = ist_hms(now_ms()).replace(':', "");
    let name = format!(
        "{}-{}-{reason}.log",
        ist_date(now_ms()),
        stamp
    );
    let path = dir.join(name);
    let mut body = String::new();
    body.push_str("# Algo live diagnostics capture\n");
    body.push_str(&format!("# reason: {reason}\n"));
    body.push_str(&format!("# captured: {} IST\n\n", format!("{} {}", ist_date(now_ms()), ist_hms(now_ms()))));
    body.push_str("## SNAPSHOT\n");
    body.push_str(&serde_json::to_string_pretty(snap).unwrap_or_else(|_| "{}".into()));
    body.push_str("\n\n## SERVER LOG TAIL\n");
    body.push_str(&read_log_tail(LOG_TAIL_LINES));
    body.push('\n');
    std::fs::write(&path, body).map_err(|e| format!("write capture: {e}"))?;
    prune_captures();
    Ok(path)
}

/// Keep only the newest `KEEP_CAPTURES` files (oldest are removed by name, which
/// sorts chronologically because the name starts with the IST date).
fn prune_captures() {
    let dir = capture_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".log"))
        .collect();
    if names.len() <= KEEP_CAPTURES {
        return;
    }
    names.sort();
    let drop = names.len() - KEEP_CAPTURES;
    for n in names.into_iter().take(drop) {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

/// Capture now (operator button). Returns the file path plus a short summary.
pub async fn capture_now(dhan: &DhanState, rt: &RealtimeState, reason: &str) -> Result<PathBuf, String> {
    let snap = snapshot_json(dhan, rt).await;
    write_capture(&snap, reason)
}

/// Automatic capture, called from the engine loop. Fires at most once per
/// `AUTO_CAPTURE_GAP_MS` while at least one engine is armed during the NSE
/// session and no entry has landed for `NO_TRADE_CAPTURE_MS`. Never blocks
/// trading: it only reads state and writes a file.
pub async fn maybe_auto_capture(dhan: &DhanState, rt: &RealtimeState) {
    let clock_ok = ist_clock()
        .get("nseSessionOpen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !clock_ok {
        AUTO_CAPTURE_ARMED.store(false, Ordering::Relaxed);
        return;
    }
    let mut armed = false;
    let mut newest_entry = 0i64;
    for id in rt_engine_ids() {
        let Some(state) = rt.engine_handle(id) else { continue };
        if let Some((on, arm, _)) = state.diagnostic_engine_state() {
            if on && arm {
                armed = true;
            }
        }
        newest_entry = newest_entry.max(state.diagnostic_last_entry());
    }
    if !armed {
        AUTO_CAPTURE_ARMED.store(false, Ordering::Relaxed);
        return;
    }
    // Arm the clock at the first sighting so a freshly started app is not
    // captured instantly (the first scan may still be warming the feed).
    if !AUTO_CAPTURE_ARMED.swap(true, Ordering::Relaxed) {
        LAST_AUTO_CAPTURE.store(now_ms(), Ordering::Relaxed);
        return;
    }
    let now = now_ms();
    let last_capture = LAST_AUTO_CAPTURE.load(Ordering::Relaxed);
    if now - last_capture < AUTO_CAPTURE_GAP_MS {
        return;
    }
    let quiet_since = if newest_entry > 0 { newest_entry } else { last_capture };
    if now - quiet_since < NO_TRADE_CAPTURE_MS {
        return;
    }
    LAST_AUTO_CAPTURE.store(now, Ordering::Relaxed);
    let _ = capture_now(dhan, rt, "auto-noentry").await;
}

/// Startup self-check: a handful of lines written once at boot so a Windows
/// cold start is documented even if the operator never presses anything.
pub fn startup_self_check(dhan: &DhanState, rt: &RealtimeState) {
    let clock = ist_clock();
    let scrip_status = scrip::warm_status();
    let contracts = scrip::contract_count();
    let paper_cap = shared_margin::cap_for("paper").unwrap_or(0.0);
    let real_cap = shared_margin::cap_for("realtime").unwrap_or(0.0);
    let mut lines = Vec::new();
    lines.push("=== startup self-check ===".to_string());
    lines.push(format!(
        "clock: {} {} (NSE session open = {})",
        clock.get("istDate").and_then(|v| v.as_str()).unwrap_or("?"),
        clock.get("istTime").and_then(|v| v.as_str()).unwrap_or("?"),
        clock.get("nseSessionOpen").and_then(|v| v.as_bool()).unwrap_or(false),
    ));
    lines.push(format!(
        "host TZ env: {}",
        clock.get("hostTz").and_then(|v| v.as_str()).unwrap_or("?")
    ));
    lines.push(format!("data dir: {}", data_dir().display()));
    lines.push(format!("scrip master: {scrip_status} ({contracts} contracts)"));
    lines.push(format!("shared margin: paper cap Rs {paper_cap:.0}, realtime cap Rs {real_cap:.0}"));
    lines.push(format!("dhan connected: {}", rt_dhan_connected_hint(dhan)));
    for id in rt_engine_ids() {
        if let Some(state) = rt.engine_handle(id) {
            let (on, armed, doc) = state.diagnostic_engine_state().unwrap_or((false, false, Value::Null));
            let strategies = doc
                .get("strategies")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let positions = doc
                .get("positions")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            lines.push(format!(
                "engine {id}: on={on} armed={armed} strategies={strategies} openPositions={positions}"
            ));
        }
    }
    for mode in ["paper", "live"] {
        let block = equity_engine::diagnostic_block(mode);
        lines.push(format!(
            "equity {mode}: enabled={} open={} pendingPush={} dailyExit={}@{}",
            block.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
            block.get("openPositions").and_then(|v| v.as_i64()).unwrap_or(0),
            block.get("pendingPush").and_then(|v| v.as_i64()).unwrap_or(0),
            block.get("dailyExitOn").and_then(|v| v.as_bool()).unwrap_or(false),
            block.get("dailyExitTime").and_then(|v| v.as_str()).unwrap_or(""),
        ));
    }
    let brokers = crate::brokers::presence_json();
    if let Some(list) = brokers.get("present").and_then(|v| v.as_array()) {
        lines.push(format!("brokers configured: {:?}", list));
    }
    lines.push("=== end self-check ===".to_string());
    for l in &lines {
        tracing::info!("{l}");
    }
    // Also append to a dedicated boot log so the history of cold starts is kept
    // even when the operator only shares `data/server.log` from one run.
    let path = data_dir().join("startup_selfcheck.log");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "--- {} {} ---", ist_date(now_ms()), ist_hms(now_ms()))?;
            for l in &lines {
                writeln!(f, "{l}")?;
            }
            Ok(())
        });
}

fn rt_dhan_connected_hint(dhan: &DhanState) -> bool {
    // Synchronous best-effort read of the feed flag; the async `is_connected`
    // check needs a runtime handle, which boot does not have inside this fn.
    dhan.feed_up()
}

/// The single downloadable support bundle.
pub async fn bundle_text(dhan: &DhanState, rt: &RealtimeState) -> String {
    let snap = snapshot_json(dhan, rt).await;
    let mut out = String::new();
    out.push_str("=========================================================\n");
    out.push_str(" ALGO SUPPORT BUNDLE\n");
    out.push_str(" Secrets are NEVER included - only present/absent.\n");
    out.push_str("=========================================================\n\n");
    out.push_str(&format!("generated: {} IST\n", format!("{} {}", ist_date(now_ms()), ist_hms(now_ms()))));
    out.push_str(&format!("server pid: {}\n\n", std::process::id()));
    out.push_str("## SNAPSHOT\n");
    out.push_str(&serde_json::to_string_pretty(&snap).unwrap_or_else(|_| "{}".into()));
    out.push_str("\n\n## STARTUP SELF-CHECK LOG (tail)\n");
    let selfcheck = std::fs::read_to_string(data_dir().join("startup_selfcheck.log")).unwrap_or_default();
    let lines: Vec<&str> = selfcheck.lines().collect();
    let start = lines.len().saturating_sub(200);
    out.push_str(&lines[start..].join("\n"));
    out.push_str("\n\n## LATEST CAPTURES\n");
    let dir = capture_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".log"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.reverse();
    if names.is_empty() {
        out.push_str("(no captures yet)\n");
    }
    for n in names.iter().take(5) {
        out.push_str(&format!("\n----- {n} -----\n"));
        if let Ok(t) = std::fs::read_to_string(dir.join(n)) {
            // Each capture already ends with its own server-log tail, and the
            // bundle appends one below; drop the embedded copy so five captures
            // do not balloon the file with the same log block five times.
            let head = t.split("\n## SERVER LOG TAIL").next().unwrap_or(&t);
            let l: Vec<&str> = head.lines().collect();
            let s = l.len().saturating_sub(120);
            out.push_str(&l[s..].join("\n"));
            out.push('\n');
        }
    }
    out.push_str("\n\n## SERVER LOG TAIL\n");
    out.push_str(&read_log_tail(LOG_TAIL_LINES));
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

pub async fn get(State(dhan): State<DhanState>, State(rt): State<RealtimeState>) -> Json<Value> {
    Json(snapshot_json(&dhan, &rt).await)
}

#[derive(serde::Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CaptureReq {
    pub reason: Option<String>,
}

pub async fn capture(
    State(dhan): State<DhanState>,
    State(rt): State<RealtimeState>,
    Json(body): Json<CaptureReq>,
) -> Json<Value> {
    let reason = body
        .reason
        .map(|r| r.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect::<String>())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "manual".to_string());
    match capture_now(&dhan, &rt, &reason).await {
        Ok(path) => Json(json!({"ok": true, "file": path.display().to_string()})),
        Err(e) => Json(json!({"ok": false, "error": e})),
    }
}

pub async fn bundle(State(dhan): State<DhanState>, State(rt): State<RealtimeState>) -> Json<Value> {
    Json(json!({ "ok": true, "bundle": bundle_text(&dhan, &rt).await }))
}

/// Write the bundle to a file (the desktop shell cannot offer a download dialog,
/// so the operator needs a path on disk).
pub async fn bundle_file(State(dhan): State<DhanState>, State(rt): State<RealtimeState>) -> Json<Value> {
    let text = bundle_text(&dhan, &rt).await;
    let name = format!(
        "support-bundle-{}-{}.txt",
        ist_date(now_ms()),
        ist_hms(now_ms()).replace(':', "")
    );
    let path = capture_dir().join(name);
    match std::fs::write(&path, text) {
        Ok(()) => Json(json!({"ok": true, "file": path.display().to_string()})),
        Err(e) => Json(json!({"ok": false, "error": e.to_string()})),
    }
}

/// List captures so the UI can show what has already been recorded.
pub async fn captures() -> Json<Value> {
    let dir = capture_dir();
    let mut rows: Vec<Value> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    if !name.ends_with(".log") {
                        return None;
                    }
                    let size = e.metadata().ok().map(|m| m.len()).unwrap_or(0);
                    Some(json!({"file": name, "bytes": size}))
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| {
        b.get("file").and_then(|v| v.as_str()).cmp(&a.get("file").and_then(|v| v.as_str()))
    });
    Json(json!({"ok": true, "dir": capture_dir().display().to_string(), "captures": rows}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ist_clock_reports_a_plausible_session_flag() {
        let c = ist_clock();
        let mins = c.get("istMinuteOfDay").and_then(|v| v.as_i64()).unwrap();
        assert!((0..1440).contains(&mins), "minute of day out of range: {mins}");
        let open = c.get("nseSessionOpen").and_then(|v| v.as_bool()).unwrap();
        let weekend = c.get("isWeekend").and_then(|v| v.as_bool()).unwrap();
        if weekend {
            assert!(!open, "weekend must never report an open NSE session");
        }
        let time = c.get("istTime").and_then(|v| v.as_str()).unwrap();
        assert_eq!(time.len(), 8, "HH:MM:SS expected, got {time}");
    }

    #[test]
    fn age_of_a_missing_stamp_is_null_not_zero() {
        // A zero stamp means "never happened"; reporting 0 seconds would look
        // like a fresh event and hide exactly the bug we are chasing.
        assert_eq!(age_secs(0), Value::Null);
        let fresh = age_secs(now_ms());
        assert!(fresh.as_f64().unwrap() < 5.0);
    }

    #[test]
    fn ist_hms_rolls_over_at_midnight_ist() {
        // 2026-11-02 00:00:00 IST as epoch ms.
        let midnight = (1_793_557_800i64) * 1000;
        assert_eq!(ist_hms(midnight), "00:00:00");
        assert_eq!(ist_hms(midnight + 15 * 3_600_000 + 20 * 60_000), "15:20:00");
        assert_eq!(ist_date(midnight), "2026-11-02");
    }
}
