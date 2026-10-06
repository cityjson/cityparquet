#!/usr/bin/env bash
# Fetch the CityParquet benchmark corpus (`just fetch-data`) — SEVEN real
# city models, about 1.2 GB on the wire, from 2.7 MB (Rotterdam Delfshaven)
# to 498 MB (Montréal), every one of them a single-object-table building
# dataset that yields ALL FIVE compared formats.
#
# That last property is the whole point of this corpus. The read benchmark's
# claim is a comparison BETWEEN formats, so a dataset that cannot produce
# every format contributes a row with a hole in it. Depth over breadth: a
# handful of datasets that are fully comparable beat many that are partly
# comparable.
#
# Provenance: five entries are from the CityJSON project's own dataset page,
#   https://www.cityjson.org/datasets/
# and two (Tokyo, Montréal) are derived and hosted by this project, recorded
# with their derivation in `benchmark/formats/corpus_urls.txt` — read that
# file for WHY an entry is here and how the two derived ones were built. This
# script is the other half: WHAT gets fetched, under which local name, and at
# exactly which byte size and sha256. Its test suite
# (`scripts/tests/fetch_benchmark_test.sh`, `just scripts-test`) requires the
# two lists to stay the same set, so neither can drift alone.
#
# EVERY ENTRY IS FETCHED AS CityJSON, and the `citygml` artefact is SYNTHESISED
# from it by `readbench_prepare.sh` (`citygml-tools from-cityjson -v 2.0`). See
# the CityGML synthesis section of benchmark/formats/READ_BENCHMARK.md, which
# states what it costs: no published CityGML for these datasets is CityGML 2.0,
# the only version this repository reads, and deriving every artefact from one
# source document is what makes the five rows comparable.
#
# Downloaded to DEST (default benchmark/runs/data/benchmark/, an optional
# positional argument; benchmark/runs/ is gitignored — see .gitignore).
#
# Reproducibility beats freshness. Each entry pins the byte size and the
# sha256 of the download itself; both are verified after every fetch and a
# mismatch HARD-FAILS rather than silently benchmarking against different
# bytes. Idempotent: an already-present file is skipped when its fetch receipt
# still matches it, so a re-run only collects what is missing, truncated or
# changed.
#
# Every entry is `plain`: what lands in DEST is byte-for-byte what the origin
# served. The normalisation machinery below (`gz`, `zip:MEMBER`) serves
# $CORPUS_MANIFEST inputs.
#
# Needs `curl`, and `shasum` or `sha256sum`. No `gsutil`: every entry is
# fetched over plain HTTPS.
set -euo pipefail

BENCHMARK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The whole benchmark — harness crate, scripts, corpora, results, plots — lives
# under this one directory. $BENCH_ROOT is where its generated inputs and
# results go (benchmark/runs/, ignored); overriding it moves the corpus.
MONO_ROOT="$(cd "$BENCHMARK_DIR/.." && pwd)"
BENCH_ROOT="${BENCH_ROOT:-$BENCHMARK_DIR/runs}"

