#!/usr/bin/env bash
# Fetch the LEGACY CityJSONSeq benchmark corpus (`just fetch-seq-data`) — the
# 11 CityJSONSeq datasets published at gs://cityjson/benchmark_dataset/, a
# cross-city, cross-producer spread (2.8 MB Rotterdam .. 675 MB textured
# Helsinki), ~1.7 GB in total.
#
# WHY THIS STILL EXISTS. `just fetch-data` (scripts/fetch_benchmark.sh) now
# fetches the catalogue-derived corpus of real CityGML/CityJSON documents,
# which is what the cross-format read comparison measures. This corpus is
# CityJSONSeq only — one already-converted intermediate — so it cannot answer
# "how does the format the data ships in compare", and it is no longer the
# default. It is kept, unchanged, for two jobs:
#
#   1. The ORDERING benchmark (`just ordering-bench`), a deliberately
#      single-axis run of `cityparquet` against `cityparquet-hilbert` over
#      CityJSONSeq inputs.
#   2. CONTINUITY. Every published read result in bench/READ_BENCHMARK.md and
#      bench/CORPUS_REPORT.md was measured on these exact files at these exact
#      sizes. Re-running an old number requires the old bytes.
#
# Downloaded to DEST (default bench/data/benchmark_seq/, an optional first
# argument; the whole of bench/data/ is gitignored — see .gitignore). Note the
# DIFFERENT default directory from `fetch-data`: the two corpora must not land
# on top of each other, because `just bench FOLDER` measures everything it
# finds under FOLDER.
#
# Reproducibility beats freshness: the object names and their byte sizes below
# are the snapshot taken 2026-07-08 from
# `gsutil ls -l gs://cityjson/benchmark_dataset/`, re-verified over HTTPS on
# 2026-08-16. Each file's size is verified after download; a mismatch
# hard-fails rather than silently benchmarking against different bytes.
#
# Idempotent: an already-present file whose size matches is skipped, so a
# re-run only fetches what is missing or truncated.
#
# Needs `curl`. `gsutil` is NOT needed any more — the bucket is public and
# reachable over plain HTTPS at https://storage.googleapis.com/cityjson/…,
# which is how `just fixtures` has always fetched from it.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST=${1:-bench/data/benchmark_seq}
case "$DEST" in
  /*) DATA_DIR="$DEST" ;;
  *) DATA_DIR="$REPO_ROOT/$DEST" ;;
esac
BASE_URL="https://storage.googleapis.com/cityjson/benchmark_dataset"
mkdir -p "$DATA_DIR"

if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl not found on PATH (needed to fetch $BASE_URL)" >&2
  exit 1
fi

# local filename | expected byte size | remote object name (URL-encoded)
# The remote name defaults to the local filename; it differs only for
# Montréal, whose object name is stored in the bucket as NFD — an `e` followed
# by U+0301 COMBINING ACUTE ACCENT, so `Montre%CC%81al`, NOT the NFC
# `Montr%C3%A9al` (which 404s). It is saved under an ASCII local name for
# painless downstream use, which is also why the old gsutil-era `Montr*al`
# wildcard is not needed here: an exact percent-encoded URL is unambiguous.
FILES=(
  "Rotterdam.jsonl|2822644"
  "Ingolstadt.city.jsonl|4025892"
  "Railway.city.jsonl|4246374"
  "Montreal.city.jsonl|4822019|Montre%CC%81al.city.jsonl"
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
  # third field (remote object name) is optional; defaults to the local name
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

  echo "fetch $name <- $BASE_URL/$src"
  # `-o` a temporary, never `$dest` directly: a curl that dies midway would
  # otherwise leave a truncated file where `just bench` would find it.
  part="$dest.part"
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$part" "$BASE_URL/$src"; then
    rm -f "$part"
    echo "error: $name download failed <- $BASE_URL/$src" >&2
    exit 1
  fi
  have="$(size_of "$part")"
  if [[ "$have" != "$want" ]]; then
    rm -f "$part"
    echo "error: $name size mismatch after download (got $have, want $want)" \
      "-- refusing to benchmark against a truncated/changed file" >&2
    exit 1
  fi
  mv "$part" "$dest"
  echo "  size verified: $name ($have bytes)"
done

echo "legacy CityJSONSeq corpus fetch complete ($DATA_DIR)"
