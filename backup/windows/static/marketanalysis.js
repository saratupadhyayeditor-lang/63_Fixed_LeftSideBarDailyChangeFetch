// Market Analysis tab: FII/DII cash flows, participant-wise F&O open interest,
// global indices and the live India indices. Data comes from the single
// `/api/market/analysis` endpoint (server-side caching + browser-safe number
// formatting). The tab only polls while it is the visible tab.

const MA_CSS = `
#tab-marketanalysis { overflow-y: auto; padding: 10px; }
#tab-marketanalysis .ma-head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 8px; }
#tab-marketanalysis .ma-status { font-size: 11px; color: #8888b8; }
#tab-marketanalysis .ma-btn { background: #22224a; color: #eaeaf5; border: 1px solid #35356a; border-radius: 4px; padding: 4px 12px; cursor: pointer; font-size: 12px; }
#tab-marketanalysis .ma-btn:hover { border-color: #b39ddb; }
#tab-marketanalysis .ma-sec { margin-bottom: 16px; }
#tab-marketanalysis .ma-sec > h3 { font-size: 11px; color: #b39ddb; text-transform: uppercase; letter-spacing: 1px; margin: 0 0 6px; display: flex; align-items: baseline; gap: 8px; }
#tab-marketanalysis .ma-sub { font-size: 10px; color: #8888b8; text-transform: none; letter-spacing: 0; }
#tab-marketanalysis .ma-cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 8px; }
#tab-marketanalysis .ma-card { background: #12122a; border: 1px solid #2d2d50; border-radius: 6px; padding: 8px 10px; }
#tab-marketanalysis .ma-card .ma-name { font-size: 11px; color: #9d9dc8; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
#tab-marketanalysis .ma-card .ma-val { font-size: 17px; color: #eaeaf5; margin: 3px 0 1px; font-variant-numeric: tabular-nums; }
#tab-marketanalysis .ma-card .ma-chg { font-size: 11px; font-variant-numeric: tabular-nums; }
#tab-marketanalysis .ma-up { color: #7CFFB2; }
#tab-marketanalysis .ma-down { color: #ff8888; }
#tab-marketanalysis .ma-flat { color: #9d9dc8; }
#tab-marketanalysis .ma-scroll { overflow-x: auto; border: 1px solid #1e1e40; border-radius: 4px; }
#tab-marketanalysis .account-table { width: 100%; border-collapse: collapse; font-size: 10.5px; }
#tab-marketanalysis .account-table th { text-align: right; color: #8888b8; font-weight: 500; padding: 5px 7px; border-bottom: 1px solid #2a2a4a; white-space: nowrap; background: #111127; }
#tab-marketanalysis .account-table th:first-child, #tab-marketanalysis .account-table td:first-child { text-align: left; }
#tab-marketanalysis .account-table td { padding: 5px 7px; border-bottom: 1px solid #1c1c38; white-space: nowrap; text-align: right; font-variant-numeric: tabular-nums; }
#tab-marketanalysis .account-table tr.ma-fii { background: rgba(255,120,120,.07); }
#tab-marketanalysis .account-table tr.ma-dii { background: rgba(120,255,178,.07); }
#tab-marketanalysis .ma-empty { color: #8888b8; font-size: 11px; padding: 6px 2px; }
#tab-marketanalysis .ma-pred { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-bottom: 10px; }
#tab-marketanalysis .ma-verdict { border: 1px solid #2d2d50; border-radius: 8px; padding: 12px 14px; background: #12122a; }
#tab-marketanalysis .ma-verdict.up { border-color: #1f7a4d; background: rgba(30,180,110,.10); }
#tab-marketanalysis .ma-verdict.down { border-color: #8a2a2a; background: rgba(200,60,60,.10); }
#tab-marketanalysis .ma-verdict.flat { border-color: #55557a; }
#tab-marketanalysis .ma-verdict .ma-vkind { font-size: 10px; color: #9d9dc8; text-transform: uppercase; letter-spacing: 1px; }
#tab-marketanalysis .ma-verdict .ma-vlabel { font-size: 22px; font-weight: 600; margin: 4px 0; color: #eaeaf5; }
#tab-marketanalysis .ma-verdict.up .ma-vlabel { color: #7CFFB2; }
#tab-marketanalysis .ma-verdict.down .ma-vlabel { color: #ff8888; }
#tab-marketanalysis .ma-verdict .ma-vmeta { font-size: 11px; color: #b8b8d0; }
#tab-marketanalysis .ma-bar { height: 6px; border-radius: 3px; background: #22224a; margin-top: 8px; overflow: hidden; }
#tab-marketanalysis .ma-bar > i { display: block; height: 100%; background: #b39ddb; }
#tab-marketanalysis .ma-disc { font-size: 10px; color: #8888b8; margin-top: 6px; line-height: 1.5; }
#tab-marketanalysis .ma-news-head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 8px; }
#tab-marketanalysis .ma-news-pill { font-size: 11px; padding: 3px 10px; border-radius: 12px; border: 1px solid #2d2d50; }
#tab-marketanalysis .ma-news-pill.up { border-color: #1f7a4d; background: rgba(30,180,110,.12); color: #7CFFB2; }
#tab-marketanalysis .ma-news-pill.down { border-color: #8a2a2a; background: rgba(200,60,60,.12); color: #ff8888; }
#tab-marketanalysis .ma-news-pill.flat { color: #b8b8d0; }
#tab-marketanalysis .ma-news-pill.risk { border-color: #8a6a2a; background: rgba(200,160,60,.12); color: #ffd479; }
#tab-marketanalysis .ma-events { font-size: 11px; color: #b8b8d0; margin-bottom: 8px; }
#tab-marketanalysis .ma-events b { color: #ffd479; font-weight: 500; }
#tab-marketanalysis .ma-news-list { display: flex; flex-direction: column; gap: 1px; max-height: 420px; overflow-y: auto; border: 1px solid #1e1e40; border-radius: 4px; }
#tab-marketanalysis .ma-news-item { display: flex; align-items: baseline; gap: 8px; padding: 6px 9px; border-bottom: 1px solid #1c1c38; font-size: 11.5px; line-height: 1.4; }
#tab-marketanalysis .ma-news-item:last-child { border-bottom: none; }
#tab-marketanalysis .ma-news-item.bull { border-left: 3px solid #1f7a4d; }
#tab-marketanalysis .ma-news-item.bear { border-left: 3px solid #8a2a2a; }
#tab-marketanalysis .ma-news-item.neutral { border-left: 3px solid #2d2d50; }
#tab-marketanalysis .ma-news-tag { flex: 0 0 auto; font-size: 9.5px; color: #8888b8; text-transform: uppercase; letter-spacing: .5px; }
#tab-marketanalysis .ma-news-src { flex: 0 0 auto; font-size: 9.5px; color: #b39ddb; max-width: 110px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
#tab-marketanalysis .ma-news-item a { color: #eaeaf5; text-decoration: none; flex: 1 1 auto; }
#tab-marketanalysis .ma-news-item a:hover { color: #b39ddb; text-decoration: underline; }
#tab-marketanalysis .ma-news-time { flex: 0 0 auto; font-size: 9.5px; color: #7777a0; font-variant-numeric: tabular-nums; }
#tab-marketanalysis .ma-news-flt { display: flex; gap: 6px; margin-bottom: 8px; flex-wrap: wrap; }
#tab-marketanalysis .ma-news-flt button { background: #1a1a38; color: #c8c8e0; border: 1px solid #33335c; border-radius: 12px; padding: 3px 11px; cursor: pointer; font-size: 11px; }
#tab-marketanalysis .ma-news-flt button.on { border-color: #b39ddb; color: #eaeaf5; background: #26264e; }
#tab-marketanalysis .ma-gap-controls { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 10px; font-size: 11px; color: #b8b8d0; }
#tab-marketanalysis .ma-gap-controls label { display: flex; align-items: center; gap: 4px; }
#tab-marketanalysis .ma-gap-controls select, #tab-marketanalysis .ma-gap-controls input { background: #12122a; color: #eaeaf5; border: 1px solid #35356a; border-radius: 4px; padding: 3px 6px; font-size: 11px; color-scheme: dark; }
#tab-marketanalysis .ma-gap-controls button { background: #22224a; color: #eaeaf5; border: 1px solid #35356a; border-radius: 4px; padding: 3px 10px; cursor: pointer; font-size: 11px; }
#tab-marketanalysis .ma-gap-controls button:hover { border-color: #b39ddb; }
#tab-marketanalysis .ma-gap-sep { color: #55557a; }
#tab-marketanalysis .ma-gap-charts { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin: 10px 0; }
#tab-marketanalysis .ma-chart-box { background: #12122a; border: 1px solid #2d2d50; border-radius: 6px; padding: 8px 10px; min-width: 0; }
#tab-marketanalysis .ma-chart-title { font-size: 11px; color: #9d9dc8; margin-bottom: 6px; }
#tab-marketanalysis .ma-chart-title span { color: #7777a0; }
#tab-marketanalysis .ma-chart-box canvas { width: 100% !important; height: 220px !important; }
#tab-marketanalysis .ma-gap-table { max-height: 300px; overflow: auto; margin-bottom: 10px; }
#tab-marketanalysis .ma-gap-table th { position: sticky; top: 0; z-index: 1; }
#tab-marketanalysis .ma-ok { color: #7CFFB2; }
#tab-marketanalysis .ma-bad { color: #ff8888; }
@media (max-width: 900px) { #tab-marketanalysis .ma-gap-charts { grid-template-columns: 1fr; } }
`;

