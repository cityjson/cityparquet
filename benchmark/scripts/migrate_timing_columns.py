#!/usr/bin/env python3
"""Recompute a results CSV's timing block from its own raw samples.

Both benchmark families write the same seven-column timing block,
``time_mean_s,time_std_s,time_median_s,time_min_s,time_max_s,time_q1_s,time_q3_s``
(and the database family a parallel ``server_time_*`` block). Every statistic is
recomputed here from the raw warm samples the run saved beside it:

- a readbench CSV (``benchmark/readbench``) from its ``<csv>.samples.json``
  sidecar, matched to a row by (dataset, format, scenario, query tag);
- a database CSV (``benchmark/databases``) from its own ``raw_time_samples_s``
  and ``raw_server_time_samples_s`` columns.

Two modes:

``migrate``  rewrites a CSV that still has the single-statistic header
             (``time_s,time_std_s`` and, for the database family,
             ``server_time_s``) into the seven-column block. Before writing it
             asserts, per row, that the recomputed mean and standard deviation,
             printed at the CSV's precision, equal the old cells exactly, and
             that no other column changes.
``verify``   checks a CSV that already has the seven-column block: every timing
             cell must equal the statistic recomputed from the samples.

Either mode fails naming the file and row on a mismatch or on missing samples.
A path may be a CSV or a results directory (every ``*.csv`` but ``sizes.csv``).

Statistic definitions (identical to ``benchmark/readbench/src/stats.rs`` and
``benchmark/databases/src/citybench/stats.py``): arithmetic mean; population
standard deviation; median and quartiles by linear interpolation at position
``p * (n - 1)`` on the sorted samples (numpy's default,
``statistics.quantiles(method="inclusive")``). The readbench mean and std are
computed with the same left-to-right float summation as the Rust coordinator,
and the database ones with ``statistics.mean``/``statistics.pstdev`` as
citybench does, so the printed values reproduce bit for bit.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import math
import statistics
import sys
from pathlib import Path

PRECISION = 6
STATS = ("mean", "std", "median", "min", "max", "q1", "q3")
TIME_BLOCK = [f"time_{s}_s" for s in STATS]
SERVER_BLOCK = [f"server_time_{s}_s" for s in STATS]
# Old single-statistic column -> the block that replaces it (empty: absorbed).
LEGACY = {"time_s": TIME_BLOCK, "time_std_s": [], "server_time_s": SERVER_BLOCK}


class MigrationError(Exception):
    pass


def quantile(ordered: list[float], p: float) -> float:
    """Linear interpolation at ``p * (n - 1)`` on already-sorted samples."""
    position = p * (len(ordered) - 1)
    lower = math.floor(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def summary(values: list[float], *, family: str) -> dict[str, float]:
    if family == "readbench":
        total = 0.0
        for v in values:
            total += v
        mean = total / len(values)
        sq = 0.0
        for v in values:
            sq += (v - mean) * (v - mean)
        std = math.sqrt(sq / len(values))
    else:
        mean = statistics.mean(values)
        std = statistics.pstdev(values)
    ordered = sorted(values)
    return {
        "mean": mean,
        "std": std,
        "median": quantile(ordered, 0.5),
        "min": ordered[0],
        "max": ordered[-1],
        "q1": quantile(ordered, 0.25),
        "q3": quantile(ordered, 0.75),
    }


def fmt(value: float) -> str:
    return f"{value:.{PRECISION}f}"


def _readbench_samples(path: Path) -> dict[tuple[str, str, str, str], list[float]]:
    sidecar = Path(str(path) + ".samples.json")
    if not sidecar.is_file():
        raise MigrationError(f"{path}: samples sidecar {sidecar.name} is missing")
    groups: dict[tuple[str, str, str, str], list[tuple[int, float]]] = {}
    for s in json.loads(sidecar.read_text()):
        if s["warmup"]:
            continue
        key = (s["dataset"], s["format"], s["scenario"], s["query_tag"])
        groups.setdefault(key, []).append((s["sample_index"], s["time_s"]))
    return {k: [t for _, t in sorted(v)] for k, v in groups.items()}


def _row_samples(path: Path, rows: list[dict[str, str]]) -> list[dict[str, list[float]]]:
    """Per row, the raw samples of each timing block: {'time': [...], 'server_time': [...]}."""
    out: list[dict[str, list[float]]] = []
    if rows and "raw_time_samples_s" in rows[0]:
        for row in rows:
            out.append(
                {
                    "time": json.loads(row["raw_time_samples_s"] or "[]"),
                    "server_time": json.loads(row["raw_server_time_samples_s"] or "[]"),
                }
            )
        return out
    groups = _readbench_samples(path)
    used: set[tuple[str, str, str, str]] = set()
    for line, row in enumerate(rows, start=2):
        base = (row["dataset"], row["format"], row["scenario"])
        first_tag = row["notes"].split(";")[0]
        key = base + (first_tag,) if base + (first_tag,) in groups else base + ("",)
        if key not in groups or key in used:
            raise MigrationError(f"{path}: row {line} ({';'.join(key)}): no warm samples in the sidecar")
        used.add(key)
        if str(len(groups[key])) != row["repeat"]:
            raise MigrationError(
                f"{path}: row {line}: {len(groups[key])} warm samples, but repeat={row['repeat']}"
            )
        out.append({"time": groups[key]})
    return out


def _read(path: Path) -> tuple[list[str], list[dict[str, str]], str]:
    with path.open(newline="") as fh:
        text = fh.read()
    terminator = "\r\n" if "\r\n" in text.split("\n", 1)[0] + "\n" else "\n"
    reader = csv.DictReader(io.StringIO(text, newline=""))
    rows = list(reader)
    return list(reader.fieldnames or []), rows, terminator


def _family(header: list[str]) -> str:
    return "databases" if "raw_time_samples_s" in header else "readbench"


def _expected(prefix: str, samples: list[float], family: str) -> dict[str, str]:
    """The block's printed cells; all blank when there are no samples (a failed cell)."""
    if not samples:
        return {f"{prefix}_{s}_s": "" for s in STATS}
    stats = summary(samples, family=family)
    return {f"{prefix}_{s}_s": fmt(stats[s]) for s in STATS}


