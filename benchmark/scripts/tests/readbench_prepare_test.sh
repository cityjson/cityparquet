#!/usr/bin/env bash
# Tests for `benchmark/scripts/readbench_prepare.sh`'s format selection and its
# conditional external-binary guards.
#
# Plain bash, no framework: `bats` is not a dependency of this repo. One
# `ok`/`not ok` line per case, non-zero exit if any case fails.
#
# Every case runs the prepare script inside a throwaway sandbox:
#
#   $SANDBOX/repo/benchmark/scripts/readbench_prepare.sh   a copy of the real script,
#   $SANDBOX/repo/lib/cityparquet-rs/             where it cds to build the CLI,
#                                                so its `BENCHMARK_DIR` (derived
#                                                from `BASH_SOURCE`) is the
#                                                sandbox, never this checkout
#                                                (and so `benchmark/formats/tools/` — where
#                                                `benchmark/scripts/fetch_tools.sh` puts
#                                                citygml-tools — is absent)
#   $SANDBOX/bin/{cargo,fcb,citygml-tools,cjseq}  stubs, prepended to PATH
#   $SANDBOX/data/tiny.city.jsonl                a stand-in CityJSONSeq input
#   $SANDBOX/data/tiny.city.json                 a stand-in whole-document CityJSON
#   $SANDBOX/data/tiny.gml                       CityGML 2.0, 1 member, gml:id
#   $SANDBOX/data/empty.gml                      no city objects at all
#   $SANDBOX/data/no_id.gml                      a member with no gml:id
#   $SANDBOX/data/citygml1.gml                   the version the reader refuses
#   $SANDBOX/data/multiline.gml                  member tags split across lines
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
# CityJSON, so the script's object-count checks run for real against them. Its
# own guard case hides it differently — see `jq_free_bin`.
#
# The stubs RECORD what they were fed (`"stub_source"` in the CityJSON they
# emit, `stub-source.txt` in a package, the whole content of a .fcb), because
# an assertion made against the script's own log line proves only that the
# script printed a path, not that it read one: a mutation swapping `cjseq cat
# -f "$CITYJSON_OUT"` for `-f "$INPUT"` once left this whole suite green.
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHMARK_DIR="$(cd "$TEST_DIR/../.." && pwd)"
PREPARE="$BENCHMARK_DIR/scripts/readbench_prepare.sh"
FORMAT_RS="$BENCHMARK_DIR/readbench/src/format.rs"

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

# `jq` cannot be hidden the way `fcb` and `cjseq` can. Those live in
# ~/.cargo/bin, a directory that holds nothing else the script needs, so
# dropping it from PATH is harmless; `jq` lives in /usr/bin beside `bash`,
# `dirname` and everything else, and dropping THAT leaves the script unable to
# start (exit 127 from `env bash`, which is not the guard firing).
#
# So the jq case gets a hand-built PATH instead: one directory of symlinks to
# exactly the commands the script can reach before the guard. If the guard is
# deleted the run does not limp on — it dies on the first missing tool — which
# is why the case asserts the guard's own wording and exit 1, not merely
# "non-zero".
jq_free_bin() {
  local dir=$1 tool resolved
  mkdir -p "$dir/nojq"
  # `env` resolves `bash` through PATH, and BENCHMARK_DIR is computed with
  # `dirname`; the rest are what the build steps would need if the guard were
  # gone and the run continued.
  for tool in bash env dirname basename mkdir cat sed grep awk head tr wc \
    find mktemp cp mv rm gzip gunzip; do
    resolved="$(PATH="$BASE_PATH" command -v "$tool" 2>/dev/null || true)"
    if [[ -n "$resolved" ]]; then
      ln -sf "$resolved" "$dir/nojq/$tool"
    fi
  done
  printf '%s' "$dir/nojq"
}

# new_sandbox [stub ...] -> prints the sandbox directory.
# Named stubs ("cargo", "fcb", "citygml-tools", "cjseq") are installed into
# $SANDBOX/bin; anything not named is simply absent from PATH.
new_sandbox() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/readbench_prepare_test.XXXXXX")"
  mkdir -p "$dir/repo/benchmark/scripts" "$dir/repo/lib/cityparquet-rs" "$dir/bin" "$dir/data" "$dir/out"
  cp "$PREPARE" "$dir/repo/benchmark/scripts/readbench_prepare.sh"
  # One CityJSONFeature line — enough for the script's feature count to be 1.
  printf '{"type":"CityJSONFeature","id":"tiny"}\n' >"$dir/data/tiny.city.jsonl"
  # A whole-document CityJSON input: the other shape the catalogue corpus
  # ships in, and the one for which `cityjsonseq` used to build no artefact at
  # all (so `cityjson` and `cityjsonseq` were measured over one file).
  printf '{"type":"CityJSON","version":"2.0","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{"tiny0":{"type":"Building","geometry":[]}},"vertices":[]}\n' \
    >"$dir/data/tiny.city.json"
  # CityGML inputs. No stub parses them as XML; they exist so the script's own
  # namespace sniff and its two counts (top-level members, and how many of them
  # carry a gml:id) have something to read.
  #
  # The good one: CityGML 2.0, one member, WITH a gml:id — the shape every
  # positive CityGML case needs, because an id-less document is now refused.
  cat >"$dir/data/tiny.gml" <<'TINY_GML'
<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml">
  <cityObjectMember><Building gml:id="tiny-1"/></cityObjectMember>
</CityModel>
TINY_GML
  # No members at all, for the "exists but is vacuous" case.
  cat >"$dir/data/empty.gml" <<'EMPTY_GML'
