# ===========================================================================
# CityParquet monorepo — top-level task runner.
#
# WHAT LIVES HERE, AND WHY.
#
# The benchmark is ONE tree: `benchmark/` holds the harness crate
# (`readbench/`), its scripts (`scripts/`), its corpora and results
# (`formats/`, `databases/`) and its renderers (`plot/`). But a benchmark
# recipe still has to reach the LIBRARY — to build the converter it measures —
# and the catalogue driver reaches it too, so those recipes sit HERE, where
# both are visible. Every path below is relative to the repository root.
#
# `lib/cityparquet-rs/justfile` keeps what belongs to the library alone:
# `test`, `lint`, `fmt`, `check`, `vendor-check`, `fixtures`, `interop`. Run
# those from inside that directory; its `check` needs no `uv`, no `jq` and no
# corpus, which is the point of the split.
#
# The three per-dataset recipes (`convert-all`, `bench`,
# `variant-bench`) are deliberately in ONE file: they share the
# input-extension convention below verbatim, and
# `benchmark/readbench/tests/strip_extension.rs` extracts all three
# out of this file and RUNS them to prove they have not drifted apart. Split
# them across two justfiles and that check has nothing to compare.
# ===========================================================================

RS := "lib/cityparquet-rs"
BENCH := "benchmark"
PLOT := "benchmark/plot"
BENCH_SCRIPTS := "benchmark/scripts"
# Two workspaces, two manifests. The library's builds the converter; the
# benchmark harness is its own workspace under benchmark/, so `cargo` has to be
# told which one each recipe means.
CARGO := "--manifest-path " + RS + "/Cargo.toml"
READBENCH_CARGO := "--manifest-path benchmark/readbench/Cargo.toml"

default:
    @just --list

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

# Full checkout: every submodule, including the two DuckDB extensions' own
# nested `duckdb`/`extension-ci-tools`/`vcpkg`. This is the ~1.2 GB path, and
# it is what you need to build the extensions or run `test/run-all.sh`.
[doc("Check out every submodule, recursively (~1.2 GB)")]
setup:
    git submodule update --init --recursive

# Spec-and-Rust-only checkout: the same submodules at depth 1. Enough to read
# and build the specification site and the Rust library; NOT enough to build
# the DuckDB extensions, whose build wants real vcpkg history.
[doc("The same at --depth 1: enough for the spec and the Rust library")]
setup-shallow:
    git submodule update --init --recursive --depth 1

# Point git at the repo's hooks, so .githooks/pre-commit formats the staged
# Rust and Markdown on every commit. One-off per clone — git checks the hook
# out but does not activate it.
[doc("Activate .githooks/pre-commit (one-off per clone)")]
hooks:
    git config core.hooksPath .githooks

# ---------------------------------------------------------------------------
# Gates
# ---------------------------------------------------------------------------

# Everything that gates a change to the Rust library, the benchmark harness
# and CityLake. The three are separate Cargo workspaces, so each needs its own
# clippy/test/fmt pass — `cd lib/cityparquet-rs && just check` deliberately
# does NOT reach the harness or CityLake, which is what keeps that gate
# runnable with no `uv`, no `jq`, no corpus and no local extension build.
[doc("The full gate: three Rust workspaces, the plotting suite, the shell suites")]
check:
    cd {{RS}} && just check
    cargo clippy {{READBENCH_CARGO}} --all-targets -- -D warnings
    cargo test {{READBENCH_CARGO}}
    cargo fmt {{READBENCH_CARGO}} --check
    just plot-test
    just scripts-test
    just mcp-check
    just citylake-check

# The cross-module manual walkthrough, automated. Needs both DuckDB
# extensions built — see test/TESTING.md for what each step proves.
[doc("Every automated suite in the tree (test/run-all.sh)")]
test-all:
    ./test/run-all.sh

# ---------------------------------------------------------------------------
# Specification site (documents/)
# ---------------------------------------------------------------------------

