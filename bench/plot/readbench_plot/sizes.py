"""On-disk size + compression-ratio report from the readbench prepared dir.

`readbench_prepare.sh` (see `scripts/readbench_prepare.sh`) leaves, for
every benchmarked dataset `<name>`, four (or five, once Hilbert-ordering is
counted separately) per-format artefacts under a "prepared dir" (default
`bench/data/readbench/`):

    <name>.parquet/          CityParquet package directory (source order)
    <name>-hilbert.parquet/  CityParquet package directory, Hilbert-ordered
    <name>.fcb               FlatCityBuf file
    <name>.jsonl.gz          gzip -9 of the original CityJSON/CityJSONSeq input

This module measures on-disk bytes for each artefact and adds a fifth,
synthetic "format": raw (uncompressed) CityJSONSeq. Decompressing the whole
`.jsonl.gz` just to get its size would be wasteful (this corpus has 600 MB+
inputs), so instead we read the ISIZE trailer gzip stores in the last 4
bytes of the stream: the uncompressed size modulo 2**32, per RFC 1952 §2.3.1.
Every input in this benchmark is well under 4 GiB, and `gzip -9 -c SOURCE >
*.jsonl.gz` in `readbench_prepare.sh` always writes a single-member stream,
so the modulo never wraps and ISIZE is exactly the raw size.

Output: `<out_dir>/sizes.csv` (`dataset,format,bytes,mb,ratio_vs_cityjsonseq`)
and two PNGs under `<out_dir>/plots/`: a grouped bar chart of size in MB per
dataset x format, and one of `ratio_vs_cityjsonseq` (bytes of raw
CityJSONSeq / bytes of the format; >1 means smaller than raw, i.e. better
compression) with a reference line at 1.0.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path
from typing import cast

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import pandas as pd

# Same left-to-right preference as plot.py's FORMAT_ORDER, plus the
# synthetic raw-size format; anything unrecognised is appended afterwards
# (alphabetically) rather than dropped.
FORMAT_ORDER = [
    "cityjsonseq",
    "cityjsonseq-gz",
    "flatcitybuf",
    "cityparquet",
    "cityparquet-hilbert",
]

# Consistent per-format colors, shared with plot.py's bar charts where the
# format names overlap (matplotlib's default "tab10" cycle, assigned by
# FORMAT_ORDER position so a given format keeps the same color across every
# figure this project renders).
FORMAT_COLORS = dict(zip(FORMAT_ORDER, plt.get_cmap("tab10").colors, strict=False))


def _ordered(seen: list[str], preferred: list[str]) -> list[str]:
    seen_set = set(seen)
    head = [v for v in preferred if v in seen_set]
    tail = sorted(seen_set - set(head))
    return head + tail


def gzip_isize(path: Path) -> int:
    """Uncompressed size of a single-member gzip file, from its ISIZE trailer.

    Reads only the last 4 bytes of the file (RFC 1952 §2.3.1's ISIZE field:
    the uncompressed input size modulo 2**32) instead of decompressing the
    whole stream. Correct for any single-member gzip under 4 GiB
    uncompressed, which every input in this benchmark corpus is.
    """
    with path.open("rb") as fh:
        fh.seek(-4, 2)
        (isize,) = struct.unpack("<I", fh.read(4))
    return isize


def dir_size(path: Path) -> int:
    """Recursive sum of file sizes under a CityParquet package directory."""
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def discover_datasets(prepared_dir: Path) -> list[str]:
    """Dataset names in `prepared_dir`, one per `<name>.fcb` or `<name>.jsonl.gz`.

    `<name>-hilbert.parquet` directories are a per-format artefact of an
    already-discovered dataset, not a dataset of their own, so they're
    excluded from the name set (only `.fcb`/`.jsonl.gz` seed discovery).
    """
    names: set[str] = set()
    for p in prepared_dir.glob("*.fcb"):
        names.add(p.stem)
    for p in prepared_dir.glob("*.jsonl.gz"):
        names.add(p.name[: -len(".jsonl.gz")])
    return sorted(names)


def measure_dataset(name: str, prepared_dir: Path) -> list[dict[str, object]]:
    """Bytes per available format for one dataset; missing artefacts are skipped."""
    rows: list[dict[str, object]] = []

    gz_path = prepared_dir / f"{name}.jsonl.gz"
    raw_bytes: int | None = None
    if gz_path.is_file():
        raw_bytes = gzip_isize(gz_path)
        rows.append({"dataset": name, "format": "cityjsonseq", "bytes": raw_bytes})
        rows.append(
            {"dataset": name, "format": "cityjsonseq-gz", "bytes": gz_path.stat().st_size}
        )
    else:
        print(f"warn: {name}: missing {gz_path.name}, skipping cityjsonseq(-gz)", file=sys.stderr)

    parquet_dir = prepared_dir / f"{name}.parquet"
    if parquet_dir.is_dir():
        rows.append({"dataset": name, "format": "cityparquet", "bytes": dir_size(parquet_dir)})
    else:
        print(f"warn: {name}: missing {parquet_dir.name}, skipping cityparquet", file=sys.stderr)

    hilbert_dir = prepared_dir / f"{name}-hilbert.parquet"
    if hilbert_dir.is_dir():
        rows.append(
            {"dataset": name, "format": "cityparquet-hilbert", "bytes": dir_size(hilbert_dir)}
        )
    else:
        print(
            f"warn: {name}: missing {hilbert_dir.name}, skipping cityparquet-hilbert",
            file=sys.stderr,
        )

    fcb_path = prepared_dir / f"{name}.fcb"
    if fcb_path.is_file():
        rows.append({"dataset": name, "format": "flatcitybuf", "bytes": fcb_path.stat().st_size})
    else:
        print(f"warn: {name}: missing {fcb_path.name}, skipping flatcitybuf", file=sys.stderr)

    if raw_bytes is not None:
        for row in rows:
            row["ratio_vs_cityjsonseq"] = raw_bytes / cast(int, row["bytes"])
    else:
        for row in rows:
            row["ratio_vs_cityjsonseq"] = float("nan")

    return rows


def build_report(prepared_dir: Path) -> pd.DataFrame:
    names = discover_datasets(prepared_dir)
    rows: list[dict[str, object]] = []
    for name in names:
        rows.extend(measure_dataset(name, prepared_dir))

    df = pd.DataFrame(rows, columns=["dataset", "format", "bytes", "ratio_vs_cityjsonseq"])
    df["bytes"] = cast(pd.Series, pd.to_numeric(df["bytes"], errors="coerce")).astype(
        "int64"
    )
    df["mb"] = df["bytes"] / (1024 * 1024)
    df["ratio_vs_cityjsonseq"] = pd.to_numeric(df["ratio_vs_cityjsonseq"], errors="coerce")
    return cast(
        pd.DataFrame, df[["dataset", "format", "bytes", "mb", "ratio_vs_cityjsonseq"]]
    )


def _grouped_bar(
    df: pd.DataFrame,
    value_col: str,
    title: str,
    ylabel: str,
    out_path: Path,
    *,
    ref_line: float | None = None,
) -> None:
    datasets = _ordered(list(df["dataset"].unique()), sorted(df["dataset"].unique()))
    formats = _ordered(list(df["format"].unique()), FORMAT_ORDER)

    pivot = df.pivot(index="dataset", columns="format", values=value_col)
    pivot = pivot.reindex(index=datasets, columns=formats)

    fig, ax = plt.subplots(figsize=(max(6, len(datasets) * 1.6), 4.5))
    n_formats = len(formats)
    width = 0.8 / max(n_formats, 1)
    x = range(len(datasets))

    for j, fmt in enumerate(formats):
        values = pivot[fmt].tolist()
        if all(pd.isna(v) for v in values):
            continue
        offsets = [xi + (j - (n_formats - 1) / 2) * width for xi in x]
        heights = [v if pd.notna(v) else 0 for v in values]
        ax.bar(offsets, heights, width=width, label=fmt, color=FORMAT_COLORS.get(fmt))

    if ref_line is not None:
        ax.axhline(ref_line, color="black", linestyle="--", linewidth=1, label="raw CityJSONSeq")

    ax.set_xticks(list(x))
    ax.set_xticklabels(datasets, rotation=20, ha="right")
    ax.set_ylabel(ylabel)
    ax.set_title(title)
    ax.legend(title="format", fontsize="small")
    fig.tight_layout()
    fig.savefig(out_path, dpi=150)
    plt.close(fig)


def plot_sizes(df: pd.DataFrame, plots_dir: Path) -> None:
    _grouped_bar(
        df,
        "mb",
        "On-disk size per format",
        "size (MB)",
        plots_dir / "sizes.png",
    )
    _grouped_bar(
        df,
        "ratio_vs_cityjsonseq",
        "Compression ratio vs raw CityJSONSeq (higher = smaller)",
        "ratio vs raw CityJSONSeq",
        plots_dir / "compression-ratio.png",
        ref_line=1.0,
    )


def run(prepared_dir: Path, out_dir: Path) -> int:
    if not prepared_dir.is_dir():
        print(f"error: not a directory: {prepared_dir}", file=sys.stderr)
        return 1

    df = build_report(prepared_dir)
    if df.empty:
        print(f"no datasets found in {prepared_dir}", file=sys.stderr)
        return 1

    out_dir.mkdir(parents=True, exist_ok=True)
    csv_path = out_dir / "sizes.csv"
    df.to_csv(csv_path, index=False)
    print(f"wrote {csv_path}")

    plots_dir = out_dir / "plots"
    plots_dir.mkdir(exist_ok=True)
    plot_sizes(df, plots_dir)
    print(f"wrote {plots_dir / 'sizes.png'}")
    print(f"wrote {plots_dir / 'compression-ratio.png'}")

    return 0


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="readbench_plot.sizes",
        description=(
            "Measure on-disk size and compression ratio (vs raw CityJSONSeq) "
            "per format, from a readbench prepared dir (e.g. bench/data/readbench)."
        ),
    )
    parser.add_argument(
        "prepared_dir",
        type=Path,
        help="directory of per-format readbench artefacts (see scripts/readbench_prepare.sh)",
    )
    parser.add_argument(
        "out_dir",
        type=Path,
        nargs="?",
        default=Path("bench/read_results"),
        help="output directory for sizes.csv and plots/ (default: bench/read_results)",
    )
    args = parser.parse_args()
    sys.exit(run(args.prepared_dir, args.out_dir))


if __name__ == "__main__":
    main()
