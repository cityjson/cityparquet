#!/usr/bin/env bash
# Fetch the CityParquet benchmark corpus (`just fetch-data`) — the
# catalogue-derived spread of REAL published city models the read benchmark
# measures: 30 datasets, 6.5 GB on the wire, from 923 KB (a 3DBAG tile) to
# 1.9 GB (Freiburg's city-wide LoD2), in the two formats the data actually
# ships in (CityGML 2.0 `.gml`, CityJSON 2.0 `.city.json`) rather than in one
# pre-converted intermediate.
#
# Provenance: every URL comes from the city3d STAC catalogue, recorded with
# its collection and its verification date in `bench/catalogue_benchmark_urls.txt`
# — read that file for WHY an entry is here and why the blocked ones are not.
# This script is the other half: WHAT gets fetched, under which local name,
# and at exactly which byte size. Its test suite
# (`scripts/tests/fetch_benchmark_test.sh`, `just scripts-test`) requires the
# two lists to stay the same set, so neither can drift alone.
#
# Downloaded to DEST (default bench/data/benchmark/, an optional positional
# argument; the whole of bench/data/ is gitignored — see .gitignore).
#
# Reproducibility beats freshness. Each entry pins the byte size of the
# download itself, measured 2026-08-16; the size is verified after every
# fetch and a mismatch HARD-FAILS rather than silently benchmarking against
# different bytes. Idempotent: an already-present file is skipped when it
# still matches what was fetched, so a re-run only collects what is missing,
# truncated or changed.
#
# NORMALISED ON ARRIVAL: a `.gz` is gunzipped and a `.zip`'s pinned member is
# extracted, so what lands in DEST is always a plain `.gml` / `.city.json`
# that the recipes downstream can see (`just bench` and `just convert-all`
# discover inputs by extension — a file left gzipped is a file they skip).
# NOTE that the on-disk size is therefore often much larger than the pinned
# wire size: Estonia's 11 MB national canopy archive expands to 323 MB, and
# Kuopio's 1.5 GB archive to a 982 MB GML.
#
# Needs `curl`; `gunzip` and `unzip` only if the selection includes an entry
# that needs them. No `gsutil`: every entry is fetched over plain HTTPS.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --- the pinned corpus -----------------------------------------------------
#
#   local_name | wire_bytes | form | sets | url
#
# local_name  What lands in DEST, and therefore the dataset name in every
#             benchmark CSV and chart. Pinned rather than derived because 11
#             of these URLs have no filename in them at all: the CRAIG and
#             Estonia endpoints carry it in a query parameter (`&files=`,
#             `&f=`), and the path component is the same `/download` or
#             `/index.php` for every dataset they publish.
#
# wire_bytes  The bytes of the DOWNLOAD (so, the compressed size for a `.gz`
#             or `.zip`), as measured on 2026-08-16 — by `Content-Length` /
#             `Content-Range` where the origin publishes one, and by counting
#             a streamed GET where it does not. The six PLATEAU tiles served
#             with `x-goog-stored-content-encoding: gzip` fall in that second
#             group: the CDN gunzips them in flight, so the bytes that arrive
#             are the decompressed GML and no header states their length.
#
# form        `plain` (write it through), `gz` (gunzip it), or
#             `zip:MEMBER` (extract exactly that one member — these archives
#             ship the model beside thousands of texture images, and only the
#             model is an input; no measured runner opens the images).
#
# sets        Which benchmark set the entry can serve:
#               default      the DEFAULT format set, `citygml` row included
#               no-citygml   everything EXCEPT the `citygml` row
#             Two entries are `no-citygml` only, and both would poison a
#             default-set run rather than merely weaken it — the coordinator
#             aborts a dataset when any child fails, and
#             `readbench_prepare.sh` refuses the input outright:
#               riga_atgazene_lod2.gml    703 of 703 top-level objects carry
#                 NO gml:id (identity lives in a `gen:intAttribute` named
#                 OBJECTID). citygml-tools mints a fresh random id for each,
#                 so `citygml`'s id-lookup would query an id that is not in
#                 the .gml at all and score a miss beside every other format's
#                 hit. Every other format reads it fine (the derived CityJSON
#                 is self-consistent), so it stays in the corpus.
#               plateau_chuo_brid.gml     this repository's CityGML reader
#                 hard-errors on it: its solids reference polygons defined in
#                 a different city object ("cross-building/shared geometry is
#                 out of scope"). Again a `citygml`-only defect — citygml-tools
#                 resolves the references, so every derived artefact is fine.
#             Neither degrades a default-set run, both ABORT one: the
#             coordinator bails when a child fails, `readbench_prepare.sh`
#             refuses Riga outright, and `just bench`'s folder loop runs under
#             `set -e` — so one of these files in the directory does not lose
#             its own dataset, it kills the loop and takes every dataset
#             sorting after it down with it. That is why `--only` DEFAULTS to
#             `default`. Ask for them with `--only no-citygml` (or `all`), and
#             measure them with an explicit `--formats` list that omits
#             `citygml`.
#
# url         Verbatim from bench/catalogue_benchmark_urls.txt.
#
# Ordered by wire size, smallest first, so a truncated fetch still leaves a
# usable spread of formats, geographies and CityGML modules.
#
# EVERY entry was verified before being pinned: its declared CityGML version
# is 2.0, every top-level object carries a gml:id (except Riga, above), and
# its city objects all resolve to ONE CityParquet object table. That last
# check is not cosmetic — `cityparquet-readbench`'s coordinator refuses a
# multi-table package outright (`locate_cityparquet_table`), so a two-family
# dataset cannot be measured at all. Three catalogue entries were dropped for
# failing it and are recorded, with their reasons, in the EXCLUDED section of
# bench/catalogue_benchmark_urls.txt.
CORPUS=(
  "3dbag_amsterdam_10-432-718.city.json|944802|gz|default,no-citygml|https://data.3dbag.nl/v20250903/tiles/10/432/718/10-432-718.city.json.gz"
  "3dbag_amsterdam_10-432-720.city.json|1705153|gz|default,no-citygml|https://data.3dbag.nl/v20250903/tiles/10/432/720/10-432-720.city.json.gz"
  "estonia_anija_lod2.gml|2581034|zip:hooned_lod2-Anija_vald.gml|default,no-citygml|https://geoportaal.maaamet.ee/index.php?lang_id=2&plugin_act=otsing&andmetyyp=hooned_lod2&dl=1&f=hooned_lod2-Anija_vald-citygml.zip&page_id=837"
  "rotterdam_delfshaven.city.json|2731804|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/3-20-DELFSHAVEN.city.json"
  "plateau_chuo_brid.gml|2800795|plain|no-citygml|https://assets.cms.plateau.reearth.io/assets/0c/b282cf-c242-4dd7-af9b-4ec3d4373018/13102_chuo-ku_pref_2023_citygml_2_op/udx/brid/53394611_brid_6697_op.gml"
  "plateau_chuo_fld.gml|5439116|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/0c/b282cf-c242-4dd7-af9b-4ec3d4373018/13102_chuo-ku_pref_2023_citygml_2_op/udx/fld/pref/kandagawa-ryuiki/53394611_fld_6697_l2_op.gml"
  "plateau_yokohama_squr.gml|5569485|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/04/96d45a-a30b-4c9f-88c0-c1323b5a0f86/14100_yokohama-shi_city_2024_citygml_2_op/udx/squr/53391368_squr_6697_op.gml"
  "vienna_102081.city.json|5635634|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Vienna_102081.city.json"
  "riga_atgazene_lod2.gml|10509018|plain|no-citygml|https://data.gov.lv/dati/dataset/rigas-apkaimju-lod2-modeli/resource/2e00bb17-0c84-4ec4-9626-48002d0a47c8/download/atgazene_lod2.gml"
  "estonia_canopies_lod1.gml|11718012|zip:katusealused_lod1-eesti.gml|default,no-citygml|https://geoportaal.maaamet.ee/index.php?lang_id=2&plugin_act=otsing&andmetyyp=katusealused_lod1&dl=1&f=katusealused_lod1-eesti-citygml.zip&page_id=837"
  "craig_valence_lod3_1.gml|16185668|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_ZAE%2F26_Valence_Romans_Agglo&files=02_Valence_LOD3_1.gml"
  "craig_clermont_secteur5_textured.gml|23101949|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2F2021_aura_3d%2F02_Bati3D%2FCityGML%2FSecteur_05_Clermont&files=Secteur_5_Textured_L93.gml"
  "craig_voironnais_lod2_solar.gml|27956169|zip:38_Voironnais_LoD2.gml|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d_isere%2F02_CIM%2FCityGML%2FCA_Pays_Voironnais&files=2024_BC2_Voironnais_2020_LoD2_solaire.zip"
  "plateau_chuo_tran.gml|36597975|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/0c/b282cf-c242-4dd7-af9b-4ec3d4373018/13102_chuo-ku_pref_2023_citygml_2_op/udx/tran/53394611_tran_6697_op.gml"
  "craig_vichy_2016_lod3.gml|52921317|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_ZAE%2F03_Vichy_Communaute&files=10_Vichy_2016_LOD3.gml"
  "luxembourg_diekirch_bastendorf_lod2.gml|57876265|plain|default,no-citygml|https://download.data.public.lu/resources/batiments-3d-lod-2-3-level-of-detail-2-3/20190603-110058/lod2-batiments-diekirch-bastendorf.gml"
  "craig_riom_lod2_2.gml|59620390|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_Building%2FRiom_Chatel-Guyon&files=14_Riom_LOD2_2.gml"
  "plateau_chuo_veg.gml|63744856|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/0c/b282cf-c242-4dd7-af9b-4ec3d4373018/13102_chuo-ku_pref_2023_citygml_2_op/udx/veg/53394611_veg_6697_op.gml"
  "craig_riom_lod2_1.gml|78433203|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_Building%2FRiom_Chatel-Guyon&files=14_Riom_LOD2_1.gml"
  "nyc_da13_buildings.city.json|110083137|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/DA13_3D_Buildings_Merged.city.json"
  "plateau_chuo_frn.gml|117380205|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/0c/b282cf-c242-4dd7-af9b-4ec3d4373018/13102_chuo-ku_pref_2023_citygml_2_op/udx/frn/53394611_frn_6697_op.gml"
  "craig_clermont_lod3_1.gml|161907437|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_ZAE%2F63_Clermont_Auvergne_Metropole&files=03_Clermont_LOD3_1.gml"
  "plateau_chuo_bldg.gml|168306746|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/0c/b282cf-c242-4dd7-af9b-4ec3d4373018/13102_chuo-ku_pref_2023_citygml_2_op/udx/bldg/53394611_bldg_6697_op.gml"
  "craig_vichy_unesco_lod3.gml|215430331|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_Building%2FVichy_UNESCO&files=2024_BC1_Vichy_UNESCO_2021_LoD3.gml"
  "zurich_building_lod2.city.json|292500409|plain|default,no-citygml|https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Zurich_Building_LoD2_V10.city.json"
  "craig_clermont_lod2_textured_1.gml|320756374|plain|default,no-citygml|https://drive.opendata.craig.fr/s/opendata/download?path=%2F3d%2Fbati3d%2F02_CIM%2FCityGML%2F2024_Building%2FClermont&files=Clermont_LoD2_texture_1.gml"
  "plateau_edogawa_htd.gml|532458174|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/92/28e36f-472f-42dc-8d9c-67348e3a7261/13123_edogawa-ku_pref_2023_citygml_2_op/udx/htd/13_1/533946_htd_6697_op.gml"
  "plateau_edogawa_luse.gml|561498161|plain|default,no-citygml|https://assets.cms.plateau.reearth.io/assets/92/28e36f-472f-42dc-8d9c-67348e3a7261/13123_edogawa-ku_pref_2023_citygml_2_op/udx/luse/533946_luse_6697_op.gml"
  "kuopio_lod2_2_textured.gml|1531892325|zip:building.gml|default,no-citygml|https://avoindata.suomi.fi/data/dataset/1cda485e-0c0a-4f77-9f7d-185bae9144f7/resource/b4aa97c2-f41c-4863-82f0-aaa88687a20b/download/kuopion_3d_rakennukset_citygml_lod2_2_2022_textured.zip"
  "freiburg_lod2.gml|1998672154|plain|default,no-citygml|https://geoportal.freiburg.de/stadtmodell/20240426_Freiburg_LoD2.gml"
)

