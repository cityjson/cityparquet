#!/usr/bin/env bash
# DuckDB-over-Parquet SQL-engine baseline for the read-benchmark milestone
# (Task 12). Appends `duckdb-parquet` rows to the SAME CSV the
# `cityparquet-readbench` coordinator (`crates/cityparquet-readbench`) owns,
# using the EXACT header contract:
#   dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,
#   peak_heap_bytes,peak_rss_bytes,repeat,notes
#
# UNLIKE `scripts/bench_duckdb.sh` (M5's write-side baseline, which reads
# CityJSON/CityJSONSeq through the community `cityjson` extension's
# `read_cityjson`/`read_cityjsonseq` table functions — an extension with
# well-documented partial-geometry gaps, see that script's own header), THIS
# script queries a `cityparquet-rs`-WRITTEN CityParquet package directly via
# plain `read_parquet(...)`. It carries full geometry (every LoD column the
# package declares) and a typed `bbox` STRUCT(xmin, ymin, zmin, xmax, ymax,
# zmax) column, DuckDB-verified via `DESCRIBE` against a real converted
# package:
#   duckdb -c "DESCRIBE SELECT * FROM read_parquet('<pkg>/cityobjects.parquet');"
# -> bbox  struct(xmin double, ymin double, zmin double, xmax double, ymax
#    double, zmax double)
# So the `cityjson`-extension geometry-coverage/COPY caveats in
# `bench_duckdb.sh` do NOT apply here: this is a clean SQL-engine-over-
# columnar-Parquet baseline, and reading plain Parquet needs NO
# `INSTALL`/`LOAD` of any extension.
#
# `peak_heap_bytes` is EMPTY for every row this script writes: DuckDB is an
# out-of-process SQL engine here, so there is no `peak_alloc` hook into its
# allocator the way the in-process `cityparquet-readbench --child` protocol
# has one. `peak_rss_bytes` is captured via a SEPARATE, untimed
# `/usr/bin/time -l duckdb ...` invocation per scenario, parsing "maximum
# resident set size" (bytes on macOS, this script's development platform;
# GNU coreutils' `/usr/bin/time -v` reports "Maximum resident set size" in
# KiB and is used as a fallback, converted to bytes) — if neither form is
# available/parseable, the field is left empty rather than fabricated.
#
# GEOMETRY-AUTOCONVERSION FINDING (verified 2026-07-08 against a real
# converted package, DuckDB v1.5.3): our packages' WKB geometry columns
# carry proper GeoParquet "geo" file metadata, so DuckDB's spatial
# extension (autoloaded) eagerly decodes them into its own native
# `GEOMETRY` logical type the moment a query references the column at
# all — even a bare `SELECT geometry_lod1_2 FROM ...` with no function
# applied. That native decode does not support every WKB shape our LoD1.2/
# LoD1.3/LoD2 solids use (multi-surface/solid geometry) and fails with
# `Invalid Input Error: Unsupported geometry type in WKB` — confirmed to
# reproduce identically whether the column is bare, hashed, or explicitly
# `CAST(... AS BLOB)` afterwards (the failure is in the Parquet scan's own
# decode step, before any function runs). `duckdb_settings()` exposes the
# fix: `enable_geoparquet_conversion` ("Attempt to decode/encode geometry
# data in/as GeoParquet files if the spatial extension is present.",
# default true). This script sets it `false` on EVERY invocation (see
# `run_sql`/`timed_duckdb` below), which makes every geometry column read
# back as plain `BLOB` — verified to fix `full-read`'s `SELECT
# sum(hash(COLUMNS(*)))` (the only scenario that touches every geometry
# column) — and is, if anything, MORE faithful to the M5 "force full byte-
# level decode" intent than hashing DuckDB's own re-parsed spatial objects
# would have been. `bbox`/attribute-only scenarios are column-pruned and
# never touch a geometry column either way, so this setting is a no-op for
# them; it is applied uniformly for simplicity and to avoid this exact trap
# resurfacing on some other dataset's geometry shape.
#
# Scenarios (SQL over the main table `<pkg>/cityobjects.parquet`), each
# timed via a shell-measured `duckdb -c "..."` (median of `--repeat`, default
# 5, 6-decimal `time_s`/`time_mad_s`):
#   count        SELECT count(*)                                    (selectivity empty)
#   full-read    SELECT sum(hash(COLUMNS(*)))  (forces full decode, M5 pattern; selectivity empty)
#   bbox-query   one row per 1%/5%/25% lower-left window (tagged in notes),
#                testing bbox.{xmax,xmin,ymax,ymin} overlap only (the window's
#                z span is always the dataset's FULL z range, so a z test
#                would never exclude a row — see `bbox_window` below)
#   attr-filter  the most-frequent `object_type`, WHERE-equality count
#   attr-stats   only if --numeric-column is given (else skipped, noted on stderr)
#   project      SELECT count(object_type) (a projected non-null count)
#
# Window construction and the AttrFilter tie-break MATCH
# `crates/cityparquet-readbench/src/coordinator.rs` exactly, so `duckdb-
# parquet` rows are directly comparable to that coordinator's `cityparquet`/
# `cityparquet-hilbert` rows in the same CSV:
#   - `bbox_window(bbox, frac)`: lower-left corner anchored, `frac` of the
#     x/y extent, full original z range — identical to `coordinator.rs`'s own
#     `bbox_window` (and `crates/cityparquet-cli/src/bench.rs`'s window
#     before it).
#   - `AttrFilter`'s predicate value: the most-frequent `object_type`,
#     ties broken by the SMALLEST string value (`ORDER BY c DESC, object_type
#     LIMIT 1` picks the lexicographically-first name among equal counts) —
#     the SQL equivalent of `coordinator.rs`'s
#     `counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))`,
#     which (worked through: for a tie, `b.0.cmp(&a.0)` is `Greater` exactly
#     when `a.0 < b.0`, so the smaller string is what `max_by` keeps) selects
#     the SAME value this script's `ORDER BY ... object_type LIMIT 1` does.
#   - The object-level denominator for `attr-filter`/`attr-stats`/`project`
#     selectivity is `count(*)` over THIS table — which, because this
#     script always queries a `cityparquet` package's own main table (one
#     row per CityObject), is exactly the coordinator's own
#     dataset-global CityObject total for the SAME package.
#
# `dataset` naming: this script only receives the `.parquet` PACKAGE
# directory (not the original CityJSON/CityJSONSeq input path), so it
# cannot recover the coordinator's own `dataset` value verbatim (that is
# the ORIGINAL input's file name, e.g. `9-196-328.city.json`, WITH
# extension). It derives `dataset` from the package directory's own base
# name with a trailing `.parquet` and (if present) `-hilbert` suffix
# stripped, e.g. `bench/data/readbench/9-196-328.parquet` and
# `9-196-328-hilbert.parquet` both yield `dataset=9-196-328` — matching how
# the coordinator itself uses ONE shared `dataset` string across its own
# `cityparquet`/`cityparquet-hilbert` rows for the same package. Rows from
# this script are therefore joinable with the coordinator's rows on
# `dataset` only up to that original-extension difference; disclosed here
# rather than silently mismatched.
#
# Timing honesty: the fixed per-invocation `duckdb` process-startup overhead
# every timed sample carries is measured (median of 5 of `duckdb -c "SELECT
# 1;"`) and disclosed on stderr as a `# calibration:` line — mirroring
# `scripts/bench_duckdb.sh` — and is NEVER subtracted from the reported
# `time_s`/`time_mad_s`.
#
# This is a local dev tool: needs `duckdb` (v1.5.x tested) + `python3` on
# PATH, and a package produced by `just readbench-prepare`/`cityparquet
# convert`. NOT wired into `just check`/CI.
set -euo pipefail

