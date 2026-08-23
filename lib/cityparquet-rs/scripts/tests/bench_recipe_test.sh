#!/usr/bin/env bash
# Tests for the `bench` recipe's FORMAT-SELECTION block in the justfile —
# which formats get prepared, and whether the `duckdb-parquet` SQL-engine
# baseline is appended to the CSV.
#
# Plain bash, no framework: `bats` is not a dependency of this repo. One
# `ok`/`not ok` line per case, non-zero exit if any case fails. Same shape as
# `readbench_prepare_test.sh` and `fetch_benchmark_test.sh` beside it.
#
# WHY THIS EXISTS. `Format::DEFAULT_SET`
# (crates/cityparquet-readbench/src/format.rs) is the FORMAT-comparison set:
# five tags, one per format family. `duckdb-parquet` is deliberately not in it
# — it is an SQL-ENGINE baseline over a file already in the set, not a format,
# so a run labelled "format comparison" must not carry it unasked. The
# justfile disagreed: `want_duckdb` started at 1 and was only cleared when
# FORMATS was NON-EMPTY, so a bare `just bench <folder>` appended a sixth,
# non-format series to a CSV that `benchmark/formats/READ_BENCHMARK.md` describes as
# holding five. Two sources of truth for one decision, disagreeing silently in
# the default case — the one nobody passes flags to.
#
# EXECUTABLE, NOT TEXTUAL. The block is EXTRACTED from the justfile between
# its own `# BEGIN format-selection` / `# END format-selection` markers and
# RUN under bash, once per case, with `{{FORMATS}}` substituted the way `just`
# would substitute it. Asserting on the justfile's TEXT (grepping for
# `want_duckdb=0`, say) would pass any rearrangement that kept the string and
# changed the logic — which is precisely the failure mode being pinned here.
#
# WHAT IS NOT COVERED. This runs the decision block, not the whole recipe: it
# proves which formats the recipe DECIDES on, not that the later
# `readbench_duckdb.sh` invocation is correctly guarded by `want_duckdb` (that
# is three lines of `if` in the same recipe, and running it for real would
# need cargo, fcb and duckdb). Case 6 covers the gap textually — the only
# assertion here that is deliberately a text check, and labelled as such.
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TEST_DIR/../.." && pwd)"
# The `bench` recipe lives in the MONOREPO's root justfile — it reaches both
# this workspace's harness crate and the corpora under benchmark/, so neither
# tree can own it. `Format::DEFAULT_SET`, which it must agree with, is local.
MONO_ROOT="$(cd "$REPO_ROOT/../.." && pwd)"
JUSTFILE="$MONO_ROOT/justfile"
FORMAT_RS="$REPO_ROOT/crates/cityparquet-readbench/src/format.rs"

PASSED=0
FAILED=0

pass() {
  echo "ok   - $1"
  PASSED=$((PASSED + 1))
}

fail() {
  echo "not ok - $1: $2" >&2
  FAILED=$((FAILED + 1))
}

# The format-selection block, lifted verbatim out of the `bench` recipe.
#
# `just` strips the recipe body's leading indentation and substitutes
# `{{FORMATS}}` before handing the result to bash; both are reproduced here so
# the code under test is the code that runs. The markers are required to
# exist: a silently-empty extraction would make every case below pass
# vacuously.
extract_block() {
  sed -n '/# BEGIN format-selection/,/# END format-selection/p' "$JUSTFILE" \
    | sed 's/^    //'
}

# run_selection FORMATS -> prints "<want_duckdb> <prepare_formats>"
#
# stdout is the two values the rest of the recipe branches on. Everything the
# block itself needs is local to it, so nothing else has to be stubbed.
run_selection() {
  local formats=$1 block
  block="$(extract_block)"
  # The substitution `just` performs. FORMATS is a plain comma-separated list
  # with no quoting metacharacters (it is validated downstream by both the
  # coordinator and the prepare script), so a literal replacement is faithful.
  block="${block//\{\{FORMATS\}\}/$formats}"
  bash -euo pipefail -c "
    $block
    printf '%s %s\n' \"\$want_duckdb\" \"\$prepare_formats\"
  "
}

want_duckdb_for() {
  run_selection "$1" | cut -d' ' -f1
}

