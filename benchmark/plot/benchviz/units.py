"""Decimal byte units: 1 MB = 10^6 bytes, 1 GB = 10^9 bytes.

The one place a byte count becomes megabytes or gigabytes — for file sizes and
for memory alike — so every figure, table, CSV and stdout line of the benchmark
reads in the same unit. Raw byte columns stay the source of truth; these
helpers only derive from them.

It is standard-library only, so the benchmark scripts (which run outside this
project's environment) import it by putting `benchmark/plot` on `sys.path`.
"""

from __future__ import annotations

from collections.abc import Iterable

MB = 1_000_000
GB = 1_000_000_000


def megabytes(count: float) -> float:
    """A byte count in decimal megabytes."""
    return float(count) / MB


def mb_decimal(count: int) -> str:
    """A byte count in decimal megabytes, spelt for a CSV column (six decimals)."""
    return f"{count / MB:.6f}"


def format_bytes(count: float | None) -> str:
    """A byte count for a reader: MB below a gigabyte, GB from one."""
    if count is None:
        return "—"
    value = float(count)
    if value >= GB:
        return f"{value / GB:.2f} GB"
    return f"{value / MB:.1f} MB"


def size_unit(counts: Iterable[float | None]) -> tuple[str, int]:
    """The unit, and its divisor, a whole panel is drawn in: GB once its largest value needs one."""
    known = [float(c) for c in counts if c is not None]
    if known and max(known) >= GB:
        return "GB", GB
    return "MB", MB
