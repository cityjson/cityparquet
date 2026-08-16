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
#   $SANDBOX/bin/{cargo,fcb}                     stubs, prepended to PATH
#   $SANDBOX/data/tiny.city.jsonl                a stand-in input
#   $SANDBOX/out                                 OUTDIR
#
# The copy is what lets the `cargo` stub plant a fake `target/release/
# cityparquet` without touching the real one, so no case needs a real
# compile, a real `fcb`, or a real conversion — only the selection and guard
# logic is under test.
#
# PATH is built from a copy of the caller's PATH with every entry that
# carries a real `fcb` removed, so "fcb is not installed" is reproducible on
# a machine where it IS installed. "cargo was not needed" is asserted
# differently — via a marker file the stub drops — because removing cargo
# from PATH would also remove much else.
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

# PATH with every directory holding a real `fcb` dropped.
fcb_free_path() {
  local out="" dir
  local IFS=:
  for dir in $PATH; do
    if [[ -z "$dir" ]]; then
      continue
    fi
    if [[ -x "$dir/fcb" ]]; then
      continue
    fi
    out+="${out:+:}$dir"
  done
  printf '%s' "$out"
}

BASE_PATH="$(fcb_free_path)"

# new_sandbox [stub ...] -> prints the sandbox directory.
# Named stubs ("cargo", "fcb") are installed into $SANDBOX/bin; anything not
# named is simply absent from PATH.
new_sandbox() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/readbench_prepare_test.XXXXXX")"
  mkdir -p "$dir/repo/scripts" "$dir/bin" "$dir/data" "$dir/out"
  cp "$PREPARE" "$dir/repo/scripts/readbench_prepare.sh"
  # Content is irrelevant: no stub ever parses it.
  printf '{"type":"CityJSONFeature"}\n' >"$dir/data/tiny.city.jsonl"

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
out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    *) shift ;;
  esac
done
[[ -n "$out" ]] || { echo "stub cityparquet: no -o given" >&2; exit 1; }
mkdir -p "$out"
echo stub >"$out/building.parquet"
CITYPARQUET_STUB
chmod +x target/release/cityparquet
CARGO_STUB
        chmod +x "$dir/bin/cargo"
        ;;
      fcb)
        cat >"$dir/bin/fcb" <<'FCB_STUB'
#!/usr/bin/env bash
set -euo pipefail
sub=${1:-}
shift || true
out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    *) shift ;;
  esac
done
case "$sub" in
  ser) echo stub >"$out" ;;
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
  PATH="$dir/bin:$BASE_PATH" "$dir/repo/scripts/readbench_prepare.sh" "$@" \
    >"$LAST_LOG" 2>&1
  LAST_RC=$?
  set -e
}

log_mentions() {
  grep -qF -- "$1" "$LAST_LOG"
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
  local name="--formats flatcitybuf without fcb fails, naming fcb"
  local dir
  dir="$(new_sandbox cargo)"
  run_prepare "$dir" --formats flatcitybuf "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "expected a non-zero exit, got 0"
    return
  fi
  if ! log_mentions "fcb"; then
    fail "$name" "message does not name fcb; log: $(cat "$LAST_LOG")"
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
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "expected a non-zero exit, got 0"
    return
  fi
  if ! log_mentions "bogus"; then
    fail "$name" "message does not name the offending format; log: $(cat "$LAST_LOG")"
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
# Case 5: no `--formats` builds every artefact this script can currently
# produce — the four that predate the flag.
# --------------------------------------------------------------------------
case_default_builds_the_existing_four() {
  local name="no --formats builds the four artefacts the script can produce"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  local artefact
  for artefact in tiny.parquet tiny-hilbert.parquet tiny.fcb tiny.jsonl.gz; do
    if [[ ! -e "$dir/out/$artefact" ]]; then
      fail "$name" "missing $artefact"
      return
    fi
  done
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
# Case 7: a format the script has no build step for yet fails loudly rather
# than being quietly dropped from the request.
#
# Task 7 teaches the script to build `citygml`/`cityjson`; when it does, this
# case is the one to replace with a positive build assertion.
# --------------------------------------------------------------------------
case_unimplemented_format_fails_loudly() {
  local name="a valid format with no build step yet fails loudly"
  local dir
  dir="$(new_sandbox cargo fcb)"
  run_prepare "$dir" --formats citygml "$dir/data/tiny.city.jsonl" "$dir/out"
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "expected a non-zero exit, got 0"
    return
  fi
  if ! log_mentions "citygml"; then
    fail "$name" "message does not name citygml; log: $(cat "$LAST_LOG")"
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
  if ! log_mentions "skip"; then
    fail "$name" "second run did not skip; log: $(cat "$LAST_LOG")"
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
case_default_builds_the_existing_four
case_cityjsonseq_is_a_no_op
case_unimplemented_format_fails_loudly
case_second_run_skips
case_vocabulary_matches_the_rust_enum

echo "readbench_prepare_test: $PASSED passed, $FAILED failed"
[[ $FAILED -eq 0 ]]
