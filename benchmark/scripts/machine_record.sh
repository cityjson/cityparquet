#!/usr/bin/env bash
# The measurement host, for a results directory's MACHINE.md: what
# benchmark/formats/READ_BENCHMARK.md's "Machine" section asks every run to
# capture. Prints Markdown on stdout; the caller redirects it.
#
# `uname -srm` rather than `uname -a`: kernel, release and architecture are what
# a reader needs to judge a timing, while the node name it would add is the
# host's address on someone's network. These files are committed and published.
set -euo pipefail
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
rustc --version
cargo --version
echo "git $(git rev-parse HEAD)"
echo '```'
