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
    # CityGML 2.0 reader fixtures (jklimke/libcitygml, pinned to a commit SHA):
    # b1_lod2_s = one Building / lod2 gml:Solid (gml:pos coords, no CRS/attrs/semantics);
    # b1_lod2_cs_w_sem = lod2 Solid + boundedBy Wall/Roof/Ground via xlink (semantics).
    curl -sSfo tests/fixtures/b1_lod2_s.gml https://raw.githubusercontent.com/jklimke/libcitygml/141ed719c0ccdf8691e1dc98aa4f915438292b6b/data/b1_lod2_s.gml
    curl -sSfo tests/fixtures/b1_lod2_cs_w_sem.gml https://raw.githubusercontent.com/jklimke/libcitygml/141ed719c0ccdf8691e1dc98aa4f915438292b6b/data/b1_lod2_cs_w_sem.gml
    # Zero-object CityJSONSeq fixture (synthetic, no network fetch needed):
    # a single CityJSON header line, empty CityObjects/vertices, no feature
    # lines — the minimal input that scans to zero city-object rows. Used by
    # `zero_object_input_is_rejected` (crates/cityparquet/tests/bytype_layout.rs)
    # to prove a zero-object conversion is rejected rather than silently
    # producing an empty package.
    printf '%s\n' '{"type":"CityJSON","version":"2.0","transform":{"scale":[0.001,0.001,0.001],"translate":[0.0,0.0,0.0]},"CityObjects":{},"vertices":[]}' > tests/fixtures/empty.city.jsonl

interop:
    ./scripts/interop.sh

# Fetch the CityJSON benchmark corpus (11 CityJSONSeq datasets, ~1.7 GiB)
# from gs://cityjson/benchmark_dataset/ into DEST (default
# bench/data/benchmark/, gitignored). Needs gsutil; network-dependent; kept
# OUT of `just check`/CI. Verifies each file's byte size after download (see
# scripts/fetch_benchmark.sh).
fetch-data DEST='bench/data/benchmark':
    ./scripts/fetch_benchmark.sh {{DEST}}

