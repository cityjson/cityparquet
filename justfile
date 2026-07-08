test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

check: lint test isolation
    cargo fmt --all --check

isolation:
    cargo tree -p cityparquet-schema --prefix none | grep -E '^(arrow-array|arrow |parquet) ' && exit 1 || echo "isolation ok"

fixtures:
    mkdir -p tests/fixtures
    curl -sSfo tests/fixtures/delft.city.jsonl https://storage.googleapis.com/cityjson/delft.city.jsonl
    curl -sSfo tests/fixtures/lod3_railway.city.json https://storage.googleapis.com/cityjson/lod3_railway.city.json

# Benchmark every CityJSON/CityJSONSeq file found under FOLDER (recursive;
# default tests/fixtures), writing one bench/results/<name>.csv per input
# where <name> is the input's basename minus its .city.jsonl/.city.json/
# .jsonl/.json extension. Each CSV is removed first so a re-run is clean
# (the `bench` harness appends when the CSV already exists). This runs only
# the cityparquet-rs variant matrix, not the DuckDB baseline (see bench-all).
bench-fixtures FOLDER='tests/fixtures':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p bench/results
    found=0
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        name="${base%.city.jsonl}"; name="${name%.city.json}"
        name="${name%.jsonl}"; name="${name%.json}"
        out="bench/results/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"
        cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
            --input "$f" --out "$out"
        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( -name '*.json' -o -name '*.jsonl' \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "bench-fixtures: no CityJSON/CityJSONSeq files found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "bench-fixtures: ${found} file(s) processed"

# Convert every CityJSON/CityJSONSeq file found under FOLDER (recursive;
# default tests/fixtures) into a CityParquet package under OUT (default
# out/cityparquet), one OUT/<name>/ package directory per input where
# <name> is the input's basename minus its .city.jsonl/.city.json/.jsonl/
# .json extension. PROFILE is core (default) or compatibility. Existing
# packages of the same name are overwritten.
convert-all FOLDER='tests/fixtures' OUT='out/cityparquet' PROFILE='core':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        name="${base%.city.jsonl}"; name="${name%.city.json}"
        name="${name%.jsonl}"; name="${name%.json}"
        dest="{{OUT}}/${name}"
        echo ">> ${f} -> ${dest} (profile {{PROFILE}})"
        cargo run --release -p cityparquet-cli --bin cityparquet -- convert \
            "$f" "$dest" --profile "{{PROFILE}}" --overwrite
        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( -name '*.json' -o -name '*.jsonl' \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "convert-all: no CityJSON/CityJSONSeq files found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "convert-all: ${found} file(s) converted into {{OUT}}"

interop:
    ./scripts/interop.sh

bench-baseline INPUT CSV:
    ./scripts/bench_duckdb.sh {{INPUT}} {{CSV}}

# Prepare the per-format read-benchmark inputs for INPUT: a core-profile and
# a Hilbert-ordered CityParquet package, an FCB file (spatial + all-attribute
# index), and a gzip of the original, all under OUTDIR (default
# bench/data/readbench/, gitignored). Idempotent; needs `fcb` on PATH;
# network-independent but local-CLI-dependent, so kept OUT of `just
# check`/CI like the other bench-* recipes.
readbench-prepare INPUT OUTDIR='bench/data/readbench':
    ./scripts/readbench_prepare.sh {{INPUT}} {{OUTDIR}}

# DuckDB-over-Parquet SQL-engine baseline (Task 12): appends `duckdb-parquet`
# rows for the SQL-expressible scenarios (count/full-read/bbox-query/
# attr-filter/attr-stats/project) to OUT_CSV, querying PARQUET_PKG's own
# `cityobjects.parquet` main table directly (no CityJSON extension, no
# LOAD needed). PARQUET_PKG is a package produced by `just readbench-prepare`
# or `cityparquet convert`; OUT_CSV is the SAME CSV the
# `cityparquet-readbench` coordinator writes (header must match exactly).
# Pass `--numeric-column COL` (a real Int64/Float64 attribute column) to
# also emit the attr-stats row; `--repeat N` overrides the default of 5.
# Local-only: needs `duckdb` + `python3` on PATH; kept OUT of `just
# check`/CI.
readbench-baseline PARQUET_PKG OUT_CSV *ARGS:
    ./scripts/readbench_duckdb.sh {{PARQUET_PKG}} {{OUT_CSV}} {{ARGS}}

# Fetch the 3 pinned 3DBAG tiles (dense-urban/suburban/rural) into
# bench/data/ (gitignored). Network-dependent; kept OUT of `just check`/CI.
bench-data:
    ./scripts/fetch_3dbag.sh

# Fetch the CityJSON benchmark corpus (11 CityJSONSeq datasets, ~1.7 GiB)
# from gs://cityjson/benchmark_dataset/ into bench/data/benchmark/
# (gitignored). Needs gsutil; network-dependent; kept OUT of `just check`/CI.
bench-corpus:
    ./scripts/fetch_benchmark.sh

# Full M5 benchmark run: fixtures + the 3 pinned 3DBAG tiles, each through
# `cityparquet bench` (default 10-variant set, repeat=5) and the DuckDB
# `COPY` baseline, into one CSV per dataset under bench/results/. Requires
# `just bench-data` to have populated bench/data/ first. Network-dependent
# (duckdb baseline installs the `cityjson` community extension); kept OUT
# of `just check`/CI.
bench-all:
    mkdir -p bench/results
    rm -f bench/results/delft.csv bench/results/railway.csv \
        bench/results/9-284-556.csv bench/results/9-304-532.csv bench/results/9-196-328.csv
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input tests/fixtures/delft.city.jsonl --out bench/results/delft.csv
    ./scripts/bench_duckdb.sh tests/fixtures/delft.city.jsonl bench/results/delft.csv
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input tests/fixtures/lod3_railway.city.json --out bench/results/railway.csv
    ./scripts/bench_duckdb.sh tests/fixtures/lod3_railway.city.json bench/results/railway.csv
    # object NL.IMBAG.Pand.0503100000025101-0 (lod 2.2) used to fail the
    # export+compare round-trip check on this tile: a 3-index exterior ring
    # whose distinct vertex indices all quantise to one coordinate, a blind
    # spot in the comparator's (INDEX-only) degenerate-ring normalisation.
    # Resolved by extending the comparator to also drop coordinate-degenerate
    # rings (see crate::compare's module docs); the round trip is now checked
    # like every other dataset, no --skip-roundtrip.
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input bench/data/9-284-556.city.json --out bench/results/9-284-556.csv
    ./scripts/bench_duckdb.sh bench/data/9-284-556.city.json bench/results/9-284-556.csv
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input bench/data/9-304-532.city.json --out bench/results/9-304-532.csv
    ./scripts/bench_duckdb.sh bench/data/9-304-532.city.json bench/results/9-304-532.csv
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input bench/data/9-196-328.city.json --out bench/results/9-196-328.csv
    ./scripts/bench_duckdb.sh bench/data/9-196-328.city.json bench/results/9-196-328.csv