def migrate_file(path: Path) -> int:
    header, rows, terminator = _read(path)
    if "time_s" not in header:
        raise MigrationError(f"{path}: no legacy time_s column (use verify for a migrated CSV)")
    family = _family(header)
    new_header: list[str] = []
    for column in header:
        new_header.extend(LEGACY.get(column, [column]))
    per_row = _row_samples(path, rows)
    new_rows: list[dict[str, str]] = []
    for line, (row, samples) in enumerate(zip(rows, per_row), start=2):
        new = {k: v for k, v in row.items() if k not in LEGACY}
        blocks = [("time", "time_s", "time_std_s")]
        if "server_time_s" in header:
            blocks.append(("server_time", "server_time_s", None))
        for prefix, old_mean, old_std in blocks:
            cells = _expected(prefix, samples[prefix], family)
            checks = [(old_mean, f"{prefix}_mean_s")]
            if old_std:
                checks.append((old_std, f"{prefix}_std_s"))
            for old, fresh in checks:
                if row[old] != cells[fresh]:
                    raise MigrationError(
                        f"{path}: row {line}: recomputed {fresh}={cells[fresh]!r} "
                        f"!= committed {old}={row[old]!r}"
                    )
            new.update(cells)
        new_rows.append(new)
    buffer = io.StringIO(newline="")
    writer = csv.DictWriter(buffer, fieldnames=new_header, lineterminator=terminator)
    writer.writeheader()
    writer.writerows(new_rows)
    # No column but the timing ones may change: re-read and compare.
    reread = list(csv.DictReader(io.StringIO(buffer.getvalue(), newline="")))
    for line, (old, new) in enumerate(zip(rows, reread), start=2):
        for column in header:
            if column not in LEGACY and old[column] != new[column]:
                raise MigrationError(f"{path}: row {line}: column {column} would change")
    with path.open("w", newline="") as fh:
        fh.write(buffer.getvalue())
    return len(rows)


def verify_file(path: Path) -> int:
    header, rows, _ = _read(path)
    if "time_s" in header or "time_mean_s" not in header:
        raise MigrationError(f"{path}: not a seven-column timing CSV (migrate it first)")
    family = _family(header)
    prefixes = ["time"] + (["server_time"] if "server_time_mean_s" in header else [])
    for line, (row, samples) in enumerate(zip(rows, _row_samples(path, rows)), start=2):
        for prefix in prefixes:
            for column, want in _expected(prefix, samples[prefix], family).items():
                if row[column] != want:
                    raise MigrationError(
                        f"{path}: row {line}: {column}={row[column]!r}, samples give {want!r}"
                    )
    return len(rows)


def _csvs(paths: list[Path]) -> list[Path]:
    out: list[Path] = []
    for p in paths:
        if p.is_dir():
            out.extend(sorted(c for c in p.glob("*.csv") if c.name != "sizes.csv"))
        else:
            out.append(p)
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("mode", choices=("migrate", "verify"))
    parser.add_argument("paths", nargs="+", type=Path, help="results CSVs or directories")
    args = parser.parse_args(argv)
    action = migrate_file if args.mode == "migrate" else verify_file
    files = _csvs(args.paths)
    if not files:
        print("no results CSVs found", file=sys.stderr)
        return 1
    try:
        for path in files:
            n = action(path)
            print(f"{args.mode}: {path}: {n} rows ok")
    except MigrationError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
