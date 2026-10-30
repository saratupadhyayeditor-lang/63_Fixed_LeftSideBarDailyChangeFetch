//! Deterministic decision layer for the auto-trading brain.
//!
//! This module holds the PURE logic the engine calls: cost models, evidence
//! scoring, walk-forward validation, volatility gates, risk-based sizing and
//! exit policy. Nothing here touches the Dhan client, the state file or the
//! websocket, so every decision is reproducible from candles + settings alone
//! and can be unit tested exhaustively.
//!
//! Design rule: the brain is not an opaque model. Every number it produces is
//! traceable to a formula in this file, and every choice it commits is logged
//! with the evidence that produced it.

use algo_core::model::Candle;

// ---------------------------------------------------------------------------
// 1. Cost model - a filter's edge is worthless if charges eat it
// ---------------------------------------------------------------------------

/// Round-trip cost expressed in R units (R = risk per trade in rupees).
///
/// Options are the dominant case: brokerage is per order, STT lands on the sell
/// leg, and the bid-ask spread is a real cost the backtest never sees. The model
/// is deliberately simple and conservative - it is a gate, not an accounting
/// ledger - so it can run inside the ranking walk.
#[derive(Clone, Copy, Debug, Default)]
pub struct CostModel {
    /// Brokerage per executed order (rupee, per leg).
    pub brokerage_per_order: f64,
    /// STT as a fraction of the SELL turnover (0.000625 for options).
    pub stt_sell_frac: f64,
    /// Exchange + SEBI + IPFT + stamp combined, as a fraction of turnover.
    pub other_frac: f64,
    /// GST on (brokerage + exchange charges), e.g. 0.18.
    pub gst_frac: f64,
    /// Expected adverse slippage per side, as a fraction of price.
    pub slippage_frac: f64,
    /// Half the quoted bid-ask spread, as a fraction of price.
    pub half_spread_frac: f64,
}

impl CostModel {
    /// Dhan-style option costs. Conservative on the spread because a wide book
    /// is exactly where a "backtested edge" turns into a live loss.
    pub fn options() -> Self {
        Self {
            brokerage_per_order: 20.0,
            stt_sell_frac: 0.000625,
            other_frac: 0.0003672,
            gst_frac: 0.18,
            slippage_frac: 0.0005,
            half_spread_frac: 0.0008,
        }
    }

    /// Futures: cheaper STT, no spread premium on liquid contracts.
    pub fn futures() -> Self {
        Self {
            brokerage_per_order: 20.0,
            stt_sell_frac: 0.0002,
            other_frac: 0.0000183,
            gst_frac: 0.18,
            slippage_frac: 0.0004,
            half_spread_frac: 0.0003,
        }
    }

    pub fn for_instrument(instrument: &str, trading_symbol: &str) -> Self {
        let inst = instrument.trim().to_ascii_uppercase();
        if matches!(inst.as_str(), "OPTIDX" | "OPTSTK" | "OPTFUT" | "OPT") {
            return Self::options();
        }
        if matches!(inst.as_str(), "FUTIDX" | "FUTSTK" | "FUTCOM" | "FUT") {
            return Self::futures();
        }
        let nm = trading_symbol.trim().to_ascii_uppercase();
        if nm.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "CE" || w == "PE") {
            return Self::options();
        }
        Self::futures()
    }

    /// One side's all-in cost for `turnover` rupees (brokerage + tax + friction).
    pub fn side_cost(&self, turnover: f64, is_sell: bool) -> f64 {
        let t = turnover.abs();
        if t <= 0.0 {
            return 0.0;
        }
        let stt = if is_sell { t * self.stt_sell_frac } else { 0.0 };
        let other = t * self.other_frac;
        let gst = (self.brokerage_per_order + other) * self.gst_frac;
        self.brokerage_per_order + stt + other + gst
    }

    /// Total round-trip cost for one option/futures position.
    pub fn round_trip(&self, entry: f64, exit: f64, qty: f64) -> f64 {
        let q = qty.abs();
        if q <= 0.0 || entry <= 0.0 || exit <= 0.0 {
            return 0.0;
        }
        // Entry BUY: no STT. Exit SELL: STT applies.
        let entry_c = self.side_cost(entry * q, false);
        let exit_c = self.side_cost(exit * q, true);
        // Spread + slippage are paid on both sides as a fraction of turnover.
        let friction = (self.slippage_frac + self.half_spread_frac) * (entry + exit) * q;
        entry_c + exit_c + friction
    }

    /// Cost expressed in R units for a trade risking `risk_rupees`.
    pub fn cost_r(&self, entry: f64, exit: f64, qty: f64, risk_rupees: f64) -> f64 {
        if risk_rupees <= 0.0 {
            return 0.0;
        }
        self.round_trip(entry, exit, qty) / risk_rupees
    }
}

