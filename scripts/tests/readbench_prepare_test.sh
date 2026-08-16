#!/usr/bin/env bash
# Tests for `scripts/readbench_prepare.sh`'s format selection and its
# conditional external-binary guards.
#
# Plain bash, no framework: `bats` is not a dependency of this repo. One
# `ok`/`not ok` line per case, non-zero exit if any case fails.
#
# Every case runs the prepare script inside a throwaway sandbox:
#
#   $SANDBOX/repo/scripts/readbench_prepare.sh   a copy of the real script,
#                                                so its `REPO_ROOT` (derived
#                                                from `BASH_SOURCE`) is the
#                                                sandbox, never this checkout
#                                                (and so `bench/tools/` — where
#                                                `scripts/fetch_tools.sh` puts
#                                                citygml-tools — is absent)
#   $SANDBOX/bin/{cargo,fcb,citygml-tools,cjseq}  stubs, prepended to PATH
#   $SANDBOX/data/tiny.city.jsonl                a stand-in CityJSONSeq input
#   $SANDBOX/data/tiny.gml                       a stand-in CityGML input
#   $SANDBOX/data/empty.gml                      one with no city objects at all
#   $SANDBOX/out                                 OUTDIR
#
# The copy is what lets the `cargo` stub plant a fake `target/release/
# cityparquet` without touching the real one, so no case needs a real
# compile, a real `fcb`, or a real conversion — only the selection, guard and
# chain-wiring logic is under test.
#
# PATH is built from a copy of the caller's PATH with every entry that carries
# a real `fcb`, `cjseq` or `citygml-tools` removed, so "that tool is not
# installed" is reproducible on a machine where it IS installed (`cjseq` in
# particular ships in ~/.cargo/bin on the development machine). "cargo was not
# needed" is asserted differently — via a marker file the stub drops — because
# removing cargo from PATH would also remove much else.
#
# `jq` is deliberately NOT stubbed: the stubs below emit real, parseable
# CityJSON, so the script's object-count checks run for real against them.
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TEST_DIR/../.." && pwd)"
PREPARE="$REPO_ROOT/scripts/readbench_prepare.sh"
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

# PATH with every directory holding a real copy of any of the named tools
# dropped, so an "X is not installed" case is reproducible on a machine where
# X IS installed.
tool_free_path() {
  local out="" dir tool drop
  local IFS=:
  for dir in $PATH; do
    if [[ -z "$dir" ]]; then
      continue
    fi
    drop=0
    for tool in "$@"; do
      if [[ -x "$dir/$tool" ]]; then
        drop=1
        break
      fi
    done
    if [[ "$drop" -eq 1 ]]; then
      continue
    fi
    out+="${out:+:}$dir"
  done
  printf '%s' "$out"
}

BASE_PATH="$(tool_free_path fcb cjseq citygml-tools)"

# new_sandbox [stub ...] -> prints the sandbox directory.
# Named stubs ("cargo", "fcb", "citygml-tools", "cjseq") are installed into
# $SANDBOX/bin; anything not named is simply absent from PATH.
new_sandbox() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/readbench_prepare_test.XXXXXX")"
  mkdir -p "$dir/repo/scripts" "$dir/bin" "$dir/data" "$dir/out"
  cp "$PREPARE" "$dir/repo/scripts/readbench_prepare.sh"
  # One CityJSONFeature line — enough for the script's feature count to be 1.
  printf '{"type":"CityJSONFeature","id":"tiny"}\n' >"$dir/data/tiny.city.jsonl"
  # One <cityObjectMember> — enough for the script's CityGML object count to
  # be 1. No stub parses it as XML.
  cat >"$dir/data/tiny.gml" <<'TINY_GML'
<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns="http://www.opengis.net/citygml/2.0">
  <cityObjectMember><Building/></cityObjectMember>
</CityModel>
TINY_GML
  # …and one with none at all, for the "exists but is vacuous" case.
  cat >"$dir/data/empty.gml" <<'EMPTY_GML'
