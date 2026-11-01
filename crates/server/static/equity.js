// Equity Research tab - free-source multibagger screener across NSE + BSE.
// Universe: Yahoo screener (NSI + BSE). Enrichment: Yahoo chart/fundamentals,
// NSE bhavcopy delivery, NSE shareholding, Dhan F&O option metrics.

const STORE = {
  status: null,
  rows: [],
  backtest: null,
  asOf: "",
  note: "",
  q: "",
  minScore: 0,
  fnoOnly: false,
  strategy: "composite",
  capBand: "all",
  sort: "total",
  limit: 60,
  selected: null,
  wasBusy: false,
  timer: null,
  statusFilter: "all",
  minConf: 0,
  minUpside: 0,
  accuracy: null,
  accView: "replay",
  engines: null,
  enginesBusy: false,
};

const STRATS = [
  ["composite", "Composite"],
  ["coffee", "Coffee Can"],
  ["garp", "GARP"],
  ["microcap", "Microcap"],
  ["turnaround", "Turnaround"],
  ["value", "Deep Value"],
  ["momentum", "Momentum"],
  ["accumulation", "Accumulation"],
  ["quality", "Quality"],
];

let root = null;
let cssDone = false;

export function bootEquityResearch() {
  root = document.getElementById("rtEquityResearch");
  if (!root) return;
  injectCss();
  root.innerHTML = shell();
  bind();
  paintScanAll();
  loadCache();
  loadEngines();
  setTimeout(loadStatus, 600);
  setInterval(() => {
    if (document.body.getAttribute("data-tab") === "equityresearch") loadEngines();
  }, 10000);
  STORE.timer = setInterval(() => {
    if (document.body.getAttribute("data-tab") !== "equityresearch") return;
    loadStatus();
  }, 5000);
  document.addEventListener("tabshown", (e) => {
    if (e.detail === "equityresearch") loadStatus();
  });
}

