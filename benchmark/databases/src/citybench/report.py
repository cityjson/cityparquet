"""The results CSV contract.

The first thirteen columns (``dataset`` through ``http_requests``) match the
format harness's committed CSVs (``benchmark/runs/formats/results/*.csv``) in
name and order; ``bytes_read``/``http_requests`` are always empty here.
``server_time_s``/``size_bytes``/``size_bytes_no_index``/``status`` and the
raw-sample columns are this harness's own, added once server-bound databases
entered the comparison, and the format harness's last three columns are its
lookup counters. Concatenating the two harnesses' rows is therefore a
column mapping — trivial and lossless, but not "no transformation".
"""

from __future__ import annotations

import csv
import json
from pathlib import Path

from citybench.config import Measurement
from citybench.stats import mean, standard_deviation

COLUMNS: tuple[str, ...] = (
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    "time_s",
    "time_std_s",
    "peak_heap_bytes",
    "peak_rss_bytes",
    "repeat",
    "notes",
    "bytes_read",
    "http_requests",
    "server_time_s",
    "size_bytes",
    "size_bytes_no_index",
    "status",
    "raw_time_samples_s",
    "raw_server_time_samples_s",
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
    status: str | None = None,
) -> dict[str, str]:
    """One CSV row from one system's repeated samples of one scenario.

    The two size figures describe the system, not the scenario, and are
    repeated on every row so the CSV can be plotted without a join.

    ``status`` is the runner's verdict on the cross-system count check —
    ``"ok"``, ``"ok-deviation"`` or ``"mismatch"``. It is supplied rather
    than re-derived here because ``ok-deviation`` is not visible in the
    notes: a deviating row keeps its full ``count-mismatch: ...`` detail
    (the decomposition is the point), and only the status says whether the
    spread was inside the run's stated tolerance. ``error:`` and
    ``skipped:`` notes still win over it — a system that never answered has
    no count to have deviated.
    """
    times = measurement.times_s
    server = measurement.server_times_s
    return {
        "dataset": dataset,
        "format": fmt,
        "scenario": scenario,
        "selectivity": _fmt(selectivity),
        "result_count": _int(measurement.result_count),
        "time_s": _fmt(mean(times)) if times else "",
        "time_std_s": _fmt(standard_deviation(times)) if times else "",
        "peak_heap_bytes": _int(measurement.peak_heap_bytes),
        "peak_rss_bytes": _int(measurement.peak_rss_bytes),
        "repeat": str(len(times)),
        "notes": measurement.notes,
        # Always empty: this harness measures local transport only.
        "bytes_read": "",
        "http_requests": "",
        "server_time_s": _fmt(mean(server)) if server else "",
        "size_bytes": _int(size_bytes),
        "size_bytes_no_index": _int(size_bytes_no_index),
        "status": ("error" if measurement.notes.startswith("error:") else
                   "skipped" if measurement.notes.startswith("skipped:") else
                   status if status is not None else
                   "mismatch" if "count-mismatch" in measurement.notes else "ok"),
        "raw_time_samples_s": json.dumps(times, separators=(",", ":")),
        "raw_server_time_samples_s": json.dumps(server, separators=(",", ":")),
    }


def write_csv(path: Path, rows: list[dict[str, str]]) -> None:
    """Write ``rows`` to ``path``, replacing any existing file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=list(COLUMNS))
        writer.writeheader()
        writer.writerows(rows)
