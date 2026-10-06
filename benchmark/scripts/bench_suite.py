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
FAMILIES = ("sizes", "formats", "bloom", "databases")

# A run profile fixes which datasets are measured, how many repetitions and
# where the results go, so that a test run can never overwrite the paper's
# evidence:
#   full   the corpus and the 3DBAG slice, 25 read repetitions, results in each
#          family's own directory; the database family measures the slice;
#   short  the corpus without the slice, the same repetitions, results under
#          `<family>/short/`; for iterating on the harness in an hour instead
#          of a day;
#   smoke  Rotterdam alone, 1 repetition, `<family>/smoke/`; a pipeline check,
#          never a measurement.
# Under `short` and `smoke` the database family measures the manifest's
# `small_database_dataset` (Rotterdam) in the slice's place.
PROFILES = ("full", "short", "smoke")
# The dataset roles that are format, size and bloom inputs.
INPUT_ROLES = frozenset({"corpus", "slice"})
# The read recipes' shared-host isolation (benchmark/README.md, "Running on
# a shared host"): NUMA node, children's memory ceiling, load gate.
DEFAULT_ISOLATION = {"numa_node": os.environ.get("BENCH_NUMA_NODE", "auto"), "memory_max": None, "max_load": "auto", "max_load_wait_s": 600}


def slice_dataset(manifest: dict) -> str:
    """The manifest key of the 3DBAG slice."""
    return manifest["suite"]["slice_dataset"]


def database_dataset(manifest: dict, profile: str) -> str:
    """The manifest key of the dataset the database family measures."""
    if profile == "full":
        return slice_dataset(manifest)
    return manifest["suite"]["small_database_dataset"]


def profile_subdir(profile: str) -> str:
    return "" if profile == "full" else profile


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


def dataset_selection(manifest: dict, families: list[str], requested: str, profile: str) -> list[str]:
    datasets = manifest["datasets"]
    if requested:
        result = [slice_dataset(manifest) if item == "3dbag" else item for item in values(requested)]
    elif profile == "smoke":
        result = ["rotterdam"]
    else:
        result = []
        if any(family in {"sizes", "formats", "bloom"} for family in families):
            result.extend(
                key for key, entry in datasets.items()
                if entry["role"] in INPUT_ROLES
                and (profile != "short" or entry["role"] != "slice")
            )
        if "databases" in families:
            result.append(database_dataset(manifest, profile))
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
        "3dbag": root / "data" / "3dbag",
        "prepared": root / "data" / "readbench",
        "formats": root / "formats",
        "databases": root / "databases",
        "summary": root / "summary",
        "work": root / "work",
    }


def source(entry: dict, locations: dict[str, Path]) -> Path:
    location = locations["corpus"] if entry["role"] == "corpus" else locations["3dbag"]
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


def write_run_manifest(source_path: Path, result_csv: Path, *, family: str, repeat: int, smoke: bool, fixed_configuration: str, profile: str = "full", cell_budget_s: float | None = None, min_repeat: int = 7, isolation: dict | None = None) -> None:
    """Persist enough local evidence to identify one benchmark result exactly."""
    artefacts = [
        result_csv,
        Path(f"{result_csv}.samples.json"),
        Path(f"{result_csv}.params.json"),
    ]
    files = {str(path.name): sha256(path) for path in artefacts if path.is_file()}
    params = Path(f"{result_csv}.params.json")
    # What the run asked for beside what readbench recorded as applied.
    applied = json.loads(params.read_text()).get("isolation") if params.is_file() else None
    manifest = {
        "schema": 1,
        "family": family,
        "smoke": smoke,
        "profile": profile,
        "source": {"path": str(source_path.resolve()), "sha256": sha256(source_path)},
        "result": {"csv": str(result_csv.resolve()), "files_sha256": files, "params_sha256": sha256(params) if params.is_file() else None},
        "measurement": {"read_repeat": repeat, "cell_budget_s": cell_budget_s, "min_repeat": min_repeat, "fixed_configuration": fixed_configuration, "isolation": {"requested": isolation or DEFAULT_ISOLATION, "applied": applied}},
        "code": code_identity(),
        "machine": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "processor": platform.processor(), "python": sys.version.split()[0]},
        "tools": {"rust": version("rustc", "--version"), "fcb": version("fcb", "--version"), "cjseq": version("cjseq", "--version"), "cityparquet": version("lib/cityparquet-rs/target/release/cityparquet", "--version")},
    }
    target = result_csv.with_suffix(".run.json")
    temporary = target.with_suffix(".run.json.tmp")
    temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    temporary.replace(target)


