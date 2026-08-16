#!/usr/bin/env bash
# Fetch the pinned external converters the read benchmark's conversion chain
# needs (`just fetch-tools`), so the CityGML and CityJSON artefacts every
# format-comparison run measures are reproducible byte-for-byte:
#
#   CityGML --citygml-tools to-cityjson--> CityJSON --cjseq cat--> CityJSONSeq
#                                                            |--fcb ser -A--> FlatCityBuf
#                                                            |--cityparquet convert--> CityParquet
#
# citygml-tools is unpacked into bench/tools/ (gitignored, like bench/data/);
# cjseq is a Rust binary and goes wherever `cargo install` puts it.
#
# Reproducibility beats freshness, exactly as in scripts/fetch_3dbag.sh: the
# citygml-tools version, its download URL and its archive's sha256 are
# HARDCODED below rather than re-derived from "latest" at run time, and a
# mismatch retries the download ONCE before hard-failing — the benchmark must
# never quietly measure artefacts produced by a different converter than the
# one bench/READ_BENCHMARK.md's Environment block names.
#
# WHY BOTH TOOLS. citygml-tools converts CityGML to CityJSON; cjseq performs
# the CityJSON -> CityJSONSeq hop. FlatCityBuf and CityParquet are then both
# built from that SAME CityJSONSeq, which is what makes their comparison fair.
#
# Idempotent: an already-unpacked citygml-tools of the pinned version is left
# alone, and an already-installed cjseq is never reinstalled (a version other
# than the pin is reported loudly, not silently downgraded — it is the
# developer's machine, and the version actually used is recorded in
# bench/tools/tool_versions.txt for the Environment block).
#
# Network-dependent; like the other fetch_*.sh scripts it is NOT wired into
# `just check`/CI.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS_DIR="$REPO_ROOT/bench/tools"

# --- the pins --------------------------------------------------------------
# citygml-tools 2.5.0 (2026-04-19), the current release of citygml4j/
# citygml-tools. sha256 computed 2026-08-16 against the bytes this exact URL
# served.
CITYGML_TOOLS_VERSION="2.5.0"
CITYGML_TOOLS_URL="https://github.com/citygml4j/citygml-tools/releases/download/v${CITYGML_TOOLS_VERSION}/citygml-tools-${CITYGML_TOOLS_VERSION}.zip"
CITYGML_TOOLS_SHA256="bb2949fbc6c3ec44ec85c25a0bcdfe9accde9cbdb9e26ec773889779694710b1"

# cjseq 0.3.1 (cityjson/cjseq), installed via cargo.
CJSEQ_VERSION="0.3.1"

# citygml-tools 2.x runs on Java 17 or newer.
JAVA_MIN_MAJOR=17

# Where readbench_prepare.sh looks: a version-independent symlink, so the
# pinned version lives in THIS file only and never has to be repeated there.
CITYGML_TOOLS_HOME="$TOOLS_DIR/citygml-tools-${CITYGML_TOOLS_VERSION}"
CITYGML_TOOLS_LINK="$TOOLS_DIR/citygml-tools"

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

require_tool() {
  local tool=$1 reason=$2
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: $tool not found on PATH (needed for $reason)" >&2
    exit 1
  fi
}

