#!/usr/bin/env bash
# Bulk-encode a list of 3DBAG tiles into CityParquet packages, one tile at a
# time, so you have many separate .cityparquet packages to query against.
#
# For every URL in TILES_FILE (one .city.json.gz URL per line) this:
#   1. downloads the gzipped whole-document CityJSON tile,
#   2. gunzips it to a local .city.json,
#   3. runs `cityparquet convert` into OUT_DIR/<tile-id>/ (core profile),
#   4. (unless KEEP_JSON=1) deletes the intermediate .city.json to save disk.
#
# Idempotent: a tile whose OUT_DIR/<tile-id>/ package already exists is
# skipped. Fault-tolerant: a tile that fails to download/convert is logged
# and the run continues, so one bad tile does not abort the whole batch; a
# per-tile pass/fail/skip tally is printed at the end and the script exits
# non-zero if any tile failed.
#
# Usage:
#   scripts/encode_3dbag_tiles.sh [TILES_FILE] [OUT_DIR] [COUNT]
#
#   COUNT   how many URLs to process this run (0/omitted = all). Combine with
#           START to process the list in batches, e.g. lines 201..300:
#             START=201 scripts/encode_3dbag_tiles.sh '' '' 100
#
# Env knobs (equivalent to / on top of the positional args):
#   TILES_FILE=PATH    same as the 1st positional (input URL list)
#   OUT_DIR=DIR        same as the 2nd positional (where packages are written)
#   LIMIT=N            same as the COUNT positional (how many tiles to process)
#   START=N            1-based line number to begin at (default 1); lines
#                      before START are not counted toward COUNT/LIMIT
#   KEEP_JSON=1        keep the decompressed .city.json alongside the package
#   CONVERT_ARGS="..." extra flags passed through to `cityparquet convert`
#                      (e.g. CONVERT_ARGS="--ordering hilbert --layout single")
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Each of TILES_FILE / OUT_DIR: positional arg wins, else same-named env var,
# else the default under bench/data/.
TILES_FILE="${1:-${TILES_FILE:-$ROOT/bench/data/3dbag_tiles.txt}}"
OUT_DIR="${2:-${OUT_DIR:-$ROOT/bench/data/3dbag_cityparquet}}"
# COUNT: 3rd positional wins; else LIMIT env; else 0 (= all).
LIMIT="${3:-${LIMIT:-0}}"
START="${START:-1}"            # 1-based line to begin at
JSON_DIR="$OUT_DIR/_src"        # scratch dir for downloaded/decompressed inputs
KEEP_JSON="${KEEP_JSON:-0}"
CONVERT_ARGS="${CONVERT_ARGS:-}"

[[ -f "$TILES_FILE" ]] || { echo "error: tiles file not found: $TILES_FILE" >&2; exit 1; }
mkdir -p "$OUT_DIR" "$JSON_DIR"

# Build the CLI once up front so the per-tile loop is a plain binary call and
# the timing/output isn't polluted by cargo's build chatter.
echo ">> building cityparquet CLI (release) ..."
cargo build --release -q -p cityparquet-cli --bin cityparquet
CLI="$ROOT/target/release/cityparquet"

pass=0 fail=0 skip=0 n=0 line=0
failed_tiles=()

# `read` returns non-zero on the final line when it lacks a trailing newline
# (this file's last line does); `|| [[ -n "$url" ]]` keeps that line.
while IFS= read -r url || [[ -n "$url" ]]; do
  url="${url%%[[:space:]]}"                 # trim any trailing whitespace/CR
  [[ -z "$url" ]] && continue               # skip blank lines
  [[ "$url" == \#* ]] && continue           # skip comment lines

  line=$((line + 1))
  [[ "$line" -lt "$START" ]] && continue    # not yet at the START line

  if [[ "$LIMIT" -gt 0 && "$n" -ge "$LIMIT" ]]; then
    break                                   # processed COUNT tiles already
  fi
  n=$((n + 1))

  base="$(basename "$url")"                 # e.g. 8-760-72.city.json.gz
  tile="${base%.city.json.gz}"              # e.g. 8-760-72
  dest="$OUT_DIR/$tile"

  if [[ -d "$dest" ]]; then
    echo "[line $line] skip $tile (package already exists: $dest)"
    skip=$((skip + 1))
    continue
  fi

  gz="$JSON_DIR/$tile.city.json.gz"
  json="$JSON_DIR/$tile.city.json"
  echo "[line $line] fetch $tile <- $url"

  if ! curl -fsS -o "$gz" "$url"; then
    echo "    ! download failed: $tile" >&2
    rm -f "$gz"
    fail=$((fail + 1)); failed_tiles+=("$tile")
    continue
  fi
  if ! gunzip -f "$gz"; then                # -> $json
    echo "    ! gunzip failed: $tile" >&2
    rm -f "$gz" "$json"
    fail=$((fail + 1)); failed_tiles+=("$tile")
    continue
  fi

  echo "    encode -> $dest"
  # shellcheck disable=SC2086  # CONVERT_ARGS is intentionally word-split
  if "$CLI" convert "$json" "$dest" --overwrite $CONVERT_ARGS; then
    pass=$((pass + 1))
  else
    echo "    ! convert failed: $tile" >&2
    rm -rf "$dest"
    fail=$((fail + 1)); failed_tiles+=("$tile")
  fi

  [[ "$KEEP_JSON" == "1" ]] || rm -f "$json"
done < "$TILES_FILE"

# Clean up the scratch dir if we didn't keep any inputs and it's now empty.
[[ "$KEEP_JSON" == "1" ]] || rmdir "$JSON_DIR" 2>/dev/null || true

echo
echo "done: $pass encoded, $skip skipped, $fail failed (of $n processed)"
echo "packages in: $OUT_DIR"
if [[ "$fail" -gt 0 ]]; then
  printf '  failed: %s\n' "${failed_tiles[*]}" >&2
  exit 1
fi