/// Net expectancy in R units for a strategy with `win_rate` (0..1), payoff
/// `reward_r` (target in R) and `cost_r` round-trip cost in R units.
///
/// `win_rate * reward - (1 - win_rate) * 1 - cost_r`. This is the number that
/// actually decides profitability - a filter with a 49% hit-rate and a 3R
/// target still needs the cost term to be small.
pub fn net_expectancy(win_rate: f64, reward_r: f64, cost_r: f64) -> f64 {
    let w = win_rate.clamp(0.0, 1.0);
    w * reward_r - (1.0 - w) - cost_r
}

/// Win rate required to break even at `reward_r` after `cost_r`.
///
/// `(1 + cost) / (1 + reward)`. Used to reject a setup outright: if the measured
/// hit-rate is below this, no amount of trade management saves it.
pub fn breakeven_win_rate(reward_r: f64, cost_r: f64) -> f64 {
    if reward_r <= 0.0 {
        return 1.0;
    }
    ((1.0 + cost_r) / (1.0 + reward_r)).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// 2. Evidence scoring - measured on candles, gated on costs
// ---------------------------------------------------------------------------

/// One filter's measured record over a sample window.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    pub samples: usize,
    pub wins: usize,
    pub losses: usize,
    /// Mean favourable excursion in R (how far a winner ran).
    pub avg_win_r: f64,
    /// Mean adverse excursion in R (how far a loser went against).
    pub avg_loss_r: f64,
}

impl Evidence {
    pub fn win_rate(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.wins as f64 / self.samples as f64
        }
    }

    /// Gross payoff ratio (average win / average loss).
    ///
    /// With no observed losses the ratio is mathematically unbounded. Returning
    /// 0 there would wrongly read as "no payoff" and reject a perfect sample, so
    /// a lossless sample is scored conservatively as 1:1 against its own average
    /// win - never as infinite. That keeps the ranking monotonic without
    /// inventing a fantasy payoff.
    pub fn payoff_r(&self) -> f64 {
        if self.avg_win_r <= 0.0 {
            return 0.0;
        }
        if self.avg_loss_r <= 0.0 {
            return 1.0;
        }
        self.avg_win_r / self.avg_loss_r
    }

    pub fn gross_expectancy(&self) -> f64 {
        net_expectancy(self.win_rate(), self.payoff_r(), 0.0)
    }

    pub fn net_expectancy(&self, cost_r: f64) -> f64 {
        net_expectancy(self.win_rate(), self.payoff_r(), cost_r)
    }

    /// Wilson lower bound of the win rate at ~95% confidence. Used so a filter
    /// with 8/10 wins cannot outrank one with 60/100: small samples are
    /// penalised instead of trusted.
    pub fn win_rate_lower_bound(&self) -> f64 {
        let n = self.samples as f64;
        if n <= 0.0 {
            return 0.0;
        }
        let p = self.win_rate();
        let z = 1.96;
        let z2 = z * z;
        let denom = 1.0 + z2 / n;
        let centre = p + z2 / (2.0 * n);
        let margin = z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
        ((centre - margin) / denom).clamp(0.0, 1.0)
    }

    /// Conservative net expectancy: lower-bound win rate, measured payoff,
    /// full cost deducted. This is the ranking key - it rewards both edge AND
    /// sample size, and it refuses to reward luck.
    pub fn conservative_net(&self, cost_r: f64) -> f64 {
        net_expectancy(self.win_rate_lower_bound(), self.payoff_r(), cost_r)
    }
}

/// Smallest sample the brain will trust. Below this a filter is still measured
/// but cannot be armed, because its record is indistinguishable from noise.
pub const MIN_TRUSTED_SAMPLES: usize = 30;

