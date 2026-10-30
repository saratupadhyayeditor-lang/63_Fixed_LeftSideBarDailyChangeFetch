// Target Lab - multi-factor trend target research (indices + MCX commodities).
//
// Reads `/api/target/cache`, which the Rust target engine refreshes every 10
// minutes from live 5-minute candles + the option chain. Each card shows the
// confluence target, invalidation, reward:risk, confidence, OI/PCR/IV read,
// the historical hit-rate and every factor that voted on the level.
//
// Each engine has an "Auto-cut" switch: when ON, new entries attach the cached
// plan for their underlying and the engine exits as soon as the UNDERLYING
// reaches the target (a TREND_TARGET exit).

const ENGINES = [
  { id: "rt", base: "/api/rt", label: "Realtime 1" },
  { id: "rt2", base: "/api/realtime2", label: "Realtime 2" },
  { id: "rt3", base: "/api/realtime3", label: "Realtime 3" },
  { id: "pt1", base: "/api/paper", label: "Paper 1" },
  { id: "pt2", base: "/api/paper2", label: "Paper 2" },
  { id: "pt3", base: "/api/paper3", label: "Paper 3" },
];

const STYLE_ID = "rtTargetLabStyle";
const state = { rows: [], at: 0, engine: {}, busy: false };

function q(sel, root) {
  return (root || document).querySelector(sel);
}

function num(v) {
  const n = Number(v);
  return Number.isFinite(n) ? n : 0;
}

function esc(s) {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}

function fmt(v, dp) {
  const n = num(v);
  return n.toLocaleString("en-IN", { maximumFractionDigits: dp == null ? 2 : dp, minimumFractionDigits: dp == null ? 2 : dp });
}

function ago(ms) {
  if (!ms) return "-";
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return s + "s ago";
  if (s < 3600) return Math.round(s / 60) + "m ago";
  return Math.round(s / 3600) + "h ago";
}

