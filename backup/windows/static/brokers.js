// Broker Settings tab.
//
// ONE page owns the API credentials for every supported broker plus the master
// enable switch per broker. The trading engines never read this page directly:
// they only keep their own "send orders to this broker" tick, and a broker can
// only be ticked there once it is switched ON and fully configured here.
//
// Secrets are masked by the backend. An input left showing the mask is sent back
// as-is and the server keeps the stored value, so opening this page and saving
// can never erase a token.

const API = {
  catalog: () => get("/api/brokers"),
  save: (v) => post("/api/brokers", v),
  test: (v) => post("/api/brokers/test", v),
};

async function get(url) {
  const r = await fetch(url);
  return r.json();
}
async function post(url, body) {
  const r = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body || {}),
  });
  return r.json();
}

let CATALOG = null;
let styleDone = false;

function esc(s) {
  return String(s == null ? "" : s).replace(/[&<>"']/g, (c) => (
    { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]
  ));
}

function css() {
  if (styleDone) return;
  styleDone = true;
  const s = document.createElement("style");
  s.textContent = `
    .bs-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:10px}
    .bs-title{font-size:16px;font-weight:800;color:#c4a9ff;text-transform:uppercase;letter-spacing:.3px;margin:0}
    .bs-sub{font-size:11px;color:#8a8ab0;flex:1 1 380px;min-width:240px}
    .bs-btn{background:#1b1b36;color:#d8d8f0;border:1px solid #3a3a60;border-radius:4px;
      padding:4px 10px;font-size:11px;font-weight:700;cursor:pointer}
    .bs-btn:hover{border-color:#6a5aa0;color:#fff}
    .bs-btn.primary{background:#241b42;border-color:#6a4ea8;color:#e6dcff}
    .bs-btn.danger{background:#2a1414;border-color:#7e2d2d;color:#ffb0b0}
    .bs-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:10px}
    .bs-card{background:#0d0d1e;border:1px solid #2a2a55;border-radius:6px;padding:10px 12px}
    .bs-card.on{border-color:#2f6f5a;background:#0b1a16}
    .bs-card-head{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-bottom:6px}
    .bs-card-head b{font-size:13px;color:#e0e0ff}
    .bs-tag{font-size:9px;font-weight:700;border-radius:3px;padding:2px 6px;white-space:nowrap}
    .bs-tag.native{color:#00d4aa;background:#0e2c24;border:1px solid #1f6f4a}
    .bs-tag.engine{color:#ffb74d;background:#2a220d;border:1px solid #7e6a2d}
    .bs-tag.ready{color:#66ccff;background:#0e2433;border:1px solid #2a6a8f}
    .bs-tag.pending{color:#ff9e9e;background:#2a1414;border:1px solid #7e2d2d}
    .bs-card .bs-note{font-size:10px;color:#8a8ab0;margin:0 0 8px}
    .bs-row{display:flex;align-items:center;gap:8px;margin:5px 0}
    .bs-row label{flex:0 0 118px;font-size:11px;color:#b0b0d0}
    .bs-row label.req::after{content:" *";color:#ff7b72}
    .bs-row input{flex:1 1 auto;min-width:0;background:#0a0a18;color:#e0e0ff;border:1px solid #3a3a60;
      border-radius:3px;padding:3px 6px;font-size:11px}
    .bs-row input:focus{outline:none;border-color:#6a5aa0}
    .bs-foot{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:8px;
      border-top:1px dashed #2a2a55;padding-top:8px}
    .bs-status{font-size:10px;color:#8a8ab0;flex:1 1 auto;min-width:120px}
    .bs-status.ok{color:#00d4aa}
    .bs-status.err{color:#ff7b72}
    .bs-status.warn{color:#e3b341}
    .bs-switch{display:flex;align-items:center;gap:5px;font-size:11px;font-weight:700;color:#b0b0d0;cursor:pointer}
    .bs-switch input{accent-color:#00d4aa;width:15px;height:15px}
    .bs-err{color:#ff7b72;font-size:11px;margin:6px 0}
  `;
  document.head.appendChild(s);
}