/// Score one filter over `candles` with a cost-aware, sample-size-aware walk.
///
/// For every bar where the filter held, the forward path is walked bar by bar:
/// the first bar that touches `stop_r` R against is a loss, the first that
/// touches `target_r` R in favour is a win, a timeout counts as a small win or
/// loss by the sign of the final move. That mirrors how the engine actually
/// exits (stop or target), which is why the measured win rate is meaningful.
pub fn score_filter(
    bull: bool,
    firings: &[bool],
    candles: &[Candle],
    atr: f64,
    risk_r: f64,
    reward_r: f64,
    max_forward: usize,
) -> Option<Evidence> {
    let n = candles.len();
    if n < 10 || !(atr > 0.0) || risk_r <= 0.0 {
        return None;
    }
    let mut ev = Evidence::default();
    let mut win_sum = 0.0;
    let mut loss_sum = 0.0;
    for i in 0..n.saturating_sub(1) {
        if !firings.get(i).copied().unwrap_or(false) {
            continue;
        }
        let entry = candles[i].close;
        if entry <= 0.0 {
            continue;
        }
        let stop_px = if bull { entry - risk_r * atr } else { entry + risk_r * atr };
        let target_px = if bull { entry + reward_r * atr } else { entry - reward_r * atr };
        let last = (i + max_forward).min(n - 1);
        let mut won = false;
        let mut lost = false;
        let mut progress_r = 0.0;
        for j in (i + 1)..=last {
            let c = &candles[j];
            let hit_stop = if bull { c.low <= stop_px } else { c.high >= stop_px };
            let hit_target = if bull { c.high >= target_px } else { c.low <= target_px };
            // Conservative order: assume the adverse level is touched first when
            // both are inside the same bar. This under-reports winners rather
            // than inventing them.
            if hit_stop {
                lost = true;
                progress_r = -1.0;
                break;
            }
            if hit_target {
                won = true;
                progress_r = reward_r;
                break;
            }
            let move_ = if bull { c.close - entry } else { entry - c.close };
            progress_r = move_ / (risk_r * atr);
        }
        ev.samples += 1;
        if won {
            ev.wins += 1;
            win_sum += progress_r.max(0.0);
        } else if lost {
            ev.losses += 1;
            loss_sum += progress_r.abs().max(0.0);
        } else {
            // Timeout: score by the sign of the final progress, half weight.
            if progress_r > 0.0 {
                ev.wins += 1;
                win_sum += progress_r;
            } else if progress_r < 0.0 {
                ev.losses += 1;
                loss_sum += progress_r.abs();
            }
        }
    }
    if ev.samples == 0 {
        return None;
    }
    let decisive = (ev.wins + ev.losses).max(1) as f64;
    ev.avg_win_r = win_sum / decisive;
    ev.avg_loss_r = loss_sum / decisive;
    Some(ev)
}

// ---------------------------------------------------------------------------
// 3. Walk-forward validation - never trust a filter chosen on the same bars it
//    was tested on
// ---------------------------------------------------------------------------

/// Split a sample into (in-sample, out-of-sample) counts for walk-forward.
///
/// The filter is RANKED on the first part and must also hold up on the second.
/// A row that only works in-sample is discarded, which is the cheapest defence
/// against overfitting a 120-bar window.
pub fn walk_forward_split(total: usize, train_frac: f64) -> (usize, usize) {
    let f = train_frac.clamp(0.3, 0.9);
    let train = ((total as f64) * f).round() as usize;
    let train = train.min(total);
    (train, total - train)
}

/// Combined validation score: the out-of-sample expectancy is what is reported,
/// but a filter must also have been positive in-sample to qualify at all. This
/// stops a fluke run of wins in the holdout from promoting a bad row.
pub fn validated_expectancy(in_sample: &Evidence, out_sample: &Evidence, cost_r: f64) -> Option<f64> {
    if in_sample.samples < MIN_TRUSTED_SAMPLES / 2 || out_sample.samples < MIN_TRUSTED_SAMPLES / 4 {
        return None;
    }
    if in_sample.gross_expectancy() <= 0.0 {
        return None;
    }
    let oos = out_sample.net_expectancy(cost_r);
    if oos <= 0.0 {
        return None;
    }
    Some(oos)
}

// ---------------------------------------------------------------------------
// 4. Volatility gates - a "no-trade" zone at both extremes
// ---------------------------------------------------------------------------

/// Where the current ATR sits in its own recent distribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolZone {
    /// Too quiet: breakouts fail, costs dominate.
    Quiet,
    /// Healthy: the regime where trend systems actually work.
    Normal,
    /// Too wild: stops get hit by noise, option spreads blow out.
    Wild,
}

/// Empirical percentile of `current` inside `history` (0..100).
pub fn percentile_of(current: f64, history: &[f64]) -> f64 {
    if history.is_empty() {
        return 50.0;
    }
    // Ties count as half so a perfectly steady volatility history reads as the
    // median (50th) instead of the 0th percentile. Without this, a series whose
    // ATR never changes would be mislabelled "unusually quiet" and the brain
    // would refuse to trade a perfectly normal tape.
    let below = history.iter().filter(|v| **v < current).count() as f64;
    let equal = history.iter().filter(|v| **v == current).count() as f64;
    (100.0 * (below + 0.5 * equal) / history.len() as f64).clamp(0.0, 100.0)
}

/// Classify volatility: below the 20th percentile is Quiet, above the 90th is
/// Wild, everything between is tradeable.
pub fn vol_zone(atr_pct: f64, history_pct: f64) -> VolZone {
    if history_pct < 20.0 || atr_pct <= 0.0 {
        VolZone::Quiet
    } else if history_pct > 90.0 {
        VolZone::Wild
    } else {
        VolZone::Normal
    }
}

