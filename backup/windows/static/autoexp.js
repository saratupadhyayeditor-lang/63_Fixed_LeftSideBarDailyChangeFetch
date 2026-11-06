const API = "/api/experiment";

const $ = (id) => document.getElementById(id);
const esc = (v) => String(v == null ? "" : v).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmt = (v, d = 2) => (Number.isFinite(Number(v)) ? Number(v).toLocaleString("en-IN", { maximumFractionDigits: d, minimumFractionDigits: d }) : "-");
const cls = (v) => (Number(v) > 0 ? "up" : Number(v) < 0 ? "down" : "dim");

async function api(path, body) {
  const opt = body
    ? { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }
    : { headers: { "Content-Type": "application/json" } };
  const r = await fetch(path, opt);
  const txt = await r.text();
  let data = null;
  try { data = txt ? JSON.parse(txt) : null; } catch (_) { data = null; }
  if (!r.ok) throw new Error((data && (data.error || data.message)) || txt || r.statusText);
  return data;
}

let timer = null;
// Remember the latest result rows so the per-side save buttons can send keys.
let LAST = { bullish: [], bearish: [], results: [] };

export function bootAutoExp() {
  $("aeRun").addEventListener("click", run);
  $("aeStop").addEventListener("click", stop);
  $("aeClear").addEventListener("click", clear);
  $("aeSaveBull").addEventListener("click", () => saveKeys("bullish"));
  $("aeSaveBear").addEventListener("click", () => saveKeys("bearish"));
  $("aeSaveAll").addEventListener("click", () => saveAll());
  load();
  // Poll only while this pane is actually visible in the parent tab switcher.
  timer = setInterval(() => {
    const fe = window.frameElement;
    if (fe && !(fe.offsetParent && fe.clientHeight > 0)) return;
    load();
  }, 3000);
}

async function run() {
  $("aeError").textContent = "";
  $("aeSaved").textContent = "";
  try {
    await api(API + "/start", {
      universe: $("aeUniverse").value,
      years: Number($("aeYears").value) || 3,
      minTrades: Number($("aeMinTrades").value) || 8,
    });
    load();
  } catch (e) {
    $("aeError").textContent = String(e.message || e);
  }
}

async function stop() {
  $("aeStop").disabled = true;
  $("aeStop").textContent = "Stopping...";
  try {
    await api(API + "/stop", {});
    load();
  } catch (e) {
    $("aeError").textContent = String(e.message || e);
  } finally {
    $("aeStop").textContent = "Stop";
  }
}

async function clear() {
  try {
    await api(API + "/clear", {});
    load();
  } catch (e) {
    $("aeError").textContent = String(e.message || e);
  }
}

// Build the "securityId|side|combo|variant" key the backend uses to match rows.
function rowKey(r) {
  return [r.securityId, r.side, r.combo, r.variant].join("|");
}

async function saveKeys(side) {
  $("aeError").textContent = "";
  const rows = side === "bullish" ? LAST.bullish : LAST.bearish;
  if (!rows.length) {
    $("aeError").textContent = "No " + side + " results to save - run an experiment first.";
    return;
  }
  try {
    const d = await api(API + "/save", { all: false, keys: rows.map(rowKey) });
    $("aeSaved").textContent = "Saved " + (d.saved || 0) + " " + side + " template(s) to the Universal Template Pool (serial up to " + (d.serial || 0) + ").";
    load();
  } catch (e) {
    $("aeError").textContent = String(e.message || e);
  }
}

async function saveAll() {
  $("aeError").textContent = "";
  try {
    const d = await api(API + "/save", { all: true });
    $("aeSaved").textContent = "Saved " + (d.saved || 0) + " template(s) to the Universal Template Pool (serial up to " + (d.serial || 0) + ").";
    load();
  } catch (e) {
    $("aeError").textContent = String(e.message || e);
  }
}

async function load() {
  try {
    const d = await api(API + "/status");
    render(d);
  } catch (e) {
    $("aeError").textContent = String(e.message || e);
  }
}

function render(d) {
  const running = !!d.running;
  $("aeRun").disabled = running;
  $("aeStop").disabled = !running;
  $("aeClear").disabled = running;
  const hasRows = !!(d.results && d.results.length);
  $("aeSaveBull").disabled = running || !((d.bullish || []).length);
  $("aeSaveBear").disabled = running || !((d.bearish || []).length);
  $("aeSaveAll").disabled = running || !hasRows;

  let prog = d.phase || "idle";
  if (running) prog = "Running " + (d.done || 0) + "/" + (d.total || 0) + (d.current ? " - " + d.current : "") + " (" + (d.results || []).length + " ranked so far)";
  else if (d.phase === "done") prog = "Done - " + (d.bullish || []).length + " bullish, " + (d.bearish || []).length + " bearish over " + (d.years || "-") + "y.";
  else if (d.phase === "stopped") prog = "Stopped at " + (d.done || 0) + "/" + (d.total || 0);
  $("aeProgress").textContent = prog;

  if (!running && d.saved > 0 && (d.savedNames || []).length) {
    $("aeSaved").textContent = "Saved " + d.saved + " template(s): " + d.savedNames.slice(0, 3).join(", ") + (d.savedNames.length > 3 ? ", ..." : "");
  }

  LAST = {
    bullish: d.bullish || [],
    bearish: d.bearish || [],
    results: d.results || [],
  };

  let trades = 0, pnl = 0, wins = 0;
  for (const r of LAST.results) {
    trades += Number(r.trades || 0);
    pnl += Number(r.pnl || 0);
    if (Number(r.winRate) >= 50) wins += 1;
  }
  $("aeCards").innerHTML = [
    card(LAST.bullish.length, "Bullish"),
    card(LAST.bearish.length, "Bearish"),
    card(trades, "Trades"),
    card(LAST.results.length ? (wins / LAST.results.length * 100).toFixed(1) + "%" : "-", "Profitable"),
    card("Rs " + fmt(pnl, 0), "Total P&L", cls(pnl)),
  ].join("");

  renderList("aeBullBody", LAST.bullish);
  renderList("aeBearBody", LAST.bearish);
}

function renderList(bodyId, rows) {
  const body = $(bodyId);
  if (!rows.length) {
    body.innerHTML = '<tr><td colspan="10" class="eq-empty">No results yet - press Run Auto Strategy.</td></tr>';
    return;
  }
  body.innerHTML = rows.map((r, i) => {
    const exit = esc([r.variant, "SL " + r.slPct + "%", "RR " + r.rr].filter(Boolean).join(" / "));
    return `<tr>
      <td>${i + 1}</td>
      <td class="sym">${esc(r.symbol)}</td>
      <td>${esc(r.universe)}</td>
      <td class="combo" title="${esc((r.filters || []).join(" + "))}">${esc(r.combo)}</td>
      <td>${exit}</td>
      <td>${r.trades}</td>
      <td class="${cls(r.winRate - 50)}">${fmt(r.winRate, 1)}%</td>
      <td class="${cls(r.pnl)}">${fmt(r.pnl, 0)}</td>
      <td>${fmt(r.maxDd, 0)}</td>
      <td>${fmt(r.profitFactor, 2)}</td>
    </tr>`;
  }).join("");
}

function card(v, label, c) {
  return `<div class="eq-card"><b class="${c || ""}">${esc(v)}</b><span>${esc(label)}</span></div>`;
}
