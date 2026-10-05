#!/usr/bin/env bash
# Tests for the justfile's bloom-axis recipes: the variant lists, scenarios,
# probes and positional arguments `bloom-bench` and `bloom-bench-http` hand
# to `variant-bench`. The variant list each passes IS the benchmark, so it is
# read out of the recipe rather than trusted from a comment.
#
# Plain bash, no framework: `bats` is not a dependency of this repo. One
# `ok`/`not ok` line per case, non-zero exit if any case fails. Same shape as
# `readbench_prepare_test.sh` and `fetch_benchmark_test.sh` beside it.
#
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHMARK_DIR="$(cd "$TEST_DIR/../.." && pwd)"
# The recipes live in the MONOREPO's root justfile — they reach both this
# workspace's harness crate and the corpora under benchmark/, so neither tree
# can own them.
MONO_ROOT="$(cd "$BENCHMARK_DIR/.." && pwd)"
JUSTFILE="$MONO_ROOT/justfile"

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

# --------------------------------------------------------------------------
# Cases 1-2: the two bloom-axis recipes pass exactly their lists.
#
# `bloom-bench` and `bloom-bench-http` are one-line delegations to
# `variant-bench`, and the variant list each passes IS the benchmark: read
# them out of the recipe rather than trusting a comment.
# --------------------------------------------------------------------------
recipe_variants() {
  # The quoted variant list a delegating recipe passes to `variant-bench`.
  sed -n "/^$1 /,/^$/p" "$JUSTFILE" \
    | sed -n 's/.*just variant-bench "{{FOLDER}}" "{{OUT}}" "\([^"]*\)".*/\1/p'
}

case_bloom_bench_list() {
  local name="bloom-bench passes the one bloom pair and the lookup scenarios"
  local expected="cityparquet,cityparquet+nobloom"
  local actual line
  actual="$(recipe_variants bloom-bench)"
  if [[ "$actual" != "$expected" ]]; then
    fail "$name" "bloom-bench passes '$actual'"
    return
  fi
  line="$(sed -n '/^bloom-bench /,/^$/p' "$JUSTFILE" | grep 'just variant-bench')"
  if [[ "$line" != *'"id-lookup,feature-lookup" "id-50pct,id-miss" "feature-50pct,feature-miss"'* ]]; then
    fail "$name" "bloom-bench does not pass the lookup scenarios and probes: $line"
    return
  fi
  pass "$name"
}

case_bloom_http_matches_the_local_pair() {
  local name="bloom-bench-http reads the same pair, scenarios and probes as bloom-bench, over HTTP"
  local actual line
  actual="$(recipe_variants bloom-bench-http)"
  if [[ "$actual" != "cityparquet,cityparquet+nobloom" ]]; then
    fail "$name" "bloom-bench-http passes '$actual'"
    return
  fi
  line="$(sed -n '/^bloom-bench-http /,/^$/p' "$JUSTFILE" | grep 'just variant-bench')"
  if [[ "$line" != *'"id-lookup,feature-lookup" "id-50pct,id-miss" "feature-50pct,feature-miss" "{{BASE_URL}}"'* ]]; then
    fail "$name" "bloom-bench-http does not pass the lookups and its BASE_URL: $line"
    return
  fi
  # The recipe body has blank lines of its own: range to the next unindented
  # line, which is where the recipe ends.
  if ! sed -n '/^variant-bench /,/^[^[:space:]]/p' "$JUSTFILE" | grep -q -- '--transport http --base-url "{{BASE_URL}}"'; then
    fail "$name" "variant-bench does not turn BASE_URL into --transport http"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 3: every positional argument lands on the parameter it is meant for.
#
# `bloom-bench` and `bloom-bench-http` call `variant-bench` POSITIONALLY, so
# removing or reordering one of its parameters silently shifts every
# argument after it. Map each passed argument onto `variant-bench`'s own
# parameter list and check the ones that carry meaning.
# --------------------------------------------------------------------------
variant_bench_params() {
  # Parameter names in order: defaults (parenthesised or quoted) stripped first.
  grep -E '^variant-bench ' "$JUSTFILE" \
    | sed -e 's/([^)]*)//g' -e "s/'[^']*'//g" -e 's/^variant-bench //' -e 's/:$//' \
    | grep -oE '[A-Z_]+'
}

recipe_arguments() {
  sed -n "/^$1 /,/^$/p" "$JUSTFILE" | grep 'just variant-bench' | grep -oE '"[^"]*"' | tr -d '"'
}

argument_for() {
  # argument_for RECIPE PARAM: what RECIPE passes in PARAM's position.
  local recipe="$1" param="$2" index=-1 i=0 line
  while IFS= read -r line; do
    if [[ "$line" == "$param" ]]; then index=$i; fi
    i=$((i + 1))
  done < <(variant_bench_params)
  [[ $index -ge 0 ]] || return 0
  i=0
  while IFS= read -r line; do
    if [[ $i -eq $index ]]; then
      printf '%s' "$line"
      return
    fi
    i=$((i + 1))
  done < <(recipe_arguments "$recipe")
}

case_positional_arguments_line_up() {
  local name="bloom-bench and bloom-bench-http pass each argument in its own parameter's position"
  local params
  params="$(variant_bench_params | tr '\n' ' ')"
  if [[ "$params" != "FOLDER OUT VARIANTS PREPARED REPEAT SCENARIOS ID_PROBES FEATURE_PROBES BASE_URL " ]]; then
    fail "$name" "variant-bench's parameters changed: $params"
    return
  fi
  local recipe
  for recipe in bloom-bench bloom-bench-http; do
    if [[ "$(argument_for "$recipe" REPEAT)" != "{{REPEAT}}" \
      || "$(argument_for "$recipe" SCENARIOS)" != "id-lookup,feature-lookup" \
      || "$(argument_for "$recipe" ID_PROBES)" != "id-50pct,id-miss" \
      || "$(argument_for "$recipe" FEATURE_PROBES)" != "feature-50pct,feature-miss" ]]; then
      fail "$name" "$recipe passes its arguments out of position: $(recipe_arguments "$recipe" | tr '\n' ' ')"
      return
    fi
  done
  if [[ "$(argument_for bloom-bench-http BASE_URL)" != "{{BASE_URL}}" ]]; then
    fail "$name" "bloom-bench-http does not pass BASE_URL in BASE_URL's position"
    return
  fi
  if [[ -n "$(argument_for bloom-bench BASE_URL)" ]]; then
    fail "$name" "bloom-bench passes a BASE_URL; the local run must not go over HTTP"
    return
  fi
  pass "$name"
}

case_bloom_bench_list
case_bloom_http_matches_the_local_pair
case_positional_arguments_line_up

echo "bench_recipe_test: $PASSED passed, $FAILED failed"
[[ "$FAILED" -eq 0 ]]
