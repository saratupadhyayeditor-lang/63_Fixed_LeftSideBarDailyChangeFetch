// Broker Account tab (Rust `/api/account`).
//
// Ported 1:1 from the old Python Flask app's dedicated "Account" tab: four
// balance cards (Available Balance / Used Margin / Collateral / Day P&L) plus
// the Open Positions and Holdings tables, with a manual Refresh. Every value
// comes from the Rust `/api/account` endpoint which reads the live Dhan session
// in pure Rust - this module never talks to the broker itself.

import { istTime } from "./ist.js?v=1";

let booted = false;

function el(id) {
  return document.getElementById(id);
}

function num(v) {
  const n = Number(v);
  return Number.isFinite(n) ? n : 0;
}

function fmt(v) {
  return num(v).toLocaleString("en-IN", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

function esc(s) {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function pnlClass(v) {
  return num(v) >= 0 ? "green" : "red";
}

function setText(id, text) {
  const e = el(id);
  if (e) e.textContent = text;
}

function setStatus(text, color) {
  const s = el("accountStatus");
  if (!s) return;
  s.textContent = text;
  s.style.color = color || "#888";
}

function render(data) {
  const bal = (data && data.balance) || {};
  setText("acAvailable", fmt(bal.available));
  setText("acUsedMargin", fmt(bal.used_margin));
  setText("acCollateral", fmt(bal.collateral));

  const pnlEl = el("acPnl");
  if (pnlEl) {
    pnlEl.textContent = fmt(data && data.pnl);
    pnlEl.className = "value " + (num(data && data.pnl) >= 0 ? "green" : "red");
  }

  const posBody = el("accountPositionsBody");
  const positions = (data && data.positions) || [];
  if (posBody) {
    if (!positions.length) {
      posBody.innerHTML = '<tr><td colspan="9" class="account-empty">No open positions</td></tr>';
    } else {
      posBody.innerHTML = positions
        .map((p) => {
          const cls = pnlClass(p.pnl);
          return `<tr>
            <td><b>${esc(p.symbol)}</b></td>
            <td>${esc(p.exchange)}</td>
            <td>${esc(p.type)}</td>
            <td>${esc(p.qty)}</td>
            <td>${fmt(p.buy_avg)}</td>
            <td>${fmt(p.ltp)}</td>
            <td class="${cls}">${fmt(p.pnl)}</td>
            <td class="${cls}">${fmt(p.pnl_pct)}%</td>
            <td>${esc(p.product)}</td>
          </tr>`;
        })
        .join("");
    }
  }

  const holdBody = el("accountHoldingsBody");
  const holdings = (data && data.holdings) || [];
  if (holdBody) {
    if (!holdings.length) {
      holdBody.innerHTML = '<tr><td colspan="8" class="account-empty">No holdings</td></tr>';
    } else {
      holdBody.innerHTML = holdings
        .map((h) => {
          const cls = pnlClass(h.pnl);
          return `<tr>
            <td><b>${esc(h.symbol)}</b></td>
            <td>${esc(h.exchange)}</td>
            <td>${esc(h.qty)}</td>
            <td>${fmt(h.buy_avg)}</td>
            <td>${fmt(h.ltp)}</td>
            <td class="${cls}">${fmt(h.pnl)}</td>
            <td class="${cls}">${fmt(h.pnl_pct)}%</td>
            <td style="font-size:9px">${esc(h.isin || "-")}</td>
          </tr>`;
        })
        .join("");
    }
  }
}

async function refresh() {
  const btn = el("accountRefreshBtn");
  if (btn) {
    btn.disabled = true;
    btn.textContent = "Refreshing...";
  }
  try {
    const r = await fetch("/api/account", { cache: "no-store" });
    const d = await r.json();
    if (d && d.status === "success") {
      render(d.data || {});
      setStatus("Updated " + istTime(Date.now()), "#00d4aa");
    } else {
      render({});
      setStatus((d && d.message) || "Account unavailable", "#ff9800");
    }
  } catch (e) {
    render({});
    setStatus("Account error: " + e, "#ef5350");
  }
  if (btn) {
    btn.disabled = false;
    btn.textContent = "Refresh";
  }
}

function active() {
  const e = el("tab-account");
  return !!e && e.classList.contains("active");
}

export function bootAccount() {
  if (booted) return;
  booted = true;
  const btn = el("accountRefreshBtn");
  if (btn) btn.onclick = () => refresh();
  document.addEventListener("tabshown", (ev) => {
    if (ev.detail === "account") refresh();
  });
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && active()) refresh();
  });
  setInterval(() => {
    if (active()) refresh();
  }, 5000);
}