[doc("Install the specification site's dependencies")]
docs-install:
    cd documents && pnpm install

[doc("Serve the specification site locally")]
docs-dev: docs-install
    cd documents && pnpm dev

[doc("Build the specification site into documents/dist")]
docs-build: docs-install
    cd documents && pnpm build

# ---------------------------------------------------------------------------
# The INPUT-EXTENSION CONVENTION, shared by every per-dataset recipe below.
#
# A benchmark input is `<dataset><ext>`, and `<dataset>` names everything
# derived from it (a package directory, a results CSV, every prepared
# artefact). The rule is implemented three times over — here, in
# `benchmark/readbench/src/naming.rs` and in
# `benchmark/scripts/readbench_prepare.sh` — because a shell script cannot
# import a Rust function and `just` has no functions of its own.
# `benchmark/readbench/tests/strip_extension.rs`
# extracts the shell ones from their own source files and RUNS them over the
# same table, so a copy that drifts fails `just check`.
#
# Both lists knew only `.json`/`.jsonl` until CityGML became a measured
# format: a `.gml` input was invisible to every `find` below, and one that
# got through anyway kept its extension and misnamed every artefact
# (`foo.gml.parquet`, `foo.gml.csv`).
#
# KNOWN_INPUT_EXTENSIONS is MOST SPECIFIC FIRST (so `.city.jsonl` wins over
# `.jsonl`) and must match the Rust list exactly, in order.
# ---------------------------------------------------------------------------
KNOWN_INPUT_EXTENSIONS := ".city.jsonl .city.json .citygml .jsonl .json .gml .xml"
# The discovery half of the same convention: a stripper that knows an
# extension `find` never matches is dead code. `*.gml` does NOT match
# `*.citygml` (the suffix would have to be `.gml`, not `gml`), so both are
# listed. `metadata.json` is excluded at each use site — it is a CityParquet
# package's own manifest, not an input.
KNOWN_INPUT_FIND := "-name '*.json' -o -name '*.jsonl' -o -name '*.gml' -o -name '*.citygml' -o -name '*.xml'"

# ---------------------------------------------------------------------------
# Corpora — all network-dependent, all kept OUT of `just check`/CI
# ---------------------------------------------------------------------------

# Fetch the CityParquet benchmark corpus — SEVEN city models (CityJSON 2.0
# `.city.json`, 2.7 MB .. 498 MB, about 1.2 GB on the wire): five from the
# CityJSON project's own dataset page, Tokyo and Montréal from this project's
# mirror. Into DEST (default benchmark/runs/data/benchmark/, gitignored).
# Every entry's byte size and sha256 are pinned and verified, and an
# already-present file is skipped — see
# benchmark/scripts/fetch_benchmark.sh for the table and
# benchmark/formats/corpus_urls.txt for each URL's provenance. Needs curl;
# network-dependent; kept OUT of `just check`/CI.
#
# EVERY ENTRY PRODUCES ALL FIVE COMPARED FORMATS, which is the property the
# corpus is selected for: the read benchmark's claim is a comparison BETWEEN
# formats, so a dataset producing four of them contributes a comparison with a
# hole in it. The `citygml` artefact is SYNTHESISED from the CityJSON by
# `readbench_prepare.sh` — see benchmark/formats/READ_BENCHMARK.md's CityGML
# synthesis section for what that costs.
#
# ONLY selects the entries that can serve one benchmark set: `default` (the
# DEFAULT, the default format set with the `citygml` row included),
# `no-citygml` (every format but citygml), or `all` (every pinned entry). For
# the pinned corpus all three select the same seven entries; the flag matters
# only for a $CORPUS_MANIFEST input that carries entries that cannot serve a
# default-set run.
#
# The fetch REFUSES to add to a DEST that already holds city-model files the
# table does not describe — most likely the previous corpus, which used this
# same directory (`--allow-foreign` overrides).
[doc("Fetch the read benchmark's seven-dataset corpus (about 1.2 GB, pinned)")]
fetch-data DEST=(BENCH / "runs/data/benchmark") ONLY='default':
    ./{{BENCH_SCRIPTS}}/fetch_benchmark.sh --only {{ONLY}} {{DEST}}

