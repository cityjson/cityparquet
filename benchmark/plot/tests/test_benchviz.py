"""Regression checks for the paper-focused benchmark renderer."""

import csv
import json
import shutil
from pathlib import Path

from benchviz import figures, prep

FIXTURE = Path(__file__).parent / "fixtures" / "benchviz"
LIVE = Path(__file__).parents[2] / "formats"


def fixture_bench(tmp_path: Path) -> Path:
    bench = tmp_path / "benchmark" / "formats"
    shutil.copytree(FIXTURE, bench)
    shutil.copy(LIVE / "READ_BENCHMARK.md", bench / "READ_BENCHMARK.md")
    return bench


def test_preparation_and_paper_figure_set(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    path = tmp_path / "bench_data.json"
    path.write_text(prep.json.dumps(data), encoding="utf-8")
    output = figures.main(path, tmp_path / "figures")
    names = {p.name for p in output.glob("*")}
    expected = {
        f"{name}.{kind}"
        for name in ("sizes", "heatmap", "codec", "codec-scaling", "rowgroup", "rowgroup-scaling")
        for kind in ("svg", "png")
    }
    assert expected <= names
    assert not any("pareto" in name for name in names)


def test_cityparquet_label_and_missing_database_are_honest(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    assert figures._label("cityparquet-hilbert") == "CityParquet"
    assert data["databases"] == {"baseline": "3dcitydb", "records": [], "sizes": []}


DB_COLUMNS = [
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
    "status",
]


def _db_csv(path: Path, rows: list[dict]) -> None:
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=DB_COLUMNS)
        writer.writeheader()
        writer.writerows(rows)


def test_database_loader_selects_largest_and_preserves_bbox_keys(tmp_path: Path):
    root = tmp_path / "benchmark"
    smoke = root / "databases" / "smoke"
    smoke.mkdir(parents=True)
    base = {key: "" for key in DB_COLUMNS}
    _db_csv(
        smoke / "small.csv",
        [
            {
                **base,
                "dataset": "small",
                "format": "cjdb",
                "scenario": "bbox-query",
                "notes": "bbox-1pct",
                "status": "ok",
            }
        ],
    )
    (smoke / "small.params.json").write_text(json.dumps({"total_city_objects": 10}))
    rows = []
    for tag in ("bbox-1pct", "bbox-5pct", "bbox-25pct"):
        rows.append(
            {
                **base,
                "dataset": "large",
                "format": "3dcitydb",
                "scenario": "bbox-query",
                "notes": tag,
                "status": "ok",
                "time_s": "2.0",
                "peak_rss_bytes": "100",
                "size_bytes": "20",
            }
        )
    rows.append(
        {
            **base,
            "dataset": "large",
            "format": "cjdb",
            "scenario": "bbox-query",
            "notes": "bbox-5pct",
            "status": "error",
            "time_s": "bad",
            "size_bytes": "10",
        }
    )
    _db_csv(smoke / "large.csv", rows)
    (smoke / "large.params.json").write_text(json.dumps({"total_city_objects": 100}))
    loaded = prep.load_databases(prep.Inputs(root / "formats" / "smoke"))
    assert loaded["dataset"] == "large"
    assert {r["scenario"] for r in loaded["records"]} == {"bbox-1pct", "bbox-5pct", "bbox-25pct"}
    error = next(r for r in loaded["records"] if r["format"] == "cjdb")
    assert error["time_s"] is None and error["status"] == "error"


def test_database_loader_uses_explicit_smoke_mode(tmp_path: Path):
    root = tmp_path / "benchmark"
    results = root / "databases" / "results"
    smoke = root / "databases" / "smoke"
    results.mkdir(parents=True)
    smoke.mkdir(parents=True)
    base = {key: "" for key in DB_COLUMNS}
    _db_csv(
        results / "full.csv",
        [{**base, "dataset": "full", "format": "3dcitydb", "scenario": "count", "status": "ok"}],
    )
    (results / "full.params.json").write_text(json.dumps({"total_city_objects": 20}))
    _db_csv(
        smoke / "smoke.csv",
        [{**base, "dataset": "smoke", "format": "3dcitydb", "scenario": "count", "status": "ok"}],
    )
    (smoke / "smoke.params.json").write_text(json.dumps({"total_city_objects": 100}))
    assert prep.load_databases(prep.Inputs(root / "formats" / "smoke"))["dataset"] == "smoke"
