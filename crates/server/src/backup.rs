//! Backup & Restore system for the complete algo suite (Rust port).
//!
//! Direct port of the old Flask app's `/api/backup/*` endpoints and
//! `static/backup.js`. The whole algo system state persists in the browser's
//! localStorage, so a backup is simply a snapshot of every localStorage key.
//!
//! Endpoints (identical shape to the old app):
//!   GET  /api/backup/config      -> { ok, config }
//!   POST /api/backup/config      -> { ok, config }
//!   POST /api/backup/path_test   -> { ok, message, resolved }
//!   POST /api/backup/snapshot    -> { ok, file, created_at, list, config }
//!   GET  /api/backup/list        -> { ok, list, config }
//!   GET  /api/backup/read?file=  -> { ok, file, backup }
//!   POST /api/backup/import      -> { ok, file, created_at, message, list, config }
//!
//! The schedule config is stored server-side in `backup_config.json` next to the
//! server crate (mirrors the old `backup_config.json` next to `app.py`) so it
//! survives even if the browser profile is cleared.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

/// 50 MB safety cap on a single snapshot payload.
const MAX_PAYLOAD: usize = 50 * 1024 * 1024;
/// Hard cap on the number of `.json` files kept on disk.
const MAX_FILES: usize = 300;

static LOCK: Mutex<()> = Mutex::new(());
static SEQ: AtomicU64 = AtomicU64::new(0);

fn config_path() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/backup_config.json"))
}

// ---------------------------------------------------------------------------
// Time helpers (naive local time, matching Python's datetime.now().isoformat()).
// The server runs in UTC; the old app used the server's local clock too.
// ---------------------------------------------------------------------------

/// Civil date from a Unix day number (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Days since 1970-01-01 for a civil date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe
}

/// ISO-8601 naive timestamp (no timezone) from Unix seconds + microseconds.
fn iso_from_unix(secs: i64, micros: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    if micros == 0 {
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, d, h, mi, s)
    } else {
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}", y, mo, d, h, mi, s, micros)
    }
}

fn iso_from_secs(secs: i64) -> String {
    iso_from_unix(secs, 0)
}

fn now_parts() -> (i64, u32) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros()),
        Err(_) => (0, 0),
    }
}

fn now_iso() -> String {
    let (s, u) = now_parts();
    iso_from_unix(s, u)
}

fn now_secs() -> i64 {
    now_parts().0
}

/// Parse a naive/ISO timestamp into Unix seconds (fraction discarded,
/// timezone offsets ignored - our own timestamps carry no offset).
fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 10 {
        return None;
    }
    let p = |a: usize, b: usize| -> Option<i64> { s.get(a..b).and_then(|x| x.parse::<i64>().ok()) };
    let y = p(0, 4)?;
    let mo = p(5, 7)?;
    let d = p(8, 10)?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut secs = days_from_civil(y, mo as u32, d as u32) * 86_400;
    if s.len() >= 19 && (s.as_bytes().get(10) == Some(&b'T') || s.as_bytes().get(10) == Some(&b' ')) {
        let h = p(11, 13).unwrap_or(0);
        let mi = p(14, 16).unwrap_or(0);
        let se = p(17, 19).unwrap_or(0);
        secs += h * 3600 + mi * 60 + se;
    }
    Some(secs)
}

// ---------------------------------------------------------------------------
// Config load/save (mirrors _backup_load_config / _backup_save_config)
// ---------------------------------------------------------------------------

fn default_config() -> Value {
    json!({
        "path": "",
        "enabled": false,
        "schedule": {
            "type": "minute",
            "interval": 5,
            "time": "18:00",
            "weekday": 0,
            "keep": 30,
        },
        "lastBackup": null,
        "nextDue": null,
        "lastResult": { "ok": true, "message": "ready", "at": null, "file": null },
    })
}

