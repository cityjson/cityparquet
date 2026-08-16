test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

check: lint test isolation vendor-check
    cargo fmt --all --check

# Lint + test the vendored submodules under vendor/ (currently just
# city3d-stac-tool). They are deliberately kept OUT of this Cargo workspace
# (see `exclude` in Cargo.toml) because each is an independent upstream project
# with its own lockfile and toolchain — which also means `cargo --workspace`
# never reaches them, so the local patches carried there (e.g. the
# `update-collection --items-dir` flag the catalogue driver depends on) and
# their tests would otherwise go unverified by `just check`. Mirrors the tool's
# own verification baseline (fmt + clippy + test), and fails loudly rather than
# skipping when the submodule has not been checked out.
vendor-check:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="vendor/city3d-stac-tool"
    if [[ ! -f "${dir}/Cargo.toml" ]]; then
        echo "vendor-check: ${dir} is not checked out; run 'git submodule update --init'" >&2
        exit 1
    fi
    cargo fmt --manifest-path "${dir}/Cargo.toml" --all --check
    cargo clippy --manifest-path "${dir}/Cargo.toml" --all-targets --all-features -- -D warnings
    cargo test --manifest-path "${dir}/Cargo.toml" --all-features

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
    # Real CityGML 1.0 fixture (jklimke/libcitygml, same pinned commit as the
    # 2.0 fixtures above): a Berlin open-data sample whose root <CityModel>
    # binds the default namespace to .../citygml/1.0. Used by
    # `citygml_version_error` to prove a non-2.0 document fails with a clear
    # version message instead of a bogus "invalid CityJSON" JSON parse error.
    curl -sSfo tests/fixtures/berlin_citygml1.gml https://raw.githubusercontent.com/jklimke/libcitygml/141ed719c0ccdf8691e1dc98aa4f915438292b6b/data/berlin_open_data_sample_data.citygml
    # First 400 kB of Freiburg's real LoD2 CityGML 2.0 export: a file that
    # declares its CRS ONLY inside city objects (srsName appears 60,108 times
    # in the full file, never before the first cityObjectMember at byte 2534).
    # A range request, not the whole 1.86 GiB - `parse_header` stops at the
    # first srsName, so the truncated tail is never read. Used by
    # `citygml_srsname_fallback`; this exact shape could not be found in any
    # smaller published CityGML 2.0 file.
    curl -sSf -r 0-399999 -o tests/fixtures/freiburg_no_preamble_srs.gml https://geoportal.freiburg.de/stadtmodell/20240426_Freiburg_LoD2.gml
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

# Fetch the pinned external converters the read benchmark's conversion chain
# needs: citygml-tools (CityGML -> CityJSON) into bench/tools/ (gitignored,
# sha256-verified) and cjseq (CityJSON -> CityJSONSeq) via `cargo install`.
# Needs java 17+; network-dependent; kept OUT of `just check`/CI. The exact
# versions used are written to bench/tools/tool_versions.txt for
# bench/READ_BENCHMARK.md's Environment block.
fetch-tools:
    ./scripts/fetch_tools.sh

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
            "$f" --output "$dest" --overwrite
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
        # By-type is the only, mandatory table layout: resolve the package's
        # single main table from its own metadata.json STAC Item (the
        # `cityparquet-objects` asset role) rather than assuming the
        # pre-by-type "cityobjects.parquet" name. `package_tables.py --single`
        # succeeds only for a single-family dataset; an empty `main_table`
        # here just skips the optional attr-stats column detection, and
        # `readbench_duckdb.sh` below still hard-fails clearly for a
        # multi-family/multi-table package.
        main_table="$(./scripts/package_tables.py "$pkg" --single 2>/dev/null || true)"
        numeric_col=""
        if [[ -n "$main_table" ]]; then
            numeric_col="$(duckdb -csv -noheader -c "
                SELECT column_name FROM (DESCRIBE SELECT * FROM read_parquet('${pkg}/${main_table}'))
                WHERE column_type IN ('BIGINT', 'DOUBLE')
                  AND column_name NOT IN ('id', 'feature_id', 'object_type', 'parents',
                    'children', 'children_roles', 'bbox', 'material', 'texture',
                    'template', 'other')
                  AND column_name NOT LIKE 'geometry_lod%'
                  AND column_name NOT LIKE 'geometry_properties_lod%'
                ORDER BY column_name LIMIT 1;
            " 2>/dev/null || true)"
        fi

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

# ---------------------------------------------------------------------------
# STAC catalogue -> CityParquet mirror (tools/catalog2cityparquet)
# ---------------------------------------------------------------------------

# Build the two release binaries the Python driver shells out to: the
# `cityparquet` converter (one package per catalogue item) and the vendored
# `city3dstac` aggregator (collection.json / items.parquet / catalog.json).
# The driver's own defaults point at exactly these two paths, so building them
# here is what makes the `catalog-*` recipes below runnable from a clean tree.
# Compiling only; kept OUT of `just check`, which builds and tests both trees
# anyway (see `vendor-check`).
catalog-tools:
    cargo build --release -p cityparquet-cli
    cargo build --release --manifest-path vendor/city3d-stac-tool/Cargo.toml

# Convert every collection of the published City3D STAC catalogue into a
# CityParquet mirror under OUT. Resumable: an item whose package already
# carries a valid STAC Item is skipped, so a re-run continues where the last
# one stopped. Failures never abort the run — each is recorded in
# OUT/_reports/ and the next item (or collection) starts, which is what makes
# the end-of-run histogram a measurement rather than a crash report. Extra
# driver flags go after the OUT argument (e.g. `just catalog-convert out/x
# --jobs 4`). Network-dependent, and hours long on the whole catalogue; kept
# OUT of `just check`/CI.
catalog-convert OUT='out/cityparquet-catalog' *ARGS: catalog-tools
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} {{ARGS}}

# Convert a single collection (e.g. `just catalog-convert-collection
# rotterdam-3d`), which is how a change to the driver or the converter is
# proven against real data without paying for the whole catalogue.
# Network-dependent; kept OUT of `just check`/CI.
catalog-convert-collection ID OUT='out/cityparquet-catalog' *ARGS: catalog-tools
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} --collection {{ID}} {{ARGS}}

# Rebuild the mirror's ROOT catalog.json from the collection.json files an
# earlier run left under OUT — no downloads, no conversions, no per-collection
# re-aggregation. Reach for it when a run was interrupted after its collections
# were written but before they were linked together. Rebuilding a single
# collection.json/items.parquet instead needs a plain `catalog-convert` for that
# collection (already-converted items are skipped, and the aggregation step
# still runs). Contacts the catalogue root for the mirror's identity metadata,
# and degrades to defaults if it cannot; kept OUT of `just check`/CI.
catalog-aggregate OUT='out/cityparquet-catalog': catalog-tools
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} --aggregate-only

# Reduce a run's cumulative ledger (OUT/_reports) to one outcome per item and
# print the conformance histogram. This — not a hand roll-up of the JSONL — is
# how the published number is produced: the files are append-only and
# resumption re-attempts a previously FAILED item, so an item legitimately
# appears twice with two different outcomes and counting lines over-counts
# failures. Needs no network and no binaries.
catalog-histogram OUT='out/cityparquet-catalog':
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        histogram {{OUT}}/_reports

# The driver's own test suite. No network and no binaries: every origin,
# subprocess and catalogue document is faked, so this is safe to run anywhere.
# Not part of `just check`, which is the Rust workspace's gate — run both.
catalog-test:
    uv run --project tools/catalog2cityparquet --extra dev pytest -v
