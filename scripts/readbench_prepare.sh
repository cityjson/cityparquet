#!/usr/bin/env bash
# Prepare the per-format inputs for the read-benchmark milestone: given one
# CityJSON/CityJSONSeq INPUT, produce a same-content package in each format
# the read benchmark compares (`just readbench-prepare`). Which formats are
# built is chosen with `--formats`; by default every artefact this script
# knows how to build:
#
#   cityparquet          OUTDIR/<x>.parquet/          core-profile CityParquet package (source order)
#   cityparquet-hilbert  OUTDIR/<x>-hilbert.parquet/  CityParquet package, Hilbert-ordered rows
#   flatcitybuf          OUTDIR/<x>.fcb               FlatCityBuf, spatial index + ALL-attribute B+Tree index
#   cityjsonseq-gz       OUTDIR/<x>.jsonl.gz          the original input, gzip -9
#
# `cityjsonseq` is also a valid request and is a no-op: that format's
# "artefact" is INPUT itself, which the benchmark reads in place.
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
# EXTERNAL TOOLS ARE GUARDED PER FORMAT, not up front: `fcb` is only required
# when `flatcitybuf` was requested, and the release CityParquet CLI is only
# built when a CityParquet artefact was requested. A missing `fcb` used to
# kill every run of this script, including runs that never asked for
# FlatCityBuf.
#
# A REQUESTED FORMAT THAT CANNOT BE BUILT IS AN ERROR HERE, never a silent
# skip: this script's job is to build what was asked for or say precisely why
# it could not. Skipping-and-continuing is the coordinator's job
# (`crates/cityparquet-readbench/src/coordinator.rs` warns about a missing
# artefact and carries on with the rest of the matrix).
#
# Idempotent: an output that already exists and passes its validity check
# (non-empty file / non-empty directory) is skipped, so a re-run only fills
# in what is missing. This is a local dev tool; like the fetch_*.sh scripts,
# it is NOT wired into `just check`/CI. Its own tests live in
# `scripts/tests/readbench_prepare_test.sh`.
set -euo pipefail

# The format vocabulary, in the benchmark's canonical order. Owned by
# `Format::ALL` in `crates/cityparquet-readbench/src/format.rs`; this is a
# copy, because a shell script cannot import a Rust enum. Keep it on ONE line
# and in that order: `scripts/tests/readbench_prepare_test.sh` reads both
# lists out of their own sources and fails if they disagree (a duplicated
# vocabulary drifting apart is exactly how this benchmark's CSV header
# contract ended up with three incompatible versions).
#
# `duckdb-parquet` is deliberately absent: it is an SQL-engine baseline that
# `scripts/readbench_duckdb.sh` runs over an already-prepared CityParquet
# package, so there is no artefact for this script to build (the Rust side
# says the same with `Artefact::NotCoordinated`).
VALID_FORMATS=(citygml cityjson cityjsonseq cityjsonseq-gz flatcitybuf cityparquet cityparquet-hilbert)

# What `--formats` defaults to: everything this script can currently produce.
# NOT the same list as the coordinator's `DEFAULT_FORMATS` (which is what the
# benchmark MEASURES by default) — this one can only ever name formats a
# build step below exists for, so that a bare run never fails on its own
# default.
DEFAULT_BUILD_FORMATS=(cityparquet cityparquet-hilbert flatcitybuf cityjsonseq-gz)

usage() {
  cat >&2 <<EOF
usage: $0 [--formats a,b,c] INPUT [OUTDIR]
  INPUT      CityJSON (.city.json) or CityJSONSeq (.city.jsonl) file
  OUTDIR     default: bench/data/readbench
  --formats  comma-separated formats to build, from: ${VALID_FORMATS[*]}
             (default: ${DEFAULT_BUILD_FORMATS[*]})
EOF
}

