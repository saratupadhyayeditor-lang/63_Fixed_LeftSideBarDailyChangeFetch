// Keep the chart toolbar dropdown menus fully inside the visible panel.
//
// The Indicators menu and the Chart Type menu are absolutely positioned at
// left:0 under their buttons (.ind-menu). The toolbar sits inside .content,
// which clips its overflow (overflow:hidden). Buttons on the right half of the
// toolbar therefore push a 300px-wide menu past the right edge of .content,
// cutting half of it off so it cannot be clicked. The indicator settings panel
// (.ind-settings) has the same problem at the right edge.
//
// Rather than special-casing each menu, re-anchor any open menu to the nearest
// clipping ancestor and cap its size to fit. The whole app is scaled with a CSS
// `zoom` on <html> (textsize.js), and zoom scales both getBoundingClientRect
// and the pixel lengths we write back, so all measurements are taken in the
// zoomed (device) space and converted back to the menu's own offset-parent
// space by dividing by the zoom factor.

const MENU_SELECTOR = ".ind-menu, .ind-settings";
const PAD = 8;

function zoomFactor() {
  const z = parseFloat(getComputedStyle(document.documentElement).zoom);
  return isFinite(z) && z > 0 ? z : 1;
}

// Nearest ancestor that clips overflow (the element that would cut a menu off).
function clipAncestor(el) {
  let node = el.parentElement;
  while (node && node !== document.body && node !== document.documentElement) {
    const cs = getComputedStyle(node);
    const clipped = /(auto|hidden|scroll|clip)/;
    if (clipped.test(cs.overflowX) || clipped.test(cs.overflowY)) return node;
    node = node.parentElement;
  }
  return null;
}

function clampOne(menu) {
  if (!menu || menu.classList.contains("hidden")) return;
  const wrap = menu.offsetParent;
  if (!wrap) return;

  // Undo the previous pass so the natural layout is measured again.
  menu.style.left = "";
  menu.style.top = "";
  menu.style.right = "";
  menu.style.maxWidth = "";
  menu.style.maxHeight = "";

  const z = zoomFactor();
  const r = menu.getBoundingClientRect();
  if (!r.width || !r.height) return; // not laid out yet

  const clipEl = clipAncestor(menu);
  let clip;
  if (clipEl) {
    const c = clipEl.getBoundingClientRect();
    clip = { left: c.left, top: c.top, right: c.right, bottom: c.bottom };
  } else {
    clip = {
      left: 0,
      top: 0,
      right: window.innerWidth || document.documentElement.clientWidth,
      bottom: window.innerHeight || document.documentElement.clientHeight,
    };
  }

  const maxW = Math.max(0, clip.right - clip.left - 2 * PAD);
  const maxH = Math.max(0, clip.bottom - clip.top - 2 * PAD);
  const w = Math.min(r.width, maxW);
  const h = Math.min(r.height, maxH);

  let left = r.left;
  if (left + w > clip.right - PAD) left = clip.right - PAD - w;
  if (left < clip.left + PAD) left = clip.left + PAD;

  const wr = wrap.getBoundingClientRect();
  const gap = Math.max(4, r.top - wr.bottom);
  let top = r.top;
  if (top + h > clip.bottom - PAD) {
    const above = wr.top - gap - h;
    top = above >= clip.top + PAD ? above : clip.bottom - PAD - h;
  }
  if (top < clip.top + PAD) top = clip.top + PAD;

  // Only shrink when the menu would not otherwise fit, so the original CSS
  // caps (e.g. .ind-menu max-height:420px) still apply when there is room.
  if (maxW < r.width) menu.style.maxWidth = maxW / z + "px";
  if (maxH < r.height) menu.style.maxHeight = maxH / z + "px";
  menu.style.left = (left - wr.left) / z + "px";
  menu.style.top = (top - wr.top) / z + "px";
  menu.style.right = "auto";
}

const pending = new Set();
let rafId = 0;

function scheduleClamp(el) {
  if (el) pending.add(el);
  else document.querySelectorAll(MENU_SELECTOR).forEach((m) => pending.add(m));
  if (rafId) return;
  rafId = requestAnimationFrame(() => {
    rafId = 0;
    const batch = Array.from(pending);
    pending.clear();
    batch.forEach(clampOne);
  });
}

export function bootFloatMenu() {
  // Menus reveal/hide themselves by swapping the `hidden` class (WASM does this
  // for Indicators + the settings panel, index.html for Chart Type). Clamp each
  // menu as soon as it becomes visible. Only class changes are observed, so the
  // inline styles written by clampOne can never retrigger the observer.
  const menuObserver = new MutationObserver((records) => {
    for (const rec of records) {
      const t = rec.target;
      if (t instanceof Element && t.matches(MENU_SELECTOR) && !t.classList.contains("hidden")) {
        scheduleClamp(t);
      }
    }
  });
  menuObserver.observe(document.body, {
    attributes: true,
    attributeFilter: ["class"],
    subtree: true,
  });

  // The text-size control writes documentElement.style.zoom, which moves every
  // edge without firing a resize; watch it and re-clamp. Window resizes and any
  // container scroll (chart panel, etc.) also move the anchor.
  const rootObserver = new MutationObserver(() => scheduleClamp());
  rootObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["style"] });
  window.addEventListener("resize", () => scheduleClamp());
  window.addEventListener("scroll", () => scheduleClamp(), true);

  scheduleClamp();
}