<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml">
</CityModel>
EMPTY_GML
  # Two members, only one of which carries a gml:id — the shape of Riga's
  # published atgazene_lod2.gml (703 top-level objects, 0 with gml:id; identity
  # lives in a gen:intAttribute), which citygml-tools would give fresh random
  # ids, making its id-lookup row a miss beside every other format's hit.
  cat >"$dir/data/no_id.gml" <<'NO_ID_GML'
<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml">
  <cityObjectMember><Building gml:id="has-one"/></cityObjectMember>
  <cityObjectMember><Building><gen:intAttribute name="OBJECTID"/></Building></cityObjectMember>
</CityModel>
NO_ID_GML
  # CityGML 1.0: convertible, but the benchmark's own reader refuses it, so
  # preparing it would leave an artefact that can never be measured.
  cat >"$dir/data/citygml1.gml" <<'CITYGML1_GML'
<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns="http://www.opengis.net/citygml/1.0" xmlns:gml="http://www.opengis.net/gml">
  <cityObjectMember><Building gml:id="old-1"/></cityObjectMember>
</CityModel>
CITYGML1_GML
  # Opening tags whose attributes continue on the next line — valid, and
  # invisible to a line-oriented count.
  cat >"$dir/data/multiline.gml" <<'MULTILINE_GML'
<?xml version="1.0" encoding="UTF-8"?>
<core:CityModel xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml">
  <core:cityObjectMember
      >
    <Building
        gml:id="split-1"/>
  </core:cityObjectMember>
  <core:cityObjectMember
      >
    <Building
        gml:id="split-2"/>
  </core:cityObjectMember>
</core:CityModel>
MULTILINE_GML

  local stub
  for stub in "$@"; do
    case "$stub" in
      cargo)
        # `cargo build --release -p cityparquet-cli` runs with cwd =
        # $MONO_ROOT/lib/cityparquet-rs — the LIBRARY workspace, which owns the
        # converter — so both the marker and the planted binary land there.
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
# The WHOLE command line, kept before the parse loop below consumes it. A
# recorded `src` proves which file was converted; only the full argv proves
# HOW -- and `--ordering hilbert` is the entire difference between the two
# CityParquet rows the ordering benchmark compares.
argv=("$@")
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
printf '%s\n' "${argv[@]}" >"$out/stub-argv.txt"
CITYPARQUET_STUB
chmod +x target/release/cityparquet
CARGO_STUB
        chmod +x "$dir/bin/cargo"
        ;;
      citygml-tools)
        # `citygml-tools to-cityjson -o DIR FILE` writes DIR/<base>.json, and
        # `from-cityjson -o DIR FILE` writes DIR/<base>.gml (the real tool's
        # naming in both directions: the input's basename with its extension
        # replaced, NOT the .city.json/.gml the benchmark's artefact names use
        # — the script globs for the one file produced and renames it).
        #
        # $STUB_CITYJSON_OBJECTS (default 1) is how a case makes the converter
        # emit a well-formed but object-less — or a lossy — CityJSON.
        #
        # The from-cityjson direction has three knobs of its own, one per
        # check `readbench_prepare.sh` runs against the artefact it gets back:
        #   $STUB_GML_OBJECTS      (default 1) top-level members written
        #   $STUB_GML_WITHOUT_IDS  (default 0) how many of them lack a gml:id
        #   $STUB_GML_VERSION      (default 2.0) the declared CityGML version
        cat >"$dir/bin/citygml-tools" <<'CGT_STUB'