function injectCss() {
  if (cssDone) return;
  cssDone = true;
  const s = document.createElement("style");
  s.textContent = `
#rtEquityResearch{--er-line:#233046;--er-soft:#131c2b;--er-dim:#8ea0bb;--er-text:#e7eefb;
  --er-up:#2fd08a;--er-down:#ff6272;--er-accent:#4aa8ff;--er-warn:#ffb547;
  padding:12px 14px 22px;color:var(--er-text);font-size:13px}
#tab-equityresearch.active{display:block;height:100%;overflow-y:auto;overflow-x:hidden}
body.light #rtEquityResearch{--er-line:#d6dfea;--er-soft:#f2f6fb;--er-dim:#5c6b80;--er-text:#17202e;
  --er-up:#128a56;--er-down:#c02a3a;--er-accent:#1668c2;--er-warn:#a86500}
#rtEquityResearch .er-head{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin-bottom:10px}
#rtEquityResearch .er-title{font-size:17px;font-weight:700;letter-spacing:.2px}
#rtEquityResearch .er-sub{color:var(--er-dim);font-size:12px}
#rtEquityResearch .er-spacer{flex:1}
#rtEquityResearch button{background:var(--er-soft);color:var(--er-text);border:1px solid var(--er-line);
  border-radius:6px;padding:6px 12px;font-size:12px;cursor:pointer}
#rtEquityResearch button:hover{border-color:var(--er-accent)}
#rtEquityResearch button:disabled{opacity:.45;cursor:default}
#rtEquityResearch .er-progress{height:4px;background:var(--er-soft);border-radius:3px;overflow:hidden;margin:4px 0 10px}
#rtEquityResearch .er-progress > i{display:block;height:100%;background:var(--er-accent);width:0;transition:width .3s}
#rtEquityResearch .er-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(112px,1fr));gap:6px;margin-bottom:10px}
#rtEquityResearch .er-card{background:var(--er-soft);border:1px solid var(--er-line);border-radius:8px;padding:7px 10px;min-width:0}
#rtEquityResearch .er-card b{display:block;font-size:16px;line-height:1.25;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
#rtEquityResearch .er-card span{color:var(--er-dim);font-size:11px}
#rtEquityResearch .er-tools{display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:10px}
#rtEquityResearch input[type=search],#rtEquityResearch select{background:var(--er-soft);color:var(--er-text);
  border:1px solid var(--er-line);border-radius:6px;padding:6px 9px;font-size:12px;min-width:120px}
#rtEquityResearch input[type=search]{min-width:190px}
#rtEquityResearch label.er-chk{display:flex;gap:5px;align-items:center;color:var(--er-dim);font-size:12px}
#rtEquityResearch .er-scroll{overflow:auto;border:1px solid var(--er-line);border-radius:8px;max-height:calc(100vh - 590px);min-height:200px}
#rtEquityResearch .er-panels{display:grid;grid-template-columns:1.4fr 1fr;gap:10px;align-items:start;margin-bottom:10px}
#rtEquityResearch .er-panels > *{min-width:0;margin-bottom:0}
@media (max-width:1100px){#rtEquityResearch .er-panels{grid-template-columns:1fr}}
#rtEquityResearch table{border-collapse:collapse;width:100%;font-size:12px;white-space:nowrap}
#rtEquityResearch th{position:sticky;top:0;background:var(--er-soft);color:var(--er-dim);text-align:right;
  padding:7px 8px;border-bottom:1px solid var(--er-line);cursor:pointer;font-weight:600;z-index:2}
#rtEquityResearch th:nth-child(-n+5),#rtEquityResearch td:nth-child(-n+5){text-align:left}
#rtEquityResearch td{padding:6px 8px;border-bottom:1px solid var(--er-line);text-align:right}
#rtEquityResearch tbody tr{cursor:pointer}
#rtEquityResearch tbody tr:hover{background:var(--er-soft)}
#rtEquityResearch .er-sym{font-weight:700;color:var(--er-accent)}
#rtEquityResearch .er-ex{display:inline-block;padding:1px 5px;border-radius:4px;font-size:9px;
  border:1px solid var(--er-line);color:var(--er-dim);margin-left:4px;vertical-align:1px}
#rtEquityResearch .er-chip{display:inline-block;padding:1px 6px;border-radius:10px;font-size:10px;
  border:1px solid var(--er-line);color:var(--er-dim);margin-right:4px}
#rtEquityResearch .er-tag{display:inline-block;padding:1px 7px;border-radius:10px;font-size:10px;
  border:1px solid var(--er-accent);color:var(--er-accent);margin-right:4px}
#rtEquityResearch .er-score{font-weight:700}
#rtEquityResearch .up{color:var(--er-up)}#rtEquityResearch .down{color:var(--er-down)}
#rtEquityResearch .er-more{display:block;margin:8px auto 0}
#rtEquityResearch .er-drawer{position:fixed;top:56px;right:0;bottom:0;width:min(600px,96vw);z-index:120;
  background:var(--er-soft);border-left:1px solid var(--er-line);box-shadow:-12px 0 34px rgba(0,0,0,.35);
  padding:14px 16px 40px;overflow:auto;transform:translateX(102%);transition:transform .22s ease}
#rtEquityResearch .er-drawer.open{transform:none}
#rtEquityResearch .er-drawer h3{margin:0 0 2px;font-size:16px}
#rtEquityResearch .er-drawer .er-close{position:absolute;top:10px;right:12px;padding:3px 9px}
#rtEquityResearch .er-sec{margin-top:14px;border-top:1px solid var(--er-line);padding-top:10px}
#rtEquityResearch .er-sec h4{margin:0 0 8px;font-size:12px;text-transform:uppercase;letter-spacing:.7px;color:var(--er-dim)}
#rtEquityResearch .er-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(130px,1fr));gap:6px 14px}
#rtEquityResearch .er-kv{display:flex;justify-content:space-between;gap:8px;padding:3px 0;border-bottom:1px dashed var(--er-line)}
#rtEquityResearch .er-kv span:first-child{color:var(--er-dim)}
#rtEquityResearch .er-bar{height:6px;border-radius:3px;background:var(--er-line);overflow:hidden;margin-top:3px}
#rtEquityResearch .er-bar > i{display:block;height:100%}
#rtEquityResearch .er-upg{display:inline-block;margin-left:4px;padding:1px 5px;border-radius:8px;font-size:9px;font-weight:700;border:1px solid var(--er-up);color:var(--er-up);vertical-align:1px}
#rtEquityResearch .er-flag{display:inline-block;padding:2px 7px;border-radius:10px;font-size:11px;
  border:1px solid var(--er-warn);color:var(--er-warn);margin:2px 4px 0 0}
#rtEquityResearch .er-list{margin:4px 0 0;padding-left:18px;line-height:1.6}
#rtEquityResearch .er-muted{color:var(--er-dim);font-size:11px;line-height:1.5}
#rtEquityResearch .er-bt{background:var(--er-soft);border:1px solid var(--er-line);border-radius:8px;padding:8px 10px}
#rtEquityResearch .er-bt-head{display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;align-items:baseline}
#rtEquityResearch .er-bt-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(90px,1fr));gap:8px;margin-top:8px}
#rtEquityResearch .er-bt-grid b{display:block;font-size:14px}
#rtEquityResearch .er-bt-grid span{color:var(--er-dim);font-size:10px}
#rtEquityResearch .er-mbars{display:flex;gap:2px;align-items:flex-end;height:34px;margin-top:9px}
#rtEquityResearch .er-mbars i{flex:1;min-width:2px;border-radius:1px}
#rtEquityResearch .er-empty{padding:24px;text-align:center;color:var(--er-dim)}
#rtEquityResearch .er-status{display:inline-block;padding:1px 7px;border-radius:10px;font-size:10px;font-weight:700;border:1px solid}
#rtEquityResearch .st-strong{border-color:var(--er-up);color:var(--er-up);background:rgba(47,208,138,.1)}
#rtEquityResearch .st-buy{border-color:#39c47e;color:#39c47e;background:rgba(57,196,126,.08)}
#rtEquityResearch .st-acc{border-color:var(--er-accent);color:var(--er-accent)}
#rtEquityResearch .st-watch{border-color:var(--er-warn);color:var(--er-warn)}
#rtEquityResearch .st-avoid{border-color:var(--er-down);color:var(--er-down)}
#rtEquityResearch .er-acc{background:var(--er-soft);border:1px solid var(--er-line);border-radius:8px;padding:8px 10px}
#rtEquityResearch .er-acc-head{display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;align-items:center}
#rtEquityResearch .er-acc-head b{font-size:13px}
#rtEquityResearch .er-acc-tabs{display:flex;gap:6px}
#rtEquityResearch .er-acc-tabs button{padding:3px 9px;font-size:11px}
#rtEquityResearch .er-acc-tabs button.on{border-color:var(--er-accent);color:var(--er-accent)}
#rtEquityResearch .er-acc-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(78px,1fr));gap:6px;margin-top:7px}
#rtEquityResearch .er-acc-grid b{display:block;font-size:14px}
#rtEquityResearch .er-acc-grid span{color:var(--er-dim);font-size:10px}
#rtEquityResearch .er-chart{display:flex;gap:3px;align-items:flex-end;height:48px;margin-top:8px}
#rtEquityResearch .er-chart i{flex:1;min-width:4px;border-radius:2px 2px 0 0;position:relative;background:var(--er-accent)}
#rtEquityResearch .er-chart i span{position:absolute;bottom:100%;left:50%;transform:translateX(-50%);font-size:9px;color:var(--er-dim);white-space:nowrap}
#rtEquityResearch .er-cal{display:grid;grid-template-columns:repeat(auto-fit,minmax(120px,1fr));gap:6px 14px;margin-top:8px}
#rtEquityResearch .er-cal .er-kv{border-bottom:1px dashed var(--er-line);display:block}
#rtEquityResearch .er-log{width:100%;border-collapse:collapse;font-size:11px;margin-top:8px}
#rtEquityResearch .er-log th{position:static;background:transparent;border-bottom:1px solid var(--er-line);padding:4px 6px}
#rtEquityResearch .er-log td{padding:4px 6px;border-bottom:1px solid var(--er-line)}
#rtEquityResearch .er-log tr{cursor:default}
#rtEquityResearch .er-push{border:1px solid var(--er-line);background:var(--er-soft);border-radius:8px;
  padding:8px 10px;margin:0 0 10px;transition:border-color .3s ease,background .3s ease,box-shadow .3s ease}
#rtEquityResearch .er-push.on{border-color:var(--er-up);background:rgba(47,208,138,.07);box-shadow:0 0 0 1px rgba(47,208,138,.18) inset}
#rtEquityResearch .er-push.busy{border-color:var(--er-warn)}
#rtEquityResearch .er-push-row{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
#rtEquityResearch .er-push-info{flex:1;min-width:200px}
#rtEquityResearch .er-push-info b{display:block;font-size:12px}
#rtEquityResearch .er-push-info span{display:block;color:var(--er-dim);font-size:10px;margin-top:2px;line-height:1.45}
#rtEquityResearch .er-push-meta{display:flex;gap:10px;flex-wrap:wrap}
#rtEquityResearch .er-push-meta i{font-style:normal;display:block;font-size:12px;font-weight:700}
#rtEquityResearch .er-push-meta em{font-style:normal;display:block;color:var(--er-dim);font-size:9px;text-transform:uppercase;letter-spacing:.3px}
#rtEquityResearch .er-push-note{font-size:10px;color:var(--er-dim);margin-top:6px;line-height:1.5;
  max-height:0;opacity:0;overflow:hidden;transition:max-height .35s ease,opacity .35s ease,margin-top .35s ease}
#rtEquityResearch .er-push.on .er-push-note{max-height:120px;opacity:1}
#rtEquityResearch .er-push-badge{font-size:10px;font-weight:700;padding:1px 8px;border-radius:10px;border:1px solid var(--er-line);color:var(--er-dim)}
#rtEquityResearch .er-push.on .er-push-badge{border-color:var(--er-up);color:var(--er-up)}
#rtEquityResearch .er-push.fade-in{animation:erPushIn .45s ease both}
@keyframes erPushIn{from{opacity:0;transform:translateY(-6px)}to{opacity:1;transform:none}}
#rtEquityResearch .er-engines{display:grid;grid-template-columns:1fr 1fr;gap:10px;margin:0 0 10px}
@media (max-width:900px){#rtEquityResearch .er-engines{grid-template-columns:1fr}}
#rtEquityResearch .er-eng{border:1px solid var(--er-line);background:var(--er-soft);border-radius:8px;
  padding:8px 10px;display:flex;align-items:center;gap:10px;flex-wrap:wrap}
#rtEquityResearch .er-eng-info{flex:1;min-width:150px}
#rtEquityResearch .er-eng-info b{display:block;font-size:12px}
#rtEquityResearch .er-eng-info span{display:block;color:var(--er-dim);font-size:10px;margin-top:2px}
#rtEquityResearch .er-eng-meta{display:flex;gap:10px;flex-wrap:wrap}
#rtEquityResearch .er-eng-meta i{font-style:normal;display:block;font-size:12px;font-weight:700}
#rtEquityResearch .er-eng-meta em{font-style:normal;display:block;color:var(--er-dim);font-size:9px;text-transform:uppercase;letter-spacing:.3px}
#rtEquityResearch .er-switch{position:relative;display:inline-block;width:40px;height:21px;flex:0 0 auto}
#rtEquityResearch .er-switch input{opacity:0;width:0;height:0}
#rtEquityResearch .er-slider{position:absolute;cursor:pointer;inset:0;background:#2a3145;border:1px solid var(--er-line);
  border-radius:12px;transition:.18s}
#rtEquityResearch .er-slider::before{content:"";position:absolute;width:15px;height:15px;left:2px;top:2px;
  background:#9fb0cc;border-radius:50%;transition:.18s}
#rtEquityResearch .er-switch input:checked + .er-slider{background:rgba(47,208,138,.28);border-color:var(--er-up)}
#rtEquityResearch .er-switch input:checked + .er-slider::before{transform:translateX(19px);background:var(--er-up)}
#rtEquityResearch .er-switch input:disabled + .er-slider{opacity:.4;cursor:not-allowed}
#rtEquityResearch .er-eng-msg{color:var(--er-warn);font-size:10px;flex:0 0 100%}
`;
  document.head.appendChild(s);
}

