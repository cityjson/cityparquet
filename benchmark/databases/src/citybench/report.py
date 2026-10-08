"""The results CSV contract.

The first eighteen columns (``dataset`` through ``http_requests``) match the
format harness's CSVs (``benchmark/runs/formats/results/*.csv``) in
name and order; ``bytes_read``/``http_requests`` are always empty here.
the ``server_time_*`` block/``size_bytes``/``size_bytes_no_index``/``status`` and the
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
from citybench.stats import timing_summary

# The timing block both benchmark families write, in this order (see
# citybench.stats.timing_summary for the definitions).
_STATS = ("mean", "std", "median", "min", "max", "q1", "q3")
TIME_BLOCK = tuple(f"time_{s}_s" for s in _STATS)
SERVER_TIME_BLOCK = tuple(f"server_time_{s}_s" for s in _STATS)

COLUMNS: tuple[str, ...] = (
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    *TIME_BLOCK,
    "peak_heap_bytes",
    "peak_working_mem_bytes",
    "repeat",
    "notes",
    "bytes_read",
    "http_requests",
    *SERVER_TIME_BLOCK,
    "size_bytes",
    "size_bytes_no_index",
    "status",
    "raw_time_samples_s",
    "raw_server_time_samples_s",
)

_PRECISION = 6


def _fmt(value: float | None) -> str:
    return "" if value is None else f"{value:.{_PRECISION}f}"


def _block(prefix: str, samples: list[float]) -> dict[str, str]:
    """One timing block's cells; all empty when there are no samples."""
    if not samples:
        return {f"{prefix}_{s}_s": "" for s in _STATS}
    stats = timing_summary(samples)
    return {f"{prefix}_{s}_s": _fmt(stats[s]) for s in _STATS}


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
        **_block("time", times),
        "peak_heap_bytes": _int(measurement.peak_heap_bytes),
        "peak_working_mem_bytes": _int(measurement.peak_working_mem_bytes),
        "repeat": str(len(times)),
        "notes": measurement.notes,
        # Always empty: this harness measures local transport only.
        "bytes_read": "",
        "http_requests": "",
        **_block("server_time", server),
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
