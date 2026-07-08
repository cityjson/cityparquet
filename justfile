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

bench-fixtures:
    mkdir -p bench/results
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input tests/fixtures/delft.city.jsonl --out bench/results/delft.csv
    cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
        --input tests/fixtures/lod3_railway.city.json --out bench/results/railway.csv

interop:
    ./scripts/interop.sh

bench-baseline INPUT CSV:
    ./scripts/bench_duckdb.sh {{INPUT}} {{CSV}}

# Fetch the 3 pinned 3DBAG tiles (dense-urban/suburban/rural) into
# bench/data/ (gitignored). Network-dependent; kept OUT of `just check`/CI.
bench-data:
    ./scripts/fetch_3dbag.sh

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
