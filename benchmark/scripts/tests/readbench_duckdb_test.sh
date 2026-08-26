#!/usr/bin/env bash
# `readbench_duckdb.sh`'s contract with the coordinator's resolved-parameters
# sidecar: it reads the windows, the attribute choices and the id probes from
# that file, and refuses to run without it. A silent fall back to its own bash
# derivation is exactly the drift the sidecar exists to remove.
#
# Needs no DuckDB: every assertion here is about argument handling and about
# what the script no longer contains.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DUCKDB_SH="$SCRIPT_DIR/../readbench_duckdb.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# --- A missing --params is a hard failure, never a fallback ---
if "$DUCKDB_SH" "$tmp/nonexistent.parquet" "$tmp/out.csv" \
  --params "$tmp/nonexistent.params.json" >"$tmp/log" 2>&1; then
  fail "the script succeeded with no params file; it must refuse"
fi
grep -qi "params" "$tmp/log" ||
  fail "the failure message must name the missing params file; got: $(cat "$tmp/log")"

# --- --params is REQUIRED, not optional ---
if "$DUCKDB_SH" "$tmp/nonexistent.parquet" "$tmp/out.csv" >"$tmp/log2" 2>&1; then
  fail "the script succeeded with no --params at all; it must refuse"
fi
grep -qi -- "--params" "$tmp/log2" ||
  fail "omitting --params must say so; got: $(cat "$tmp/log2")"

# --- The script must not carry its own parameter derivation any more ---
#
# Comment lines are stripped first: this file's header deliberately RECORDS
# what was removed, and a grep over the whole file would match that prose and
# fail for the wrong reason.
code="$tmp/code.sh"
grep -vE '^\s*#' "$DUCKDB_SH" > "$code"

if grep -qE '^\s*(function\s+)?bbox_window\s*\(\)' "$code"; then
  fail "readbench_duckdb.sh still defines bbox_window; it must read the sidecar"
fi
if grep -q 'GROUP BY object_type ORDER BY' "$code"; then
  fail "readbench_duckdb.sh still derives the most-frequent object_type; it must read the sidecar"
fi
if grep -q -- '--numeric-column' "$code"; then
  fail "readbench_duckdb.sh still takes --numeric-column; the sidecar carries that choice"
fi

# --- project must use the sidecar's numeric column, not object_type ---
if grep -q 'count(object_type) FROM read_parquet' "$code"; then
  fail "project still counts object_type; the coordinator projects the numeric column"
fi

# --- and it must actually READ the sidecar ---
grep -q "jq -r '.windows\[\]" "$code" ||
  fail "the script must read its windows from the sidecar with jq"
grep -q "jq -r '.object_type'" "$code" ||
  fail "the script must read object_type from the sidecar"
grep -q "jq -r '.numeric_attr" "$code" ||
  fail "the script must read numeric_attr from the sidecar"

echo "PASS: readbench_duckdb.sh reads the resolved-parameters sidecar"
