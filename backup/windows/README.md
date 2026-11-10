# Windows build (Left Sidebar Daily-Change fix)

Pre-built Windows x86_64 binaries for this repo, built from the exact source in
this commit. Everything needed to install and run on Windows is here.

Build: cross-compiled from Linux with `x86_64-pc-windows-gnu` (mingw-w64), so
the executables depend only on standard Windows system DLLs plus
`WebView2Loader.dll` (bundled below) - no VC++ runtime or admin needed.

Headline of this build: **the left sidebar's daily-change column now fills fast
and together on reload, instead of trickling in one row at a time (or staying
blank for F&O stocks / indices / commodities).**

What was wrong and what changed:

- **Blank rows with a close but no price.** Dhan's PrevClose packet fills
  `close` without a last-traded price, and the historical backfill skipped any
  row that already had a close. Those rows stayed blank forever even though the
  close was known. The backfill now requires BOTH a close and an LTP before it
  skips a row, so the missing price is filled from history / the REST quote.
- **Prev-close snapshot now carries the price.** The on-disk snapshot used to
  store only previous closes, so a same-day reload could not paint the change
  (change = ltp - close). It now persists each row's last traded price too, and
  republishes those rows on startup. Restart now restores the sidebar instantly
  with no Dhan call.
- **Rate-limit starvation fixed.** Dhan answers a CLOSED cash / F&O segment's
  `/marketfeed/quote` with an empty payload yet still spends the shared
  ~1 request/second data slot. Three callers kept firing it overnight - the
  browser `/api/quotes` poll, the scanner quote fallback and the 2-second
  positions poller - so the adaptive gate sat in a constant 429 hold and one
  historical candle took ~19s instead of ~0.3s. Now every quote path is gated by
  `segment_session_open()` (cash/F&O/index 09:15-15:30 IST, MCX to 23:30), and
  the positions poller backs off to 55s outside session hours (urgent
  order-triggered refreshes still run immediately). Verified overnight: 0 rate
  limits, backfill ~4-5 rows/30s (was ~1/min).

Still included from the previous build: the per-strike premium gate now forces
the rising-premium (bullish) filter set for BOTH legs so option entries never
fire against the trend; per-engine WebSocket snapshot push replaces the 1s REST
live-display poll; and the MCX commodities section resolves real contract ids
instead of made-up ones.

## What is in this folder

| File | Purpose |
|------|---------|
| `AlgoTradingSetup.exe` | **Recommended.** Per-user NSIS installer (no admin/UAC). Installs to `%LOCALAPPDATA%\AlgoTrading`, adds Start Menu + Desktop shortcuts and an uninstall entry. |
| `algo-desktop-windows-x86_64.zip` | Portable folder build: `algo-desktop.exe` + `algo-server.exe` + `WebView2Loader.dll` + `static/`. Unzip and run `algo-desktop.exe`. |
| `algo-desktop-windows-x86_64-portable.exe` | Single-file portable launcher from an EARLIER build (kept for convenience). For this build prefer the installer or the zip above. |
| `algo-server.exe` | Backend server (serves the UI + all APIs). |
| `algo-desktop.exe` | Native window shell (WebView2). Starts `algo-server.exe` and opens the app. |
| `WebView2Loader.dll` | Required next to the shell; must ship with `algo-desktop.exe`. |
| `static/` | The full UI served by the server. |
| `SHA256SUMS` | SHA-256 of every artifact above. |

## Install on Windows

1. Copy `AlgoTradingSetup.exe` to the Windows machine.
2. Run it. Windows SmartScreen may warn because the binary is unsigned - choose
   *More info → Run anyway*.
3. Finish the wizard and let it launch the app.

No installer / no admin alternative: unzip
`algo-desktop-windows-x86_64.zip` and run `algo-desktop.exe`.

## Notes

- WebView2 runtime is required (pre-installed on current Windows 10/11). If it is
  missing, install the Microsoft Edge WebView2 Runtime.
- Runtime data (engine state, settings) is written to a `data/` folder next to
  the app / in the user data dir; keep it for persistence, delete it for a clean
  slate.
- `WebView2Loader.dll` must stay in the same folder as `algo-desktop.exe`.

## Verifying a download

```powershell
Get-FileHash .\AlgoTradingSetup.exe -Algorithm SHA256
```

Compare the output with the matching line in `SHA256SUMS`.

## Rebuilding from source

Cross-compile from Linux:

```bash
cargo build --release --target x86_64-pc-windows-gnu -p algo-server
cargo build --release --target x86_64-pc-windows-gnu --manifest-path desktop/Cargo.toml
makensis -DSTAGE="$PWD/dist-windows" -DOUTDIR="$PWD/dist-windows" \
  desktop/scripts/installer-windows.nsi
```

Or on Windows, use `desktop/scripts/build-desktop.ps1`.