function shell() {
  const opts = STRATS.map(([v, l]) => `<option value="${v}">${l}</option>`).join("");
  return `
  <div class="er-head">
    <div>
      <div class="er-title">Multibagger Research</div>
      <div class="er-sub" id="erSub">NSE + BSE active equities. Free sources: Yahoo universe/fundamentals, NSE bhavcopy + delivery, NSE shareholding, Dhan F&amp;O options.</div>
    </div>
    <div class="er-spacer"></div>
    <span class="er-sub" id="erStage"></span>
    <button id="erRefresh">Refresh now</button>
    <label class="er-chk" id="erAutoLbl" title="Auto scan: ON karne par Equity Research diye gaye minutes ke interval par khud re-scan karega, taaki naye listed / naye liquid stocks automatically aa jaayein aur Equity Paper + Realtime Equity engine ki list me fetch ho jaayein. OFF = sirf manual Refresh now, ya purana 6-hour TTL."><input type="checkbox" id="erAuto"> Auto scan</label>
    <label class="er-chk" title="Scan interval in hours + minutes. Auto scan ON hone par itne samay me ek naya scan chalega (minimum 5 minutes, maximum 24 hours).">every <input type="number" id="erAutoHr" min="0" max="24" step="1" value="1" style="width:52px"> h <input type="number" id="erAutoMin" min="0" max="59" step="1" value="0" style="width:52px"> m</label>
  </div>
  <div class="er-progress"><i id="erBar"></i></div>
  <div class="er-cards" id="erCards"></div>
  <div class="er-push" id="erPush">
    <div class="er-push-row">
      <div class="er-push-info">
        <b>Auto send scanned stocks to trading engines</b>
        <span>ON: har naye scan me jo naye stocks aayenge wo turant Equity Paper + Realtime Equity dono tabs me send ho jayenge, aur slot + margin available hote hi buy ho jayenge.</span>
      </div>
      <div class="er-push-meta">
        <div><i id="erPushState">OFF</i><em>Status</em></div>
        <div><i id="erPushSent">0</i><em>Known</em></div>
        <div><i id="erPushPending">0</i><em>Waiting buy</em></div>
      </div>
      <span class="er-push-badge" id="erPushBadge">INACTIVE</span>
      <label class="er-switch" id="erPushLbl" title="Auto send: naye scanned stocks ko dono equity engines me bhejta hai. Engine unhe sirf slot + shared margin check par buy karta hai.">
        <input type="checkbox" id="erPushChk">
        <span class="er-slider"></span>
      </label>
    </div>
    <div class="er-push-note" id="erPushNote"></div>
  </div>
  <div class="er-engines" id="erEngines"></div>
  <div class="er-panels">
    <div id="erAcc"></div>
    <div id="erBt"></div>
  </div>
  <div class="er-tools">
    <input type="search" id="erQ" placeholder="Search symbol / name / sector">
    <select id="erStrat">${opts}</select>
    <select id="erCap">
      <option value="all">All market caps</option>
      <option value="micro">Micro (below 500 cr)</option>
      <option value="small">Small (500 - 5,000 cr)</option>
      <option value="mid">Mid (5,000 - 50,000 cr)</option>
      <option value="large">Large (above 50,000 cr)</option>
    </select>
    <select id="erMin">
      <option value="0">All scores</option>
      <option value="50">Score 50+</option>
      <option value="60">Score 60+</option>
      <option value="70">Score 70+</option>
    </select>
    <select id="erStatus">
      <option value="all">All calls</option>
      <option value="Strong Buy">Strong Buy</option>
      <option value="Buy">Buy</option>
      <option value="Accumulate">Accumulate</option>
      <option value="Watch">Watch</option>
      <option value="Avoid">Avoid</option>
    </select>
    <select id="erMinConf">
      <option value="0">Any confidence</option>
      <option value="55">Confidence 55+</option>
      <option value="65">Confidence 65+</option>
      <option value="75">Confidence 75+</option>
    </select>
    <select id="erMinUp">
      <option value="0">Any upside</option>
      <option value="10">Upside 10%+</option>
      <option value="20">Upside 20%+</option>
      <option value="30">Upside 30%+</option>
    </select>
    <select id="erSort">
      <option value="total">Sort: composite</option>
      <option value="upside">Sort: upside target</option>
      <option value="buyConf">Sort: buy confidence</option>
      <option value="eta">Sort: fastest target</option>
      <option value="strategy">Sort: selected playbook</option>
      <option value="fundamental">Sort: fundamentals</option>
      <option value="technical">Sort: technicals</option>
      <option value="base">Sort: base quality</option>
      <option value="valuation">Sort: valuation</option>
      <option value="quality">Sort: quality</option>
      <option value="marketCap">Sort: market cap</option>
      <option value="turnoverLacs">Sort: turnover</option>
    </select>
    <label class="er-chk"><input type="checkbox" id="erFno"> F&amp;O only</label>
    <label class="er-chk" id="erAllLbl" title="ON: liquidity screen hata kar har listed NSE + BSE equity scan hoti hai (price history + research). Slow: hazaaron chart calls, 30-90 min lag sakte hain. OFF: default cap-balanced 900-stock sample. Active/inactive turant dikhta hai."><input type="checkbox" id="erAll"> Scan all listed stocks</label>
    <span class="er-sub" id="erAllStat"></span>
    <span class="er-sub" id="erCount"></span>
  </div>
  <div class="er-scroll"><table>
    <thead><tr>
      <th>#</th><th>Symbol</th><th>Exch</th><th>Name</th><th>Sector</th><th>Call</th><th>Price</th>
      <th>Upside</th><th>Conf</th><th>Target</th><th>ETA</th><th>P 3/6/12m</th><th>RR</th><th>Chg%</th><th>Total</th><th>Playbook</th><th>Fund</th><th>Tech</th><th>Base</th><th>Val</th><th>Coverage</th><th>Tags</th>
    </tr></thead>
    <tbody id="erRows"><tr><td colspan="22" class="er-empty">Loading the screen...</td></tr></tbody>
  </table></div>
  <button class="er-more" id="erMore" style="display:none">Load more</button>
  <div class="er-drawer" id="erDrawer"></div>`;
}

function bind() {
  const $ = (id) => document.getElementById(id);
  $("erRefresh").addEventListener("click", refresh);
  $("erQ").addEventListener("input", (e) => {
    STORE.q = e.target.value.trim().toLowerCase();
    STORE.limit = 60;
    renderRows();
  });
  $("erAll").addEventListener("change", (e) => {
    const cb = e.target;
    const on = !!cb.checked;
    cb.disabled = true;
    api("/api/equity/filters", { method: "POST", body: JSON.stringify({ scanAll: on }) })
      .then((r) => {
        const okNow = !!(r && r.ok && r.filters);
        const saved = okNow
          ? (typeof r.filters.scan_all !== "undefined" ? r.filters.scan_all : r.filters.scanAll)
          : on;
        cb.checked = !!saved;
        if (okNow && r.rescan) {
          STORE.wasBusy = true;
          const st = document.getElementById("erStage");
          if (st) st.textContent = "starting full-market scan...";
        }
      })
      .catch(() => {
        cb.checked = !on;
      })
      .finally(() => {
        cb.disabled = false;
        paintScanAll();
      });
  });
  // Auto scan: persist the switch + interval and let the server refresher do the
  // cadence. A new scan brings newly listed / newly liquid names into the
  // research rows, which the equity books read from the same snapshot.
  const saveAutoScan = () => {
    const cb = $("erAuto");
    const hrEl = $("erAutoHr");
    const minEl = $("erAutoMin");
    if (!cb) return;
    // Operator enters HOURS + MINUTES; the server stores a single minute count.
    let hrs = Math.max(0, Math.min(24, Math.floor(Number(hrEl && hrEl.value) || 0)));
    let mins = Math.max(0, Math.min(59, Math.floor(Number(minEl && minEl.value) || 0)));
    let total = hrs * 60 + mins;
    if (total < 5) {
      total = 5;
      hrs = 0;
      mins = 5;
    }
    if (total > 1440) {
      total = 1440;
      hrs = 24;
      mins = 0;
    }
    if (hrEl && String(hrEl.value) !== String(hrs)) hrEl.value = String(hrs);
    if (minEl && String(minEl.value) !== String(mins)) minEl.value = String(mins);
    cb.disabled = true;
    if (hrEl) hrEl.disabled = true;
    if (minEl) minEl.disabled = true;
    api("/api/equity/filters", {
      method: "POST",
      body: JSON.stringify({ autoScan: !!cb.checked, autoScanMinutes: total }),
    })
      .then((r) => {
        const f = (r && r.filters) || null;
        if (f) paintAutoScan(f);
        if (r && r.rescan) {
          STORE.wasBusy = true;
          const st = document.getElementById("erStage");
          if (st) st.textContent = "starting scan...";
        }
      })
      .catch(() => {})
      .finally(() => {
        cb.disabled = false;
        if (hrEl) hrEl.disabled = false;
        if (minEl) minEl.disabled = false;
      });
  };
  if ($("erPushChk")) $("erPushChk").addEventListener("change", saveAutoPush);
  if ($("erPushBadge")) {
    // Clicking the badge toggles the same switch, so the ACTIVE / INACTIVE label
    // is a real control too.
    $("erPushBadge").addEventListener("click", () => {
      const chk = $("erPushChk");
      if (!chk) return;
      chk.checked = !chk.checked;
      saveAutoPush();
    });
  }
  if ($("erAuto")) $("erAuto").addEventListener("change", saveAutoScan);
  if ($("erAutoHr")) {
    $("erAutoHr").addEventListener("change", saveAutoScan);
    $("erAutoHr").addEventListener("blur", saveAutoScan);
  }
  if ($("erAutoMin")) {
    $("erAutoMin").addEventListener("change", saveAutoScan);
    $("erAutoMin").addEventListener("blur", saveAutoScan);
  }
  $("erMin").addEventListener("change", (e) => {
    STORE.minScore = Number(e.target.value) || 0;
    STORE.limit = 60;
    renderRows();
  });
  $("erStrat").addEventListener("change", (e) => {
    STORE.strategy = e.target.value;
    STORE.limit = 60;
    const sortEl = document.getElementById("erSort");
    if (sortEl) {
      if (STORE.strategy !== "composite" && sortEl.value === "total") sortEl.value = "strategy";
      if (STORE.strategy === "composite" && sortEl.value === "strategy") sortEl.value = "total";
      STORE.sort = sortEl.value;
    }
    renderRows();
  });
  $("erStatus").addEventListener("change", (e) => {
    STORE.statusFilter = e.target.value;
    STORE.limit = 60;
    renderRows();
  });
  $("erMinConf").addEventListener("change", (e) => {
    STORE.minConf = Number(e.target.value) || 0;
    STORE.limit = 60;
    renderRows();
  });
  $("erMinUp").addEventListener("change", (e) => {
    STORE.minUpside = Number(e.target.value) || 0;
    STORE.limit = 60;
    renderRows();
  });
  $("erCap").addEventListener("change", (e) => {
    STORE.capBand = e.target.value;
    STORE.limit = 60;
    renderRows();
  });
  $("erSort").addEventListener("change", (e) => {
    STORE.sort = e.target.value;
    renderRows();
  });
  $("erFno").addEventListener("change", (e) => {
    STORE.fnoOnly = e.target.checked;
    STORE.limit = 60;
    renderRows();
  });
  $("erMore").addEventListener("click", () => {
    STORE.limit += 60;
    renderRows();
  });
  $("erRows").addEventListener("click", (e) => {
    const tr = e.target.closest("tr[data-sym]");
    if (!tr) return;
    const sym = tr.getAttribute("data-sym");
    const ex = tr.getAttribute("data-ex");
    STORE.selected = STORE.rows.find((r) => r.symbol === sym && (!ex || r.exchange === ex)) || null;
    renderDrawer();
  });
  $("erAcc").addEventListener("click", (e) => {
    const v = e.target && e.target.getAttribute && e.target.getAttribute("data-acc");
    if (!v || v === STORE.accView) return;
    STORE.accView = v;
    renderAccuracy();
  });
  $("erEngines").addEventListener("change", (e) => {
    const inp = e.target.closest("input[data-mode]");
    if (inp) toggleEngine(inp.getAttribute("data-mode"), inp.checked);
  });
  $("erDrawer").addEventListener("click", (e) => {
    if (e.target.id === "erClose" || e.target.classList.contains("er-drawer")) {
      $("erDrawer").classList.remove("open");
    }
  });
}