# Fetch the pinned external converters the read benchmark's conversion chain
# needs: citygml-tools (CityGML -> CityJSON) into benchmark/formats/tools/
# (gitignored, sha256-verified) and cjseq (CityJSON -> CityJSONSeq) via
# `cargo install`. Needs java 17+; network-dependent; kept OUT of `just
# check`/CI. The exact versions used are written to
# benchmark/formats/tools/tool_versions.txt for
# benchmark/formats/READ_BENCHMARK.md's Environment block.
[doc("Fetch the pinned external converters (citygml-tools, cjseq)")]
fetch-tools:
    ./{{BENCH_SCRIPTS}}/fetch_tools.sh

# Fetch the 3DBAG source — one 7.6 GB FlatCityBuf export of a 3DBAG subset
# (flatcitybuf.open3d.city, pinned byte size, resumable, cached under
# benchmark/runs/data/ and skipped once complete) — and cut the benchmark's
# 3DBAG dataset from it: DEST/3dbag_n<SIZE>.city.jsonl, the first SIZE
# CityObjects in source feature order. The manifest's one 3DBAG dataset is
# the default SIZE, 1000000; another SIZE cuts a smaller prefix of the same
# stream for trying the harness out, which no profile measures.
#
# The slice is cut at FEATURE boundaries (a CityJSONSeq feature is
# indivisible), so its actual CityObject count can slightly exceed its
# nominal SIZE — the `fcb-slice` binary prints the exact count. A SIZE the
# source cannot fill is an ERROR, not a silently short file.
#
# The slice is cut WITHOUT LoD 1.2 (`--drop-lod 1.2`): 3DBAG carries LoD
# 0, 1.2, 1.3 and 2.2, but CityGML 2.0 has integer LoDs only, so the
# synthesised CityGML could keep just one LoD-1 solid (citygml-tools keeps
# 1.3). Dropping 1.2 at the source gives all five formats the same content:
# LoD 0, 1.3 and 2.2. The vertices only LoD 1.2 used go with it, and every
# CityObject stays, so the count is unchanged.
#
# The slice carries no .gml of its own, but `readbench_prepare.sh`
# SYNTHESISES one with citygml-tools, exactly as it does for the corpus's
# .city.json entries. Budget for that: the 1,000,000-object slice is a
# stream of a few GB and its .gml roughly four times larger, and `citygml`
# is the slowest format in the matrix by an order of magnitude.
#
# Needs curl; network-dependent on the first run (~7.6 GB); kept
# OUT of `just check`/CI.
[doc("Fetch the 3DBAG source (7.6 GB) and cut the benchmark's 3DBAG slice")]
fetch-3dbag DEST=(BENCH / "runs/data/3dbag") SIZES='1000000':
    #!/usr/bin/env bash
    set -euo pipefail
    url='https://flatcitybuf.open3d.city/data/3dbag_subset2_all_index.fcb'
    data_root="${CITYPARQUET_BENCH_ROOT:-{{BENCH}}/runs}"
    src="$data_root/data/3dbag_subset2_all_index.fcb"
    expected=7587969439
    mkdir -p "$data_root/data" "{{DEST}}"
    actual=$(wc -c < "$src" 2>/dev/null || echo 0)
    if [[ "$actual" -ne "$expected" ]]; then
        curl -fL --retry 3 -C - -o "$src" "$url"
        actual=$(wc -c < "$src")
        if [[ "$actual" -ne "$expected" ]]; then
            echo "fetch-3dbag: $src is $actual bytes, expected $expected — delete it and re-run" >&2
            exit 1
        fi
    fi
    cargo run --release {{READBENCH_CARGO}} --bin fcb-slice -- \
        --input "$src" --out-dir "{{DEST}}" --stem 3dbag --sizes "{{SIZES}}" \
        --drop-lod 1.2

