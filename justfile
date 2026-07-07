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
