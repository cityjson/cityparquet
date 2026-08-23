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
# The four per-dataset recipes (`convert-all`, `bench`, `write-bench`,
# `compression-bench`) are deliberately in ONE file: they share the
# input-extension convention below verbatim, and
# `benchmark/readbench/tests/strip_extension.rs` extracts all four
# out of this file and RUNS them to prove they have not drifted apart. Split
# them across two justfiles and that check has nothing to compare.
# ===========================================================================

RS := "lib/cityparquet-rs"
BENCH := "benchmark/formats"
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

# Everything that gates a change to the Rust library and the benchmark
# harness. The library and the harness are separate Cargo workspaces, so each
# needs its own clippy/test/fmt pass — `cd lib/cityparquet-rs && just check`
# deliberately does NOT reach the harness, which is what keeps that gate
# runnable with no `uv`, no `jq` and no corpus.
[doc("The full gate: both Rust workspaces, the plotting suite, the shell suites")]
check:
    cd {{RS}} && just check
    cargo clippy {{READBENCH_CARGO}} --all-targets -- -D warnings
    cargo test {{READBENCH_CARGO}}
    cargo fmt {{READBENCH_CARGO}} --check
    just plot-test
    just scripts-test

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
# artefact). The rule is implemented four times over — here, in
# `benchmark/readbench/src/naming.rs`, in
# `benchmark/scripts/readbench_prepare.sh`, and (as its composable
# package-name counterpart) in `benchmark/scripts/readbench_duckdb.sh`
# — because a shell script cannot import a Rust function and `just` has no
# functions of its own.
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

# Fetch the CityParquet benchmark corpus — SIX REAL published city models
# (CityJSON 2.0 `.city.json`, 2.7 MB .. 293 MB, 423 MB on the wire) from the
# CityJSON project's own dataset page, into DEST (default
# benchmark/formats/data/benchmark/, gitignored). Every entry's byte size is
# pinned and verified and an already-present file is skipped — see
# benchmark/scripts/fetch_benchmark.sh for the table and
# benchmark/formats/corpus_urls.txt for each URL's provenance. Needs curl;
# network-dependent; kept OUT of `just check`/CI.
#
# EVERY ENTRY PRODUCES ALL EIGHT COMPARED FORMATS, which is the property the
# corpus is selected for: the read benchmark's claim is a comparison BETWEEN
# formats, so a dataset producing seven of them contributes a comparison with
# the baseline missing. The `citygml` artefact is SYNTHESISED from the CityJSON
# by `readbench_prepare.sh` — see benchmark/formats/READ_BENCHMARK.md's CityGML
# synthesis section for what that costs. The 30-dataset catalogue corpus this
# replaced is archived, still fetchable, under
# benchmark/formats/archive/2026-08-17-catalogue-corpus/.
#
# ONLY selects the entries that can serve one benchmark set: `default` (the
# DEFAULT, the default format set with the `citygml` row included),
# `no-citygml` (every format but citygml), or `all` (every pinned entry). For
# the pinned corpus all three select the same six entries; the flag matters
# only for a $CORPUS_MANIFEST input, such as the archived corpus, which does
# carry entries that cannot serve a default-set run.
#
# The fetch REFUSES to add to a DEST that already holds city-model files the
# table does not describe — most likely the previous corpus, which used this
# same directory (`--allow-foreign` overrides).
[doc("Fetch the read benchmark's six-dataset corpus (423 MB, pinned)")]
fetch-data DEST=(BENCH / "data/benchmark") ONLY='default':
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