# ---------------------------------------------------------------------------
# Conversion
# ---------------------------------------------------------------------------

# Convert every CityGML/CityJSON/CityJSONSeq file found under FOLDER
# (recursive) into a CityParquet package under OUT (default out/cityparquet),
# one OUT/<name>/ package directory per input where <name> is the input's
# basename minus its known input extension (see KNOWN_INPUT_EXTENSIONS at the
# top of this file; core profile, and existing packages of the same name are
# overwritten).
[doc("Convert every city-model input under FOLDER into a CityParquet package")]
convert-all FOLDER OUT='out/cityparquet':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        dest="{{OUT}}/${name}"
        echo ">> ${f} -> ${dest}"
        cargo run --release {{CARGO}} -p cityparquet-cli --bin cityparquet -- convert \
            "$f" --output "$dest" --overwrite
        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "convert-all: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "convert-all: ${found} file(s) converted into {{OUT}}"

# ---------------------------------------------------------------------------
# Benchmarks
# ---------------------------------------------------------------------------

# Prepare the per-format artefacts for ONE input, WITHOUT measuring anything
# (`just bench-prep` runs exactly this for every selected input; `bench`
# only reads what it built). A thin wrapper over
# `benchmark/scripts/readbench_prepare.sh`, which owns the conversion
# chain, its per-format tool guards and its refusals.
#
# It exists because four of the coordinator's own error messages
# (benchmark/readbench/src/coordinator.rs) tell the
# operator to run `just readbench-prepare <input>` when an artefact is missing,
# and benchmark/formats/READ_BENCHMARK.md documents it as the per-dataset
# manual path — a recipe named by an error message has to be a recipe that
# exists. (It was dropped in 16880cf when the bench recipes were consolidated;
# those four strings were not.)
#
# FORMATS is a comma-separated list of format names (`Format::ALL`, see
# benchmark/readbench/src/format.rs); empty
# (the default) builds every artefact the script knows how to build. Needs
# whichever external tools the requested hop of the chain uses (`just
# fetch-tools` for citygml-tools + cjseq; `fcb`, `jq`);
# network-independent given already-fetched inputs and tools; kept OUT of
# `just check`/CI.
[private]
[doc("Build the per-format artefacts for ONE input, measuring nothing")]
readbench-prepare INPUT OUTDIR=(BENCH / "runs/data/readbench") FORMATS='':
    #!/usr/bin/env bash
    set -euo pipefail
    # `${a[@]+"${a[@]}"}`, never a bare `"${a[@]}"`: see the same note in
    # `bench` below — under `set -u` an EMPTY array is unbound in bash 3.2.
    args=()
    if [[ -n "{{FORMATS}}" ]]; then
        args=(--formats "{{FORMATS}}")
    fi
    ./{{BENCH_SCRIPTS}}/readbench_prepare.sh ${args[@]+"${args[@]}"} "{{INPUT}}" "{{OUTDIR}}"