function installStyle() {
  if (document.getElementById(STYLE_ID)) return;
  const st = document.createElement("style");
  st.id = STYLE_ID;
  st.textContent = `
  #rtTargetLab{padding:10px;font-size:11px;color:#d8d8e8;overflow-y:auto;height:100%}
  #rtTargetLab .tl-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap;border:1px solid #2d2d50;border-radius:5px;padding:8px 10px;background:#12122a;margin-bottom:10px}
  #rtTargetLab .tl-title{font-size:13px;font-weight:700;color:#c4a9ff;letter-spacing:.4px}
  #rtTargetLab .tl-sub{font-size:10px;color:#888}
  #rtTargetLab .tl-btn{background:#2d2d5e;border:1px solid #4a4a80;color:#e6e6ff;border-radius:3px;padding:3px 10px;font-size:10px;cursor:pointer;font-weight:700}
  #rtTargetLab .tl-btn:hover{border-color:#b39ddb}
  #rtTargetLab .tl-eng{width:100%;border-collapse:collapse;margin-bottom:10px}
  #rtTargetLab .tl-eng th,#rtTargetLab .tl-eng td{border:1px solid #23234a;padding:4px 6px;text-align:left;font-size:10px}
  #rtTargetLab .tl-eng th{color:#8888b8;background:#101027;font-weight:600}
  #rtTargetLab .tl-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(330px,1fr));gap:9px}
  #rtTargetLab .tl-card{border:1px solid #2a2a50;border-left:4px solid #555;border-radius:5px;padding:8px 10px;background:#0d0d1e}
  #rtTargetLab .tl-card.long{border-left-color:#26a69a}
  #rtTargetLab .tl-card.short{border-left-color:#ef5350}
  #rtTargetLab .tl-card.neutral{border-left-color:#ffb74d}
  #rtTargetLab .tl-row{display:flex;align-items:center;justify-content:space-between;gap:8px;flex-wrap:wrap}
  #rtTargetLab .tl-name{font-size:12px;font-weight:700;color:#fff}
  #rtTargetLab .tl-dir{font-size:9px;font-weight:700;border-radius:8px;padding:1px 7px}
  #rtTargetLab .tl-dir.long{background:#123f34;color:#7CFFB2}
  #rtTargetLab .tl-dir.short{background:#4a1212;color:#ff9d9d}
  #rtTargetLab .tl-dir.neutral{background:#4a3a10;color:#ffd479}
  #rtTargetLab .tl-kv{display:grid;grid-template-columns:repeat(3,1fr);gap:4px 8px;margin:7px 0}
  #rtTargetLab .tl-k{font-size:9px;color:#7777a0}
  #rtTargetLab .tl-v{font-size:11px;font-weight:700;color:#eaeaf5}
  #rtTargetLab .tl-bar{height:5px;border-radius:3px;background:#1c1c38;overflow:hidden;margin-top:3px}
  #rtTargetLab .tl-bar>i{display:block;height:100%}
  #rtTargetLab .tl-badges{display:flex;gap:4px;flex-wrap:wrap;margin-top:5px}
  #rtTargetLab .tl-badge{font-size:9px;border:1px solid #30305a;border-radius:8px;padding:1px 6px;background:#161630;color:#b9b9d8}
  #rtTargetLab .tl-badge.good{border-color:#2d7e5a;color:#8ff0bd}
  #rtTargetLab .tl-badge.warn{border-color:#7e5a2d;color:#ffd18f}
  #rtTargetLab .tl-badge.bad{border-color:#7e2d2d;color:#ffaaaa}
  #rtTargetLab .tl-facts{font-size:9.5px;color:#a9a9c8;margin-top:6px;line-height:1.5}
  #rtTargetLab .tl-warn{font-size:9.5px;color:#ffb74d;margin-top:5px;line-height:1.45}
  #rtTargetLab .tl-empty{font-size:11px;color:#7777a0;padding:8px}
  #rtTargetLab input[type=number]{background:#1a1a35;border:1px solid #35356a;color:#eaeaf5;border-radius:3px;padding:2px 5px;font-size:10px;width:52px}
  #rtTargetLab input[type=checkbox]{accent-color:#b39ddb;width:14px;height:14px}
  `;
  document.head.appendChild(st);
}

function factorLine(f) {
  return `<span class="tl-badge">${esc(f.kind)} <b>${fmt(f.weight, 1)}</b></span>`;
}

function edgeBadges(e, dir) {
  if (!e) return "";
  const out = [];
  const r = e.openingRange || {};
  if (r.ready && num(r.high) > 0) {
    let cls = "warn";
    let txt = "OR inside";
    if (r.failed) { cls = "bad"; txt = "OR breakout failed"; }
    else if (r.failedDown) { cls = "bad"; txt = "OR breakdown failed"; }
    else if (r.breakout) { cls = r.retest ? "good" : "warn"; txt = r.retest ? "OR break+retest" : "OR breakout"; }
    else if (r.breakdown) { cls = r.retestDown ? "good" : "warn"; txt = r.retestDown ? "OR break+retest down" : "OR breakdown"; }
    out.push(`<span class="tl-badge ${cls}">${txt} ${fmt(r.low, 0)}-${fmt(r.high, 0)}</span>`);
  }
  const bull = e.liquidity && e.liquidity.bullSweep;
  const bear = e.liquidity && e.liquidity.bearSweep;
  if (bull) out.push(`<span class="tl-badge good">Sweep low reclaimed ${fmt(bull.level, 0)} (${num(bull.barsAgo)}b)</span>`);
  if (bear) out.push(`<span class="tl-badge bad">Sweep high rejected ${fmt(bear.level, 0)} (${num(bear.barsAgo)}b)</span>`);
  const fb = e.fvg && e.fvg.bullish;
  const fs = e.fvg && e.fvg.bearish;
  if (fb) out.push(`<span class="tl-badge good">Bull FVG ${fmt(fb.low, 0)}-${fmt(fb.high, 0)} (${fmt(fb.fillPct, 0)}% filled)</span>`);
  if (fs) out.push(`<span class="tl-badge warn">Bear FVG ${fmt(fs.low, 0)}-${fmt(fs.high, 0)} (${fmt(fs.fillPct, 0)}% filled)</span>`);
  const v = e.volume || {};
  if (num(v.buyPressure) > 0) {
    const bearish = num(dir) < 0;
    const strength = bearish ? 100 - num(v.buyPressure) : num(v.buyPressure);
    const cls = strength >= 60 ? "good" : strength < 40 ? "bad" : "warn";
    out.push(`<span class="tl-badge ${cls}">${bearish ? "Sell" : "Buy"} pressure ${fmt(strength, 0)}/100 &middot; vol z ${fmt(v.z, 1)}</span>`);
  }
  const c = e.components || {};
  if (c.available) {
    const cls = c.bearish ? "bad" : c.lead ? (c.confirmed ? "good" : "warn") : "warn";
    const txt = c.bearish ? "Heavies bearish" : c.lead ? (c.confirmed ? "Heavies lead + volume" : "Heavies lead, no volume") : "Heavies neutral";
    out.push(`<span class="tl-badge ${cls}">${txt} ${fmt(c.retPct, 2)}% vs idx (${fmt(c.divergencePct, 2)}%) &middot; ${esc(c.source || "")}</span>`);
  }
  return out.join(" ");
}

