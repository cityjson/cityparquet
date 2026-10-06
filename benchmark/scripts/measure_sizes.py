#!/usr/bin/env python3
"""Record the on-disk sizes of the five prepared format-comparison artefacts.

One row per format and dataset goes to a ``sizes.csv`` that accumulates the
corpus: a re-run replaces its own dataset's rows and keeps the others. Bytes
are the source of truth. ``mb_decimal`` is derived from them in decimal
megabytes (1 MB = 10^6 bytes). Ratios between formats are not written here;
the renderer derives them from the byte column.

A CityParquet package is a directory, and its size is every file beneath it:
the table files with their Bloom filters, the sidecar tables and
``metadata.json``. The caller names the dataset; this script never re-derives
it from a path.
"""

from __future__ import annotations

import argparse
import csv
from pathlib import Path

HEADER = ["dataset", "format", "bytes", "mb_decimal"]
FORMATS = {
    "citygml": ".gml",
    "cityjson": ".city.json",
    "cityjsonseq": ".city.jsonl",
    "flatcitybuf": ".fcb",
    "cityparquet": ".parquet",
}
BYTES_PER_MB = 1_000_000


def artefact_bytes(path: Path) -> int:
    """Size of one artefact: a file's own size, or every file under a directory.

    A missing path raises ``FileNotFoundError``; it is never counted as zero.
    """
    if path.is_dir():
        return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())
    if not path.exists():
        raise FileNotFoundError(path)
    return path.stat().st_size


def measure(prepared: Path, dataset: str) -> list[tuple[str, int]]:
    """``(format, bytes)`` for every format of one dataset, in ``FORMATS`` order."""
    rows = []
    for fmt, suffix in FORMATS.items():
        artefact = prepared / f"{dataset}{suffix}"
        if not artefact.exists():
            raise SystemExit(f"missing prepared {fmt} artefact: {artefact}")
        rows.append((fmt, artefact_bytes(artefact)))
    return rows


def kept_rows(out: Path, dataset: str) -> list[list[str]]:
    """Rows of other datasets already in ``out``; a foreign header is refused."""
    if not out.exists():
        return []
    with out.open(newline="") as stream:
        existing = list(csv.reader(stream))
    if existing and existing[0] != HEADER:
        raise SystemExit(f"unexpected size header in {out}: {existing[0]}; expected {HEADER}")
    return [row for row in existing[1:] if row and row[0] != dataset]


def write_sizes(out: Path, dataset: str, sizes: list[tuple[str, int]]) -> None:
    kept = kept_rows(out, dataset)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(HEADER)
        writer.writerows(kept)
        for fmt, size in sizes:
            writer.writerow([dataset, fmt, size, f"{size / BYTES_PER_MB:.6f}"])


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dataset", required=True, help="dataset name: the prepared artefacts' stem")
    parser.add_argument("--prepared", type=Path, required=True, help="directory of prepared artefacts")
    parser.add_argument("--out", type=Path, required=True, help="sizes.csv to create or update")
    args = parser.parse_args(argv)
    write_sizes(args.out, args.dataset, measure(args.prepared, args.dataset))


if __name__ == "__main__":
    main()