# Cross-format READ benchmark (see benchmark/formats/READ_BENCHMARK.md): for
# every CityGML/CityJSON/CityJSONSeq file found under FOLDER (recursive), run
# the `cityparquet-readbench` coordinator across the whole (format x scenario)
# matrix into one OUT/<name>.csv, reading the artefacts `just bench-prep`
# built in PREPARED. Each OUT/<name>.csv is removed first so a re-run is
# always clean. Network-independent given prepared artefacts; kept OUT of
# `just check`/CI.
#
# FORMATS is a comma-separated format list (`Format::ALL`'s canonical names,
# benchmark/readbench/src/format.rs) handed to the coordinator; empty (the
# default) measures every format. It is APPENDED to the parameter list rather
# than inserted before OUT because `just` parameters are
# positional-with-defaults — inserting it would silently reinterpret every
# existing `just bench FOLDER OUT` call's second argument.
#
# REPEAT is the number of timed samples per cell, each cell's samples run
# back to back after one discarded warm-up. CELL_BUDGET_S (empty: off) stops
# a cell's sampling once its runs, warm-up included, have taken that many
# seconds and at least MIN_REPEAT samples exist; such a row carries the
# `budget` tag in `notes`.
[private]
[doc("Cross-format READ benchmark over every input under FOLDER")]
bench FOLDER OUT=(BENCH / "runs/formats/results") FORMATS='' PREPARED=(BENCH / "runs/data/readbench") REPEAT='25' CELL_BUDGET_S='' MIN_REPEAT='7':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}" "{{PREPARED}}"
    # `${a[@]+"${a[@]}"}`, never a bare `"${a[@]}"`: under `set -u` an EMPTY
    # array is an unbound variable to bash 4.3 and older (macOS still ships
    # 3.2 as /bin/bash), which would abort every default-FORMATS run.
    run_args=()
    if [[ -n "{{FORMATS}}" ]]; then
        run_args=(--formats "{{FORMATS}}")
    fi
    if [[ -n "{{CELL_BUDGET_S}}" ]]; then
        run_args+=(--cell-budget-s "{{CELL_BUDGET_S}}")
    fi
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        # Preparation belongs to `just bench-prep`; a run never turns a
        # missing artefact into an unrecorded conversion.
        if [[ ! -e "{{PREPARED}}/${name}.city.jsonl" ]]; then
            echo "bench: missing prepared artefacts for ${name}; run bench-prep first" >&2
            exit 1
        fi

        cargo run --release {{READBENCH_CARGO}} -- run \
            --input "$f" \
            --prepared-dir "{{PREPARED}}" \
            --out "$out" \
            --repeat {{REPEAT}} \
            --min-repeat {{MIN_REPEAT}} \
            ${run_args[@]+"${run_args[@]}"}

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "bench: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "bench: ${found} file(s) benchmarked into {{OUT}}"


# The configuration-axis runner behind `bloom-bench` and `bloom-bench-http`:
# for every CityJSON/CityJSONSeq file under FOLDER (recursive), build the
# `cityparquet` artefact the query parameters derive from (and the
# CityJSONSeq the variants are converted from), then run the coordinator's
# `--variants` path: per variant an untimed conversion, the package kept as
# `PREPARED/<name>.<variant>.parquet`, then the read SCENARIOS, with their
# ID_PROBES and FEATURE_PROBES, against it. The callers pass every one of
# these: no default stands in for a benchmark nobody chose, and
# benchmark/scripts/tests/bench_recipe_test.sh holds the recipe to that.
# One OUT/<name>.csv per input in the read run's exact
# CSV shape (the variant id in the `format` column), package bytes in
# OUT/sizes.csv, and the host in OUT/MACHINE.md.
# Each OUT/<name>.csv is removed first; OUT/sizes.csv is removed once at the
# start, and each input's run then appends its own rows. Network-independent
# given already-fetched inputs; multi-hour at the 1M-object slice; kept OUT
# of `just check`/CI.
# With BASE_URL the run is read-only over HTTP: it reads the variant
# packages a local run left in PREPARED, uploaded to BASE_URL.
#
# VARIANTS is the whole benchmark: the two recipes below pass their lists
# here and nowhere else, and benchmark/scripts/tests/bench_recipe_test.sh
# reads those lists back out of this file.
[private]
[doc("Configuration-axis run: reads and package size per variant, over every input under FOLDER")]
variant-bench FOLDER OUT VARIANTS PREPARED REPEAT CELL_BUDGET_S MIN_REPEAT SCENARIOS ID_PROBES FEATURE_PROBES BASE_URL='':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}" "{{PREPARED}}"
    rm -f "{{OUT}}/sizes.csv"
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        if [[ ! -e "{{PREPARED}}/${name}.city.jsonl" ]]; then
            echo "variant-bench: missing prepared artefacts for ${name}; run bench-prep first" >&2
            exit 1
        fi

        feature_args=()
        if [[ -n "{{FEATURE_PROBES}}" ]]; then
            feature_args=(--feature-probes "{{FEATURE_PROBES}}")
        fi
        budget_args=()
        if [[ -n "{{CELL_BUDGET_S}}" ]]; then
            budget_args=(--cell-budget-s "{{CELL_BUDGET_S}}")
        fi
        transport_args=()
        if [[ -n "{{BASE_URL}}" ]]; then
            transport_args=(--transport http --base-url "{{BASE_URL}}")
        fi
        cargo run --release {{READBENCH_CARGO}} -- run \
            --input "$f" \
            --prepared-dir "{{PREPARED}}" \
            --out "$out" \
            --repeat {{REPEAT}} \
            --min-repeat {{MIN_REPEAT}} \
            ${budget_args[@]+"${budget_args[@]}"} \
            --scenarios "{{SCENARIOS}}" \
            --id-probes "{{ID_PROBES}}" \
            ${feature_args[@]+"${feature_args[@]}"} \
            ${transport_args[@]+"${transport_args[@]}"} \
            --variants "{{VARIANTS}}"

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "variant-bench: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    ./{{BENCH_SCRIPTS}}/machine_record.sh > "{{OUT}}/MACHINE.md"
    echo "variant-bench: ${found} file(s) benchmarked into {{OUT}}"