# Convert every CityJSON/CityJSONSeq file found under FOLDER (recursive)
# into a CityParquet package under OUT (default out/cityparquet), one
# OUT/<name>/ package directory per input where <name> is the input's
# basename minus its .city.jsonl/.city.json/.jsonl/.json extension (core
# profile; existing packages of the same name are overwritten).
convert-all FOLDER OUT='out/cityparquet':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        name="${base%.city.jsonl}"; name="${name%.city.json}"
        name="${name%.jsonl}"; name="${name%.json}"
        dest="{{OUT}}/${name}"
        echo ">> ${f} -> ${dest}"
        cargo run --release -p cityparquet-cli --bin cityparquet -- convert \
            "$f" "$dest" --overwrite
        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( -name '*.json' -o -name '*.jsonl' \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "convert-all: no CityJSON/CityJSONSeq files found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "convert-all: ${found} file(s) converted into {{OUT}}"

# Cross-format READ benchmark (see bench/READ_BENCHMARK.md): for every
# CityJSON/CityJSONSeq file found under FOLDER (recursive), prepare every
# compared format (parquet/hilbert/fcb/gz, `scripts/readbench_prepare.sh`),
# run the `cityparquet-readbench` coordinator across the whole (format x
# scenario) matrix into one OUT/<name>.csv, then append the `duckdb-parquet`
# SQL-engine baseline to the SAME csv (`scripts/readbench_duckdb.sh`),
# auto-detecting a numeric attribute column via a `DESCRIBE` query where
# possible (omitted, skipping attr-stats, if none is found). Each
# OUT/<name>.csv is removed first so a re-run is always clean. Once every
# dataset is done, renders charts from the CSVs via the `plot` recipe
# (best-effort: a missing `uv`/plotting setup doesn't fail the benchmark
# run, only skips the charts). Needs `fcb`+`duckdb` on PATH; network-
# independent given already-fetched inputs; kept OUT of `just check`/CI.
bench FOLDER OUT='bench/read_results':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}" bench/data/readbench
    found=0
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        name="${base%.city.jsonl}"; name="${name%.city.json}"
        name="${name%.jsonl}"; name="${name%.json}"
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        ./scripts/readbench_prepare.sh "$f" bench/data/readbench

        cargo run --release -p cityparquet-readbench -- run \
            --input "$f" \
            --prepared-dir bench/data/readbench \
            --out "$out" \
            --repeat 7

        pkg="bench/data/readbench/${name}.parquet"
        numeric_col="$(duckdb -csv -noheader -c "
            SELECT column_name FROM (DESCRIBE SELECT * FROM read_parquet('${pkg}/cityobjects.parquet'))
            WHERE column_type IN ('BIGINT', 'DOUBLE')
              AND column_name NOT IN ('id', 'feature_id', 'object_type', 'parents',
                'children', 'children_roles', 'bbox', 'material', 'texture',
                'template', 'other')
              AND column_name NOT LIKE 'geometry_lod%'
              AND column_name NOT LIKE 'geometry_properties_lod%'
            ORDER BY column_name LIMIT 1;
        " 2>/dev/null || true)"

        if [[ -n "$numeric_col" ]]; then
            echo "-- numeric attribute column for attr-stats: ${numeric_col}"
            ./scripts/readbench_duckdb.sh "$pkg" "$out" --numeric-column "$numeric_col" --repeat 7
        else
            echo "-- no numeric attribute column detected; skipping attr-stats for duckdb-parquet"
            ./scripts/readbench_duckdb.sh "$pkg" "$out" --repeat 7
        fi

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( -name '*.json' -o -name '*.jsonl' \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "bench: no CityJSON/CityJSONSeq files found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "bench: ${found} file(s) benchmarked into {{OUT}}"

    just plot "{{OUT}}" || echo "plot skipped (uv not available)"
    just sizes "bench/data/readbench" "{{OUT}}" || echo "sizes skipped (uv not available)"

# Encoding-variant WRITE benchmark (M5): for every CityJSON/CityJSONSeq file
# found under FOLDER (recursive), run the `cityparquet bench` variant matrix
# and append the DuckDB `COPY` baseline into one OUT/<name>.csv. Each
# OUT/<name>.csv is removed first so a re-run is always clean.
# Network-dependent (the DuckDB baseline installs the `cityjson` community
# extension); kept OUT of `just check`/CI.
write-bench FOLDER OUT='bench/results':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        name="${base%.city.jsonl}"; name="${name%.city.json}"
        name="${name%.jsonl}"; name="${name%.json}"
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
            --input "$f" --out "$out"
        ./scripts/bench_duckdb.sh "$f" "$out"

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( -name '*.json' -o -name '*.jsonl' \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "write-bench: no CityJSON/CityJSONSeq files found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "write-bench: ${found} file(s) benchmarked into {{OUT}}"

# Compression-codec + row-group WRITE-bench recipe: for every CityJSON/
# CityJSONSeq file found under FOLDER (recursive), runs `cityparquet bench`
# over an 8-variant matrix — a codec axis (bare `cityparquet` = zstd,
# `+uncompressed`, `+snappy`, `+gzip`, `+lz4`, `+brotli`, all at the default
# row-group size 65536) and a row-group axis (`cityparquet`, `+rg512`,
# `+rg4096`, all zstd) — into one OUT/<name>.csv per dataset. Each
# OUT/<name>.csv is removed first so a re-run is always clean. Once every
# dataset is done, renders charts from the CSVs via the `compression-plot`
# recipe (best-effort: a missing `uv`/plotting setup doesn't fail the
# benchmark run, only skips the charts). Network-independent given already-
# fetched inputs; kept OUT of `just check`/CI.
compression-bench FOLDER OUT='bench/compression_results':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        name="${base%.city.jsonl}"; name="${name%.city.json}"
        name="${name%.jsonl}"; name="${name%.json}"
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        cargo run --release -p cityparquet-cli --bin cityparquet -- bench \
            --input "$f" --out "$out" \
            --variants "cityparquet,cityparquet+uncompressed,cityparquet+snappy,cityparquet+gzip,cityparquet+lz4,cityparquet+brotli,cityparquet+rg512,cityparquet+rg4096"

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( -name '*.json' -o -name '*.jsonl' \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "compression-bench: no CityJSON/CityJSONSeq files found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "compression-bench: ${found} file(s) benchmarked into {{OUT}}"

    just compression-plot "{{OUT}}" || echo "plot skipped"

# Render compression-codec and row-group comparison charts from the
# compression-bench CSVs in RESULTS (default bench/compression_results) via
# the `bench/plot` uv project: per dataset, two codec-axis charts
# (<name>-codec-size.png, <name>-codec-time.png) and one row-group-axis
# chart (<name>-rowgroup.png), under RESULTS/plots/. Needs `uv` on PATH.
compression-plot RESULTS='bench/compression_results':
    uv run --project bench/plot python -m readbench_plot.compression {{RESULTS}}

# Render charts from the read-benchmark CSVs in RESULTS (default
# bench/read_results) via the `bench/plot` uv project: a grouped bar chart
# of median time_s and one of peak_heap_bytes per scenario x format, one PNG
# pair per dataset CSV, under RESULTS/plots/. Needs `uv` on PATH.
plot RESULTS='bench/read_results':
    uv run --project bench/plot python -m readbench_plot {{RESULTS}}

# Render the file-size / compression-ratio report from PREPARED_DIR (default
# bench/data/readbench, the same per-format artefacts `readbench_prepare.sh`
# populates): OUT/sizes.csv (dataset, format, bytes, mb,
# ratio_vs_cityjsonseq) plus two grouped bar charts under OUT/plots/
# (sizes.png, compression-ratio.png). Needs `uv` on PATH.
sizes PREPARED_DIR='bench/data/readbench' OUT='bench/read_results':
    uv run --project bench/plot python -m readbench_plot.sizes {{PREPARED_DIR}} {{OUT}}