VALID_SETS=(default no-citygml all)

# Some of these origins serve a 403 to a bare `curl/x.y` — Kuopio's
# avoindata.suomi.fi redirects and refuses, and Montréal's does the same on
# the legacy corpus — so every request carries a browser User-Agent. Not a
# trick to get at private data: all of it is published open data, and the
# origins simply gate on the header.
UA='Mozilla/5.0 (X11; Linux x86_64) cityparquet-bench/1.0'

usage() {
  cat >&2 <<'USAGE'
usage: fetch_benchmark.sh [--only SET] [--allow-foreign] [DEST]

  --only SET       fetch only the entries that can serve SET:
                     default      the default format set, citygml row
                                  included (THE DEFAULT — see `sets` below)
                     no-citygml   every format except citygml
                     all          every pinned entry
  --allow-foreign  proceed even though DEST holds city-model files this
                   table does not describe (they will still be measured by
                   `just bench DEST` — this only says you meant it)
  DEST             destination directory (default bench/data/benchmark)

  $CORPUS_MANIFEST  fetch a manifest file's entries instead of the pinned
                    corpus. Same `name|bytes|form|sets|url` line format;
                    `#` comments and blank lines are ignored.
USAGE
}

# `default`, not `all`: the two `no-citygml` entries below do not degrade a
# default-set run, they ABORT it (see the `sets` notes in the table), and this
# script's output is fed to `just bench DEST`, which measures the whole
# directory. Fetching them has to be asked for.
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

DEST=${DEST:-bench/data/benchmark}
case "$DEST" in
  /*) DATA_DIR="$DEST" ;;
  *) DATA_DIR="$REPO_ROOT/$DEST" ;;
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
# Every name the TABLE knows, `--only` notwithstanding — the foreign-input
# check below is about what this corpus is, not about what this run selected.
ALL_NAMES=()
SKIPPED_BY_SET=()
need_gunzip=0
need_unzip=0
for entry in ${ENTRIES[@]+"${ENTRIES[@]}"}; do
  IFS='|' read -r e_name e_want e_form e_sets e_url <<<"$entry"
  if [[ -z "$e_name" || -z "$e_want" || -z "$e_form" || -z "$e_sets" || -z "$e_url" ]]; then
    echo "error: malformed corpus entry (want name|bytes|form|sets|url): $entry" >&2
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