function researchBlock(row) {
  const r = row.research;
  if (!r) return "";
  const reg = r.regime || {};
  const mt = r.multiTimeframe || {};
  const gl = r.global || {};
  const fl = r.flows || {};
  const sec = r.sectors || {};
  const gx = r.gamma || {};
  const com = r.commodity || {};
  const feas = r.feasibility || {};
  const ev = r.events || {};
  const cal = r.calibration || {};
  const th = r.thesis || {};
  const badges = [];
  if (reg.label) {
    const cls = reg.label === "TREND" ? "good" : reg.label === "RANGE" ? "bad" : reg.label === "VOLATILE" ? "warn" : "";
    badges.push(`<span class="tl-badge ${cls}">${esc(reg.label)} &middot; ADX ${fmt(reg.adx, 0)} &middot; CHOP ${fmt(reg.chop, 0)}</span>`);
  }
  if (mt.aligned != null) {
    const cls = num(mt.aligned) >= 3 ? "good" : num(mt.against) >= 2 ? "bad" : "";
    const against = num(mt.against) ? ` &middot; ${num(mt.against)} against` : "";
    badges.push(`<span class="tl-badge ${cls}">MTF ${num(mt.aligned)}/4 aligned${against}</span>`);
  }
  if (gl.available) {
    const cls = num(gl.score) > 5 ? "good" : num(gl.score) < -5 ? "bad" : "";
    badges.push(`<span class="tl-badge ${cls}">Global ${num(gl.score) >= 0 ? "+" : ""}${fmt(gl.score, 0)}</span>`);
  }
  if (fl.available) {
    const net = num(fl.fiiNet) + num(fl.diiNet);
    badges.push(`<span class="tl-badge ${net > 0 ? "good" : net < 0 ? "bad" : ""}">FII ${fmt(fl.fiiNet, 0)} &middot; DII ${fmt(fl.diiNet, 0)}</span>`);
  }
  if (sec.available) {
    const cls = num(sec.breadthPct) >= 60 ? "good" : num(sec.breadthPct) <= 40 ? "bad" : "warn";
    badges.push(`<span class="tl-badge ${cls}">Sector breadth ${fmt(sec.breadthPct, 0)}%</span>`);
  }
  if (gx.available && num(gx.flip) > 0) {
    const gcls = gx.regime === "amplifying" ? "warn" : "good";
    badges.push(`<span class="tl-badge ${gcls}">Gamma flip ${fmt(gx.flip, 0)} &middot; net ${fmt(gx.net, 0)} &middot; ${esc(gx.regime || "")}</span>`);
  }
  const season = com.seasonality || {};
  if (season.available) {
    const scls = num(season.avgPct) >= 0 ? "good" : "bad";
    badges.push(`<span class="tl-badge ${scls}">Month season ${num(season.avgPct) >= 0 ? "+" : ""}${fmt(season.avgPct, 2)}% &middot; ${fmt(season.winRate, 0)}% win (${num(season.samples)}y)</span>`);
  } else if (com.rollBasisPct != null) {
    badges.push(`<span class="tl-badge">Roll basis ${fmt(com.rollBasisPct, 3)}%</span>`);
  }
  if (feas.feasible != null) {
    badges.push(`<span class="tl-badge ${feas.feasible ? "good" : "bad"}">${feas.feasible ? "Feasible" : "Infeasible"} &middot; ${fmt(feas.distanceAtr, 1)} ATR &middot; net RR ${fmt(feas.netRr, 2)}</span>`);
  }
  if (ev.blackout) {
    const why = Array.isArray(ev.reasons) ? ev.reasons.join(", ") : "";
    badges.push(`<span class="tl-badge bad">Blackout${why ? ": " + esc(why) : ""}</span>`);
  }
  if (cal.probability != null) {
    badges.push(`<span class="tl-badge">Calibrated ${fmt(cal.probability, 1)}%${row.confidenceRaw != null ? ` (raw ${fmt(row.confidenceRaw, 1)})` : ""}</span>`);
  }
  const att = Array.isArray(r.attribution) ? r.attribution : [];
  const attLine = att.length
    ? `<div class="tl-facts"><b style="color:#c4a9ff">Attribution:</b> ${att.map((a) => `${esc(a.label)} <b style="color:${num(a.delta) >= 0 ? "#7CFFB2" : "#ff9d9d"}">${num(a.delta) >= 0 ? "+" : ""}${fmt(a.delta, 1)}</b>`).join(" &middot; ")}</div>`
    : "";
  const summary = th.summary ? `<div class="tl-facts" style="color:#ddd">${esc(th.summary)}</div>` : "";
  const risks = Array.isArray(th.risks) && th.risks.length
    ? `<div class="tl-warn">${th.risks.slice(0, 3).map((x) => "risk: " + esc(x)).join("<br>")}</div>`
    : "";
  return `${summary}${badges.length ? `<div class="tl-badges">${badges.join(" ")}</div>` : ""}${attLine}${risks}`;
}

