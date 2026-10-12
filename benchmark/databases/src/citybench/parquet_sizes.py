"""Byte accounting of a CityParquet package, with and without its indexes.

A Parquet file is laid out as the magic ``PAR1``, the column-chunk data,
optional Bloom filters and page indexes (column index and offset index), the
Thrift footer, a 4-byte footer length and the magic again. The footer carries
the schema and the row-group statistics; it is what a reader needs to read
the file at all, so it is counted as data. "Without indexes" removes the
Bloom filters and the page indexes, the structures a writer can omit without
changing what the file holds. Neither pyarrow nor DuckDB exposes the page
index lengths, so they are derived: everything in the file that is not magic,
column-chunk data, Bloom filter or footer.
"""

from __future__ import annotations

from pathlib import Path

import pyarrow.parquet as pq

from citybench.config import SizeReport

_MAGIC = 4


def parquet_file_sizes(path: Path) -> dict[str, int]:
    """The byte split of one Parquet file."""
    total = path.stat().st_size
    with path.open("rb") as fh:
        fh.seek(total - 8)
        footer = int.from_bytes(fh.read(4), "little")
    meta = pq.ParquetFile(path).metadata
    data = bloom = 0
    for rg in range(meta.num_row_groups):
        group = meta.row_group(rg)
        for c in range(group.num_columns):
            chunk = group.column(c)
            data += chunk.total_compressed_size
            length = chunk.bloom_filter_length
            if length is not None and length > 0:
                bloom += length
    rest = total - 2 * _MAGIC - 4 - footer - data - bloom
    return {
        "total_bytes": total,
        "data_bytes": data,
        "bloom_filter_bytes": bloom,
        "page_index_bytes": max(rest, 0),
        "footer_bytes": footer,
    }


def package_sizes(package: Path) -> SizeReport:
    """Size of a package directory; ``size_bytes_no_index`` excludes the
    Bloom filters and page indexes of every Parquet file in it."""
    total = 0
    detail = {"bloom_filter_bytes": 0, "page_index_bytes": 0, "footer_bytes": 0}
    for f in sorted(p for p in package.rglob("*") if p.is_file()):
        total += f.stat().st_size
        if f.suffix == ".parquet":
            split = parquet_file_sizes(f)
            for key in detail:
                detail[key] += split[key]
    no_index = total - detail["bloom_filter_bytes"] - detail["page_index_bytes"]
    return SizeReport(size_bytes=total, size_bytes_no_index=no_index, detail=detail)