async function loadEngines() {
  try {
    const d = await api("/api/equity/engines");
    STORE.engines = d;
    renderEngines(d);
    paintAutoPush(STORE.status && STORE.status.filters, d);
  } catch (_) {}
}

function renderEngines(d) {
  const el = document.getElementById("erEngines");
  if (!el) return;
  const broker = !!(d && d.brokerConnected);
  const paper = (d && d.paper) || {};
  const live = (d && d.live) || {};
  el.innerHTML = engineBox("paper", "Equity Paper Engine", "Simulated delivery fills from Buy calls", paper, broker, false)
    + engineBox("live", "Realtime Equity Engine", "LIVE Dhan CNC orders from Buy calls", live, broker, true);
}

function engineBox(mode, title, sub, eng, broker, live) {
  const st = eng.stats || {};
  const on = !!eng.enabled;
  const disabled = !on && live && !broker;
  const realized = Number(st.realizedPnl) || 0;
  const unrealized = Number(st.unrealizedPnl) || 0;
  const total = realized + unrealized;
  const tone = total > 0 ? "up" : total < 0 ? "down" : "";
  const msg = live && !broker ? "Dhan connect karo - LIVE switch locked" : "";
  return `<div class="er-eng">
    <div class="er-eng-info"><b>${title}</b><span>${sub}</span></div>
    <div class="er-eng-meta">
      <div><i>${on ? "ON" : "OFF"}</i><em>Engine</em></div>
      <div><i>${st.open || 0}</i><em>Open</em></div>
      <div><i>${st.closed || 0}</i><em>Closed</em></div>
      <div title="Realized + unrealized (delivery charges ke bina)"><i class="${tone}">${fmtMoney(total)}</i><em>Total P&amp;L</em></div>
      <div><i class="${realized > 0 ? "up" : realized < 0 ? "down" : ""}">${fmtMoney(realized)}</i><em>Realized</em></div>
      <div><i class="${unrealized > 0 ? "up" : unrealized < 0 ? "down" : ""}">${fmtMoney(unrealized)}</i><em>Unrealized</em></div>
    </div>
    <label class="er-switch" title="${disabled ? "Connect Dhan first" : on ? "Disconnect engine" : "Connect engine"}">
      <input type="checkbox" data-mode="${mode}" ${on ? "checked" : ""} ${disabled ? "disabled" : ""}>
      <span class="er-slider"></span>
    </label>
    ${msg ? `<div class="er-eng-msg">${msg}</div>` : ""}
  </div>`;
}

async function toggleEngine(mode, on) {
  if (mode === "live" && on) {
    const ok = confirm("ARM LIVE EQUITY ENGINE?\n\nYe Equity Research ke Buy calls par REAL Dhan CNC delivery orders place karega, aur target/stop/Avoid par SELL karega.\n\nContinue?");
    if (!ok) {
      loadEngines();
      return;
    }
  }
  if (STORE.enginesBusy) return;
  STORE.enginesBusy = true;
  try {
    const r = await api("/api/equity/engines/toggle", {
      method: "POST",
      body: JSON.stringify({ mode, enabled: on }),
    });
    if (r.error) throw new Error(r.error);
    if (r.engine) {
      if (!STORE.engines) STORE.engines = {};
      STORE.engines[mode] = r.engine;
      renderEngines(STORE.engines);
    }
  } catch (e) {
    const stage = document.getElementById("erStage");
    if (stage) stage.textContent = String(e.message || e);
    loadEngines();
  } finally {
    STORE.enginesBusy = false;
    loadEngines();
  }
}

async function api(path, opts) {
  const r = await fetch(path, Object.assign({ headers: { "Content-Type": "application/json" } }, opts || {}));
  if (!r.ok) throw new Error(await r.text().catch(() => r.statusText));
  return r.json();
}

async function loadStatus() {
  try {
    const s = await api("/api/equity/status");
    STORE.status = s;
    paintStatus(s);
    if (STORE.wasBusy && !s.busy) {
      STORE.wasBusy = false;
      loadCache();
    }
    STORE.wasBusy = s.busy;
  } catch (e) {
    const el = document.getElementById("erStage");
    if (el) el.textContent = "status unavailable";
  }
}

// "Scan all listed stocks": mirror the persisted flag onto the checkbox and give
// it a live ACTIVE / INACTIVE badge so the operator can see the mode at a glance
// (and while a scan is busy the badge says the mode the running scan uses).
function paintScanAll(filters) {
  paintAutoScan(filters);
  paintAutoPush(filters, STORE.engines);
  const cb = document.getElementById("erAll");
  const stat = document.getElementById("erAllStat");
  const lbl = document.getElementById("erAllLbl");
  if (!cb) return;
  const f = filters || (STORE.status && STORE.status.filters) || null;
  if (f && typeof f.scan_all !== "undefined") cb.checked = !!f.scan_all;
  else if (f && typeof f.scanAll !== "undefined") cb.checked = !!f.scanAll;
  const on = !!cb.checked;
  if (stat) {
    stat.textContent = on ? "ACTIVE - full market" : "INACTIVE - top 900";
    stat.style.color = on ? "var(--er-up)" : "var(--er-dim)";
    stat.style.fontWeight = on ? "700" : "400";
  }
  if (lbl) lbl.style.color = on ? "var(--er-up)" : "";
}

// Reflect the saved Auto-scan switch + interval (never clobber a field the user
// is actively typing in).
function paintAutoScan(filters) {
  const cb = document.getElementById("erAuto");
  const hrEl = document.getElementById("erAutoHr");
  const minEl = document.getElementById("erAutoMin");
  if (!filters) return;
  const on = typeof filters.auto_scan !== "undefined" ? !!filters.auto_scan : !!filters.autoScan;
  const total = Math.round(
    Number(
      typeof filters.auto_scan_minutes !== "undefined" ? filters.auto_scan_minutes : filters.autoScanMinutes
    )
  );
  const ae = document.activeElement;
  if (cb && ae !== cb) cb.checked = on;
  if (Number.isFinite(total) && total > 0) {
    const hrs = Math.floor(total / 60);
    const mins = total % 60;
    if (hrEl && ae !== hrEl) hrEl.value = String(hrs);
    if (minEl && ae !== minEl) minEl.value = String(mins);
  }
  const lbl = document.getElementById("erAutoLbl");
  if (lbl) lbl.style.color = on ? "var(--er-up)" : "";
}

