#!/usr/bin/env bash
# The measurement host, for a results directory's MACHINE.md: what
# benchmark/formats/READ_BENCHMARK.md's "Machine" section asks every run to
# capture. Prints Markdown on stdout; the caller redirects it.
#
# `uname -srm` rather than `uname -a`: kernel, release and architecture are what
# a reader needs to judge a timing, while the node name it would add is the
# host's address on someone's network. These files are committed and published.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
version() { if command -v "$1" >/dev/null 2>&1; then "$@"; else echo "$1: not found"; fi; }
echo "# Measurement host"
echo
echo "Captured by benchmark/scripts/machine_record.sh at $(date -u +%Y-%m-%dT%H:%M:%SZ)."
echo
echo '```'
uname -srm
if command -v lscpu >/dev/null 2>&1; then lscpu | sed -n '1,15p'; fi
if command -v free >/dev/null 2>&1; then free -b | head -2; fi
if command -v sysctl >/dev/null 2>&1 && [[ "$(uname -s)" == "Darwin" ]]; then
  sysctl -n machdep.cpu.brand_string hw.memsize
fi
version rustc --version
version cargo --version
echo "git $(git -C "$ROOT" rev-parse HEAD)"
echo '```'
echo
echo "## Isolation"
echo
echo "What benchmark/README.md, \"Running on a shared host\", applies, read at the"
echo "start of the run. Each result CSV's \`.params.json\` records what was applied to"
echo "that run under \`isolation\`."
echo
echo "| Fact | Value |"
echo "| --- | --- |"
row() { echo "| $1 | $2 |"; }
row numactl "$(command -v numactl >/dev/null 2>&1 && echo found || echo "not found")"
row taskset "$(command -v taskset >/dev/null 2>&1 && echo found || echo "not found")"
nodes=/sys/devices/system/node
if [[ "$(uname -s)" != "Linux" ]]; then
  row "NUMA node" "not applied: not Linux"
  row "Pinning command" "not applied: not Linux"
else
  node="${BENCH_NUMA_NODE:-auto}"
  if [[ "$node" == "auto" ]]; then
    # The node with the most MemFree, ties to the lowest id; no NUMA
    # information means node 0 (readbench's own rule).
    node=0 best=-1
    for info in "$nodes"/node*/meminfo; do
      [[ -r "$info" ]] || continue
      id="${info#"$nodes"/node}"; id="${id%%/*}"
      free="$(awk '/MemFree:/{print $4}' "$info")"
      if (( free > best )) || { (( free == best )) && (( id < node )); }; then node="$id" best="$free"; fi
    done
  fi
  if [[ "$node" == "off" ]]; then
    row "NUMA node" "not applied: --numa-node off"
    row "Pinning command" "not applied: --numa-node off"
  else
    cores="$(cat "$nodes/node$node/cpulist" 2>/dev/null || echo "not readable")"
    row "NUMA node" "$node (cores $cores; coordinator on the first, children on the rest)"
    if command -v numactl >/dev/null 2>&1; then
      row "Pinning command" "numactl --physcpubind=<node $node cores minus the first> --membind=$node"
    elif command -v taskset >/dev/null 2>&1; then
      row "Pinning command" "taskset -c <node $node cores minus the first> (memory binding not applied: numactl missing)"
    else
      row "Pinning command" "not applied: neither numactl nor taskset found"
    fi
  fi
fi
if [[ "${MEMORY_MAX:-}" == off ]]; then
  row "Memory limit" "not applied: switched off (readbench --memory-max off)"
elif [[ -n "${MEMORY_MAX:-}" ]]; then
  row "Memory limit" "requested MemoryMax=${MEMORY_MAX} (decimal bytes, $((MEMORY_MAX / 1000000000)) GB) via systemd-run --user --scope"
else
  row "Memory limit" "not applied: not requested (readbench --memory-max)"
fi
controllers="/sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/cgroup.controllers"
if [[ -r "$controllers" ]]; then
  delegated="$(tr ' ' '\n' <"$controllers" | grep -xE 'cpuset|memory' | tr '\n' ' ')"
  row "cgroup delegation" "user@$(id -u).service delegates: ${delegated:-neither cpuset nor memory}"
else
  row "cgroup delegation" "not readable: $controllers"
fi
read_or() { if [[ -r "$1" ]]; then cat "$1"; else echo "not readable"; fi; }
row "CPU governor" "$(read_or /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)"
row "SMT" "$(read_or /sys/devices/system/cpu/smt/active) (/sys/devices/system/cpu/smt/active: 1 = on)"
row "Kernel" "$(uname -sr)"
row "Other users' CPU" "$(ps -eo user=,pcpu= 2>/dev/null | awk -v me="${USER:-$(id -un)}" '$1 != me {s += $2} END {printf "%.1f%% (summed ps pcpu)", s}')"
meminfo() { if [[ -r /proc/meminfo ]]; then awk -v k="$1:" '$1 == k {printf "%d bytes", $2 * 1024}' /proc/meminfo; else echo "not readable: no /proc"; fi; }
row "MemTotal" "$(meminfo MemTotal)"
row "MemAvailable" "$(meminfo MemAvailable) (at start)"
echo
echo "## Tool versions"
echo
echo "| Tool | Version |"
echo "| --- | --- |"
tool() { local name="$1"; shift; if command -v "$1" >/dev/null 2>&1; then row "$name" "$("$@" 2>&1 | head -1)"; else row "$name" "not found"; fi; }
row cityparquet-rs "$(git -C "$ROOT" log -1 --format=%H -- lib/cityparquet-rs 2>/dev/null || echo "not found") (last commit touching lib/cityparquet-rs)"
tool cjseq cjseq --version
tool "fcb CLI" fcb --version
row fcb_core "$(awk '/^name = "fcb_core"/{getline; gsub(/[^0-9.]/,""); print; exit}' "$ROOT/benchmark/readbench/Cargo.lock" 2>/dev/null || echo "not found") (readbench Cargo.lock pin)"
tool citygml-tools citygml-tools --version
tool DuckDB duckdb --version
image() {
  local found
  found="$(grep -rhoE --include='*.py' --include='*.toml' --include='*.yml' --include='*.yaml' --include=justfile \
    "$1[:A-Za-z0-9._-]*" "$ROOT/benchmark/databases" 2>/dev/null | grep -v '/.venv/' | sort -u | head -1 || true)"
  echo "${found:-not found}"
}
row "PostgreSQL/PostGIS" "image $(image postgis/postgis); host psql: $(command -v psql >/dev/null 2>&1 && psql --version || echo "not found")"
row 3DCityDB "image $(image 3dcitydb/3dcitydb-pg)"
tool cjdb cjdb --version