# The BLOOM axis: the default package, which carries bloom filters, against
# the same package without them. Identifier lookups only — what the filters
# exist for — by `id` and by `feature_id`, each at the middle position and a
# verified miss. Every variant at the default codec and row-group size.
[private]
[doc("Bloom axis over every input under FOLDER: cityparquet vs cityparquet+nobloom")]
bloom-bench FOLDER OUT=(BENCH / "runs/formats/bloom_results") PREPARED=(BENCH / "runs/data/readbench") REPEAT='25' CELL_BUDGET_S='' MIN_REPEAT='7':
    just variant-bench "{{FOLDER}}" "{{OUT}}" "cityparquet,cityparquet+nobloom" "{{PREPARED}}" "{{REPEAT}}" "{{CELL_BUDGET_S}}" "{{MIN_REPEAT}}" "id-lookup,feature-lookup" "id-50pct,id-miss" "feature-50pct,feature-miss"

# The bloom axis over HTTP: reads (never builds) the two packages a local
# `bloom-bench` run left in PREPARED, after PREPARED was uploaded to BASE_URL
# (benchmark/scripts/readbench_upload.md). Not part of `bench-run`: it needs a
# real bucket, and its timings are a snapshot of one network path.
[private]
[doc("Bloom axis over HTTP, against uploaded bloom-bench packages")]
bloom-bench-http FOLDER BASE_URL OUT=(BENCH / "runs/formats/bloom_http_results") PREPARED=(BENCH / "runs/data/readbench") REPEAT='25' CELL_BUDGET_S='' MIN_REPEAT='7':
    just variant-bench "{{FOLDER}}" "{{OUT}}" "cityparquet,cityparquet+nobloom" "{{PREPARED}}" "{{REPEAT}}" "{{CELL_BUDGET_S}}" "{{MIN_REPEAT}}" "id-lookup,feature-lookup" "id-50pct,id-miss" "feature-50pct,feature-miss" "{{BASE_URL}}"

# ---------------------------------------------------------------------------
# The harness's own test suites
#
# Both are deliberately outside `cd lib/cityparquet-rs && just check` — the
# Rust workspace's gate — because each needs a tool that gate does not require
# of a machine: `plot-test` needs `uv`, `scripts-test` needs `jq`, `uv` and a
# bash new enough for its stubs. Run them alongside it when touching
# `benchmark/plot/` or `benchmark/scripts/`; `just check` at this level runs
# all three.
#
# The one convention that MUST NOT drift silently — the input-extension rule
# this justfile and those scripts each implement — is instead enforced from
# inside the Rust gate, by
# `benchmark/readbench/tests/strip_extension.rs`.
# ---------------------------------------------------------------------------

