"""The results CSV contract.

The first eleven columns (``dataset`` through ``notes``) match the sibling
``cityparquet-rs`` harness's own *committed* CSVs (``bench/read_results/
*.csv``) in name and order. ``bytes_read``/``http_requests`` are columns
12-13 of that harness's *documented* contract but are absent from every one
of its committed files; this harness carries them anyway (always empty
here) for forward compatibility with that documented shape.
``server_time_s``/``size_bytes``/``size_bytes_no_index`` (columns 14-16)
are genuinely new, added once server-bound databases entered the
comparison. Concatenating the sibling harness's committed rows with this
one's therefore needs five empty fields appended to each of its rows —
trivial and lossless, but not "no transformation".
"""

from __future__ import annotations

import csv
from pathlib import Path

from citybench.config import Measurement
from citybench.stats import mad, median

COLUMNS: tuple[str, ...] = (
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    "time_s",
    "time_mad_s",
    "peak_heap_bytes",
    "peak_rss_bytes",
    "repeat",
    "notes",
    "bytes_read",
    "http_requests",
    "server_time_s",
    "size_bytes",
    "size_bytes_no_index",
)

_PRECISION = 6


def _fmt(value: float | None) -> str:
    return "" if value is None else f"{value:.{_PRECISION}f}"


def _int(value: int | None) -> str:
    return "" if value is None else str(value)


def row_from_measurement(
    *,
    dataset: str,
    fmt: str,
    scenario: str,
    measurement: Measurement,
    selectivity: float | None,
    size_bytes: int | None = None,
    size_bytes_no_index: int | None = None,
) -> dict[str, str]:
    """One CSV row from one system's repeated samples of one scenario.

    The two size figures describe the system, not the scenario, and are
    repeated on every row so the CSV can be plotted without a join.
    """
    times = measurement.times_s
    server = measurement.server_times_s
    return {
        "dataset": dataset,
        "format": fmt,
        "scenario": scenario,
        "selectivity": _fmt(selectivity),
        "result_count": _int(measurement.result_count),
        "time_s": _fmt(median(times)) if times else "",
        "time_mad_s": _fmt(mad(times)) if times else "",
        "peak_heap_bytes": _int(measurement.peak_heap_bytes),
        "peak_rss_bytes": _int(measurement.peak_rss_bytes),
        "repeat": str(len(times)),
        "notes": measurement.notes,
        # Always empty: this harness measures local transport only.
        "bytes_read": "",
        "http_requests": "",
        "server_time_s": _fmt(median(server)) if server else "",
        "size_bytes": _int(size_bytes),
        "size_bytes_no_index": _int(size_bytes_no_index),
    }


def write_csv(path: Path, rows: list[dict[str, str]]) -> None:
    """Write ``rows`` to ``path``, replacing any existing file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=list(COLUMNS))
        writer.writeheader()
        writer.writerows(rows)