function card(row) {
  const dir = num(row.direction);
  const cls = dir > 0 ? "long" : dir < 0 ? "short" : "neutral";
  const txt = dir > 0 ? "BUY CALL" : dir < 0 ? "BUY PUT" : "NO BUY";
  const planName = dir < 0 ? "Buy PUT target" : "Buy CALL target";
  const conf = Math.max(0, Math.min(100, num(row.confidence)));
  const cc = conf >= 70 ? "#26a69a" : conf >= 50 ? "#ffb74d" : "#ef5350";
  const c = row.chain || {};
  const bt = row.backtest || {};
  const factors = Array.isArray(row.factors) ? row.factors.slice(0, 6) : [];
  const warnings = Array.isArray(row.warnings) ? row.warnings : [];
  const warn = warnings.length
    ? `<div class="tl-warn">${warnings.slice(0, 4).map((w) => "! " + esc(w)).join("<br>")}</div>`
    : "";
  const chainBadges = c.available
    ? `<span class="tl-badge ${num(c.pcr) >= 1 ? "good" : "warn"}">PCR ${fmt(c.pcr, 2)}</span>
       <span class="tl-badge">CE wall ${fmt(c.ceWall, 0)}</span>
       <span class="tl-badge">PE wall ${fmt(c.peWall, 0)}</span>
       <span class="tl-badge">Max pain ${fmt(c.maxPain, 0)}</span>
       <span class="tl-badge">IV ${fmt(c.atmIv, 1)}%</span>
       <span class="tl-badge">Exp move ${fmt(c.expectedMove, 1)}</span>`
    : `<span class="tl-badge bad">No option chain</span>`;
  const rr = num(row.rr);
  const btProb = num(bt.trades) >= 10
    ? num(bt.hitRate)
    : rr > 0 ? 100 / (1 + rr) : conf;
  const btBasis = num(bt.trades) >= 10 ? `backtest ${num(bt.trades)} trades` : rr > 0 ? "RR estimate" : "confidence";
  const btBadge = `<span class="tl-badge ${btProb >= 50 ? "good" : "bad"}">Win chance ${fmt(btProb, 1)}% (${btBasis})</span>`;
  return `<div class="tl-card ${cls}">
    <div class="tl-row">
      <span class="tl-name">${esc(row.symbol || row.underlying)}</span>
      <span class="tl-dir ${cls}">${txt}</span>
    </div>
    <div class="tl-row" style="margin-top:2px">
      <span class="tl-sub">Spot ${fmt(row.spot)} &middot; ${esc(row.exchangeSegment || "")} &middot; ${ago(num(row.asOf))}</span>
      <span class="tl-sub" style="color:${cc};font-weight:700">${fmt(conf, 1)}% confidence</span>
    </div>
    <div class="tl-bar"><i style="width:${conf}%;background:${cc}"></i></div>
    <div class="tl-kv">
      <div><div class="tl-k">${planName}</div><div class="tl-v" style="color:${dir < 0 ? "#ff9d9d" : "#7CFFB2"}">${row.target > 0 ? fmt(row.target) : "--"}</div></div>
      <div><div class="tl-k">Invalidation</div><div class="tl-v">${row.stop > 0 ? fmt(row.stop) : "--"}</div></div>
      <div><div class="tl-k">Reward : risk</div><div class="tl-v">${fmt(row.rr, 2)}</div></div>
      <div><div class="tl-k">Extension</div><div class="tl-v">${row.target2 > 0 ? fmt(row.target2) : "--"}</div></div>
      <div><div class="tl-k">Move</div><div class="tl-v">${fmt(row.movePct, 2)}%</div></div>
      <div><div class="tl-k">ATR</div><div class="tl-v">${fmt(row.atr, 2)}</div></div>
    </div>
    <div class="tl-badges">${chainBadges}${edgeBadges(row.edge, dir)}${btBadge}</div>
    ${researchBlock(row)}
    <div class="tl-facts">${factors.map(factorLine).join(" ")}</div>
    ${warn}
  </div>`;
}