<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns="http://www.opengis.net/citygml/2.0">
</CityModel>
EMPTY_GML

  local stub
  for stub in "$@"; do
    case "$stub" in
      cargo)
        # `cargo build --release -p cityparquet-cli` runs with cwd =
        # REPO_ROOT, so both the marker and the planted binary are written
        # relative to it.
        cat >"$dir/bin/cargo" <<'CARGO_STUB'
#!/usr/bin/env bash
set -euo pipefail
touch cargo-invoked
mkdir -p target/release
cat >target/release/cityparquet <<'CITYPARQUET_STUB'
#!/usr/bin/env bash
set -euo pipefail
# `convert` is the ONLY subcommand the prepare chain may ever run. Artefacts
# derive forward from the source; deriving a competitor's input from the
# format under test (`cityparquet export`) would favour it, so this stub makes
# that a test failure rather than a silent bias.
sub=${1:-}
if [[ "$sub" != "convert" ]]; then
  echo "stub cityparquet: refusing subcommand '$sub' -- the prepare chain must never derive an artefact from CityParquet" >&2
  exit 1
fi
out=""
src=""
shift  # the `convert` subcommand itself
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--output) out=$2; shift 2 ;;
    # Value-taking too: without this its value ("hilbert") would be mistaken
    # for the input path.
    --ordering) shift 2 ;;
    -*) shift ;;
    *) src=$1; shift ;;
  esac
done
[[ -n "$out" ]] || { echo "stub cityparquet: no -o given" >&2; exit 1; }
[[ -n "$src" ]] || { echo "stub cityparquet: no input given" >&2; exit 1; }
mkdir -p "$out"
echo stub >"$out/building.parquet"
# The path actually converted, so a case can assert WHICH file fed the
# package rather than merely trusting the script's own echo of it.
printf '%s\n' "$src" >"$out/stub-source.txt"
CITYPARQUET_STUB
chmod +x target/release/cityparquet
CARGO_STUB
        chmod +x "$dir/bin/cargo"
        ;;
      citygml-tools)
        # `citygml-tools to-cityjson -o DIR FILE` writes DIR/<base>.json (the
        # real tool's naming: the input's basename with its extension replaced,
        # NOT the .city.json the benchmark's artefact names use).
        #
        # $STUB_CITYJSON_OBJECTS (default 1) is how a case makes the converter
        # emit a well-formed but object-less — or a lossy — CityJSON.
        cat >"$dir/bin/citygml-tools" <<'CGT_STUB'
#!/usr/bin/env bash
set -euo pipefail
# A CityJSON document with $1 top-level CityObjects.
cityjson_doc() {
  local n=$1 i objs=""
  for ((i = 0; i < n; i++)); do
    objs+="${objs:+,}\"tiny$i\":{\"type\":\"Building\",\"geometry\":[]}"
  done
  printf '{"type":"CityJSON","version":"2.0","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{%s},"vertices":[]}\n' "$objs"
}
sub=${1:-}
shift || true
outdir=""
src=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--output) outdir=$2; shift 2 ;;
    -*) shift ;;
    *) src=$1; shift ;;
  esac
done
case "$sub" in
  to-cityjson)
    [[ -n "$outdir" && -n "$src" ]] || { echo "stub citygml-tools: need -o and a source" >&2; exit 1; }
    base="$(basename "$src")"; base="${base%.*}"
    mkdir -p "$outdir"
    cityjson_doc "${STUB_CITYJSON_OBJECTS:-1}" >"$outdir/$base.json"
    ;;
  --version|-V) echo "citygml-tools 0.0.0-stub" ;;
  *) echo "stub citygml-tools: unknown command '$sub'" >&2; exit 1 ;;
esac
CGT_STUB
        chmod +x "$dir/bin/citygml-tools"
        ;;
      cjseq)
        # `cjseq cat -f CITYJSON` -> CityJSONSeq on stdout;
        # `cjseq collect -f CITYJSONSEQ` -> CityJSON on stdout.
        #
        # $STUB_SEQ_FEATURES / $STUB_CITYJSON_OBJECTS (both default 1) are how
        # a case makes a hop of the chain emit an empty or a lossy result.
        cat >"$dir/bin/cjseq" <<'CJSEQ_STUB'
