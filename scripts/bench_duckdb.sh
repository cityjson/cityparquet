#!/usr/bin/env bash
# DuckDB `COPY` baseline for the cityparquet bench CSV (M5 task 7).
#
# This measures what an untunable, off-the-shelf writer gets you: the
# `cityjson` community DuckDB extension's `read_cityjson`/`read_cityjsonseq`
# table functions, piped straight through `COPY ... TO ... (FORMAT PARQUET)`.
# It appends two rows per invocation — `duckdb-copy` (SNAPPY, DuckDB's
# default) and `duckdb-copy-zstd` (ZSTD) — to the SAME CSV the
# `cityparquet bench` harness (Task 6) writes, using the identical header:
#   dataset,variant,object_count,write_s,total_bytes,cityobjects_bytes,
#   sidecar_bytes,full_scan_s,window_query_s,row_groups_total,
#   row_groups_touched,roundtrip_equal
#
# Verified real output schema (2026-07-07, cityjson community extension,
# DuckDB v1.5.3), via:
#   duckdb -c "LOAD cityjson; DESCRIBE SELECT * FROM read_cityjsonseq('tests/fixtures/delft.city.jsonl');"
#   duckdb -c "LOAD cityjson; DESCRIBE SELECT * FROM read_cityjson('tests/fixtures/lod3_railway.city.json');"
# Both variants (Seq and doc) share the same shape: one row per CityObject,
# with `id`, `feature_id`, `object_type`, `children`/`children_roles`/
# `parents` VARCHAR[] columns, an `other` catch-all VARCHAR, every CityGML
# attribute the source data carries as a plain scalar column (delft's
# BAG/3DBAG `b3_*` fields; railway's `class`/`function`/`species`), and one
# STRUCT(lod, "type", boundaries, semantics, material, texture) column per
# LoD present in the dataset (delft: geom_lod0/geom_lod1_2/geom_lod1_3/
# geom_lod2_2; railway: geom_lod/geom_lod3) — `boundaries`/`semantics` are
# VARCHAR (JSON-encoded text), not a typed/WKB geometry column.
#
# Row semantics: one row per CityObject, matching cityparquet's
# `object_count` exactly — verified against the known fixture counts
# (delft: 2231, from scripts/interop.sh; railway: 121, ditto) via
# `SELECT count(*) FROM read_cityjsonseq(...)` / `read_cityjson(...)`.
#
# window_query_s is left EMPTY: the schema above carries NO bbox-like
# column (no struct/scalar field named or shaped like an extent) on either
# fixture. A window query here would either force a full unfiltered scan
# (not a "window" query at all) or require deriving a synthetic bbox from
# `boundaries` JSON at query time (an apples-to-oranges comparison against
# cityparquet's native bbox-statistics row-group pruning). Fabricating a
# comparable query would misrepresent the baseline, so the field stays
# empty and this is the documented reason.
#
# full_scan_s: `SELECT count(*) FROM read_parquet(...)` was verified (via
# `EXPLAIN`) to compile to a bare COLUMN_DATA_SCAN over Parquet footer
# metadata — it never touches column data, at any repeat count, so it is
# not comparable to the harness's full_scan_s (which decodes every row of
# every column). `SELECT count(*) FROM (SELECT * FROM read_parquet(...))`
# collapses to the exact same metadata-only plan (DuckDB's projection
# pruning removes the wrapping subquery). The honest replacement verified
# here is `SELECT sum(hash(COLUMNS(*))) FROM read_parquet(...)`: `EXPLAIN`
# confirms this compiles to a READ_PARQUET node whose Projections list every
# column (id, feature_id, ..., every geom_lod* struct), i.e. it forces
# DuckDB to decode every column's data for every row — a genuine full scan,
# analogous in spirit to the harness's read-everything measurement, without
# fabricating semantics the extension does not have.
#
# row_groups_total/row_groups_touched: EMPTY. The baseline exposes no
# row-group-pruning API to query against (and there is no window query to
# prune for in the first place — see above).
#
# roundtrip_equal: EMPTY. The `cityjson` extension has no CityJSON *export*
# path (no COPY TO cityjson/cityjsonseq) — there is nothing to round-trip
# against. This asymmetry (a writer with no matching reader-back-out) is
# exactly the gap cityparquet-rs's own round-trip guarantee is contrasted
# against in the paper.
#
# write_s overhead DISCLOSURE: every timed sample launches a fresh `duckdb`
# process and must `LOAD cityjson` inside it, so each write_s sample
# carries a fixed process-bootstrap(+LOAD) cost the Rust harness (which
# times an in-process function call) does not pay — measured at ~0.08s per
# invocation on this machine (duckdb startup ~0.06s + LOAD ~0.02s), i.e.
# roughly 15-25% of a delft-sized write_s sample.
# `INSTALL cityjson FROM community` is hoisted OUT of the timed block and
# run once, untimed, before the codec loop (INSTALL is idempotent; only
# LOAD must recur per process). The residual per-invocation overhead is
# calibrated at script start — median of 3 of
# `"$DUCKDB" -c "LOAD cityjson; SELECT 1;"` — and reported on stderr as a
# `# calibration:` line. It is deliberately NOT subtracted from the
# reported write_s (disclosure over adjustment, consistent with the
# full_scan_s caveat above): subtract the calibration value downstream if
# a pure-COPY figure is needed. full_scan_s samples carry the same process
# bootstrap minus the LOAD (read_parquet needs no extension).
#
# Timing methodology: each timed step is measured with python3
# `time.time()` INSIDE a single interpreter that `subprocess.run`s the
# duckdb invocation — NOT with two `$(python3 -c ...)` command
# substitutions bracketing the command. The bracketing pattern (as in the
# original skeleton) puts one full python3 interpreter startup inside
# every timed window (the closing timestamp is only taken after python
# boots — ~0.2s here through a pyenv shim), silently inflating every
# sample; measured and rejected during implementation.
set -euo pipefail

