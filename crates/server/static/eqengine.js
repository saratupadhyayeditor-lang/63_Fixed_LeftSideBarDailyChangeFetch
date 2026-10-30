import { mountSharedMargin } from "./sharedmargin.js?v=7";

const MODE = new URLSearchParams(location.search).get("mode") === "live" ? "live" : "paper";
const API = "/api/equity/engines";
const ENGINE_ID = MODE === "live" ? "equitylive" : "equitypaper";

const S = {
  data: null,
  broker: false,
  busy: false,
  timer: null,
  initialized: false,
  symbolsLoaded: false,
  research: null,
  researchErr: "",
  callsSig: "",
  callsLimit: 60,
  callsHold: false,
  callsPending: false,
  dirty: new Set(),
  saveTimer: null,
};

const $ = (id) => document.getElementById(id);
const esc = (v) => String(v == null ? "" : v).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmt = (v, d = 2) => (Number.isFinite(Number(v)) ? Number(v).toLocaleString("en-IN", { maximumFractionDigits: d, minimumFractionDigits: d }) : "-");
const inr = (v) => "Rs " + fmt(v, 0);
const pct = (v) => (Number.isFinite(Number(v)) ? (Number(v) >= 0 ? "+" : "") + fmt(v, 2) + "%" : "-");
const cls = (v) => (Number(v) > 0 ? "up" : Number(v) < 0 ? "down" : "dim");

function daysOpen(t) {
  if (!t.openedAt) return 0;
  return Math.max(0, Math.floor((Date.now() / 1000 - t.openedAt) / 86400));
}

async function api(path, opts) {
  const r = await fetch(path, Object.assign({ headers: { "Content-Type": "application/json" } }, opts || {}));
  const txt = await r.text();
  let body = null;
  try { body = txt ? JSON.parse(txt) : null; } catch (_) { body = null; }
  if (!r.ok) throw new Error((body && body.error) || txt || r.statusText);
  return body;
}

export function bootEqEngine() {
  if (S.initialized) return;
  S.initialized = true;
  const live = MODE === "live";
  $("eqTitle").textContent = live ? "Realtime Equity Engine" : "Equity Paper Engine";
  $("eqSub").textContent = live
    ? "LIVE broker execution: real Dhan CNC delivery orders from Equity Research Buy calls."
    : "Long-term Buy calls from Equity Research, simulated delivery fills (no real orders).";
  $("eqMode").textContent = live ? "LIVE REAL ORDERS" : "PAPER";
  $("eqMode").className = "eq-badge " + MODE;
  $("eqLiveBanner").style.display = live ? "block" : "none";
  // Reset P&L / Clear book are PAPER-only: the LIVE book is a real-money record.
  for (const id of ["eqResetPnl", "eqClearBook"]) {
    const el = $(id);
    if (el) el.style.display = live ? "none" : "";
  }
  $("eqToggle").addEventListener("click", toggle);
  $("eqSave").addEventListener("click", () => saveSettings(true));
  mountSharedMargin(document.querySelector(".eq-settings"), ENGINE_ID);
  $("mBuy").addEventListener("click", manualBuy);
  $("mQty").addEventListener("keydown", (e) => { if (e.key === "Enter") manualBuy(); });
  $("mSym").addEventListener("keydown", (e) => { if (e.key === "Enter") manualBuy(); });
  $("eqRefresh").addEventListener("click", () => load(true));
  $("sDist").addEventListener("change", applyDistributeState);
  $("sTrailOn").addEventListener("change", applyTrailState);
  $("sNetTpOn").addEventListener("change", applyTrailState);
  $("eqOpen").addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-exit]");
    if (!btn) return;
    const id = btn.getAttribute("data-exit");
    const sym = btn.getAttribute("data-sym") || id;
    if (!confirm(`Exit ${sym} now?`)) return;
    exitTrade(id);
  });
  $("eqCalls").addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-buy]");
    if (!btn || btn.disabled) return;
    buyCall(btn.getAttribute("data-buy"), btn.getAttribute("data-status"));
  });
  // The 5-second poll must never rebuild the table under the operator's cursor:
  // replacing the row between mousedown and mouseup swallows the click, which
  // is exactly what made the Buy buttons feel dead. Hold the re-render while the
  // pointer is over the table and flush it on leave.
  const callsEl = $("eqCalls");
  callsEl.addEventListener("pointerenter", () => { S.callsHold = true; });
  callsEl.addEventListener("pointerleave", () => {
    S.callsHold = false;
    if (S.callsPending) {
      S.callsPending = false;
      renderCalls(S.data ? S.data[MODE] || {} : {});
    }
  });
  wireSettingsAutosave();
  $("eqAutoBuy").addEventListener("click", toggleAutoBuy);
  if ($("eqResetPnl")) $("eqResetPnl").addEventListener("click", () => resetPnl());
  if ($("eqClearBook")) $("eqClearBook").addEventListener("click", () => clearBook());
  $("eqMore").addEventListener("click", () => {
    S.callsLimit += 60;
    S.callsSig = "";
    renderCalls(S.data ? S.data[MODE] || {} : {});
  });
  load();
  loadSymbols();
  loadResearch();
  S.timer = setInterval(() => {
    const fe = window.frameElement;
    if (fe && !(fe.offsetParent && fe.clientHeight > 0)) return;
    load();
  }, 5000);
  S.researchTimer = setInterval(loadResearch, 300000);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) { load(); loadResearch(); } });
}