#!/usr/bin/env bash
set -euo pipefail
# A CityJSON document with $1 top-level CityObjects.
cityjson_doc() {
  local n=$1 i objs=""
  for ((i = 0; i < n; i++)); do
    objs+="${objs:+,}\"tiny$i\":{\"type\":\"Building\",\"geometry\":[]}"
  done
  printf '{"type":"CityJSON","version":"2.0","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{%s},"vertices":[]}\n' "$objs"
}
sub=${1:-}
shift || true
src=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -f|--file) src=$2; shift 2 ;;
    *) shift ;;
  esac
done
[[ -f "$src" ]] || { echo "stub cjseq: no readable -f input ('$src')" >&2; exit 1; }
case "$sub" in
  cat)
    printf '%s\n' '{"type":"CityJSON","version":"2.0","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{},"vertices":[]}'
    for ((i = 0; i < ${STUB_SEQ_FEATURES:-1}; i++)); do
      printf '{"type":"CityJSONFeature","id":"tiny%s","CityObjects":{"tiny%s":{"type":"Building","geometry":[]}},"vertices":[]}\n' "$i" "$i"
    done
    ;;
  collect)
    cityjson_doc "${STUB_CITYJSON_OBJECTS:-1}"
    ;;
  *) echo "stub cjseq: unknown subcommand '$sub'" >&2; exit 1 ;;
esac
CJSEQ_STUB
        chmod +x "$dir/bin/cjseq"
        ;;
      fcb)
        cat >"$dir/bin/fcb" <<'FCB_STUB'
#!/usr/bin/env bash
set -euo pipefail
sub=${1:-}
shift || true
out=""
src=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    -i) src=$2; shift 2 ;;
    *) shift ;;
  esac
done
case "$sub" in
  # The .fcb's whole content is the path it was serialised from, so a case can
  # assert WHICH file fed it rather than trusting the script's echo.
  ser) printf '%s\n' "$src" >"$out" ;;
  info) printf 'Dataset:\n  Features: 3\n' ;;
  *) echo "stub fcb: unknown subcommand '$sub'" >&2; exit 1 ;;
esac
FCB_STUB
        chmod +x "$dir/bin/fcb"
        ;;
      *)
        echo "new_sandbox: unknown stub '$stub'" >&2
        exit 1
        ;;
    esac
  done
  printf '%s' "$dir"
}

# run_prepare SANDBOX [args ...] -> sets LAST_RC and LAST_LOG.
LAST_RC=0
LAST_LOG=""
run_prepare() {
  local dir=$1
  shift
  LAST_LOG="$dir/run.log"
  set +e
  # CITYGML_TOOLS is blanked, not inherited: a developer who exported it to
  # point at their own checkout would otherwise smuggle a real citygml-tools
  # into the cases that exist to prove it is absent.
  PATH="$dir/bin:$BASE_PATH" CITYGML_TOOLS="" \
    "$dir/repo/scripts/readbench_prepare.sh" "$@" \
    >"$LAST_LOG" 2>&1
  LAST_RC=$?
  set -e
}

log_mentions() {
  grep -qF -- "$1" "$LAST_LOG"
}

# Every negative case below asserts BOTH the script's own exit 1 AND the exact
# wording of the message that arm produces. Neither half is decoration: a bare
# "nonzero exit" also matches bash's own 127 for an unresolvable command, and a
# loose substring ("fcb", "citygml") also matches bash's `fcb: command not
# found` or the script's own `-- formats: citygml` echo. Asserting only those
# weak forms makes the case pass whether the guard fired or was deleted, which
# is precisely the regression each case exists to catch.
expect_guard() {
  local name=$1 needle=$2
  if [[ $LAST_RC -ne 1 ]]; then
    fail "$name" "expected exit 1 from the script's own guard, got $LAST_RC; log: $(cat "$LAST_LOG")"
    return 1
  fi
  if ! log_mentions "$needle"; then
    fail "$name" "expected the guard's own wording '$needle'; log: $(cat "$LAST_LOG")"
    return 1
  fi
  return 0
}