# `benchmark/plot`'s pytest suite (CSV-header contract + chart building). Needs
# `uv` on PATH; no network beyond uv's own dependency resolution.
#
# `--directory`, not `--project`: pytest's rootdir follows the working
# directory, so `--project benchmark/plot` alone leaves it at the repo root,
# where `testpaths = ["tests"]` no longer resolves and collection wanders into
# other projects' test trees (different deps) and errors out.
[doc("benchmark/plot's pytest suite (needs uv)")]
plot-test:
    uv run --directory {{PLOT}} --extra dev pytest -q

# The benchmark shell scripts' own test suites: plain bash, no framework, no
# network. `readbench_prepare_test.sh` stubs every external binary inside a
# throwaway sandbox (so it needs no real `fcb`/`cjseq`/`citygml-tools` and
# performs no real conversion); `fetch_benchmark_test.sh` serves a throwaway
# corpus of `file://` URLs to the real fetcher, and lints its pinned table
# against `benchmark/formats/corpus_urls.txt`; `bench_recipe_test.sh` extracts
# the variant lists and positional arguments the bloom recipes pass out of
# THIS file. The Python unit tests cover `bench_suite.py`'s dataset and
# profile selection against the real manifest, and `cityjson_merge.py` on a
# real fixture (`just fixtures` in lib/cityparquet-rs); they run in
# benchmark/plot's uv environment, as `bench-run` does, for its Python 3.11+
# (`tomllib`). Needs `jq`, `zip`/`unzip` and `uv`.
[doc("The benchmark scripts' own suites (needs jq and uv)")]
scripts-test:
    ./{{BENCH_SCRIPTS}}/tests/readbench_prepare_test.sh
    ./{{BENCH_SCRIPTS}}/tests/fetch_benchmark_test.sh
    ./{{BENCH_SCRIPTS}}/tests/bench_recipe_test.sh
    uv run --project {{PLOT}} python -m unittest discover -s {{BENCH_SCRIPTS}}/tests -p 'test_*.py'

# ---------------------------------------------------------------------------
# Database benchmark (benchmark/databases) — its own uv project and justfile
# ---------------------------------------------------------------------------

# The database comparison's own recipes, forwarded. `just db --list` shows
# them; see benchmark/databases/README.md for what it does and does not claim.
[doc("Forward a recipe to the database comparison's own justfile")]
db *ARGS:
    cd benchmark/databases && just {{ARGS}}

# ---------------------------------------------------------------------------
# STAC catalogue -> CityParquet mirror (scripts/catalog2cityparquet)
# ---------------------------------------------------------------------------

# Build the two release binaries the Python driver shells out to: the
# `cityparquet` converter (one package per catalogue item) and the vendored
# `city3dstac` aggregator (collection.json / items.parquet / catalog.json).
# The driver's own defaults point at exactly these two paths, so building them
# here is what makes the `catalog-*` recipes below runnable from a clean tree.
# Compiling only; kept OUT of `just check`, which builds and tests both trees
# anyway (see `vendor-check`).
catalog-tools:
    cargo build --release {{CARGO}} -p cityparquet-cli
    cargo build --release --manifest-path {{RS}}/vendor/city3d-stac-tool/Cargo.toml

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
    uv run --project scripts/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} {{ARGS}}

# Convert a single collection (e.g. `just catalog-convert-collection
# rotterdam-3d`), which is how a change to the driver or the converter is
# proven against real data without paying for the whole catalogue.
# Network-dependent; kept OUT of `just check`/CI.
catalog-convert-collection ID OUT='out/cityparquet-catalog' *ARGS: catalog-tools
    uv run --project scripts/catalog2cityparquet python -m catalog2cityparquet \
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
    uv run --project scripts/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} --aggregate-only