# Fetch the SCALING corpus source — one 7.6 GB FlatCityBuf export of a
# 3DBAG subset (flatcitybuf.open3d.city, pinned byte size, resumable,
# cached under benchmark/formats/data/ and skipped once complete) — and cut
# CityJSONSeq prefixes with a fixed number of CityObjects each: one
# DEST/3dbag_n<SIZE>.city.jsonl per SIZE, every slice a strict prefix of
# the next larger one, in source feature order. This is the input for the
# CONFIGURATION-axis benchmarks (`compression-bench`, `write-bench`,
# `ordering-bench`): one dataset at several cardinalities shows the trend
# over size with the data held constant, where a corpus of unrelated city
# models would entangle every configuration delta with a data delta.
#
# Slices cut at FEATURE boundaries (a CityJSONSeq feature is indivisible),
# so a slice's actual CityObject count can slightly exceed its nominal
# SIZE — the `scaling-corpus` binary prints the exact counts per slice. A
# SIZE the source cannot fill is an ERROR, not a silently short file. No
# .gml source exists for these slices, so pointing `bench` at DEST skips
# the `citygml` row with a warning, exactly like the corpus's .city.json
# entries. Needs curl; network-dependent on the first run (~7.6 GB); kept
# OUT of `just check`/CI.
[doc("Fetch and slice the configuration-axis corpus (7.6 GB source)")]
fetch-scaling-data DEST=(BENCH / "data/scaling") SIZES='1000,5000,10000,50000':
    #!/usr/bin/env bash
    set -euo pipefail
    url='https://flatcitybuf.open3d.city/data/3dbag_subset2_all_index.fcb'
    src='{{BENCH}}/data/3dbag_subset2_all_index.fcb'
    expected=7587969439
    mkdir -p '{{BENCH}}/data'
    actual=$(wc -c < "$src" 2>/dev/null || echo 0)
    if [[ "$actual" -ne "$expected" ]]; then
        curl -fL --retry 3 -C - -o "$src" "$url"
        actual=$(wc -c < "$src")
        if [[ "$actual" -ne "$expected" ]]; then
            echo "fetch-scaling-data: $src is $actual bytes, expected $expected — delete it and re-run" >&2
            exit 1
        fi
    fi
    cargo run --release {{READBENCH_CARGO}} --bin scaling-corpus -- \
        --input "$src" --out-dir "{{DEST}}" --stem 3dbag --sizes "{{SIZES}}"

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
# (`just bench FOLDER` runs exactly this as its first step, for every input
# under FOLDER). A thin wrapper over
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
# FORMATS is a comma-separated list of artefact-BEARING format names
# (`Format::ALL` minus `duckdb-parquet`, which has no artefact of its own —
# see benchmark/readbench/src/format.rs); empty
# (the default) builds every artefact the script knows how to build. Needs
# whichever external tools the requested hop of the chain uses (`just
# fetch-tools` for citygml-tools + cjseq; `fcb`, `jq`, `gzip`);
# network-independent given already-fetched inputs and tools; kept OUT of
# `just check`/CI.
[doc("Build the per-format artefacts for ONE input, measuring nothing")]
readbench-prepare INPUT OUTDIR=(BENCH / "data/readbench") FORMATS='':
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
# every CityGML/CityJSON/CityJSONSeq file found under FOLDER (recursive),
# prepare every compared format
# (`benchmark/scripts/readbench_prepare.sh`), then run the
# `cityparquet-readbench` coordinator across the whole (format x scenario)
# matrix into one OUT/<name>.csv. Each OUT/<name>.csv is removed first so a
# re-run is always clean. Once every dataset is done, renders charts from the
# CSVs via the `plot` recipe (best-effort: a missing `uv`/plotting setup
# doesn't fail the benchmark run, only skips the charts). Needs `fcb` on PATH
# (and `duckdb` only for the opt-in baseline below); network-independent given
# already-fetched inputs; kept OUT of `just check`/CI.
#
# FORMATS is a comma-separated format list (`Format::ALL`'s canonical names,
# benchmark/readbench/src/format.rs) threaded to
# BOTH the prepare script and the coordinator, so exactly the requested
# artefacts are built and exactly they are measured. Empty (the default) means:
# prepare every artefact, measure `Format::DEFAULT_SET` — the five-tag
# FORMAT-comparison set, one tag per format family. It is APPENDED to the
# parameter list rather than inserted before OUT because `just` parameters are
# positional-with-defaults — inserting it would silently reinterpret every
# existing `just bench FOLDER OUT` call's second argument.
#
# THE `duckdb-parquet` BASELINE IS OPT-IN. It is appended to the same CSV
# (`benchmark/scripts/readbench_duckdb.sh`, auto-detecting a numeric
# attribute column via a `DESCRIBE` query where possible — omitted, skipping
# attr-stats, if none is found) ONLY when `duckdb-parquet` is named in FORMATS.
# It is an SQL-ENGINE baseline over a file already in the set, not a format, so
# a run labelled "format comparison" must not carry it unasked:
# `Format::DEFAULT_SET` excludes it, and this recipe now agrees rather than
# quietly adding a sixth, non-format series to a CSV that
# benchmark/formats/READ_BENCHMARK.md documents as holding five.
# `benchmark/scripts/tests/bench_recipe_test.sh` pins that both ways —
# a bare run must not append it, naming it must.
[doc("Cross-format READ benchmark over every input under FOLDER")]
bench FOLDER OUT=(BENCH / "read_results") FORMATS='':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}" '{{BENCH}}/data/readbench'
    # FORMATS reaches two consumers that do NOT accept the same vocabulary:
    #   - the coordinator takes the list verbatim (it knows every
    #     `Format::ALL` name, `duckdb-parquet` included, and reports the ones
    #     it does not itself drive);
    #   - `readbench_prepare.sh` builds ARTEFACTS, so it rejects
    #     `duckdb-parquet` outright (that baseline has no artefact of its
    #     own), and must always be asked for `cityparquet` whatever was
    #     requested: the coordinator derives EVERY query parameter — bbox
    #     windows, the id, the attribute predicate — from that one package.
    # Naming `duckdb-parquet` is also the ONLY thing that appends the
    # SQL-engine baseline below (see the header): a deliberately single-axis
    # run — the default format comparison, or `ordering-bench` — must not have
    # an extra series quietly added to its CSV.
    # BEGIN format-selection (extracted and RUN by
    # benchmark/scripts/tests/bench_recipe_test.sh — keep both markers
    # in column 5, and keep this block free of anything the test cannot
    # evaluate standalone)
    prepare_formats=""
    want_duckdb=0
    if [[ -n "{{FORMATS}}" ]]; then
        IFS=',' read -r -a requested <<<"{{FORMATS}}"
        for fmt in "${requested[@]}"; do
            if [[ "$fmt" == "duckdb-parquet" ]]; then
                want_duckdb=1
            else
                prepare_formats+="${prepare_formats:+,}$fmt"
            fi
        done
        case ",$prepare_formats," in
            *,cityparquet,*) ;;
            *) prepare_formats="cityparquet${prepare_formats:+,$prepare_formats}" ;;
        esac
    fi
    # END format-selection
    # `${a[@]+"${a[@]}"}`, never a bare `"${a[@]}"`: under `set -u` an EMPTY
    # array is an unbound variable to bash 4.3 and older (macOS still ships
    # 3.2 as /bin/bash), which would abort every default-FORMATS run.
    prepare_args=()
    run_args=()
    if [[ -n "$prepare_formats" ]]; then
        prepare_args=(--formats "$prepare_formats")
    fi
    if [[ -n "{{FORMATS}}" ]]; then
        run_args=(--formats "{{FORMATS}}")
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

        ./{{BENCH_SCRIPTS}}/readbench_prepare.sh ${prepare_args[@]+"${prepare_args[@]}"} \
            "$f" '{{BENCH}}/data/readbench'

        cargo run --release {{READBENCH_CARGO}} -- run \
            --input "$f" \
            --prepared-dir '{{BENCH}}/data/readbench' \
            --out "$out" \
            --repeat 7 \
            ${run_args[@]+"${run_args[@]}"}

        if [[ "$want_duckdb" -eq 1 ]]; then
            pkg="{{BENCH}}/data/readbench/${name}.parquet"
            # By-type is the only, mandatory table layout: resolve the
            # package's single main table from its own metadata.json STAC
            # Item (the `cityparquet-objects` asset role) rather than
            # assuming the pre-by-type "cityobjects.parquet" name.
            # `package_tables.py --single` succeeds only for a single-family
            # dataset; an empty `main_table` here just skips the optional
            # attr-stats column detection, and `readbench_duckdb.sh` below
            # still hard-fails clearly for a multi-family/multi-table package.
            main_table="$(./{{BENCH_SCRIPTS}}/package_tables.py "$pkg" --single 2>/dev/null || true)"
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
                ./{{BENCH_SCRIPTS}}/readbench_duckdb.sh "$pkg" "$out" --numeric-column "$numeric_col" --repeat 7
            else
                echo "-- no numeric attribute column detected; skipping attr-stats for duckdb-parquet"
                ./{{BENCH_SCRIPTS}}/readbench_duckdb.sh "$pkg" "$out" --repeat 7
            fi
        else
            echo "-- duckdb-parquet not requested; the SQL-engine baseline is not appended"
        fi

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "bench: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "bench: ${found} file(s) benchmarked into {{OUT}}"

    # Best-effort: a missing `uv` must not fail a benchmark that already
    # produced its CSVs. `sizes` additionally reports nothing for a run whose
    # FORMATS built no `.fcb`/`.jsonl.gz` (it discovers datasets by those two
    # artefacts) — an ordering-only run, for instance — which is a skip, not
    # a failure, so the message names both causes rather than blaming `uv`.
    just plot "{{OUT}}" || echo "plot skipped (needs uv)"
    just sizes '{{BENCH}}/data/readbench' "{{OUT}}" \
        || echo "sizes skipped (needs uv, and a run that built the .fcb/.jsonl.gz artefacts it sizes)"