async function load(force) {
  if (S.busy && !force) return;
  S.busy = true;
  try {
    const d = await api(API);
    S.data = d;
    S.broker = !!d.brokerConnected;
    render(d[MODE] || {});
    $("eqError").textContent = "";
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

// Auto Buy Mode button: a full-width, colour-coded toggle so the operator can
// see at a glance whether the engine will open new positions on its own.
function paintAutoBuy(v) {
  const btn = $("eqAutoBuy");
  if (!btn) return;
  const enabled = !!(v && v.enabled);
  const on = enabled && !!(v && v.settings && v.settings.autoBuy !== false);
  btn.setAttribute("data-on", on ? "1" : "0");
  btn.classList.toggle("armed", on);
  btn.disabled = !enabled;
  if (!enabled) {
    btn.textContent = "Auto Buy Mode: ENGINE OFF";
    btn.title = "Engine connect karo - Auto Buy tabhi chalega jab engine ON ho.";
  } else if (on) {
    btn.textContent = "Auto Buy Mode: ON";
    btn.title = "ON: engine research Buy calls se khud naye positions khol raha hai. Click to pause new entries.";
  } else {
    btn.textContent = "Auto Buy Mode: PAUSED";
    btn.title = "PAUSED: koi naya automatic buy nahi. Position management aur manual buy chalu hai. Click to resume.";
  }
}

// A settings field the operator has typed into must keep the typed value until
// it is saved - the 5-second poll used to repaint every input from the last
// saved state, so numbers "reset to default" right after entry.
function markDirty(el) {
  if (!el || !el.id) return;
  S.dirty.add(el.id);
}

function wireSettingsAutosave() {
  const box = document.querySelector(".eq-settings");
  if (!box) return;
  const schedule = () => {
    if (S.saveTimer) clearTimeout(S.saveTimer);
    S.saveTimer = setTimeout(() => {
      S.saveTimer = null;
      // Never autosave a half-typed form: an empty number box would be sent as 0
      // and the server would clamp it, snapping the field away from the value
      // the operator is still entering.
      const invalid = [...box.querySelectorAll("input[type=number]")].some(
        (el) => el.value === "" || !Number.isFinite(Number(el.value))
      );
      if (invalid) return;
      saveSettings(false);
    }, 700);
  };
  // Only the engine's own settings fields are autosaved; the shared-margin panel
  // that lives in the same container manages (and persists) itself.
  box.querySelectorAll("input[id]").forEach((el) => {
    // Typing only pins the field (so the poll cannot repaint over it); the save
    // happens on commit (blur / spinner / checkbox toggle), never mid-keystroke.
    el.addEventListener("input", () => markDirty(el));
    el.addEventListener("change", () => { markDirty(el); schedule(); });
    el.addEventListener("blur", () => { if (S.dirty.has(el.id)) schedule(); });
  });
}

function applyDistributeState() {
  const on = $("sDist").checked;
  for (const id of ["sPer", "sQty"]) {
    const el = $(id);
    if (!el) continue;
    el.disabled = on;
    el.title = on ? "Auto-distribute ON - this field is ignored" : "";
  }
}

function applyTrailState() {
  const tOn = $("sTrailOn").checked;
  const tpOn = $("sNetTpOn").checked;
  const tEl = $("sTrailPct");
  if (tEl) {
    tEl.disabled = !tOn;
    tEl.title = tOn ? "" : "Percentage trailing stop OFF";
  }
  for (const id of ["sNetTpMin", "sNetTpGb"]) {
    const el = $(id);
    if (!el) continue;
    el.disabled = !tpOn;
    el.title = tpOn ? "" : "Net Take-Profit OFF";
  }
}

function render(v) {
  const st = v.stats || {};
  const s = v.settings || {};
  const live = MODE === "live";
  const on = !!v.enabled;
  $("eqToggle").textContent = on ? "Disconnect" : "Connect";
  $("eqToggle").className = "eq-btn " + (on ? "danger" : "primary");
  $("eqToggle").setAttribute("data-on", on ? "1" : "0");
  $("eqToggle").disabled = !on && live && !S.broker;
  $("eqToggle").title = !on && live && !S.broker ? "Connect Dhan first" : "";

  const brokerEl = $("eqBroker");
  if (live) {
    brokerEl.textContent = S.broker ? "Dhan: connected" : "Dhan: not connected";
    brokerEl.className = "eq-broker " + (S.broker ? "ok" : "bad");
  } else {
    brokerEl.textContent = S.broker ? "Live LTP feed: on" : "Price feed: cached";
    brokerEl.className = "eq-broker " + (S.broker ? "ok" : "bad");
  }

  $("eqStats").innerHTML = [
    card(on ? "ON" : "OFF", "Engine", on ? "up" : "dim"),
    card(st.open || 0, "Open positions"),
    card(st.closed || 0, "Closed trades"),
    card((st.winRate || 0).toFixed(1) + "%", "Win rate"),
    card(inr(st.realizedPnl || 0), "Realized P&L", cls(st.realizedPnl)),
    card(inr(st.unrealizedPnl || 0), "Unrealized P&L", cls(st.unrealizedPnl)),
    card(inr(st.invested || 0), "Invested"),
    card(inr(v.effectiveCapital || 0), "Shared margin",
      (v.sharedCap || 0) > 0 ? "" : "dim"),
    card((st.avgDays || 0).toFixed(0) + " d", "Avg holding"),
    card(pct(st.bestPct || 0) + " / " + pct(st.worstPct || 0), "Best / worst"),
  ].join("");

  const ae = document.activeElement;
  const editing = !!ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA" || ae.tagName === "SELECT");
  const skip = (id) => (editing && ae && ae.id === id) || S.dirty.has(id);
  const setVal = (id, v) => { if (!skip(id)) $(id).value = v; };
  const setChk = (id, v) => { if (!skip(id)) $(id).checked = v; };
  setVal("sPer", s.perTradePct ?? 10);
  setVal("sMax", s.maxPositions ?? 8);
  setVal("sConf", s.minConfidence ?? 70);
  setVal("sUp", s.minUpsidePct ?? 12);
  setVal("sDays", s.maxHoldingDays ?? 180);
  setChk("sAuto", s.autoExit !== false);
  setChk("sAcc", !!s.allowAccumulate);
  setChk("sDist", !!s.autoDistribute);
  setChk("sCharges", s.chargesAware !== false);
  setVal("sQty", s.fixedQty ?? 0);
  setVal("sWatch", (s.watchSymbols || []).join(", "));
  setChk("sTrailOn", !!s.trailSlOn);
  setVal("sTrailPct", s.trailSlPct ?? 25);
  setChk("sNetTpOn", !!s.netTpOn);
  setVal("sNetTpMin", s.netTpMin ?? 0);
  setVal("sNetTpGb", s.netTpGivebackPct ?? 25);
  applyDistributeState();
  applyTrailState();
  paintAutoBuy(v);

  if (v.lastError) $("eqError").textContent = v.lastError;

  const open = (v.trades || []).filter((t) => !t.exitedAt);
  $("eqOpen").innerHTML = open.length
    ? open.map((t) => {
        const pnl = (Number(t.lastPrice) - Number(t.entryPrice)) * Number(t.qty);
        const pp = Number(t.entryPrice) > 0 ? (Number(t.lastPrice) / Number(t.entryPrice) - 1) * 100 : 0;
        const trail = Number(t.trailStop) || 0;
        const net = Number.isFinite(Number(t.netNow)) ? Number(t.netNow) : pnl;
        return `<tr>
          <td class="sym">${esc(t.symbol)}<span class="nm">${esc(t.name)}</span></td>
          <td>${esc(t.status)}${t.confidence ? ` <span class="dim">${Number(t.confidence).toFixed(0)}%</span>` : ""}</td>
          <td>${fmt(t.entryPrice)}</td><td>${fmt(t.lastPrice)}</td><td>${fmt(t.qty, 0)}</td>
          <td>${inr(t.invested)}</td><td class="up">${fmt(t.target)}</td><td class="down">${fmt(t.stop)}</td>
          <td class="${trail > 0 ? "up" : "dim"}" title="${trail > 0 ? "Percentage trail armed - ratchets behind peak profit" : "Trail not armed yet (needs profit)"}">${trail > 0 ? fmt(trail) : "-"}</td>
          <td class="${cls(net)}" title="Net of delivery charges - the number Net TP watches">${inr(net)}</td>
          <td class="${cls(pnl)}">${inr(pnl)}</td><td class="${cls(pp)}">${pct(pp)}</td>
          <td>${daysOpen(t)}</td>
          <td><button class="eq-btn small danger" data-exit="${esc(t.id)}" data-sym="${esc(t.symbol)}">Exit</button></td>
        </tr>`;
      }).join("")
    : `<tr><td colspan="14" class="eq-empty">No open positions.</td></tr>`;

  const closed = (v.trades || []).filter((t) => t.exitedAt);
  $("eqClosed").innerHTML = closed.length
    ? closed.map((t) => `<tr>
        <td class="sym">${esc(t.symbol)}<span class="nm">${esc(t.name)}</span></td>
        <td>${esc(t.status)}</td><td>${fmt(t.entryPrice)}</td><td>${fmt(t.exitPrice)}</td><td>${fmt(t.qty, 0)}</td>
        <td class="${cls(t.pnl)}">${inr(t.pnl)}</td><td class="${cls(t.pnlPct)}">${pct(t.pnlPct)}</td>
        <td>${t.days || 0}</td><td>${esc(t.exitDate)}</td><td class="dim">${esc(t.exitReason)}</td>
      </tr>`).join("")
    : `<tr><td colspan="10" class="eq-empty">No closed trades yet. Trades close on target, stop, Avoid downgrade or max holding.</td></tr>`;

  $("eqLog").innerHTML = (v.log && v.log.length)
    ? v.log.map((l) => `<div>${esc(l)}</div>`).join("")
    : `<div class="dim">No activity yet.</div>`;

  renderCalls(v);
}

// Equity book order: HIGHEST possible return (upside to target) first, lowest
// last, so both the Paper and Realtime equity engines present the same
// highest-to-lowest opportunity list. Buy confidence and the composite score
// only break ties between two names advertising the same upside.
function callRows() {
  const rows = S.research && Array.isArray(S.research.rows) ? S.research.rows : [];
  return rows
    .map((r) => ({ r, c: r.call || {}, st: String((r.call || {}).status || "") }))
    .sort((a, b) =>
      (Number(b.c.upsidePct) || 0) - (Number(a.c.upsidePct) || 0) ||
      (Number(b.c.confidence) || 0) - (Number(a.c.confidence) || 0) ||
      (Number((b.r.scores || {}).total) || 0) - (Number((a.r.scores || {}).total) || 0));
}

function canBuyStatus(st, allowAccumulate) {
  return st === "Strong Buy" || st === "Buy" || (allowAccumulate && st === "Accumulate");
}

function callsLabel(rows, allowAccumulate) {
  if (!rows.length) return "";
  const actionable = rows.filter((x) => canBuyStatus(x.st, allowAccumulate)).length;
  return `${actionable} actionable / ${rows.length} stocks`;
}

function renderCalls(v) {
  const el = $("eqCalls");
  if (!el) return;
  if (S.callsHold) { S.callsPending = true; return; }
  const s = (v && v.settings) || {};
  const openSyms = new Set(
    ((v && v.trades) || []).filter((t) => !t.exitedAt).map((t) => String(t.symbol || "").toUpperCase())
  );
  if (!S.research) {
    el.innerHTML = `<tr><td colspan="9" class="eq-empty">${
      S.researchErr ? `Equity Research load nahi hua: ${esc(S.researchErr)}` : "Loading Equity Research calls..."
    }</td></tr>`;
    $("eqCallCount").textContent = "";
    return;
  }
  const all = callRows();
  const shown = all.slice(0, S.callsLimit);
  const noBroker = MODE === "live" && !S.broker;
  const sig = [
    S.research.asOf || "",
    !!s.allowAccumulate,
    MODE === "live" && !S.broker,
    S.callsLimit,
    [...openSyms].sort().join(","),
    shown
      .map((x) =>
        [
          x.r.symbol,
          x.st,
          Math.round(Number(x.c.confidence) || 0),
          Math.round((Number(x.c.upsidePct) || 0) * 10),
          Math.round((Number(x.r.price) || Number(x.r.close) || 0) * 100),
          Math.round((Number(x.c.targetPrice) || 0) * 100),
          Math.round((Number(x.c.stopPrice) || 0) * 100),
        ].join(":")
      )
      .join(","),
  ].join("|");
  $("eqCallCount").textContent = callsLabel(all, !!s.allowAccumulate);
  const moreBtn = $("eqMore");
  if (moreBtn) {
    moreBtn.style.display = all.length > shown.length ? "block" : "none";
    moreBtn.textContent = `Load more (${shown.length} / ${all.length})`;
  }
  if (sig === S.callsSig && el.innerHTML) return;
  S.callsSig = sig;
  el.innerHTML = shown.length
    ? shown.map(({ r, c, st }) => {
        const sym = String(r.symbol || "").toUpperCase();
        const held = openSyms.has(sym);
        const tone = st === "Strong Buy" ? "strong" : st === "Buy" ? "buy" : "acc";
        return `<tr>
          <td class="sym">${esc(sym)}<span class="nm">${esc(r.name || "")}</span></td>
          <td><span class="eq-pill ${tone}">${esc(st)}</span></td>
          <td>${fmt(c.confidence, 0)}%</td>
          <td class="up">${pct(c.upsidePct)}</td>
          <td>${fmt(Number(r.price) > 0 ? r.price : r.close)}</td>
          <td class="up">${fmt(c.targetPrice)}</td>
          <td class="down">${fmt(c.stopPrice)}</td>
          <td class="dim">${esc(c.timeframe || "-")}</td>
          <td>${
            held
              ? `<span class="eq-open-tag" title="Engine me is symbol par position already open hai">Position open</span>`
              : canBuyStatus(st, !!s.allowAccumulate)
                ? `<button class="eq-btn small primary" data-buy="${esc(sym)}" data-status="${esc(st)}"${
                    noBroker ? ` disabled title="Dhan connect karo - LIVE buy ke liye broker zaroori hai"` : ""
                  }>Buy</button>`
                : `<span class="dim" title="${esc(st)} call par engine buy nahi karta">no entry</span>`
          }</td>
        </tr>`;
      }).join("")
    : `<tr><td colspan="9" class="eq-empty">Equity Research cache khali hai. Pehle Equity Research tab me scan chalao.</td></tr>`;
}

async function loadResearch() {
  try {
    const c = await api("/api/equity/cache");
    S.research = c || {};
    S.researchErr = "";
  } catch (e) {
    S.researchErr = String(e.message || e);
  }
  renderCalls(S.data ? S.data[MODE] || {} : {});
}

async function buyCall(sym, status) {
  if (!sym) return;
  if (MODE === "live") {
    if (!S.broker) { $("eqError").textContent = "Dhan connected nahi hai."; return; }
    if (!confirm(`LIVE BUY ${sym} (${status})?\n\nReal Dhan CNC delivery order jayega.`)) return;
  } else if (!confirm(`Paper BUY ${sym} (${status})?`)) {
    return;
  }
  S.busy = true;
  try {
    const r = await api(API + "/enter", { method: "POST", body: JSON.stringify({ mode: MODE, symbol: sym }) });
    if (r.error) throw new Error(r.error);
    $("eqError").textContent = "";
    if (r.message) $("eqLog").insertAdjacentHTML("afterbegin", `<div>${esc(r.message)}</div>`);
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
    loadResearch();
  } finally {
    S.busy = false;
  }
}

async function loadSymbols() {
  if (S.symbolsLoaded) return;
  S.symbolsLoaded = true;
  try {
    const c = await api("/api/equity/cache");
    const rows = Array.isArray(c.rows) ? c.rows : [];
    const seen = new Set();
    const opts = [];
    rows.forEach((r) => {
      const sym = String(r.symbol || "").toUpperCase();
      if (!sym || seen.has(sym)) return;
      seen.add(sym);
      opts.push(`<option value="${esc(sym)}">${esc(r.name || "")}</option>`);
    });
    $("eqSyms").innerHTML = opts.join("");
  } catch (_) {
    S.symbolsLoaded = false;
  }
}

async function manualBuy() {
  const sym = String($("mSym").value || "").trim().toUpperCase();
  const qty = Math.floor(Number($("mQty").value) || 0);
  if (!sym) { $("eqError").textContent = "Manual buy ke liye symbol likho."; return; }
  if (MODE === "live") {
    if (!S.broker) { $("eqError").textContent = "Dhan connected nahi hai."; return; }
    const q = qty > 0 ? `${qty} shares` : "auto quantity";
    if (!confirm(`LIVE BUY ${sym} - ${q}?\n\nReal Dhan CNC delivery order jayega.`)) return;
  }
  S.busy = true;
  try {
    const body = { mode: MODE, symbol: sym };
    if (qty > 0) body.qty = qty;
    const r = await api(API + "/enter", { method: "POST", body: JSON.stringify(body) });
    if (r.error) throw new Error(r.error);
    $("eqError").textContent = "";
    $("mSym").value = "";
    $("mQty").value = "";
    if (r.message) $("eqLog").insertAdjacentHTML("afterbegin", `<div>${esc(r.message)}</div>`);
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

function card(value, label, tone) {
  return `<div class="eq-card"><b class="${tone || ""}">${esc(value)}</b><span>${esc(label)}</span></div>`;
}

async function toggle() {
  const on = $("eqToggle").getAttribute("data-on") !== "1";
  if (MODE === "live" && on) {
    if (!S.broker) { $("eqError").textContent = "Dhan is not connected. Connect the broker first."; return; }
    if (!confirm("ARM LIVE EQUITY ENGINE?\n\nThis will place REAL Dhan CNC delivery BUY orders for qualifying Equity Research calls, and later SELL orders on target/stop/Avoid.")) return;
  }
  S.busy = true;
  try {
    const r = await api(API + "/toggle", { method: "POST", body: JSON.stringify({ mode: MODE, enabled: on }) });
    if (r.error) throw new Error(r.error);
    if (r.engine) render(r.engine);
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

// Field id -> settings key. Autosave sends ONLY the fields the operator actually
// touched, so a stale value in some other box can never clobber a setting that
// was changed elsewhere (or by another tab).
const SETTING_FIELDS = {
  sPer: () => ({ perTradePct: Number($("sPer").value) || 0 }),
  sMax: () => ({ maxPositions: Number($("sMax").value) || 0 }),
  sConf: () => ({ minConfidence: Number($("sConf").value) || 0 }),
  sUp: () => ({ minUpsidePct: Number($("sUp").value) || 0 }),
  sDays: () => ({ maxHoldingDays: Number($("sDays").value) || 0 }),
  sAuto: () => ({ autoExit: $("sAuto").checked }),
  sAcc: () => ({ allowAccumulate: $("sAcc").checked }),
  sDist: () => ({ autoDistribute: $("sDist").checked }),
  sCharges: () => ({ chargesAware: $("sCharges").checked }),
  sQty: () => ({ fixedQty: Math.max(0, Math.floor(Number($("sQty").value) || 0)) }),
  sTrailOn: () => ({ trailSlOn: $("sTrailOn").checked }),
  sTrailPct: () => ({ trailSlPct: Number($("sTrailPct").value) || 0 }),
  sNetTpOn: () => ({ netTpOn: $("sNetTpOn").checked }),
  sNetTpMin: () => ({ netTpMin: Number($("sNetTpMin").value) || 0 }),
  sNetTpGb: () => ({ netTpGivebackPct: Number($("sNetTpGb").value) || 0 }),
  sWatch: () => ({
    watchSymbols: String($("sWatch").value || "")
      .split(/[,\s;]+/)
      .map((x) => x.trim().toUpperCase())
      .filter(Boolean),
  }),
};

async function saveSettings(full) {
  const body = { mode: MODE };
  if (full) {
    for (const fn of Object.values(SETTING_FIELDS)) Object.assign(body, fn());
  } else {
    for (const id of S.dirty) {
      const fn = SETTING_FIELDS[id];
      if (fn) Object.assign(body, fn());
    }
  }
  if (!full && Object.keys(body).length === 1) return;
  S.busy = true;
  try {
    const r = await api(API + "/settings", { method: "POST", body: JSON.stringify(body) });
    if (r.error) throw new Error(r.error);
    $("eqError").textContent = "";
    S.dirty.clear();
    if (r.engine) render(r.engine);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

async function toggleAutoBuy() {
  const btn = $("eqAutoBuy");
  if (!btn) return;
  const currentlyOn = btn.getAttribute("data-on") === "1";
  const next = !currentlyOn;
  if (next && MODE === "live" && !S.broker) {
    $("eqError").textContent = "Dhan connected nahi hai - LIVE auto buy enable nahi ho sakta.";
    return;
  }
  if (next && MODE === "live" && !confirm("LIVE Auto Buy ON?\n\nEngine REAL Dhan CNC delivery orders place karega research Buy calls par.")) return;
  S.busy = true;
  try {
    const r = await api(API + "/settings", { method: "POST", body: JSON.stringify({ mode: MODE, autoBuy: next }) });
    if (r.error) throw new Error(r.error);
    $("eqError").textContent = "";
    if (r.engine) {
      S.dirty.delete("sAutoBuy");
      render(r.engine);
    }
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

async function resetPnl() {
  if (MODE === "live") return;
  if (!confirm("Reset realised P&L?\n\nSaare closed trades hata diye jaayenge (realised P&L 0 ho jayega). Open positions safe rahenge.")) return;
  S.busy = true;
  try {
    const r = await api(API + "/reset-pnl", { method: "POST", body: JSON.stringify({ mode: MODE }) });
    if (r.error) throw new Error(r.error);
    $("eqError").textContent = "";
    if (r.engine) render(r.engine);
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

async function clearBook() {
  if (MODE === "live") return;
  if (!confirm("Clear the whole paper book?\n\nSaare added stocks (open + closed) hata diye jaayenge aur realised P&L reset ho jaayega. Ye undo nahi hota.")) return;
  S.busy = true;
  try {
    const r = await api(API + "/clear", { method: "POST", body: JSON.stringify({ mode: MODE }) });
    if (r.error) throw new Error(r.error);
    $("eqError").textContent = "";
    if (r.engine) render(r.engine);
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}

async function exitTrade(id) {
  S.busy = true;
  try {
    const r = await api(API + "/exit", { method: "POST", body: JSON.stringify({ mode: MODE, id }) });
    if (r.error) throw new Error(r.error);
    if (r.engine) render(r.engine);
    await load(true);
  } catch (e) {
    $("eqError").textContent = String(e.message || e);
  } finally {
    S.busy = false;
  }
}