# Reduce a run's cumulative ledger (OUT/_reports) to one outcome per item and
# print the conformance histogram. This — not a hand roll-up of the JSONL — is
# how the published number is produced: the files are append-only and
# resumption re-attempts a previously FAILED item, so an item legitimately
# appears twice with two different outcomes and counting lines over-counts
# failures. Needs no network and no binaries.
catalog-histogram OUT='out/cityparquet-catalog':
    uv run --project scripts/catalog2cityparquet python -m catalog2cityparquet \
        histogram {{OUT}}/_reports

# The driver's own test suite. No network and no binaries: every origin,
# subprocess and catalogue document is faked, so this is safe to run anywhere.
# Not part of `just check`, which is the Rust workspace's gate — run both.
# `--directory`, not `--project`: pytest's rootdir follows the working
# directory, so `--project scripts/catalog2cityparquet` alone would leave it at
# the repo root and collection would wander into other projects' test trees.
# That made this recipe exit 2 on a healthy tree.
catalog-test:
    uv run --directory scripts/catalog2cityparquet --extra dev pytest -v

# Regenerate the MCP server's documentation corpus from documents/docs and the
# two extension function references. Needs the submodules — `just setup` first.
mcp-corpus:
    cd ai/mcp && pnpm install --frozen-lockfile && pnpm corpus

# The MCP server's gate: typecheck, tests, and a freshness check on the
# corpus. The freshness check compares corpus content only, never the
# `generatedFrom` provenance stamp (which is a `git describe` and so changes
# on every commit) — see ai/mcp/src/check-corpus-fresh.ts. It builds the
# comparison in memory and never writes corpus/corpus.json, so this leaves the
# working tree clean whether it passes or fails. Needs the submodules, for the
# same reason mcp-corpus does.
mcp-check:
    cd ai/mcp && pnpm install --frozen-lockfile && pnpm typecheck && pnpm test
    cd ai/mcp && pnpm corpus:check

# Lint and test the CityLake crate.
#
# The integration tests need the CityParquet package pragmas, which the
# published community extension does not yet carry — so they run against the
# local build, and `just -f lib/duckdb-cityjson/justfile build` must have run
# first. Override the path by exporting CITYLAKE_CITYJSON_EXTENSION yourself.
citylake-check:
    #!/usr/bin/env bash
    set -euo pipefail
    ext="{{justfile_directory()}}/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension"
    if [ -z "${CITYLAKE_CITYJSON_EXTENSION:-}" ] && [ -f "$ext" ]; then
        export CITYLAKE_CITYJSON_EXTENSION="$ext"
    fi
    cd lib/citylake
    cargo clippy --all-targets -- -D warnings
    cargo test

# ── usecase ──────────────────────────────────────────────────────────

# run the energy tool's test suite
usecase-energy-test:
    cd usecase/energy && uv run pytest

# extract features from a CityParquet package: just usecase-energy-features IN OUT
usecase-energy-features input output="features.parquet":
    cd usecase/energy && uv run energy features --input '{{input}}' --output '{{output}}'

# The concise public benchmark interface. The selector validates families and
# datasets before delegating to the measured low-level recipes above. It runs
# under `uv` in benchmark/plot's project (Python 3.11 or later, for
# `tomllib`), so it does not depend on the system `python3`. `--project`, not
# `--directory`: relative paths such as `--data-root benchmark/runs` stay
# relative to the repository root.
[doc("Fetch and prepare selected benchmark inputs without measuring")]
[positional-arguments]
bench-prep *ARGS:
    uv run --project {{PLOT}} python benchmark/scripts/bench_suite.py prep "$@"

[doc("Run selected benchmark families; --smoke keeps validation outputs separate")]
[positional-arguments]
bench-run *ARGS:
    uv run --project {{PLOT}} python benchmark/scripts/bench_suite.py run "$@"

[doc("Render paper figures and one combined HTML page from existing results; --statistic median|mean (default median)")]
[positional-arguments]
bench-summary *ARGS:
    uv run --project {{PLOT}} python benchmark/scripts/bench_suite.py summary "$@"
