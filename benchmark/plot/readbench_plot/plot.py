"""Render grouped-bar charts from cityparquet-readbench result CSVs.

Each CSV in a results directory is expected to begin with the 13-column
read-benchmark contract `cityparquet-readbench`'s coordinator writes (see
`CSV_HEADER` in the package `__init__`, mirroring
`crates/cityparquet-readbench/src/coordinator.rs`):

    dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,
    peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests

The match is a prefix check, so a coordinator that later grows further
trailing columns still plots; files whose leading columns don't match (e.g.
the M5 write-benchmark's own, differently-shaped CSVs under benchmark/formats/results/)
are skipped, not errored on.

For each matching CSV, two PNGs are written under `<results_dir>/plots/`:
`<name>-time.png` (median `time_s` per scenario, grouped by format, log
y-axis, since the formats compared span orders of magnitude) and
`<name>-mem.png` (median `peak_heap_bytes` per scenario, grouped by format).
A format with no heap data at all for a given metric (e.g. `duckdb-parquet`,
which never populates `peak_heap_bytes`) simply draws no bars for that
metric rather than a fabricated zero. Scenarios with more than one row per
format (`bbox-query`'s three selectivity windows) are collapsed with a
median, consistent with the "median ... per scenario" framing. Rows whose
`notes` is exactly `cold` (the separate, manual purged-cache measurement)
are excluded before aggregating. A cross-dataset summary of `full-read`
timings is also produced.

Bars are coloured from the package-level `FORMAT_COLORS`, keyed by format
name and shared with `sizes.py`, so one format is one colour in every figure
this project renders and adding a format cannot recolour an existing one. A
format the colour map has never heard of is drawn hatched in grey rather
than dropped.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import cast

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import pandas as pd

from readbench_plot import CSV_HEADER, FORMAT_COLORS, FORMAT_ORDER, bar_style

# Preferred left-to-right ordering for readability; anything else seen in
# the data is appended afterwards (alphabetically), so an unrecognised
# scenario/format is never dropped, just placed last.
SCENARIO_ORDER = [
    "count",
    "full-read",
    "id-lookup",
    "bbox-query",
    "attr-filter",
    "attr-stats",
    "project",
]

# `FORMAT_ORDER` and `FORMAT_COLORS` are the package's, shared with sizes.py:
# this module used to carry its own six-entry ordering while sizes.py carried
# a five-entry one, and no colour map at all. Re-exported so
# `plot.FORMAT_ORDER` keeps working for anything that already reads it.
__all__ = ["FORMAT_COLORS", "FORMAT_ORDER", "aggregate", "load_csv", "run"]


def _ordered(seen: list[str], preferred: list[str]) -> list[str]:
    seen_set = set(seen)
    head = [v for v in preferred if v in seen_set]
    tail = sorted(seen_set - set(head))
    return head + tail


def load_csv(path: Path) -> pd.DataFrame | None:
    """Read `path` if its header matches the read-benchmark CSV contract.

    Returns None (and lets the caller decide whether to log a skip) for any
    file that doesn't match — e.g. a write-benchmark CSV with a different
    column set.
    """
    with path.open(newline="") as fh:
        header = fh.readline().strip().split(",")
    # Prefix check, not equality: the coordinator may add trailing columns
    # (it added bytes_read/http_requests for the HTTP transport). A strict
    # `!=` silently skipped every CSV and produced empty charts - a wrong
    # picture is worse than a loud failure here, because the charts go in the
    # paper.
    if header[: len(CSV_HEADER)] != CSV_HEADER:
        return None

    df = pd.read_csv(path)
    for col in ("time_s", "peak_heap_bytes"):
        df[col] = pd.to_numeric(df[col], errors="coerce")
    return cast(pd.DataFrame, df[df["notes"] != "cold"])


def aggregate(df: pd.DataFrame) -> pd.DataFrame:
    """Median `time_s`/`peak_heap_bytes` per (scenario, format).

    Scenarios with several rows per format (distinct selectivities, e.g.
    bbox-query's 1%/5%/25% windows) are collapsed to a single point.
    """
    return cast(
        pd.DataFrame,
        df.groupby(["scenario", "format"], as_index=False)[
            ["time_s", "peak_heap_bytes"]
        ].median(),
    )


def _grouped_bar(
    agg: pd.DataFrame,
    value_col: str,
    title: str,
    ylabel: str,
    out_path: Path,
    *,
    log_y: bool = False,
) -> None:
    scenarios = _ordered(list(agg["scenario"].unique()), SCENARIO_ORDER)
    formats = _ordered(list(agg["format"].unique()), FORMAT_ORDER)

    pivot = agg.pivot(index="scenario", columns="format", values=value_col)
    pivot = pivot.reindex(index=scenarios, columns=formats)

    fig, ax = plt.subplots(figsize=(max(6, len(scenarios) * 1.4), 4.5))
    n_formats = len(formats)
    width = 0.8 / max(n_formats, 1)
    x = range(len(scenarios))

    for j, fmt in enumerate(formats):
        values = pivot[fmt].tolist()
        # A format with no data at all for this metric (e.g. duckdb-parquet
        # never populates peak_heap_bytes) draws no bars rather than a
        # fabricated zero.
        if all(pd.isna(v) for v in values):
            continue
        offsets = [xi + (j - (n_formats - 1) / 2) * width for xi in x]
        heights = [v if pd.notna(v) else 0 for v in values]
        ax.bar(offsets, heights, width=width, label=fmt, **bar_style(fmt))

    ax.set_xticks(list(x))
    ax.set_xticklabels(scenarios, rotation=20, ha="right")
    ax.set_ylabel(ylabel)
    ax.set_title(title)
    if log_y:
        ax.set_yscale("log")
    ax.legend(title="format", fontsize="small")
    fig.tight_layout()
    fig.savefig(out_path, dpi=150)
    plt.close(fig)


def plot_dataset(name: str, df: pd.DataFrame, plots_dir: Path) -> None:
    agg = aggregate(df)

    _grouped_bar(
        agg,
        "time_s",
        f"{name}: median time per scenario",
        "time_s (log scale)",
        plots_dir / f"{name}-time.png",
        log_y=True,
    )
    _grouped_bar(
        agg,
        "peak_heap_bytes",
        f"{name}: median peak heap per scenario",
        "peak_heap_bytes",
        plots_dir / f"{name}-mem.png",
    )


def plot_summary(datasets: dict[str, pd.DataFrame], plots_dir: Path) -> None:
    """One cross-dataset figure: median full-read time_s per format."""
    rows = []
    for name, df in datasets.items():
        agg = aggregate(df)
        full_read = agg[agg["scenario"] == "full-read"]
        for _, row in full_read.iterrows():
            rows.append({"dataset": name, "format": row["format"], "time_s": row["time_s"]})
    if not rows:
        return
    summary = pd.DataFrame(rows)

    dataset_order = sorted(summary["dataset"].unique())
    formats = _ordered(list(summary["format"].unique()), FORMAT_ORDER)
    pivot = summary.pivot(index="dataset", columns="format", values="time_s")
    pivot = pivot.reindex(index=dataset_order, columns=formats)

    fig, ax = plt.subplots(figsize=(max(6, len(dataset_order) * 1.6), 4.5))
    n_formats = len(formats)
    width = 0.8 / max(n_formats, 1)
    x = range(len(dataset_order))
    for j, fmt in enumerate(formats):
        values = pivot[fmt].tolist()
        if all(pd.isna(v) for v in values):
            continue
        offsets = [xi + (j - (n_formats - 1) / 2) * width for xi in x]
        heights = [v if pd.notna(v) else 0 for v in values]
        ax.bar(offsets, heights, width=width, label=fmt, **bar_style(fmt))

    ax.set_xticks(list(x))
    ax.set_xticklabels(dataset_order, rotation=20, ha="right")
    ax.set_ylabel("time_s (log scale)")
    ax.set_yscale("log")
    ax.set_title("full-read: median time across datasets")
    ax.legend(title="format", fontsize="small")
    fig.tight_layout()
    fig.savefig(plots_dir / "summary-full-read.png", dpi=150)
    plt.close(fig)


def run(results_dir: Path) -> int:
    if not results_dir.is_dir():
        print(f"error: not a directory: {results_dir}", file=sys.stderr)
        return 1

    plots_dir = results_dir / "plots"
    plots_dir.mkdir(exist_ok=True)

    datasets: dict[str, pd.DataFrame] = {}
    for csv_path in sorted(results_dir.glob("*.csv")):
        df = load_csv(csv_path)
        if df is None:
            print(f"skip {csv_path.name} (not a read-benchmark CSV)", file=sys.stderr)
            continue
        datasets[csv_path.stem] = df

    if not datasets:
        print(f"no read-benchmark CSVs found in {results_dir}", file=sys.stderr)
        return 1

    for name, df in datasets.items():
        plot_dataset(name, df, plots_dir)
        print(f"wrote {plots_dir / (name + '-time.png')}")
        print(f"wrote {plots_dir / (name + '-mem.png')}")

    plot_summary(datasets, plots_dir)
    print(f"wrote {plots_dir / 'summary-full-read.png'}")

    return 0