# The ORDERING-COMPARISON run: the same benchmark, restricted to
# `Format::ORDERING_SET`
# (benchmark/readbench/src/format.rs) — a
# source-order CityParquet package and a Hilbert-ordered one, same writer,
# same reader, same scenarios, so the ONLY variable is the row order.
#
# A separate OUT default (benchmark/formats/ordering_results) rather than a
# shared one: `plot` charts a whole directory, so mixing an ordering run's CSVs
# in with the format comparison's would put two axes on one chart and answer
# neither question. `duckdb-parquet` is deliberately absent from the list,
# which is what keeps `bench` from appending the SQL-engine baseline here.
#
# It DELEGATES to `bench` rather than copying its body — a forked recipe is
# how the two would drift apart.
[doc("The same run restricted to the ordering axis (source order vs Hilbert)")]
ordering-bench FOLDER OUT=(BENCH / "ordering_results"):
    just bench "{{FOLDER}}" "{{OUT}}" "cityparquet,cityparquet-hilbert"

# Encoding-variant WRITE benchmark (M5): for every CityJSON/CityJSONSeq file
# found under FOLDER (recursive), run the `cityparquet bench` variant matrix
# and append the DuckDB `COPY` baseline into one OUT/<name>.csv. Each
# OUT/<name>.csv is removed first so a re-run is always clean.
# Network-dependent (the DuckDB baseline installs the `cityjson` community
# extension); kept OUT of `just check`/CI.
[doc("Encoding-variant WRITE benchmark plus the DuckDB COPY baseline")]
write-bench FOLDER OUT=(BENCH / "results"):
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        cargo run --release {{CARGO}} -p cityparquet-cli --bin cityparquet -- bench \
            --input "$f" --out "$out"
        ./{{BENCH_SCRIPTS}}/bench_duckdb.sh "$f" "$out"

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "write-bench: no city-model inputs found under {{FOLDER}}" >&2
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
[doc("Codec and row-group WRITE-bench matrix, plus charts")]
compression-bench FOLDER OUT=(BENCH / "compression_results"):
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        cargo run --release {{CARGO}} -p cityparquet-cli --bin cityparquet -- bench \
            --input "$f" --out "$out" \
            --variants "cityparquet,cityparquet+uncompressed,cityparquet+snappy,cityparquet+gzip,cityparquet+lz4,cityparquet+brotli,cityparquet+rg512,cityparquet+rg4096"

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "compression-bench: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    echo "compression-bench: ${found} file(s) benchmarked into {{OUT}}"

    just compression-plot "{{OUT}}" || echo "plot skipped"

