// Indian Standard Time (UTC+05:30) formatting helpers.
//
// Every timestamp the engines and ledger produce is real epoch milliseconds.
// The UI must always render them as IST regardless of the viewer's browser
// timezone, so all labels go through these helpers instead of the browser's
// local `Date#getHours()` / `toLocaleTimeString()`, which follow the machine
// timezone and would drift for a non-IST viewer.

const IST_OFFSET_MS = 5.5 * 60 * 60 * 1000;
const pad2 = (n) => String(n).padStart(2, "0");

/** IST calendar/clock fields for an epoch-millisecond timestamp. */
export function istParts(ms) {
  const d = new Date(Number(ms) + IST_OFFSET_MS);
  return {
    y: d.getUTCFullYear(),
    mo: d.getUTCMonth() + 1,
    day: d.getUTCDate(),
    h: d.getUTCHours(),
    mi: d.getUTCMinutes(),
    s: d.getUTCSeconds(),
    wd: d.getUTCDay(),
  };
}

/** `HH:MM:SS` in IST. */
export function istTime(ms) {
  const t = istParts(ms);
  return `${pad2(t.h)}:${pad2(t.mi)}:${pad2(t.s)}`;
}

/** `DD-MM HH:MM:SS` in IST. */
export function istDateTime(ms) {
  const t = istParts(ms);
  return `${pad2(t.day)}-${pad2(t.mo)} ${pad2(t.h)}:${pad2(t.mi)}:${pad2(t.s)}`;
}

/** `DD-MM-YYYY` in IST. */
export function istDate(ms) {
  const t = istParts(ms);
  return `${pad2(t.day)}-${pad2(t.mo)}-${t.y}`;
}