#!/usr/bin/env bash
set -euo pipefail
# A CityJSON document with $1 top-level CityObjects, recording in
# "stub_source" the file ($2) it was derived from — so a case can assert WHICH
# file fed this hop instead of trusting the script's own echo of the path.
cityjson_doc() {
  local n=$1 src=$2 i objs=""
  for ((i = 0; i < n; i++)); do
    objs+="${objs:+,}\"tiny$i\":{\"type\":\"Building\",\"geometry\":[]}"
  done
  printf '{"type":"CityJSON","version":"2.0","stub_source":"%s","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{%s},"vertices":[]}\n' "$src" "$objs"
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
# A CityGML document with $1 top-level members, of which the LAST $2 carry no
# gml:id, declaring version $3. The stub_source comment records the file it
# was derived from, the same way cityjson_doc's "stub_source" key does.
citygml_doc() {
  local n=$1 without=$2 version=$3 src=$4 i id
  printf '<?xml version="1.0" encoding="UTF-8"?>\n'
  printf '<!-- stub_source:%s -->\n' "$src"
  printf '<core:CityModel xmlns:core="http://www.opengis.net/citygml/%s"' "$version"
  printf ' xmlns:bldg="http://www.opengis.net/citygml/building/%s"' "$version"
  printf ' xmlns:gml="http://www.opengis.net/gml">\n'
  for ((i = 0; i < n; i++)); do
    if ((i >= n - without)); then
      id=""
    else
      id=" gml:id=\"tiny$i\""
    fi
    printf '<core:cityObjectMember><bldg:Building%s/></core:cityObjectMember>\n' "$id"
  done
  printf '</core:CityModel>\n'
}
case "$sub" in
  to-cityjson)
    [[ -n "$outdir" && -n "$src" ]] || { echo "stub citygml-tools: need -o and a source" >&2; exit 1; }
    base="$(basename "$src")"; base="${base%.*}"
    mkdir -p "$outdir"
    cityjson_doc "${STUB_CITYJSON_OBJECTS:-1}" "$src" >"$outdir/$base.json"
    ;;
  from-cityjson)
    [[ -n "$outdir" && -n "$src" ]] || { echo "stub citygml-tools: need -o and a source" >&2; exit 1; }
    base="$(basename "$src")"; base="${base%.*}"
    mkdir -p "$outdir"
    citygml_doc "${STUB_GML_OBJECTS:-1}" "${STUB_GML_WITHOUT_IDS:-0}" \
      "${STUB_GML_VERSION:-2.0}" "$src" >"$outdir/$base.gml"
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
# A CityJSON document with $1 top-level CityObjects, recording in
# "stub_source" the file ($2) it was derived from — so a case can assert WHICH
# file fed this hop instead of trusting the script's own echo of the path.
cityjson_doc() {
  local n=$1 src=$2 i objs=""
  for ((i = 0; i < n; i++)); do
    objs+="${objs:+,}\"tiny$i\":{\"type\":\"Building\",\"geometry\":[]}"
  done
  printf '{"type":"CityJSON","version":"2.0","stub_source":"%s","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{%s},"vertices":[]}\n' "$src" "$objs"
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
# $STUB_CJSEQ_FAIL names a subcommand that should emit a partial result and
# then die, so a case can prove no truncated artefact is left behind.
case "$sub" in
  cat)
    # The header line records its source, so the CityJSONSeq artefact itself
    # carries proof of which document it was cut from.
    printf '{"type":"CityJSON","version":"2.0","stub_source":"%s","transform":{"scale":[1,1,1],"translate":[0,0,0]},"CityObjects":{},"vertices":[]}\n' "$src"
    if [[ "${STUB_CJSEQ_FAIL:-}" == "cat" ]]; then
      printf '{"type":"CityJSONFeature","id":"trunc'
      exit 1
    fi
    for ((i = 0; i < ${STUB_SEQ_FEATURES:-1}; i++)); do
      printf '{"type":"CityJSONFeature","id":"tiny%s","CityObjects":{"tiny%s":{"type":"Building","geometry":[]}},"vertices":[]}\n' "$i" "$i"
    done
    ;;
  collect)
    if [[ "${STUB_CJSEQ_FAIL:-}" == "collect" ]]; then
      printf '{"type":"CityJSON","version":'
      exit 1
    fi
    cityjson_doc "${STUB_CITYJSON_OBJECTS:-1}" "$src"
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
# The sandbox root ($0 is $SANDBOX/bin/fcb), where the argv record goes: NOT
# in OUTDIR, which cases check for unrequested artefacts.
SANDBOX="$(cd "$(dirname "$0")/.." && pwd)"
sub=${1:-}
# The whole command line, before the parse loop consumes it: `-A` (build the
# all-attribute B+-tree) is what makes FlatCityBuf's indexed rows indexed, and
# nothing but the argv can prove it was passed.
printf '%s\n' "$@" >"$SANDBOX/fcb-$sub-argv.txt"
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
      gzip)
        # A RECORDING PASS-THROUGH, not a fake: the case that reads the argv
        # back also needs a real gzip stream (another case gunzips the
        # artefact and compares it to the CityJSONSeq). The real binary is
        # resolved and baked in here, because the stub shadows `gzip` on PATH
        # and calling it by name would recurse.
        local real_gzip
        real_gzip="$(PATH="$BASE_PATH" command -v gzip)"
        cat >"$dir/bin/gzip" <<GZIP_STUB
#!/usr/bin/env bash
set -euo pipefail
SANDBOX="\$(cd "\$(dirname "\$0")/.." && pwd)"
printf '%s\n' "\$@" >"\$SANDBOX/gzip-argv.txt"
exec "$real_gzip" "\$@"
GZIP_STUB
        chmod +x "$dir/bin/gzip"
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
  PATH="$dir/bin:${RUN_PREPARE_PATH:-$BASE_PATH}" CITYGML_TOOLS="" \
    "$dir/repo/benchmark/scripts/readbench_prepare.sh" "$@" \
    >"$LAST_LOG" 2>&1
  LAST_RC=$?
  set -e
}

log_mentions() {
  grep -qF -- "$1" "$LAST_LOG"
}

