#!/usr/bin/env bash
# machine_record.sh prints the host, its isolation facts and the tool
# versions, degrading to explicit "not readable" / "not found" entries.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/../machine_record.sh"
passed=0 failed=0
pass() { echo "ok   - $1"; passed=$((passed + 1)); }
fail() { echo "not ok - $1: $2"; failed=$((failed + 1)); }

out="$("$SCRIPT")"

case_sections() {
  local name="MACHINE.md has the host, isolation and tool-version sections"
  local heading
  for heading in "# Measurement host" "## Isolation" "## Tool versions"; do
    if ! grep -qxF "$heading" <<<"$out"; then fail "$name" "missing $heading"; return; fi
  done
  pass "$name"
}

case_every_fact_has_a_row() {
  local name="every isolation fact and tool has a row"
  local key
  for key in "numactl" "taskset" "NUMA node" "Pinning command" "Memory limit" \
    "cgroup delegation" "CPU governor" "SMT" "Kernel" "Other users' CPU" \
    "MemTotal" "MemAvailable" "cityparquet-rs" "cjseq" "fcb CLI" "fcb_core" \
    "citygml-tools" "DuckDB" "PostgreSQL/PostGIS" "3DCityDB" "cjdb"; do
    if ! grep -q "^| $key" <<<"$out"; then fail "$name" "no row for $key"; return; fi
  done
  pass "$name"
}

case_fcb_core_pin_is_read_from_the_lock() {
  local name="the fcb_core row carries the readbench Cargo.lock pin"
  local pin
  pin="$(awk '/^name = "fcb_core"/{getline; gsub(/[^0-9.]/,""); print; exit}' "$HERE/../../readbench/Cargo.lock")"
  if grep -q "^| fcb_core | $pin" <<<"$out"; then pass "$name"; else fail "$name" "want $pin"; fi
}

case_degrades_explicitly_off_linux() {
  local name="off Linux the Linux-only facts say why they are absent"
  if [[ "$(uname -s)" == "Linux" ]]; then pass "$name (skipped on Linux)"; return; fi
  if grep -q "| CPU governor | not readable" <<<"$out" \
    && grep -q "| NUMA node | not applied: not Linux" <<<"$out" \
    && grep -q "| cgroup delegation | not readable" <<<"$out"; then
    pass "$name"
  else
    fail "$name" "$(grep -E 'governor|NUMA node|delegation' <<<"$out")"
  fi
}

case_absent_tools_say_not_found() {
  local name="a tool missing from PATH is recorded as not found"
  local bare
  bare="$(PATH=/usr/bin:/bin "$SCRIPT")"
  if grep -q "^| cjdb | not found" <<<"$bare"; then pass "$name"; else fail "$name" "$(grep '^| cjdb' <<<"$bare")"; fi
}

case_memory_ceiling_is_recorded_in_decimal_bytes_or_off() {
  local name="the memory ceiling is recorded as decimal bytes, and off as not requested"
  local on off
  on="$(MEMORY_MAX=64000000000 "$SCRIPT" 2>/dev/null | grep '| Memory limit |')"
  off="$(MEMORY_MAX=off "$SCRIPT" 2>/dev/null | grep '| Memory limit |')"
  if [[ "$on" == *"MemoryMax=64000000000 (decimal bytes, 64 GB)"* && "$off" == *"not applied: switched off"* ]]; then
    pass "$name"
  else
    fail "$name" "$on / $off"
  fi
}

case_sections
case_every_fact_has_a_row
case_memory_ceiling_is_recorded_in_decimal_bytes_or_off
case_fcb_core_pin_is_read_from_the_lock
case_degrades_explicitly_off_linux
case_absent_tools_say_not_found
echo "machine_record_test: $passed passed, $failed failed"
[[ $failed -eq 0 ]]
