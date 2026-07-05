#!/bin/bash
set -euo pipefail

# Check if duckdb is installed
if ! command -v duckdb >/dev/null; then
    echo "interop skipped (duckdb not installed)"
    exit 0
fi

# Create temporary directory
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Convert the delft fixture to Parquet
cargo run -p cityparquet-cli -- convert tests/fixtures/delft.city.jsonl "$TMP/out" --overwrite

# DuckDB IS the assertion: it must be able to read the file natively as
# GeoParquet, with no fallback if it fails.
duckdb -c "SELECT count(*) FROM '$TMP/out/cityobjects.parquet'" | grep -q "2231"
duckdb -c "SELECT min(bbox.xmin) FROM '$TMP/out/cityobjects.parquet'"

echo "interop ok"
