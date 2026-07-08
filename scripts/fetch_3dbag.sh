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
# Idempotent, but NOT integrity-blind (M5 Codex review, Important finding
# 6): every DECOMPRESSED .city.json's sha256 is pinned below (computed
# 2026-07-08 via `shasum -a 256` against the exact bytes fetched by this
# script's URLs — the same files bench/results/*.csv was measured against).
# Both a freshly-downloaded file AND a pre-existing (skip-path) file are
# checked against the pin; a mismatch (e.g. a prior run interrupted
# mid-gunzip, or a truncated/corrupted download) deletes the bad file and
# retries the download ONCE, then hard-fails if the re-fetch still does not
# match — never silently benchmarking against different bytes than the
# ones bench/README.md's numbers were measured from.
set -euo pipefail

DATA_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/bench/data"
mkdir -p "$DATA_DIR"

# tile_id | cj_download URL | sha256 of the decompressed .city.json
TILES=(
  "9-284-556|https://data.3dbag.nl/v20250903/tiles/9/284/556/9-284-556.city.json.gz|2e14a900b12c81ece5910e8b1202a4a1490c44e60f38842338a8b289cbb3db2a"
  "9-304-532|https://data.3dbag.nl/v20250903/tiles/9/304/532/9-304-532.city.json.gz|13a952ea9fd1aae1b7e41dcc6a9e3fd58215f2be7f8c3b84d319cc53ba5d6abe"
  "9-196-328|https://data.3dbag.nl/v20250903/tiles/9/196/328/9-196-328.city.json.gz|69b70597754d8a0ad172b1d7d4c45c6b062af36ef8295224f856cc8c26f5cd4d"
)

# Prints the sha256 of `$1` (macOS/BSD `shasum -a 256`, the tool named in
# the milestone brief as available here; falls back to `sha256sum` for
# portability to a Linux CI runner).
sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

# Downloads $2 (cj_download URL) to $DATA_DIR/$1.city.json via a temporary
# .gz + gunzip, exactly as the original single-shot fetch did.
download_tile() {
  local tile_id="$1" url="$2" dest="$3" gz="$DATA_DIR/$1.city.json.gz"
  echo "fetch $tile_id <- $url"
  curl -f -o "$gz" "$url"
  gunzip -f "$gz"
  echo "  -> $dest"
}

for entry in "${TILES[@]}"; do
  tile_id="${entry%%|*}"
  rest="${entry#*|}"
  url="${rest%%|*}"
  expected_sha="${rest#*|}"
  dest="$DATA_DIR/$tile_id.city.json"

  if [[ -f "$dest" ]]; then
    actual_sha="$(sha256_of "$dest")"
    if [[ "$actual_sha" == "$expected_sha" ]]; then
      echo "skip $tile_id (already fetched, sha256 verified: $dest)"
      continue
    fi
    echo "warn $tile_id: sha256 mismatch on existing file (got $actual_sha, want $expected_sha)" \
      "-- deleting and re-fetching" >&2
    rm -f "$dest"
  fi

  download_tile "$tile_id" "$url" "$dest"
  actual_sha="$(sha256_of "$dest")"
  if [[ "$actual_sha" != "$expected_sha" ]]; then
    echo "warn $tile_id: sha256 mismatch after first download (got $actual_sha, want" \
      "$expected_sha) -- retrying once" >&2
    rm -f "$dest"
    download_tile "$tile_id" "$url" "$dest"
    actual_sha="$(sha256_of "$dest")"
    if [[ "$actual_sha" != "$expected_sha" ]]; then
      echo "error: $tile_id still fails sha256 verification after a retry" \
        "(got $actual_sha, want $expected_sha) -- refusing to benchmark against" \
        "unverified/corrupted input" >&2
      exit 1
    fi
  fi
  echo "  sha256 verified: $tile_id"
done

echo "3DBAG fetch complete"
