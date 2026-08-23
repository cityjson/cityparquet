#!/usr/bin/env bash
# Fetch the pinned heterogeneity corpus.
#
# Each dataset is verified against a pinned SHA-256, so a silently changed
# upstream file fails the run rather than quietly altering published
# numbers. On first run the checksum file does not exist: the script
# writes it and tells you to commit it. Thereafter it verifies.
set -euo pipefail

# Resolved to an ABSOLUTE path before any `cd` below: `SUMS` is read again
# after this script changes into `$DEST`, and a path built from a relative
# `dirname "$0"` (e.g. the brief's own literal `$(dirname "$0")/corpus.sha256`)
# would then resolve against the wrong directory and silently look for (or
# write) the checksum file inside `$DEST` instead of next to this script.
HERE="$(cd "$(dirname "$0")" && pwd)"
SUMS="$HERE/corpus.sha256"
DEST="${1:-data}"
mkdir -p "$DEST"

# <local filename>=<upstream URL>. Local filenames are plain ASCII to match
# every downstream `just derive-params data/<name>`/`just bench <name>`
# invocation in this task; Montreal's upstream object name is NOT plain
# ASCII, which is a real fetch bug the literal brief text (bare
# "Montreal.city.jsonl") would have hit as a 404 — confirmed against the
# bucket listing (`storage/v1/b/cityjson/o?prefix=benchmark_dataset/`): the
# real object is `Montréal.city.jsonl`, i.e. "Montre" + a COMBINING
# ACUTE ACCENT (U+0301, NFD) + "al", not the precomposed "é" (U+00E9, NFC)
# a hand-typed URL would normally use. Percent-encoded below
# (`Montre%CC%81al.city.jsonl`, `%CC%81` = UTF-8 for U+0301) so curl
# requests the exact upstream bytes; the pinned SHA-256 covers the content,
# not the filename, so this rename is lossless.
DATASETS=(
  "Montreal.city.jsonl=https://storage.googleapis.com/cityjson/benchmark_dataset/Montre%CC%81al.city.jsonl"
  "Vienna.city.jsonl=https://storage.googleapis.com/cityjson/benchmark_dataset/Vienna.city.jsonl"
  "Zurich.city.jsonl=https://storage.googleapis.com/cityjson/benchmark_dataset/Zurich.city.jsonl"
  "lod3_railway.city.json=https://storage.googleapis.com/cityjson/lod3_railway.city.json"
)

for entry in "${DATASETS[@]}"; do
    name="${entry%%=*}"
    url="${entry#*=}"
    out="$DEST/$name"
    echo ">> $name"
    curl -sSfLo "$out" "$url"
done

cd "$DEST"
FILES=()
for entry in "${DATASETS[@]}"; do FILES+=("${entry%%=*}"); done

if [[ -f "$SUMS" ]]; then
    echo ">> verifying against $SUMS"
    sha256sum -c "$SUMS"
    echo "corpus verified"
else
    sha256sum "${FILES[@]}" > "$SUMS"
    echo "WROTE $SUMS — inspect it, then commit it."
    echo "Subsequent runs will verify against it and fail on any change."
fi
