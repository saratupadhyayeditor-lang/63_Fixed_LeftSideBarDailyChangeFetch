# Algo Trading (Rust)

Native desktop algo-trading app for Indian markets (NSE / BSE / MCX) on the Dhan
broker API. One Rust server serves the whole app (all tabs, REST API, live
WebSocket feed) and a thin native shell wraps it in a desktop window
(WebView2 on Windows).

Version: **0.1.30**

## What is in this repo

| Path | Purpose |
|------|---------|
| `crates/core` | Indicators, candle/option models, backtest engines |
| `crates/dhan` | Dhan API client + live market feed (WebSocket) |
| `crates/server` | `algo-server`: REST API, engines, static UI |
| `crates/web` | Shared web helpers |
| `desktop` | `algo-desktop`: native window shell + updater (own workspace) |
| `crates/server/static` | The UI (HTML/CSS/JS) served by the server |
| `backup/windows` | Pre-built Windows installers, per version |
| `dist`, `dist-windows` | Local build output (git-ignored) |

## Features

- **Realtime Trading Engine** (live Dhan orders) with Top Movers, Liquid F&O
  Stocks, NIFTY trend following, indicator-filter gating and AI Brain scoring.
- **Paper Trade 1/2/3** - same engines on simulated fills, separate books
  (full, indices-only, commodities-only).
- **Realtime 2/3** - index + commodity variants of the live engine.
- **Equity Research** - free-source multibagger screener across NSE + BSE
  (bhavcopy, delivery %, Yahoo fundamentals, shareholding, F&O option metrics)
  with a selectable full-market scan.
- **Equity Paper / Realtime Equity engines** - delivery buy calls from the
  research screen.
- **Shared margin pool** - one total for the app, per-tab allotments.
- Multiple order-placement methods, trade sessions, risk/target/stop management,
  strict target plan, backups and exports.

## Run from source

```bash
cargo build --release -p algo-server
./target/release/algo-server          # serves on http://localhost:8080
```

Open `http://localhost:8080`. Set `ALGODHAN_DATA_DIR` to keep engine state in a
specific folder (defaults to `./data`).

Desktop shell (development):

```bash
cargo build -p algo-server
cargo run --manifest-path desktop/Cargo.toml
```

## Windows build

Cross-compile from Linux with the GNU toolchain, then package with NSIS:

```bash
# Server + desktop shell
cargo build --release --target x86_64-pc-windows-gnu -p algo-server
cargo build --release --target x86_64-pc-windows-gnu --manifest-path desktop/Cargo.toml

# Stage the installer payload
mkdir -p dist-windows/static
cp target/x86_64-pc-windows-gnu/release/algo-server.exe dist-windows/
cp desktop/target/x86_64-pc-windows-gnu/release/algo-desktop.exe dist-windows/
tar -C crates/server/static --exclude=./AlgoTradingSetup.exe -cf - . | tar -C dist-windows/static -xf -

# Build AlgoTradingSetup.exe
makensis -DSTAGE="$PWD/dist-windows" -DOUTDIR="$PWD/dist-windows" desktop/scripts/installer-windows.nsi
```

Pre-built installers live in `backup/windows/<version>/` - see
`backup/windows/README.md`.

## Notes

- `crates/server/static` ships precompressed `*.gz` copies used by the server's
  `precompressed_gzip()` static handler. **After editing any file in `static/`,
  regenerate them** or browsers keep getting the old content (gzips win over the
  plain file when the client sends `Accept-Encoding: gzip`).
- Runtime state (`data/`) and build output (`target/`, `dist*/`) are git-ignored.

## Version 0.1.30 changes

- Paper Trade 1: shared-margin seeding fixed, so auto-trading executes right
  after a fresh install instead of being blocked by a zero paper pool.
- New **Liquid F&O Stocks** scanner (Paper Trade 1 + Realtime): ranks only the
  liquid index basket (NIFTY 50 / BANK NIFTY / SENSEX / FINNIFTY / MIDCAP NIFTY)
  with top-gainer -> CE and top-loser -> PE, explicit gainer/loser pickers and a
  manual add list. Fixes a catalog id collision where an index id could resolve a
  stock pick as an index option.
- Equity Research: new **Scan all listed stocks** checkbox - OFF keeps the
  liquidity screen + 900-stock sample (top 300 rows), ON lifts the liquidity
  screen, the 900 cap and the 300-row truncation to cover the whole listed
  market. Shows an ACTIVE / INACTIVE badge.
