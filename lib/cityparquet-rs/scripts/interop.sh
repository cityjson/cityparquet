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

# delft holds Buildings (with BuildingPart children), all of which belong to
# the CityGML Building module, so the by-module layout writes exactly one
# object table: building.parquet.
#
# DuckDB IS the assertion: it must be able to read the file natively as
# GeoParquet, with no fallback if it fails.
duckdb -c "SELECT count(*) FROM '$TMP/out/building.parquet'" | grep -q "2231"
duckdb -c "SELECT min(bbox.xmin) FROM '$TMP/out/building.parquet'"

# Convert the railway fixture, which carries materials, textures and geometry
# templates, so the three sidecars are written alongside the object tables —
# sidecars are content-gated, not opted into by a flag (the `--profile` flag
# this used to pass was removed with the by-module layout).
#
# The fixture declares no CRS. That is not an error: the writer records
# `city.crs: null` and warns on stderr (GeoParquet's tri-state crs), so the
# conversion proceeds and stderr noise here is expected.
#
# DuckDB IS the assertion for the sidecars too: it must read each one natively
# as plain Parquet (no CityParquet-specific extension required, since a
# sidecar carries no geo metadata), with no fallback if it fails — and the
# object tables must still be readable, sidecars or not.
cargo run -p cityparquet-cli -- convert tests/fixtures/lod3_railway.city.json -o "$TMP/railway" --overwrite

duckdb -c "SELECT count(*) FROM read_parquet('$TMP/railway/materials.parquet')" | grep -q "85"
duckdb -c "SELECT count(*) FROM read_parquet('$TMP/railway/textures.parquet')" | grep -q "34"
duckdb -c "SELECT count(*) FROM read_parquet('$TMP/railway/geometry_templates.parquet')" | grep -q "3"

# railway spans 9 CityGML modules, so the by-module layout writes 9 object
# tables (bridge.parquet, building.parquet, ...), never one. Reading every
# object table (every *.parquet file except the three known sidecars above)
# must total the object_count `convert` itself reports.
#
# `union_by_name` because each module's schema is pruned to that module's own
# LoD/appearance needs, so two modules need not share a column list. They do
# happen to match for this fixture (every module here carries the same LoDs),
# which is exactly why a positional union would pass today and break on the
# first fixture where they diverge.
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
duckdb -c "SELECT count(*) FROM read_parquet(${main_tables_sql}, union_by_name = true)" | grep -q "121"

echo "interop ok"
