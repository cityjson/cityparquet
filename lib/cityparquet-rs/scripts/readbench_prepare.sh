#!/usr/bin/env bash
# Prepare the per-format inputs for the read-benchmark milestone: given one
# CityGML/CityJSON/CityJSONSeq INPUT, produce a same-content package in each
# format the read benchmark compares (`just readbench-prepare`). Which formats
# are built is chosen with `--formats`; by default every artefact this script
# knows how to build:
#
#   citygml              OUTDIR/<x>.gml               CityGML 2.0 — copied from a CityGML INPUT, else synthesised
#   cityjson             OUTDIR/<x>.city.json         one whole-document CityJSON
#   cityjsonseq          OUTDIR/<x>.city.jsonl        CityJSONSeq
#   cityparquet          OUTDIR/<x>.parquet/          core-profile CityParquet package (source order)
#   cityparquet-hilbert  OUTDIR/<x>-hilbert.parquet/  CityParquet package, Hilbert-ordered rows
#   flatcitybuf          OUTDIR/<x>.fcb               FlatCityBuf, spatial index + ALL-attribute B+Tree index
#   cityjsonseq-gz       OUTDIR/<x>.jsonl.gz          the CityJSONSeq, gzip -9
#
# EVERY ONE OF THOSE IS A REAL FILE IN OUTDIR, `cityjsonseq` included — from a
# CityJSONSeq INPUT it is a copy, from a CityJSON INPUT it is cut with `cjseq
# cat`. It used to be a no-op ("the artefact IS the INPUT, read in place"),
# which held only while every INPUT was a `.city.jsonl`: on a `.gml` or
# `.city.json` INPUT the benchmark then measured the INPUT'S OWN FORMAT and
# published it as CityJSONSeq (~8x too slow on a real .gml, and for a
# `.city.json` two rows over one file). The coordinator now resolves EVERY
# format to an OUTDIR artefact (`Format::artefact`), so an artefact that is
# not built is a format that is skipped — never one that is silently
# substituted.
#
# `<x>` is INPUT's basename minus its known input extension — see
# `KNOWN_INPUT_EXTENSIONS` below, which is one of four implementations of the
# same convention (this script, the justfile, the coordinator's Rust
# `naming::strip_known_extension`, and `scripts/readbench_duckdb.sh`'s
# package-name counterpart), held in lockstep by
# `crates/cityparquet-readbench/tests/strip_extension.rs`.
#
# THE CONVERSION CHAIN, AND WHY IT RUNS FORWARDS ONLY:
#
#   CityGML --citygml-tools to-cityjson--> CityJSON --cjseq cat--> CityJSONSeq
#                                                 |           |--fcb ser -A--------> FlatCityBuf
#                                                 |           |--cityparquet convert-> CityParquet
#                                                 |
#                                                 |--citygml-tools from-cityjson -v 2.0--> CityGML
#                                                    (only when INPUT is not itself CityGML)
#
# Each artefact derives from the one before it, and FlatCityBuf and
# CityParquet derive from the SAME CityJSONSeq — that is what makes their
# comparison fair. NOTHING derives from CityParquet: `cityparquet export`
# could emit the CityJSON artefacts and it would be tempting, but deriving a
# competitor's input from the format under test would favour it.
#
# WHY `cjseq` CUTS THE SEQ, and not citygml-tools. citygml-tools 2.5.0 *can*
# write CityJSONSeq directly (`-l/--json-lines`), so this is a choice, not a
# limitation. Three reasons against using it:
#
#   1. `cityjson` is itself a measured format, so the whole CityJSON document
#      has to be produced anyway. `-l` would not replace that step, it would
#      add a SECOND, independent citygml-tools run beside it.
#   2. That second run would derive the CityJSONSeq from the CityGML again,
#      instead of from the CityJSON already in hand — breaking the one rule
#      this chain exists to keep: each artefact derives from the one before it.
#   3. For input whose objects have no `gml:id`, citygml-tools mints fresh ids
#      per run, so the two runs would disagree — the `cityjson` and
#      `cityjsonseq` artefacts would carry DIFFERENT ids for the same objects,
#      silently desynchronising id-lookup across two measured formats. (The
#      verification below refuses such input outright, but the argument holds
#      regardless.)
#
# `cjseq` is unavoidable in any case: from a CityJSON/CityJSONSeq INPUT the
# CityJSON artefact is cut by `cjseq collect`, which citygml-tools cannot do at
# all — and never by a CityParquet export.
#
# CITYGML IS SYNTHESISED FROM A NON-CityGML INPUT, and this reverses an
# earlier rule of this script. It used to report the `citygml` artefact as
# "not derivable" and skip it, on the grounds that a round-trip artefact is
# not the source data. That was the right call for a corpus of published
# `.gml` files. It is the wrong one for the corpus this benchmark now uses,
# for a reason that outweighs it:
#
#   The read benchmark's claim is a comparison BETWEEN formats. A dataset that
#   produces seven artefacts and skips the eighth does not weaken that
#   comparison, it removes the baseline from it — and under the previous
#   corpus the skipped ones were exactly the datasets a reader recognises
#   (3DBAG, Rotterdam, Vienna, NYC, Zurich all ship as CityJSON). Worse, the
#   published `.gml` beside a `.city.json` is usually NOT a usable substitute:
#   of the nine on cityjson.org, six are CityGML 1.0 and two are 3.0, and this
#   repository's reader accepts only 2.0.
#
# So every artefact, CityGML included, is now derived from ONE source
# document, which is what makes the eight rows content-comparable in the first
# place. The cost is real and must be quoted with the numbers: the `citygml`
# row then measures citygml-tools' SERIALISATION, not a published file. Two
# things bound that cost, both in bench/READ_BENCHMARK.md's CityGML synthesis
# section — the synthesised document is close in size to the published
# original where one exists (Rotterdam: 14.0 MB vs 16.5 MB), and a source
# whose LoDs CityGML 2.0 cannot express loses some (3DBAG's LoD 1.2/1.3 both
# collapse to `lod1Solid`).
#
# A CityGML INPUT is still COPIED, never round-tripped: where the source data
# is already CityGML, that is what gets measured.
#
# The `-A` (index-all-attributes) flag on `fcb ser` is REQUIRED, not
# cosmetic: the later attribute-filter benchmark needs FCB's B+-tree
# attribute index to exist, and the spatial (R-tree) index is on by default
# so both of FCB's indexed-query paths are available for comparison.
#
# EXTERNAL TOOLS ARE GUARDED PER FORMAT, not up front: `fcb` is only required
# when `flatcitybuf` was requested, `citygml-tools`/`cjseq`/`jq` only when the
# request actually needs that hop of the chain, and the release CityParquet
# CLI is only built when a CityParquet artefact was requested. A missing `fcb`
# used to kill every run of this script, including runs that never asked for
# FlatCityBuf.
#
# citygml-tools comes from `just fetch-tools` (scripts/fetch_tools.sh, which
# owns the pinned version and its sha256) and is resolved via bench/tools/,
# $CITYGML_TOOLS or PATH — this script never repeats the version pin.
#
# A REQUESTED FORMAT THAT CANNOT BE BUILT IS AN ERROR HERE, never a silent
# skip: this script's job is to build what was asked for or say precisely why
# it could not. Skipping-and-continuing is the coordinator's job
# (`crates/cityparquet-readbench/src/coordinator.rs` warns about a missing
# artefact and carries on with the rest of the matrix).
#
# Idempotent: an output that already exists and passes its validity check
# (non-empty file / non-empty directory) is skipped, so a re-run only fills
# in what is missing. This is a local dev tool; like the fetch_*.sh scripts,
# it is NOT wired into `just check`/CI. Its own tests live in
# `scripts/tests/readbench_prepare_test.sh`.
set -euo pipefail