# ---------------------------------------------------------------------------
# Rendering — reads CSVs, measures nothing
# ---------------------------------------------------------------------------

# Render compression-codec and row-group comparison charts from the
# compression-bench CSVs in RESULTS (default
# benchmark/formats/compression_results) via the `benchmark/plot` uv project:
# per dataset, two codec-axis charts (<name>-codec-size.png,
# <name>-codec-time.png) and one row-group-axis chart (<name>-rowgroup.png),
# under RESULTS/plots/. Needs `uv` on PATH.
[doc("Render codec and row-group charts from compression-bench CSVs")]
compression-plot RESULTS=(BENCH / "compression_results"):
    uv run --project {{PLOT}} python -m readbench_plot.compression {{RESULTS}}

# Render charts from the read-benchmark CSVs in RESULTS (default
# benchmark/formats/read_results) via the `benchmark/plot` uv project: a
# grouped bar chart of median time_s and one of peak_heap_bytes per scenario x
# format, one PNG pair per dataset CSV, under RESULTS/plots/. Needs `uv` on
# PATH.
[doc("Render per-dataset charts from read-benchmark CSVs")]
plot RESULTS=(BENCH / "read_results"):
    uv run --project {{PLOT}} python -m readbench_plot {{RESULTS}}

# The CROSS-DATASET view of the SAME already-measured CSVs: one self-contained
# `bench-summary.html` (every dataset and every format on one page, as ratios
# against the CityJSONSeq baseline, with the fairness caveats quoted verbatim
# from benchmark/formats/READ_BENCHMARK.md at generation time) plus the static
# print figures the paper embeds. Written under OUT (default
# benchmark/summary/, gitignored).
#
# Measures nothing — it reads what `bench`, `compression-bench` and `sizes` left
# behind, so it is the recipe to run after a benchmark, or after editing the
# renderers, and re-running it is free. `plot` (above) stays the per-dataset
# view of one run; this is the comparison across runs' datasets. Needs `uv`.
#
# The FIGURES STEP IS ALLOWED TO FAIL and says why: those five figures are
# hand-laid print sheets with room for 11 dataset panels, and each asserts its
# finding in a written caption tied to the corpus it was drawn from. A corpus
# that outgrows the grid needs a layout decision and revised captions, not a
# silent stretch — the HTML page has no such limit and still covers everything.
# See benchmark/plot/benchviz/DESIGN.md for the data contract and honesty rules.
[doc("Render the cross-dataset summary page and the print figures")]
plot-pretty OUT='benchmark/summary':
    uv run --project {{PLOT}} python -m benchviz prep --out {{OUT}}
    uv run --project {{PLOT}} python -m benchviz html --out {{OUT}}
    uv run --project {{PLOT}} python -m benchviz figures --out {{OUT}} \
        || echo "figures skipped (see the message above; the HTML page is written)"