// "Auto send scanned stocks to trading engines": persist the switch, reflect it
// with an ACTIVE / INACTIVE badge and a fade-in card, and show how many names
// have been sent and how many are still waiting for a free slot / margin.
function paintAutoPush(filters, engines) {
  const f = filters || (STORE.status && STORE.status.filters) || null;
  const box = document.getElementById("erPush");
  const chk = document.getElementById("erPushChk");
  const badge = document.getElementById("erPushBadge");
  const state = document.getElementById("erPushState");
  const note = document.getElementById("erPushNote");
  const sentEl = document.getElementById("erPushSent");
  const pendEl = document.getElementById("erPushPending");
  if (!box || !chk) return;
  if (f && typeof f.auto_push !== "undefined") chk.checked = !!f.auto_push;
  else if (f && typeof f.autoPush !== "undefined") chk.checked = !!f.autoPush;
  const on = !!chk.checked;
  box.classList.toggle("on", on);
  if (badge) badge.textContent = on ? "ACTIVE" : "INACTIVE";
  if (state) {
    state.textContent = on ? "ON" : "OFF";
    state.style.color = on ? "var(--er-up)" : "";
  }
  if (sentEl) {
    const n = f ? Number(typeof f.pushedCount !== "undefined" ? f.pushedCount : f.pushed_symbols ? f.pushed_symbols.length : 0) : 0;
    sentEl.textContent = String(Number.isFinite(n) ? n : 0);
  }
  if (pendEl) {
    const paper = engines && engines.paper ? Number(engines.paper.autoPushPending) || 0 : 0;
    const live = engines && engines.live ? Number(engines.live.autoPushPending) || 0 : 0;
    pendEl.textContent = `${paper}/${live}`;
    pendEl.title = `Paper ${paper} · Live ${live} - slot ya margin ka wait`;
  }
  if (note) {
    note.textContent = on
      ? "Naya scan finish hote hi naye symbols dono engines me jayenge; jo buy nahi ho paaye (slot/margin full) wo waiting me rahenge aur margin khulte hi buy honge."
      : "OFF hai: engines sirf apne normal Buy-call rules par chalte rahenge, koi auto push nahi hoga.";
  }
}

const saveAutoPush = () => {
  const chk = document.getElementById("erPushChk");
  const box = document.getElementById("erPush");
  if (!chk) return;
  const on = !!chk.checked;
  if (box) {
    box.classList.toggle("on", on);
    box.classList.add("fade-in");
    setTimeout(() => box.classList.remove("fade-in"), 500);
  }
  chk.disabled = true;
  api("/api/equity/filters", { method: "POST", body: JSON.stringify({ autoPush: on }) })
    .then((r) => {
      const f = (r && r.filters) || null;
      if (f) paintAutoPush(f, STORE.engines);
      if (on) {
        // A fresh hand-over should also clear the "already sent" ledger? No -
        // the server seeds it on enable, so only genuinely NEW scans are sent.
        loadStatus();
      }
    })
    .catch(() => {
      chk.checked = !on;
      if (box) box.classList.toggle("on", !on);
    })
    .finally(() => {
      chk.disabled = false;
    });
};

// "1h 30m" style label for the stage line.
function fmtCadence(totalMins) {
  const t = Math.max(0, Math.round(Number(totalMins) || 0));
  const h = Math.floor(t / 60);
  const m = t % 60;
  if (h > 0 && m > 0) return `${h}h ${m}m`;
  if (h > 0) return `${h}h`;
  return `${m}m`;
}

function paintStatus(s) {
  const stage = document.getElementById("erStage");
  const bar = document.getElementById("erBar");
  const btn = document.getElementById("erRefresh");
  if (!stage || !bar || !btn) return;
  btn.disabled = !!s.busy;
  btn.textContent = s.busy ? "Scanning..." : "Refresh now";
  paintScanAll(s.filters);
  if (s.busy) {
    stage.textContent = s.total ? `${s.stage} ${s.done}/${s.total}` : s.stage || "scanning";
    bar.style.width = s.total ? Math.min(100, Math.round((s.done / s.total) * 100)) + "%" : "12%";
  } else {
    const parts = [];
    if (s.asOf) parts.push(`EOD ${s.asOf}`);
    if (s.listed) parts.push(`${s.listed} listed`);
    if (s.universe) parts.push(`${s.universe} liquid`);
    if (s.filters && s.filters.scan_all) parts.push("full-market mode");
    if (s.filters && s.filters.auto_scan) {
      const mins = Math.round(Number(s.filters.auto_scan_minutes) || 0);
      if (mins > 0) parts.push(`auto scan ${fmtCadence(mins)}`);
    }
    if (s.nse || s.bse) parts.push(`NSE ${s.nse} / BSE ${s.bse} scanned`);
    if (s.errors && s.errors.length) parts.push(`${s.errors.length} warning(s)`);
    stage.textContent = parts.join(" | ") || "no scan yet";
    bar.style.width = "100%";
  }
}

async function loadCache() {
  try {
    const c = await api("/api/equity/cache");
    STORE.rows = Array.isArray(c.rows) ? c.rows : [];
    STORE.backtest = c.backtest || null;
    STORE.asOf = c.asOf || "";
    STORE.note = c.note || "";
    STORE.accuracy = c.accuracy || null;
    if (c.busy) {
      STORE.wasBusy = true;
      paintStatus({ busy: true, stage: c.stage, done: 0, total: 0, asOf: c.asOf });
    }
    renderCards(c);
    renderAccuracy();
    renderBacktest();
    renderRows();
  } catch (e) {
    const tb = document.getElementById("erRows");
    if (tb) tb.innerHTML = `<tr><td colspan="22" class="er-empty">${esc(String(e.message || e))}</td></tr>`;
  }
}

async function refresh() {
  try {
    const r = await api("/api/equity/refresh", { method: "POST", body: "{}" });
    if (r.busy && !r.started) {
      const stage = document.getElementById("erStage");
      if (stage) stage.textContent = "a scan is already running";
      return;
    }
    STORE.wasBusy = true;
    paintStatus({ busy: true, stage: "starting", done: 0, total: 0 });
  } catch (e) {
    const stage = document.getElementById("erStage");
    if (stage) stage.textContent = String(e.message || e);
  }
}

function capBandOf(r) {
  const cr = (Number(r.marketCap) || 0) / 1e7;
  if (cr < 500) return "micro";
  if (cr < 5000) return "small";
  if (cr < 50000) return "mid";
  return "large";
}

function playbookScore(r) {
  if (STORE.strategy === "composite") return (r.scores && r.scores.total) || 0;
  return (r.strategies && r.strategies[STORE.strategy]) || 0;
}

function filtered() {
  const rows = STORE.rows.filter((r) => {
    if (STORE.fnoOnly && !(r.options && r.options.fno)) return false;
    if (STORE.capBand !== "all" && capBandOf(r) !== STORE.capBand) return false;
    const call = r.call || {};
    if (STORE.statusFilter !== "all" && call.status !== STORE.statusFilter) return false;
    if ((Number(call.confidence) || 0) < STORE.minConf) return false;
    if ((Number(call.upsidePct) || 0) < STORE.minUpside) return false;
    if (playbookScore(r) < STORE.minScore) return false;
    if (!STORE.q) return true;
    const hay = `${r.symbol} ${r.name} ${r.sector} ${r.industry} ${(r.tags || []).join(" ")}`.toLowerCase();
    return hay.includes(STORE.q);
  });
  const key = STORE.sort;
  rows.sort((a, b) => {
    let av = 0;
    let bv = 0;
    if (key === "strategy") {
      av = playbookScore(a);
      bv = playbookScore(b);
    } else if (key === "turnoverLacs") {
      av = a.turnoverLacs;
      bv = b.turnoverLacs;
    } else if (key === "marketCap") {
      av = a.marketCap;
      bv = b.marketCap;
    } else if (key === "upside") {
      av = (a.call && a.call.upsidePct) || 0;
      bv = (b.call && b.call.upsidePct) || 0;
    } else if (key === "buyConf") {
      av = (a.call && a.call.confidence) || 0;
      bv = (b.call && b.call.confidence) || 0;
    } else if (key === "eta") {
      return ((a.call && a.call.expectedMonths) || 999) - ((b.call && b.call.expectedMonths) || 999);
    } else {
      av = (a.scores && a.scores[key]) || 0;
      bv = (b.scores && b.scores[key]) || 0;
    }
    return (bv || 0) - (av || 0);
  });
  return rows;
}

