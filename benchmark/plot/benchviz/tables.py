"""bench_data.json -> the format comparison's ratios as CSV tables.

Three tables, written under ``<figures>/formats/`` beside the figures they
belong to, so ``--figures DIR`` moves them with the rest:

- ``size_factors.csv``: one row per dataset — its CityObject count, CityGML's
  bytes, then each other format's bytes and its factor.
- ``size_extremes.csv``: the best and worst dataset by CityParquet's factor.
- ``query_factors.csv``: one row per dataset, query and format — the absolute
  time and peak memory, CityGML's, and the two factors.

Every factor is CityGML's value divided by the format's, so a larger factor is
better: "CityParquet is 5 times faster (or smaller) than CityGML". A factor whose
CityGML or format cell is missing, skipped or failed is an EMPTY cell, never a
zero and never a ratio against another format; the query table's ``note`` says
why. Nothing here names a dataset: the rows follow ``bench_data.json``.

stdlib only, like ``prep``.
"""

from __future__ import annotations

import csv
from pathlib import Path

from . import prep

# The size table's columns after CityGML: (column prefix, format id).
SIZE_FORMATS = {
    "cityjson": "cityjson",
    "cityjsonseq": "cityjsonseq",
    "flatcitybuf": "flatcitybuf",
    "cityparquet": "cityparquet-hilbert",
}
# The format comparison's sub-folder of the figures directory, and its
# per-dataset metrics: (file name, record field, page title, axis title).
FORMATS_DIR = "formats"
FORMAT_METRICS = (
    ("time", "time_s", "read time", "Read time (s)"),
    ("rss", "rss_b", "read peak memory", "Read peak RSS (MiB)"),
)
ORIENTATION = (
    "Every factor is CityGML's value divided by the format's, so a larger factor is "
    "better (faster, leaner or smaller than CityGML). An empty factor is unavailable: "
    "the CityGML or the format cell is missing, skipped or failed; the note says which."
)
SIZE_TABLE = "size_factors.csv"
SIZE_EXTREMES = "size_extremes.csv"
QUERY_TABLE = "query_factors.csv"


def size_table(data: dict) -> list[dict]:
    """One row per dataset: CityGML's bytes, then each format's bytes and factor."""
    by = {(r["dataset"], r["format"]): r for r in data.get("sizes", [])}
    rows = []
    for dataset in data.get("datasets", []):
        name = dataset["id"]
        if not any(key[0] == name for key in by):
            continue
        row = {
            "dataset": name,
            "title": dataset.get("title") or name,
            "city_objects": dataset.get("objects"),
            "citygml_bytes": by.get((name, prep.BASELINE_FORMAT), {}).get("bytes"),
        }
        for prefix, fmt in SIZE_FORMATS.items():
            record = by.get((name, fmt), {})
            row[f"{prefix}_bytes"] = record.get("bytes")
            row[f"{prefix}_factor"] = record.get("factor")
        rows.append(row)
    return rows


def size_extremes(rows: list[dict]) -> list[dict]:
    """The datasets with the largest and the smallest CityParquet factor."""
    known = [r for r in rows if r.get("cityparquet_factor") is not None]
    if not known:
        return []
    picks = (
        ("best", max(known, key=lambda r: r["cityparquet_factor"])),
        ("worst", min(known, key=lambda r: r["cityparquet_factor"])),
    )
    return [
        {
            "rank": rank,
            "dataset": row["dataset"],
            "title": row["title"],
            "city_objects": row["city_objects"],
            "cityparquet_factor": row["cityparquet_factor"],
        }
        for rank, row in picks
    ]


def dataset_queries(records: list[dict]) -> list[str]:
    """The queries a dataset measured, in the preferred order."""
    present = {r["scenario_key"] for r in records if r.get("scenario_key")}
    ordered = [q for q in prep.QUERY_ORDER if q in present]
    return ordered + sorted(present - set(ordered))


def query_table(data: dict) -> list[dict]:
    """One row per dataset, query and format, with both factors against CityGML."""
    rows = []
    for dataset in data.get("datasets", []):
        name = dataset["id"]
        records = [r for r in data.get("read", []) if r.get("dataset") == name]
        index = {(r["format"], r["scenario_key"]): r for r in records}
        for query in dataset_queries(records):
            base = index.get((prep.BASELINE_FORMAT, query))
            base_reason = (base or {}).get("unavailable") if base else "not measured"
            for fmt in prep.FIGURE_FORMATS:
                record = index.get((fmt, query))
                if record is None:
                    note = "not measured"
                elif record.get("unavailable"):
                    note = record["unavailable"]
                elif base_reason:
                    note = f"CityGML unavailable: {base_reason}"
                else:
                    note = ""
                record = record or {}
                rows.append(
                    {
                        "dataset": name,
                        "query": query,
                        "format": prep.FORMAT_NAMES.get(fmt, fmt),
                        "format_id": fmt,
                        "time_s": record.get("time_s"),
                        "time_std_s": record.get("time_std_s"),
                        "peak_rss_bytes": record.get("rss_b"),
                        "citygml_time_s": record.get("base_time_s"),
                        "citygml_peak_rss_bytes": record.get("base_rss_b"),
                        "time_factor_vs_citygml": record.get("time_factor"),
                        "rss_factor_vs_citygml": record.get("rss_factor"),
                        "note": note,
                    }
                )
    return rows


def _cell(column: str, value) -> str:
    """A CSV cell: unavailable is empty, never zero."""
    if value is None:
        return ""
    if isinstance(value, float):
        if column.endswith("_factor") or column.endswith("_vs_citygml"):
            return f"{value:.4f}"
        if column.endswith("_bytes"):
            return str(int(value))
        return f"{value:.6f}"
    return str(value)


def _write(path: Path, rows: list[dict], columns: list[str]) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(columns)
        for row in rows:
            writer.writerow([_cell(c, row.get(c)) for c in columns])
    return path


def write_tables(data: dict, out: Path) -> list[Path]:
    """Write the three tables under `out/formats/`."""
    folder = out / FORMATS_DIR
    sizes = size_table(data)
    size_columns = ["dataset", "title", "city_objects", "citygml_bytes"]
    for prefix in SIZE_FORMATS:
        size_columns += [f"{prefix}_bytes", f"{prefix}_factor"]
    queries = query_table(data)
    query_columns = [
        "dataset",
        "query",
        "format",
        "format_id",
        "time_s",
        "time_std_s",
        "peak_rss_bytes",
        "citygml_time_s",
        "citygml_peak_rss_bytes",
        "time_factor_vs_citygml",
        "rss_factor_vs_citygml",
        "note",
    ]
    return [
        _write(folder / SIZE_TABLE, sizes, size_columns),
        _write(
            folder / SIZE_EXTREMES,
            size_extremes(sizes),
            ["rank", "dataset", "title", "city_objects", "cityparquet_factor"],
        ),
        _write(folder / QUERY_TABLE, queries, query_columns),
    ]
