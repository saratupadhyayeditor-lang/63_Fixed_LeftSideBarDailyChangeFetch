# Windows build (v0.1.32)

Pre-built Windows x86_64 binaries for this repo, built from the exact source in
this commit. Everything needed to install and run on Windows is here.

Build: cross-compiled from Linux with `x86_64-pc-windows-gnu` (mingw-w64), so
the executables depend only on standard Windows system DLLs plus
`WebView2Loader.dll` (bundled below) - no VC++ runtime or admin needed.

Headline of this build: **the MCX commodities section no longer stuck on `-`.**
The old bundled MCX list used made-up ids (`500001..500015`) that are not Dhan
contracts at all, so their quotes never resolved and the sidebar showed no
daily change. On a fresh install that list was also served during the
scrip-master warm-up window and the UI cached it forever. The static fallback is
gone: while the master downloads the API returns an empty list with
`warming: true`, and the sidebar retries and replaces the rows once real
contract ids arrive (CRUDEOIL/COPPER/... with their current expiry ids). Also in
this build:

- Scrip-master warm-up now retries with backoff instead of giving up forever
  after three failed downloads.
- Saved commodity picks survive a restart that starts before the master is ready,
  then get reconciled (stale/rolled/futures-only ids dropped) once it is.
- Every engine tab's commodity picker repopulates when the real list arrives.

Still included from v0.1.31: run a strategy on all selected strikes via each
strike's own premium chart; option type (CE/PE) restricts the scanner universe
instead of being ignored; the one-side-per-underlying guard now blocks the
opposite-side entry instead of force-closing the running leg; and the option
chain segment / scrip resolution fixes.

Still included from earlier versions: the "Trend check before entry" gate, the
AI Trader (Paper + Live) autonomous brain engines, the Auto Experiment engine,
the Market Analysis refresh fix, the anti-fake entry gate, the liquid F&O
scanner and bid/ask paper fills.

## What is in this folder

| File | Purpose |
|------|---------|
| `AlgoTradingSetup.exe` | **Recommended.** Per-user NSIS installer (no admin/UAC). Installs to `%LOCALAPPDATA%\AlgoTrading`, adds Start Menu + Desktop shortcuts and an uninstall entry. |
| `algo-desktop-windows-x86_64-portable.exe` | Single-file portable launcher. Double-click; it self-extracts once and opens the app. No install, no admin. |
| `algo-desktop-windows-x86_64.zip` | Portable folder build: `algo-desktop.exe` + `algo-server.exe` + `WebView2Loader.dll` + `static/`. Unzip and run `algo-desktop.exe`. |
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

No installer / no admin alternative: run
`algo-desktop-windows-x86_64-portable.exe` directly.

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
