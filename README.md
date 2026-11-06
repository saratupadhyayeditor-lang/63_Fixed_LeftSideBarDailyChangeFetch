# Algo Trading (Rust)

Native desktop algo-trading app for Indian markets (NSE / BSE / MCX) on the Dhan
broker API. One Rust server serves the whole app (all tabs, REST API, live
WebSocket feed) and a thin native shell wraps it in a desktop window
(WebView2 on Windows).

Version: **0.1.32**

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

## Version 0.1.32 changes

- **MCX commodities section stuck on `-` fixed.** The old bundled MCX list used
  made-up ids (`500001..500015`) that are not Dhan contracts at all, so their
  quotes never resolved and the sidebar showed no daily change. On a fresh
  install the app also served that list during the scrip-master warm-up window
  and the UI cached it permanently. The static fallback is gone: while the
  master downloads the API now returns an empty list with `warming: true`, and
  the sidebar retries and replaces the rows once real contract ids arrive
  (CRUDEOIL/COPPER/... with their current expiry ids).
- Scrip-master warm-up now retries with backoff instead of giving up forever
  after three failed downloads.
- Saved commodity picks survive a restart that starts before the master is
  ready, then get reconciled (stale/rolled/futures-only ids dropped) once it is.
- Every engine tab's commodity picker now repopulates when the real list
  arrives instead of staying empty until the next reload.

## Version 0.1.31 changes

- **Run a strategy on every selected strike via that strike's own premium
  chart.** In premium-only mode each selected strike now resolves its individual
  option chart and must pass the indicator filters on its own; a failing strike
  no longer drags a passing sibling. `strikeCount` is honoured (previously only
  one strike resolved).
- **Option type (CE / PE) now restricts the scanner universe** to that side
  instead of being ignored. It is a filter, not a conversion: a bearish stock is
  not turned into a call.
- **One-side-per-underlying guard fixed:** it now blocks the opposite-side entry
  instead of force-closing the running leg. The old `SIDE_FLIP` close path was
  booking avoidable losses on every side wobble.
- Option chain segment and scrip resolution fixes.

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
