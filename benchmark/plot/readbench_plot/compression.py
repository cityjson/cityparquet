"""Compression-codec and row-group-size comparison charts.

Reads every `cityparquet bench` write-bench CSV in a results directory (see
`just compression-bench`, `crates/cityparquet-cli/src/bench.rs`'s
`CSV_HEADER`) and, per dataset, renders two axes of the writer-tuning space:

- **codec axis**: the variants sharing the default row-group size (65536)
  but varying compression codec (`cityparquet` = zstd, plus
  `+uncompressed`/`+snappy`/`+gzip`/`+lz4`/`+brotli`) — how codec trades
  on-disk size against write/read time.
- **row-group axis**: the zstd variants varying row-group size
  (`cityparquet` = 65536, `+rg512`, `+rg4096`) — how row-group size trades
  bbox-pruning selectivity against per-group overhead.

A variant identifier is `<preset>[+hilbert][+by-type][+rg<N>][+<codec>]`
(suffixes in any order; see `bench.rs`'s `parse_variant`). This module only
needs the row-group size and compression codec each variant resolved to, so
it re-derives both from the `variant` column rather than importing Rust:
split on `+`, a token matching a known codec name
(`uncompressed`/`snappy`/`gzip`/`lz4`/`brotli`/`zstd`) sets the codec
(default `zstd` if no such token — the tuned `cityparquet` preset's
default), and a `rg<N>` token sets the row-group size (default `65536`,
`WriterRecipe`'s default). `hilbert`/`by-type` tokens vary a dimension
neither chart covers, so any variant carrying one is dropped (and named on
stderr) rather than silently folded into a codec/row-group bucket it
doesn't represent.

Any CSV in the results directory that isn't a write-bench CSV (e.g. a
`cityparquet-readbench` result under the same directory) is skipped rather
than errored on, exactly as `plot.py` skips non-matching headers.

Output: `<results_dir>/plots/<dataset>-codec-size.png`,
`<results_dir>/plots/<dataset>-codec-time.png`, and
`<results_dir>/plots/<dataset>-rowgroup.png` per dataset.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import cast

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import pandas as pd

# The exact CSV header `cityparquet bench` writes (see `CSV_HEADER` in
# crates/cityparquet-cli/src/bench.rs). A CSV whose first line doesn't match
# this exactly is not a write-bench result and is skipped.
WRITE_BENCH_HEADER = [
    "dataset",
    "variant",
    "object_count",
    "write_s",
    "total_bytes",
    "cityobjects_bytes",
    "sidecar_bytes",
    "full_scan_s",
    "window_query_s",
    "row_groups_total",
    "row_groups_touched",
    "roundtrip_equal",
]

KNOWN_CODECS = {"uncompressed", "snappy", "gzip", "lz4", "brotli", "zstd"}
IGNORED_TOKENS = {"hilbert", "by-type"}
DEFAULT_CODEC = "zstd"
DEFAULT_ROW_GROUP_SIZE = 65536

# Preferred left-to-right ordering (roughly fastest/biggest to
# slowest/smallest); anything else seen in the data is appended afterwards
# (alphabetically). Shared color-per-codec across both codec charts.
CODEC_ORDER = ["uncompressed", "snappy", "lz4", "zstd", "gzip", "brotli"]
CODEC_COLORS = dict(zip(CODEC_ORDER, plt.get_cmap("tab10").colors, strict=False))


def _ordered(seen: list[str], preferred: list[str]) -> list[str]:
    seen_set = set(seen)
    head = [v for v in preferred if v in seen_set]
    tail = sorted(seen_set - set(head))
    return head + tail


def parse_variant(variant: str) -> tuple[str, int] | None:
    """Resolve a `cityparquet bench` variant id to `(codec, row_group_size)`.

    Returns `None` for a variant carrying a `hilbert`/`by-type` token — a
    dimension neither the codec nor row-group chart covers, so such variants
    are dropped by the caller rather than mis-bucketed.
    """
    tokens = variant.split("+")
    if any(t in IGNORED_TOKENS for t in tokens):
        return None

    codec = DEFAULT_CODEC
    row_group_size = DEFAULT_ROW_GROUP_SIZE
    for token in tokens:
        if token in KNOWN_CODECS:
            codec = token
        elif token.startswith("rg") and token[2:].isdigit():
            row_group_size = int(token[2:])
    return codec, row_group_size


def load_csv(path: Path) -> pd.DataFrame | None:
    """Read `path` if its header matches the write-bench CSV contract."""
    with path.open(newline="") as fh:
        header = fh.readline().strip().split(",")
    if header != WRITE_BENCH_HEADER:
        return None

    df = pd.read_csv(path)
    for col in (
        "write_s",
        "total_bytes",
        "full_scan_s",
        "window_query_s",
        "row_groups_total",
        "row_groups_touched",
    ):
        df[col] = pd.to_numeric(df[col], errors="coerce")
    return df


def annotate(name: str, df: pd.DataFrame) -> pd.DataFrame:
    """Add `codec`/`row_group_size` columns, dropping hilbert/by-type variants."""
    codecs: list[str | None] = []
    row_group_sizes: list[int | None] = []
    for variant in df["variant"]:
        parsed = parse_variant(str(variant))
        if parsed is None:
            codecs.append(None)
            row_group_sizes.append(None)
        else:
            codecs.append(parsed[0])
            row_group_sizes.append(parsed[1])

    out = df.copy()
    out["codec"] = codecs
    out["row_group_size"] = row_group_sizes

    skipped = out.loc[out["codec"].isna(), "variant"].tolist()
    if skipped:
        print(
            f"note: {name}: skipping hilbert/by-type variant(s) not on the "
            f"codec/row-group axes: {skipped}",
            file=sys.stderr,
        )
    return cast(pd.DataFrame, out.dropna(subset=["codec"]))


def plot_codec_size(name: str, df: pd.DataFrame, plots_dir: Path) -> None:
    codec_df = df[df["row_group_size"] == DEFAULT_ROW_GROUP_SIZE]
    if codec_df.empty:
        print(
            f"note: {name}: no default-row-group-size variants, skipping codec-size",
            file=sys.stderr,
        )
        return

    codecs = _ordered(list(cast(pd.Series, codec_df["codec"]).unique()), CODEC_ORDER)
    sizes_mb = cast(pd.Series, codec_df.groupby("codec")["total_bytes"].mean()).reindex(codecs) / (
        1024 * 1024
    )

    fig, ax = plt.subplots(figsize=(max(6, len(codecs) * 1.2), 4.5))
    ax.bar(codecs, sizes_mb.tolist(), color=[CODEC_COLORS.get(c) for c in codecs])
    ax.set_ylabel("size (MB)")
    ax.set_title(f"{name}: CityParquet size by compression codec")
    fig.tight_layout()
    fig.savefig(plots_dir / f"{name}-codec-size.png", dpi=150)
    plt.close(fig)


def plot_codec_time(name: str, df: pd.DataFrame, plots_dir: Path) -> None:
    codec_df = df[df["row_group_size"] == DEFAULT_ROW_GROUP_SIZE]
    if codec_df.empty:
        print(
            f"note: {name}: no default-row-group-size variants, skipping codec-time",
            file=sys.stderr,
        )
        return

    codecs = _ordered(list(cast(pd.Series, codec_df["codec"]).unique()), CODEC_ORDER)
    agg = cast(pd.DataFrame, codec_df.groupby("codec")[["write_s", "full_scan_s"]].mean()).reindex(
        codecs
    )

    metrics = ["write_s", "full_scan_s"]
    width = 0.8 / len(metrics)
    x = range(len(codecs))

    fig, ax = plt.subplots(figsize=(max(6, len(codecs) * 1.4), 4.5))
    for j, metric in enumerate(metrics):
        offsets = [xi + (j - (len(metrics) - 1) / 2) * width for xi in x]
        ax.bar(offsets, agg[metric].tolist(), width=width, label=metric)

    ax.set_xticks(list(x))
    ax.set_xticklabels(codecs)
    ax.set_ylabel("time (s)")
    ax.set_title(f"{name}: write vs full-read time by codec")
    ax.legend(fontsize="small")
    fig.tight_layout()
    fig.savefig(plots_dir / f"{name}-codec-time.png", dpi=150)
    plt.close(fig)


def plot_rowgroup(name: str, df: pd.DataFrame, plots_dir: Path) -> None:
    rg_df = df[df["codec"] == "zstd"]
    if rg_df.empty:
        print(f"note: {name}: no zstd variants, skipping rowgroup chart", file=sys.stderr)
        return

    rg_sizes = sorted(cast(list, cast(pd.Series, rg_df["row_group_size"]).unique().tolist()))
    agg = cast(
        pd.DataFrame,
        rg_df.groupby("row_group_size")[
            ["window_query_s", "row_groups_total", "row_groups_touched"]
        ].mean(),
    ).reindex(rg_sizes)
    pruning_fraction = agg["row_groups_touched"] / agg["row_groups_total"]

    labels = [str(rg) for rg in rg_sizes]
    x = range(len(rg_sizes))

    fig, ax1 = plt.subplots(figsize=(max(6, len(rg_sizes) * 1.6), 4.5))
    ax1.bar(
        x,
        agg["window_query_s"].tolist(),
        width=0.4,
        color=CODEC_COLORS.get("zstd"),
        label="window_query_s",
    )
    ax1.set_xticks(list(x))
    ax1.set_xticklabels(labels)
    ax1.set_xlabel("row-group size (rows)")
    ax1.set_ylabel("window_query_s")

    ax2 = ax1.twinx()
    ax2.plot(
        list(x),
        pruning_fraction.tolist(),
        color="black",
        marker="o",
        linestyle="--",
        label="row_groups_touched / row_groups_total",
    )
    ax2.set_ylabel("row_groups_touched / row_groups_total")
    ax2.set_ylim(0, 1.05)

    lines1, labels1 = ax1.get_legend_handles_labels()
    lines2, labels2 = ax2.get_legend_handles_labels()
    ax1.legend(lines1 + lines2, labels1 + labels2, fontsize="small", loc="upper right")

    ax1.set_title(f"{name}: row-group size — window-query time & pruning")
    fig.tight_layout()
    fig.savefig(plots_dir / f"{name}-rowgroup.png", dpi=150)
    plt.close(fig)


def plot_dataset(name: str, df: pd.DataFrame, plots_dir: Path) -> None:
    annotated = annotate(name, df)
    plot_codec_size(name, annotated, plots_dir)
    plot_codec_time(name, annotated, plots_dir)
    plot_rowgroup(name, annotated, plots_dir)


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
            print(f"skip {csv_path.name} (not a write-bench CSV)", file=sys.stderr)
            continue
        datasets[csv_path.stem] = df

    if not datasets:
        print(f"no write-bench CSVs found in {results_dir}", file=sys.stderr)
        return 1

    for name, df in datasets.items():
        plot_dataset(name, df, plots_dir)
        print(f"wrote {plots_dir / (name + '-codec-size.png')}")
        print(f"wrote {plots_dir / (name + '-codec-time.png')}")
        print(f"wrote {plots_dir / (name + '-rowgroup.png')}")

    return 0


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="readbench_plot.compression",
        description=(
            "Render compression-codec and row-group-size comparison charts "
            "from cityparquet bench write-bench result CSVs (e.g. "
            "benchmark/formats/compression_results)."
        ),
    )
    parser.add_argument(
        "results_dir",
        type=Path,
        help="directory containing write-bench result CSVs (e.g. benchmark/formats/compression_results)",
    )
    args = parser.parse_args()
    sys.exit(run(args.results_dir))


if __name__ == "__main__":
    main()