/// Rolling ATR-percent history from a candle series, so the percentile is
/// computed against the instrument's own behaviour rather than a fixed number.
pub fn atr_percent_history(candles: &[Candle], period: usize, window: usize) -> Vec<f64> {
    let mut out = Vec::new();
    if candles.len() < period + 2 || window == 0 {
        return out;
    }
    // The percentile must compare TODAY's volatility against the MOST RECENT
    // window, not the oldest data in the buffer. Walking backwards keeps the
    // read relevant to the current market instead of a stale session.
    let first = (period + 1).max(candles.len().saturating_sub(window));
    for end in first..candles.len() {
        let slice = &candles[end.saturating_sub(period)..=end];
        let close = candles[end].close;
        if close <= 0.0 {
            continue;
        }
        let mut tr_sum = 0.0;
        for w in 1..slice.len() {
            let c = &slice[w];
            let prev = &slice[w - 1];
            let tr = (c.high - c.low)
                .max((c.high - prev.close).abs())
                .max((c.low - prev.close).abs());
            tr_sum += tr;
        }
        let atr = tr_sum / (slice.len() - 1) as f64;
        out.push(atr / close * 100.0);
    }
    out
}

// ---------------------------------------------------------------------------
// 5. Risk-based sizing - size the STOP, not the margin
// ---------------------------------------------------------------------------

/// One sizing decision.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizingPlan {
    /// Whole contracts (lots x lot_size), floored to a lot multiple.
    pub qty: i64,
    pub lots: f64,
    /// Rupees actually at risk with this qty.
    pub risk_rupees: f64,
    /// The qty that WOULD have hit the risk cap exactly (for logging).
    pub ideal_qty: f64,
    /// True when the market lot forced a larger risk than requested.
    pub over_cap: bool,
}

/// Size a position so that `stop_distance` (price units per unit) costs at most
/// `risk_rupees`, never exceeding `margin_cap_qty` (the shared-allotment limit).
///
/// This is the difference between "size from margin" (what the engine did
/// before) and "size from risk": a wide range stop now buys FEWER contracts so
/// each trade risks the same rupees regardless of regime.
pub fn risk_based_qty(
    risk_rupees: f64,
    stop_distance: f64,
    lot: f64,
    margin_cap_qty: i64,
    max_qty: i64,
) -> SizingPlan {
    let lot = if lot > 0.0 { lot } else { 1.0 };
    if risk_rupees <= 0.0 || stop_distance <= 0.0 {
        return SizingPlan::default();
    }
    let ideal = risk_rupees / stop_distance;
    let raw_lots = (ideal / lot).floor();
    let mut lots = raw_lots.max(0.0);
    let mut qty = (lots * lot).round() as i64;
    // Respect the margin and absolute caps, always in whole lots.
    let cap = margin_cap_qty.min(max_qty).max(0);
    if qty > cap {
        lots = (cap as f64 / lot).floor();
        qty = (lots * lot).round() as i64;
    }
    if qty <= 0 {
        // Not even one lot fits the budget. Report the risk of ONE lot so the
        // caller can decide to skip (never silently size to zero and trade).
        return SizingPlan {
            qty: 0,
            lots: 0.0,
            risk_rupees: 0.0,
            ideal_qty: ideal,
            over_cap: true,
        };
    }
    let risk = qty as f64 * stop_distance;
    SizingPlan {
        qty,
        lots,
        risk_rupees: risk,
        ideal_qty: ideal,
        over_cap: qty as f64 > ideal,
    }
}

// ---------------------------------------------------------------------------
// 6. Exit policy - breakeven, partial, time stop, regime flip
// ---------------------------------------------------------------------------

/// Everything the position manager needs to run an adaptive exit.
#[derive(Clone, Copy, Debug)]
pub struct ExitPolicy {
    /// Move the stop to entry once this many R of profit is reached (0 = off).
    pub breakeven_r: f64,
    /// Book this fraction of the position at `partial_r` (0 = never).
    pub partial_frac: f64,
    pub partial_r: f64,
    /// Force an exit after this many milliseconds in trade (0 = off). Guards
    /// against theta bleed and dead capital.
    pub time_stop_ms: i64,
    /// Exit if the entry regime flips (trend -> range or vice versa).
    pub exit_on_regime_flip: bool,
}

impl ExitPolicy {
    /// Trend: let it run. Breakeven early, partial late, long time stop.
    pub fn trend() -> Self {
        Self {
            breakeven_r: 1.0,
            partial_frac: 0.5,
            partial_r: 2.0,
            time_stop_ms: 6 * 60 * 60 * 1000,
            exit_on_regime_flip: true,
        }
    }

    /// Range: take money sooner. Breakeven late, partial early, short time stop.
    pub fn range() -> Self {
        Self {
            breakeven_r: 1.5,
            partial_frac: 0.5,
            partial_r: 1.0,
            time_stop_ms: 90 * 60 * 1000,
            exit_on_regime_flip: true,
        }
    }

    pub fn for_regime(regime: &str) -> Self {
        if regime.eq_ignore_ascii_case("range") {
            Self::range()
        } else {
            Self::trend()
        }
    }
}