# A WHOLE log line, not a substring of one. `log_mentions ".../tiny.city.json"`
# also matches a line naming `tiny.city.jsonl`, so a case pinning which
# artefact was reported could pass while the script reported a different one
# (it did: the intermediates case below).
log_has_line() {
  grep -qFx -- "$1" "$LAST_LOG"
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
  # `.readbench-chain` is provenance, not an artefact: the chain-version stamp
  # every run writes (see `CHAIN_VERSION` in the script). It is excluded by
  # name everywhere an "exactly these artefacts" assertion is made, so the
  # assertions stay about artefacts.
  unexpected="$(find "$dir/out" -mindepth 1 -maxdepth 1 \
    ! -name 'tiny.parquet' ! -name '.readbench-chain' -print)"
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
  if [[ -f "$dir/repo/lib/cityparquet-rs/cargo-invoked" ]]; then
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
# Case 5: no `--formats` on a CityJSONSeq input builds EVERY artefact,
# `citygml` included — synthesised from the CityJSON stage by
# `from-cityjson`. This case used to assert the opposite (that CityGML was
# reported as not derivable and skipped); see the header's CITYGML IS
# SYNTHESISED block for why that reversed. The property that matters now is
# that all EIGHT formats exist for one input, because a dataset producing
# seven of them contributes a comparison with the baseline missing.
#
# The synthesised CityGML is derived from the CityJSON STAGE, not from the
# input directly — asserted below from the stub's recorded source, not from
# the script's own echo of a path.
# --------------------------------------------------------------------------
case_default_on_cityjsonseq_builds_every_format() {
  local name="no --formats on a CityJSONSeq input builds every artefact, citygml included"
  local dir
  dir="$(new_sandbox cargo fcb cjseq citygml-tools)"
  run_prepare "$dir" "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  local artefact
  for artefact in tiny.gml tiny.city.json tiny.city.jsonl tiny.parquet \
    tiny-hilbert.parquet tiny.fcb tiny.jsonl.gz; do
    if [[ ! -e "$dir/out/$artefact" ]]; then
      fail "$name" "missing $artefact; log: $(cat "$LAST_LOG")"
      return
    fi
  done
  # The CityJSON was collected from the INPUT itself — never out of a
  # CityParquet package. Asserted from the artefact's recorded source.
  if ! grep -qF "\"stub_source\":\"$dir/data/tiny.city.jsonl\"" "$dir/out/tiny.city.json"; then
    fail "$name" "the CityJSON was not collected from the input; got: $(cat "$dir/out/tiny.city.json")"
    return
  fi
  # ...and the CityGML from the CityJSON STAGE, keeping the chain's one rule
  # (each artefact derives from the one before it) true in this direction too.
  if ! grep -qF "stub_source:$dir/out/tiny.city.json" "$dir/out/tiny.gml"; then
    fail "$name" "the CityGML was not synthesised from the CityJSON stage; got: $(head -2 "$dir/out/tiny.gml")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 6: `cityjsonseq` from a CityJSONSeq input is a real artefact in OUTDIR,
# a copy of the input.
#
# It used to be a no-op ("the artefact IS the input, read in place"), and that
# is the shape of THE bug this file's suite missed: the coordinator resolves
# every format to an OUTDIR artefact, so with none built the `cityjsonseq` row
# fell back to `--input` — which on the catalogue corpus is a `.gml` or a
# `.city.json`, i.e. another format entirely, published under this one's name.
# A copy costs one file; a wrong number costs a paper.
# --------------------------------------------------------------------------
case_cityjsonseq_is_materialised_from_a_seq_input() {
  local name="--formats cityjsonseq copies the input into OUTDIR as the artefact"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats cityjsonseq "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ ! -s "$dir/out/tiny.city.jsonl" ]]; then
    fail "$name" "no CityJSONSeq artefact in OUTDIR; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! cmp -s "$dir/data/tiny.city.jsonl" "$dir/out/tiny.city.jsonl"; then
    fail "$name" "the artefact is not the input's own bytes"
    return
  fi
  # …and nothing else: a copy is all this format needs, and no CityJSON hop
  # was taken to make it (the sandbox has no cjseq at all).
  local unexpected
  unexpected="$(find "$dir/out" -mindepth 1 -maxdepth 1 \
    ! -name 'tiny.city.jsonl' ! -name '.readbench-chain' -print)"
  if [[ -n "$unexpected" ]]; then
    fail "$name" "unrequested artefacts built: $unexpected"
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
  # produced — not from the .gml, and certainly not from the package. This is
  # THE edge the chain's fairness rests on, so it is asserted from the
  # artefact's own recorded source, not from the script's echo: a hop that
  # logs one file and reads another would sail past a log-only match (it did,
  # until the reviewer mutated `cjseq cat -f "$CITYJSON_OUT"` to `-f "$INPUT"`
  # and the whole suite stayed green).
  if ! grep -qF "\"stub_source\":\"$dir/out/tiny.city.json\"" "$dir/out/tiny.city.jsonl"; then
    fail "$name" "the CityJSONSeq records source '$(head -1 "$dir/out/tiny.city.jsonl")', not the derived CityJSON"
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
# Case 7b: asking for `citygml` ALONE from a CityJSON input builds it, and
# drags in the CityJSON stage it has to be converted back from.
#
# That intermediate is the point of the case. `citygml` is the only requested
# format, so nothing else needs a CityJSON — but `from-cityjson` does, and a
# NEED_CITYJSON that failed to account for the synthesis path would leave this
# run trying to convert a file that was never written.
# --------------------------------------------------------------------------
case_explicit_citygml_from_cityjson_is_synthesised() {
  local name="--formats citygml on a CityJSON input synthesises it via a CityJSON stage"
  local dir
  dir="$(new_sandbox cargo fcb cjseq citygml-tools)"
  run_prepare "$dir" --formats citygml "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ ! -e "$dir/out/tiny.gml" ]]; then
    fail "$name" "no CityGML artefact; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! grep -qF "stub_source:$dir/out/tiny.city.json" "$dir/out/tiny.gml"; then
    fail "$name" "the CityGML was not synthesised from the CityJSON stage; got: $(head -2 "$dir/out/tiny.gml")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7b-i: a synthesised CityGML whose objects lack a `gml:id` is REFUSED,
# not published.
#
# The same defect the preflight refuses in a CityGML INPUT, on the other side
# of the conversion. citygml-tools mints a fresh random id for an object that
# has none, so the id the benchmark samples from the CityJSONSeq would be
# absent from the .gml entirely and `id-lookup` would score a miss beside
# every other format's hit — not a slower lookup, a different query. Nothing
# downstream catches it: the coordinator's cross-format self-consistency check
# covers AttrFilter(object_type) only, never IdLookup.
#
# Not expected to fire in practice (an object that reached the CityJSON stage
# has a key, which is what becomes the gml:id), which is exactly why it is
# tested: an unexercised check is one that stops working at the next tool
# upgrade without anyone noticing.
# --------------------------------------------------------------------------
case_synthesised_citygml_without_ids_is_refused() {
  local name="a synthesised CityGML short of gml:ids is refused"
  local dir
  dir="$(new_sandbox cargo fcb cjseq citygml-tools)"
  STUB_GML_OBJECTS=3 STUB_GML_WITHOUT_IDS=1 \
    run_prepare "$dir" --formats citygml "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "accepted a synthesised CityGML with a missing gml:id"
    return
  fi
  if ! log_mentions "only 2 of 3 top-level objects carry a gml:id"; then
    fail "$name" "did not report the missing ids; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7b-ii: a synthesised CityGML that is not version 2.0 is REFUSED.
#
# citygml-tools' own `from-cityjson` default is CityGML 3.0, and this
# repository's reader accepts only 2.0 (`sniff_citygml`) — so a dropped `-v
# 2.0` produces an artefact that converts cleanly, passes every file-exists
# check, and can never be read. The failure would otherwise surface as an
# unexplained missing `citygml` row per dataset.
# --------------------------------------------------------------------------
case_synthesised_citygml_wrong_version_is_refused() {
  local name="a synthesised CityGML that is not 2.0 is refused"
  local dir
  dir="$(new_sandbox cargo fcb cjseq citygml-tools)"
  STUB_GML_VERSION=3.0 \
    run_prepare "$dir" --formats citygml "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "accepted a synthesised CityGML 3.0 artefact"
    return
  fi
  if ! log_mentions "declares CityGML '3.0', not 2.0"; then
    fail "$name" "did not report the version; log: $(cat "$LAST_LOG")"
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
# Case 7i: a CityGML document whose top-level objects carry no `gml:id` is
# refused outright.
#
# citygml-tools mints a fresh random id for such an object — a different one
# on every run — so the id the benchmark samples (from the derived CityJSONSeq)
# does not appear in the .gml at all. `citygml`'s id-lookup then scores a MISS
# beside every other format's hit: not a slower lookup, a different query, and
# never publishable. The coordinator's cross-format self-consistency check
# covers only AttrFilter(object_type), so nothing downstream would catch it —
# which is why the refusal has to happen here.
#
# This is the shape of Riga's published atgazene_lod2.gml (703 top-level
# objects, none with a gml:id).
# --------------------------------------------------------------------------
case_citygml_without_gml_ids_is_refused() {
  local name="a CityGML input whose objects lack gml:id is refused"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  # The DEFAULT format set, so this also pins where the refusal happens: it is
  # a property of the input, so it must fire before anything is built. A
  # refusal at the end would leave a .gml sitting in the prepared directory
  # that a later run — or a coordinator pointed at that directory — would
  # happily measure, which is the very row this check exists to prevent.
  run_prepare "$dir" "$dir/data/no_id.gml" "$dir/out"
  if ! expect_guard "$name" "1 of 2 top-level objects carry a gml:id"; then
    return
  fi
  # The reason, not just the count: this refusal only makes sense if it says
  # which measurement it is protecting.
  if ! log_mentions "id-lookup"; then
    fail "$name" "the refusal does not name id-lookup as the reason; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "a refused input still left artefacts behind: $(find "$dir/out" -mindepth 1)"
    return
  fi
  if [[ -f "$dir/repo/lib/cityparquet-rs/cargo-invoked" ]]; then
    fail "$name" "built the CityParquet CLI before refusing an input it cannot use"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7j: a CityGML 1.0 input is refused. citygml-tools converts it happily,
# so the whole chain would go green — but the benchmark's own reader accepts
# only CityGML 2.0 (crates/core/src/source.rs), so the `citygml`
# artefact could never be measured. Preparing an unmeasurable artefact is the
# same failure as preparing an empty one.
# --------------------------------------------------------------------------
case_citygml_1_0_is_refused() {
  local name="a CityGML 1.0 input is refused as unmeasurable"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" "$dir/data/citygml1.gml" "$dir/out"
  if ! expect_guard "$name" "declares CityGML 1.0"; then
    return
  fi
  if ! log_mentions "only CityGML 2.0"; then
    fail "$name" "the refusal does not say which version is required; log: $(cat "$LAST_LOG")"
    return
  fi
  # Same fail-fast requirement as the gml:id case above.
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "a refused input still left artefacts behind: $(find "$dir/out" -mindepth 1)"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7k: an opening tag whose attributes continue on the next line is still
# counted. A line-oriented count returns 0 for it and hard-fails a perfectly
# valid document.
# --------------------------------------------------------------------------
case_multiline_member_tags_are_counted() {
  local name="member tags split across lines are counted"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" --formats citygml "$dir/data/multiline.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "citygml: 2 top-level object(s)"; then
    fail "$name" "wrong count for split tags; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7l: `jq` is guarded like every other external tool it stands beside.
# --------------------------------------------------------------------------
case_cityjson_without_jq() {
  local name="--formats cityjson without jq fails at the guard"
  local dir
  dir="$(new_sandbox cargo fcb cjseq)"
  RUN_PREPARE_PATH="$(jq_free_bin "$dir")" \
    run_prepare "$dir" --formats cityjson "$dir/data/tiny.city.jsonl" "$dir/out"
  if ! expect_guard "$name" "error: jq not found"; then
    return
  fi
  if [[ -n "$(find "$dir/out" -mindepth 1 -print -quit)" ]]; then
    fail "$name" "a run that failed its guard still built something"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7m: a converter that dies midway leaves no truncated artefact behind —
# not even the temporary it was writing. A leftover `.tmp` is litter; a
# leftover half-written artefact would be measured.
# --------------------------------------------------------------------------
case_a_dying_converter_leaves_no_debris() {
  local name="a converter that dies midway leaves no debris"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  STUB_CJSEQ_FAIL=cat \
    run_prepare "$dir" --formats cityjsonseq "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "a dying converter did not fail the run; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ -e "$dir/out/tiny.city.jsonl" ]]; then
    fail "$name" "left a truncated CityJSONSeq artefact behind"
    return
  fi
  local debris
  # The parentheses are load-bearing: `find A -o B -print` prints only for B,
  # so an unbracketed version silently never looks for the *.tmp at all.
  debris="$(find "$dir/out" \( -name '*.tmp' -o -name '.cityjson.*' \) -print)"
  if [[ -n "$debris" ]]; then
    fail "$name" "left debris behind: $debris"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7n: an intermediate the chain had to build but nobody asked for is
# reported. `--formats cityparquet` on a .gml cannot avoid writing a CityJSON
# on the way to the CityJSONSeq; leaving it unmentioned makes the prepared
# directory look like it holds an artefact that was measured.
# --------------------------------------------------------------------------
case_chain_intermediates_are_reported() {
  local name="an unrequested chain intermediate is reported as one"
  local dir intermediate
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  for intermediate in tiny.city.json tiny.city.jsonl; do
    if [[ ! -e "$dir/out/$intermediate" ]]; then
      fail "$name" "the $intermediate intermediate was not built"
      return
    fi
  done
  # The WHOLE line, and both intermediates: `--formats cityparquet` on a .gml
  # has to write a CityJSON *and* a CityJSONSeq on the way to the package, and
  # each of them sits in the prepared directory looking like a measured
  # artefact until this line says otherwise. (A substring match on the
  # `.city.json` name also matched a line naming only `.city.jsonl` — a
  # prefix, so the assertion held whichever artefact was reported.)
  if ! log_has_line "chain intermediates (not requested): $dir/out/tiny.city.json, $dir/out/tiny.city.jsonl"; then
    fail "$name" "the intermediates are not reported as such; log: $(cat "$LAST_LOG")"
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
# Case 8b: a whole-document CityJSON input builds a REAL CityJSONSeq artefact,
# cut from the CityJSON stage — and everything downstream is fed that seq.
#
# This is the input shape where the old "cityjsonseq is the input" rule was
# quietest: six of the corpus's datasets are `.city.json`, no `.city.jsonl`
# was ever built for them, and so `cityjson` and `cityjsonseq` were two rows
# over ONE file. Their counts agreed, the self-consistency check said OK, and
# the CSV published a CityJSONSeq column that no CityJSONSeq was ever read
# for.
# --------------------------------------------------------------------------
case_cityjson_input_builds_a_real_seq_artefact() {
  local name="a CityJSON input cuts a real CityJSONSeq artefact for the seq row"
  local dir
  # citygml-tools is in the sandbox because this runs the DEFAULT format set,
  # which now includes `citygml` and therefore the from-cityjson hop. The case
  # is about the seq artefact, not about CityGML; without the stub it would
  # fail at the tool guard before reaching what it means to assert.
  dir="$(new_sandbox cargo fcb cjseq citygml-tools)"
  run_prepare "$dir" "$dir/data/tiny.city.json" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ ! -s "$dir/out/tiny.city.jsonl" ]]; then
    fail "$name" "no CityJSONSeq artefact was built; log: $(cat "$LAST_LOG")"
    return
  fi
  # Cut from the CityJSON artefact, asserted from what the stub RECORDED —
  # never from the script's own echo of a path.
  if ! grep -qF "\"stub_source\":\"$dir/out/tiny.city.json\"" "$dir/out/tiny.city.jsonl"; then
    fail "$name" "the seq records source '$(head -1 "$dir/out/tiny.city.jsonl")', not the CityJSON artefact"
    return
  fi
  # …and the seq is what FlatCityBuf and both packages were fed, so the two
  # halves of every comparison read the same bytes.
  local fed
  for fed in "$dir/out/tiny.fcb" "$dir/out/tiny.parquet/stub-source.txt" \
    "$dir/out/tiny-hilbert.parquet/stub-source.txt"; do
    if [[ "$(cat "$fed")" != "$dir/out/tiny.city.jsonl" ]]; then
      fail "$name" "$fed records input '$(cat "$fed")', not the derived CityJSONSeq"
      return
    fi
  done
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 8c: the flags that DEFINE what an artefact is are actually passed.
#
# Each of these is one word in one line of the prepare script, and dropping
# any of them leaves an artefact that is still non-empty, still counts
# correctly, and still passes every other case here — while silently changing
# what the benchmark measures:
#
#   --ordering hilbert  without it the "Hilbert" package is byte-identical to
#                       the source-order one, and `just ordering-bench`
#                       publishes "ordering makes no difference" — a null
#                       result that reads as a finding, on one of this
#                       branch's two deliverables.
#   fcb ser -A          without it there is no B+-tree attribute index, so
#                       FlatCityBuf falls back to a full scan on
#                       attr-filter/id-lookup and the row is published as an
#                       indexed query.
#   gzip -9             a different level is a different compression baseline
#                       in the size chart.
#
# Asserted from the stubs' own recorded argv, not from the script's echo.
# --------------------------------------------------------------------------
case_measurement_flags_are_passed() {
  local name="--ordering hilbert, fcb -A and gzip -9 all reach the tools"
  local dir
  dir="$(new_sandbox cargo fcb citygml-tools cjseq gzip)"
  run_prepare "$dir" "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  # One line per argument, so `grep -qFx` matches a whole argument and never
  # a fragment of a path.
  if ! grep -qFx -- "--ordering" "$dir/out/tiny-hilbert.parquet/stub-argv.txt" \
    || ! grep -qFx -- "hilbert" "$dir/out/tiny-hilbert.parquet/stub-argv.txt"; then
    fail "$name" "the Hilbert package was not written with --ordering hilbert: $(
      tr '\n' ' ' <"$dir/out/tiny-hilbert.parquet/stub-argv.txt"
    )"
    return
  fi
  # …and its source-order twin must NOT carry it, or the two rows are the
  # same package twice and the comparison is vacuous in the other direction.
  if grep -qFx -- "--ordering" "$dir/out/tiny.parquet/stub-argv.txt"; then
    fail "$name" "the source-order package was written with --ordering: $(
      tr '\n' ' ' <"$dir/out/tiny.parquet/stub-argv.txt"
    )"
    return
  fi
  if ! grep -qFx -- "-A" "$dir/fcb-ser-argv.txt"; then
    fail "$name" "fcb ser was not given -A (no attribute index): $(
      tr '\n' ' ' <"$dir/fcb-ser-argv.txt"
    )"
    return
  fi
  if ! grep -qFx -- "-9" "$dir/gzip-argv.txt"; then
    fail "$name" "the gz baseline was not gzip -9: $(tr '\n' ' ' <"$dir/gzip-argv.txt")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 8d: the `fcb info` verification block actually verifies something.
#
# The block reads a feature count out of `fcb info` and refuses a count that
# is missing or <= 0 — but nothing asserted the number it read, so the whole
# block was inert: hardcoding `FCB_INFO="Features: 1"` left the suite green.
# It also used `grep -E '^\s*Features:'`, and `\s` is a GNU extension POSIX
# ERE does not define, so on BSD/macOS grep — the measurement machine — it
# matched nothing and every FlatCityBuf prepare died at the guard.
# --------------------------------------------------------------------------
case_fcb_info_count_is_reported() {
  local name="the fcb info feature count is read from fcb and reported"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats flatcitybuf "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  # The stub's `fcb info` reports 3 features; the whole line, so a count read
  # from anywhere but `fcb info` fails here.
  if ! log_has_line "  fcb info: 3 features in $dir/out/tiny.fcb"; then
    fail "$name" "the fcb info count was not read/reported; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 9b: the ARTEFACT NAMES must equal `Format::artefact`'s.
#
# The coordinator resolves `<prepared-dir>/<name>` per format
# (benchmark/readbench/src/format.rs); this script writes those
# files. Both sources CLAIM the contract in their own comments, and nothing
# checked it — which is exactly how `cityjsonseq` came to have no artefact on
# one side and a `<base>.city.jsonl` on the other, with the coordinator
# quietly measuring `--input` instead.
#
# Two halves, textual and executable: the two name lists must agree, and a
# real default-set run must produce exactly that set of files.
# --------------------------------------------------------------------------
case_artefact_names_match_the_rust_enum() {
  local name="the script's artefact names match Format::artefact"
  local rust_names script_names dir produced
  # `format!("{base}.city.jsonl")` -> `.city.jsonl`
  rust_names="$(sed -n '/pub fn artefact/,/^    }$/p' "$FORMAT_RS" \
    | grep -oE 'format!\("\{base\}[^"]*"\)' \
    | sed 's/.*{base}\(.*\)")/\1/' \
    | sort | tr '\n' ' ')"
  # `FOO_OUT="$OUTDIR/${BASE}.city.jsonl"` -> `.city.jsonl`
  script_names="$(grep -oE '^[A-Z_]+_OUT="\$OUTDIR/\$\{BASE\}[^"]*"' "$PREPARE" \
    | sed 's/.*{BASE}\(.*\)"/\1/' \
    | sort | tr '\n' ' ')"
  rust_names="$(echo "$rust_names" | xargs)"
  script_names="$(echo "$script_names" | xargs)"
  if [[ -z "$rust_names" ]]; then
    fail "$name" "could not read the artefact names out of $FORMAT_RS"
    return
  fi
  if [[ -z "$script_names" ]]; then
    fail "$name" "could not read the *_OUT names out of $PREPARE"
    return
  fi
  if [[ "$rust_names" != "$script_names" ]]; then
    fail "$name" "rust: [$rust_names] != script: [$script_names]"
    return
  fi
  # The executable half: a default-set run on a CityGML input builds every
  # artefact this script knows how to, so OUTDIR must hold exactly the names
  # the Rust side resolves — no more (an artefact nothing measures) and no
  # fewer (a format that silently falls back to something else).
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  produced="$(find "$dir/out" -mindepth 1 -maxdepth 1 ! -name '.readbench-chain' \
    -exec basename {} \; \
    | sed 's/^tiny//' | sort | tr '\n' ' ')"
  produced="$(echo "$produced" | xargs)"
  if [[ "$produced" != "$rust_names" ]]; then
    fail "$name" "a default run produced [$produced], Format::artefact resolves [$rust_names]"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 8e: artefacts built by an OLDER derivation chain must not be reused.
#
# The prepare script skips an artefact that already exists and passes its
# validity check, and `benchmark/formats/data/readbench/` persists across runs — so a
# directory prepared before the chain changed keeps serving artefacts derived
# from a stage that no longer exists, and nothing says so. That is C1's bug
# class one level up: a pre-fix `<base>.jsonl.gz` is a gzip of the WHOLE
# CityJSON document, which the gz runner reads quite happily (measured:
# 0.254909 s / 61,192,614 B against the real seq-gz's 0.092799 s / 1,798,710 B
# — 2.75x too slow, 34x too heavy, and the same whole-document parse C1's own
# "before" figure was).
#
# A sentence in the docs cannot fix this: it is missed by exactly the person
# who most needs it, and the failure publishes plausible-looking numbers. So
# the prepared directory carries a per-dataset chain-version stamp, and a
# stale (or absent) one is a refusal, not a warning.
# --------------------------------------------------------------------------
case_stale_chain_artefacts_are_refused() {
  local name="artefacts from an older derivation chain are refused, not reused"
  local dir before
  dir="$(new_sandbox cargo)"
  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "first run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ ! -f "$dir/out/.readbench-chain/tiny" ]]; then
    fail "$name" "the first run left no chain-version stamp"
    return
  fi
  # Age the stamp: exactly what a directory prepared by an older version of
  # this script looks like.
  printf '1\n' >"$dir/out/.readbench-chain/tiny"
  before="$(cat "$dir/out/tiny.parquet/stub-source.txt")"

  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if ! expect_guard "$name" "built by an older derivation chain"; then
    return
  fi
  # The refusal must say WHAT to do about it, naming the artefact.
  if ! log_mentions "$dir/out/tiny.parquet"; then
    fail "$name" "the refusal does not name the stale artefact; log: $(cat "$LAST_LOG")"
    return
  fi
  # …and must not have quietly rebuilt or removed anything on its way out: a
  # refusal that mutates the directory is a refusal nobody can inspect.
  if [[ "$(cat "$dir/out/tiny.parquet/stub-source.txt")" != "$before" ]]; then
    fail "$name" "the refused run rebuilt the stale artefact"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 8f: an UNSTAMPED directory holding artefacts is the same case.
#
# This is the shape every prepared directory on disk today has — they were
# built before the stamp existed. "No stamp" must therefore mean "older
# chain", never "fresh directory": treating an unknown provenance as current
# would let exactly the directories this guard exists for through.
# --------------------------------------------------------------------------
case_unstamped_artefacts_are_refused() {
  local name="artefacts with no chain-version stamp at all are refused"
  local dir
  dir="$(new_sandbox cargo)"
  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "first run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  rm -rf "$dir/out/.readbench-chain"

  run_prepare "$dir" --formats cityparquet "$dir/data/tiny.city.jsonl" "$dir/out"
  if ! expect_guard "$name" "built by an older derivation chain"; then
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 8g: the guard must not fire on a directory this chain built.
#
# The other half of the pair — a guard that refused a current directory would
# simply be a broken script, and idempotency (case 8) is a documented
# property.
# --------------------------------------------------------------------------
case_current_chain_artefacts_are_reused() {
  local name="a directory built by this chain is still reused"
  local dir stamp
  dir="$(new_sandbox cargo fcb citygml-tools cjseq)"
  run_prepare "$dir" "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "first run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  # The stamp is the script's own current version, read out of the script
  # rather than retyped, so bumping the version does not silently pass here.
  stamp="$(sed -n 's/^CHAIN_VERSION=\([0-9]*\)$/\1/p' "$PREPARE")"
  if [[ -z "$stamp" ]]; then
    fail "$name" "could not read CHAIN_VERSION out of $PREPARE"
    return
  fi
  if [[ "$(cat "$dir/out/.readbench-chain/tiny")" != "$stamp" ]]; then
    fail "$name" "stamp is '$(cat "$dir/out/.readbench-chain/tiny")', CHAIN_VERSION is '$stamp'"
    return
  fi
  run_prepare "$dir" "$dir/data/tiny.gml" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "second run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "skip $dir/out/tiny.parquet (already present)"; then
    fail "$name" "a current directory was not reused; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 9: the script's format vocabulary must equal the Rust one.
#
# `Format::ALL` (benchmark/readbench/src/format.rs) owns the names;
# the shell script cannot import it, so it carries a copy — and a copy of a
# vocabulary is exactly how this benchmark's CSV header contract drifted into
# three incompatible versions. This case reads BOTH lists out of their own
# source files and compares them, the same trick
# `benchmark/plot/tests/test_csv_contract.py` uses for the CSV header.
#
# `duckdb-parquet` is deliberately excluded: it is an SQL-engine baseline
# driven by `benchmark/scripts/readbench_duckdb.sh` over an already-prepared
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
case_default_on_cityjsonseq_builds_every_format
case_cityjsonseq_is_materialised_from_a_seq_input
case_citygml_input_builds_the_whole_chain
case_explicit_citygml_from_cityjson_is_synthesised
case_synthesised_citygml_without_ids_is_refused
case_synthesised_citygml_wrong_version_is_refused
case_citygml_input_without_citygml_tools
case_cityjson_without_cjseq
case_object_less_citygml_is_rejected
case_object_less_cityjson_is_rejected
case_feature_less_cityjsonseq_is_rejected
case_conversion_loss_is_reported
case_citygml_without_gml_ids_is_refused
case_citygml_1_0_is_refused
case_multiline_member_tags_are_counted
case_cityjson_without_jq
case_a_dying_converter_leaves_no_debris
case_chain_intermediates_are_reported
case_second_run_skips
case_cityjson_input_builds_a_real_seq_artefact
case_measurement_flags_are_passed
case_fcb_info_count_is_reported
case_stale_chain_artefacts_are_refused
case_unstamped_artefacts_are_refused
case_current_chain_artefacts_are_reused
case_vocabulary_matches_the_rust_enum
case_artefact_names_match_the_rust_enum

echo "readbench_prepare_test: $PASSED passed, $FAILED failed"
[[ $FAILED -eq 0 ]]
