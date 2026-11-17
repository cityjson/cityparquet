#!/usr/bin/env bash
# `fcb-ser-stream` (benchmark/fcbser) against the tool it stands in for.
#
# The national dataset's FlatCityBuf artefact is written by `fcb-ser-stream`
# because `fcb ser -A` cannot hold that input in memory
# (benchmark/scripts/readbench_prepare.sh, `FCB_SER_STREAM`). The benchmark
# still states ONE FlatCityBuf configuration, so this proves the two writers
# produce the same file on inputs both can read:
#
#   - the same size in bytes — header, R-tree, every attribute index and the
#     feature section together;
#   - the same features in the same (Hilbert) order with the same content,
#     compared as `fcb deser` output with object keys sorted.
#
# NOT byte-for-byte, and that is `fcb ser`'s property rather than a gap here:
# two `fcb ser -A` runs over one input already differ byte for byte (measured
# on a 1,084-feature 3DBAG sample: three runs, three checksums, one size).
#
# Needs the REAL pinned `fcb` (just fetch-tools), `cargo` and `jq`, and
# compiles `fcb_core` — which is why it is `just fcbser-test` and not part of
# `scripts-test`, whose suites stub every external binary.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHMARK_DIR="$(cd "$HERE/../.." && pwd)"
MONO_ROOT="$(cd "$BENCHMARK_DIR/.." && pwd)"

for tool in fcb cargo jq; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found on PATH" >&2; exit 1; }
done

# Real fixtures with attributes, semantic surfaces and more than one feature.
# Extra inputs may be given as arguments (a corpus dataset, say).
INPUTS=(
  "$MONO_ROOT/lib/cityparquet-rs/crates/core/tests/data/helsinki_address.city.jsonl"
  "$MONO_ROOT/lib/cityparquet-rs/crates/core/tests/data/helsinki_tex_uv_count.city.jsonl"
  "$@"
)

( cd "$BENCHMARK_DIR/fcbser" && cargo build --release -q )
STREAM="$BENCHMARK_DIR/fcbser/target/release/fcb-ser-stream"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

canonical() { # FCB -> one sorted-key JSON document per line
  fcb deser "$1" "$WORK/deser.city.jsonl" >/dev/null 2>&1
  jq -S -c . "$WORK/deser.city.jsonl"
}

fails=0
for input in "${INPUTS[@]}"; do
  name="$(basename "$input")"
  [[ -s "$input" ]] || { echo "FAIL $name: no such input" >&2; fails=$((fails + 1)); continue; }
  fcb ser -A "$input" "$WORK/cli.fcb" >/dev/null 2>&1
  "$STREAM" "$input" "$WORK/stream.fcb" 2>/dev/null
  cli_size="$(wc -c <"$WORK/cli.fcb")"
  stream_size="$(wc -c <"$WORK/stream.fcb")"
  if [[ "$cli_size" != "$stream_size" ]]; then
    echo "FAIL $name: fcb ser -A wrote $cli_size bytes, fcb-ser-stream $stream_size" >&2
    fails=$((fails + 1))
    continue
  fi
  canonical "$WORK/cli.fcb" >"$WORK/cli.canonical"
  canonical "$WORK/stream.fcb" >"$WORK/stream.canonical"
  if ! cmp -s "$WORK/cli.canonical" "$WORK/stream.canonical"; then
    echo "FAIL $name: the two files decode to different features" >&2
    fails=$((fails + 1))
    continue
  fi
  [[ -s "$WORK/cli.canonical" ]] || { echo "FAIL $name: nothing decoded" >&2; fails=$((fails + 1)); continue; }
  echo "ok   $name: $cli_size bytes, $(($(wc -l <"$WORK/cli.canonical") - 1)) features, identical content"
done

exit $((fails > 0))