/// Current R multiple of an open trade (0 at entry, negative when losing).
pub fn current_r(is_buy: bool, entry: f64, ltp: f64, risk_per_unit: f64) -> f64 {
    if risk_per_unit <= 0.0 || entry <= 0.0 {
        return 0.0;
    }
    let move_ = if is_buy { ltp - entry } else { entry - ltp };
    move_ / risk_per_unit
}

/// Decision from the exit policy for one open position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitAction {
    None,
    /// Ratchet the stop to the given price.
    MoveStop,
    /// Book `frac` of the position.
    Partial,
    /// Exit everything now with the given reason tag.
    Exit(&'static str),
}

impl ExitAction {
    pub fn is_exit(self) -> bool {
        matches!(self, ExitAction::Exit(_))
    }
}

/// Evaluate the exit policy for one position.
///
/// Order of precedence: hard exits first (time stop, regime flip), then the
/// breakeven ratchet, then the partial booking. A partial is reported only once
/// - the caller records `partialDone` on the position so it cannot repeat.
pub fn evaluate_exit(
    policy: &ExitPolicy,
    is_buy: bool,
    entry: f64,
    ltp: f64,
    risk_per_unit: f64,
    age_ms: i64,
    regime_flipped: bool,
    partial_done: bool,
    breakeven_done: bool,
) -> ExitAction {
    if policy.time_stop_ms > 0 && age_ms >= policy.time_stop_ms {
        return ExitAction::Exit("TIME_STOP");
    }
    if policy.exit_on_regime_flip && regime_flipped {
        return ExitAction::Exit("REGIME_FLIP");
    }
    let r = current_r(is_buy, entry, ltp, risk_per_unit);
    if policy.partial_frac > 0.0 && !partial_done && policy.partial_r > 0.0 && r >= policy.partial_r {
        return ExitAction::Partial;
    }
    if !breakeven_done && policy.breakeven_r > 0.0 && r >= policy.breakeven_r {
        return ExitAction::MoveStop;
    }
    ExitAction::None
}

// ---------------------------------------------------------------------------
// 7. Instrument selection - rank WHAT to trade, not just WHEN
// ---------------------------------------------------------------------------

/// One candidate instrument's quality read. Higher is better.
#[derive(Clone, Debug, Default)]
pub struct InstrumentScore {
    /// Absolute % move today (momentum, but log-scaled so a 20% gap does not
    /// swamp a broad 2% trend).
    pub momentum: f64,
    /// Turnover/liquidity proxy (0..1), from relative volume.
    pub liquidity: f64,
    /// Spread penalty (0..1, 1 = tight book).
    pub spread_quality: f64,
    /// Measured edge of the filters on THIS instrument, in R.
    pub edge_r: f64,
    /// True when the option economics are acceptable (premium, DTE, spread).
    pub economics_ok: bool,
}

impl InstrumentScore {
    /// Combined rank. Edge dominates, then momentum, then liquidity/spread;
    /// an instrument that fails economics is pushed far below everything else
    /// instead of being silently dropped (so the readout still shows it).
    pub fn rank(&self) -> f64 {
        if !self.economics_ok {
            return -1.0e9;
        }
        self.edge_r * 3.0 + self.momentum * 0.5 + self.liquidity * 0.3 + self.spread_quality * 0.2
    }
}

/// Log-scaled momentum score from a percent change. A 2% move is "worth" ~0.3,
/// a 5% move ~0.7, and a 20% gap does not get 10x the weight of a 2% move.
pub fn momentum_score(change_pct: f64) -> f64 {
    let x = change_pct.abs() / 100.0;
    if x <= 0.0 {
        0.0
    } else {
        (1.0 + x * 40.0).ln() / 4.0
    }
}

/// Option economics check: reject a contract whose premium is so small that
/// charges dominate, or so large that a single lot blows the risk budget.
pub fn option_economics_ok(premium: f64, lot: f64, risk_rupees: f64, cost_per_contract: f64) -> bool {
    if premium <= 0.0 || lot <= 0.0 || risk_rupees <= 0.0 {
        return false;
    }
    // Cost of ONE contract must be a small fraction of the risk budget.
    let one_lot_notional = premium * lot;
    let cost_frac = cost_per_contract / risk_rupees;
    cost_frac < 0.35 && one_lot_notional > 0.0
}

// ---------------------------------------------------------------------------
// 8. Feedback ledger - learn from ACTUAL fills, not just candles
// ---------------------------------------------------------------------------

/// Key that groups realised outcomes: regime x instrument class x side.
pub fn evidence_key(regime: &str, inst_class: &str, side: &str) -> String {
    format!(
        "{}|{}|{}",
        regime.trim().to_ascii_lowercase(),
        inst_class.trim().to_ascii_lowercase(),
        side.trim().to_ascii_uppercase()
    )
}

/// A realised-outcome bucket. Stored in the engine state so it survives
/// restarts; the brain blends it with the candle backtest.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RealisedBucket {
    pub trades: usize,
    pub wins: usize,
    pub net_rupees: f64,
    /// Sum of R multiples across closed trades.
    pub sum_r: f64,
}