prepare_formats_for() {
  run_selection "$1" | cut -d' ' -f2-
}

# --------------------------------------------------------------------------
# Case 0: the extraction found something.
#
# First, because every other case is meaningless if the markers moved — and a
# `sed` range that matches nothing exits 0 with empty output, so the failure
# would otherwise be silent and green.
# --------------------------------------------------------------------------
case_block_is_extractable() {
  local name="the format-selection block is extractable from the justfile"
  local block
  block="$(extract_block)"
  if [[ -z "$block" ]]; then
    fail "$name" "no '# BEGIN format-selection'..'# END format-selection' range in $JUSTFILE"
    return
  fi
  if [[ "$block" != *"want_duckdb"* || "$block" != *"prepare_formats"* ]]; then
    fail "$name" "the extracted range does not set want_duckdb/prepare_formats"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 1: A BARE RUN MUST NOT APPEND THE SQL-ENGINE BASELINE.
#
# The regression this file exists for. `just bench <folder>` with no FORMATS
# measures `Format::DEFAULT_SET` — five formats, one per family. Appending
# `duckdb-parquet` would put a sixth, NON-format series in a CSV documented as
# holding five, and would do it in the default case, where no one passed a
# flag that might have warned them.
# --------------------------------------------------------------------------
case_bare_run_omits_the_baseline() {
  local name="a bare run does NOT append the duckdb-parquet baseline"
  local got
  got="$(want_duckdb_for "")"
  if [[ "$got" != "0" ]]; then
    fail "$name" "want_duckdb=$got for empty FORMATS (expected 0: duckdb-parquet is opt-in)"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 2: naming it explicitly DOES append it.
#
# Opt-in has to mean opt-IN, not "unavailable": the baseline is a real,
# documented series and asking for it by name must still work.
# --------------------------------------------------------------------------
case_naming_the_baseline_appends_it() {
  local name="naming duckdb-parquet in FORMATS appends the baseline"
  local got
  got="$(want_duckdb_for "cityparquet,duckdb-parquet")"
  if [[ "$got" != "1" ]]; then
    fail "$name" "want_duckdb=$got when duckdb-parquet was named (expected 1)"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 3: an explicit list without it does not get it.
# --------------------------------------------------------------------------
case_explicit_list_without_baseline() {
  local name="an explicit FORMATS without duckdb-parquet does not append it"
  local got
  got="$(want_duckdb_for "citygml,cityjson,cityjsonseq,flatcitybuf,cityparquet-hilbert")"
  if [[ "$got" != "0" ]]; then
    fail "$name" "want_duckdb=$got for a baseline-free list (expected 0)"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 4: the ORDERING run stays single-axis.
#
# `ordering-bench` delegates to `bench` with exactly `Format::ORDERING_SET`.
# Its whole point is that the ONLY variable is row order, so a third series —
# a different ENGINE, no less — would confound the one comparison it exists to
# make. The tags are read out of `Format::ORDERING_SET` rather than retyped,
# so a change to the enum reaches this case.
# --------------------------------------------------------------------------
case_ordering_run_stays_single_axis() {
  local name="the ordering set does not append the baseline"
  local variant tag expected="" actual got recipe
  # `Format::ORDERING_SET`'s members, mapped through `Format::as_str` to the
  # CLI spelling `ordering-bench` has to pass. The old version read the
  # variant names, checked only that the list was non-empty, and then compared
  # the recipe against a HARDCODED string — so `ORDERING_SET := [CityParquet,
  # FlatCityBuf]` and an `ordering-bench` passing a third format both left
  # this case green. The enum is the authority; the recipe must match it.
  while IFS= read -r variant; do
    [[ -n "$variant" ]] || continue
    tag="$(sed -n "s/^ *Format::${variant#Format::} => \"\([a-z0-9-]*\)\",\$/\1/p" "$FORMAT_RS")"
    tag=${tag%%$'\n'*}
    if [[ -z "$tag" ]]; then
      fail "$name" "no as_str spelling for $variant in $FORMAT_RS"
      return
    fi
    expected+="${expected:+,}$tag"
  # The declaration only, ending at ITS OWN `];`. A `sed` range would run on
  # to the next `];` in the file and drag unrelated `Format::` mentions in
  # with it.
  done < <(awk '/pub const ORDERING_SET/ { f = 1 } f { print; if (/\];/) exit }' "$FORMAT_RS" \
    | grep -oE 'Format::[A-Za-z]+' | grep -v 'Format::ORDERING_SET' || true)
  if [[ -z "$expected" ]]; then
    fail "$name" "could not read ORDERING_SET out of $FORMAT_RS"
    return
  fi
  # What `ordering-bench` actually passes, taken from the recipe itself.
  recipe="$(grep -F 'just bench "{{FOLDER}}" "{{OUT}}"' "$JUSTFILE" || true)"
  actual="$(printf '%s' "$recipe" | sed -n 's/.*"{{OUT}}"[[:space:]]*"\([^"]*\)".*/\1/p')"
  if [[ "$actual" != "$expected" ]]; then
    fail "$name" "ordering-bench passes '$actual', Format::ORDERING_SET is '$expected'"
    return
  fi
  got="$(want_duckdb_for "$expected")"
  if [[ "$got" != "0" ]]; then
    fail "$name" "want_duckdb=$got for the ordering set (expected 0)"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 5: `cityparquet` is always prepared, and the baseline is never asked
# of the prepare script.
#
# Two invariants that must survive the change above:
#   - the coordinator derives EVERY query parameter (bbox window, sampled id,
#     attribute predicate) from the cityparquet package, so it must be built
#     whatever was requested;
#   - `readbench_prepare.sh` rejects `duckdb-parquet` outright (it has no
#     artefact of its own), so passing it through would hard-fail the run.
# --------------------------------------------------------------------------
case_prepare_list_invariants() {
  local name="prepare always includes cityparquet and never duckdb-parquet"
  local formats got
  for formats in "duckdb-parquet" "flatcitybuf,duckdb-parquet" \
    "citygml,cityjson,cityjsonseq,flatcitybuf,cityparquet-hilbert" \
    "cityparquet,cityparquet-hilbert"; do
    got="$(prepare_formats_for "$formats")"
    if [[ ",$got," != *",cityparquet,"* ]]; then
      fail "$name" "FORMATS='$formats' -> prepare_formats='$got' (missing cityparquet)"
      return
    fi
    if [[ "$got" == *"duckdb-parquet"* ]]; then
      fail "$name" "FORMATS='$formats' -> prepare_formats='$got' (leaks duckdb-parquet)"
      return
    fi
  done
  # The empty case is different in kind: an empty prepare list means "pass no
  # --formats flag at all", which makes the prepare script build every
  # artefact it knows how to — cityparquet included. Asserting the string
  # would be wrong here.
  got="$(prepare_formats_for "")"
  if [[ -n "$got" ]]; then
    fail "$name" "FORMATS='' -> prepare_formats='$got' (expected empty: prepare everything)"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 6: the baseline invocation is actually guarded by `want_duckdb`.
#
# TEXTUAL, deliberately, and the only such assertion here: running the real
# `readbench_duckdb.sh` branch would need cargo, fcb and duckdb. Deciding
# `want_duckdb=0` is worthless if the recipe then calls the baseline
# regardless, so the wiring is checked even though the check is weaker than
# the rest of this file.
# --------------------------------------------------------------------------
case_baseline_invocation_is_guarded() {
  local name="the readbench_duckdb.sh call sits behind the want_duckdb guard"
  local body
  body="$(sed -n '/^bench FOLDER/,/^# The ORDERING-COMPARISON run/p' "$JUSTFILE")"
  if [[ "$body" != *'if [[ "$want_duckdb" -eq 1 ]]; then'* ]]; then
    fail "$name" "no 'if [[ \$want_duckdb -eq 1 ]]' guard found in the bench recipe"
    return
  fi
  if [[ "$body" != *"readbench_duckdb.sh"* ]]; then
    fail "$name" "the bench recipe no longer calls readbench_duckdb.sh"
    return
  fi
  pass "$name"
}

case_block_is_extractable
case_bare_run_omits_the_baseline
case_naming_the_baseline_appends_it
case_explicit_list_without_baseline
case_ordering_run_stays_single_axis
case_prepare_list_invariants
case_baseline_invocation_is_guarded

echo "bench_recipe_test: $PASSED passed, $FAILED failed"
[[ "$FAILED" -eq 0 ]]