FORMATS_ARG=""
POSITIONAL=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --formats)
      [[ $# -ge 2 ]] || { echo "error: --formats requires a value" >&2; exit 1; }
      FORMATS_ARG=$2
      shift 2
      ;;
    --formats=*)
      FORMATS_ARG=${1#--formats=}
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown option '$1'" >&2
      usage
      exit 1
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

if [[ ${#POSITIONAL[@]} -lt 1 || ${#POSITIONAL[@]} -gt 2 ]]; then
  usage
  exit 1
fi

INPUT=${POSITIONAL[0]}
OUTDIR=${POSITIONAL[1]:-bench/data/readbench}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -f "$INPUT" ]]; then
  echo "error: input file not found: $INPUT" >&2
  exit 1
fi

# Requested formats, in the order given; validated against VALID_FORMATS
# before anything is built, so a typo costs nothing.
REQUESTED=()
if [[ -n "$FORMATS_ARG" ]]; then
  IFS=',' read -r -a REQUESTED <<<"$FORMATS_ARG"
else
  REQUESTED=("${DEFAULT_BUILD_FORMATS[@]}")
fi

is_valid_format() {
  local candidate=$1 known
  for known in "${VALID_FORMATS[@]}"; do
    if [[ "$known" == "$candidate" ]]; then
      return 0
    fi
  done
  return 1
}

for fmt in "${REQUESTED[@]}"; do
  if [[ -z "$fmt" ]]; then
    echo "error: empty format name in --formats '$FORMATS_ARG'" >&2
    exit 1
  fi
  if ! is_valid_format "$fmt"; then
    echo "error: unknown format '$fmt'; expected one of: ${VALID_FORMATS[*]}" >&2
    exit 1
  fi
done

# Was FORMAT requested?
want() {
  local candidate=$1 fmt
  for fmt in "${REQUESTED[@]}"; do
    if [[ "$fmt" == "$candidate" ]]; then
      return 0
    fi
  done
  return 1
}

require_tool() {
  local tool=$1 reason=$2
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: $tool not found on PATH (needed for $reason)" >&2
    exit 1
  fi
}

# --- preflight -------------------------------------------------------------
# One pass over the request: reject what cannot be built, and require only
# the tools the request actually needs. Everything below this point is
# expected to succeed.

# Formats the read benchmark knows but this script cannot yet produce. Task 7
# adds the `citygml`/`cityjson` build steps and empties this list.
for fmt in citygml cityjson; do
  if want "$fmt"; then
    echo "error: format '$fmt' is a valid read-benchmark format, but this script has" >&2
    echo "       no build step for it yet; drop it from --formats" >&2
    exit 1
  fi
done

NEED_CLI=0
if want cityparquet || want cityparquet-hilbert; then
  NEED_CLI=1
fi
if want flatcitybuf; then
  require_tool fcb "the FlatCityBuf artefact"
fi
if want cityjsonseq-gz; then
  require_tool gzip "the gzipped CityJSONSeq artefact"
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
# exact member set depends on how many 1st-level CityObject families INPUT
# has, so this doesn't hardcode filenames).
dir_is_valid() {
  [[ -d "$1" ]] && [[ -n "$(find "$1" -type f -print -quit)" ]]
}

# Non-empty regular file.
file_is_valid() {
  [[ -f "$1" ]] && [[ -s "$1" ]]
}

echo "== readbench_prepare: $INPUT -> $OUTDIR (base: $BASE) =="
echo "-- formats: ${REQUESTED[*]}"

# Build the release CLI once, up front, so none of the per-artefact steps
# below pay a `cargo run` recompile-check cost. Only when a format that needs
# it was requested.
CITYPARQUET=""
if [[ "$NEED_CLI" -eq 1 ]]; then
  echo "-- building release CLI (cargo build --release -p cityparquet-cli)"
  ( cd "$REPO_ROOT" && cargo build --release -p cityparquet-cli )
  CITYPARQUET="$REPO_ROOT/target/release/cityparquet"
  if [[ ! -x "$CITYPARQUET" ]]; then
    echo "error: expected binary not found after build: $CITYPARQUET" >&2
    exit 1
  fi
fi

# Artefacts this run is responsible for, in build order, for the closing
# summary.
BUILT=()

# 1. Core-profile CityParquet package (source row order).
if want cityparquet; then
  if dir_is_valid "$PARQUET_OUT"; then
    echo "skip $PARQUET_OUT (already present)"
  else
    echo "-- convert -> $PARQUET_OUT"
    # By-type is the only, mandatory table layout (2026-07-21): one
    # `<snake>.parquet` table per 1st-level CityObject family. The
    # read-benchmark's CityParquetRunner only supports a package whose
    # manifest lists exactly one table, so INPUT for this script must be a
    # single-family dataset (e.g. a Building-only 3D BAG tile) — a
    # multi-family INPUT prepares fine here but the read-benchmark itself
    # rejects it later with a clear error.
    "$CITYPARQUET" convert "$INPUT" -o "$PARQUET_OUT" --overwrite
  fi
  BUILT+=("$PARQUET_OUT")
fi

# 2. Hilbert-ordered CityParquet package.
if want cityparquet-hilbert; then
  if dir_is_valid "$HILBERT_OUT"; then
    echo "skip $HILBERT_OUT (already present)"
  else
    echo "-- convert --ordering hilbert -> $HILBERT_OUT"
    "$CITYPARQUET" convert "$INPUT" -o "$HILBERT_OUT" --ordering hilbert --overwrite
  fi
  BUILT+=("$HILBERT_OUT")
fi

# 3. FlatCityBuf, spatial index (default-on) + all-attribute B+Tree index.
if want flatcitybuf; then
  if file_is_valid "$FCB_OUT"; then
    echo "skip $FCB_OUT (already present)"
  else
    echo "-- fcb ser -> $FCB_OUT"
    fcb ser -i "$INPUT" -o "$FCB_OUT" -A
  fi
  BUILT+=("$FCB_OUT")
fi

# 4. Gzip of the original input, for a whole-document-gzip baseline.
if want cityjsonseq-gz; then
  if file_is_valid "$GZ_OUT"; then
    echo "skip $GZ_OUT (already present)"
  else
    echo "-- gzip -9 -> $GZ_OUT"
    gzip -9 -c "$INPUT" > "$GZ_OUT"
  fi
  BUILT+=("$GZ_OUT")
fi

# 5. Plain CityJSONSeq needs no artefact at all: the benchmark reads INPUT
# itself (`Artefact::TheInputItself`). Requesting it is a no-op, NOT a
# "cannot build" error.
if want cityjsonseq; then
  echo "-- cityjsonseq: no artefact needed (the benchmark reads $INPUT in place)"
fi

# Sanity checks: every artefact this run was responsible for exists and is
# non-empty, and — when FlatCityBuf was built — the FCB file reports a
# positive feature count via `fcb info` (fcb prints a "Features: N" line
# under "Dataset"; N need not equal cityparquet's object_count, since FCB
# counts top-level features while cityparquet's object_count includes
# descendant CityObjects such as BuildingParts).
echo "-- verifying artefacts"
if want cityparquet; then
  dir_is_valid "$PARQUET_OUT" || { echo "error: missing/empty package: $PARQUET_OUT" >&2; exit 1; }
fi
if want cityparquet-hilbert; then
  dir_is_valid "$HILBERT_OUT" || { echo "error: missing/empty package: $HILBERT_OUT" >&2; exit 1; }
fi
if want cityjsonseq-gz; then
  file_is_valid "$GZ_OUT" || { echo "error: missing/empty file: $GZ_OUT" >&2; exit 1; }
fi
if want flatcitybuf; then
  file_is_valid "$FCB_OUT" || { echo "error: missing/empty file: $FCB_OUT" >&2; exit 1; }

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
fi

if [[ ${#BUILT[@]} -eq 0 ]]; then
  echo "readbench_prepare complete: nothing to build for ${REQUESTED[*]}"
else
  SUMMARY=""
  for out in "${BUILT[@]}"; do
    SUMMARY+="${SUMMARY:+, }$out"
  done
  echo "readbench_prepare complete: $SUMMARY"
fi
