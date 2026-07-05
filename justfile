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

interop:
    ./scripts/interop.sh