# --- the pinned corpus -----------------------------------------------------
#
#   local_name | wire_bytes | form | sets | url | sha256
#
# local_name  What lands in DEST, and therefore the dataset name in every
#             benchmark CSV and chart. Pinned rather than derived because the
#             published filenames are not self-describing: Rotterdam's ships
#             as `3-20-DELFSHAVEN.city.json` and New York's as
#             `DA13_3D_Buildings_Merged.city.json`, neither of which reads
#             well in a chart legend.
#
# wire_bytes  The bytes of the download, as measured on 2026-08-23 by counting
#             a streamed GET. Every entry here is served uncompressed, so this
#             is also the on-disk size — unlike the archived corpus, where the
#             two differed by up to 30x.
#
# form        `plain` for every entry (see the header). `gz` and `zip:MEMBER`
#             remain implemented for $CORPUS_MANIFEST inputs.
#
# sets        Which benchmark set the entry can serve. EVERY entry serves BOTH
#             — that is this corpus's defining property, and the reason
#             `--only` no longer has anything to exclude. The flag is kept
#             because $CORPUS_MANIFEST inputs (the archived corpus among them)
#             still carry entries that cannot serve a default-set run.
#
# url         Verbatim from benchmark/formats/corpus_urls.txt.
#
# sha256      The sha256 of the download. Optional in a $CORPUS_MANIFEST line
#             (a sixth field); every pinned entry carries one.
#
# Ordered by wire size, smallest first, so a truncated fetch still leaves a
# usable size ladder.
#
# EVERY entry was verified before being pinned against the three gates that
# decide whether a dataset can be MEASURED at all:
#
#   1. CityJSON 2.0.
#   2. ONE CityParquet object table. `cityparquet-readbench`'s coordinator
#      refuses a multi-table package outright (`locate_cityparquet_table`), so
#      a dataset spanning two CityGML modules cannot be measured. Every entry
#      below is pure Building module (`Building` / `BuildingPart` /
#      `BuildingInstallation`).
#   3. A `gml:id` on every top-level object of the SYNTHESISED CityGML, so the
#      `id-lookup` scenario samples an id that is actually present in all five
#      artefacts. citygml-tools mints a fresh random id where the source has
#      none, which would make `citygml` score a miss beside every other
#      format's hit; `readbench_prepare.sh` re-checks this per run.
#
#   dataset               objects   LoD
#   rotterdam_delfshaven      853   2
#   vienna_102081           1,322   2
#   nyc_da13_buildings     23,777   2
#   montreal               31,415   2
#   tokyo                  49,915   0 / 1 / 2
#   zurich_building_lod2  198,699   2
#
# The 1,000,001-object 3DBAG slice that completes the format comparison is not
# fetched here: `just fetch-3dbag` cuts it from its own pinned source.
CORPUS=(
  "rotterdam_delfshaven.city.json|2731804|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/3-20-DELFSHAVEN.city.json|958460c670b4cb85a9159d1e5e566905fba8812864f64b554ecad38333badcf6"
  "vienna_102081.city.json|5635634|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Vienna_102081.city.json|849a148b5fb91ebfa6e3efe7bd78be47cc5cdd2fb6699e0cb042f44dd702cc62"
  "nyc_da13_buildings.city.json|110083137|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/DA13_3D_Buildings_Merged.city.json|c885bb6af297531c95b097aff5d38e694935ac22fe1b973cbad15948021f6cb8"
  "zurich_building_lod2.city.json|292500409|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Zurich_Building_LoD2_V10.city.json|a7dde7f306e69a4134b2bf30b8ead58fe11d3499f8b30d7596ba346aeae6f0f8"
  "tokyo.city.json|315968009|plain|default,no-citygml|https://pub-7aad9a74319741828dbafdbf5e2df201.r2.dev/cityparquet-paper/benchmark/cityjson20/tokyo.city.json|4a3e2e01bf5136e14f4d43bc993861d6d19234488f2867cc4e83e4d7dd8822ac"
  "montreal.city.json|498371022|plain|default,no-citygml|https://pub-7aad9a74319741828dbafdbf5e2df201.r2.dev/cityparquet-paper/benchmark/cityjson20/montreal.city.json|e54940e3aec2d2e4157b4dc0b5575c903066a2502e7d4a2e99c95e6180fc74cb"
)

VALID_SETS=(default no-citygml all)

# Every request carries a browser User-Agent. The corpus's origins do not
# require it, but several origins reachable through $CORPUS_MANIFEST do — the archived corpus's Kuopio and Montréal hosts
# both serve a 403 to a bare `curl/x.y`. Not a trick to get at private data:
# all of it is published open data, and those origins simply gate on the
# header.
UA='Mozilla/5.0 (X11; Linux x86_64) cityparquet-bench/1.0'

usage() {
  cat >&2 <<'USAGE'
usage: fetch_benchmark.sh [--only SET] [--allow-foreign] [DEST]

  --only SET       fetch only the entries that can serve SET:
                     default      the default format set, citygml row
                                  included (THE DEFAULT — see `sets` below)
                     no-citygml   every format except citygml
                     all          every pinned entry
                   Every entry of the PINNED corpus serves every set, so this
                   flag only distinguishes $CORPUS_MANIFEST inputs.
  --allow-foreign  proceed even though DEST holds city-model files this
                   table does not describe (they will still be measured by
                   `just bench DEST` — this only says you meant it)
  DEST             destination directory (default benchmark/runs/data/benchmark)

  $CORPUS_MANIFEST  fetch a manifest file's entries instead of the pinned
                    corpus. Same `name|bytes|form|sets|url[|sha256]` line format;
                    `#` comments and blank lines are ignored.
USAGE
}