function maEsc(s) {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function maNum(v) {
  return v == null ? "–" : Number(v).toLocaleString("en-IN", { maximumFractionDigits: 0 });
}

function maPrice(v, cur) {
  if (v == null || v === 0) return "–";
  const n = Number(v).toLocaleString("en-IN", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return cur ? n + " " + cur : n;
}

function maCr(v) {
  if (v == null) return "–";
  const n = Number(v).toLocaleString("en-IN", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return (v > 0 ? "+" : "") + n;
}

function maPct(v) {
  return (v >= 0 ? "+" : "") + Number(v).toFixed(2) + "%";
}

function maCard(name, valueHTML, changeHTML) {
  return '<div class="ma-card"><div class="ma-name">' + maEsc(name) + '</div>' +
    '<div class="ma-val">' + valueHTML + '</div>' +
    '<div class="ma-chg">' + (changeHTML || "") + '</div></div>';
}

function maIndexCard(row) {
  if (!row || !row.ok) {
    return maCard(row && row.name, '<span class="ma-flat">–</span>', '<span class="ma-flat">n/a</span>');
  }
  const cls = row.change > 0 ? "ma-up" : row.change < 0 ? "ma-down" : "ma-flat";
  const chg = '<span class="' + cls + '">' + maPct(row.changePct) + "</span>";
  return maCard(row.name, maPrice(row.price, row.currency), chg);
}

function maRenderFiidii(section) {
  const cards = document.getElementById("maFiidiiCards");
  const dateEl = document.getElementById("maFiidiiDate");
  if (!section || !section.ok) {
    dateEl.textContent = section && section.error ? "(" + section.error + ")" : "";
    cards.innerHTML = '<div class="ma-empty">FII/DII data unavailable.</div>';
    return;
  }
  dateEl.textContent = section.date ? "as on " + section.date : "";
  const row = (label, d) => {
    if (!d) return "";
    const cls = d.net > 0 ? "ma-up" : d.net < 0 ? "ma-down" : "ma-flat";
    return maCard(label, '<span class="' + cls + '">' + maCr(d.net) + " cr</span>",
      "Buy " + maCr(d.buy) + " / Sell " + maCr(d.sell));
  };
  cards.innerHTML = row("FII / FPI Net", section.fii) + row("DII Net", section.dii);
}

function maRenderParticipant(section) {
  const head = document.getElementById("maPartHead");
  const body = document.getElementById("maPartBody");
  const dateEl = document.getElementById("maPartDate");
  if (!section || !section.ok) {
    dateEl.textContent = section && section.error ? "(" + section.error + ")" : "";
    head.innerHTML = "";
    body.innerHTML = '<tr><td class="ma-empty" colspan="15">Participant OI data unavailable.</td></tr>';
    return;
  }
  dateEl.textContent = (section.date ? "as on " + section.date : "") + " · contracts";
  const cols = section.columns || [];
  head.innerHTML = "<tr>" + cols.map((c) => "<th>" + maEsc(c) + "</th>").join("") + "</tr>";
  const rows = section.rows || [];
  body.innerHTML = rows.map((r) => {
    const n = String(r.name || "");
    let cls = "";
    if (/fii|fpi/i.test(n)) cls = ' class="ma-fii"';
    else if (/dii/i.test(n)) cls = ' class="ma-dii"';
    const cells = (r.values || []).map((v) => "<td>" + maNum(v) + "</td>").join("");
    return "<tr" + cls + "><td>" + maEsc(n) + "</td>" + cells + "</tr>";
  }).join("");
}

function maRenderGlobal(section) {
  const el = document.getElementById("maGlobalCards");
  if (!section || !section.ok) {
    el.innerHTML = '<div class="ma-empty">Global index data unavailable.</div>';
    return;
  }
  el.innerHTML = (section.rows || []).map(maIndexCard).join("");
}

function maRenderIndia(section) {
  const el = document.getElementById("maIndiaCards");
  if (!section || !section.ok) {
    el.innerHTML = '<div class="ma-empty">' +
      maEsc(section && section.error ? section.error : "Unavailable") +
      " (connect Dhan for live India indices).</div>";
    return;
  }
  el.innerHTML = (section.rows || []).map(maIndexCard).join("");
}

function maVerdictCard(kind, obj, upDir, downDir) {
  if (!obj) return "";
  let cls = "flat";
  if (obj.direction === upDir) cls = "up";
  else if (obj.direction === downDir) cls = "down";
  const conf = Number(obj.confidence || 0);
  let meta;
  if (obj.estPct != null) {
    meta = "Est. gap " + (obj.estPct >= 0 ? "+" : "") + Number(obj.estPct).toFixed(2) +
      "% · Confidence " + conf + "%";
  } else {
    meta = "Score " + Number(obj.score || 0).toFixed(2) + " · Confidence " + conf + "%";
  }
  return '<div class="ma-verdict ' + cls + '">' +
    '<div class="ma-vkind">' + maEsc(kind) + '</div>' +
    '<div class="ma-vlabel">' + maEsc(obj.label) + '</div>' +
    '<div class="ma-vmeta">' + meta + '</div>' +
    '<div class="ma-bar"><i style="width:' + Math.max(0, Math.min(100, conf)) + '%"></i></div>' +
    '</div>';
}

function maRenderPrediction(pred) {
  const wrap = document.getElementById("maPredCards");
  const body = document.getElementById("maPredFactors");
  const disc = document.getElementById("maPredDisc");
  if (!wrap) return;
  if (!pred || !pred.ok) {
    wrap.innerHTML = '<div class="ma-empty">Prediction unavailable.</div>';
    body.innerHTML = "";
    disc.textContent = "";
    return;
  }
  wrap.innerHTML =
    maVerdictCard("Next-session gap", pred.gap, "up", "down") +
    maVerdictCard("Next-session trend", pred.trend, "bull", "bear");
  const cls = (im) => im === "bull" ? "ma-up" : im === "bear" ? "ma-down" : "ma-flat";
  const rows = (pred.factors || []).map((f) => {
    const grp = f.group === "gap" ? "Gap" : "Trend";
    const pts = (f.points >= 0 ? "+" : "") + Number(f.points).toFixed(2);
    return "<tr><td>" + grp + "</td><td>" + maEsc(f.name) + '</td><td>' + maEsc(f.value) +
      '</td><td class="' + cls(f.impact) + '">' + pts + "</td><td>" + maEsc(f.note) + "</td></tr>";
  }).join("");
  body.innerHTML = rows || '<tr><td colspan="5" class="ma-empty">No factors.</td></tr>';
  disc.textContent = pred.disclaimer || "";
}

let maNewsData = null;
let maNewsFilter = "all";
let maNewsCount = 20;

function maRelTime(ts) {
  if (!ts) return "";
  const s = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (s < 3600) return Math.max(1, Math.floor(s / 60)) + "m";
  if (s < 86400) return Math.floor(s / 3600) + "h";
  return Math.floor(s / 86400) + "d";
}

function maRenderNewsItems() {
  const list = document.getElementById("maNewsList");
  if (!list) return;
  const sec = maNewsData;
  if (!sec || !sec.ok || !(sec.items || []).length) {
    list.innerHTML = '<div class="ma-empty">News feed unavailable (all sources unreachable).</div>';
    return;
  }
  let items = sec.items;
  if (maNewsFilter === "India" || maNewsFilter === "Global") {
    items = items.filter((i) => i.region === maNewsFilter);
  } else if (maNewsFilter === "bull" || maNewsFilter === "bear") {
    items = items.filter((i) => i.sentiment === maNewsFilter);
  }
  items = items.slice(0, maNewsCount);
  if (!items.length) {
    list.innerHTML = '<div class="ma-empty">No items for this filter.</div>';
    return;
  }
  list.innerHTML = items.map((i) => {
    const s = i.sentiment === "bull" ? "bull" : i.sentiment === "bear" ? "bear" : "neutral";
    const tag = maEsc((i.region || "") + " · " + (i.category || ""));
    const title = i.link
      ? '<a href="' + maEsc(i.link) + '" target="_blank" rel="noopener noreferrer">' + maEsc(i.title) + "</a>"
      : "<span>" + maEsc(i.title) + "</span>";
    return '<div class="ma-news-item ' + s + '">' +
      '<span class="ma-news-tag">' + tag + "</span>" +
      '<span class="ma-news-src">' + maEsc(i.source || "") + "</span>" +
      title +
      '<span class="ma-news-time">' + maRelTime(i.ts) + "</span></div>";
  }).join("");
}

function maRenderNews(section, np) {
  const head = document.getElementById("maNewsHead");
  const list = document.getElementById("maNewsList");
  if (!head || !list) return;
  maNewsData = section;

  const pills = [];
  if (np && np.ok) {
    const cls = np.direction === "bull" ? "up" : np.direction === "bear" ? "down" : "flat";
    pills.push('<span class="ma-news-pill ' + cls + '">' + maEsc(np.label) +
      " · " + Number(np.confidence || 0) + "%</span>");
    pills.push('<span class="ma-news-pill ' + (np.eventRisk === "High" ? "risk" : "flat") +
      '">Event risk: ' + maEsc(np.eventRisk || "Low") + "</span>");
    const c = np.counts || {};
    pills.push('<span class="ma-news-pill flat">' + (c.bull || 0) + " up / " + (c.bear || 0) +
      " down / " + (c.neutral || 0) + " flat</span>");
  } else {
    pills.push('<span class="ma-news-pill flat">Headline bias: n/a</span>');
  }
  const events = (np && np.events) || [];
  const evHTML = events.length
    ? '<div class="ma-events"><b>Watch:</b> ' + events.map(maEsc).join(" · ") + "</div>"
    : "";
  const filters = ["all", "India", "Global", "bull", "bear"];
  const fltHTML = '<div class="ma-news-flt">' + filters.map((f) =>
    '<button data-flt="' + f + '"' + (maNewsFilter === f ? ' class="on"' : "") + ">" +
    (f === "all" ? "All" : f[0].toUpperCase() + f.slice(1)) + "</button>").join("") +
    '<button data-more="1">More +20</button></div>';

  head.innerHTML = '<div class="ma-news-head">' + pills.join("") + "</div>" + evHTML + fltHTML;
  head.querySelectorAll("button[data-flt]").forEach((b) => {
    b.addEventListener("click", () => {
      maNewsFilter = b.dataset.flt;
      maNewsCount = 20;
      head.querySelectorAll("button[data-flt]").forEach((x) => x.classList.toggle("on", x === b));
      maRenderNewsItems();
    });
  });
  const more = head.querySelector("button[data-more]");
  if (more) more.addEventListener("click", () => {
    maNewsCount += 20;
    maRenderNewsItems();
  });

  maRenderNewsItems();
}

function maRenderOverall(ov) {
  const wrap = document.getElementById("maOverallCards");
  const body = document.getElementById("maOverallRows");
  const note = document.getElementById("maOverallNote");
  if (!wrap) return;
  if (!ov || !ov.ok) {
    wrap.innerHTML = '<div class="ma-empty">Overall prediction unavailable.</div>';
    body.innerHTML = "";
    if (note) note.textContent = "";
    return;
  }
  let cls = "flat";
  if (ov.direction === "bull") cls = "up";
  else if (ov.direction === "bear") cls = "down";
  const agreeBadge = ov.agreement === "aligned" ? " · signals aligned"
    : ov.agreement === "conflict" ? " · signals conflict"
    : "";
  wrap.innerHTML =
    '<div class="ma-verdict ' + cls + '">' +
    '<div class="ma-vkind">Combined next-session view</div>' +
    '<div class="ma-vlabel">' + maEsc(ov.label) + '</div>' +
    '<div class="ma-vmeta">Score ' + Number(ov.score || 0).toFixed(2) +
    ' · Confidence ' + Number(ov.confidence || 0) + "%" + agreeBadge + '</div>' +
    '<div class="ma-bar"><i style="width:' + Math.max(0, Math.min(100, Number(ov.confidence || 0))) + '%"></i></div>' +
    '</div>';
  body.innerHTML = (ov.components || []).map((c) => {
    const pts = Number(c.points || 0);
    const pcls = pts > 0 ? "ma-up" : pts < 0 ? "ma-down" : "ma-flat";
    const ptxt = Math.abs(pts) < 0.005 ? "–" : (pts >= 0 ? "+" : "") + pts.toFixed(2);
    return "<tr><td>" + maEsc(c.name) + "</td><td>" + maEsc(c.value) + '</td><td class="' +
      pcls + '">' + ptxt + "</td><td>" + maEsc(c.weight) + "</td></tr>";
  }).join("") || '<tr><td colspan="4" class="ma-empty">No components.</td></tr>';
  if (note) note.textContent = (ov.note || "") + " " + (ov.disclaimer || "");
}

let maGapCharts = {};

function maIsoDate(d) {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return y + "-" + m + "-" + day;
}

function maRenderGapStats(d) {
  const cards = document.getElementById("maGapCards");
  const buckBody = document.getElementById("maGapBuckets");
  const rowBody = document.getElementById("maGapRows");
  const note = document.getElementById("maGapNote");
  const sub = document.getElementById("maGapChartSub");
  if (!cards) return;
  if (!d || !d.ok) {
    cards.innerHTML = '<div class="ma-empty">' +
      maEsc((d && d.error) || "Gap statistics unavailable (connect Dhan for daily NIFTY candles).") +
      "</div>";
    buckBody.innerHTML = "";
    rowBody.innerHTML = "";
    if (note) note.textContent = "";
    return;
  }
  const flat = Math.max(0, (d.sessions || 0) - (d.gapUps || 0) - (d.gapDowns || 0));
  cards.innerHTML =
    maCard("Sessions", maNum(d.sessions), maEsc(d.from + " to " + d.to)) +
    maCard("Gap-ups", maNum(d.gapUps),
      '<span class="ma-ok">' + maNum(d.gapUpSuccess) + " held</span> · " +
      Number(d.gapUpRate || 0).toFixed(1) + "%") +
    maCard("Gap-ups failed", maNum(d.gapUpFail), "closed below the gap-open") +
    maCard("Gap-downs", maNum(d.gapDowns),
      '<span class="ma-ok">' + maNum(d.gapDownSuccess) + " held</span> · " +
      Number(d.gapDownRate || 0).toFixed(1) + "%") +
    maCard("Trend accuracy", Number(d.trendAccuracy || 0).toFixed(1) + "%",
      maNum(d.trendCorrect) + "/" + maNum(d.trendPredictions) + " correct") +
    maCard("Flat opens", maNum(flat), "open = previous close");

  const buckets = d.buckets || [];
  buckBody.innerHTML = buckets.map((b) => {
    const gcls = b.gapUps > 0 && b.gapUpRate >= 50 ? "ma-ok" : b.gapUps > 0 ? "ma-bad" : "ma-flat";
    const dcls = b.gapDowns > 0 && b.gapDownRate >= 50 ? "ma-ok" : b.gapDowns > 0 ? "ma-bad" : "ma-flat";
    const tcls = b.trendPredictions > 0 && b.trendAccuracy >= 50 ? "ma-ok" : b.trendPredictions > 0 ? "ma-bad" : "ma-flat";
    return "<tr><td>" + maEsc(b.label) + "</td><td>" + maNum(b.sessions) +
      "</td><td>" + maNum(b.gapUps) + "</td><td>" + maNum(b.gapUpSuccess) +
      '</td><td class="' + gcls + '">' + (b.gapUps ? Number(b.gapUpRate).toFixed(1) + "%" : "–") +
      "</td><td>" + maNum(b.gapDowns) + "</td><td>" + maNum(b.gapDownSuccess) +
      '</td><td class="' + dcls + '">' + (b.gapDowns ? Number(b.gapDownRate).toFixed(1) + "%" : "–") +
      '</td><td class="' + tcls + '">' +
      (b.trendPredictions ? Number(b.trendAccuracy).toFixed(1) + "%" : "–") + "</td></tr>";
  }).join("") || '<tr><td colspan="9" class="ma-empty">No sessions in this range.</td></tr>';

  const rows = (d.rows || []).slice(-40).reverse();
  rowBody.innerHTML = rows.map((r) => {
    const up = r.direction === "up";
    const dir = '<span class="' + (up ? "ma-up" : "ma-down") + '">' +
      (up ? "GAP UP" : "GAP DOWN") + "</span>";
    const res = '<span class="' + (r.success ? "ma-ok" : "ma-bad") + '">' +
      (r.success ? "held" : "failed") + "</span>";
    const corr = r.actual
      ? '<span class="' + (r.correct ? "ma-ok" : "ma-bad") + '">' + maEsc(r.actual) + "</span>"
      : '<span class="ma-flat">–</span>';
    return "<tr><td>" + maEsc(r.date) + "</td><td>" + dir +
      "</td><td>" + Number(r.gapPct).toFixed(2) + "%</td><td>" + maPrice(r.open) +
      "</td><td>" + maPrice(r.close) + "</td><td>" + res + "</td><td>" +
      maEsc(r.prediction || "–") + "</td><td>" + corr + "</td></tr>";
  }).join("") || '<tr><td colspan="8" class="ma-empty">No gap sessions in this range.</td></tr>';

  if (note) note.textContent = d.note || "";
  if (sub) sub.textContent = "· " + d.bucket + " buckets · " + buckets.length + " points";
  maDrawGapCharts(d);
}

function maDrawGapCharts(d) {
  if (!window.Chart) return;
  const buckets = d.buckets || [];
  const labels = buckets.map((b) => b.label);
  const baseOpts = {
    responsive: true,
    maintainAspectRatio: false,
    animation: false,
    plugins: { legend: { display: false } },
    scales: {
      x: {
        ticks: { color: "#8888b8", maxRotation: 0, autoSkip: true, maxTicksLimit: 14, font: { size: 9 } },
        grid: { color: "#1c1c38" },
      },
      y: {
        min: 0,
        max: 100,
        ticks: { color: "#8888b8", font: { size: 9 }, callback: (v) => v + "%" },
        grid: { color: "#1c1c38" },
      },
    },
  };

  const gapCanvas = document.getElementById("maGapChart");
  if (gapCanvas) {
    if (maGapCharts.gap) maGapCharts.gap.destroy();
    maGapCharts.gap = new window.Chart(gapCanvas.getContext("2d"), {
      type: "bar",
      data: {
        labels,
        datasets: [{
          data: buckets.map((b) => (b.gapUps > 0 ? b.gapUpRate : null)),
          backgroundColor: buckets.map((b) =>
            b.gapUps === 0 ? "#33335c" : b.gapUpRate >= 50 ? "#1f9d6b" : "#b34a4a"),
          borderRadius: 3,
        }],
      },
      options: Object.assign({}, baseOpts, {
        plugins: {
          legend: { display: false },
          tooltip: {
            callbacks: {
              label: (ctx) => {
                const b = buckets[ctx.dataIndex] || {};
                return b.gapUps
                  ? b.gapUpSuccess + "/" + b.gapUps + " gap-ups held (" + b.gapUpRate + "%)"
                  : "no gap-ups";
              },
            },
          },
        },
      }),
    });
  }

  const trendCanvas = document.getElementById("maTrendChart");
  if (trendCanvas) {
    if (maGapCharts.trend) maGapCharts.trend.destroy();
    maGapCharts.trend = new window.Chart(trendCanvas.getContext("2d"), {
      type: "line",
      data: {
        labels,
        datasets: [{
          data: buckets.map((b) => (b.trendPredictions > 0 ? b.trendAccuracy : null)),
          borderColor: "#b39ddb",
          backgroundColor: "rgba(179,157,219,.15)",
          fill: true,
          tension: 0.25,
          pointRadius: labels.length > 60 ? 0 : 2,
          spanGaps: true,
        }],
      },
      options: Object.assign({}, baseOpts, {
        plugins: {
          legend: { display: false },
          tooltip: {
            callbacks: {
              label: (ctx) => {
                const b = buckets[ctx.dataIndex] || {};
                return b.trendPredictions
                  ? b.trendCorrect + "/" + b.trendPredictions + " correct (" + b.trendAccuracy + "%)"
                  : "no predictions";
              },
            },
          },
        },
      }),
    });
  }
}

async function maLoadGapStats() {
  const fromEl = document.getElementById("maGapFrom");
  const toEl = document.getElementById("maGapTo");
  const bucketEl = document.getElementById("maGapBucket");
  const statusEl = document.getElementById("maGapStatus");
  if (!fromEl || !toEl || !bucketEl) return;
  const qs = new URLSearchParams({
    from: fromEl.value,
    to: toEl.value,
    bucket: bucketEl.value,
  });
  if (statusEl) statusEl.textContent = "Loading...";
  try {
    const resp = await fetch("/api/market/gap-stats?" + qs.toString(), { cache: "no-store" });
    const d = await resp.json();
    maRenderGapStats(d);
    if (statusEl) {
      statusEl.textContent = d && d.ok
        ? "Updated " + new Date().toLocaleTimeString("en-IN")
        : "Unavailable";
    }
  } catch (e) {
    if (statusEl) statusEl.textContent = "Failed: " + e;
  }
}

export function bootMarketAnalysis() {
  const pane = document.getElementById("tab-marketanalysis");
  if (!pane || pane.dataset.maBooted) return;
  pane.dataset.maBooted = "1";

  const style = document.createElement("style");
  style.textContent = MA_CSS;
  document.head.appendChild(style);

  pane.innerHTML = `
    <div class="ma-head">
      <button class="ma-btn" id="maRefresh">Refresh</button>
      <span class="ma-status" id="maStatus">Loading...</span>
    </div>
    <section class="ma-sec">
      <h3>NIFTY Next-Session Prediction <span class="ma-sub">GIFT + global + crude oil + flows</span></h3>
      <div class="ma-pred" id="maPredCards"></div>
      <div class="ma-scroll">
        <table class="account-table">
          <thead><tr><th>Group</th><th>Factor</th><th>Value</th><th>Points</th><th>Note</th></tr></thead>
          <tbody id="maPredFactors"></tbody>
        </table>
      </div>
      <div class="ma-disc" id="maPredDisc"></div>
    </section>
    <section class="ma-sec">
      <h3>Gap-Up Success + Prediction Accuracy <span class="ma-sub">NIFTY daily · backtest</span></h3>
      <div class="ma-gap-controls">
        <label>Group
          <select id="maGapBucket">
            <option value="day">Daily</option>
            <option value="week">Weekly</option>
            <option value="month">Monthly</option>
            <option value="year">Yearly</option>
          </select>
        </label>
        <label>From <input type="date" id="maGapFrom"></label>
        <label>To <input type="date" id="maGapTo"></label>
        <button class="ma-btn" id="maGapApply">Apply</button>
        <span class="ma-gap-sep">or</span>
        <label>Month <input type="month" id="maGapMonth"></label>
        <button class="ma-btn" id="maGapMonthBtn">Use month</button>
        <label>Year <input type="number" id="maGapYear" min="2000" max="2100" step="1"></label>
        <button class="ma-btn" id="maGapYearBtn">Use year</button>
        <span class="ma-status" id="maGapStatus"></span>
      </div>
      <div class="ma-cards" id="maGapCards"></div>
      <div class="ma-gap-charts">
        <div class="ma-chart-box">
          <div class="ma-chart-title">Gap-up success rate <span id="maGapChartSub"></span></div>
          <canvas id="maGapChart"></canvas>
        </div>
        <div class="ma-chart-box">
          <div class="ma-chart-title">Trend prediction accuracy <span>· price-action rule</span></div>
          <canvas id="maTrendChart"></canvas>
        </div>
      </div>
      <div class="ma-scroll ma-gap-table">
        <table class="account-table">
          <thead><tr><th>Period</th><th>Sessions</th><th>Gap-ups</th><th>Held</th><th>Gap-up %</th>
            <th>Gap-downs</th><th>Held</th><th>Gap-down %</th><th>Trend acc.</th></tr></thead>
          <tbody id="maGapBuckets"></tbody>
        </table>
      </div>
      <div class="ma-scroll ma-gap-table">
        <table class="account-table">
          <thead><tr><th>Date</th><th>Gap</th><th>Gap %</th><th>Open</th><th>Close</th>
            <th>Result</th><th>Prediction</th><th>Actual</th></tr></thead>
          <tbody id="maGapRows"></tbody>
        </table>
      </div>
      <div class="ma-disc" id="maGapNote"></div>
    </section>
    <section class="ma-sec">
      <h3>FII / DII Cash Market <span class="ma-sub" id="maFiidiiDate"></span></h3>
      <div class="ma-cards" id="maFiidiiCards"></div>
    </section>
    <section class="ma-sec">
      <h3>FII / DII F&amp;O Positions (Participant OI) <span class="ma-sub" id="maPartDate"></span></h3>
      <div class="ma-scroll">
        <table class="account-table">
          <thead id="maPartHead"></thead>
          <tbody id="maPartBody"></tbody>
        </table>
      </div>
    </section>
    <section class="ma-sec">
      <h3>Global Indices</h3>
      <div class="ma-cards" id="maGlobalCards"></div>
    </section>
    <section class="ma-sec">
      <h3>India Indices / Volatility</h3>
      <div class="ma-cards" id="maIndiaCards"></div>
    </section>
    <section class="ma-sec">
      <h3>News + Event Risk</h3>
      <div id="maNewsHead"></div>
      <div class="ma-news-list" id="maNewsList"></div>
      <div class="ma-disc" id="maNewsDisc">Headline sentiment is keyword-based and indicative only - treat as context, not advice.</div>
    </section>
    <section class="ma-sec">
      <h3>Overall Combined Prediction <span class="ma-sub">price/flow + news + crude</span></h3>
      <div class="ma-pred" id="maOverallCards" style="grid-template-columns:1fr"></div>
      <div class="ma-scroll">
        <table class="account-table">
          <thead><tr><th>Component</th><th>Value</th><th>Points</th><th>Weight</th></tr></thead>
          <tbody id="maOverallRows"></tbody>
        </table>
      </div>
      <div class="ma-disc" id="maOverallNote"></div>
    </section>`;

  const statusEl = document.getElementById("maStatus");
  const refreshBtn = document.getElementById("maRefresh");

  let timer = null;
  let loading = false;

  async function load() {
    if (loading) return;
    loading = true;
    try {
      const resp = await fetch("/api/market/analysis", { cache: "no-store" });
      const d = await resp.json();
      maRenderPrediction(d.prediction);
      maRenderFiidii(d.fiidii);
      maRenderParticipant(d.participant);
      maRenderGlobal(d.global);
      maRenderIndia(d.india);
      maRenderNews(d.news, d.newsPrediction);
      maRenderOverall(d.overall);
      statusEl.textContent = "Updated " + new Date().toLocaleTimeString("en-IN");
    } catch (e) {
      statusEl.textContent = "Load failed: " + e;
    } finally {
      loading = false;
    }
  }

  function start() {
    if (timer) return;
    load();
    timer = setInterval(load, 60000);
  }
  function stop() {
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
  }

  refreshBtn.addEventListener("click", load);

  // Gap-up statistics controls: calendar range, month/year shortcuts and the
  // graph grouping (daily / weekly / monthly / yearly).
  const gapFrom = document.getElementById("maGapFrom");
  const gapTo = document.getElementById("maGapTo");
  const gapMonth = document.getElementById("maGapMonth");
  const gapYear = document.getElementById("maGapYear");
  const gapApply = document.getElementById("maGapApply");
  const gapMonthBtn = document.getElementById("maGapMonthBtn");
  const gapYearBtn = document.getElementById("maGapYearBtn");
  const today = new Date();
  const defFrom = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 90);
  if (gapFrom) gapFrom.value = maIsoDate(defFrom);
  if (gapTo) gapTo.value = maIsoDate(today);
  if (gapMonth) gapMonth.value = maIsoDate(today).slice(0, 7);
  if (gapYear) gapYear.value = today.getFullYear();
  if (gapApply) gapApply.addEventListener("click", maLoadGapStats);
  if (gapMonthBtn) gapMonthBtn.addEventListener("click", () => {
    const v = String(gapMonth && gapMonth.value || "");
    if (!/^\d{4}-\d{2}$/.test(v)) return;
    const y = Number(v.slice(0, 4));
    const m = Number(v.slice(5, 7));
    const last = new Date(y, m, 0).getDate();
    gapFrom.value = v + "-01";
    gapTo.value = v + "-" + String(last).padStart(2, "0");
    maLoadGapStats();
  });
  if (gapYearBtn) gapYearBtn.addEventListener("click", () => {
    const y = Number(gapYear && gapYear.value || 0);
    if (!(y >= 2000 && y <= 2100)) return;
    gapFrom.value = y + "-01-01";
    gapTo.value = y + "-12-31";
    maLoadGapStats();
  });

  document.addEventListener("tabshown", (e) => {
    if (e.detail === "marketanalysis") {
      start();
      maLoadGapStats();
    } else stop();
  });
}
