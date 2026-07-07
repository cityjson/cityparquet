#!/usr/bin/env bash
# Fetch the 3 pinned 3DBAG tiles used by the M5 real-data benchmark
# (`just bench-data` / `just bench-all`).
#
# Reproducibility beats freshness: the tile ids and download URLs below are
# HARDCODED, not re-derived from "latest" at run time, so the benchmark
# numbers in bench/README.md always refer to the exact same input bytes.
#
# Selection (2026-07-07), queried against the live 3DBAG tile index
# (https://data.3dbag.nl/latest/tile_index.fgb, v20250903 release) via:
#
#   duckdb -c "INSTALL spatial; LOAD spatial;
#     SELECT tile_id, cj_download, ST_XMin(geom), ST_YMin(geom),
#            ST_XMax(geom), ST_YMax(geom)
#     FROM ST_Read('https://data.3dbag.nl/latest/tile_index.fgb')
#     WHERE ST_Intersects(geom, ST_MakeEnvelope(<x0>, <y0>, <x1>, <y1>));"
#
# All three tiles are at zoom level 9 (~1km x 1km each) so their gzipped
# content-length is a like-for-like density proxy (verified via
# `curl -sI <cj_download> | grep -i content-length`, and every URL was
# confirmed to respond 200 via `curl -fI` before pinning):
#
#   dense-urban  9/284/556  bbox [84593, 445890]-[85593, 446890] (RD/EPSG:28992)
#                Delft historic city centre. 1,900,526 bytes gzipped —
#                the largest of the z=9 candidates probed around the
#                Delft centre bbox (84500,445500)-(85500,446500), i.e.
#                the densest built-up cell in that search window.
#
#   suburban     9/304/532  bbox [89593, 439890]-[90593, 440890] (RD/EPSG:28992)
#                Residential/light-industrial area between Delft and
#                Rotterdam (searched around (90000,440000)-(91500,441500)).
#                525,507 bytes gzipped — roughly a quarter the density of
#                the dense-urban tile, consistent with lower-rise suburban
#                housing rather than a historic core.
#
#   rural        9/196/328  bbox [62593, 388890]-[63593, 389890] (RD/EPSG:28992)
#                Zeeland farmland (searched around (55000,385000)-
#                (70000,400000), a province-scale window with no city).
#                52,103 bytes gzipped — a neighbour cell (9/196/332) in the
#                same search window came back at only 2,663 bytes (all but
#                empty of buildings), so this cell was chosen instead: it
#                has genuine rural content (scattered farm buildings)
#                without being a near-empty edge case.
#
# All three are gzipped whole-document CityJSON (v20250903 release,
# EPSG:7415, quantised vertices), unpacked here to bench/data/<tile>.city.json.
#
# Idempotent: skips the download+gunzip for any tile whose .city.json
# already exists.
set -euo pipefail

DATA_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/bench/data"
mkdir -p "$DATA_DIR"

# tile_id | cj_download URL
TILES=(
  "9-284-556|https://data.3dbag.nl/v20250903/tiles/9/284/556/9-284-556.city.json.gz"
  "9-304-532|https://data.3dbag.nl/v20250903/tiles/9/304/532/9-304-532.city.json.gz"
  "9-196-328|https://data.3dbag.nl/v20250903/tiles/9/196/328/9-196-328.city.json.gz"
)

for entry in "${TILES[@]}"; do
  tile_id="${entry%%|*}"
  url="${entry#*|}"
  dest="$DATA_DIR/$tile_id.city.json"
  gz="$DATA_DIR/$tile_id.city.json.gz"

  if [[ -f "$dest" ]]; then
    echo "skip $tile_id (already fetched: $dest)"
    continue
  fi

  echo "fetch $tile_id <- $url"
  curl -f -o "$gz" "$url"
  gunzip -f "$gz"
  echo "  -> $dest"
done

echo "3DBAG fetch complete"