# The format vocabulary, in the benchmark's canonical order. Owned by
# `Format::ALL` in `crates/cityparquet-readbench/src/format.rs`; this is a
# copy, because a shell script cannot import a Rust enum. Keep it on ONE line
# and in that order: `scripts/tests/readbench_prepare_test.sh` reads both
# lists out of their own sources and fails if they disagree (a duplicated
# vocabulary drifting apart is exactly how this benchmark's CSV header
# contract ended up with three incompatible versions).
#
# `duckdb-parquet` is deliberately absent: it is an SQL-engine baseline that
# `scripts/readbench_duckdb.sh` runs over an already-prepared CityParquet
# package, so there is no artefact for this script to build (the Rust side
# says the same with `Artefact::NotCoordinated`).
VALID_FORMATS=(citygml cityjson cityjsonseq cityjsonseq-gz flatcitybuf cityparquet cityparquet-hilbert)

# What `--formats` defaults to: the full format-comparison set, i.e. every
# artefact this script can produce. `duckdb-parquet` is absent for the reason
# given above (no artefact of its own); `citygml` is present but is skipped
# with a report, not built, when INPUT is not CityGML.
# NOT the same list as the coordinator's `DEFAULT_FORMATS` (which is what the
# benchmark MEASURES by default) — this one can only ever name formats a
# build step below exists for, so that a bare run never fails on its own
# default.
DEFAULT_BUILD_FORMATS=(citygml cityjson cityjsonseq cityjsonseq-gz flatcitybuf cityparquet cityparquet-hilbert)

# The INPUT-EXTENSION CONVENTION, most specific first — the same list, in the
# same order, as `KNOWN_INPUT_EXTENSIONS` in
# `crates/cityparquet-readbench/src/naming.rs` and in the justfile. Keep the
# array on ONE line and the function's closing brace in column 1:
# `crates/cityparquet-readbench/tests/strip_extension.rs` extracts both out of
# this file and RUNS them against the Rust implementation's own table, so a
# copy that drifts fails `just check` rather than silently misnaming every
# artefact of a `.gml` input.
KNOWN_INPUT_EXTENSIONS=(.city.jsonl .city.json .citygml .jsonl .json .gml .xml)

strip_known_extension() {
  local name=$1 ext
  for ext in "${KNOWN_INPUT_EXTENSIONS[@]}"; do
    if [[ "$name" == *"$ext" ]]; then
      printf '%s' "${name%"$ext"}"
      return
    fi
  done
  printf '%s' "$name"
}

usage() {
  cat >&2 <<EOF
usage: $0 [--formats a,b,c] INPUT [OUTDIR]
  INPUT      CityGML (.gml/.citygml), CityJSON (.city.json) or CityJSONSeq
             (.city.jsonl) file
  OUTDIR     default: bench/data/readbench
  --formats  comma-separated formats to build, from: ${VALID_FORMATS[*]}
             (default: ${DEFAULT_BUILD_FORMATS[*]})
EOF
}