fn load_config() -> Value {
    let mut cfg = default_config();
    if let Ok(raw) = fs::read_to_string(config_path()) {
        if let Ok(saved) = serde_json::from_str::<Value>(&raw) {
            if saved.is_object() {
                let obj = cfg.as_object_mut().unwrap();
                for k in ["path", "enabled", "lastBackup", "nextDue"] {
                    if let Some(v) = saved.get(k) {
                        obj.insert(k.to_string(), v.clone());
                    }
                }
                if let Some(sch) = saved.get("schedule").and_then(|v| v.as_object()) {
                    if let Some(slot) = obj.get_mut("schedule").and_then(|v| v.as_object_mut()) {
                        for k in ["type", "time", "weekday", "keep", "interval"] {
                            if let Some(v) = sch.get(k) {
                                slot.insert(k.to_string(), v.clone());
                            }
                        }
                    }
                }
                if let Some(lr) = saved.get("lastResult").and_then(|v| v.as_object()) {
                    if let Some(slot) = obj.get_mut("lastResult").and_then(|v| v.as_object_mut()) {
                        for (k, v) in lr {
                            slot.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }
    cfg
}

fn save_config(cfg: &Value) -> bool {
    let pretty = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".to_string());
    fs::write(config_path(), pretty).is_ok()
}

// ---------------------------------------------------------------------------
// Path / file helpers
// ---------------------------------------------------------------------------

fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return format!("{}/{}", home.trim_end_matches('/'), rest);
            }
        }
    } else if p == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    p.to_string()
}

/// Resolve the configured backup path, creating it when asked to.
fn backup_path(cfg: &Value, create: bool) -> Option<PathBuf> {
    let raw = cfg.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        return None;
    }
    let expanded = expand_home(raw);
    let p = PathBuf::from(expanded);
    if create {
        let _ = fs::create_dir_all(&p);
    }
    Some(p)
}

fn file_mtime_ms(p: &Path) -> i64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn file_name(kind: &str) -> String {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let (s, u) = now_parts();
    let days = s.div_euclid(86_400);
    let rem = s.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let stamp = format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}{:03}",
        y,
        mo,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        u / 1000
    );
    format!("backup_{}_{}_{:03}.json", kind, stamp, seq % 1000)
}

/// List `.json` files in the backup path: `{ name, size, mtime }`, newest first.
fn list_files(cfg: &Value) -> Vec<Value> {
    let Some(p) = backup_path(cfg, false) else {
        return Vec::new();
    };
    if !p.is_dir() {
        return Vec::new();
    }
    let mut out: Vec<(i64, Value)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&p) {
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = ent.file_name().to_string_lossy().to_string();
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let mtime_ms = file_mtime_ms(&path);
            out.push((
                mtime_ms,
                json!({
                    "name": name,
                    "size": size,
                    "mtime": iso_from_unix(mtime_ms.div_euclid(1000), (mtime_ms.rem_euclid(1000) * 1000) as u32),
                }),
            ));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// Keep only the newest `keep` auto snapshots, then hard-cap total files.
fn prune(cfg: &Value) {
    let Some(p) = backup_path(cfg, false) else { return };
    if !p.is_dir() {
        return;
    }
    let keep = cfg
        .get("schedule")
        .and_then(|s| s.get("keep"))
        .and_then(|v| v.as_i64())
        .unwrap_or(30)
        .max(1) as usize;

    let mut auto_files: Vec<(i64, PathBuf)> = Vec::new();
    let mut all_files: Vec<(i64, PathBuf)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&p) {
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = ent.file_name().to_string_lossy().to_string();
            let mt = file_mtime_ms(&path);
            if name.starts_with("backup_auto_") {
                auto_files.push((mt, path.clone()));
            }
            all_files.push((mt, path));
        }
    }
    auto_files.sort_by(|a, b| a.0.cmp(&b.0));
    let drop_count = auto_files.len().saturating_sub(keep);
    for (_, path) in auto_files.into_iter().take(drop_count) {
        let _ = fs::remove_file(path);
    }
    all_files.sort_by(|a, b| a.0.cmp(&b.0));
    let over = all_files.len().saturating_sub(MAX_FILES);
    for (_, path) in all_files.into_iter().take(over) {
        let _ = fs::remove_file(path);
    }
}

/// Write one snapshot JSON to the configured path. Returns `(file, created_at)`.
fn write_snapshot(cfg: &Value, payload: &Value, kind: &str) -> Result<(String, String), String> {
    if !payload.get("data").map(|d| d.is_object()).unwrap_or(false) {
        return Err("snapshot payload missing 'data'".to_string());
    }
    let raw = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    if raw.len() > MAX_PAYLOAD {
        return Err(format!("snapshot too large ({} bytes > {} limit)", raw.len(), MAX_PAYLOAD));
    }
    let p = backup_path(cfg, true).ok_or_else(|| "no backup path configured".to_string())?;
    if !p.is_dir() {
        return Err(format!("backup path is not a directory: {}", p.display()));
    }
    let name = file_name(kind);
    let fp = p.join(&name);
    fs::write(&fp, &raw).map_err(|e| e.to_string())?;
    // Keep a stable "latest.json" copy for quick restore (best effort).
    let _ = fs::write(p.join("latest.json"), &raw);
    Ok((name, now_iso()))
}

