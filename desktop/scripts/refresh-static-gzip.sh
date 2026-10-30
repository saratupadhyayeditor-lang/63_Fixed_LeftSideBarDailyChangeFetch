#!/usr/bin/env bash
# Regenerate the precompressed *.gz copies of crates/server/static.
#
# The server serves static files through `ServeDir::precompressed_gzip()`, so a
# client that sends `Accept-Encoding: gzip` gets `<file>.gz` whenever it exists.
# Editing a source file without refreshing its .gz makes the server (and any CDN
# in front of it) keep serving the OLD content - the classic "my change is not
# visible" trap. Run this after every static/ edit, before packaging.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STATIC="$ROOT/crates/server/static"

python3 - "$STATIC" <<'PY'
import gzip, os, sys

static = sys.argv[1]
regen = stale = 0
for name in sorted(os.listdir(static)):
    if not name.endswith(".gz"):
        continue
    gz = os.path.join(static, name)
    src = gz[:-3]
    if not os.path.isfile(src):
        print("skip (no source):", name)
        continue
    if os.path.getmtime(src) <= os.path.getmtime(gz):
        continue
    with open(src, "rb") as f:
        data = f.read()
    with gzip.open(gz, "wb", compresslevel=9) as g:
        g.write(data)
    regen += 1

print(f"regenerated {regen} stale .gz file(s)")
PY