DUCKDB=${DUCKDB:-duckdb}

CSV_HEADER="dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes"

usage() {
  cat >&2 <<EOF
usage: $0 PARQUET_PKG OUT_CSV [--numeric-column COL] [--repeat N]
  PARQUET_PKG        a CityParquet package directory (contains cityobjects.parquet)
  OUT_CSV            the read-benchmark result CSV to append duckdb-parquet rows to
  --numeric-column   a real numeric (Int64/Float64/Double) attribute column;
                      enables the attr-stats scenario (skipped without it)
  --repeat N         warm repeats per timed measurement (default: 5)
EOF
}

if [[ $# -lt 2 ]]; then
  usage
  exit 1
fi

PARQUET_PKG=$1
OUT_CSV=$2
shift 2

NUMERIC_COLUMN=""
REPEAT=5

while [[ $# -gt 0 ]]; do
  case "$1" in
    --numeric-column)
      [[ $# -ge 2 ]] || { echo "error: --numeric-column requires a value" >&2; exit 1; }
      NUMERIC_COLUMN=$2
      shift 2
      ;;
    --repeat)
      [[ $# -ge 2 ]] || { echo "error: --repeat requires a value" >&2; exit 1; }
      REPEAT=$2
      shift 2
      ;;
    *)
      echo "error: unknown argument '$1'" >&2
      usage
      exit 1
      ;;
  esac
done

if ! [[ "$REPEAT" =~ ^[0-9]+$ ]] || [[ "$REPEAT" -lt 1 ]]; then
  echo "error: --repeat must be a positive integer, got '$REPEAT'" >&2
  exit 1
fi

TABLE="$PARQUET_PKG/cityobjects.parquet"
if [[ ! -f "$TABLE" ]]; then
  echo "error: no main table at $TABLE (expected a TableLayout::Single CityParquet package)" >&2
  exit 1
fi

# Derive `dataset` from the package directory's own name (see this script's
# header for why it cannot recover the coordinator's original-extension
# name): strip a trailing `.parquet`, then a trailing `-hilbert`.
PKG_BASE="$(basename "$PARQUET_PKG")"
PKG_BASE="${PKG_BASE%.parquet}"
PKG_BASE="${PKG_BASE%-hilbert}"
DATASET="$PKG_BASE"

if [[ -f "$OUT_CSV" ]]; then
  EXISTING_HEADER="$(head -n 1 "$OUT_CSV")"
  if [[ "$EXISTING_HEADER" != "$CSV_HEADER" ]]; then
    echo "error: $OUT_CSV already exists with an unexpected header:" >&2
    echo "  found:    $EXISTING_HEADER" >&2
    echo "  expected: $CSV_HEADER" >&2
    exit 1
  fi
else
  mkdir -p "$(dirname "$OUT_CSV")"
  echo "$CSV_HEADER" > "$OUT_CSV"
fi

# Median + median-absolute-deviation of float-second samples (`$@`), at
# 6-decimal (microsecond) precision, printed as "MEDIAN MAD" on one line —
# matching the coordinator's own `median`/`mad` helpers
# (`crates/cityparquet-readbench/src/coordinator.rs`) so the two tools'
# numbers are directly comparable.
median_and_mad() {
  python3 -c "
import sys

def med(xs):
    xs = sorted(xs)
    n = len(xs)
    mid = n // 2
    return xs[mid] if n % 2 else (xs[mid - 1] + xs[mid]) / 2

vals = [float(v) for v in sys.argv[1:]]
m = med(vals)
mad = med([abs(v - m) for v in vals])
print(f'{m:.6f} {mad:.6f}')
" "$@"
}

# Prepended to EVERY duckdb -c invocation below (timed, untimed, and RSS
# capture alike) — see this script's header "GEOMETRY-AUTOCONVERSION
# FINDING" for why: without it, any query that materialises a geometry
# column (only `full-read`'s `COLUMNS(*)` does) fails with `Invalid Input
# Error: Unsupported geometry type in WKB` on our LoD1.2/LoD1.3/LoD2 solids.
DUCKDB_PRELUDE="SET enable_geoparquet_conversion=false;"

# Runs `$1` (untimed) via `"$DUCKDB" -csv -noheader -c`, prefixed with
# `$DUCKDB_PRELUDE`, and prints its one-line/one-value CSV result.
run_sql() {
  "$DUCKDB" -csv -noheader -c "$DUCKDB_PRELUDE $1"
}

# Prints the wall-clock seconds `"$DUCKDB" -c "$DUCKDB_PRELUDE $1"` takes,
# measured with time.time() inside ONE python3 interpreter wrapping the
# subprocess (so interpreter startup falls outside the timed window) —
# identical technique to `scripts/bench_duckdb.sh`'s own `timed_duckdb`.
timed_duckdb() {
  python3 -c '
import subprocess, sys, time
t0 = time.time()
subprocess.run([sys.argv[1], "-c", sys.argv[2] + " " + sys.argv[3]], check=True, stdout=subprocess.DEVNULL)
print(time.time() - t0)
' "$DUCKDB" "$DUCKDB_PRELUDE" "$1"
}

# Runs `$1` `$REPEAT` times via `timed_duckdb`, prints "MEDIAN MAD".
timed_median() {
  local sql="$1"
  local times=()
  local i
  for ((i = 0; i < REPEAT; i++)); do
    times+=("$(timed_duckdb "$sql")")
  done
  median_and_mad "${times[@]}"
}

# One extra, UNTIMED invocation of `$1` wrapped in `/usr/bin/time`, to
# capture peak RSS without perturbing the timing samples above. Tries BSD/
# macOS `-l` first (bytes), then GNU coreutils `-v` (KiB, converted to
# bytes); prints an empty string (never fabricates a number) if neither
# form is available or parseable.
capture_rss() {
  local sql="$1"
  local tmp
  tmp=$(mktemp)

  if /usr/bin/time -l "$DUCKDB" -c "$DUCKDB_PRELUDE $sql" >/dev/null 2>"$tmp"; then
    local rss
    rss=$(grep -E 'maximum resident set size' "$tmp" | awk '{print $1}')
    if [[ -n "$rss" ]]; then
      rm -f "$tmp"
      echo "$rss"
      return
    fi
  fi

  : > "$tmp"
  if /usr/bin/time -v "$DUCKDB" -c "$DUCKDB_PRELUDE $sql" >/dev/null 2>"$tmp"; then
    local rss_kib
    rss_kib=$(grep -E 'Maximum resident set size' "$tmp" | grep -oE '[0-9]+' | head -1)
    if [[ -n "$rss_kib" ]]; then
      rm -f "$tmp"
      echo "$((rss_kib * 1024))"
      return
    fi
  fi

  rm -f "$tmp"
  echo ""
}

# `num / den` at 6-decimal precision, or empty if `den` is not positive
# (never divides by zero; never fabricates a selectivity).
safe_div() {
  python3 -c "
import sys
num = float(sys.argv[1])
den = float(sys.argv[2])
print(f'{num / den:.6f}' if den > 0 else '')
" "$1" "$2"
}

append_row() {
  local dataset="$1" format="$2" scenario="$3" selectivity="$4" result_count="$5" \
    time_s="$6" time_mad_s="$7" peak_heap_bytes="$8" peak_rss_bytes="$9" repeat="${10}" notes="${11}"
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$dataset" "$format" "$scenario" "$selectivity" "$result_count" \
    "$time_s" "$time_mad_s" "$peak_heap_bytes" "$peak_rss_bytes" "$repeat" "$notes" \
    >> "$OUT_CSV"
}

# Calibrate the fixed per-invocation `duckdb` process-startup overhead every
# timed sample below still carries (plain Parquet needs no
# INSTALL/LOAD): median of 5, disclosed on stderr, deliberately NOT
# subtracted from any reported time_s/time_mad_s.
CAL_TIMES=()
for _ in 1 2 3 4 5; do
  CAL_TIMES+=("$(timed_duckdb "SELECT 1;")")
done
read -r CAL_MEDIAN CAL_MAD <<< "$(median_and_mad "${CAL_TIMES[@]}")"
echo "# calibration: duckdb process startup (no extension LOAD needed for plain Parquet) = ${CAL_MEDIAN}s per invocation (median of 5, MAD ${CAL_MAD}s); included, undeducted, in every time_s sample below" >&2

TOTAL=$(run_sql "SELECT count(*) FROM read_parquet('$TABLE');")
echo "cityparquet-readbench duckdb-parquet baseline: dataset=$DATASET table=$TABLE total_objects=$TOTAL repeat=$REPEAT numeric_column=${NUMERIC_COLUMN:-<none>}" >&2

# --- count ---
SQL="SELECT count(*) FROM read_parquet('$TABLE');"
read -r TIME_S TIME_MAD_S <<< "$(timed_median "$SQL")"
RSS=$(capture_rss "$SQL")
append_row "$DATASET" "duckdb-parquet" "count" "" "$TOTAL" "$TIME_S" "$TIME_MAD_S" "" "$RSS" "$REPEAT" ""

# --- full-read (forces full column decode, M5 pattern) ---
SQL="SELECT sum(hash(COLUMNS(*))) FROM read_parquet('$TABLE');"
read -r TIME_S TIME_MAD_S <<< "$(timed_median "$SQL")"
RSS=$(capture_rss "$SQL")
append_row "$DATASET" "duckdb-parquet" "full-read" "" "$TOTAL" "$TIME_S" "$TIME_MAD_S" "" "$RSS" "$REPEAT" ""

# --- bbox-query: 1%/5%/25% lower-left windows of the dataset's own x/y extent ---
BBOX_ROW=$(run_sql "SELECT min(bbox.xmin), min(bbox.ymin), max(bbox.xmax), max(bbox.ymax) FROM read_parquet('$TABLE');")
IFS=',' read -r DXMIN DYMIN DXMAX DYMAX <<< "$BBOX_ROW"

for entry in "0.01:bbox-1pct" "0.05:bbox-5pct" "0.25:bbox-25pct"; do
  FRAC="${entry%%:*}"
  TAG="${entry##*:}"
  WXMAX=$(python3 -c "print($DXMIN + ($DXMAX - $DXMIN) * $FRAC)")
  WYMAX=$(python3 -c "print($DYMIN + ($DYMAX - $DYMIN) * $FRAC)")
  # z is always the FULL dataset range (see this script's header), so no
  # z clause is needed: it can never exclude a row.
  SQL="SELECT count(*) FROM read_parquet('$TABLE') WHERE bbox.xmax >= $DXMIN AND bbox.xmin <= $WXMAX AND bbox.ymax >= $DYMIN AND bbox.ymin <= $WYMAX;"
  MATCHES=$(run_sql "$SQL")
  read -r TIME_S TIME_MAD_S <<< "$(timed_median "$SQL")"
  RSS=$(capture_rss "$SQL")
  SEL=$(safe_div "$MATCHES" "$TOTAL")
  append_row "$DATASET" "duckdb-parquet" "bbox-query" "$SEL" "$MATCHES" "$TIME_S" "$TIME_MAD_S" "" "$RSS" "$REPEAT" "$TAG"
done

# --- attr-filter: object_type = <most-frequent value>, ties broken by the
# smallest string (matches coordinator.rs's max_by tie-break exactly; see
# this script's header) ---
ATTR_ROW=$(run_sql "SELECT object_type, count(*) c FROM read_parquet('$TABLE') GROUP BY object_type ORDER BY c DESC, object_type LIMIT 1;")
IFS=',' read -r OBJECT_TYPE OT_COUNT <<< "$ATTR_ROW"

SQL="SELECT count(*) FROM read_parquet('$TABLE') WHERE object_type = '$OBJECT_TYPE';"
MATCHES=$(run_sql "$SQL")
read -r TIME_S TIME_MAD_S <<< "$(timed_median "$SQL")"
RSS=$(capture_rss "$SQL")
SEL=$(safe_div "$MATCHES" "$TOTAL")
append_row "$DATASET" "duckdb-parquet" "attr-filter" "$SEL" "$MATCHES" "$TIME_S" "$TIME_MAD_S" "" "$RSS" "$REPEAT" "attr=object_type=$OBJECT_TYPE"

# --- attr-stats: only if a numeric column was given ---
if [[ -n "$NUMERIC_COLUMN" ]]; then
  SQL="SELECT min($NUMERIC_COLUMN), max($NUMERIC_COLUMN), sum($NUMERIC_COLUMN), count($NUMERIC_COLUMN) FROM read_parquet('$TABLE');"
  STATS_ROW=$(run_sql "$SQL")
  IFS=',' read -r MIN_V MAX_V SUM_V CNT_V <<< "$STATS_ROW"
  read -r TIME_S TIME_MAD_S <<< "$(timed_median "$SQL")"
  RSS=$(capture_rss "$SQL")
  SEL=$(safe_div "$CNT_V" "$TOTAL")
  append_row "$DATASET" "duckdb-parquet" "attr-stats" "$SEL" "$CNT_V" "$TIME_S" "$TIME_MAD_S" "" "$RSS" "$REPEAT" \
    "attr=$NUMERIC_COLUMN min=$MIN_V max=$MAX_V sum=$SUM_V"
else
  echo "# skip: attr-stats scenario requires --numeric-column (none given)" >&2
fi

# --- project: single-column projected non-null count ---
SQL="SELECT count(object_type) FROM read_parquet('$TABLE');"
CNT=$(run_sql "$SQL")
read -r TIME_S TIME_MAD_S <<< "$(timed_median "$SQL")"
RSS=$(capture_rss "$SQL")
SEL=$(safe_div "$CNT" "$TOTAL")
append_row "$DATASET" "duckdb-parquet" "project" "$SEL" "$CNT" "$TIME_S" "$TIME_MAD_S" "" "$RSS" "$REPEAT" ""

echo "readbench_duckdb: appended duckdb-parquet rows for dataset=$DATASET to $OUT_CSV" >&2
