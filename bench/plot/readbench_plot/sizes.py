"""On-disk size + compression-ratio report from the readbench prepared dir.

`readbench_prepare.sh` (see `scripts/readbench_prepare.sh`) leaves, for every
benchmarked dataset `<name>`, one artefact per format it was asked to build
under a "prepared dir" (default `bench/data/readbench/`) — the same names
`Format::artefact` resolves in
`crates/cityparquet-readbench/src/format.rs`:

    <name>.gml               CityGML 2.0 source (only for a CityGML input)
    <name>.city.json         whole-document CityJSON
    <name>.city.jsonl        CityJSONSeq (always materialised when the
                             `cityjsonseq` format was prepared, whatever the
                             input kind — copied from a `.city.jsonl` input,
                             `cjseq cat` from anything else)
    <name>.jsonl.gz          gzip -9 of the CityJSONSeq
    <name>.fcb               FlatCityBuf file
    <name>.parquet/          CityParquet package directory (source order)
    <name>-hilbert.parquet/  CityParquet package directory, Hilbert-ordered

Which of these exist depends on the `--formats` the prepare run was given, so
every block below is conditional and a missing artefact is a skip, not an
error. `duckdb-parquet` has no artefact of its own — it is an SQL engine
reading the CityParquet package — so it never appears in this report.

Raw (uncompressed) CityJSONSeq is measured from `<name>.city.jsonl` when the
prepare run left one. When it did not, but a `.jsonl.gz` exists, the raw size
comes from that file's ISIZE trailer instead of decompressing 600 MB+ of
input: the last 4 bytes of a gzip stream hold the uncompressed size modulo
2**32 (RFC 1952 §2.3.1), and since `readbench_prepare.sh` writes a
single-member stream from an input well under 4 GiB, the modulo never wraps.

THE BASELINE, AND WHY THERE ARE TWO RATIO COLUMNS: the report's original
question was "how much smaller than raw CityJSONSeq is this?", which a
CityGML-native dataset built without any CityJSONSeq artefact simply cannot
answer — and answering it with the CityGML's size under a column named
`ratio_vs_cityjsonseq` would be a lie in the paper's own measurement
artefact. So `ratio_vs_cityjsonseq` keeps its exact meaning and is left empty
when no raw CityJSONSeq size is knowable, and a self-describing pair
(`baseline_format`, `ratio_vs_baseline`) carries the ratio that is always
computable: `baseline_format` names what the denominator actually was — raw
CityJSONSeq when there is one, otherwise the least-processed form the dataset
exists in (its CityGML, say).

Output: `<out_dir>/sizes.csv`
(`dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline`)
and two PNGs under `<out_dir>/plots/`: a grouped bar chart of size in MB per
dataset x format, and one of `ratio_vs_baseline` (baseline bytes / format
bytes; >1 means smaller than the baseline, i.e. better compression) with a
reference line at 1.0.
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

from readbench_plot import FORMAT_COLORS, FORMAT_ORDER, bar_style

# `FORMAT_ORDER` and `FORMAT_COLORS` are the package's, shared with plot.py.
# This module used to carry a five-entry ordering of its own (plot.py had a
# six-entry one) and a colour map derived from tab10 *by position*, which
# claimed consistency across figures while guaranteeing the opposite: plot.py
# passed no colour at all, and inserting a format here shifted every colour
# after it. Re-exported so `sizes.FORMAT_ORDER` keeps working for anything
# that already reads it.
__all__ = ["FORMAT_COLORS", "FORMAT_ORDER", "build_report", "run"]

# Display names for the baseline a ratio was computed against, for chart
# titles and axis labels.
_BASELINE_LABELS = {
    "citygml": "raw CityGML",
    "cityjson": "raw CityJSON",
    "cityjsonseq": "raw CityJSONSeq",
    "cityjsonseq-gz": "gzipped CityJSONSeq",
}


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


# Every artefact suffix a dataset name can be recovered from, longest first
# so `.city.jsonl` is never mistaken for `.city.json` plus a stray `l`.
# `-hilbert.parquet` is deliberately absent: a Hilbert-ordered package is a
# per-format artefact of an already-named dataset, not a dataset of its own,
# and is stripped separately below.
_NAME_SUFFIXES = [
    ".city.jsonl",
    ".city.json",
    ".jsonl.gz",
    ".parquet",
    ".fcb",
    ".gml",
]


def discover_datasets(prepared_dir: Path) -> list[str]:
    """Dataset names in `prepared_dir`, one per recognised artefact.

    Seeded from EVERY artefact `Format::artefact` can produce, not just
    `.fcb`/`.jsonl.gz`: a dataset prepared from CityGML may legitimately have
    neither (its artefacts are `<name>.gml` and `<name>.city.json`), and such
    a dataset used to be invisible to this report entirely.
    """
    names: set[str] = set()
    for path in prepared_dir.iterdir():
        for suffix in _NAME_SUFFIXES:
            if not path.name.endswith(suffix):
                continue
            base = path.name[: -len(suffix)]
            # `<name>-hilbert.parquet` names `<name>`, not `<name>-hilbert`.
            if suffix == ".parquet" and base.endswith("-hilbert"):
                base = base[: -len("-hilbert")]
            if base:
                names.add(base)
            break
    return sorted(names)


def artefact_bytes(path: Path) -> int | None:
    """On-disk bytes of one artefact, or None if it was never prepared.

    A CityParquet artefact is a package *directory*, every other one a single
    file — hence the branch rather than a bare `stat()`.
    """
    if path.is_dir():
        return dir_size(path)
    if path.is_file():
        return path.stat().st_size
    return None


def measure_dataset(name: str, prepared_dir: Path) -> list[dict[str, object]]:
    """Bytes per available format for one dataset; missing artefacts are skipped.

    The per-format artefact names mirror `Format::artefact`
    (`crates/cityparquet-readbench/src/format.rs`). `duckdb-parquet` is
    absent because it has no artefact of its own (`Artefact::NotCoordinated`
    — it is an SQL engine reading the CityParquet package).
    """
    measured: dict[str, int] = {}

    def measure(fmt: str, path: Path) -> None:
        size = artefact_bytes(path)
        if size is None:
            print(f"warn: {name}: no {path.name}, skipping {fmt}", file=sys.stderr)
            return
        measured[fmt] = size

    measure("citygml", prepared_dir / f"{name}.gml")
    measure("cityjson", prepared_dir / f"{name}.city.json")

    # Raw CityJSONSeq: measured directly when the prepare run left a
    # `.city.jsonl`, otherwise inferred from the gzip trailer. Reading a real
    # file beats inferring a size, so the file wins when both exist.
    seq_path = prepared_dir / f"{name}.city.jsonl"
    gz_path = prepared_dir / f"{name}.jsonl.gz"
    if seq_path.is_file():
        measured["cityjsonseq"] = seq_path.stat().st_size
    elif gz_path.is_file():
        measured["cityjsonseq"] = gzip_isize(gz_path)
    else:
        print(f"warn: {name}: no {seq_path.name} or {gz_path.name}, skipping cityjsonseq",
              file=sys.stderr)

    measure("cityjsonseq-gz", gz_path)
    measure("flatcitybuf", prepared_dir / f"{name}.fcb")
    measure("cityparquet", prepared_dir / f"{name}.parquet")
    measure("cityparquet-hilbert", prepared_dir / f"{name}-hilbert.parquet")

    baseline_format, baseline_bytes = _baseline(measured)
    raw_seq = measured.get("cityjsonseq")

    rows: list[dict[str, object]] = []
    for fmt in FORMAT_ORDER:
        if fmt not in measured:
            continue
        size = measured[fmt]
        rows.append(
            {
                "dataset": name,
                "format": fmt,
                "bytes": size,
                # Keeps its exact original meaning — bytes of raw CityJSONSeq
                # per byte of this format — and stays empty rather than
                # quietly changing denominator when there is no CityJSONSeq.
                "ratio_vs_cityjsonseq": raw_seq / size if raw_seq is not None else float("nan"),
                "baseline_format": baseline_format,
                "ratio_vs_baseline": (
                    baseline_bytes / size if baseline_bytes is not None else float("nan")
                ),
            }
        )
    return rows


def _baseline(measured: dict[str, int]) -> tuple[str, int | None]:
    """Which measured format the compression ratio is taken against.

    Raw CityJSONSeq when the dataset has one, so the number stays comparable
    with every figure published before CityGML joined the benchmark;
    otherwise the least-processed form the dataset exists in (the earliest
    entry of the canonical `FORMAT_ORDER` that was measured — its CityGML,
    typically). `("", None)` if nothing at all was measured.
    """
    if "cityjsonseq" in measured:
        return "cityjsonseq", measured["cityjsonseq"]
    for fmt in FORMAT_ORDER:
        if fmt in measured:
            return fmt, measured[fmt]
    return "", None


def build_report(prepared_dir: Path) -> pd.DataFrame:
    names = discover_datasets(prepared_dir)
    rows: list[dict[str, object]] = []
    for name in names:
        rows.extend(measure_dataset(name, prepared_dir))

    df = pd.DataFrame(
        rows,
        columns=[
            "dataset",
            "format",
            "bytes",
            "ratio_vs_cityjsonseq",
            "baseline_format",
            "ratio_vs_baseline",
        ],
    )
    df["bytes"] = cast(pd.Series, pd.to_numeric(df["bytes"], errors="coerce")).astype(
        "int64"
    )
    df["mb"] = df["bytes"] / (1024 * 1024)
    df["ratio_vs_cityjsonseq"] = pd.to_numeric(df["ratio_vs_cityjsonseq"], errors="coerce")
    df["ratio_vs_baseline"] = pd.to_numeric(df["ratio_vs_baseline"], errors="coerce")
    return cast(
        pd.DataFrame,
        df[
            [
                "dataset",
                "format",
                "bytes",
                "mb",
                "ratio_vs_cityjsonseq",
                "baseline_format",
                "ratio_vs_baseline",
            ]
        ],
    )


def _grouped_bar(
    df: pd.DataFrame,
    value_col: str,
    title: str,
    ylabel: str,
    out_path: Path,
    *,
    ref_line: float | None = None,
    ref_label: str | None = None,
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
        ax.bar(offsets, heights, width=width, label=fmt, **bar_style(fmt))

    if ref_line is not None:
        ax.axhline(
            ref_line, color="black", linestyle="--", linewidth=1, label=ref_label or "baseline"
        )

    ax.set_xticks(list(x))
    ax.set_xticklabels(datasets, rotation=20, ha="right")
    ax.set_ylabel(ylabel)
    ax.set_title(title)
    ax.legend(title="format", fontsize="small")
    fig.tight_layout()
    fig.savefig(out_path, dpi=150)
    plt.close(fig)


def baseline_label(df: pd.DataFrame) -> str:
    """How to name the ratio's denominator on a chart.

    One label when every dataset shares a baseline; a generic one when a run
    mixes, say, a CityJSONSeq-native dataset with a CityGML-native one — the
    per-dataset truth is in `sizes.csv`'s `baseline_format` column, which is
    why that column exists.
    """
    used = sorted({b for b in df["baseline_format"] if b})
    if len(used) == 1:
        return _BASELINE_LABELS.get(used[0], f"raw {used[0]}")
    return "each dataset's source format"


def plot_sizes(df: pd.DataFrame, plots_dir: Path) -> None:
    label = baseline_label(df)
    _grouped_bar(
        df,
        "mb",
        "On-disk size per format",
        "size (MB)",
        plots_dir / "sizes.png",
    )
    _grouped_bar(
        df,
        "ratio_vs_baseline",
        # The baseline goes in the title and the "higher = smaller" hint in
        # the y-label: with a long baseline name (a run mixing a
        # CityGML-native dataset with a CityJSONSeq-native one) both in the
        # title overflowed the 6-inch figure and clipped.
        f"Compression ratio vs {label}",
        "ratio (higher = smaller)",
        plots_dir / "compression-ratio.png",
        ref_line=1.0,
        ref_label=label,
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
            "Measure on-disk size and compression ratio per format, from a "
            "readbench prepared dir (e.g. bench/data/readbench). The ratio is "
            "taken against raw CityJSONSeq where the dataset has one and "
            "against its least-processed form (its CityGML, say) otherwise; "
            "sizes.csv's baseline_format column records which."
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