// ---------------------------------------------------------------------------
// Schedule math (mirrors _backup_next_due / _backup_last_scheduled / _backup_due_next)
// ---------------------------------------------------------------------------

struct Schedule {
    kind: String,
    interval: i64,
    hh: i64,
    mm: i64,
    weekday: i64,
}

fn schedule_of(cfg: &Value) -> Schedule {
    let sch = cfg.get("schedule");
    let kind = sch
        .and_then(|s| s.get("type"))
        .and_then(|v| v.as_str())
        .filter(|t| matches!(*t, "minute" | "hour" | "daily" | "weekly"))
        .unwrap_or("daily")
        .to_string();
    let interval = sch
        .and_then(|s| s.get("interval"))
        .and_then(|v| v.as_i64())
        .unwrap_or(5)
        .max(1);
    let (hh, mm) = sch
        .and_then(|s| s.get("time"))
        .and_then(|v| v.as_str())
        .and_then(|t| {
            let mut it = t.split(':');
            let a = it.next()?.trim().parse::<i64>().ok()?;
            let b = it.next()?.trim().parse::<i64>().ok()?;
            Some((a, b))
        })
        .unwrap_or((18, 0));
    let weekday = sch
        .and_then(|s| s.get("weekday"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .rem_euclid(7);
    Schedule { kind, interval, hh, mm, weekday }
}

fn weekday_of(secs: i64) -> i64 {
    (secs.div_euclid(86_400) + 3).rem_euclid(7)
}

fn day_start(secs: i64) -> i64 {
    secs.div_euclid(86_400) * 86_400
}

fn next_due(cfg: &Value, now: i64) -> String {
    let s = schedule_of(cfg);
    match s.kind.as_str() {
        "minute" => iso_from_secs(now + s.interval * 60),
        "hour" => iso_from_secs(now + s.interval * 3600),
        "weekly" => {
            let target = s.weekday;
            let delta = (target - weekday_of(now)).rem_euclid(7);
            let mut cand = day_start(now) + s.hh * 3600 + s.mm * 60 + delta * 86_400;
            if cand <= now {
                cand += 7 * 86_400;
            }
            iso_from_secs(cand)
        }
        _ => {
            let mut cand = day_start(now) + s.hh * 3600 + s.mm * 60;
            if cand <= now {
                cand += 86_400;
            }
            iso_from_secs(cand)
        }
    }
}

fn last_scheduled(cfg: &Value, now: i64) -> String {
    let s = schedule_of(cfg);
    match s.kind.as_str() {
        "minute" => iso_from_secs(now - s.interval * 60),
        "hour" => iso_from_secs(now - s.interval * 3600),
        "weekly" => {
            let target = s.weekday;
            let delta = (weekday_of(now) - target).rem_euclid(7);
            let mut cand = day_start(now) + s.hh * 3600 + s.mm * 60 - delta * 86_400;
            if cand > now {
                cand -= 7 * 86_400;
            }
            iso_from_secs(cand)
        }
        _ => {
            let mut cand = day_start(now) + s.hh * 3600 + s.mm * 60;
            if cand > now {
                cand -= 86_400;
            }
            iso_from_secs(cand)
        }
    }
}

/// Effective next-due for reads: if a scheduled time passed since the last
/// backup (or there has never been one) the backup is due NOW for catch-up.
fn due_next(cfg: &Value, now: i64) -> String {
    if !cfg.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
        return next_due(cfg, now);
    }
    let ls_ts = parse_iso(&last_scheduled(cfg, now));
    let last_bk = cfg.get("lastBackup").and_then(|v| v.as_str()).and_then(parse_iso);
    if let Some(ls) = ls_ts {
        if last_bk.map(|b| ls > b).unwrap_or(true) {
            return iso_from_secs(now);
        }
    }
    next_due(cfg, now)
}

// ---------------------------------------------------------------------------
// Endpoint implementations (called from async handlers; no guards held across .await)
// ---------------------------------------------------------------------------

fn config_read() -> Value {
    let _g = LOCK.lock().unwrap();
    let mut cfg = load_config();
    let nd = due_next(&cfg, now_secs());
    cfg["nextDue"] = json!(nd);
    save_config(&cfg);
    json!({ "ok": true, "config": cfg })
}

fn config_write(body: &Value) -> Value {
    let _g = LOCK.lock().unwrap();
    let mut cfg = load_config();
    {
        let obj = cfg.as_object_mut().unwrap();
        if let Some(p) = body.get("path") {
            obj.insert("path".into(), json!(p.as_str().unwrap_or("").trim()));
        }
        if let Some(e) = body.get("enabled") {
            obj.insert("enabled".into(), json!(e.as_bool().unwrap_or(false)));
        }
        if let Some(sch) = body.get("schedule").and_then(|v| v.as_object()) {
            let slot = obj.get_mut("schedule").and_then(|v| v.as_object_mut()).unwrap();
            if let Some(t) = sch.get("type").and_then(|v| v.as_str()) {
                if matches!(t, "minute" | "hour" | "daily" | "weekly") {
                    slot.insert("type".into(), json!(t));
                }
            }
            if let Some(iv) = sch.get("interval").and_then(|v| v.as_i64()) {
                slot.insert("interval".into(), json!(iv.clamp(1, 100_000)));
            }
            if let Some(t) = sch.get("time").and_then(|v| v.as_str()) {
                if t.contains(':') {
                    slot.insert("time".into(), json!(t));
                }
            }
            if let Some(wd) = sch.get("weekday").and_then(|v| v.as_i64()) {
                slot.insert("weekday".into(), json!(wd.rem_euclid(7)));
            }
            if let Some(k) = sch.get("keep").and_then(|v| v.as_i64()) {
                slot.insert("keep".into(), json!(k.max(1)));
            }
        }
    }
    let nd = due_next(&cfg, now_secs());
    cfg["nextDue"] = json!(nd);
    save_config(&cfg);
    json!({ "ok": true, "config": cfg })
}

fn path_test(body: &Value) -> (StatusCode, Value) {
    let raw = body.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        return (StatusCode::BAD_REQUEST, json!({ "ok": false, "message": "No path given" }));
    }
    let resolved = expand_home(raw);
    let p = PathBuf::from(&resolved);
    match fs::create_dir_all(&p) {
        Ok(_) => {
            let probe = p.join(".backup_probe.tmp");
            match fs::write(&probe, b"ok") {
                Ok(_) => {
                    let _ = fs::remove_file(&probe);
                    (
                        StatusCode::OK,
                        json!({ "ok": true, "message": format!("Path writable: {}", p.display()), "resolved": resolved }),
                    )
                }
                Err(e) => (
                    StatusCode::OK,
                    json!({ "ok": false, "message": format!("Path not writable: {}", e), "resolved": resolved }),
                ),
            }
        }
        Err(e) => (
            StatusCode::OK,
            json!({ "ok": false, "message": format!("Path not writable: {}", e), "resolved": resolved }),
        ),
    }
}

