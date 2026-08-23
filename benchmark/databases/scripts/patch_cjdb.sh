#!/usr/bin/env bash
# Build a patched local copy of cjdb==2.2.0 that CjdbSystem drives instead
# of stock cjdb from PyPI. See benchmarking/vendor/cjdb/README.md for why.
#
# Reproducible: downloads the exact pinned sdist (checksum-verified below),
# extracts it, and applies the committed patch
# (benchmarking/vendor/cjdb/ground-surfaces-tie.patch) with `patch -p1` --
# no hand-editing of anything under a venv or a cache directory. Anyone
# cloning this repo gets byte-identical patched source from this one
# command.
#
# Idempotent and content-addressed: the output directory's name embeds the
# patch file's own sha256 prefix
# (benchmarking/.cjdb-patched/cjdb-2.2.0+<patch-hash>/), so editing the
# patch produces a NEW directory rather than silently reusing stale
# content -- this also sidesteps a real `uv run --with <local-dir>`
# gotcha, confirmed while building this script: uv's build cache for a
# local path dependency does NOT reliably invalidate when that path's
# file contents change in place (verified: even `uv cache clean cjdb
# --force` served a stale, pre-patch build; only a full `uv cache clean`
# or a genuinely new path picked it up). A fixed path would silently keep
# running unpatched code after a patch update; a content-addressed path
# cannot.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_ROOT="$(cd "$HERE/.." && pwd)"
PATCH_FILE="$BENCH_ROOT/vendor/cjdb/ground-surfaces-tie.patch"
OUT_ROOT="$BENCH_ROOT/.cjdb-patched"
POINTER_FILE="$OUT_ROOT/current-path"

CJDB_VERSION="2.2.0"
# sha256 of cjdb-2.2.0.tar.gz as published on PyPI, pinned so a compromised
# or silently-altered release under the same version string is caught
# rather than patched-and-trusted blind.
CJDB_SDIST_SHA256="f805bb8fee124230c7d9cb5ffdcedcfc23bfc849a3f0a8a9998052f8077a4a43"

if [[ ! -f "$PATCH_FILE" ]]; then
    echo "error: patch file not found at $PATCH_FILE" >&2
    exit 1
fi

PATCH_HASH="$(sha256sum "$PATCH_FILE" | cut -c1-12)"
BUILD_DIR="$OUT_ROOT/cjdb-${CJDB_VERSION}+${PATCH_HASH}"

if [[ -d "$BUILD_DIR" && -f "$BUILD_DIR/pyproject.toml" ]]; then
    echo "already built: $BUILD_DIR (patch hash unchanged)"
    mkdir -p "$OUT_ROOT"
    echo "$BUILD_DIR" > "$POINTER_FILE"
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo ">> downloading cjdb==$CJDB_VERSION sdist"
python3 -m pip download --no-deps --no-binary :all: \
    "cjdb==$CJDB_VERSION" -d "$WORK" --quiet

SDIST="$WORK/cjdb-${CJDB_VERSION}.tar.gz"
if [[ ! -f "$SDIST" ]]; then
    echo "error: expected $SDIST after download, not found" >&2
    exit 1
fi

ACTUAL_SHA256="$(sha256sum "$SDIST" | cut -d' ' -f1)"
if [[ "$ACTUAL_SHA256" != "$CJDB_SDIST_SHA256" ]]; then
    echo "error: cjdb-${CJDB_VERSION}.tar.gz checksum mismatch" >&2
    echo "  expected: $CJDB_SDIST_SHA256" >&2
    echo "  actual:   $ACTUAL_SHA256" >&2
    echo "  refusing to patch and use an unexpected upstream release." >&2
    exit 1
fi

echo ">> extracting"
tar xzf "$SDIST" -C "$WORK"
SRC="$WORK/cjdb-${CJDB_VERSION}"

echo ">> applying $PATCH_FILE"
(cd "$SRC" && patch -p1 < "$PATCH_FILE")

mkdir -p "$OUT_ROOT"
rm -rf "$BUILD_DIR"
mv "$SRC" "$BUILD_DIR"
echo "$BUILD_DIR" > "$POINTER_FILE"

echo ">> built patched cjdb at $BUILD_DIR"
echo ">> pointer written to $POINTER_FILE"