FORMATS_ARG=""
POSITIONAL=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --formats)
      [[ $# -ge 2 ]] || { echo "error: --formats requires a value" >&2; exit 1; }
      FORMATS_ARG=$2
      shift 2
      ;;
    --formats=*)
      FORMATS_ARG=${1#--formats=}
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "error: unknown option '$1'" >&2
      usage
      exit 1
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

if [[ ${#POSITIONAL[@]} -lt 1 || ${#POSITIONAL[@]} -gt 2 ]]; then
  usage
  exit 1
fi

INPUT=${POSITIONAL[0]}
OUTDIR=${POSITIONAL[1]:-bench/data/readbench}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -f "$INPUT" ]]; then
  echo "error: input file not found: $INPUT" >&2
  exit 1
fi

# Requested formats, in the order given; validated against VALID_FORMATS
# before anything is built, so a typo costs nothing.
REQUESTED=()
if [[ -n "$FORMATS_ARG" ]]; then
  IFS=',' read -r -a REQUESTED <<<"$FORMATS_ARG"
else
  REQUESTED=("${DEFAULT_BUILD_FORMATS[@]}")
fi

is_valid_format() {
  local candidate=$1 known
  for known in "${VALID_FORMATS[@]}"; do
    if [[ "$known" == "$candidate" ]]; then
      return 0
    fi
  done
  return 1
}

for fmt in "${REQUESTED[@]}"; do
  if [[ -z "$fmt" ]]; then
    echo "error: empty format name in --formats '$FORMATS_ARG'" >&2
    exit 1
  fi
  if ! is_valid_format "$fmt"; then
    echo "error: unknown format '$fmt'; expected one of: ${VALID_FORMATS[*]}" >&2
    exit 1
  fi
done

# Was FORMAT requested?
want() {
  local candidate=$1 fmt
  for fmt in "${REQUESTED[@]}"; do
    if [[ "$fmt" == "$candidate" ]]; then
      return 0
    fi
  done
  return 1
}

require_tool() {
  local tool=$1 reason=$2
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: $tool not found on PATH (needed for $reason)" >&2
    exit 1
  fi
}

# --- preflight -------------------------------------------------------------
# One pass over the request: reject what cannot be built, and require only
# the tools the request actually needs. Everything below this point is
# expected to succeed.

# Two numbers about a CityGML document, printed as "MEMBERS WITH_GML_ID":
# how many top-level <cityObjectMember> elements it has, and how many of those
# members' city objects carry a `gml:id`.
#
# awk with RS="<" rather than grep, because the count must be tag-oriented,
# not line-oriented: a perfectly valid
#
#     <core:cityObjectMember
#         xlink:href="#x">
#
# splits an opening tag across two lines, and a line-oriented regex counts it
# as zero — hard-failing a good document. Splitting on '<' makes every record
# exactly one tag plus its following text, whatever the line breaks.
#
# The gml:id half exists because citygml-tools mints a FRESH RANDOM id for an
# object that has none, so the id the benchmark samples from the derived
# CityJSONSeq is absent from the .gml entirely — see the refusal below.
citygml_counts() {
  awk '
    BEGIN { RS = "<"; members = 0; ids = 0; expect = 0 }
    {
      tag = $0
      p = index(tag, ">")
      if (p > 0) tag = substr(tag, 1, p - 1)
      if (tag == "" || tag ~ /^[!?]/ || tag ~ /^\//) next   # text, comment/PI, closing
      name = tag
      sub(/[ \t\r\n\/].*$/, "", name)
      bare = name
      sub(/^[^:]*:/, "", bare)
      if (bare == "cityObjectMember") {
        members++
        # A self-closing member (an xlink reference) has no city object of its
        # own following it, so nothing may be attributed to it.
        expect = (tag ~ /\/[ \t\r\n]*$/) ? 0 : 1
        next
      }
      if (expect) {
        if (tag ~ /gml:id[ \t\r\n]*=/) ids++
        expect = 0
      }
    }
    END { print members, ids }
  ' "$1"
}

# The CityGML version a document declares, from the citygml namespace URI its
# root element binds (default or prefixed); empty when none is found. Only the
# head of the file is read — the root element is the first thing in it, and
# these documents run to gigabytes.
citygml_declared_version() {
  local found
  # No `| head -1` on the end of this pipeline: `head` exiting early can hand
  # `grep` a SIGPIPE, and under `set -o pipefail` that would abort the script
  # on a perfectly good document — a latent failure that only shows up on a
  # header with enough matches to fill the pipe buffer. Take the first match
  # with a parameter expansion instead.
  found="$(head -c 262144 "$1" \
    | { grep -oE 'http://www\.opengis\.net/citygml/[0-9]+\.[0-9]+' || true; })"
  found=${found%%$'\n'*}
  printf '%s' "${found##*/}"
}

# What kind of document INPUT is. Only a CityGML input starts the chain at its
# head; a CityJSON/CityJSONSeq input joins it midway.
INPUT_KIND="cityjson"   # any of: citygml | cityjson | cityjsonseq
case "$INPUT" in
  # `.xml` is here for the same reason it is in KNOWN_INPUT_EXTENSIONS: real
  # CityGML exports ship under all three spellings, and a `.xml` classed as
  # CityJSON would be handed to `cjseq`/`jq` and fail with a JSON parse error
  # instead of being converted.
  *.gml|*.citygml|*.xml) INPUT_KIND="citygml" ;;
  *.city.jsonl|*.jsonl) INPUT_KIND="cityjsonseq" ;;
esac

# `citygml` is built for EVERY input kind (see the header). Two flags, because
# the two paths differ in kind and in what has to be verified:
#
#   BUILD_CITYGML  a `citygml` artefact will exist in OUTDIR. Every later
#                  `want citygml` is paired with this flag.
#   SYNTH_CITYGML  ...and it is being SYNTHESISED from the CityJSON stage
#                  rather than copied from a CityGML INPUT. This decides where
#                  the fitness checks read from: a copied artefact is verified
#                  in the preflight against INPUT (an unfit input must cost
#                  nothing and leave nothing behind), a synthesised one can
#                  only be verified after it exists.
BUILD_CITYGML=0
SYNTH_CITYGML=0
if want citygml; then
  BUILD_CITYGML=1
  if [[ "$INPUT_KIND" != "citygml" ]]; then
    SYNTH_CITYGML=1
  fi
fi

# Which hops of the chain this request needs.
#
# NEED_SEQ: a CityJSONSeq artefact must exist in OUTDIR — either because
# something downstream of it (gz / fcb / cityparquet) is cut from it and the
# input carries no CityJSONSeq of its own, or because `cityjsonseq` itself was
# requested, which is a REAL artefact for every input kind (see the header).
NEED_SEQ=0
if [[ "$INPUT_KIND" == "citygml" ]]; then
  if want cityjsonseq || want cityjsonseq-gz || want flatcitybuf \
    || want cityparquet || want cityparquet-hilbert; then
    NEED_SEQ=1
  fi
elif want cityjsonseq; then
  NEED_SEQ=1
fi

# HOW block 3 produces it. A CityJSONSeq input already IS one, so it is
# copied; anything else is cut from the CityJSON stage with `cjseq cat`,
# keeping the chain's one rule (each artefact derives from the one before it)
# true for a CityJSON input too.
SEQ_FROM=""
if [[ "$NEED_SEQ" -eq 1 ]]; then
  if [[ "$INPUT_KIND" == "cityjsonseq" ]]; then
    SEQ_FROM="copy"
  else
    SEQ_FROM="cjseq-cat"
  fi
fi

# NEED_CITYJSON: requested outright, needed as the stage the CityJSONSeq is
# cut from (never for the copy case — a `.city.jsonl` input needs no CityJSON
# to become a `.city.jsonl` artefact), or needed as the stage the synthesised
# CityGML is converted BACK from. That last one is why `--formats citygml`
# alone, on a CityJSON input, still produces a CityJSON intermediate.
NEED_CITYJSON=0
if want cityjson || [[ "$SEQ_FROM" == "cjseq-cat" ]] || [[ "$SYNTH_CITYGML" -eq 1 ]]; then
  NEED_CITYJSON=1
fi

# citygml-tools is resolved, never version-pinned here: the pin lives in
# scripts/fetch_tools.sh, which unpacks it under bench/tools/ behind a
# version-independent symlink. $CITYGML_TOOLS overrides, PATH is the fallback
# for a system-wide install.
CITYGML_TOOLS_BIN=""
resolve_citygml_tools() {
  if [[ -n "${CITYGML_TOOLS:-}" ]]; then
    if [[ ! -x "$CITYGML_TOOLS" ]]; then
      echo "error: \$CITYGML_TOOLS is set to '$CITYGML_TOOLS', which is not executable" >&2
      exit 1
    fi
    CITYGML_TOOLS_BIN="$CITYGML_TOOLS"
  elif [[ -x "$REPO_ROOT/bench/tools/citygml-tools/citygml-tools" ]]; then
    CITYGML_TOOLS_BIN="$REPO_ROOT/bench/tools/citygml-tools/citygml-tools"
  elif command -v citygml-tools >/dev/null 2>&1; then
    CITYGML_TOOLS_BIN="$(command -v citygml-tools)"
  else
    echo "error: citygml-tools not found (needed for the CityGML <-> CityJSON conversion)" >&2
    echo "       run \`just fetch-tools\`, or point \$CITYGML_TOOLS at an install" >&2
    exit 1
  fi
}

if [[ "$INPUT_KIND" == "citygml" && "$NEED_CITYJSON" -eq 1 ]] \
  || [[ "$SYNTH_CITYGML" -eq 1 ]]; then
  resolve_citygml_tools
fi
# cjseq performs whichever CityJSON <-> CityJSONSeq hop this request needs:
# `cat` down from the CityJSON the chain just produced, or `collect` up from a
# CityJSONSeq input.
if [[ "$SEQ_FROM" == "cjseq-cat" ]] \
  || [[ "$NEED_CITYJSON" -eq 1 && "$INPUT_KIND" == "cityjsonseq" ]]; then
  require_tool cjseq "the CityJSON <-> CityJSONSeq conversion"
fi
# jq reads the CityJSON artefact's object count back out; a converter that
# writes a well-formed but object-less document is worse than one that fails.
if [[ "$NEED_CITYJSON" -eq 1 ]]; then
  require_tool jq "the CityJSON object-count check"
fi

# --- is this CityGML input fit to prepare at all? --------------------------
# Read from INPUT, here in the preflight, so an unfit input costs nothing and
# — crucially — leaves NOTHING behind. A refusal at verification time would
# already have copied the .gml into OUTDIR, where a later run, or a
# coordinator pointed at that directory, would find it and measure it: exactly
# the incomparable row these checks exist to prevent.
GML_OBJECTS=""
GML_WITH_IDS=""
if [[ "$INPUT_KIND" == "citygml" ]] \
  && { [[ "$BUILD_CITYGML" -eq 1 ]] || [[ "$NEED_CITYJSON" -eq 1 ]]; }; then
  read -r GML_OBJECTS GML_WITH_IDS <<<"$(citygml_counts "$INPUT")"
  if [[ "$GML_OBJECTS" -le 0 ]]; then
    echo "error: $INPUT contains no <cityObjectMember> elements (expected > 0)" >&2
    exit 1
  fi

  # Two ways a CityGML document can convert perfectly and still be unfit to
  # MEASURE. Both are refusals rather than warnings: this script's job is to
  # produce artefacts the benchmark can compare, and an artefact that will
  # silently produce an incomparable row is worse than no artefact at all —
  # the coordinator would carry on and publish the row. They apply only when
  # the `citygml` artefact itself is being built: every other format is read
  # from the derived CityJSON, whose ids are self-consistent whatever the
  # source did.
  if [[ "$BUILD_CITYGML" -eq 1 ]]; then
    # 1. The reader accepts only CityGML 2.0 (`sniff_citygml` in
    #    crates/cityparquet/src/source.rs rejects everything else), while
    #    citygml-tools converts 1.0 happily — so without this the whole chain
    #    goes green around an artefact that can never be read.
    GML_VERSION="$(citygml_declared_version "$INPUT")"
    if [[ -z "$GML_VERSION" ]]; then
      echo "warn: could not find a CityGML namespace in $INPUT; the benchmark reader" \
        "accepts only CityGML 2.0 and may refuse this artefact" >&2
    elif [[ "$GML_VERSION" != "2.0" ]]; then
      echo "error: $INPUT declares CityGML $GML_VERSION; the benchmark's reader" >&2
      echo "       supports only CityGML 2.0, so this artefact could never be measured" >&2
      exit 1
    fi

    # 2. Identity. citygml-tools mints a fresh random id for a top-level
    #    object with no `gml:id` — a different one on every run — so the id
    #    the benchmark samples from the derived CityJSONSeq is not in the .gml
    #    at all, and `citygml`'s id-lookup scores a MISS (result_count 0)
    #    beside every other format's hit. That is not a slower lookup, it is a
    #    different query, and the row is not publishable. Nothing downstream
    #    catches it either: the coordinator's cross-format self-consistency
    #    check covers AttrFilter(object_type) only, never IdLookup.
    #
    #    Not hypothetical: Riga's published atgazene_lod2.gml has 703 top-level
    #    objects and 0 gml:ids (identity lives in a gen:intAttribute OBJECTID).
    if [[ "$GML_WITH_IDS" -lt "$GML_OBJECTS" ]]; then
      echo "error: only $GML_WITH_IDS of $GML_OBJECTS top-level objects carry a gml:id in $INPUT" >&2
      echo "       citygml-tools mints a fresh random id for each of the rest, so the id" >&2
      echo "       the benchmark samples would be absent from this artefact and the" >&2
      echo "       id-lookup scenario would measure a miss against every other format's" >&2
      echo "       hit; refusing to prepare an artefact that cannot be compared" >&2
      exit 1
    fi
  fi
fi

NEED_CLI=0
if want cityparquet || want cityparquet-hilbert; then
  NEED_CLI=1
fi
if want flatcitybuf; then
  require_tool fcb "the FlatCityBuf artefact"
fi
if want cityjsonseq-gz; then
  require_tool gzip "the gzipped CityJSONSeq artefact"
fi

mkdir -p "$OUTDIR"

BASE="$(strip_known_extension "$(basename "$INPUT")")"

# Artefact names come from `Format::artefact` in
# crates/cityparquet-readbench/src/format.rs — the coordinator resolves them
# relative to its --prepared-dir, so they must match exactly.
GML_OUT="$OUTDIR/${BASE}.gml"
CITYJSON_OUT="$OUTDIR/${BASE}.city.json"
SEQ_OUT="$OUTDIR/${BASE}.city.jsonl"
PARQUET_OUT="$OUTDIR/${BASE}.parquet"
HILBERT_OUT="$OUTDIR/${BASE}-hilbert.parquet"
FCB_OUT="$OUTDIR/${BASE}.fcb"
GZ_OUT="$OUTDIR/${BASE}.jsonl.gz"

# Non-empty directory: at least one file inside (a CityParquet package is
# always a directory of one or more Parquet files + metadata.json; the
# exact member set depends on how many 1st-level CityObject families INPUT
# has, so this doesn't hardcode filenames).
dir_is_valid() {
  [[ -d "$1" ]] && [[ -n "$(find "$1" -type f -print -quit)" ]]
}

# Non-empty regular file.
file_is_valid() {
  [[ -f "$1" ]] && [[ -s "$1" ]]
}

# Do two paths name the same file? (INPUT may already BE an artefact path when
# OUTDIR happens to hold it, in which case copying it onto itself would fail.)
same_file() {
  [[ -e "$1" && -e "$2" ]] && [[ "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")" \
    == "$(cd "$(dirname "$2")" && pwd)/$(basename "$2")" ]]
}

# --- chain provenance ------------------------------------------------------
# WHY A STAMP AND NOT A SENTENCE IN THE DOCS.
#
# This script skips an artefact that already exists and passes its validity
# check, and OUTDIR (bench/data/readbench by default) persists across runs and
# across checkouts. So a directory prepared before the derivation chain
# changed keeps serving artefacts derived from a stage that no longer exists —
# silently, under the same names, with nothing to look at. The sharp case is
# the gz baseline: before `cityjsonseq` became a real artefact for every input
# kind, a `.city.json` input's `<x>.jsonl.gz` was a gzip of the WHOLE CityJSON
# DOCUMENT, and the gz runner reads one happily (0.254909 s / 61,192,614 B
# against the real seq-gz's 0.092799 s / 1,798,710 B — 2.75x too slow, 34x too
# heavy). A documentation line is missed by exactly the person who most needs
# it, and this failure publishes plausible-looking numbers.
#
# So each dataset's artefacts carry the version of the chain that built them,
# in `$OUTDIR/.readbench-chain/<base>`, and a stale one is a REFUSAL. Absent
# counts as stale: every directory prepared before this stamp existed is
# exactly the kind this guard is for, so unknown provenance can never mean
# "current".
#
# BUMP CHAIN_VERSION whenever a change here makes an artefact built by an
# older version wrong to reuse (a different derivation stage, different flags,
# a different tool) — NOT for a change that only affects which artefacts get
# built, or reporting. History:
#   1  every version up to and including commit ba6e5fa (unstamped): from a
#      CityJSON/CityJSONSeq input the CityJSONSeq artefact was not built at
#      all, and fcb/cityparquet/gz were derived from INPUT itself.
#   2  the CityJSONSeq artefact is always materialised, and everything
#      downstream derives from IT.
CHAIN_VERSION=2
CHAIN_DIR="$OUTDIR/.readbench-chain"
CHAIN_STAMP="$CHAIN_DIR/$BASE"

# Every artefact path this script owns for this dataset, whatever was
# requested: a stale artefact nobody asked for today is still one the
# coordinator will measure tomorrow.
ALL_OUTPUTS=("$GML_OUT" "$CITYJSON_OUT" "$SEQ_OUT" "$PARQUET_OUT" "$HILBERT_OUT" \
  "$FCB_OUT" "$GZ_OUT")

STALE=()
STAMPED=""
if [[ -f "$CHAIN_STAMP" ]]; then
  STAMPED="$(cat "$CHAIN_STAMP")"
  STAMPED=${STAMPED%%$'\n'*}
fi
if [[ "$STAMPED" != "$CHAIN_VERSION" ]]; then
  for out in "${ALL_OUTPUTS[@]}"; do
    # INPUT sitting in OUTDIR under an artefact's own name was not built by
    # any run of this script, so it is not evidence of an older chain (block
    # 1/2/3 each report it as "the input is already the artefact").
    if same_file "$INPUT" "$out"; then
      continue
    fi
    if file_is_valid "$out" || dir_is_valid "$out"; then
      STALE+=("$out")
    fi
  done
fi
if [[ ${#STALE[@]} -gt 0 ]]; then
  echo "error: $OUTDIR holds artefacts for '$BASE' built by an older derivation chain" >&2
  echo "       (chain version ${STAMPED:-none recorded}; this script builds version $CHAIN_VERSION)." >&2
  echo "       Reusing them would measure a stage this chain no longer produces, and the" >&2
  echo "       numbers would look entirely plausible. Delete them and re-run:" >&2
  echo "         rm -rf ${STALE[*]} $CHAIN_STAMP" >&2
  exit 1
fi

# Stamped BEFORE anything is built, so that whatever this run does write is
# covered by it (a run that dies midway leaves artefacts this chain built, and
# they are the ones the stamp claims).
mkdir -p "$CHAIN_DIR"
printf '%s\n' "$CHAIN_VERSION" >"$CHAIN_STAMP"

# --- object counts ---------------------------------------------------------
# Cheap, per-format counts of the same quantity: TOP-LEVEL city objects. They
# serve two purposes — proving an artefact is not merely non-empty but
# non-vacuous, and surfacing conversion loss across the chain, which the
# design spec's fairness caveats say must be asserted rather than assumed (a
# CityGML row and a CityParquet row are only comparable if the conversion
# between them was lossless). The CityGML end of it is counted up in the
# preflight, because it also decides whether this input is fit to prepare
# at all.
#
# `fcb info`'s feature count is deliberately NOT folded into the comparison:
# it is checked separately below, and CityParquet's own object_count counts
# descendants (BuildingParts) too, so neither is the same quantity.

# CityObjects with no "parents" member, i.e. the first-level ones — the same
# quantity a CityJSONSeq counts as features.
cityjson_object_count() {
  jq '[.CityObjects[] | select(has("parents") | not)] | length' "$1"
}

# One CityJSONFeature per line.
cityjsonseq_feature_count() {
  { grep -c '"CityJSONFeature"' "$1" || true; } | tr -d ' '
}

# Report (never fail on) a count that changed across a conversion: real
# CityGML routinely carries ADE content citygml-tools skips, so this is
# evidence for the write-up, not a gate.
report_count_drift() {
  local from_label=$1 from_count=$2 to_label=$3 to_count=$4
  if [[ "$from_count" != "$to_count" ]]; then
    echo "warn: conversion loss: $from_label has $from_count top-level object(s) but" \
      "$to_label has $to_count -- the cross-format comparison is only fair where the" \
      "conversion was lossless" >&2
  fi
}

echo "== readbench_prepare: $INPUT -> $OUTDIR (base: $BASE) =="
echo "-- formats: ${REQUESTED[*]}"

# Build the release CLI once, up front, so none of the per-artefact steps
# below pay a `cargo run` recompile-check cost. Only when a format that needs
# it was requested.
CITYPARQUET=""
if [[ "$NEED_CLI" -eq 1 ]]; then
  echo "-- building release CLI (cargo build --release -p cityparquet-cli)"
  ( cd "$REPO_ROOT" && cargo build --release -p cityparquet-cli )
  CITYPARQUET="$REPO_ROOT/target/release/cityparquet"
  if [[ ! -x "$CITYPARQUET" ]]; then
    echo "error: expected binary not found after build: $CITYPARQUET" >&2
    exit 1
  fi
fi

# Artefacts this run is responsible for, in build order, for the closing
# summary.
BUILT=()

# Files the chain HAD to write on the way to something that was requested —
# `--formats cityparquet` on a .gml cannot reach the CityJSONSeq without a
# CityJSON. They are reported separately at the end rather than silently left
# in OUTDIR, where they would look like artefacts this run measured.
INTERMEDIATES=()

# The CityJSONSeq every downstream artefact (gz / fcb / cityparquet) is cut
# from. It is INPUT itself unless block 3 below materialises a `.city.jsonl`
# artefact (whenever NEED_SEQ), in which case all of them read THAT — the
# same bytes the `cityjsonseq` row is measured on, which is what makes their
# comparison fair.
SEQ_INPUT="$INPUT"

# 1. CityGML, from a CityGML INPUT: the source document itself, placed where
# the coordinator looks for it (--prepared-dir/<base>.gml). Copied, never
# round-tripped — where the source data IS CityGML, that is what gets
# measured. The synthesis path for every other input kind is step 2b below,
# which has to wait for the CityJSON stage to exist.
if [[ "$BUILD_CITYGML" -eq 1 && "$SYNTH_CITYGML" -eq 0 ]]; then
  if file_is_valid "$GML_OUT" && same_file "$INPUT" "$GML_OUT"; then
    echo "skip $GML_OUT (the input is already the artefact)"
  elif file_is_valid "$GML_OUT"; then
    echo "skip $GML_OUT (already present)"
  else
    echo "-- copy $INPUT -> $GML_OUT"
    cp "$INPUT" "$GML_OUT"
  fi
  BUILT+=("$GML_OUT")
fi

# 2. CityJSON: one whole document. From CityGML via citygml-tools (the head
# of the chain); from a CityJSONSeq input via `cjseq collect` (up the chain,
# but still from the source data — never out of a CityParquet package).
if [[ "$NEED_CITYJSON" -eq 1 ]]; then
  if file_is_valid "$CITYJSON_OUT" && same_file "$INPUT" "$CITYJSON_OUT"; then
    echo "skip $CITYJSON_OUT (the input is already the artefact)"
  elif file_is_valid "$CITYJSON_OUT"; then
    echo "skip $CITYJSON_OUT (already present)"
  elif [[ "$INPUT_KIND" == "citygml" ]]; then
    echo "-- citygml-tools to-cityjson $INPUT -> $CITYJSON_OUT"
    # citygml-tools writes <basename>.json into an output DIRECTORY, not to a
    # path of our choosing, so it converts into a scratch directory that is
    # cleaned up whatever happens, and the single result is moved into place.
    CJ_TMP="$(mktemp -d "$OUTDIR/.cityjson.XXXXXX")"
    trap 'rm -rf "$CJ_TMP"' EXIT
    "$CITYGML_TOOLS_BIN" to-cityjson -o "$CJ_TMP" "$INPUT"
    CJ_PRODUCED=()
    while IFS= read -r -d '' produced; do
      CJ_PRODUCED+=("$produced")
    done < <(find "$CJ_TMP" -maxdepth 1 -type f -name '*.json' -print0)
    if [[ ${#CJ_PRODUCED[@]} -ne 1 ]]; then
      echo "error: citygml-tools produced ${#CJ_PRODUCED[@]} .json files for $INPUT (expected exactly 1)" >&2
      exit 1
    fi
    mv "${CJ_PRODUCED[0]}" "$CITYJSON_OUT"
    rm -rf "$CJ_TMP"
    trap - EXIT
  elif [[ "$INPUT_KIND" == "cityjsonseq" ]]; then
    echo "-- cjseq collect $INPUT -> $CITYJSON_OUT"
    # Via a temporary, and the temporary is cleaned up on any exit: a `cjseq`
    # that dies midway must leave neither a truncated document that the next
    # run's validity check would happily accept, nor debris beside it.
    trap 'rm -f "$CITYJSON_OUT.tmp"' EXIT
    cjseq collect -f "$INPUT" >"$CITYJSON_OUT.tmp"
    mv "$CITYJSON_OUT.tmp" "$CITYJSON_OUT"
    trap - EXIT
  else
    echo "-- copy $INPUT -> $CITYJSON_OUT"
    cp "$INPUT" "$CITYJSON_OUT"
  fi
  if want cityjson; then
    BUILT+=("$CITYJSON_OUT")
  else
    INTERMEDIATES+=("$CITYJSON_OUT")
  fi
fi

# 2b. CityGML, synthesised: converted BACK from the CityJSON stage step 2 just
# produced, for an INPUT that is not itself CityGML. Numbered out of order
# because it cannot run in step 1 — there is no CityJSON to convert yet.
#
# It derives from $CITYJSON_OUT rather than from $INPUT even when the two hold
# the same content, so that the chain's one rule (each artefact derives from
# the one before it) stays true here too, and so a `.city.jsonl` INPUT — whose
# CityJSON stage is `cjseq collect`'s output — takes the same path as a
# `.city.json` one.
#
# `-v 2.0` because this repository's reader accepts only CityGML 2.0
# (`sniff_citygml`); the tool's own default is 3.0, which would produce an
# artefact that converts cleanly and can never be read.
#
# `--no-pretty-print` is a MEASUREMENT decision, not a tidiness one. On
# Rotterdam the same content serialises to 18.8 MB indented and 14.0 MB
# compact, against a 16.5 MB published original — so indentation alone would
# move the `citygml` row by a third. Compact is the conservative choice: it
# gives the baseline this benchmark argues against its BEST case, so no size
# or parse-time gap can be dismissed as an artefact of whitespace.
if [[ "$SYNTH_CITYGML" -eq 1 ]]; then
  if file_is_valid "$GML_OUT"; then
    echo "skip $GML_OUT (already present)"
  else
    echo "-- citygml-tools from-cityjson -v 2.0 $CITYJSON_OUT -> $GML_OUT"
    # Same shape as the to-cityjson hop above: citygml-tools writes
    # <basename>.gml into an output DIRECTORY, not to a path of our choosing,
    # so it converts into a scratch directory that is cleaned up whatever
    # happens and the single result is moved into place.
    GML_TMP="$(mktemp -d "$OUTDIR/.citygml.XXXXXX")"
    trap 'rm -rf "$GML_TMP"' EXIT
    "$CITYGML_TOOLS_BIN" from-cityjson -v 2.0 --no-pretty-print \
      -o "$GML_TMP" "$CITYJSON_OUT"
    GML_PRODUCED=()
    while IFS= read -r -d '' produced; do
      GML_PRODUCED+=("$produced")
    done < <(find "$GML_TMP" -maxdepth 1 -type f -name '*.gml' -print0)
    if [[ ${#GML_PRODUCED[@]} -ne 1 ]]; then
      echo "error: citygml-tools produced ${#GML_PRODUCED[@]} .gml files for $CITYJSON_OUT (expected exactly 1)" >&2
      exit 1
    fi
    mv "${GML_PRODUCED[0]}" "$GML_OUT"
    rm -rf "$GML_TMP"
    trap - EXIT
  fi

  # The same two fitness checks the preflight runs against a CityGML INPUT,
  # run here instead because the artefact did not exist until a moment ago.
  # Neither is expected to fire — `-v 2.0` settles the version, and an object
  # that reached the CityJSON stage has a key, which is what becomes the
  # gml:id — but "not expected to fire" is exactly the assumption that stops
  # being true after a tool upgrade, and both failures are silent in the
  # results CSV rather than loud.
  read -r GML_OBJECTS GML_WITH_IDS <<<"$(citygml_counts "$GML_OUT")"
  if [[ "$GML_OBJECTS" -le 0 ]]; then
    echo "error: the synthesised $GML_OUT contains no <cityObjectMember> elements (expected > 0)" >&2
    exit 1
  fi
  SYNTH_VERSION="$(citygml_declared_version "$GML_OUT")"
  if [[ "$SYNTH_VERSION" != "2.0" ]]; then
    echo "error: the synthesised $GML_OUT declares CityGML '${SYNTH_VERSION:-none}', not 2.0;" >&2
    echo "       the benchmark's reader supports only 2.0, so it could never be measured" >&2
    exit 1
  fi
  if [[ "$GML_WITH_IDS" -lt "$GML_OBJECTS" ]]; then
    echo "error: only $GML_WITH_IDS of $GML_OBJECTS top-level objects carry a gml:id in the" >&2
    echo "       synthesised $GML_OUT; citygml-tools mints a fresh random id for each of the" >&2
    echo "       rest, so the id the benchmark samples would be absent from this artefact and" >&2
    echo "       the id-lookup scenario would measure a miss against every other format's hit" >&2
    exit 1
  fi
  BUILT+=("$GML_OUT")
fi

# 3. CityJSONSeq: cut from the CityJSON above, or copied when INPUT already
# IS one. Either way it lands in OUTDIR under the name the coordinator
# resolves (`Format::artefact`) — never left as "read INPUT in place", which
# is how a `.gml`/`.city.json` input got measured under this format's name.
if [[ "$NEED_SEQ" -eq 1 ]]; then
  if file_is_valid "$SEQ_OUT" && same_file "$INPUT" "$SEQ_OUT"; then
    echo "skip $SEQ_OUT (the input is already the artefact)"
  elif file_is_valid "$SEQ_OUT"; then
    echo "skip $SEQ_OUT (already present)"
  elif [[ "$SEQ_FROM" == "copy" ]]; then
    echo "-- copy $INPUT -> $SEQ_OUT"
    cp "$INPUT" "$SEQ_OUT"
  else
    echo "-- cjseq cat $CITYJSON_OUT -> $SEQ_OUT"
    trap 'rm -f "$SEQ_OUT.tmp"' EXIT
    cjseq cat -f "$CITYJSON_OUT" >"$SEQ_OUT.tmp"
    mv "$SEQ_OUT.tmp" "$SEQ_OUT"
    trap - EXIT
  fi
  # Everything downstream now reads the SAME bytes the `cityjsonseq` row is
  # measured on, for every input kind — the chain diagram in the header is
  # true of a CityJSON input too, not only a CityGML one.
  SEQ_INPUT="$SEQ_OUT"
  if want cityjsonseq; then
    BUILT+=("$SEQ_OUT")
  else
    INTERMEDIATES+=("$SEQ_OUT")
  fi
fi

# 4. Core-profile CityParquet package (source row order).
if want cityparquet; then
  if dir_is_valid "$PARQUET_OUT"; then
    echo "skip $PARQUET_OUT (already present)"
  else
    echo "-- convert $SEQ_INPUT -> $PARQUET_OUT"
    # By-type is the only, mandatory table layout (2026-07-21): one
    # `<snake>.parquet` table per 1st-level CityObject family. The
    # read-benchmark's CityParquetRunner only supports a package whose
    # manifest lists exactly one table, so INPUT for this script must be a
    # single-family dataset (e.g. a Building-only 3D BAG tile) — a
    # multi-family INPUT prepares fine here but the read-benchmark itself
    # rejects it later with a clear error.
    "$CITYPARQUET" convert "$SEQ_INPUT" -o "$PARQUET_OUT" --overwrite
  fi
  BUILT+=("$PARQUET_OUT")
fi

# 5. Hilbert-ordered CityParquet package.
if want cityparquet-hilbert; then
  if dir_is_valid "$HILBERT_OUT"; then
    echo "skip $HILBERT_OUT (already present)"
  else
    echo "-- convert --ordering hilbert $SEQ_INPUT -> $HILBERT_OUT"
    "$CITYPARQUET" convert "$SEQ_INPUT" -o "$HILBERT_OUT" --ordering hilbert --overwrite
  fi
  BUILT+=("$HILBERT_OUT")
fi

# 6. FlatCityBuf, spatial index (default-on) + all-attribute B+Tree index.
if want flatcitybuf; then
  if file_is_valid "$FCB_OUT"; then
    echo "skip $FCB_OUT (already present)"
  else
    echo "-- fcb ser $SEQ_INPUT -> $FCB_OUT"
    fcb ser -i "$SEQ_INPUT" -o "$FCB_OUT" -A
  fi
  BUILT+=("$FCB_OUT")
fi

# 7. Gzip of the CityJSONSeq, for a whole-document-gzip baseline.
if want cityjsonseq-gz; then
  if file_is_valid "$GZ_OUT"; then
    echo "skip $GZ_OUT (already present)"
  else
    echo "-- gzip -9 $SEQ_INPUT -> $GZ_OUT"
    gzip -9 -c "$SEQ_INPUT" > "$GZ_OUT"
  fi
  BUILT+=("$GZ_OUT")
fi

# (There is no block 8: `cityjsonseq` is built by block 3 for EVERY input
# kind. It used to be a documented no-op here — see the header for what that
# cost.)

# Sanity checks: every artefact this run was responsible for exists and is
# non-empty, and — when FlatCityBuf was built — the FCB file reports a
# positive feature count via `fcb info` (fcb prints a "Features: N" line
# under "Dataset"; N need not equal cityparquet's object_count, since FCB
# counts top-level features while cityparquet's object_count includes
# descendant CityObjects such as BuildingParts).
echo "-- verifying artefacts"
# The chain's own artefacts are checked for CONTENT, not just existence: an
# artefact that exists but holds no city objects is worse than one that is
# missing, because the benchmark would happily measure it. The counts are then
# compared to each other, so a lossy hop shows up as a warning rather than as
# an unexplained gap in the results table.
# (GML_OBJECTS/GML_WITH_IDS are NOT reset here — the preflight computed them
# from the source document and this block reports and compares them.)
CITYJSON_OBJECTS=""
SEQ_FEATURES=""

if [[ "$BUILD_CITYGML" -eq 1 ]]; then
  file_is_valid "$GML_OUT" || { echo "error: missing/empty file: $GML_OUT" >&2; exit 1; }
fi
# Where the CityGML counts came from depends on which way the conversion ran,
# and so does what they can be compared against:
#
#   COPIED (CityGML INPUT)  the preflight read them off the source document
#     and refused it if it was empty, the wrong version, or short of gml:ids.
#     They are the BASELINE the derived CityJSON is checked against — which is
#     why they are taken even by a run that never asked for the `citygml`
#     artefact: the CityGML -> CityJSON hop is where loss is likeliest
#     (citygml-tools skips ADE content it has no extension for), and it would
#     otherwise go unmeasured.
#
#   SYNTHESISED  step 2b read them off the artefact it had just written, and
#     the CityJSON is UPSTREAM of it rather than downstream. No drift check is
#     run in that direction, and it would be wrong to run one: CityGML nests a
#     BuildingPart inside its parent Building where CityJSON lists both at top
#     level, so the two counts legitimately differ — on the corpus's 3DBAG
#     tile, 1,110 top-level GML members against 2,221 CityObjects. Comparing
#     them would report a 50% "loss" that did not happen.
GML_COUNTED_IN="$INPUT"
if [[ "$SYNTH_CITYGML" -eq 1 ]]; then
  GML_COUNTED_IN="$GML_OUT"
fi
if [[ -n "$GML_OBJECTS" ]]; then
  echo "  citygml: $GML_OBJECTS top-level object(s) in $GML_COUNTED_IN"
  if [[ "$BUILD_CITYGML" -eq 1 ]]; then
    echo "  citygml: all $GML_WITH_IDS object(s) carry a gml:id (id-lookup is comparable)"
  fi
fi
if [[ "$NEED_CITYJSON" -eq 1 ]]; then
  file_is_valid "$CITYJSON_OUT" || { echo "error: missing/empty file: $CITYJSON_OUT" >&2; exit 1; }
  CITYJSON_OBJECTS="$(cityjson_object_count "$CITYJSON_OUT")"
  if [[ "$CITYJSON_OBJECTS" -le 0 ]]; then
    echo "error: $CITYJSON_OUT contains no top-level CityObjects (expected > 0)" >&2
    exit 1
  fi
  echo "  cityjson: $CITYJSON_OBJECTS top-level object(s) in $CITYJSON_OUT"
  if [[ -n "$GML_OBJECTS" && "$SYNTH_CITYGML" -eq 0 ]]; then
    report_count_drift "$INPUT" "$GML_OBJECTS" "$CITYJSON_OUT" "$CITYJSON_OBJECTS"
  fi
fi
if [[ "$NEED_SEQ" -eq 1 ]]; then
  file_is_valid "$SEQ_OUT" || { echo "error: missing/empty file: $SEQ_OUT" >&2; exit 1; }
  SEQ_FEATURES="$(cityjsonseq_feature_count "$SEQ_OUT")"
  if [[ "$SEQ_FEATURES" -le 0 ]]; then
    echo "error: $SEQ_OUT contains no CityJSONFeature lines (expected > 0)" >&2
    exit 1
  fi
  echo "  cityjsonseq: $SEQ_FEATURES feature(s) in $SEQ_OUT"
  # Only the `cjseq cat` hop has a CityJSON stage to have lost anything
  # against; a copied artefact has no upstream count here (CITYJSON_OBJECTS is
  # empty when the CityJSON was never needed), and comparing against an empty
  # string would report a "loss" that never happened.
  if [[ -n "$CITYJSON_OBJECTS" ]]; then
    report_count_drift "$CITYJSON_OUT" "$CITYJSON_OBJECTS" "$SEQ_OUT" "$SEQ_FEATURES"
  fi
fi

if want cityparquet; then
  dir_is_valid "$PARQUET_OUT" || { echo "error: missing/empty package: $PARQUET_OUT" >&2; exit 1; }
fi
if want cityparquet-hilbert; then
  dir_is_valid "$HILBERT_OUT" || { echo "error: missing/empty package: $HILBERT_OUT" >&2; exit 1; }
fi
if want cityjsonseq-gz; then
  file_is_valid "$GZ_OUT" || { echo "error: missing/empty file: $GZ_OUT" >&2; exit 1; }
fi
if want flatcitybuf; then
  file_is_valid "$FCB_OUT" || { echo "error: missing/empty file: $FCB_OUT" >&2; exit 1; }

  FCB_INFO="$(fcb info -i "$FCB_OUT")"
  # `[[:space:]]`, not `\s`: `\s` is a GNU extension that POSIX ERE does not
  # define, so BSD/macOS `grep -E` matches nothing with it — and the
  # measurement machine (bench/READ_BENCHMARK.md's own record) is Darwin
  # arm64, where this block therefore failed EVERY FlatCityBuf prepare.
  #
  # No `| head -1` either: `head` exiting early can hand `grep` a SIGPIPE,
  # which under `set -o pipefail` aborts the script on perfectly good output
  # (see `citygml_declared_version` above). Take the first match with a
  # parameter expansion instead.
  FEATURES="$(echo "$FCB_INFO" \
    | { grep -E '^[[:space:]]*Features:' || true; } \
    | { grep -oE '[0-9]+' || true; })"
  FEATURES=${FEATURES%%$'\n'*}
  if [[ -z "$FEATURES" ]]; then
    echo "error: could not find a 'Features:' count in \`fcb info\` output for $FCB_OUT" >&2
    echo "$FCB_INFO" >&2
    exit 1
  fi
  if [[ "$FEATURES" -le 0 ]]; then
    echo "error: fcb info reports $FEATURES features for $FCB_OUT (expected > 0)" >&2
    exit 1
  fi
  echo "  fcb info: $FEATURES features in $FCB_OUT"
fi

if [[ ${#INTERMEDIATES[@]} -gt 0 ]]; then
  INTERMEDIATE_SUMMARY=""
  for out in "${INTERMEDIATES[@]}"; do
    INTERMEDIATE_SUMMARY+="${INTERMEDIATE_SUMMARY:+, }$out"
  done
  echo "chain intermediates (not requested): $INTERMEDIATE_SUMMARY"
fi

if [[ ${#BUILT[@]} -eq 0 ]]; then
  echo "readbench_prepare complete: nothing to build for ${REQUESTED[*]}"
else
  SUMMARY=""
  for out in "${BUILT[@]}"; do
    SUMMARY+="${SUMMARY:+, }$out"
  done
  echo "readbench_prepare complete: $SUMMARY"
fi
