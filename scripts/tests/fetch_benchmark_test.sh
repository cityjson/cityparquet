#!/usr/bin/env bash
# Tests for `scripts/fetch_benchmark.sh` — the catalogue corpus fetcher.
#
# Plain bash, no framework: `bats` is not a dependency of this repo. One
# `ok`/`not ok` line per case, non-zero exit if any case fails. Same shape as
# `readbench_prepare_test.sh` beside it.
#
# NO NETWORK. Every case builds a throwaway origin of local files and a
# manifest of `file://` URLs pointing at them, handed to the script through
# `$CORPUS_MANIFEST` (the same override a user gets for fetching a private
# subset with the script's verified-fetch machinery). `curl` reads `file://`
# happily, so the fetch path under test is the real one — no stub, no
# recorded-invocation theatre.
#
# The last case is different in kind: it lints the script's OWN pinned table
# and cross-checks it against `bench/catalogue_benchmark_urls.txt`. Those two
# files are a corpus definition kept in two places, which is exactly how the
# five PLATEAU modules Task 4 blocked stayed "usable" in one of them for a
# while. The lint makes the drift a test failure.
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TEST_DIR/../.." && pwd)"
FETCH="$REPO_ROOT/scripts/fetch_benchmark.sh"
CATALOGUE="$REPO_ROOT/bench/catalogue_benchmark_urls.txt"

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

size_of() {
  stat -f%z "$1" 2>/dev/null || stat -c%s "$1"
}

# new_sandbox -> prints a directory holding
#   origin/   the files a manifest's file:// URLs point at
#   dest/     DEST for the script
#
# Three origin files, one per normalisation form the corpus needs:
#   plain.city.json   a plain document, copied through untouched
#   packed.json.gz    a gzip whose plain form is `plain.city.json`'s content
#   bundle.zip        a zip holding TWO members, only one of which is wanted
# The zip deliberately has a second member: the corpus's real archives ship a
# GML beside thousands of texture images, so "extract the one pinned member"
# has to be the behaviour, not "extract the archive".
new_sandbox() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/fetch_benchmark_test.XXXXXX")"
  mkdir -p "$dir/origin" "$dir/dest"
  printf '{"type":"CityJSON","version":"2.0","CityObjects":{},"vertices":[]}\n' \
    >"$dir/origin/plain.city.json"
  gzip -cn "$dir/origin/plain.city.json" >"$dir/origin/packed.json.gz"
  printf '<CityModel/>\n' >"$dir/origin/wanted.gml"
  printf 'not the city model\n' >"$dir/origin/decoy.txt"
  (cd "$dir/origin" && zip -q bundle.zip wanted.gml decoy.txt)
  printf '%s' "$dir"
}

# manifest_line NAME PATH FORM SETS -> a manifest line whose pinned byte size
# is PATH's actual size, so a case that wants a MISMATCH has to create it
# deliberately rather than inherit one from a typo.
manifest_line() {
  local name=$1 path=$2 form=$3 sets=$4
  printf '%s|%s|%s|%s|file://%s\n' "$name" "$(size_of "$path")" "$form" "$sets" "$path"
}

# run_fetch SANDBOX [args ...] -> sets LAST_RC and LAST_LOG.
LAST_RC=0
LAST_LOG=""
run_fetch() {
  local dir=$1
  shift
  LAST_LOG="$dir/run.log"
  set +e
  CORPUS_MANIFEST="$dir/manifest.txt" "$FETCH" "$@" >"$LAST_LOG" 2>&1
  LAST_RC=$?
  set -e
}

log_mentions() {
  grep -qF -- "$1" "$LAST_LOG"
}