# `default`, not `all`. For the PINNED corpus the two are identical — every
# entry serves both sets — so this default is inert and `--only` has nothing to
# exclude. It matters for $CORPUS_MANIFEST inputs: the archived corpus carries
# entries that do not merely degrade a default-set run but ABORT it (this
# script's output is fed to `just bench DEST`, which measures the whole
# directory under `set -e`, so one unfit file takes every dataset sorting after
# it down with it). Fetching those has to be asked for.
ONLY=default
ALLOW_FOREIGN=0
DEST=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --only)
      [[ $# -ge 2 ]] || {
        echo "error: --only needs a set name" >&2
        usage
        exit 1
      }
      ONLY=$2
      shift 2
      ;;
    --allow-foreign)
      ALLOW_FOREIGN=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown option '$1'" >&2
      usage
      exit 1
      ;;
    *)
      if [[ -n "$DEST" ]]; then
        echo "error: unexpected extra argument '$1'" >&2
        usage
        exit 1
      fi
      DEST=$1
      shift
      ;;
  esac
done

set_is_valid=0
for s in "${VALID_SETS[@]}"; do
  [[ "$ONLY" == "$s" ]] && set_is_valid=1
done
if [[ "$set_is_valid" -ne 1 ]]; then
  echo "error: unknown --only set '$ONLY' (valid: ${VALID_SETS[*]})" >&2
  exit 1
fi