fn snapshot_write(body: &Value) -> (StatusCode, Value) {
    let data = body.get("data").cloned();
    let kind = match body.get("kind").and_then(|v| v.as_str()) {
        Some(k) if matches!(k, "auto" | "manual" | "import" | "pre_restore") => k.to_string(),
        _ => "manual".to_string(),
    };
    let Some(data) = data.filter(|d| d.is_object()) else {
        return (StatusCode::BAD_REQUEST, json!({ "ok": false, "message": "Missing snapshot data" }));
    };
    let pc_only = body.get("pc_only").and_then(|v| v.as_bool()).unwrap_or(false);

    let _g = LOCK.lock().unwrap();
    let mut cfg = load_config();
    let key_count = data
        .get("localStorage")
        .and_then(|v| v.as_object())
        .map(|m| m.len())
        .unwrap_or(0);
    let created = json!({
        "format": "algodhan_backup",
        "version": 1,
        "created_at": now_iso(),
        "kind": kind,
        "app": "Smart NTrader + Algo Suite",
        "count": { "keys": key_count },
        "data": data,
        "config": cfg,
    });

    let (name, at): (Option<String>, String) = if pc_only {
        (None, now_iso())
    } else {
        match write_snapshot(&cfg, &created, &kind) {
            Ok((n, at)) => (Some(n), at),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    json!({ "ok": false, "message": e }),
                )
            }
        }
    };

    if kind == "auto" {
        cfg["lastBackup"] = json!(at);
        cfg["nextDue"] = json!(next_due(&cfg, now_secs()));
        let msg = if pc_only { "auto backup saved to PC" } else { "auto backup saved" };
        cfg["lastResult"] = json!({ "ok": true, "message": msg, "at": at, "file": name });
        save_config(&cfg);
        if !pc_only {
            prune(&cfg);
        }
    }
    (
        StatusCode::OK,
        json!({ "ok": true, "file": name, "created_at": at, "list": list_files(&cfg), "config": cfg }),
    )
}

