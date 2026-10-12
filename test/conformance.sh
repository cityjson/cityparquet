#!/usr/bin/env bash
# Cross-writer conformance: the two CityParquet writers, checked against each
# other and against the specification, on real fixtures.
#
# For each source dataset:
#
#   rs     source ──convert──▶ P_rs ──validate (rs)
#   duckdb source ──insert_cityjson + cityparquet_write──▶ P_dk ──validate (rs)
#   duckdb P_rs ──cityparquet_read + cityparquet_validate + cityparquet_write──▶ P_rs_dk ──validate (rs)
#
# and every package is exported by rs and `compare`d with the source, so a
# package that conforms yet carries the wrong content still fails.
#
# The conformance oracle is `cityparquet validate`, which checks the
# specification's MUST statements at the Parquet logical-type level and
# assumes nothing about how either writer lays a file out. The content oracle
# is `cityparquet compare`. Neither is the writer under test for P_dk.
#
#   ./test/conformance.sh            rebuilds the rs CLI (release), needs a
#                                    duckdb-cityjson build and rs fixtures
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DUCK=${CITYPARQUET_DUCKDB_CITYJSON:-lib/duckdb-cityjson/build/release/duckdb}
FIX=lib/cityparquet-rs/tests/fixtures
[[ -x "$DUCK" ]] || { echo "conformance: no duckdb-cityjson build at $DUCK (just -f lib/duckdb-cityjson/justfile build)" >&2; exit 1; }
[[ -f "$FIX/delft.city.jsonl" ]] || { echo "conformance: fixtures absent; run 'just fixtures' in lib/cityparquet-rs" >&2; exit 1; }

cargo build --quiet --release --manifest-path lib/cityparquet-rs/Cargo.toml -p cityparquet-cli
RS=lib/cityparquet-rs/target/release/cityparquet

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAILED=()

check() { # NAME -- command...
    local name="$1"; shift 2
    if "$@" >"$TMP/log" 2>&1; then
        printf '\033[32mok\033[0m   %s\n' "$name"
    else
        printf '\033[31mFAIL\033[0m %s\n' "$name"; sed 's/^/     /' "$TMP/log" | tail -20
        FAILED+=("$name")
    fi
}

# `compare` prints `equal` (with an optional exclusion count) on a match.
same() { "$RS" compare "$1" "$2" | tee /dev/stderr | grep -q '^equal'; }

# Export a package and compare it with its source.
roundtrip() { # SOURCE PACKAGE
    "$RS" export "$2" "$2.city.jsonl" && same "$1" "$2.city.jsonl"
}

# duckdb-cityjson writes a package from a source: an empty schema, initialised,
# then `insert_cityjson` creates every module table and takes the source's CRS.
duck_write() { # SOURCE OUT
    local insert=insert_cityjson
    [[ "$1" == *.jsonl ]] && insert=insert_cityjsonseq
    "$DUCK" -bail \
        -c "CREATE SCHEMA pkg;" \
        -c "PRAGMA cityparquet_init('pkg');" \
        -c "PRAGMA $insert('pkg', '$1', create_tables = true);" \
        -c "SELECT count(*) FROM cityparquet_write('pkg', '$2');"
}

# duckdb-cityjson reads a package, checks its structure, and writes it again.
duck_rewrite() { # PACKAGE OUT
    "$DUCK" -bail \
        -c "PRAGMA cityparquet_read('$1', 'pkg');" \
        -c "PRAGMA cityparquet_validate('pkg');" \
        -c "SELECT CASE WHEN count(*) = 0 THEN 'ok' ELSE error(string_agg(DISTINCT check_name, ', ')) END FROM cityparquet_validation WHERE severity = 'error';" \
        -c "SELECT count(*) FROM cityparquet_write('pkg', '$2');"
}

# address_location: a real Helsinki building whose `Integrate_LoD[1]` attribute is a
# JSON object (and whose `address` is filled in), from cityparquet-rs's test data.
for src in "$FIX/delft.city.jsonl" "$FIX/lod3_railway.city.json" \
    lib/cityparquet-rs/crates/core/tests/data/address_location.city.jsonl; do
    src="$ROOT/$src"
    name=$(basename "$src" | cut -d. -f1)
    echo "== $name"
    rs="$TMP/$name.rs" dk="$TMP/$name.dk" rsdk="$TMP/$name.rs_dk"

    check "rs writes, rs validates"          -- "$RS" convert "$src" -o "$rs"
    check "  validate P_rs"                  -- "$RS" validate "$rs"
    check "  P_rs round trip"                -- roundtrip "$src" "$rs"

    check "duckdb writes"                    -- duck_write "$src" "$dk"
    check "  validate P_dk"                  -- "$RS" validate "$dk"
    check "  P_dk round trip (rs export)"    -- roundtrip "$src" "$dk"

    check "duckdb reads P_rs, writes again"  -- duck_rewrite "$rs" "$rsdk"
    check "  validate P_rs_dk"               -- "$RS" validate "$rsdk"
    check "  P_rs_dk round trip (rs export)" -- roundtrip "$src" "$rsdk"
done

if ((${#FAILED[@]})); then
    printf '\n%d check(s) failed\n' "${#FAILED[@]}"; exit 1
fi
echo; echo "conformance ok"
