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
cargo run -p cityparquet-cli -- convert tests/fixtures/delft.city.jsonl -o "$TMP/out" --overwrite

# delft is a single 1st-level family (Building, with BuildingPart children
# folded into it), so by-type conversion (the only, mandatory layout) writes
# exactly one main table: building.parquet.
#
# DuckDB IS the assertion: it must be able to read the file natively as
# GeoParquet, with no fallback if it fails.
duckdb -c "SELECT count(*) FROM '$TMP/out/building.parquet'" | grep -q "2231"
duckdb -c "SELECT min(bbox.xmin) FROM '$TMP/out/building.parquet'"

# Convert the railway fixture under the Compatibility profile, which writes
# the materials/textures/geometry_templates sidecars alongside the main
# table. DuckDB IS the assertion for these too: it must read each sidecar
# natively as plain Parquet (no CityParquet-specific extension required,
# since a sidecar carries no geo metadata), with no fallback if it fails —
# and the main table must still be readable, sidecars or not.
cargo run -p cityparquet-cli -- convert tests/fixtures/lod3_railway.city.json -o "$TMP/railway" --profile compatibility --overwrite

duckdb -c "SELECT count(*) FROM read_parquet('$TMP/railway/materials.parquet')" | grep -q "85"
duckdb -c "SELECT count(*) FROM read_parquet('$TMP/railway/textures.parquet')" | grep -q "34"
duckdb -c "SELECT count(*) FROM read_parquet('$TMP/railway/geometry_templates.parquet')" | grep -q "3"

# railway has 10 1st-level families, so by-type conversion writes 10 main
# tables (bridge.parquet, building.parquet, ...), never one main table —
# every main table shares the identical schema (see
# `crate::package::TableWriters`'s own doc comment in the Rust source), so
# `read_parquet` over the list of every main table (every *.parquet file
# except the three known sidecars above) unions their rows directly,
# matching the object_count `convert` itself reports for this fixture.
main_tables_sql="["
first=1
for f in "$TMP"/railway/*.parquet; do
  base="$(basename "$f")"
  case "$base" in
    materials.parquet|textures.parquet|geometry_templates.parquet) continue ;;
  esac
  if [[ "$first" -eq 0 ]]; then
    main_tables_sql+=","
  fi
  main_tables_sql+="'$f'"
  first=0
done
main_tables_sql+="]"
duckdb -c "SELECT count(*) FROM read_parquet(${main_tables_sql})" | grep -q "121"

echo "interop ok"