# --- java ------------------------------------------------------------------
# citygml-tools is a Java program; without a new enough JVM the fetch is
# pointless, so this fails here rather than at the first conversion.
check_java() {
  if ! command -v java >/dev/null 2>&1; then
    echo "error: java not found on PATH; citygml-tools ${CITYGML_TOOLS_VERSION} needs Java ${JAVA_MIN_MAJOR} or newer" >&2
    exit 1
  fi
  local raw major
  # `java -version` prints to stderr, e.g. `openjdk version "21.0.11" ...` or
  # the legacy `java version "1.8.0_402"`.
  raw="$(java -version 2>&1 | head -1 | sed -n 's/.*version "\([^"]*\)".*/\1/p')"
  if [[ -z "$raw" ]]; then
    echo "error: could not read a version out of \`java -version\`; citygml-tools needs Java ${JAVA_MIN_MAJOR} or newer" >&2
    java -version >&2 || true
    exit 1
  fi
  major="${raw%%.*}"
  # Java 8 and older report 1.x; the release number is the second component.
  if [[ "$major" == "1" ]]; then
    local rest="${raw#*.}"
    major="${rest%%.*}"
  fi
  if [[ ! "$major" =~ ^[0-9]+$ ]]; then
    echo "error: could not parse a major version out of java version '$raw'" >&2
    exit 1
  fi
  if [[ "$major" -lt "$JAVA_MIN_MAJOR" ]]; then
    echo "error: java $raw is too old; citygml-tools ${CITYGML_TOOLS_VERSION} needs Java ${JAVA_MIN_MAJOR} or newer" >&2
    exit 1
  fi
  echo "java $raw (>= $JAVA_MIN_MAJOR) ok"
}

# --- citygml-tools ---------------------------------------------------------
download_citygml_tools() {
  local zip=$1
  echo "fetch citygml-tools ${CITYGML_TOOLS_VERSION} <- $CITYGML_TOOLS_URL"
  curl -fL -o "$zip" "$CITYGML_TOOLS_URL"
}

install_citygml_tools() {
  local zip="$TOOLS_DIR/citygml-tools-${CITYGML_TOOLS_VERSION}.zip"
  local actual_sha

  if [[ -x "$CITYGML_TOOLS_HOME/citygml-tools" ]]; then
    echo "skip citygml-tools ${CITYGML_TOOLS_VERSION} (already unpacked: $CITYGML_TOOLS_HOME)"
  else
    rm -rf "$CITYGML_TOOLS_HOME"
    download_citygml_tools "$zip"
    actual_sha="$(sha256_of "$zip")"
    if [[ "$actual_sha" != "$CITYGML_TOOLS_SHA256" ]]; then
      echo "warn citygml-tools: sha256 mismatch after first download (got $actual_sha," \
        "want $CITYGML_TOOLS_SHA256) -- retrying once" >&2
      rm -f "$zip"
      download_citygml_tools "$zip"
      actual_sha="$(sha256_of "$zip")"
      if [[ "$actual_sha" != "$CITYGML_TOOLS_SHA256" ]]; then
        echo "error: citygml-tools still fails sha256 verification after a retry" \
          "(got $actual_sha, want $CITYGML_TOOLS_SHA256) -- refusing to build benchmark" \
          "artefacts with an unverified converter" >&2
        rm -f "$zip"
        exit 1
      fi
    fi
    echo "  sha256 verified: citygml-tools-${CITYGML_TOOLS_VERSION}.zip"
    unzip -q "$zip" -d "$TOOLS_DIR"
    rm -f "$zip"
    if [[ ! -f "$CITYGML_TOOLS_HOME/citygml-tools" ]]; then
      echo "error: the archive did not contain $CITYGML_TOOLS_HOME/citygml-tools" >&2
      exit 1
    fi
    chmod +x "$CITYGML_TOOLS_HOME/citygml-tools"
    echo "  -> $CITYGML_TOOLS_HOME"
  fi

  # The version-independent entry point readbench_prepare.sh resolves.
  ln -sfn "citygml-tools-${CITYGML_TOOLS_VERSION}" "$CITYGML_TOOLS_LINK"

  # Prove it actually runs, rather than trusting that unzip succeeded.
  if ! "$CITYGML_TOOLS_LINK/citygml-tools" --version >/dev/null 2>&1; then
    echo "error: $CITYGML_TOOLS_LINK/citygml-tools --version failed; the install is not usable" >&2
    "$CITYGML_TOOLS_LINK/citygml-tools" --version >&2 || true
    exit 1
  fi
}

# --- cjseq -----------------------------------------------------------------
install_cjseq() {
  if command -v cjseq >/dev/null 2>&1; then
    local have
    have="$(cjseq --version 2>/dev/null | awk '{print $2}')"
    if [[ "$have" == "$CJSEQ_VERSION" ]]; then
      echo "skip cjseq ${CJSEQ_VERSION} (already installed)"
    else
      echo "warn cjseq: installed version '$have' is not the pinned ${CJSEQ_VERSION};" \
        "leaving it alone (run \`cargo install cjseq --version ${CJSEQ_VERSION} --locked\`" \
        "to match the pin). The version actually used is recorded in" \
        "bench/tools/tool_versions.txt." >&2
    fi
    return
  fi
  require_tool cargo "installing cjseq ${CJSEQ_VERSION}"
  echo "install cjseq ${CJSEQ_VERSION} (cargo install)"
  cargo install cjseq --version "$CJSEQ_VERSION" --locked
  if ! command -v cjseq >/dev/null 2>&1; then
    echo "error: cjseq is still not on PATH after \`cargo install\`; is ~/.cargo/bin on your PATH?" >&2
    exit 1
  fi
}

# --- provenance ------------------------------------------------------------
# The exact versions used, for bench/READ_BENCHMARK.md's Environment block.
# Written from what the binaries REPORT, not from the pins, so a warned-about
# version drift is visible in the record rather than papered over.
write_versions() {
  local file="$TOOLS_DIR/tool_versions.txt"
  {
    echo "# Conversion-chain tool versions, written by scripts/fetch_tools.sh."
    echo "# Copy into bench/READ_BENCHMARK.md's Environment block."
    echo "citygml-tools = $("$CITYGML_TOOLS_LINK/citygml-tools" --version 2>&1 | head -1)"
    echo "cjseq = $(cjseq --version 2>&1 | head -1)"
    echo "java = $(java -version 2>&1 | head -1)"
  } >"$file"
  echo "-- versions recorded in $file"
  cat "$file"
}

require_tool curl "downloading citygml-tools"
require_tool unzip "unpacking citygml-tools"
mkdir -p "$TOOLS_DIR"
check_java
install_citygml_tools
install_cjseq
write_versions

echo "fetch-tools complete"