DEST=${DEST:-$BENCH_ROOT/data/benchmark}
case "$DEST" in
  /*) DATA_DIR="$DEST" ;;
  # A relative DEST resolves against the MONOREPO root, which is where `just
  # fetch-data` runs and what every path in the root justfile is relative to.
  # Resolving it against this script's own workspace would bury the corpus
  # under lib/cityparquet-rs/.
  *) DATA_DIR="$MONO_ROOT/$DEST" ;;
esac

# The entries to fetch: the pinned corpus, or a manifest handed in through
# $CORPUS_MANIFEST (this script's own test suite uses it to serve a throwaway
# corpus of `file://` URLs; a user can use it to fetch a private subset with
# the same verified-fetch machinery).
ENTRIES=()
if [[ -n "${CORPUS_MANIFEST:-}" ]]; then
  if [[ ! -r "$CORPUS_MANIFEST" ]]; then
    echo "error: CORPUS_MANIFEST is set but $CORPUS_MANIFEST is not readable" >&2
    exit 1
  fi
  while IFS= read -r line; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    ENTRIES+=("$line")
  done <"$CORPUS_MANIFEST"
else
  ENTRIES=("${CORPUS[@]}")
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl not found on PATH (needed to fetch the benchmark corpus)" >&2
  exit 1
fi

# Split every selected entry up front, so a malformed line, an unknown form or
# a missing decompressor is reported before the first byte is downloaded.
NAMES=()
WANTS=()
FORMS=()
URLS=()
SHAS=()
# Every name the TABLE knows, `--only` notwithstanding — the foreign-input
# check below is about what this corpus is, not about what this run selected.
ALL_NAMES=()
SKIPPED_BY_SET=()
need_gunzip=0
need_unzip=0
for entry in ${ENTRIES[@]+"${ENTRIES[@]}"}; do
  IFS='|' read -r e_name e_want e_form e_sets e_url e_sha <<<"$entry"
  if [[ -z "$e_name" || -z "$e_want" || -z "$e_form" || -z "$e_sets" || -z "$e_url" ]]; then
    echo "error: malformed corpus entry (want name|bytes|form|sets|url[|sha256]): $entry" >&2
    exit 1
  fi
  if [[ -n "$e_sha" && ! "$e_sha" =~ ^[0-9a-f]{64}$ ]]; then
    echo "error: entry '$e_name' has a malformed sha256 '$e_sha'" >&2
    exit 1
  fi
  ALL_NAMES+=("$e_name")
  if [[ "$ONLY" != "all" ]] && [[ ",$e_sets," != *",$ONLY,"* ]]; then
    SKIPPED_BY_SET+=("$e_name")
    continue
  fi
  case "$e_form" in
    plain) ;;
    gz) need_gunzip=1 ;;
    zip:?*) need_unzip=1 ;;
    *)
      echo "error: entry '$e_name' has an unknown form '$e_form'" \
        "(want plain, gz or zip:MEMBER)" >&2
      exit 1
      ;;
  esac
  NAMES+=("$e_name")
  WANTS+=("$e_want")
  FORMS+=("$e_form")
  URLS+=("$e_url")
  SHAS+=("$e_sha")
done

if [[ "$need_gunzip" -eq 1 ]] && ! command -v gunzip >/dev/null 2>&1; then
  echo "error: gunzip not found on PATH (needed to normalise a .gz entry)" >&2
  exit 1
fi
if [[ "$need_unzip" -eq 1 ]] && ! command -v unzip >/dev/null 2>&1; then
  echo "error: unzip not found on PATH (needed to normalise a .zip entry)" >&2
  exit 1
fi

mkdir -p "$DATA_DIR"

# A city-model file in DEST that this table does not describe is refused, not
# warned about.
#
# DEST defaults to the directory the PREVIOUS fetcher used, so a developer who
# ran it still has 11 CityJSONSeq files sitting there — and `just bench FOLDER`
# measures every input under FOLDER. Left alone, that publishes 41 datasets as
# "the corpus" with nothing anywhere saying where 11 of them came from. A
# warning would be printed in the middle of a 30-line fetch log and read by
# nobody, so the run stops instead; `--allow-foreign` is there for the case
# where the extra input is deliberate.
#
# `-maxdepth 1`, and the extension list is the justfile's KNOWN_INPUT_FIND: a
# file `just bench` would not pick up is not this script's business. The
# receipts live in a dot-directory and end in `.receipt`, so they match
# neither the depth nor the patterns.
if [[ "$ALLOW_FOREIGN" -ne 1 ]]; then
  foreign=()
  while IFS= read -r found; do
    [[ -z "$found" ]] && continue
    base="$(basename "$found")"
    [[ "$base" == "metadata.json" ]] && continue
    known=0
    for n in ${ALL_NAMES[@]+"${ALL_NAMES[@]}"}; do
      [[ "$base" == "$n" ]] && known=1 && break
    done
    [[ "$known" -eq 1 ]] || foreign+=("$base")
  done < <(find "$DATA_DIR" -maxdepth 1 -type f \
    \( -name '*.json' -o -name '*.jsonl' -o -name '*.gml' -o -name '*.citygml' \
    -o -name '*.xml' \) | sort)
  if [[ "${#foreign[@]}" -gt 0 ]]; then
    echo "error: $DATA_DIR holds ${#foreign[@]} city-model file(s) this corpus does not describe:" >&2
    printf '         %s\n' "${foreign[@]}" >&2
    echo "       \`just bench\` measures EVERY input under that directory, so these would be" >&2
    echo "       published alongside the corpus as if they were part of it. Move or delete" >&2
    echo "       them, or re-run with --allow-foreign if you meant it." >&2
    exit 1
  fi
fi

WORK="$DATA_DIR/.fetch"
RECEIPTS="$DATA_DIR/.receipts"

# Nothing half-written survives this script, however it exits: a leftover
# `.part` is litter, but a leftover half-normalised artefact in DEST would be
# picked up as an input by the very next `just bench`.
cleanup() {
  rm -rf "$WORK"
  rmdir "$RECEIPTS" 2>/dev/null || true
}
trap cleanup EXIT

size_of() {
  # portable byte size of $1 (macOS/BSD stat vs GNU stat)
  stat -f%z "$1" 2>/dev/null || stat -c%s "$1"
}

sha256_of() {
  # portable sha256 of $1 (macOS/BSD shasum vs GNU sha256sum)
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

fetched=0
skipped=0
# `${a[@]+"${a[@]}"}`, never a bare `"${!a[@]}"`: under `set -u` an EMPTY array
# is an unset variable to bash 4.3 (the macOS system bash), so `--only` that
# selects nothing would abort here instead of reporting an empty selection.
for i in ${NAMES[@]+"${!NAMES[@]}"}; do
  name="${NAMES[$i]}"
  want="${WANTS[$i]}"
  form="${FORMS[$i]}"
  url="${URLS[$i]}"
  sha="${SHAS[$i]}"
  dest="$DATA_DIR/$name"
  receipt="$RECEIPTS/$name.receipt"

  # Skip-if-present is a SIZE check, not an existence check. The pinned size
  # is the size on the wire, which for a normalised entry is not the size on
  # disk, so the receipt written after a successful fetch records both: the
  # wire size proves the file came from the bytes this table pins, the disk
  # size proves nobody has truncated it since.
  if [[ -f "$dest" && -r "$receipt" ]]; then
    read -r had_want had_disk <"$receipt" || true
    have="$(size_of "$dest")"
    if [[ "$had_want" == "$want" && "$have" == "${had_disk:-}" ]]; then
      echo "skip $name (already fetched, $have bytes on disk)"
      skipped=$((skipped + 1))
      continue
    fi
    echo "warn $name: size mismatch on existing file (disk $have, expected ${had_disk:-?};" \
      "wire ${had_want:-?}, pinned $want) -- re-fetching" >&2
  elif [[ -f "$dest" ]]; then
    echo "warn $name: present but unverifiable (no fetch receipt) -- re-fetching" >&2
  fi

  mkdir -p "$WORK"
  part="$WORK/$name.part"
  echo "fetch $name <- $url"
  if ! curl -fsSL --retry 3 --retry-delay 2 -A "$UA" -o "$part" "$url"; then
    echo "error: $name download failed <- $url" >&2
    exit 1
  fi
  have="$(size_of "$part")"
  if [[ "$have" != "$want" ]]; then
    echo "error: $name size mismatch after download (got $have, want $want)" \
      "-- refusing to benchmark against a truncated/changed file" >&2
    exit 1
  fi
  echo "  size verified: $name ($have bytes on the wire)"
  if [[ -n "$sha" ]]; then
    got="$(sha256_of "$part")"
    if [[ "$got" != "$sha" ]]; then
      echo "error: $name sha256 mismatch after download (got $got, want $sha)" \
        "-- refusing to benchmark against changed bytes" >&2
      exit 1
    fi
    echo "  sha256 verified: $name"
  fi

  out="$WORK/$name.out"
  case "$form" in
    plain)
      mv "$part" "$out"
      ;;
    gz)
      gunzip -c "$part" >"$out"
      echo "  normalised: gunzipped to $(size_of "$out") bytes"
      ;;
    zip:*)
      member="${form#zip:}"
      # Exactly one member must match, or "the pinned member" is a guess:
      # `unzip -p` would concatenate every match into one nonsense file.
      #
      # The `|| true` is load-bearing, and its absence made the guard below
      # UNREACHABLE for the case it most needs to catch: `unzip -Z1` exits 11
      # when nothing matches, so under `set -euo pipefail` this assignment
      # killed the script at exit 11 with nothing printed at all — the silent
      # abort landing on whoever pins the next zip entry, after their download.
      matches="$( { unzip -Z1 "$part" "$member" 2>/dev/null || true; } \
        | wc -l | tr -d ' ')"
      if [[ "$matches" != "1" ]]; then
        echo "error: $name: '$member' matches $matches members of the archive (want exactly 1)" >&2
        exit 1
      fi
      unzip -p "$part" "$member" >"$out"
      echo "  normalised: extracted '$member' ($(size_of "$out") bytes)"
      ;;
  esac

  mv "$out" "$dest"
  mkdir -p "$RECEIPTS"
  printf '%s %s\n' "$want" "$(size_of "$dest")" >"$receipt"
  fetched=$((fetched + 1))
done

if [[ "${#SKIPPED_BY_SET[@]}" -gt 0 ]]; then
  echo "not in set '$ONLY' (${#SKIPPED_BY_SET[@]}): ${SKIPPED_BY_SET[*]}"
fi
echo "benchmark corpus fetch complete: $fetched fetched, $skipped already present ($DATA_DIR)"
