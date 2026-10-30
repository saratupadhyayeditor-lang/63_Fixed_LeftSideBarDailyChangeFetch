//! Process-wide order-rate gates shared by the three paper engines and the
//! three live engines.
//!
//! Dhan counts every order API *request* (not fills) and rejects bursts above
//! its documented ceiling, so the cap must be enforced across engines, not per
//! tab. The three paper tabs share one sliding window, the three realtime tabs
//! share another. Caps are operator-editable in the tab header and persisted
//! next to the engine state files.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Conservative default: Dhan's documented ceiling is 10 order requests/sec, so
/// 6 leaves headroom for the broker's own retries and network jitter.
pub const DEFAULT_OPS: i64 = 6;
/// Documented Dhan maximum order requests per second.
pub const MAX_OPS: i64 = 10;
const MIN_OPS: i64 = 1;
const WINDOW: Duration = Duration::from_secs(1);
const ORDER_RATE_FILE: &str = "algodhan_order_rate.json";

/// Which independent order budget an engine draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Paper Trade 1 + 2 + 3 (simulated fills, but the operator wants the same
    /// realistic throughput envelope).
    Paper,
    /// Realtime Trading Engine 1 + 2 + 3 (real Dhan orders).
    Real,
}

impl Group {
    fn index(self) -> usize {
        match self {
            Group::Paper => 0,
            Group::Real => 1,
        }
    }
}

fn clamp_ops(ops: i64) -> i64 {
    ops.clamp(MIN_OPS, MAX_OPS)
}

struct Window {
    hits: Mutex<VecDeque<Instant>>,
}

impl Window {
    fn new() -> Self {
        Self { hits: Mutex::new(VecDeque::new()) }
    }

    fn prune(&self, q: &mut VecDeque<Instant>, now: Instant) {
        while let Some(front) = q.front() {
            if now.duration_since(*front) >= WINDOW {
                q.pop_front();
            } else {
                break;
            }
        }
    }

    fn try_reserve(&self, cap: i64) -> bool {
        let Ok(mut q) = self.hits.lock() else { return false };
        let now = Instant::now();
        self.prune(&mut q, now);
        if q.len() as i64 >= cap {
            return false;
        }
        q.push_back(now);
        true
    }

    fn has_room(&self, cap: i64) -> bool {
        let Ok(mut q) = self.hits.lock() else { return false };
        let now = Instant::now();
        self.prune(&mut q, now);
        (q.len() as i64) < cap
    }

    fn used(&self) -> i64 {
        let Ok(mut q) = self.hits.lock() else { return 0 };
        self.prune(&mut q, Instant::now());
        q.len() as i64
    }
}

struct Gate {
    windows: [Window; 2],
    caps: [AtomicI64; 2],
}

static GATE: OnceLock<Gate> = OnceLock::new();

fn state_path() -> PathBuf {
    let dir = std::env::var("ALGODHAN_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));
    if std::fs::create_dir_all(&dir).is_ok() {
        dir.join(ORDER_RATE_FILE)
    } else {
        std::env::temp_dir().join(ORDER_RATE_FILE)
    }
}

fn load_caps() -> (i64, i64) {
    let parsed = std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let get = |key: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_i64())
            .unwrap_or(DEFAULT_OPS)
    };
    (clamp_ops(get("paper")), clamp_ops(get("real")))
}

fn gate() -> &'static Gate {
    GATE.get_or_init(|| {
        let (paper, real) = load_caps();
        Gate {
            windows: [Window::new(), Window::new()],
            caps: [AtomicI64::new(paper), AtomicI64::new(real)],
        }
    })
}

fn window(group: Group) -> &'static Window {
    &gate().windows[group.index()]
}

/// Configured cap for a group.
pub fn cap(group: Group) -> i64 {
    gate().caps[group.index()].load(Ordering::Relaxed)
}

/// Non-consuming check used by the entry scanners so a saturated window drops
/// the signal instead of queueing it (never chase a delayed fill).
pub fn has_room(group: Group) -> bool {
    window(group).has_room(cap(group))
}

/// Consume one slot if the trailing one-second window has room.
pub fn try_reserve(group: Group) -> bool {
    window(group).try_reserve(cap(group))
}

/// Block until a slot is available, then consume it. Used at the actual send
/// boundary: entries pre-check with `has_room` and drop, exits wait their turn
/// so a protective order is never silently lost.
pub async fn reserve_wait(group: Group) {
    loop {
        if try_reserve(group) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Read the operator-editable caps, clamped to Dhan's documented maximum.
pub fn caps() -> (i64, i64) {
    (cap(Group::Paper), cap(Group::Real))
}

/// Persist and apply new caps. Indices are clamped independently, so an
/// out-of-range value can never lift the gate above Dhan's ceiling.
pub fn set_caps(paper: i64, real: i64) -> (i64, i64) {
    let paper = clamp_ops(paper);
    let real = clamp_ops(real);
    let g = gate();
    g.caps[0].store(paper, Ordering::Relaxed);
    g.caps[1].store(real, Ordering::Relaxed);
    let bytes = serde_json::to_vec(&json!({ "paper": paper, "real": real })).unwrap_or_default();
    let path = state_path();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    (paper, real)
}

/// Snapshot payload for the tab header: caps plus the current trailing-second
/// usage, so every tab shows the same shared state.
pub fn status_json() -> Value {
    let (paper, real) = caps();
    json!({
        "paper": { "cap": paper, "used": window(Group::Paper).used() },
        "real": { "cap": real, "used": window(Group::Real).used() },
        "max": MAX_OPS,
        "default": DEFAULT_OPS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_caps_at_the_documented_maximum() {
        assert_eq!(clamp_ops(0), MIN_OPS);
        assert_eq!(clamp_ops(6), 6);
        assert_eq!(clamp_ops(10), MAX_OPS);
        assert_eq!(clamp_ops(500), MAX_OPS);
    }

    #[test]
    fn sliding_window_reserves_and_releases_after_one_second() {
        let w = Window::new();
        assert!(w.has_room(2));
        assert!(w.try_reserve(2));
        assert!(w.has_room(2));
        assert!(w.try_reserve(2));
        assert!(!w.has_room(2));
        assert!(!w.try_reserve(2));
        assert_eq!(w.used(), 2);
        // Age the hits past the window instead of sleeping in a unit test.
        {
            let mut q = w.hits.lock().unwrap();
            let old = Instant::now() - WINDOW - Duration::from_millis(1);
            for t in q.iter_mut() {
                *t = old;
            }
        }
        assert!(w.has_room(2));
        assert_eq!(w.used(), 0);
    }
}