# --------------------------------------------------------------------------
# Case 1: `--formats cityparquet` builds only the CityParquet package, and
# needs no `fcb` (the sandbox has none).
# --------------------------------------------------------------------------
case_cityparquet_only() {
  local name="--formats cityparquet builds only the package and needs no fcb"
  local dir
  dir="$(new_sandbox cargo)"
  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ ! -d "$dir/out/tiny.parquet" ]]; then
    fail "$name" "missing $dir/out/tiny.parquet"
    return
  fi
  local unexpected
  unexpected="$(find "$dir/out" -mindepth 1 -maxdepth 1 \
    ! -name 'tiny.parquet' -print)"
  if [[ -n "$unexpected" ]]; then
    fail "$name" "unrequested artefacts built: $unexpected"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 2: `--formats flatcitybuf` with no `fcb` on PATH fails loudly, naming
# the missing binary. A requested-but-unbuildable format is this script's
# error to report; silently skipping is the coordinator's job.
# --------------------------------------------------------------------------
case_flatcitybuf_without_fcb() {
  local name="--formats flatcitybuf without fcb fails at the guard"
  local dir
  dir="$(new_sandbox cargo)"
  run_prepare "$dir" --formats flatcitybuf "$dir/data/tiny.city.jsonl" "$dir/out"
  # Deliberately the guard's full wording, not a bare "fcb": with fcb genuinely
  # absent, deleting the guard makes the unguarded `fcb ser` die with bash's
  # own "fcb: command not found" at exit 127, which a loose match would accept.
  if ! expect_guard "$name" "error: fcb not found on PATH"; then
    return
  fi
  # …and nothing was built on the way to the failure.
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "a run that failed its guard still built something"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 3: `--formats flatcitybuf` never builds the CityParquet CLI — the
# cargo guard is as conditional as the fcb one.
# --------------------------------------------------------------------------
case_flatcitybuf_skips_the_cli_build() {
  local name="--formats flatcitybuf does not build the CityParquet CLI"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats flatcitybuf "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ -f "$dir/repo/cargo-invoked" ]]; then
    fail "$name" "cargo was invoked for a format that does not need it"
    return
  fi
  if [[ ! -s "$dir/out/tiny.fcb" ]]; then
    fail "$name" "missing $dir/out/tiny.fcb"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 4: an unknown format name is rejected, listing the valid ones.
# --------------------------------------------------------------------------
case_unknown_format_rejected() {
  local name="an unknown format is rejected, listing the valid names"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats cityparquet,bogus "$dir/data/tiny.city.jsonl" "$dir/out"
  # The offending name alone would be a weak assertion: `-- formats: …` echoes
  # the whole request back, so "bogus" appears in the log either way. Match the
  # rejection's own wording.
  if ! expect_guard "$name" "error: unknown format 'bogus'"; then
    return
  fi
  local valid
  for valid in cityparquet-hilbert flatcitybuf cityjsonseq-gz; do
    if ! log_mentions "$valid"; then
      fail "$name" "message does not list '$valid'; log: $(cat "$LAST_LOG")"
      return
    fi
  done
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "rejected run still built something"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 5: no `--formats` on a CityJSONSeq input builds every artefact that IS
# derivable from it — which is all of them but `citygml`. CityGML is reported
# plainly as not derivable rather than synthesised by a reverse conversion: a
# `from-cityjson` artefact is a round-trip product, not the source data, so
# measuring it would be dishonest.
#
# The sandbox deliberately has NO citygml-tools stub, which is the other half
# of the claim: the CityJSON path never reaches for it.
# --------------------------------------------------------------------------
case_default_on_cityjsonseq_skips_only_citygml() {
  local name="no --formats on a CityJSONSeq input builds everything but citygml"
  local dir
  dir="$(new_sandbox cargo fcb cjseq)"
  run_prepare "$dir" "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  local artefact
  for artefact in tiny.city.json tiny.parquet tiny-hilbert.parquet tiny.fcb tiny.jsonl.gz; do
    if [[ ! -e "$dir/out/$artefact" ]]; then
      fail "$name" "missing $artefact; log: $(cat "$LAST_LOG")"
      return
    fi
  done
  if [[ -e "$dir/out/tiny.gml" ]]; then
    fail "$name" "synthesised a CityGML artefact by reverse conversion"
    return
  fi
  # The full sentence, not a bare "citygml": the `-- formats: …` echo prints
  # that word on every run of the default set.
  if ! log_mentions "citygml: not derivable from a CityJSON input"; then
    fail "$name" "did not report citygml as not derivable; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 6: `cityjsonseq` has no artefact — it IS the input — so requesting it
# must succeed as a no-op, not fail as "cannot build".
# --------------------------------------------------------------------------
case_cityjsonseq_is_a_no_op() {
  local name="--formats cityjsonseq succeeds and builds nothing"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats cityjsonseq "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "built an artefact for the input itself"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7: a CityGML input builds the whole forward chain
#
#   CityGML --citygml-tools--> CityJSON --cjseq cat--> CityJSONSeq
#                                                  |-> gz | fcb | CityParquet
#
# and nothing in it derives from CityParquet. That last clause is what the
# `cityparquet` stub enforces: it refuses every subcommand but `convert`, so
# an `export`-fed chain fails this case instead of quietly biasing the
# comparison towards the format under test.
#
# A second run then proves the new steps kept the script's idempotency.
# --------------------------------------------------------------------------
case_citygml_input_builds_the_whole_chain() {
  local name="a CityGML input builds the whole forward chain"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  local artefact
  for artefact in tiny.gml tiny.city.json tiny.city.jsonl tiny.jsonl.gz \
    tiny.fcb tiny.parquet tiny-hilbert.parquet; do
    if [[ ! -e "$dir/out/$artefact" ]]; then
      fail "$name" "missing $artefact; log: $(cat "$LAST_LOG")"
      return
    fi
  done
  # The CityJSONSeq must have been cut from the CityJSON the chain just
  # produced — not from the .gml, and certainly not from the package. The log
  # line names both endpoints, so it pins the edge, not merely the step.
  if ! log_mentions "cjseq cat $dir/out/tiny.city.json -> $dir/out/tiny.city.jsonl"; then
    fail "$name" "the CityJSONSeq was not cut from the derived CityJSON; log: $(cat "$LAST_LOG")"
    return
  fi
  # …and FlatCityBuf and both CityParquet packages must have been fed that
  # SAME seq, which is the whole basis of their comparison being fair.
  #
  # Asserted from what the stubs RECORD (the .fcb's content is the path it was
  # serialised from; each package carries a stub-source.txt), never from the
  # script's own echo of the path: a step that logs one file and converts
  # another would sail past a log-only assertion.
  local fed
  for fed in "$dir/out/tiny.fcb" "$dir/out/tiny.parquet/stub-source.txt" \
    "$dir/out/tiny-hilbert.parquet/stub-source.txt"; do
    if [[ "$(cat "$fed")" != "$dir/out/tiny.city.jsonl" ]]; then
      fail "$name" "$fed records input '$(cat "$fed")', not the derived CityJSONSeq"
      return
    fi
  done
  # The gzip baseline is the same bytes again, so its content is the proof.
  if ! gunzip -c "$dir/out/tiny.jsonl.gz" | cmp -s - "$dir/out/tiny.city.jsonl"; then
    fail "$name" "tiny.jsonl.gz is not a gzip of the derived CityJSONSeq"
    return
  fi
  run_prepare "$dir" "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "second run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "skip $dir/out/tiny.city.json (already present)"; then
    fail "$name" "second run rebuilt the CityJSON; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7b: asking for `citygml` explicitly from a CityJSON input is reported,
# not obeyed and not fatal. This is the ONE documented exception to "a
# requested format that cannot be built is an error here": the only way to
# obey would be a reverse conversion, and a round-trip artefact is not the
# source data.
# --------------------------------------------------------------------------
case_explicit_citygml_from_cityjson_is_reported_not_fatal() {
  local name="--formats citygml on a CityJSON input is reported, not fatal"
  local dir
  dir="$(new_sandbox cargo fcb cjseq)"
  run_prepare "$dir" --formats citygml "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "citygml: not derivable from a CityJSON input"; then
    fail "$name" "did not report why; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "built something for a format it could not derive"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7c: a CityGML input with no citygml-tools anywhere fails at the guard,
# in the script's own words.
# --------------------------------------------------------------------------
case_citygml_input_without_citygml_tools() {
  local name="a CityGML input without citygml-tools fails at the guard"
  local dir
  dir="$(new_sandbox cargo cjseq)"
  run_prepare "$dir" --formats cityjson "$dir/data/tiny.gml" "$dir/out"
  # Deliberately the guard's full wording: with citygml-tools genuinely absent,
  # deleting the guard makes the unguarded invocation die with bash's own
  # "citygml-tools: command not found" at exit 127, which a loose match on
  # "citygml-tools" would happily accept.
  if ! expect_guard "$name" "error: citygml-tools not found"; then
    return
  fi
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "a run that failed its guard still built something"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7d: `cjseq` is guarded the same way — it is the hop citygml-tools
# cannot do, so a CityJSON artefact cannot be cut from a CityJSONSeq input
# without it.
# --------------------------------------------------------------------------
case_cityjson_without_cjseq() {
  local name="--formats cityjson without cjseq fails at the guard"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats cityjson "$dir/data/tiny.city.jsonl" "$dir/out"
  if ! expect_guard "$name" "error: cjseq not found"; then
    return
  fi
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "a run that failed its guard still built something"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Cases 7e-7g: an artefact that exists but holds no city objects is worse than
# one that is missing — the benchmark would measure it and report a timing for
# a dataset that is not there. Each stage of the chain is checked for content,
# so each gets a case; the stubs' $STUB_* knobs make the emptiness happen at
# exactly one stage at a time.
# --------------------------------------------------------------------------
case_object_less_citygml_is_rejected() {
  local name="a CityGML input with no city objects is rejected"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" --formats citygml "$dir/data/empty.gml" "$dir/out"
  if ! expect_guard "$name" "contains no <cityObjectMember> elements"; then
    return
  fi
  pass "$name"
}

case_object_less_cityjson_is_rejected() {
  local name="an object-less CityJSON artefact is rejected"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  STUB_CITYJSON_OBJECTS=0 \
    run_prepare "$dir" --formats cityjson "$dir/data/tiny.gml" "$dir/out"
  if ! expect_guard "$name" "contains no top-level CityObjects"; then
    return
  fi
  pass "$name"
}

case_feature_less_cityjsonseq_is_rejected() {
  local name="a feature-less CityJSONSeq artefact is rejected"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  STUB_SEQ_FEATURES=0 \
    run_prepare "$dir" --formats cityjsonseq "$dir/data/tiny.gml" "$dir/out"
  if ! expect_guard "$name" "contains no CityJSONFeature lines"; then
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7h: a hop that silently drops objects is surfaced, not swallowed. The
# design spec's fairness caveats say a CityGML row and a CityParquet row are
# only comparable if the conversion between them was lossless, and that this
# is asserted rather than assumed — so a count that shrinks is reported. It is
# a warning, not a failure: real CityGML routinely carries ADE content
# citygml-tools skips, and the run is still worth having with the loss on the
# record.
# --------------------------------------------------------------------------
case_conversion_loss_is_reported() {
  local name="a lossy hop is reported without failing the run"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  STUB_CITYJSON_OBJECTS=2 STUB_SEQ_FEATURES=1 \
    run_prepare "$dir" --formats cityjsonseq "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "a detectable loss failed the run instead of reporting it; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "conversion loss:"; then
    fail "$name" "the count drop was not reported; log: $(cat "$LAST_LOG")"
    return
  fi
  # Both endpoints named, so the report says WHERE the loss happened — and
  # both hops are watched, including the CityGML -> CityJSON one, which is
  # where loss is likeliest and which this run never asked for an artefact
  # from (the source .gml is still the baseline the CityJSON is checked
  # against).
  if ! log_mentions "$dir/data/tiny.gml has 1 top-level object(s) but"; then
    fail "$name" "the CityGML -> CityJSON hop was not watched; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "$dir/out/tiny.city.json has 2 top-level object(s) but"; then
    fail "$name" "the CityJSON -> CityJSONSeq hop was not watched; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 8: idempotency survives the flag — a second run skips what is already
# there rather than rebuilding it.
# --------------------------------------------------------------------------
case_second_run_skips() {
  local name="a re-run skips an artefact that is already present"
  local dir
  dir="$(new_sandbox cargo)"
  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "first run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "second run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  # The full skip line, not a bare "skip": this must prove THIS artefact was
  # recognised as already present, not merely that the word appeared somewhere.
  if ! log_mentions "skip $dir/out/tiny.parquet (already present)"; then
    fail "$name" "second run did not skip the existing package; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 9: the script's format vocabulary must equal the Rust one.
#
# `Format::ALL` (crates/cityparquet-readbench/src/format.rs) owns the names;
# the shell script cannot import it, so it carries a copy — and a copy of a
# vocabulary is exactly how this benchmark's CSV header contract drifted into
# three incompatible versions. This case reads BOTH lists out of their own
# source files and compares them, the same trick
# `bench/plot/tests/test_csv_contract.py` uses for the CSV header.
#
# `duckdb-parquet` is deliberately excluded: it is an SQL-engine baseline
# driven by `scripts/readbench_duckdb.sh` over an already-prepared
# CityParquet package, so this script has no artefact to build for it (the
# Rust side says the same thing as `Artefact::NotCoordinated`).
# --------------------------------------------------------------------------
case_vocabulary_matches_the_rust_enum() {
  local name="the script's format list matches Format::ALL minus duckdb-parquet"
  local rust_tags script_tags
  rust_tags="$(awk '/pub fn as_str/,/^        }$/' "$FORMAT_RS" \
    | grep -oE '=> "[a-z0-9-]+"' \
    | sed 's/.*"\(.*\)"/\1/' \
    | grep -v '^duckdb-parquet$' \
    | tr '\n' ' ')"
  script_tags="$(sed -n 's/^VALID_FORMATS=(\(.*\))$/\1/p' "$PREPARE" \
    | tr -s ' ' ' ')"
  rust_tags="$(echo "$rust_tags" | xargs)"
  script_tags="$(echo "$script_tags" | xargs)"
  if [[ -z "$rust_tags" ]]; then
    fail "$name" "could not read the tag list out of $FORMAT_RS"
    return
  fi
  if [[ -z "$script_tags" ]]; then
    fail "$name" "could not read VALID_FORMATS out of $PREPARE"
    return
  fi
  if [[ "$rust_tags" != "$script_tags" ]]; then
    fail "$name" "rust: [$rust_tags] != script: [$script_tags]"
    return
  fi
  pass "$name"
}

case_cityparquet_only
case_flatcitybuf_without_fcb
case_flatcitybuf_skips_the_cli_build
case_unknown_format_rejected
case_default_on_cityjsonseq_skips_only_citygml
case_cityjsonseq_is_a_no_op
case_citygml_input_builds_the_whole_chain
case_explicit_citygml_from_cityjson_is_reported_not_fatal
case_citygml_input_without_citygml_tools
case_cityjson_without_cjseq
case_object_less_citygml_is_rejected
case_object_less_cityjson_is_rejected
case_feature_less_cityjsonseq_is_rejected
case_conversion_loss_is_reported
case_second_run_skips
case_vocabulary_matches_the_rust_enum

echo "readbench_prepare_test: $PASSED passed, $FAILED failed"
[[ $FAILED -eq 0 ]]