def prepare(manifest: dict, locations: dict[str, Path], families: list[str], datasets: list[str], profile: str) -> None:
    selected = [manifest["datasets"][key] for key in datasets]
    if any(entry["role"] == "corpus" for entry in selected):
        just("fetch-data", str(locations["corpus"]))
    if any(entry["role"] == "slice" for entry in selected):
        just("fetch-3dbag", str(locations["3dbag"]))
    # Format and size comparisons need every artefact; the bloom and database
    # families need only CityParquet + CityJSONSeq.
    full_formats = any(family in {"sizes", "formats"} for family in families)
    if full_formats:
        just("fetch-tools")
        command("cargo", "build", "--release", "--manifest-path", "lib/cityparquet-rs/Cargo.toml", "-p", "cityparquet-cli", "--bin", "cityparquet")
        # The read harness lives in its own workspace — a second manifest, a
        # second target directory. Building it here keeps compilation out of
        # the run.
        command("cargo", "build", "--release", "--manifest-path", "benchmark/readbench/Cargo.toml", "--bin", "cityparquet-readbench")
    locations["prepared"].mkdir(parents=True, exist_ok=True)
    for key in datasets:
        entry = manifest["datasets"][key]
        input_path = source(entry, locations)
        if not input_path.is_file():
            raise SystemExit(f"prepared source missing after fetch: {input_path}")
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
        # and does not duplicate a multi-gigabyte input.
        link.hardlink_to(input_path)
    for existing in directory.iterdir():
        if existing.name not in wanted:
            existing.unlink()
    return directory


def result_dir(locations: dict[str, Path], family: str, profile: str) -> Path:
    if profile is True or profile is False:  # the old boolean spelling
        profile = "smoke" if profile else "full"
    root = locations["formats"] / profile_subdir(profile) if profile_subdir(profile) else locations["formats"]
    names = {"formats": "results", "sizes": "results", "bloom": "bloom_results"}
    return root / names[family]


def require_prepared(inputs: list[Path], locations: dict[str, Path]) -> None:
    missing = [str(item) for item in inputs if not item.is_file()]
    if missing:
        raise SystemExit("inputs are not prepared; run just bench-prep first: " + ", ".join(missing))
    if not locations["prepared"].is_dir():
        raise SystemExit(f"prepared artefacts missing: run just bench-prep first ({locations['prepared']})")


def read_repeat(profile: str) -> int:
    """Timed read repetitions per cell (each after one discarded warm-up)."""
    return 1 if profile == "smoke" else 25


def run_suite(manifest: dict, locations: dict[str, Path], families: list[str], datasets: list[str], profile: str, read_formats: str = "", cell_budget_s: float | None = None, min_repeat: int = 7, isolation: dict | None = None) -> None:
    smoke = profile == "smoke"
    repeat = read_repeat(profile)
    budget = "" if cell_budget_s is None else str(cell_budget_s)
    isolation = isolation or DEFAULT_ISOLATION
    sampling = {"cell_budget_s": cell_budget_s, "min_repeat": min_repeat, "isolation": isolation}
    isolation_args = (str(isolation["numa_node"]), "" if isolation["memory_max"] is None else str(isolation["memory_max"]), str(isolation["max_load"]), str(isolation["max_load_wait_s"]))
    selected = {key: manifest["datasets"][key] for key in datasets}
    inputs = [source(entry, locations) for entry in selected.values() if entry["role"] in INPUT_ROLES]
    require_prepared(inputs, locations)
    if any(family in {"sizes", "formats", "bloom"} for family in families) and not inputs:
        raise SystemExit("formats, sizes and bloom need a corpus dataset or the 3DBAG slice")
    if "formats" in families:
        # `bench` is the low-level format runner. It must be passed explicit
        # external output paths.
        output = result_dir(locations, "formats", profile)
        just("bench", str(stage(locations, "formats", inputs)), str(output), read_formats, str(locations["prepared"]), str(repeat), budget, str(min_repeat), *isolation_args)
        for input_path in inputs:
            # A read subset is part of the run's identity: a CSV whose
            # read rows were measured for four formats must say so.
            configuration = "CityParquet=Hilbert"
            if read_formats:
                configuration += f"; read-formats={read_formats}"
            write_run_manifest(input_path, output / f"{dataset_stem(input_path)}.csv", family="formats", repeat=repeat, smoke=smoke, fixed_configuration=configuration, profile=profile, **sampling)
    if "sizes" in families:
        output = result_dir(locations, "sizes", profile) / "sizes.csv"
        for input_path in inputs:
            command(sys.executable, "benchmark/scripts/measure_sizes.py", "--input", str(input_path), "--prepared", str(locations["prepared"]), "--out", str(output))
    if "bloom" in families:
        output = result_dir(locations, "bloom", profile)
        just("bloom-bench", str(stage(locations, "bloom", inputs)), str(output), str(locations["prepared"]), str(repeat), budget, str(min_repeat), *isolation_args)
        for input_path in inputs:
            write_run_manifest(input_path, output / f"{dataset_stem(input_path)}.csv", family="bloom", repeat=repeat, smoke=smoke, fixed_configuration="bloom/hilbert/zstd-3/default-row-groups", profile=profile, **sampling)
    if "databases" in families:
        database_key = database_dataset(manifest, profile)
        if database_key not in selected:
            raise SystemExit(f"the database family under --profile {profile} measures {database_key}; select it")
        database_entry = manifest["datasets"][database_key]
        database_input = source(database_entry, locations)
        if database_entry["role"] != "slice":
            # The database systems ingest CityJSONSeq; the prepared stream is
            # the same content the format family read.
            database_input = locations["prepared"] / f"{dataset_stem(database_input)}.city.jsonl"
        if not database_input.is_file():
            raise SystemExit("database input is not prepared; run just bench-prep --families databases first")
        output = locations["databases"] / (profile_subdir(profile) or "results")
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
    result.add_argument("--profile", choices=PROFILES, default="full", help="full: the corpus and the 3DBAG slice, 25 read repetitions, the families' own result directories (the paper's evidence); short: the corpus without the slice, the same repetitions, under <family>/short/, for iterating on the harness; smoke: Rotterdam alone, 1 repetition, under <family>/smoke/. Under short and smoke the database family measures Rotterdam")
    result.add_argument("--smoke", action="store_true", help="the same as --profile smoke")
    result.add_argument("--read-formats", default="", help="comma-separated subset of the format tags whose read rows are measured (forwarded to the bench recipe's FORMATS; default: all). The coordinator truncates the CSV per run, so a subset run replaces every read row; use it to re-measure one format into a separate results copy and merge deliberately")
    result.add_argument("--cell-budget-s", type=float, default=None, help="run only: optional per-cell time budget in seconds for the formats and bloom families (default: off); a cell stops sampling once its runs, warm-up included, took this long and at least --min-repeat samples exist")
    result.add_argument("--numa-node", default=DEFAULT_ISOLATION["numa_node"], help="run only: NUMA node for the read families' measured processes: an id, auto (most free memory; default, or $BENCH_NUMA_NODE) or off")
    result.add_argument("--memory-max", type=int, default=None, help="run only: memory ceiling in bytes for the read families' measured processes, via systemd-run --user (default: off)")
    result.add_argument("--max-load", default="auto", help="run only: load gate before every read sample: the pinned node's load share against this threshold; auto = half the node's cores, off disables")
    result.add_argument("--max-load-wait-s", type=int, default=600, help="run only: the longest a read sample waits for the load to drop before it proceeds and its cell is tagged busy (default 600)")
    result.add_argument("--min-repeat", type=int, default=7, help="run only: the fewest timed samples a budgeted cell takes (default 7, clamped to the repetitions)")
    result.add_argument("--data-root", type=Path, default=Path(os.environ.get("CITYPARQUET_BENCH_ROOT", DEFAULT_DATA_ROOT)))
    result.add_argument("--out", type=Path)
    result.add_argument("--figures", type=Path)
    result.add_argument("--statistic", choices=("median", "mean"), default="median", help="summary only: the timing statistic the figures plot; median (spread q1-q3, the default) or mean (spread +-1 population std)")
    return result


