#!/usr/bin/env bash
# Run every automated suite in the tree, in dependency order, and report one
# line per stage.
#
# This is the automatable part of TESTING.md. It is NOT a replacement for it:
# TESTING.md's value is the manual cross-module walkthrough — the expected
# outputs, the interop chains, the known issues — and none of that is checked
# here. What this does is answer "is anything obviously broken?" in one command.
#
# STAGES ARE SKIPPED, NOT FAILED, when their toolchain is absent. A machine with
# no `uv` is not a machine with a broken plotting suite, and a DuckDB extension
# that has never been built is not a failing extension. Every skip is printed
# with its reason and the run's exit code ignores it; only a suite that ran and
# failed makes this script fail.
#
#   ./test/run-all.sh              every stage the machine can run
#   ./test/run-all.sh --list       what would run, and why anything is skipped
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST_ONLY=0
[[ "${1:-}" == "--list" ]] && LIST_ONLY=1

PASSED=(); FAILED=(); SKIPPED=()

have() { command -v "$1" >/dev/null 2>&1; }

# stage NAME REASON_IF_SKIPPED -- command...
#
# REASON_IF_SKIPPED is empty when the stage can run.
stage() {
    local name="$1" skip="$2"; shift 2
    [[ "$1" == "--" ]] && shift

    if [[ -n "$skip" ]]; then
        SKIPPED+=("$name — $skip")
        printf '\033[33mSKIP\033[0m %s (%s)\n' "$name" "$skip"
        return
    fi
    if [[ "$LIST_ONLY" -eq 1 ]]; then
        printf '\033[36mWOULD RUN\033[0m %s\n' "$name"
        return
    fi

    printf '\033[1m>>>> %s\033[0m\n' "$name"
    if "$@"; then
        PASSED+=("$name")
        printf '\033[32mPASS\033[0m %s\n\n' "$name"
    else
        FAILED+=("$name")
        printf '\033[31mFAIL\033[0m %s\n\n' "$name"
    fi
}

# --- 1. the Rust workspace -------------------------------------------------
#
# `just check` gates on the vendored city3d-stac-tool being checked out, and the
# tests read real fixtures rather than inline CityJSON, so both are
# preconditions rather than part of the suite.
rs_skip=""
have cargo || rs_skip="cargo not on PATH"
have just  || rs_skip="just not on PATH"
[[ -z "$rs_skip" && ! -f lib/cityparquet-rs/vendor/city3d-stac-tool/Cargo.toml ]] \
    && rs_skip="vendor/city3d-stac-tool not checked out; run 'just setup'"
[[ -z "$rs_skip" && ! -f lib/cityparquet-rs/tests/fixtures/delft.city.jsonl ]] \
    && rs_skip="fixtures absent; run 'just fixtures' in lib/cityparquet-rs"

stage "cityparquet-rs: just check" "$rs_skip" -- \
    bash -c 'cd lib/cityparquet-rs && just check'

stage "cityparquet-rs: just interop" \
    "${rs_skip:-$(have duckdb || echo 'duckdb not on PATH')}" -- \
    bash -c 'cd lib/cityparquet-rs && just interop'

# --- 2. the benchmark harness's own suites ---------------------------------
#
# Neither needs a corpus or a network: the shell tests stub every external
# binary and serve file:// URLs to the real fetcher; the pytest suite ships its
# own CSV fixtures.
stage "benchmark: just plot-test" \
    "$(have uv || echo 'uv not on PATH')" -- just plot-test

stage "benchmark: just scripts-test" \
    "$(have jq || echo 'jq not on PATH')" -- just scripts-test

stage "benchmark/databases: pytest (unit)" \
    "$(have uv || echo 'uv not on PATH')" -- \
    bash -c 'cd benchmark/databases && uv run --quiet pytest -q -m "not integration"'

# --- 3. the catalogue driver -----------------------------------------------
stage "cityparquet-rs: just catalog-test" \
    "$(have uv || echo 'uv not on PATH')" -- \
    bash -c 'cd lib/cityparquet-rs && just catalog-test'

# --- 4. the DuckDB extensions ----------------------------------------------
#
# Built, not building: a from-source extension build compiles DuckDB itself and
# takes tens of minutes, which is not something a test runner should start
# behind your back. Build them once (`just rebuild` / `GEN=ninja make`) and this
# picks them up.
cj_skip=""
[[ -d lib/duckdb-cityjson/.git || -f lib/duckdb-cityjson/justfile ]] \
    || cj_skip="submodule not checked out; run 'just setup'"
[[ -z "$cj_skip" && ! -x lib/duckdb-cityjson/build/release/test/unittest ]] \
    && cj_skip="not built; run 'just rebuild' in lib/duckdb-cityjson"

stage "duckdb-cityjson: unittest" "$cj_skip" -- \
    bash -c 'cd lib/duckdb-cityjson && ./build/release/test/unittest "test/sql/*"'

d3_skip=""
[[ -d lib/duckdb-3d/.git || -f lib/duckdb-3d/Makefile ]] \
    || d3_skip="submodule not checked out; run 'just setup'"
[[ -z "$d3_skip" && ! -x lib/duckdb-3d/build/release/test/unittest ]] \
    && d3_skip="not built; run 'GEN=ninja make' in lib/duckdb-3d"

stage "duckdb-3d: make test" "$d3_skip" -- \
    bash -c 'cd lib/duckdb-3d && make test'

# --- 5. citylake -----------------------------------------------------------
#
# Work in progress: it builds, and that is the whole assertion for now.
stage "citylake: cargo build" \
    "$(have cargo || echo 'cargo not on PATH')" -- \
    bash -c 'cd lib/citylake && cargo build'

# --- summary ---------------------------------------------------------------
if [[ "$LIST_ONLY" -eq 1 ]]; then
    exit 0
fi

echo "================================================================"
printf 'passed  %d\n' "${#PASSED[@]}"
printf 'failed  %d\n' "${#FAILED[@]}"
printf 'skipped %d\n' "${#SKIPPED[@]}"

if [[ ${#SKIPPED[@]} -gt 0 ]]; then
    echo
    echo "Skipped — each needs something this machine does not have:"
    printf '  %s\n' "${SKIPPED[@]}"
fi

if [[ ${#FAILED[@]} -gt 0 ]]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    echo
    echo "See test/TESTING.md for what each stage proves and what a real"
    echo "failure looks like against the expected outputs recorded there."
    exit 1
fi

echo
echo "Everything that could run, ran."
