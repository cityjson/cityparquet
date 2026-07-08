#!/usr/bin/env bash
# Prepare the per-format inputs for the read-benchmark milestone: given one
# CityJSON/CityJSONSeq INPUT, produce a same-content package in every format
# the read benchmark compares (`just readbench-prepare`):
#
#   OUTDIR/<x>.parquet/          core-profile CityParquet package (source order)
#   OUTDIR/<x>-hilbert.parquet/  CityParquet package, Hilbert-ordered rows
#   OUTDIR/<x>.fcb               FlatCityBuf, spatial index + ALL-attribute B+Tree index
#   OUTDIR/<x>.jsonl.gz          the original input, gzip -9
#
# `<x>` is INPUT's basename minus its .city.jsonl/.city.json/.jsonl/.json
# extension (same stripping rule as the justfile's bench-fixtures/convert-all
# recipes).
#
# The `-A` (index-all-attributes) flag on `fcb ser` is REQUIRED, not
# cosmetic: the later attribute-filter benchmark needs FCB's B+-tree
# attribute index to exist, and the spatial (R-tree) index is on by default
# so both of FCB's indexed-query paths are available for comparison.
#
# Idempotent: an output that already exists and passes its validity check
# (non-empty file / non-empty directory) is skipped, so a re-run only fills
# in what is missing. This is a local dev tool (needs the `fcb` CLI on
# PATH); like the fetch_*.sh scripts, it is NOT wired into `just check`/CI.
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 INPUT [OUTDIR]" >&2
  echo "  INPUT   CityJSON (.city.json) or CityJSONSeq (.city.jsonl) file" >&2
  echo "  OUTDIR  default: bench/data/readbench" >&2
  exit 1
fi

INPUT=$1
OUTDIR=${2:-bench/data/readbench}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -f "$INPUT" ]]; then
  echo "error: input file not found: $INPUT" >&2
  exit 1
fi

if ! command -v fcb >/dev/null 2>&1; then
  echo "error: fcb not found on PATH (needed for the FlatCityBuf artefact)" >&2
  exit 1
fi

mkdir -p "$OUTDIR"

BASE="$(basename "$INPUT")"
BASE="${BASE%.city.jsonl}"; BASE="${BASE%.city.json}"
BASE="${BASE%.jsonl}"; BASE="${BASE%.json}"

PARQUET_OUT="$OUTDIR/${BASE}.parquet"
HILBERT_OUT="$OUTDIR/${BASE}-hilbert.parquet"
FCB_OUT="$OUTDIR/${BASE}.fcb"
GZ_OUT="$OUTDIR/${BASE}.jsonl.gz"

# Non-empty directory: at least one file inside (a CityParquet package is
# always a directory of one or more Parquet files + metadata.json; the
# exact member set depends on --layout, so this doesn't hardcode filenames).
dir_is_valid() {
  [[ -d "$1" ]] && [[ -n "$(find "$1" -type f -print -quit)" ]]
}

# Non-empty regular file.
file_is_valid() {
  [[ -f "$1" ]] && [[ -s "$1" ]]
}

echo "== readbench_prepare: $INPUT -> $OUTDIR (base: $BASE) =="

# Build the release CLI once, up front, so none of the per-artefact steps
# below pay a `cargo run` recompile-check cost.
echo "-- building release CLI (cargo build --release -p cityparquet-cli)"
( cd "$REPO_ROOT" && cargo build --release -p cityparquet-cli )
CITYPARQUET="$REPO_ROOT/target/release/cityparquet"
if [[ ! -x "$CITYPARQUET" ]]; then
  echo "error: expected binary not found after build: $CITYPARQUET" >&2
  exit 1
fi

# 1. Core-profile CityParquet package (source row order).
if dir_is_valid "$PARQUET_OUT"; then
  echo "skip $PARQUET_OUT (already present)"
else
  echo "-- convert -> $PARQUET_OUT"
  "$CITYPARQUET" convert "$INPUT" "$PARQUET_OUT" --overwrite
fi

# 2. Hilbert-ordered CityParquet package.
if dir_is_valid "$HILBERT_OUT"; then
  echo "skip $HILBERT_OUT (already present)"
else
  echo "-- convert --ordering hilbert -> $HILBERT_OUT"
  "$CITYPARQUET" convert "$INPUT" "$HILBERT_OUT" --ordering hilbert --overwrite
fi

# 3. FlatCityBuf, spatial index (default-on) + all-attribute B+Tree index.
if file_is_valid "$FCB_OUT"; then
  echo "skip $FCB_OUT (already present)"
else
  echo "-- fcb ser -> $FCB_OUT"
  fcb ser -i "$INPUT" -o "$FCB_OUT" -A
fi

# 4. Gzip of the original input, for a whole-document-gzip baseline.
if file_is_valid "$GZ_OUT"; then
  echo "skip $GZ_OUT (already present)"
else
  echo "-- gzip -9 -> $GZ_OUT"
  gzip -9 -c "$INPUT" > "$GZ_OUT"
fi

# Sanity checks: every artefact exists and is non-empty, and the FCB file
# reports a positive feature count via `fcb info` (fcb prints a "Features:
# N" line under "Dataset"; N need not equal cityparquet's object_count,
# since FCB counts top-level features while cityparquet's object_count
# includes descendant CityObjects such as BuildingParts).
echo "-- verifying artefacts"
for out in "$PARQUET_OUT" "$HILBERT_OUT"; do
  dir_is_valid "$out" || { echo "error: missing/empty package: $out" >&2; exit 1; }
done
for out in "$FCB_OUT" "$GZ_OUT"; do
  file_is_valid "$out" || { echo "error: missing/empty file: $out" >&2; exit 1; }
done

FCB_INFO="$(fcb info -i "$FCB_OUT")"
FEATURES="$(echo "$FCB_INFO" | grep -E '^\s*Features:' | grep -oE '[0-9]+' | head -1)"
if [[ -z "$FEATURES" ]]; then
  echo "error: could not find a 'Features:' count in \`fcb info\` output for $FCB_OUT" >&2
  echo "$FCB_INFO" >&2
  exit 1
fi
if [[ "$FEATURES" -le 0 ]]; then
  echo "error: fcb info reports $FEATURES features for $FCB_OUT (expected > 0)" >&2
  exit 1
fi
echo "  fcb info: $FEATURES features in $FCB_OUT"

echo "readbench_prepare complete: $PARQUET_OUT, $HILBERT_OUT, $FCB_OUT, $GZ_OUT"