def main() -> None:
    args = parser().parse_args()
    root = args.data_root.expanduser().resolve()
    allowed = DEFAULT_DATA_ROOT.resolve()
    if not root.is_absolute() or not root.is_relative_to(allowed):
        raise SystemExit(f"--data-root must be below {allowed}")
    data = load_manifest()
    families = family_selection(args.families)
    profile = "smoke" if args.smoke else args.profile
    datasets = dataset_selection(data, families, args.datasets, profile)
    locations = paths(root)
    # Low-level fetch and conversion recipes inherit this explicit root.
    os.environ["CITYPARQUET_BENCH_ROOT"] = str(root)
    if args.command == "prep":
        prepare(data, locations, families, datasets, profile)
    elif args.command == "run":
        run_suite(data, locations, families, datasets, profile, args.read_formats, args.cell_budget_s, args.min_repeat, {"numa_node": args.numa_node, "memory_max": args.memory_max, "max_load": args.max_load, "max_load_wait_s": args.max_load_wait_s})
    else:
        output = (args.out or locations["summary"] / profile).expanduser().resolve()
        figures = args.figures.expanduser().resolve() if args.figures else None
        # The summary HTML and JSON stay under the data root; the figures may be
        # exported anywhere the caller owns, which is how the paper checkout
        # writes them to paper/assets/bench/ (benchmark/README.md).
        if not output.is_relative_to(allowed):
            raise SystemExit(f"--out must be below {allowed}")
        bench_dir = locations["formats"] / profile_subdir(profile) if profile_subdir(profile) else locations["formats"]
        cmd = ["uv", "run", "--project", "benchmark/plot", "python", "-m", "benchviz", "summary", "--data-root", str(root), "--bench-dir", str(bench_dir), "--out", str(output)]
        # Render exactly the suite selection requested by the caller.  The
        # renderer filters its prepared payload before making figures and HTML.
        cmd.extend(["--families", ",".join(families), "--statistic", args.statistic])
        if args.datasets:
            cmd.extend(["--datasets", ",".join(renderer_dataset_ids(data, datasets))])
        if figures:
            cmd.extend(["--figures", str(figures)])
        command(*cmd)


if __name__ == "__main__":
    main()