function fieldRow(f, value) {
  const type = f.kind === "password" ? "password" : (f.kind === "number" ? "number" : "text");
  const v = value == null ? "" : String(value);
  const ph = f.secret ? 'placeholder="(saved - edit to replace)"' : "";
  return `<div class="bs-row">
    <label class="${f.required ? "req" : ""}" title="${esc(f.help)}">${esc(f.label)}</label>
    <input type="${type}" data-field="${esc(f.key)}" value="${esc(v)}" ${ph} autocomplete="off" spellcheck="false">
  </div>`;
}

function cardHTML(br) {
  const a = br.account || {};
  const usable = !!a.usable;
  const tags = [
    br.nativeTrail
      ? `<span class="bs-tag native" title="Broker apna trailing stop chalata hai (BO / Super Order / TSL). Engine us position par apna trail nahi chalayega.">native trail</span>`
      : `<span class="bs-tag engine" title="Is broker me native trailing nahi hai, isliye engine khud trail chalayega.">engine trail</span>`,
    br.adapterReady
      ? `<span class="bs-tag ready" title="Order adapter live hai: entry, exit aur cancel is broker par asli order bhejte hain.">adapter ready</span>`
      : `<span class="bs-tag pending" title="Is broker ka order adapter is build me available nahi hai.">adapter pending</span>`,
  ].join(" ");
  const inHeader = !!br.credentialsInHeader;
  // Dhan's keys are entered in the app header (the Dhan Connect box). Showing a
  // second copy of the same inputs here invited the operator to "fix" the
  // working header login with a stale value, so the card is display-only.
  const fields = inHeader
    ? `<div class="bs-note" style="border:1px dashed #3a3a60;border-radius:3px;padding:6px;margin:6px 0">
         Credentials header ke <b>Dhan Connect</b> box se aate hain. Wahan connected hone par yahan ka Test bhi pass hoga; yahan sirf engine routing ke liye ON/OFF karein.
       </div>`
    : (br.fields || []).map((f) => fieldRow(f, (a.fields || {})[f.key])).join("");
  const missing = inHeader ? [] : (a.missing || []);
  const missingNote = missing.length
    ? `<div class="bs-err">Missing required: ${esc(missing.join(", "))}</div>`
    : "";
  return `<div class="bs-card${usable ? " on" : ""}" data-broker="${esc(br.id)}">
    <div class="bs-card-head">
      <b>${esc(br.name)}</b>
      ${tags}
      <label class="bs-switch" style="margin-left:auto" title="ON: is broker par orders bheje ja sakte hain (engine me tick karne par). OFF: credentials save rahenge par order nahi jayega.">
        <input type="checkbox" data-enable ${a.enabled ? "checked" : ""}> enabled
      </label>
    </div>
    <p class="bs-note">${esc(br.note || "")}</p>
    ${fields}
    ${missingNote}
    <div class="bs-foot">
      <button class="bs-btn primary" data-act="save">Save</button>
      <button class="bs-btn" data-act="test">Test</button>
      <span class="bs-status" data-status></span>
    </div>
  </div>`;
}

function render() {
  const root = document.getElementById("brokerRoot");
  if (!root) return;
  if (!CATALOG) {
    root.innerHTML = `<div class="bs-sub">Loading broker settings...</div>`;
    return;
  }
  const brokers = CATALOG.brokers || [];
  root.innerHTML = `
    <div class="bs-head">
      <h3 class="bs-title">Broker Settings</h3>
      <span class="bs-sub">Sabhi brokers ke API keys / tokens ek hi jagah. Har broker ka "enabled" switch ON karne par hi trading engine us broker par order bhej sakta hai; engine ke Order Placement Method block me har engine ka apna per-broker tick hai. Secret fields masked rehte hain - page save karne se stored token kabhi erase nahi hota.</span>
      <button class="bs-btn" data-act="reload">Reload</button>
    </div>
    <div class="bs-grid">${brokers.map(cardHTML).join("")}</div>`;
  root.querySelector('[data-act="reload"]').addEventListener("click", () => load(true));
  root.querySelectorAll(".bs-card").forEach((card) => {
    const id = card.getAttribute("data-broker");
    card.querySelector('[data-act="save"]').addEventListener("click", () => saveCard(card, id));
    card.querySelector('[data-act="test"]').addEventListener("click", () => testCard(card, id));
  });
}