DUCKDB=${DUCKDB:-/opt/homebrew/bin/duckdb}
INPUT=$1
OUT_CSV=$2
DATASET=$(basename "$INPUT")

READ_FN=read_cityjson
[[ "$INPUT" == *.jsonl ]] && READ_FN=read_cityjsonseq

# Median of 3 float-second samples (`$@`), consistent with the Rust
# harness's own `median_secs` (Task 6) so the two sets of numbers are
# comparable apples-to-apples.
median3() {
  python3 -c "
import sys
vals = sorted(float(v) for v in sys.argv[1:])
n = len(vals)
mid = n // 2
print(f'{vals[mid]:.3f}' if n % 2 else f'{(vals[mid-1]+vals[mid])/2:.3f}')
" "$@"
}

# Prints the wall-clock seconds `"$DUCKDB" -c "$1"` takes, measured with
# time.time() inside ONE python3 interpreter wrapping the subprocess —
# interpreter startup falls outside the timed window (see header:
# "Timing methodology"). `check=True` + `set -e` keep failures fatal.
timed_duckdb() {
  python3 -c '
import subprocess, sys, time
t0 = time.time()
subprocess.run([sys.argv[1], "-c", sys.argv[2]], check=True, stdout=subprocess.DEVNULL)
print(time.time() - t0)
' "$DUCKDB" "$1"
}

# Untimed one-off: make sure the extension is installed BEFORE any timed
# block, so no write_s sample pays for the INSTALL (idempotent; may hit
# the network the first time on a machine).
"$DUCKDB" -c "INSTALL cityjson FROM community;" > /dev/null

# Calibrate the fixed per-invocation overhead every timed write_s sample
# still carries (process startup + LOAD): median of 3, disclosed on
# stderr, deliberately NOT subtracted from the reported numbers (see
# header).
CAL_TIMES=()
for _ in 1 2 3; do
  CAL_TIMES+=("$(timed_duckdb "LOAD cityjson; SELECT 1;")")
done
CALIBRATION_S=$(median3 "${CAL_TIMES[@]}")
echo "# calibration: duckdb process startup + LOAD cityjson = ${CALIBRATION_S}s per invocation (median of 3); included, undeducted, in every write_s sample below" >&2

for CODEC in SNAPPY ZSTD; do
  WRITE_TIMES=()
  SCAN_TIMES=()
  PARQUET=""
  TMP=""

  for _ in 1 2 3; do
    rm -rf "${TMP:-}"
    TMP=$(mktemp -d)
    PARQUET="$TMP/out.parquet"

    WRITE_TIMES+=("$(timed_duckdb "LOAD cityjson; COPY (SELECT * FROM ${READ_FN}('$INPUT')) TO '$PARQUET' (FORMAT PARQUET, COMPRESSION $CODEC);")")

    SCAN_TIMES+=("$(timed_duckdb "SELECT sum(hash(COLUMNS(*))) FROM read_parquet('$PARQUET');")")
  done

  WRITE_S=$(median3 "${WRITE_TIMES[@]}")
  FULL_SCAN_S=$(median3 "${SCAN_TIMES[@]}")
  BYTES=$(stat -f %z "$PARQUET" 2>/dev/null || stat -c %s "$PARQUET")
  OBJS=$("$DUCKDB" -csv -noheader -c "SELECT count(*) FROM read_parquet('$PARQUET');")

  NAME=duckdb-copy
  [[ $CODEC == ZSTD ]] && NAME=duckdb-copy-zstd

  if [[ ! -f "$OUT_CSV" ]]; then
    mkdir -p "$(dirname "$OUT_CSV")"
    echo "dataset,variant,object_count,write_s,total_bytes,cityobjects_bytes,sidecar_bytes,full_scan_s,window_query_s,row_groups_total,row_groups_touched,roundtrip_equal" > "$OUT_CSV"
  fi
  echo "$DATASET,$NAME,$OBJS,$WRITE_S,$BYTES,$BYTES,0,$FULL_SCAN_S,,,," >> "$OUT_CSV"

  rm -rf "$TMP"
done