# --------------------------------------------------------------------------
# Case 1: the three forms each land on disk as a plain, ready-to-read file.
# A `.gz` that stayed gzipped, or a `.zip` that stayed an archive, is invisible
# to every recipe downstream (`just bench` finds inputs by extension), so
# normalising on fetch is the whole point of the form column.
# --------------------------------------------------------------------------
case_forms_are_normalised() {
  local name="gz and zip entries are normalised to plain files"
  local dir
  dir="$(new_sandbox)"
  {
    manifest_line plain.city.json "$dir/origin/plain.city.json" plain default
    manifest_line unpacked.city.json "$dir/origin/packed.json.gz" gz default
    manifest_line extracted.gml "$dir/origin/bundle.zip" zip:wanted.gml default
  } >"$dir/manifest.txt"

  run_fetch "$dir" "$dir/dest"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! cmp -s "$dir/dest/plain.city.json" "$dir/origin/plain.city.json"; then
    fail "$name" "the plain entry did not arrive verbatim"
    return
  fi
  if ! cmp -s "$dir/dest/unpacked.city.json" "$dir/origin/plain.city.json"; then
    fail "$name" "the .gz entry was not gunzipped into place"
    return
  fi
  if ! cmp -s "$dir/dest/extracted.gml" "$dir/origin/wanted.gml"; then
    fail "$name" "the .zip entry did not yield its pinned member"
    return
  fi
  # The archive's OTHER member must not have been unpacked beside it: a corpus
  # directory is scanned by extension, and stray files are stray inputs.
  if [[ -e "$dir/dest/decoy.txt" ]]; then
    fail "$name" "the zip's unwanted member was extracted too"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 2: a size that does not match the pinned one is a hard failure, and
# leaves nothing behind. Benchmarking against different bytes than the ones
# the corpus documents is the failure this whole table exists to prevent, and
# a half-written file in DEST would be measured by the very next `just bench`.
# --------------------------------------------------------------------------
case_size_mismatch_hard_fails() {
  local name="a size mismatch hard-fails and leaves nothing behind"
  local dir
  dir="$(new_sandbox)"
  manifest_line plain.city.json "$dir/origin/plain.city.json" plain default \
    >"$dir/manifest.txt"
  # Change the bytes AFTER pinning: the origin now serves something the
  # manifest does not describe, which is exactly the drift-at-source case.
  printf 'a much longer document than the one that was pinned\n' \
    >>"$dir/origin/plain.city.json"

  run_fetch "$dir" "$dir/dest"
  if [[ $LAST_RC -eq 0 ]]; then
    fail "$name" "a changed origin was accepted; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "size mismatch"; then
    fail "$name" "the failure does not say what went wrong; log: $(cat "$LAST_LOG")"
    return
  fi
  local left
  left="$(find "$dir/dest" -mindepth 1 -print)"
  if [[ -n "$left" ]]; then
    fail "$name" "a failed fetch left files behind: $left"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 3: a re-run skips what is already there. Asserted the strong way — the
# origin is DELETED between the runs, so a second run that re-downloads cannot
# quietly succeed.
# --------------------------------------------------------------------------
case_second_run_skips() {
  local name="an already-fetched file is skipped, not re-downloaded"
  local dir
  dir="$(new_sandbox)"
  {
    manifest_line plain.city.json "$dir/origin/plain.city.json" plain default
    manifest_line unpacked.city.json "$dir/origin/packed.json.gz" gz default
  } >"$dir/manifest.txt"

  run_fetch "$dir" "$dir/dest"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "first run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  rm -f "$dir/origin/plain.city.json" "$dir/origin/packed.json.gz"
  run_fetch "$dir" "$dir/dest"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "second run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  local entry
  for entry in plain.city.json unpacked.city.json; do
    if ! log_mentions "skip $entry (already fetched"; then
      fail "$name" "$entry was not skipped; log: $(cat "$LAST_LOG")"
      return
    fi
  done
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 4: skip-if-present is a size check, not an existence check — including
# for a normalised entry, whose on-disk size is NOT the pinned wire size. A
# truncated file must be re-fetched, or the benchmark measures a fraction of a
# dataset and reports it as the whole one.
# --------------------------------------------------------------------------
case_truncated_file_is_refetched() {
  local name="a truncated already-present file is re-fetched"
  local dir
  dir="$(new_sandbox)"
  manifest_line unpacked.city.json "$dir/origin/packed.json.gz" gz default \
    >"$dir/manifest.txt"

  run_fetch "$dir" "$dir/dest"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "first run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  printf 'truncated' >"$dir/dest/unpacked.city.json"
  run_fetch "$dir" "$dir/dest"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "second run: exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "size mismatch on existing file"; then
    fail "$name" "the truncation was not reported; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! cmp -s "$dir/dest/unpacked.city.json" "$dir/origin/plain.city.json"; then
    fail "$name" "the truncated file was not repaired"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 5: `--only` selects by the set an entry can serve. Some corpus entries
# cannot serve the DEFAULT format set (their `citygml` row is unmeasurable)
# but are perfectly good for every other format, so the table records the sets
# per entry and the caller picks one.
# --------------------------------------------------------------------------
case_only_selects_by_set() {
  local name="--only fetches just the entries serving that set"
  local dir
  dir="$(new_sandbox)"
  {
    manifest_line in-default.city.json "$dir/origin/plain.city.json" plain default,no-citygml
    manifest_line no-citygml-only.gml "$dir/origin/wanted.gml" plain no-citygml
  } >"$dir/manifest.txt"

  run_fetch "$dir" --only default "$dir/dest"
  if [[ $LAST_RC -ne 0 ]]; then
    fail "$name" "exit $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ ! -e "$dir/dest/in-default.city.json" ]]; then
    fail "$name" "the default-set entry was not fetched; log: $(cat "$LAST_LOG")"
    return
  fi
  if [[ -e "$dir/dest/no-citygml-only.gml" ]]; then
    fail "$name" "an entry outside the requested set was fetched anyway"
    return
  fi
  # …and the run says so, rather than silently delivering a subset.
  if ! log_mentions "no-citygml-only.gml"; then
    fail "$name" "the skipped entry was not reported; log: $(cat "$LAST_LOG")"
    return
  fi
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 6: an unknown set name is rejected, listing the valid ones — a typo
# must not silently fetch nothing and exit 0.
# --------------------------------------------------------------------------
case_unknown_set_rejected() {
  local name="an unknown --only set is rejected, listing the valid names"
  local dir
  dir="$(new_sandbox)"
  manifest_line plain.city.json "$dir/origin/plain.city.json" plain default \
    >"$dir/manifest.txt"

  run_fetch "$dir" --only bogus "$dir/dest"
  if [[ $LAST_RC -ne 1 ]]; then
    fail "$name" "expected exit 1, got $LAST_RC; log: $(cat "$LAST_LOG")"
    return
  fi
  if ! log_mentions "unknown --only set 'bogus'"; then
    fail "$name" "wrong wording; log: $(cat "$LAST_LOG")"
    return
  fi
  local setname
  for setname in default no-citygml all; do
    if ! log_mentions "$setname"; then
      fail "$name" "message does not list '$setname'; log: $(cat "$LAST_LOG")"
      return
    fi
  done
  pass "$name"
}

# --------------------------------------------------------------------------
# Case 7: the script's own pinned table is well-formed, and it agrees with
# `bench/catalogue_benchmark_urls.txt`.
#
# The catalogue file is the corpus's provenance record (where each URL came
# from, why the blocked ones are blocked); the script's table is what actually
# gets fetched. Two hand-maintained lists of the same URLs drift — so this
# case requires them to be the SAME SET, in both directions. Adding a URL to
# one and not the other is then a test failure, not a silent corpus change.
# --------------------------------------------------------------------------
case_pinned_table_is_well_formed() {
  local name="the pinned table is well-formed and matches the catalogue"
  local table catalogue
  table="$(sed -n '/^CORPUS=(/,/^)/p' "$FETCH" \
    | grep -oE '"[^"]+\|[^"]*"' | tr -d '"')"
  if [[ -z "$table" ]]; then
    fail "$name" "could not read the CORPUS table out of $FETCH"
    return
  fi

  local line fields url_field
  local -a urls=()
  while IFS= read -r line; do
    IFS='|' read -r -a fields <<<"$line"
    if [[ "${#fields[@]}" -ne 5 ]]; then
      fail "$name" "entry '$line' has ${#fields[@]} fields, want 5"
      return
    fi
    if [[ ! "${fields[1]}" =~ ^[0-9]+$ ]]; then
      fail "$name" "entry '${fields[0]}' has a non-numeric size '${fields[1]}'"
      return
    fi
    case "${fields[2]}" in
      plain | gz | zip:?*) ;;
      *)
        fail "$name" "entry '${fields[0]}' has an unknown form '${fields[2]}'"
        return
        ;;
    esac
    case "${fields[3]}" in
      default | default,no-citygml | no-citygml) ;;
      *)
        fail "$name" "entry '${fields[0]}' has an unknown set list '${fields[3]}'"
        return
        ;;
    esac
    url_field="${fields[4]}"
    if [[ "$url_field" != http* ]]; then
      fail "$name" "entry '${fields[0]}' has a non-http URL '$url_field'"
      return
    fi
    urls+=("$url_field")
  done <<<"$table"

  local dup
  dup="$(printf '%s\n' "$table" | cut -d'|' -f1 | sort | uniq -d)"
  if [[ -n "$dup" ]]; then
    fail "$name" "duplicate local names in the table: $dup"
    return
  fi

  catalogue="$(grep -v '^#' "$CATALOGUE" | grep -v '^[[:space:]]*$' | sort)"
  local mine
  mine="$(printf '%s\n' "${urls[@]}" | sort)"
  if [[ "$mine" != "$catalogue" ]]; then
    fail "$name" "the table and $CATALOGUE's usable list disagree: $(
      diff <(printf '%s\n' "$catalogue") <(printf '%s\n' "$mine") | tr '\n' ' '
    )"
    return
  fi
  pass "$name"
}

case_forms_are_normalised
case_size_mismatch_hard_fails
case_second_run_skips
case_truncated_file_is_refetched
case_only_selects_by_set
case_unknown_set_rejected
case_pinned_table_is_well_formed

echo "fetch_benchmark_test: $PASSED passed, $FAILED failed"
[[ $FAILED -eq 0 ]]