fn list_read() -> Value {
    let _g = LOCK.lock().unwrap();
    let mut cfg = load_config();
    cfg["nextDue"] = json!(due_next(&cfg, now_secs()));
    json!({ "ok": true, "list": list_files(&cfg), "config": cfg })
}

fn read_file(name: &str) -> (StatusCode, Value) {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if !base.ends_with(".json") || base.contains("..") {
        return (StatusCode::BAD_REQUEST, json!({ "ok": false, "message": "Invalid file name" }));
    }
    let _g = LOCK.lock().unwrap();
    let cfg = load_config();
    let Some(p) = backup_path(&cfg, false) else {
        return (
            StatusCode::BAD_REQUEST,
            json!({ "ok": false, "message": "No backup path configured" }),
        );
    };
    let fp = p.join(&base);
    if !fp.is_file() {
        return (
            StatusCode::NOT_FOUND,
            json!({ "ok": false, "message": "Backup file not found" }),
        );
    }
    match fs::read_to_string(&fp).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        Some(content) => (StatusCode::OK, json!({ "ok": true, "file": base, "backup": content })),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "ok": false, "message": "Failed to read backup" }),
        ),
    }
}

fn import_payload(body: &Value) -> (StatusCode, Value) {
    let data = body.get("data").cloned();
    // A backup carries the browser localStorage (chart/UI preferences) *and*
    // the server-side engine state (settings, strategies, trade ledger). Accept
    // either, so an engine-only or an older localStorage-only file both import.
    let Some(data) = data.filter(|d| {
        d.get("localStorage")
            .map(|ls| ls.is_object())
            .unwrap_or(false)
            || d.get("engine").map(|e| e.is_object()).unwrap_or(false)
    }) else {
        return (
            StatusCode::BAD_REQUEST,
            json!({ "ok": false, "message": "Import payload must contain data.localStorage or data.engine" }),
        );
    };
    let _g = LOCK.lock().unwrap();
    let cfg = load_config();
    let key_count = data
        .get("localStorage")
        .and_then(|v| v.as_object())
        .map(|m| m.len())
        .unwrap_or(0);
    let created = json!({
        "format": "algodhan_backup",
        "version": 1,
        "created_at": now_iso(),
        "kind": "import",
        "app": "Smart NTrader + Algo Suite",
        "count": { "keys": key_count },
        "data": data,
        "config": cfg,
    });
    let mut name: Option<String> = None;
    let mut at: Option<String> = None;
    let mut msg: Option<String> = None;
    match write_snapshot(&cfg, &created, "import") {
        Ok((n, a)) => {
            name = Some(n);
            at = Some(a);
        }
        Err(e) => msg = Some(e), // path may be unconfigured; import still proceeds client-side
    }
    (
        StatusCode::OK,
        json!({ "ok": true, "file": name, "created_at": at, "message": msg, "list": list_files(&cfg), "config": cfg }),
    )
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

pub async fn config_get() -> impl IntoResponse {
    Json(config_read())
}

pub async fn config_post(Json(body): Json<Value>) -> impl IntoResponse {
    Json(config_write(&body))
}

pub async fn path_test_handler(Json(body): Json<Value>) -> impl IntoResponse {
    let (status, value) = path_test(&body);
    (status, Json(value))
}

pub async fn snapshot(Json(body): Json<Value>) -> impl IntoResponse {
    let (status, value) = snapshot_write(&body);
    (status, Json(value))
}

pub async fn list() -> impl IntoResponse {
    Json(list_read())
}

pub async fn read(Query(q): Query<HashMap<String, String>>) -> impl IntoResponse {
    let name = q.get("file").cloned().unwrap_or_default();
    let (status, value) = read_file(&name);
    (status, Json(value))
}

pub async fn import(Json(body): Json<Value>) -> impl IntoResponse {
    let (status, value) = import_payload(&body);
    (status, Json(value))
}