function renderCards(c) {
  const el = document.getElementById("erCards");
  if (!el) return;
  const rows = STORE.rows;
  const top = rows[0];
  const buys = rows.filter((r) => {
    const st = (r.call && r.call.status) || "";
    return st === "Strong Buy" || st === "Buy" || st === "Accumulate";
  });
  const avgUp = rows.length
    ? rows.reduce((s, r) => s + (Number(r.call && r.call.upsidePct) || 0), 0) / rows.length
    : 0;
  const avgConf = rows.length
    ? rows.reduce((s, r) => s + (Number(r.call && r.call.confidence) || 0), 0) / rows.length
    : 0;
  const avgEta = rows.length
    ? rows.reduce((s, r) => s + (Number(r.call && r.call.expectedMonths) || 0), 0) / rows.length
    : 0;
  el.innerHTML = `
    <div class="er-card"><b>${rows.length}</b><span>Scored stocks</span></div>
    <div class="er-card"><b>${c && c.nse != null ? c.nse : "-"} / ${c && c.bse != null ? c.bse : "-"}</b><span>NSE / BSE scanned</span></div>
    <div class="er-card"><b>${c && c.listed ? c.listed : "-"}</b><span>Listed (NSE + BSE)</span></div>
    <div class="er-card"><b>${c && c.universe ? c.universe : "-"}</b><span>Liquid universe</span></div>
    <div class="er-card"><b>${top ? esc(top.symbol) : "-"}</b><span>Top composite${top ? ` (${fmtNum(top.scores && top.scores.total)})` : ""}</span></div>
    <div class="er-card"><b class="up">${buys.length}</b><span>Actionable calls</span></div>
    <div class="er-card"><b>${fmtPct(avgUp)}</b><span>Avg 12m upside</span></div>
    <div class="er-card"><b>${avgConf ? fmtNum(avgConf) + "%" : "-"}</b><span>Avg buy confidence</span></div>
    <div class="er-card"><b>${avgEta ? fmtNum(avgEta) + " mo" : "-"}</b><span>Avg time to target</span></div>
    <div class="er-card"><b>${esc(STORE.asOf || "-")}</b><span>EOD bhavcopy date</span></div>`;
}

function renderBacktest() {
  const el = document.getElementById("erBt");
  if (!el) return;
  const bt = STORE.backtest;
  if (!bt || !bt.available) {
    el.innerHTML = `<div class="er-bt"><div class="er-bt-head"><b>Momentum backtest</b><span class="er-muted">${esc((bt && bt.note) || "waiting for the first scan")}</span></div></div>`;
    return;
  }
  const monthly = Array.isArray(bt.monthly) ? bt.monthly : [];
  const max = Math.max(1, ...monthly.map((x) => Math.abs(x)));
  const bars = monthly
    .map((m) => {
      const h = Math.max(4, Math.round((Math.abs(m) / max) * 30));
      const col = m >= 0 ? "var(--er-up)" : "var(--er-down)";
      return `<i title="${fmtPct(m)}" style="height:${h}px;background:${col}"></i>`;
    })
    .join("");
  el.innerHTML = `
  <div class="er-bt">
    <div class="er-bt-head">
      <b>Momentum backtest - top 5 equal weight, monthly</b>
      <span class="er-muted">${esc(bt.note || "")}</span>
    </div>
    <div class="er-bt-grid">
      <div><b class="${bt.totalReturnPct >= 0 ? "up" : "down"}">${fmtPct(bt.totalReturnPct)}</b><span>Total return</span></div>
      <div><b class="${bt.vsBenchmarkPct >= 0 ? "up" : "down"}">${fmtPct(bt.vsBenchmarkPct)}</b><span>Vs NIFTY</span></div>
      <div><b>${fmtPct(bt.winRatePct)}</b><span>Winning months</span></div>
      <div><b>${fmtPct(bt.avgMonthPct)}</b><span>Avg month</span></div>
      <div><b class="down">${fmtPct(bt.maxDrawdownPct)}</b><span>Max drawdown</span></div>
      <div><b>${bt.months}</b><span>Months tested</span></div>
    </div>
    <div class="er-mbars">${bars}</div>
  </div>`;
}

function renderRows() {
  const tb = document.getElementById("erRows");
  const count = document.getElementById("erCount");
  const more = document.getElementById("erMore");
  if (!tb) return;
  const rows = filtered();
  if (count) count.textContent = `${rows.length} match${rows.length === 1 ? "" : "es"}`;
  if (!rows.length) {
    tb.innerHTML = `<tr><td colspan="22" class="er-empty">${
      STORE.rows.length ? "No stock matches these filters." : "No scan yet. Press Refresh now."
    }</td></tr>`;
    if (more) more.style.display = "none";
    return;
  }
  const shown = rows.slice(0, STORE.limit);
  tb.innerHTML = shown
    .map((r, i) => {
      const s = r.scores || {};
      const tags = (r.tags || []).slice(0, 3)
        .map((x) => `<span class="er-tag">${esc(x)}</span>`)
        .join("");
      const chg = Number(r.changePct || 0);
      const pb = playbookScore(r);
      const c = r.call || {};
      return `<tr data-sym="${esc(r.symbol)}" data-ex="${esc(r.exchange || "")}">
        <td>${i + 1}</td>
        <td><span class="er-sym">${esc(r.symbol)}</span></td>
        <td><span class="er-ex">${esc(r.exchange || "-")}</span></td>
        <td>${esc(short(r.name, 30))}</td>
        <td>${esc(short(r.sector || "-", 18))}</td>
        <td>${statusBadge(c.status)}${c.upgradedFrom ? `<span class="er-upg" title="Upgraded from ${esc(c.upgradedFrom)} on ${esc(c.upgradedAt || "")}">UP</span>` : ""}</td>
        <td>${fmtPrice(r.price)}</td>
        <td class="up">${fmtPctOrDash(c.upsidePct)}</td>
        <td>${fmtNum(c.confidence)}%</td>
        <td>${fmtPrice(c.targetPrice)}</td>
        <td>${esc(c.timeframe || "-")}</td>
        <td title="Probability of touching target within 3 / 6 / 12 months">${probInt(c.prob3m)} / ${probInt(c.prob6m)} / ${probInt(c.prob12m)}%</td>
        <td>${fmtNum(c.rr)}</td>
        <td class="${chg >= 0 ? "up" : "down"}">${fmtPct(chg)}</td>
        <td><span class="er-score" style="color:${scoreColor(s.total)}">${fmtNum(s.total)}</span></td>
        <td><span class="er-score" style="color:${scoreColor(pb)}">${fmtNum(pb)}</span></td>
        <td>${fmtNum(s.fundamental)}</td>
        <td>${fmtNum(s.technical)}</td>
        <td>${fmtNum(s.base)}</td>
        <td>${fmtNum(s.valuation)}</td>
        <td>${fmtNum(s.confidence)}</td>
        <td>${tags}</td>
      </tr>`;
    })
    .join("");
  if (more) more.style.display = rows.length > STORE.limit ? "block" : "none";
}