# Render the file-size / compression-ratio report from PREPARED_DIR (default
# benchmark/formats/data/readbench, the same per-format artefacts
# `readbench_prepare.sh` populates): OUT/sizes.csv (dataset, format, bytes, mb,
# ratio_vs_cityjsonseq, baseline_format, ratio_vs_baseline) plus two grouped
# bar charts under OUT/plots/ (sizes.png, compression-ratio.png). Needs `uv`
# on PATH.
#
# TWO ratio columns, not one: `ratio_vs_cityjsonseq` keeps its literal
# meaning and is EMPTY for a CityGML-native dataset that no run ever cut a
# raw CityJSONSeq from, while (`baseline_format`, `ratio_vs_baseline`) is
# always populated and self-describing — `baseline_format` names the
# denominator actually used (raw CityJSONSeq where one exists, otherwise the
# least-processed form the dataset exists in). The chart plots
# `ratio_vs_baseline`. See benchmark/plot/readbench_plot/sizes.py's own module
# doc for why answering "how much smaller than raw CityJSONSeq?" with a
# CityGML size would be a lie in a measurement artefact.
[doc("Render the file-size / compression-ratio report from the prepared artefacts")]
sizes PREPARED_DIR=(BENCH / "data/readbench") OUT=(BENCH / "read_results"):
    uv run --project {{PLOT}} python -m readbench_plot.sizes {{PREPARED_DIR}} {{OUT}}

# ---------------------------------------------------------------------------
# The harness's own test suites
#
# Both are deliberately outside `cd lib/cityparquet-rs && just check` — the
# Rust workspace's gate — because each needs a tool that gate does not require
# of a machine: `plot-test` needs `uv`, `scripts-test` needs `jq` and a bash
# new enough for its stubs. Run them alongside it when touching
# `benchmark/plot/` or `lib/cityparquet-rs/scripts/`; `just check` at this
# level runs all three.
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
# the `bench` recipe's own format-selection block out of THIS file and runs it,
# which is what keeps the recipe and `Format::DEFAULT_SET` from disagreeing
# about whether the `duckdb-parquet` baseline is opt-in. Needs `jq`,
# `zip`/`unzip`.
[doc("The benchmark shell scripts' own suites (needs jq)")]
scripts-test:
    ./{{BENCH_SCRIPTS}}/tests/readbench_prepare_test.sh
    ./{{BENCH_SCRIPTS}}/tests/fetch_benchmark_test.sh
    ./{{BENCH_SCRIPTS}}/tests/bench_recipe_test.sh

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