function render() {
  const host = document.getElementById("rtTargetLab");
  if (!host) return;
  const rows = state.rows || [];
  const rank = (a, b) =>
    Math.abs(num(b.direction)) - Math.abs(num(a.direction)) ||
    num(b.confidence) - num(a.confidence);
  const indices = rows
    .filter((r) => (r.exchangeSegment || "").toUpperCase() === "IDX_I")
    .sort(rank);
  const comms = rows
    .filter((r) => (r.exchangeSegment || "").toUpperCase() !== "IDX_I")
    .sort(rank);
  const section = (title, list) =>
    `<div class="tl-title" style="margin:10px 0 5px;font-size:11px">${title}</div>` +
    `<div class="tl-grid">${list.length ? list.map(card).join("") : `<div class="tl-empty">No data yet. Press "Refresh now".</div>`}</div>`;
  const engRows = ENGINES.map((e) => {
    const s = state.engine[e.id] || {};
    return `<tr><td>${e.label}</td>
      <td><label><input type="checkbox" data-tl-engine="${e.id}" data-tl-kind="on" ${s.on ? "checked" : ""}> auto-cut</label></td>
      <td><label><input type="checkbox" data-tl-engine="${e.id}" data-tl-kind="confirm" ${s.confirm ? "checked" : ""}> strict</label></td>
      <td><input type="number" data-tl-engine="${e.id}" data-tl-kind="conf" min="0" max="100" step="1" value="${num(s.conf) || 60}"> % min</td></tr>`;
  }).join("");
  host.innerHTML = `
    <div class="tl-head">
      <span class="tl-title">Target Lab</span>
      <span class="tl-sub">Buy-only target edge in both directions (BUY CALL above spot, BUY PUT below spot): opening range, liquidity sweeps, FVG, volume aggression, heavyweight momentum, trend + S/R + OI/PCR/IV. Win chance = walk-forward backtest (or RR estimate); per-trade charges-aware net shows in Running Trades &rarr; Target Lab. Cache: ${ago(state.at)}.</span>
      <button class="tl-btn" id="tlRefresh">${state.busy ? "Refreshing..." : "Refresh now"}</button>
    </div>
    <div class="tl-title" style="font-size:11px;margin-bottom:4px">Engine Target Lab controls: auto-cut = exit at the underlying target; strict = a fresh Target Lab plan chooses the traded CE/PE leg (indices + MCX only), otherwise the normal scanner leg runs; minimum confidence still gates auto-cut/exit</div>
    <table class="tl-eng"><thead><tr><th>Engine</th><th>Auto-cut</th><th>Strict confirm</th><th>Minimum confidence</th></tr></thead><tbody>${engRows}</tbody></table>
    ${section("Indices", indices)}
    ${section("Commodities (MCX)", comms)}
  `;
  const btn = q("#tlRefresh", host);
  if (btn) btn.onclick = refreshNow;
  host.querySelectorAll("[data-tl-engine]").forEach((el) => {
    el.onchange = () => {
      const id = el.getAttribute("data-tl-engine");
      const kind = el.getAttribute("data-tl-kind");
      const cur = state.engine[id] || { on: false, conf: 60, confirm: false };
      if (kind === "on") cur.on = !!el.checked;
      else if (kind === "confirm") cur.confirm = !!el.checked;
      else cur.conf = Math.max(0, Math.min(100, num(el.value)));
      state.engine[id] = cur;
      saveEngine(id);
    };
  });
}