impl RealisedBucket {
    pub fn record(&mut self, net_rupees: f64, r_multiple: f64) {
        self.trades += 1;
        if net_rupees > 0.0 {
            self.wins += 1;
        }
        self.net_rupees += net_rupees;
        self.sum_r += r_multiple;
    }

    pub fn win_rate(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.wins as f64 / self.trades as f64
        }
    }

    pub fn avg_r(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.sum_r / self.trades as f64
        }
    }
}

/// Blend the candle-backtest expectancy with realised live results.
///
/// Real fills are ground truth but arrive slowly, so they get a growing weight:
/// `w = n / (n + k)` with `k = 20`. With no live trades the backtest stands
/// alone; after 60 trades the live record carries 75% of the decision.
pub fn blended_expectancy(backtest_r: f64, live_avg_r: f64, live_trades: usize) -> f64 {
    let k = 20.0;
    let w = live_trades as f64 / (live_trades as f64 + k);
    backtest_r * (1.0 - w) + live_avg_r * w
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(close: f64, high: f64, low: f64) -> Candle {
        Candle { time: 0, open: close, high, low, close, volume: 1000.0 }
    }

    #[test]
    fn cost_model_charges_more_for_options_than_futures() {
        let opt = CostModel::options().round_trip(100.0, 120.0, 50.0);
        let fut = CostModel::futures().round_trip(100.0, 120.0, 50.0);
        assert!(opt > 0.0 && fut > 0.0);
        assert!(opt > fut, "options {opt} should cost more than futures {fut}");
    }

    #[test]
    fn cost_r_scales_with_risk() {
        let m = CostModel::options();
        let big = m.cost_r(100.0, 120.0, 50.0, 10_000.0);
        let small = m.cost_r(100.0, 120.0, 50.0, 1_000.0);
        assert!(small > big, "smaller risk budget => higher relative cost");
    }

    #[test]
    fn net_expectancy_and_breakeven_agree() {
        // At exactly the breakeven win rate, expectancy is zero.
        let rr = 3.0;
        let cost = 0.1;
        let be = breakeven_win_rate(rr, cost);
        assert!((net_expectancy(be, rr, cost)).abs() < 1e-9);
        assert!(net_expectancy(be + 0.05, rr, cost) > 0.0);
        assert!(net_expectancy(be - 0.05, rr, cost) < 0.0);
    }

    #[test]
    fn costs_can_turn_a_positive_edge_negative() {
        let rr = 1.0;
        // 55% at 1R is profitable with no costs.
        assert!(net_expectancy(0.55, rr, 0.0) > 0.0);
        // A 20% round-trip cost wipes it out.
        assert!(net_expectancy(0.55, rr, 0.2) < 0.0);
    }

    #[test]
    fn wilson_bound_penalises_small_samples() {
        let lucky = Evidence { samples: 10, wins: 8, losses: 2, avg_win_r: 2.0, avg_loss_r: 1.0 };
        let solid = Evidence { samples: 100, wins: 60, losses: 40, avg_win_r: 2.0, avg_loss_r: 1.0 };
        assert!(
            solid.win_rate_lower_bound() > lucky.win_rate_lower_bound(),
            "60/100 must beat 8/10 on the conservative bound"
        );
        // The raw win rate ranks them the other way round - that is the trap.
        assert!(lucky.win_rate() > solid.win_rate());
    }

    #[test]
    fn score_filter_reads_a_clean_trend_as_positive() {
        // 120 rising bars, filter always bull. A 1R stop / 2R target walk must
        // find wins.
        let candles: Vec<Candle> = (0..120)
            .map(|i| {
                let c = 100.0 + i as f64 * 0.8;
                candle(c, c + 0.5, c - 0.5)
            })
            .collect();
        let fires = vec![true; candles.len()];
        let ev = score_filter(true, &fires, &candles, 1.0, 1.0, 2.0, 5).expect("evidence");
        assert!(ev.samples > 0);
        assert!(ev.win_rate() > 0.5, "clean trend should win: {}", ev.win_rate());
        assert!(ev.net_expectancy(0.05) > 0.0);
    }

    #[test]
    fn score_filter_treats_ambiguous_bar_as_a_loss() {
        // A single wide bar that spans BOTH stop and target must be booked as a
        // loss, because the engine cannot know which was touched first.
        // 12 flat bars with ONE wide ambiguous bar at index 4, so the sample is
        // long enough to score and the ambiguous bar is the only firing.
        // Fire at index 4; the very NEXT bar (5) is the one that spans both
        // stop and target, because the forward walk starts at i + 1.
        let mut candles: Vec<Candle> = (0..12).map(|_| candle(100.0, 100.1, 99.9)).collect();
        candles[5] = candle(100.0, 200.0, 1.0);
        let fires: Vec<bool> = (0..12).map(|i| i == 4).collect();
        let ev = score_filter(true, &fires, &candles, 1.0, 1.0, 2.0, 2).expect("evidence");
        assert_eq!(ev.wins, 0);
        assert_eq!(ev.losses, 1);
    }

    #[test]
    fn walk_forward_split_is_disjoint_and_covers_the_sample() {
        let (train, test) = walk_forward_split(100, 0.7);
        assert_eq!(train + test, 100);
        assert_eq!((train, test), (70, 30));
    }

    #[test]
    fn walk_forward_rejects_an_in_sample_loser() {
        let is = Evidence { samples: 60, wins: 20, losses: 40, avg_win_r: 1.0, avg_loss_r: 1.0 };
        let oos = Evidence { samples: 30, wins: 20, losses: 10, avg_win_r: 2.0, avg_loss_r: 1.0 };
        assert!(validated_expectancy(&is, &oos, 0.05).is_none());
    }

    #[test]
    fn walk_forward_rejects_an_out_of_sample_loser() {
        let is = Evidence { samples: 60, wins: 40, losses: 20, avg_win_r: 2.0, avg_loss_r: 1.0 };
        let oos = Evidence { samples: 30, wins: 8, losses: 22, avg_win_r: 1.0, avg_loss_r: 1.0 };
        assert!(validated_expectancy(&is, &oos, 0.05).is_none());
    }

    #[test]
    fn walk_forward_accepts_a_consistent_winner() {
        let is = Evidence { samples: 60, wins: 40, losses: 20, avg_win_r: 2.0, avg_loss_r: 1.0 };
        let oos = Evidence { samples: 30, wins: 20, losses: 10, avg_win_r: 2.0, avg_loss_r: 1.0 };
        assert!(validated_expectancy(&is, &oos, 0.05).is_some());
    }

    #[test]
    fn vol_zone_flags_both_extremes() {
        assert_eq!(vol_zone(0.1, 10.0), VolZone::Quiet);
        assert_eq!(vol_zone(0.3, 50.0), VolZone::Normal);
        assert_eq!(vol_zone(1.2, 95.0), VolZone::Wild);
    }

    #[test]
    fn percentile_of_is_empirical() {
        let hist = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        // Ties count as half, so the median value lands on 50 rather than 40.
        assert!((percentile_of(3.0, &hist) - 50.0).abs() < 1e-9);
        assert!((percentile_of(0.0, &hist) - 0.0).abs() < 1e-9);
        assert!((percentile_of(9.0, &hist) - 100.0).abs() < 1e-9);
        // A perfectly constant history is the median, NOT an artificial extreme.
        let flat = vec![0.5; 40];
        assert!((percentile_of(0.5, &flat) - 50.0).abs() < 1e-9);
        assert_eq!(vol_zone(0.5, percentile_of(0.5, &flat)), VolZone::Normal);
    }

    #[test]
    fn atr_history_reads_the_most_recent_window() {
        // A quiet first half and a volatile second half: the history must reflect
        // the recent (volatile) end, otherwise today's percentile is meaningless.
        let mut candles: Vec<Candle> = (0..60)
            .map(|_| Candle { time: 0, open: 100.0, high: 100.1, low: 99.9, close: 100.0, volume: 1.0 })
            .collect();
        candles.extend((0..60).map(|_| Candle {
            time: 0,
            open: 100.0,
            high: 103.0,
            low: 97.0,
            close: 100.0,
            volume: 1.0,
        }));
        let h = atr_percent_history(&candles, 14, 40);
        assert_eq!(h.len(), 40, "must return the requested window size");
        // The tail of the history must be the volatile regime (~6%), not the
        // quiet one (~0.2%).
        let last = *h.last().unwrap();
        assert!(last > 2.0, "recent volatile ATR% expected, got {last}");
    }

    #[test]
    fn atr_history_is_bounded_and_positive() {
        let candles: Vec<Candle> = (0..60)
            .map(|i| {
                let c = 100.0 + (i as f64 * 0.3).sin();
                candle(c, c + 0.4, c - 0.4)
            })
            .collect();
        let hist = atr_percent_history(&candles, 14, 100);
        assert!(!hist.is_empty());
        assert!(hist.iter().all(|v| *v > 0.0 && v.is_finite()));
    }

    #[test]
    fn risk_sizing_respects_the_risk_cap() {
        // 10,000 risk, 5.0 stop distance, lot 50 => ideal 2000 / lot 50 = 40 lots.
        let p = risk_based_qty(10_000.0, 5.0, 50.0, 100_000, 100_000);
        assert_eq!(p.qty, 2_000);
        assert!((p.risk_rupees - 10_000.0).abs() < 1e-9);
        assert!(!p.over_cap);
    }

    #[test]
    fn risk_sizing_shrinks_when_the_stop_widens() {
        let tight = risk_based_qty(10_000.0, 2.0, 50.0, 100_000, 100_000);
        let wide = risk_based_qty(10_000.0, 8.0, 50.0, 100_000, 100_000);
        assert!(wide.qty < tight.qty, "wider stop must buy fewer contracts");
    }

    #[test]
    fn risk_sizing_never_exceeds_the_margin_cap() {
        let p = risk_based_qty(1_000_000.0, 1.0, 50.0, 500, 10_000);
        assert!(p.qty <= 500, "qty {} must respect cap 500", p.qty);
        assert_eq!(p.qty % 50, 0, "must stay a whole-lot multiple");
    }

    #[test]
    fn risk_sizing_blocks_when_not_even_one_lot_fits() {
        let p = risk_based_qty(100.0, 50.0, 50.0, 100_000, 100_000);
        assert_eq!(p.qty, 0);
        assert!(p.over_cap);
    }

    #[test]
    fn exit_policy_times_out_and_flips_regime() {
        let pol = ExitPolicy::range();
        let t = evaluate_exit(&pol, true, 100.0, 100.0, 5.0, pol.time_stop_ms + 1, false, false, false);
        assert_eq!(t, ExitAction::Exit("TIME_STOP"));
        let f = evaluate_exit(&pol, true, 100.0, 100.0, 5.0, 1000, true, false, false);
        assert_eq!(f, ExitAction::Exit("REGIME_FLIP"));
    }

    #[test]
    fn exit_policy_partial_then_breakeven() {
        let pol = ExitPolicy::trend(); // partial at 2R, breakeven at 1R
        // At 1R: move the stop to breakeven.
        let be = evaluate_exit(&pol, true, 100.0, 105.0, 5.0, 1000, false, false, false);
        assert_eq!(be, ExitAction::MoveStop);
        // At 2R: partial.
        let pt = evaluate_exit(&pol, true, 100.0, 110.0, 5.0, 1000, false, false, true);
        assert_eq!(pt, ExitAction::Partial);
        // Already partialed and already breakevened: nothing left to do.
        let none = evaluate_exit(&pol, true, 100.0, 110.0, 5.0, 1000, false, true, true);
        assert_eq!(none, ExitAction::None);
    }

    #[test]
    fn current_r_matches_direction() {
        assert!((current_r(true, 100.0, 110.0, 5.0) - 2.0).abs() < 1e-9);
        assert!((current_r(false, 100.0, 90.0, 5.0) - 2.0).abs() < 1e-9);
        assert!((current_r(true, 100.0, 95.0, 5.0) + 1.0).abs() < 1e-9);
    }

    #[test]
    fn instrument_rank_rejects_bad_economics() {
        let good = InstrumentScore {
            momentum: 0.5,
            liquidity: 0.8,
            spread_quality: 0.9,
            edge_r: 0.4,
            economics_ok: true,
        };
        let bad = InstrumentScore { economics_ok: false, edge_r: 5.0, ..Default::default() };
        assert!(good.rank() > bad.rank());
        assert!(bad.rank() < -1.0e8);
    }

    #[test]
    fn momentum_is_log_scaled_not_linear() {
        let small = momentum_score(2.0);
        let big = momentum_score(20.0);
        assert!(big > small);
        assert!(big < small * 10.0, "a 20% move must not be 10x a 2% move");
    }

    #[test]
    fn realised_bucket_tracks_win_rate_and_r() {
        let mut b = RealisedBucket::default();
        b.record(500.0, 2.0);
        b.record(-250.0, -1.0);
        b.record(750.0, 3.0);
        assert_eq!(b.trades, 3);
        assert_eq!(b.wins, 2);
        assert!((b.win_rate() - 2.0 / 3.0).abs() < 1e-9);
        assert!((b.avg_r() - (2.0 - 1.0 + 3.0) / 3.0).abs() < 1e-9);
        assert!((b.net_rupees - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn blended_expectancy_trusts_live_data_as_it_accumulates() {
        // No live trades: pure backtest.
        assert!((blended_expectancy(0.5, -1.0, 0) - 0.5).abs() < 1e-9);
        // 60 live trades: live record dominates.
        let b = blended_expectancy(0.5, -0.5, 60);
        assert!(b < 0.1, "60 losing live trades must drag the blend down: {b}");
        // The blend moves monotonically from backtest towards live.
        let a = blended_expectancy(0.5, -1.0, 5);
        let c = blended_expectancy(0.5, -1.0, 40);
        assert!(a > c);
    }

    #[test]
    fn evidence_key_is_normalised() {
        assert_eq!(evidence_key("TREND", "Options", "buy"), "trend|options|BUY");
    }

    #[test]
    fn option_economics_rejects_a_penny_premium() {
        // 0.05 premium, lot 50 => notional 2.5; cost 40 on a 100 risk budget.
        assert!(!option_economics_ok(0.05, 50.0, 100.0, 40.0));
        // Healthy premium with a small relative cost passes.
        assert!(option_economics_ok(120.0, 75.0, 10_000.0, 40.0));
    }
}