function renderDrawer() {
  const el = document.getElementById("erDrawer");
  const r = STORE.selected;
  if (!el) return;
  if (!r) {
    el.classList.remove("open");
    return;
  }
  const s = r.scores || {};
  const f = r.fundamentals || {};
  const t = r.technicals || {};
  const o = r.options || {};
  const h = r.shareholding || {};
  const b = t.base || {};
  const st = r.strategies || {};
  const thesis = r.thesis || {};
  const bars = (v) => `<div class="er-bar"><i style="width:${Math.max(0, Math.min(100, v || 0))}%;background:${scoreColor(v)}"></i></div>`;
  const c = r.call || {};
  const flags = (r.flags || []).map((x) => `<span class="er-flag">${esc(x)}</span>`).join("") || '<span class="er-muted">none</span>';
  const tags = (r.tags || []).map((x) => `<span class="er-tag">${esc(x)}</span>`).join("") || '<span class="er-muted">none</span>';
  const list = (arr) => (arr && arr.length ? `<ul class="er-list">${arr.map((x) => `<li>${esc(x)}</li>`).join("")}</ul>` : '<div class="er-muted">none</div>');
  const kv = (k, v) => `<div class="er-kv"><span>${k}</span><span>${v}</span></div>`;
  const stratNames = { coffee: "Coffee Can", garp: "GARP", microcap: "Microcap", turnaround: "Turnaround", value: "Deep Value", momentum: "Momentum", accumulation: "Accumulation", quality: "Quality" };
  const stratRows = Object.keys(stratNames)
    .map((k) => kv(stratNames[k], `<b>${fmtNum(st[k])}</b>`) + bars(st[k]))
    .join("");
  el.innerHTML = `
    <button class="er-close" id="erClose">Close</button>
    <h3>${esc(r.symbol)} <span class="er-ex">${esc(r.exchange || "-")}</span> <span class="er-sub">${esc(r.name || "")}</span></h3>
    <div class="er-sub">${esc(r.sector || "-")}${r.industry ? " / " + esc(r.industry) : ""} | ${esc(r.isin || "")}</div>
    <div style="display:flex;gap:18px;margin-top:10px;flex-wrap:wrap">
      <div><b style="font-size:22px;color:${scoreColor(s.total)}">${fmtNum(s.total)}</b><div class="er-sub">Composite score</div></div>
      <div><b style="font-size:22px">${fmtPrice(r.price)}</b><div class="er-sub">Close</div></div>
      <div><b style="font-size:22px" class="${Number(r.changePct) >= 0 ? "up" : "down"}">${fmtPct(r.changePct)}</b><div class="er-sub">Day change</div></div>
      <div><b style="font-size:22px">${fmtCr(r.marketCap)}</b><div class="er-sub">Market cap</div></div>
      <div><b style="font-size:22px">${fmtNum(s.confidence)}%</b><div class="er-sub">Data coverage</div></div>
    </div>

    <div class="er-sec"><h4>Buy call</h4>
      <div style="display:flex;gap:16px;flex-wrap:wrap;align-items:center">
        <div>${statusBadge(c.status)}<div class="er-sub">Call</div></div>
        <div><b style="font-size:22px">${fmtNum(c.confidence)}%</b><div class="er-sub">Buy confidence</div></div>
        <div><b style="font-size:22px" class="up">${fmtPctOrDash(c.upsidePct)}</b><div class="er-sub">12m base upside</div></div>
        <div><b style="font-size:22px">${fmtPctOrDash(c.expectedProfitPct)}</b><div class="er-sub">Confidence-weighted</div></div>
      </div>
      <div class="er-grid" style="margin-top:8px">
        ${kv("Upside range", `${fmtPctOrDash(c.upsideLowPct)} to ${fmtPctOrDash(c.upsideHighPct)}`)}
        ${kv("Target price", fmtPrice(c.targetPrice))}
        ${kv("Stop loss", fmtPrice(c.stopPrice))}
        ${kv("Risk", fmtPctOrDash(c.riskPct))}
        ${kv("Reward : risk", fmtNum(c.rr))}
        ${kv("Entry zone", `${fmtPrice(c.entryLow)} - ${fmtPrice(c.entryHigh)}`)}
        ${kv("Expected timeframe", esc(c.timeframe || "-"))}
        ${kv("Probability 3m / 6m / 12m", `${probInt(c.prob3m)}% / ${probInt(c.prob6m)}% / ${probInt(c.prob12m)}%`)}
        ${kv("Median days to target", fmtNum(c.medianDays))}
        ${kv("Status since", esc(c.statusSince || "-"))}
        ${kv("Upgraded from", c.upgradedFrom ? `${esc(c.upgradedFrom)} on ${esc(c.upgradedAt || "-")}` : "-")}
        ${kv("Next step", esc(c.nextStep || "-"))}
        ${kv("Grade", esc(c.grade || "-"))}
      </div>
      <div class="er-muted" style="margin-top:8px">Base case: about ${fmtPctOrDash(c.upsidePct)} over ${esc(c.timeframe || "an estimated period")}, with a ${probInt(c.prob12m)}% chance of touching the target within 12 months. Estimated from historical replay and the stock's volatility - not a guarantee.</div>
      <div style="margin-top:6px">${barsMini(c.prob3m)}<div class="er-muted">3-month probability</div></div>
      <div style="margin-top:4px">${barsMini(c.prob6m)}<div class="er-muted">6-month probability</div></div>
      <div style="margin-top:4px">${barsMini(c.prob12m)}<div class="er-muted">12-month probability</div></div>
    </div>

    <div class="er-sec"><h4>Thesis</h4>
      <div class="er-muted" style="font-size:12px;color:var(--er-text)">${esc(thesis.summary || "No thesis yet")}</div>
      <div class="er-grid" style="margin-top:8px">
        <div><h4>Supports</h4>${list(thesis.supports)}</div>
        <div><h4>Risks</h4>${list(thesis.risks)}</div>
      </div>
      <div style="margin-top:8px"><h4>Playbook tags</h4>${tags}</div>
      <div style="margin-top:8px"><h4>Flags</h4>${flags}</div>
    </div>

    <div class="er-sec"><h4>Playbook scores</h4>${stratRows}</div>

    <div class="er-sec"><h4>Composite breakdown</h4>
      ${kv("Fundamental", `<b>${fmtNum(s.fundamental)}</b>`)}${bars(s.fundamental)}
      ${kv("Technical", `<b>${fmtNum(s.technical)}</b>`)}${bars(s.technical)}
      ${kv("Base quality", `<b>${fmtNum(s.base)}</b>`)}${bars(s.base)}
      ${kv("Valuation", `<b>${fmtNum(s.valuation)}</b>`)}${bars(s.valuation)}
      ${kv("Quality", `<b>${fmtNum(s.quality)}</b>`)}${bars(s.quality)}
      ${kv("Size fit", `<b>${fmtNum(s.size)}</b>`)}${bars(s.size)}
      ${kv("Liquidity", `<b>${fmtNum(s.liquidity)}</b>`)}${bars(s.liquidity)}
    </div>

    <div class="er-sec"><h4>Base structure</h4>
      ${b.available ? `<div class="er-grid">
        ${kv("Base depth (120d)", fmtPctOrDash(b.depth120))}
        ${kv("Tightness (20d)", fmtPctOrDash(b.tight20))}
        ${kv("26w breakout", b.breakout26w ? "Yes" : "No")}
        ${kv("Stage 2", b.stage2 ? "Yes" : "No")}
        ${kv("Weeks near high", fmtPct((Number(b.nearHighFrac) || 0) * 100))}
        ${kv("Up/down volume", fmtNum(b.updownVol))}
        ${kv("From base low", fmtPctOrDash(b.fromBaseLow))}
      </div>` : '<div class="er-muted">Not enough history for base metrics.</div>'}
    </div>

    <div class="er-sec"><h4>Fundamentals</h4>
      <div class="er-muted">${f.available ? "Yahoo Finance key statistics and 3-year statements" : "Fundamentals unavailable for this stock"}</div>
      <div class="er-grid" style="margin-top:6px">
        ${kv("Market cap", fmtCr(r.marketCap))}
        ${kv("PE", fmtNum(f.pe))}
        ${kv("PB", fmtNum(f.pb))}
        ${kv("PEG", fmtNum(f.peg))}
        ${kv("ROE", fmtPctOrDash(f.roe))}
        ${kv("Debt / equity", fmtNum(f.debtEquity))}
        ${kv("Revenue CAGR 3y", fmtPctOrDash(f.revenueCagr3))}
        ${kv("Profit CAGR 3y", fmtPctOrDash(f.profitCagr3))}
        ${kv("Operating margin", fmtPctOrDash(f.opMargin))}
        ${kv("CFO / PAT", fmtNum(f.cfoPat))}
        ${kv("Rev growth", fmtPctOrDash(f.revGrowth))}
        ${kv("Div yield", fmtPctOrDash(f.divYield))}
      </div>
    </div>

    <div class="er-sec"><h4>Technicals</h4>
      <div class="er-grid">
        ${kv("RSI 14", fmtNum(t.rsi))}
        ${kv("ATR %", fmtPctOrDash(t.atrPct))}
        ${kv("1M return", fmtPctOrDash(t.ret1m))}
        ${kv("3M return", fmtPctOrDash(t.ret3m))}
        ${kv("6M return", fmtPctOrDash(t.ret6m))}
        ${kv("1Y return", fmtPctOrDash(t.ret1y))}
        ${kv("RS vs NIFTY 6M", fmtPctOrDash(t.rs6m))}
        ${kv("Volume ratio", fmtNum(t.volRatio))}
        ${kv("52w high", fmtPrice(t.high52))}
        ${kv("From high", fmtPctOrDash(t.fromHighPct))}
        ${kv("20d breakout", t.breakout20 ? "Yes" : "No")}
        ${kv("EMA 20 / 50 / 200", `${fmtPrice(t.ema20)} / ${fmtPrice(t.ema50)} / ${fmtPrice(t.ema200)}`)}
      </div>
    </div>

    <div class="er-sec"><h4>Option metrics (F&amp;O)</h4>
      ${o.available ? `<div class="er-grid">
        ${kv("Expiry", esc(o.expiry || "-"))}
        ${kv("ATM IV", fmtPctOrDash(o.atmIv))}
        ${kv("IV percentile", fmtPctOrDash(o.ivPct))}
        ${kv("PCR", fmtNum(o.pcr))}
        ${kv("Call OI", fmtNum(o.ceOi))}
        ${kv("Put OI", fmtNum(o.peOi))}
      </div>` : '<div class="er-muted">Not an F&amp;O stock or the chain is unavailable.</div>'}
    </div>

    <div class="er-sec"><h4>Shareholding</h4>
      ${h.available ? `<div class="er-grid">
        ${kv("Promoter", fmtPctOrDash(h.promoter))}
        ${kv("Promoter change", fmtPctOrDash(h.promoterChange))}
        ${kv("Public", fmtPctOrDash(h.public))}
        ${kv("As of", esc(h.date || "-"))}
      </div>` : '<div class="er-muted">NSE shareholding unavailable.</div>'}
    </div>

    <div class="er-sec"><h4>Liquidity</h4>
      <div class="er-grid">
        ${kv("Turnover", "Rs " + fmtNum(r.turnoverLacs) + " lakh")}
        ${kv("Delivery", fmtPctOrDash(r.deliveryPct))}
      </div>
    </div>`;
  el.classList.add("open");
}