function collect(card) {
  const fields = {};
  card.querySelectorAll("input[data-field]").forEach((inp) => {
    fields[inp.getAttribute("data-field")] = inp.value;
  });
  const en = card.querySelector("input[data-enable]");
  // A header-credential broker (Dhan) has no inputs here; never post an empty
  // map that could look like "the operator cleared the credentials".
  const hasInputs = card.querySelector("input[data-field]") != null;
  return { enabled: !!(en && en.checked), fields: hasInputs ? fields : undefined };
}

function setStatus(card, text, cls) {
  const el = card.querySelector("[data-status]");
  if (!el) return;
  el.textContent = text;
  el.className = "bs-status" + (cls ? " " + cls : "");
}

async function saveCard(card, id) {
  const body = collect(card);
  setStatus(card, "saving...");
  const r = await API.save(Object.assign({ id }, body));
  if (r && r.ok) {
    CATALOG = r;
    render();
    const fresh = document.querySelector(`.bs-card[data-broker="${id}"]`);
    if (fresh) setStatus(fresh, "saved", "ok");
  } else {
    setStatus(card, (r && r.error) || "save failed", "err");
  }
}

/* Has the operator typed something that is not saved yet? Test never saves on
   its own: writing an untested (possibly wrong) secret over a working one is
   exactly how a good account used to turn into a broken one. */
function hasUnsavedEdits(card, id) {
  const broker = (CATALOG && CATALOG.brokers || []).find((b) => b.id === id);
  const stored = (broker && broker.account && broker.account.fields) || {};
  let dirty = false;
  card.querySelectorAll("input[data-field]").forEach((inp) => {
    const k = inp.getAttribute("data-field");
    if (!inp.value) return;
    if (inp.type === "password") {
      dirty = true;
      return;
    }
    if (inp.value !== String(stored[k] == null ? "" : stored[k])) dirty = true;
  });
  return dirty;
}

async function testCard(card, id) {
  if (hasUnsavedEdits(card, id)) {
    setStatus(card, "Save karke test karein - unsaved value test nahi hoti", "warn");
    return;
  }
  setStatus(card, "testing...");
  const r = await API.test({ id });
  if (r && r.ok) {
    const inst = r.instruments || {};
    // Report the contract list too: a broker can authenticate yet still fail
    // every order because its instrument master is unreachable.
    const extra = inst.loaded
      ? ` · ${inst.loaded} instruments`
      : (inst.error ? ` · instruments: ${inst.error}` : " · instruments: none");
    // Be explicit about what the probe did NOT prove: a token-only check can
    // never validate an api secret, and claiming otherwise is how wrong keys
    // looked "connected" before.
    const un = (r.unverified || []).filter(Boolean);
    const note = un.length ? ` · unverified: ${un.join(", ")}` : "";
    setStatus(card, `connected (${r.message || id})${extra}${note}`, un.length ? "warn" : "ok");
  } else {
    const msg = r && (r.error || (r.missing && `missing: ${r.missing.join(", ")}`)) || "test failed";
    setStatus(card, msg, "err");
  }
}

async function load(force) {
  if (!force && CATALOG) {
    render();
    return;
  }
  const r = await API.catalog();
  if (r && r.ok) {
    CATALOG = r;
  }
  render();
}

export function bootBrokers() {
  css();
  load(true);
  document.addEventListener("tabshown", (ev) => {
    if (ev.detail === "brokers") load(true);
  });
}