function saveEngine(id) {
  const e = ENGINES.find((x) => x.id === id);
  const s = state.engine[id] || { on: false, conf: 60, confirm: false };
  fetch(e.base + "/settings", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ targetAutoOn: !!s.on, targetConfirmOn: !!s.confirm, targetMinConfidence: num(s.conf) }),
  }).catch(() => {});
}

async function loadEngines() {
  await Promise.all(ENGINES.map(async (e) => {
    try {
      const r = await fetch(e.base + "/snapshot", { cache: "no-store" });
      const d = await r.json();
      const s = (d && d.settings) || {};
      state.engine[e.id] = {
        on: !!s.targetAutoOn,
        confirm: !!s.targetConfirmOn,
        conf: num(s.targetMinConfidence) || 60,
      };
    } catch (_) {
      state.engine[e.id] = state.engine[e.id] || { on: false, conf: 60, confirm: false };
    }
  }));
}

async function loadCache() {
  try {
    const r = await fetch("/api/target/cache", { cache: "no-store" });
    const d = await r.json();
    state.rows = Array.isArray(d.rows) ? d.rows : [];
    state.at = num(d.at);
  } catch (_) {
    /* keep the last good rows */
  }
}

async function refreshNow() {
  if (state.busy) return;
  state.busy = true;
  render();
  try {
    await fetch("/api/target/refresh", { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
  } catch (_) {}
  // The cycle walks 15 symbols sequentially; poll a few times.
  for (const wait of [6000, 12000, 12000]) {
    await new Promise((r) => setTimeout(r, wait));
    await loadCache();
    render();
  }
  state.busy = false;
  render();
}

export function bootTargetLab() {
  const host = document.getElementById("rtTargetLab");
  if (!host) return;
  installStyle();
  host.innerHTML = `<div class="tl-empty">Loading Target Lab...</div>`;
  loadEngines().then(render);
  loadCache().then(render);
  // Refresh at a low cadence, only while the tab is actually visible.
  setInterval(() => {
    const pane = document.getElementById("tab-targetlab");
    const visible = !pane || pane.classList.contains("active") || !document.querySelector(".tab-content.active");
    if (!visible) return;
    loadCache().then(render);
  }, 20000);
  document.addEventListener("tabshown", (ev) => {
    if (ev.detail === "targetlab") {
      loadEngines().then(render);
      loadCache().then(render);
    }
  });
}
