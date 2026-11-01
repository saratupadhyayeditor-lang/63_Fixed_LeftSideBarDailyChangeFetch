const API = "/api/equity/engines";

const $ = (id) => document.getElementById(id);
const esc = (v) => String(v == null ? "" : v).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmt = (v, d = 2) => (Number.isFinite(Number(v)) ? Number(v).toLocaleString("en-IN", { maximumFractionDigits: d, minimumFractionDigits: d }) : "-");
const inr = (v) => "Rs " + fmt(v, 0);
const pct = (v) => (Number.isFinite(Number(v)) ? (Number(v) >= 0 ? "+" : "") + fmt(v, 2) + "%" : "-");
const cls = (v) => (Number(v) > 0 ? "up" : Number(v) < 0 ? "down" : "dim");
let timer = null;
let busy = false;

async function api(path) {
  const r = await fetch(path, { headers: { "Content-Type": "application/json" } });
  const txt = await r.text();
  let body = null;
  try { body = txt ? JSON.parse(txt) : null; } catch (_) { body = null; }
  if (!r.ok) throw new Error((body && body.error) || txt || r.statusText);
  return body;
}

export function bootEqStats() {
  $("stRefresh").addEventListener("click", () => load(true));
  load();
  timer = setInterval(() => {
    const fe = window.frameElement;
    if (fe && !(fe.offsetParent && fe.clientHeight > 0)) return;
    load();
  }, 8000);
}

async function load(force) {
  if (busy && !force) return;
  busy = true;
  try {
    const d = await api(API);
    $("stError").textContent = "";
    $("stBroker").textContent = d.brokerConnected ? "Dhan: connected" : "Dhan: not connected";
    $("stBroker").className = "eq-broker " + (d.brokerConnected ? "ok" : "bad");
    renderEngine("Paper", d.paper || {});
    renderEngine("Live", d.live || {});
    renderCompare(d.paper || {}, d.live || {});
  } catch (e) {
    $("stError").textContent = String(e.message || e);
  } finally {
    busy = false;
  }
}

function renderEngine(prefix, v) {
  const s = v.stats || {};
  $("st" + prefix + "Cards").innerHTML = [
    card(v.enabled ? "ON" : "OFF", "Engine", v.enabled ? "up" : "dim"),
    card(s.open || 0, "Open"),
    card(s.closed || 0, "Closed"),
    card((s.winRate || 0).toFixed(1) + "%", "Win rate"),
    card(inr(s.realizedPnl || 0), "Realized", cls(s.realizedPnl)),
    card(inr(s.unrealizedPnl || 0), "Unrealized", cls(s.unrealizedPnl)),
    card(inr(s.invested || 0), "Invested"),
    card((s.avgDays || 0).toFixed(0) + " d", "Avg holding"),
  ].join("");
  const open = (v.trades || []).filter((t) => !t.exitedAt);
  $("st" + prefix + "Open").innerHTML = open.length
    ? open.map((t) => {
        const pnl = (Number(t.lastPrice) - Number(t.entryPrice)) * Number(t.qty);
        const pp = Number(t.entryPrice) > 0 ? (Number(t.lastPrice) / Number(t.entryPrice) - 1) * 100 : 0;
        const days = t.openedAt ? Math.max(0, Math.floor((Date.now() / 1000 - t.openedAt) / 86400)) : 0;
        return `<tr><td class="sym">${esc(t.symbol)}</td><td>${fmt(t.entryPrice)}</td><td>${fmt(t.lastPrice)}</td>
          <td>${fmt(t.qty, 0)}</td><td class="${cls(pnl)}">${inr(pnl)}</td><td class="${cls(pp)}">${pct(pp)}</td><td>${days}</td></tr>`;
      }).join("")
    : `<tr><td colspan="7" class="eq-empty">No open positions.</td></tr>`;
  const closed = (v.trades || []).filter((t) => t.exitedAt);
  $("st" + prefix + "Closed").innerHTML = closed.length
    ? closed.map((t) => `<tr><td class="sym">${esc(t.symbol)}</td><td>${fmt(t.entryPrice)}</td><td>${fmt(t.exitPrice)}</td>
        <td class="${cls(t.pnl)}">${inr(t.pnl)}</td><td class="${cls(t.pnlPct)}">${pct(t.pnlPct)}</td><td>${t.days || 0}</td>
        <td class="dim">${esc(t.exitReason)}</td></tr>`).join("")
    : `<tr><td colspan="7" class="eq-empty">No closed trades.</td></tr>`;
}

function renderCompare(p, l) {
  const a = p.stats || {};
  const b = l.stats || {};
  const rows = [
    ["Engine", p.enabled ? "ON" : "OFF", l.enabled ? "ON" : "OFF"],
    ["Open positions", a.open || 0, b.open || 0],
    ["Closed trades", a.closed || 0, b.closed || 0],
    ["Wins / losses", `${a.wins || 0} / ${a.losses || 0}`, `${b.wins || 0} / ${b.losses || 0}`],
    ["Win rate", (a.winRate || 0).toFixed(1) + "%", (b.winRate || 0).toFixed(1) + "%"],
    ["Realized P&L", inr(a.realizedPnl || 0), inr(b.realizedPnl || 0)],
    ["Unrealized P&L", inr(a.unrealizedPnl || 0), inr(b.unrealizedPnl || 0)],
    ["Invested", inr(a.invested || 0), inr(b.invested || 0)],
    ["Current value", inr(a.currentValue || 0), inr(b.currentValue || 0)],
    ["Profit factor", fmt(a.profitFactor || 0), fmt(b.profitFactor || 0)],
    ["Expectancy / trade", inr(a.expectancy || 0), inr(b.expectancy || 0)],
    ["Avg holding days", fmt(a.avgDays || 0, 0), fmt(b.avgDays || 0, 0)],
    ["Best / worst", pct(a.bestPct || 0) + " / " + pct(a.worstPct || 0), pct(b.bestPct || 0) + " / " + pct(b.worstPct || 0)],
  ];
  $("stCompare").innerHTML = rows.map(([k, x, y]) => `<tr><td class="sym">${esc(k)}</td><td>${esc(x)}</td><td>${esc(y)}</td></tr>`).join("");
}

function card(value, label, tone) {
  return `<div class="eq-card"><b class="${tone || ""}">${esc(value)}</b><span>${esc(label)}</span></div>`;
}