function probInt(v) {
  const n = Number(v);
  return Number.isFinite(n) ? String(Math.round(n)) : "-";
}

function statusBadge(st) {
  const s = String(st || "");
  const cls = s === "Strong Buy" ? "st-strong" : s === "Buy" ? "st-buy" : s === "Accumulate" ? "st-acc" : s === "Watch" ? "st-watch" : s === "Avoid" ? "st-avoid" : "";
  return s ? `<span class="er-status ${cls}">${esc(s)}</span>` : '<span class="er-muted">-</span>';
}

function outcomeLabel(o) {
  return ({ hit: "Target hit", near: "Near target", miss: "Miss", stopped: "Stopped", expired: "Expired", open: "Open" })[o] || o || "-";
}

function outcomeColor(o) {
  if (o === "hit") return "var(--er-up)";
  if (o === "near") return "var(--er-warn)";
  if (o === "open") return "var(--er-dim)";
  if (o === "miss" || o === "stopped") return "var(--er-down)";
  return "var(--er-dim)";
}

function renderAccuracy() {
  const el = document.getElementById("erAcc");
  if (!el) return;
  const a = STORE.accuracy;
  if (!a) {
    el.innerHTML = `<div class="er-acc"><div class="er-acc-head"><b>Prediction accuracy</b><span class="er-muted">waiting for the first scan</span></div></div>`;
    return;
  }
  const view = STORE.accView === "live" ? "live" : "replay";
  const d = a[view] || {};
  const months = Array.isArray(d.months) ? d.months.slice(-9) : [];
  const bars = months
    .map((m) => {
      const rate = Number(m.rate) || 0;
      const h = Math.max(4, Math.min(100, rate));
      const col = rate >= 60 ? "var(--er-up)" : rate >= 40 ? "var(--er-warn)" : "var(--er-down)";
      return `<i title="${esc(m.month)}: ${fmtNum(m.rate)}% of ${m.n} calls (avg ${fmtPct(m.avgRet)})" style="height:${h}%;background:${col}"><span>${esc(String(m.month).slice(5))}</span></i>`;
    })
    .join("");
  const buckets = (d.buckets || [])
    .map((b) => `<div>${b.label} conf ${barsMini(b.rate)}<div class="er-kv"><span>${b.good}/${b.n} correct</span><span>${fmtNum(b.rate)}%</span></div></div>`)
    .join("");
  const log = (d.log || []).slice(0, 14);
  const logRows = log
    .map((e) => {
      const out = e.outcome || e.state || "-";
      return `<tr>
        <td>${esc(e.date || "-")}</td>
        <td><span class="er-sym">${esc(e.symbol || "-")}</span></td>
        <td>${statusBadge(e.status)}</td>
        <td>${fmtNum(e.confidence)}%</td>
        <td>${fmtPctOrDash(e.upsidePct)}</td>
        <td>${fmtPrice(e.entry || e.price)}</td>
        <td class="${(Number(e.maxGainPct) || 0) >= 0 ? "up" : "down"}">${fmtPctOrDash(e.maxGainPct)}</td>
        <td style="color:${outcomeColor(out)}">${esc(outcomeLabel(out))}</td>
        <td class="${(Number(e.returnPct) || 0) >= 0 ? "up" : "down"}">${fmtPctOrDash(e.returnPct)}</td>
      </tr>`;
    })
    .join("");
  const liveN = (a.live && a.live.open) || 0;
  el.innerHTML = `
  <div class="er-acc">
    <div class="er-acc-head">
      <b>Prediction accuracy - did the calls work?</b>
      <div class="er-acc-tabs">
        <button data-acc="replay" class="${view === "replay" ? "on" : ""}">Replay (2y indicative)</button>
        <button data-acc="live" class="${view === "live" ? "on" : ""}">Live journal (${liveN} open)</button>
      </div>
    </div>
    <div class="er-acc-grid">
      <div><b class="up">${fmtNum(d.hit)}</b><span>Target hit</span></div>
      <div><b style="color:var(--er-warn)">${fmtNum(d.near)}</b><span>Near target (60%+)</span></div>
      <div><b class="down">${fmtNum(d.miss)}</b><span>Missed</span></div>
      <div><b>${fmtNum(d.evaluated)}</b><span>Evaluated</span></div>
      ${view === "live" ? `<div><b class="up">${fmtNum(d.upgraded)}</b><span>Upgraded to Buy</span></div>` : ""}
      <div><b>${fmtNum(d.goodRate)}%</b><span>Hit + near rate</span></div>
      <div><b class="${(Number(d.avgReturn) || 0) >= 0 ? "up" : "down"}">${fmtPct(d.avgReturn)}</b><span>Avg return (63d)</span></div>
      <div><b>${fmtPct(d.medianReturn)}</b><span>Median return</span></div>
    </div>
    ${months.length ? `<div class="er-chart">${bars}</div><div class="er-muted">Bars = % of that month's calls that hit or came within 60% of target over the next 3 months.</div>` : ""}
    ${buckets ? `<div class="er-cal"><div class="er-muted" style="grid-column:1/-1">Calibration by buy confidence</div>${buckets}</div>` : ""}
    ${logRows ? `<div class="er-scroll" style="max-height:190px;margin-top:8px"><table class="er-log"><thead><tr><th>Date</th><th>Symbol</th><th>Call</th><th>Conf</th><th>Upside</th><th>Entry</th><th>Max gain</th><th>Result</th><th>Return</th></tr></thead><tbody>${logRows}</tbody></table></div>` : `<div class="er-muted" style="margin-top:8px">No closed predictions yet - live journal will fill as scans run.</div>`}
    ${view === "live" && (a.upgrades || []).length ? `<div style="margin-top:10px"><div class="er-muted">Recent upgrades to Buy (Watch / Accumulate → Buy)</div><div class="er-scroll" style="max-height:150px;margin-top:4px"><table class="er-log"><thead><tr><th>Date</th><th>Symbol</th><th>Move</th><th>Price</th><th>Score</th></tr></thead><tbody>${(a.upgrades || []).slice(0, 12).map((u) => `<tr><td>${esc(u.date || "-")}</td><td><span class="er-sym">${esc(u.symbol || "-")}</span></td><td>${esc(u.from || "-")} to <b class="up">${esc(u.to || "Buy")}</b></td><td>${fmtPrice(u.price)}</td><td>${fmtNum(u.score)}</td></tr>`).join("")}</tbody></table></div></div>` : ""}
    <div class="er-muted" style="margin-top:8px">${esc(a.note || "")}${view === "live" ? " Live journal only counts calls after you started using this tab." : ""}</div>
  </div>`;
}

function barsMini(v) {
  const n = Math.max(0, Math.min(100, Number(v) || 0));
  const col = n >= 60 ? "var(--er-up)" : n >= 40 ? "var(--er-warn)" : "var(--er-down)";
  return `<div class="er-bar"><i style="width:${n}%;background:${col}"></i></div>`;
}

function scoreColor(v) {
  const n = Number(v) || 0;
  if (n >= 70) return "var(--er-up)";
  if (n >= 55) return "var(--er-warn)";
  return "var(--er-down)";
}

function fmtNum(v) {
  const n = Number(v);
  return Number.isFinite(n) ? n.toFixed(1) : "-";
}

function fmtMoney(v) {
  const n = Number(v) || 0;
  return `${n >= 0 ? "+" : "-"}Rs ${Math.round(Math.abs(n)).toLocaleString("en-IN")}`;
}

function fmtPct(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return "-";
  return `${n >= 0 ? "+" : ""}${n.toFixed(2)}%`;
}

function fmtPctOrDash(v) {
  return v === null || v === undefined ? "-" : fmtPct(v);
}

function fmtPrice(v) {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? `Rs ${n.toFixed(2)}` : "-";
}

function fmtCr(v) {
  const n = Number(v);
  if (!Number.isFinite(n) || n <= 0) return "-";
  const cr = n / 1e7;
  return cr >= 1000 ? `Rs ${(cr / 1000).toFixed(1)}k cr` : `Rs ${cr.toFixed(0)} cr`;
}

function short(s, n) {
  const t = String(s || "");
  return t.length > n ? t.slice(0, n - 1) + "." : t;
}

function esc(s) {
  return String(s === null || s === undefined ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
