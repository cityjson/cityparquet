#!/usr/bin/env python3
"""Select and orchestrate the CityParquet benchmark suite.

The repository contains code and the dataset manifest. Prepared inputs, raw
results, and rendered output belong beneath the operator-owned data root.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MANIFEST = REPO / "benchmark" / "manifest.toml"
FAMILIES = ("sizes", "formats", "codec", "rowgroup", "bloom", "databases")
DEFAULT_DATA_ROOT = REPO / "benchmark" / "runs"
# Relative spread below which the database family's cross-system count
# check publishes an EXPLAINED deviation (status=ok-deviation, with the
# decomposition kept in `notes`) instead of failing the run. See
# `benchmark/databases/README.md`, "Count cross-check", and
# `citybench.runner.DEFAULT_COUNT_TOLERANCE`.
DATABASE_COUNT_TOLERANCE = 0.001


def load_manifest() -> dict:
    with MANIFEST.open("rb") as stream:
        return tomllib.load(stream)


def values(text: str) -> list[str]:
    return [value.strip() for value in text.split(",") if value.strip()]


def family_selection(text: str) -> list[str]:
    selected = list(FAMILIES) if text == "all" else values(text)
    unknown = sorted(set(selected) - set(FAMILIES))
    if unknown:
        raise SystemExit(f"unknown benchmark family: {', '.join(unknown)}")
    return list(dict.fromkeys(selected))


def dataset_selection(manifest: dict, families: list[str], requested: str, smoke: bool) -> list[str]:
    datasets = manifest["datasets"]
    if requested:
        result: list[str] = []
        for item in values(requested):
            if item == "3dbag":
                result.extend(
                    key for key, entry in datasets.items()
                    if entry["role"] in {"scaling", "largest-scaling"}
                    and (not smoke or entry.get("target_objects") == 1000)
                )
            elif item in {"largest", "largest-3dbag"}:
                result.append(manifest["suite"]["largest_scaling_dataset"])
            else:
                result.append(item)
    elif smoke:
        result = ["rotterdam", "3dbag_n1000"]
    else:
        result = []
        if any(family in {"sizes", "formats"} for family in families):
            result.extend(key for key, entry in datasets.items() if entry["role"] == "corpus")
            result.append(manifest["suite"]["largest_scaling_dataset"])
        if any(family in {"codec", "rowgroup"} for family in families):
            result.extend(key for key, entry in datasets.items() if entry["role"] in {"scaling", "largest-scaling"})
        if "bloom" in families:
            result.extend(key for key, entry in datasets.items() if entry["role"] == "corpus")
            result.extend(key for key, entry in datasets.items() if entry["role"] in {"scaling", "largest-scaling"})
        if "databases" in families:
            result.append(manifest["suite"]["largest_scaling_dataset"])
    unknown = sorted(set(result) - set(datasets))
    if unknown:
        raise SystemExit(f"unknown benchmark dataset: {', '.join(unknown)}")
    return list(dict.fromkeys(result))


def command(*args: str) -> None:
    print("+", " ".join(args), flush=True)
    subprocess.run(args, cwd=REPO, check=True)


def just(*args: str) -> None:
    command(os.environ.get("JUST_CMD", "just"), *args)


def paths(root: Path) -> dict[str, Path]:
    return {
        "corpus": root / "data" / "benchmark",
        "scaling": root / "data" / "scaling",
        "prepared": root / "data" / "readbench",
        "formats": root / "formats",
        "databases": root / "databases",
        "summary": root / "summary",
        "work": root / "work",
    }


def source(entry: dict, locations: dict[str, Path]) -> Path:
    location = locations["corpus"] if entry["role"] == "corpus" else locations["scaling"]
    return location / entry["source"]


def dataset_stem(path: Path) -> str:
    for suffix in (".city.jsonl", ".city.json", ".citygml", ".jsonl", ".json", ".gml", ".xml"):
        if path.name.endswith(suffix):
            return path.name[: -len(suffix)]
    return path.stem


def renderer_dataset_ids(manifest: dict, selected: list[str]) -> list[str]:
    """Use the source identifiers retained in raw result CSV rows."""
    return [dataset_stem(Path(manifest["datasets"][key]["source"])) for key in selected]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def version(*args: str) -> str:
    try:
        return subprocess.check_output(args, cwd=REPO, text=True, stderr=subprocess.STDOUT).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"


def code_identity() -> dict[str, object]:
    """Identify tracked and relevant untracked source used for this run."""
    tracked_diff = subprocess.check_output(["git", "diff", "HEAD", "--binary"], cwd=REPO)
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard"], cwd=REPO, text=True
    ).splitlines()
    ignored_parts = {"target", "data", "results", "summary", "work", ".venv", "__pycache__", "node_modules"}
    source_roots = ("benchmark/", "lib/cityparquet-rs/", "justfile")
    source_files: dict[str, str] = {}
    for relative in untracked:
        candidate = REPO / relative
        if not relative.startswith(source_roots) or ignored_parts.intersection(candidate.parts):
            continue
        if candidate.is_file():
            source_files[relative] = sha256(candidate)
    return {
        "git_head": version("git", "rev-parse", "HEAD"),
        # `git diff HEAD` covers both staged and unstaged tracked changes.
        "tracked_diff_sha256": hashlib.sha256(tracked_diff).hexdigest(),
        "untracked_sources_sha256": source_files,
    }


def write_run_manifest(source_path: Path, result_csv: Path, *, family: str, repeat: int, write_repeat: int | None, smoke: bool, fixed_configuration: str) -> None:
    """Persist enough local evidence to identify one benchmark result exactly."""
    artefacts = [
        result_csv,
        Path(f"{result_csv}.samples.json"),
        Path(f"{result_csv}.params.json"),
        # One immutable raw write table belongs to this aggregate.  A shared
        # table would change after later datasets and invalidate this manifest.
        result_csv.with_suffix(".write.samples.csv"),
    ]
    files = {str(path.name): sha256(path) for path in artefacts if path.is_file()}
    params = Path(f"{result_csv}.params.json")
    manifest = {
        "schema": 1,
        "family": family,
        "smoke": smoke,
        "source": {"path": str(source_path.resolve()), "sha256": sha256(source_path)},
        "result": {"csv": str(result_csv.resolve()), "files_sha256": files, "params_sha256": sha256(params) if params.is_file() else None},
        "measurement": {"read_repeat": repeat, "write_repeat": write_repeat, "fixed_configuration": fixed_configuration},
        "code": code_identity(),
        "machine": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "processor": platform.processor(), "python": sys.version.split()[0]},
        "tools": {"rust": version("rustc", "--version"), "fcb": version("fcb", "--version"), "cjseq": version("cjseq", "--version"), "cityparquet": version("lib/cityparquet-rs/target/release/cityparquet", "--version")},
    }
    target = result_csv.with_suffix(".run.json")
    temporary = target.with_suffix(".run.json.tmp")
    temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    temporary.replace(target)


def prepare(manifest: dict, locations: dict[str, Path], families: list[str], datasets: list[str], smoke: bool) -> None:
    selected = [manifest["datasets"][key] for key in datasets]
    if any(entry["role"] == "corpus" for entry in selected):
        just("fetch-data", str(locations["corpus"]))
    if any(entry["role"] != "corpus" for entry in selected):
        # Generate only selected strict prefixes. Existing larger/smaller slices
        # are untouched, making a smoke preparation cheap and idempotent.
        sizes = ",".join(str(entry["target_objects"]) for entry in selected if entry["role"] != "corpus")
        just("fetch-scaling-data", str(locations["scaling"]), sizes)
    # Format and size comparisons need all artefacts for the large 3DBAG
    # replacement; configuration families only need CityParquet + CityJSONSeq.
    if any(family in {"sizes", "formats"} for family in families):
        just("fetch-tools")
    if any(family in {"sizes", "formats"} for family in families):
        command("cargo", "build", "--release", "--manifest-path", "lib/cityparquet-rs/Cargo.toml", "-p", "cityparquet-cli", "--bin", "cityparquet")
        # `format_write.py`'s CityJSONSeq writer is a subcommand of the read
        # harness, which lives in its own workspace — a second manifest, a
        # second target directory. It is the write row every ratio divides by,
        # so building it belongs in prep beside the converter, not in the
        # timed run.
        command("cargo", "build", "--release", "--manifest-path", "benchmark/readbench/Cargo.toml", "--bin", "cityparquet-readbench")
    locations["prepared"].mkdir(parents=True, exist_ok=True)
    for key in datasets:
        entry = manifest["datasets"][key]
        input_path = source(entry, locations)
        if not input_path.is_file():
            raise SystemExit(f"prepared source missing after fetch: {input_path}")
        # Full five-format artefacts are needed only by the corpus/large-format
        # comparison.  Intermediate configuration slices need CityParquet;
        # smoke runs retain all formats for their deliberately small sample.
        full_formats = (
            any(family in {"sizes", "formats"} for family in families)
            and (smoke or entry["role"] in {"corpus", "largest-scaling"})
        )
        just("readbench-prepare", str(input_path), str(locations["prepared"]), "" if full_formats else "cityparquet,cityjsonseq")


    if "databases" in families:
        root = locations["formats"].parent
        command("uv", "run", "--project", "benchmark/databases", "python", "-m", "citybench.cli", "prep", "--data-root", str(root), "--prepared-dir", str(locations["databases"] / "prepared"))


def stage(locations: dict[str, Path], name: str, inputs: list[Path]) -> Path:
    directory = locations["work"] / name
    directory.mkdir(parents=True, exist_ok=True)
    wanted = {input_path.name for input_path in inputs}
    for input_path in inputs:
        link = directory / input_path.name
        if link.exists() or link.is_symlink():
            link.unlink()
        # A hard link is discoverable by the existing low-level `find` recipes
        # and does not duplicate a multi-gigabyte scaling input.
        link.hardlink_to(input_path)
    for existing in directory.iterdir():
        if existing.name not in wanted:
            existing.unlink()
    return directory


def result_dir(locations: dict[str, Path], family: str, smoke: bool) -> Path:
    root = locations["formats"] / "smoke" if smoke else locations["formats"]
    names = {"formats": "results", "sizes": "results", "codec": "scaling_codec_results", "rowgroup": "scaling_rowgroup_results", "bloom": "scaling_bloom_results"}
    return root / names[family]


def require_prepared(inputs: list[Path], locations: dict[str, Path]) -> None:
    missing = [str(item) for item in inputs if not item.is_file()]
    if missing:
        raise SystemExit("inputs are not prepared; run just bench-prep first: " + ", ".join(missing))
    if not locations["prepared"].is_dir():
        raise SystemExit(f"prepared artefacts missing: run just bench-prep first ({locations['prepared']})")


def run_suite(manifest: dict, locations: dict[str, Path], families: list[str], datasets: list[str], smoke: bool, write_formats: str = "", read_formats: str = "") -> None:
    selected = {key: manifest["datasets"][key] for key in datasets}
    format_inputs = [source(entry, locations) for entry in selected.values() if entry["role"] in {"corpus", "largest-scaling"} or (smoke and entry["role"] == "scaling")]
    scaling_inputs = [source(entry, locations) for entry in selected.values() if entry["role"] in {"scaling", "largest-scaling"}]
    require_prepared(format_inputs + scaling_inputs, locations)
    if any(family in {"sizes", "formats"} for family in families):
        if not format_inputs:
            raise SystemExit("formats and sizes need corpus data or the largest 3DBAG slice")
        # `bench` remains the low-level format runner until its external-write
        # sampler lands. It must be passed explicit external output paths.
        if "formats" in families:
            output = result_dir(locations, "formats", smoke)
            just("bench", str(stage(locations, "formats", format_inputs)), str(output), read_formats, str(locations["prepared"]), "1" if smoke else "7")
            for input_path in format_inputs:
                command(
                    "python3", "benchmark/scripts/format_write.py", "--input", str(input_path),
                    "--canonical-seq", str(locations["prepared"] / f"{dataset_stem(input_path)}.city.jsonl"),
                    "--out", str(output / f"{dataset_stem(input_path)}.csv"),
                    "--raw-out", str(output / f"{dataset_stem(input_path)}.write.samples.csv"),
                    "--scratch", str(locations["work"] / "write-samples"),
                    "--repeat", "1" if smoke else "3",
                    *(["--formats", write_formats] if write_formats else []),
                )
                # A write subset is part of the run's identity: a CSV whose
                # write rows were measured for four formats must say so.
                configuration = "CityParquet=Hilbert; direct writers from canonical CityJSONSeq"
                if write_formats:
                    configuration += f"; write-formats={write_formats}"
                if read_formats:
                    configuration += f"; read-formats={read_formats}"
                write_run_manifest(input_path, output / f"{dataset_stem(input_path)}.csv", family="formats", repeat=1 if smoke else 7, write_repeat=1 if smoke else 3, smoke=smoke, fixed_configuration=configuration)
        if "sizes" in families:
            output = result_dir(locations, "sizes", smoke) / "sizes.csv"
            for input_path in format_inputs:
                command("python3", "benchmark/scripts/measure_sizes.py", "--input", str(input_path), "--prepared", str(locations["prepared"]), "--out", str(output))
    for family, recipe in (("codec", "codec-bench"), ("rowgroup", "rowgroup-bench")):
        if family not in families:
            continue
        if not scaling_inputs:
            raise SystemExit(f"{family} needs a 3DBAG scaling dataset")
        output = result_dir(locations, family, smoke)
        just(recipe, str(stage(locations, family, scaling_inputs)), str(output), str(locations["prepared"]), "1" if smoke else "7", "1" if smoke else "3")
        for input_path in scaling_inputs:
            write_run_manifest(input_path, output / f"{dataset_stem(input_path)}.csv", family=family, repeat=1 if smoke else 7, write_repeat=1 if smoke else 3, smoke=smoke, fixed_configuration="codec/default-row-groups" if family == "codec" else "row-groups/zstd-3")
    if "bloom" in families:
        bloom_inputs = [source(entry, locations) for entry in selected.values() if entry["role"] in {"corpus", "scaling", "largest-scaling"}]
        if not bloom_inputs:
            raise SystemExit("bloom needs a corpus dataset or a 3DBAG scaling slice")
        output = result_dir(locations, "bloom", smoke)
        just("bloom-bench", str(stage(locations, "bloom", bloom_inputs)), str(output), str(locations["prepared"]), "1" if smoke else "7", "1" if smoke else "3")
        for input_path in bloom_inputs:
            write_run_manifest(input_path, output / f"{dataset_stem(input_path)}.csv", family="bloom", repeat=1 if smoke else 7, write_repeat=1 if smoke else 3, smoke=smoke, fixed_configuration="bloom/zstd-3/default-row-groups")
    if "databases" in families:
        database_input = scaling_inputs[0] if smoke and scaling_inputs else source(manifest["datasets"][manifest["suite"]["largest_scaling_dataset"]], locations)
        if not database_input.is_file():
            raise SystemExit("database input is not prepared; run just bench-prep --families databases first")
        output = locations["databases"] / ("smoke" if smoke else "results")
        root = locations["formats"].parent
        # One invocation measures BOTH thread configurations — `single`
        # (the primary figure) and `parallel` — and then the write tier,
        # in that order: the write tier's mutations leave bloat behind that
        # a later read pass would measure as if it were the steady state.
        # The count tolerance is passed explicitly rather than left to the
        # CLI default so the suite's own choice is visible here and in the
        # run manifest.
        command("uv", "run", "--project", "benchmark/databases", "python", "-m", "citybench.cli", "smoke" if smoke else "run", "--data-root", str(root), "--prepared-dir", str(locations["prepared"]), "--dataset", str(database_input), "--output-dir", str(output), "--count-tolerance", str(DATABASE_COUNT_TOLERANCE))


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("command", choices=("prep", "run", "summary"))
    result.add_argument("--families", default="all", help=f"comma-separated families from {','.join(FAMILIES)}, or all")
    result.add_argument("--datasets", default="")
    result.add_argument("--smoke", action="store_true")
    result.add_argument("--read-formats", default="", help="comma-separated subset of the format tags whose read rows are measured (forwarded to the bench recipe's FORMATS; default: all). The coordinator truncates the CSV per run, so a subset run replaces every read row; use it to re-measure one format into a separate results copy and merge deliberately")
    result.add_argument("--write-formats", default="", help="comma-separated subset of the formats whose write rows are measured (forwarded to format_write.py --formats; default: all). A subset keeps the other formats' existing write rows out of the CSV, so use it only to re-measure part of a run and merge deliberately")
    result.add_argument("--data-root", type=Path, default=Path(os.environ.get("CITYPARQUET_BENCH_ROOT", DEFAULT_DATA_ROOT)))
    result.add_argument("--out", type=Path)
    result.add_argument("--figures", type=Path)
    return result


def main() -> None:
    args = parser().parse_args()
    root = args.data_root.expanduser().resolve()
    allowed = DEFAULT_DATA_ROOT.resolve()
    if not root.is_absolute() or not root.is_relative_to(allowed):
        raise SystemExit(f"--data-root must be below {allowed}")
    data = load_manifest()
    families = family_selection(args.families)
    datasets = dataset_selection(data, families, args.datasets, args.smoke)
    locations = paths(root)
    # Low-level fetch and conversion recipes inherit this explicit root.
    os.environ["CITYPARQUET_BENCH_ROOT"] = str(root)
    if args.command == "prep":
        prepare(data, locations, families, datasets, args.smoke)
    elif args.command == "run":
        run_suite(data, locations, families, datasets, args.smoke, args.write_formats, args.read_formats)
    else:
        output = (args.out or locations["summary"] / ("smoke" if args.smoke else "full")).expanduser().resolve()
        figures = args.figures.expanduser().resolve() if args.figures else None
        # The summary HTML and JSON stay under the data root; the figures may be
        # exported anywhere the caller owns, which is how the paper checkout
        # writes them to paper/assets/bench/ (benchmark/README.md).
        if not output.is_relative_to(allowed):
            raise SystemExit(f"--out must be below {allowed}")
        bench_dir = locations["formats"] / "smoke" if args.smoke else locations["formats"]
        cmd = ["uv", "run", "--project", "benchmark/plot", "python", "-m", "benchviz", "summary", "--data-root", str(root), "--bench-dir", str(bench_dir), "--out", str(output)]
        # Render exactly the suite selection requested by the caller.  The
        # renderer filters its prepared payload before making figures and HTML.
        cmd.extend(["--families", ",".join(families)])
        if args.datasets:
            cmd.extend(["--datasets", ",".join(renderer_dataset_ids(data, datasets))])
        if figures:
            cmd.extend(["--figures", str(figures)])
        command(*cmd)


if __name__ == "__main__":
    main()
