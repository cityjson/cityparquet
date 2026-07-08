#!/usr/bin/env bash
# Fetch the CityJSON benchmark corpus (`just bench-corpus`) — the 11
# CityJSONSeq datasets published at gs://cityjson/benchmark_dataset/, a
# cross-city, cross-producer spread (2.8 MB Rotterdam .. 675 MB textured
# Helsinki) used as the M5+ real-data benchmark inputs alongside the three
# pinned 3DBAG tiles (scripts/fetch_3dbag.sh).
#
# Downloaded to bench/data/benchmark/ (the whole of bench/data/ is
# gitignored — see .gitignore). Reproducibility beats freshness: the object
# names and their byte sizes below are the snapshot taken 2026-07-08 from
# `gsutil ls -l gs://cityjson/benchmark_dataset/`. Each file's size is
# verified after download; a mismatch hard-fails rather than silently
# benchmarking against different bytes.
#
# Idempotent: an already-present file whose size matches is skipped, so a
# re-run only fetches what is missing or truncated. Needs `gsutil` on PATH
# (the bucket is public; no auth required).
set -euo pipefail

DATA_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/bench/data/benchmark"
BUCKET="gs://cityjson/benchmark_dataset"
mkdir -p "$DATA_DIR"

if ! command -v gsutil >/dev/null 2>&1; then
  echo "error: gsutil not found on PATH (needed to fetch $BUCKET)" >&2
  exit 1
fi

# local filename | expected byte size | source object glob
# The source glob defaults to the local filename; it differs only for
# Montréal, whose accented object name is stored NFC in the bucket while the
# local filesystem normalises to NFD, so a literal-byte match fails — a
# `Montr*al` wildcard sidesteps the normalisation mismatch, and the file is
# saved under an ASCII name for painless downstream use.
# (gsutil ls -l snapshot, 2026-07-08)
FILES=(
  "Rotterdam.jsonl|2822644"
  "Ingolstadt.city.jsonl|4025892"
  "Railway.city.jsonl|4246374"
  "Montreal.city.jsonl|4822019|Montr*al.city.jsonl"
  "Vienna.city.jsonl|5038989"
  "3DBAG.city.jsonl|6158306"
  "NYC.jsonl|100087976"
  "Zurich.city.jsonl|259121439"
  "3DBV.city.jsonl|332757065"
  "Helsinki.city.jsonl|432470424"
  "Helsinki_tex.city.jsonl|674971888"
)

size_of() {
  # portable byte size of $1 (macOS/BSD stat vs GNU stat)
  stat -f%z "$1" 2>/dev/null || stat -c%s "$1"
}

for entry in "${FILES[@]}"; do
  name="${entry%%|*}"
  rest="${entry#*|}"
  want="${rest%%|*}"
  # third field (source glob) is optional; defaults to the local name
  if [[ "$rest" == *"|"* ]]; then
    src="${rest#*|}"
  else
    src="$name"
  fi
  dest="$DATA_DIR/$name"

  if [[ -f "$dest" ]]; then
    have="$(size_of "$dest")"
    if [[ "$have" == "$want" ]]; then
      echo "skip $name (already fetched, $have bytes)"
      continue
    fi
    echo "warn $name: size mismatch on existing file (got $have, want $want)" \
      "-- re-fetching" >&2
    rm -f "$dest"
  fi

  echo "fetch $name <- $BUCKET/$src"
  gsutil cp "$BUCKET/$src" "$dest"
  have="$(size_of "$dest")"
  if [[ "$have" != "$want" ]]; then
    echo "error: $name size mismatch after download (got $have, want $want)" \
      "-- refusing to benchmark against a truncated/changed file" >&2
    exit 1
  fi
  echo "  size verified: $name ($have bytes)"
done

echo "benchmark corpus fetch complete ($DATA_DIR)"
