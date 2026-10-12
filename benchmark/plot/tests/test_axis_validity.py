"""Regression coverage for invalid configuration-axis measurements."""

import csv
from pathlib import Path

from benchviz import prep


def _row(format: str, *, time: str, rss: str, status: str = "") -> dict[str, str]:
    return {
        "dataset": "slice.city.jsonl",
        "format": format,
        "scenario": "id-lookup",
        "selectivity": "",
        "result_count": "1",
        "time_mean_s": time,
        "time_std_s": "0.01",
        "time_median_s": time,
        "time_min_s": time,
        "time_max_s": time,
        "time_q1_s": time,
        "time_q3_s": time,
        "peak_heap_bytes": "50",
        "peak_rss_bytes": rss,
        "repeat": "3",
        "notes": "id-50pct",
        "status": status,
    }


def test_axis_invalid_statuses_are_null_and_reported_as_gaps(tmp_path: Path):
    output = tmp_path / "bloom_results"
    output.mkdir()
    path = output / "slice.csv"
    fields = [*prep.READ_COLUMNS, "status"]
    with path.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerows(
            [
                _row("cityparquet", time="4.0", rss="400"),
                _row("cityparquet+nobloom", time="2.0", rss="200"),
                _row("cityparquet+error", time="not-a-number", rss="bad", status="error"),
                _row("cityparquet+skipped", time="", rss="", status="skipped"),
                _row("cityparquet+mismatch", time="1.0", rss="100", status="mismatch"),
            ]
        )

    axis = prep.load_bloom_axis(output)
    by_variant = {row["variant"]: row for row in axis["records"]}
    successful = by_variant["cityparquet+nobloom"]
    assert successful["time_ratio"] == 0.5
    assert successful["rss_ratio"] == 0.5
    for variant, status in (("cityparquet+error", "error"), ("cityparquet+skipped", "skipped"), ("cityparquet+mismatch", "mismatch")):
        row = by_variant[variant]
        assert row["status"] == status
        assert row["time_s"] is None
        assert row["rss_b"] is None
        assert row["time_ratio"] is None
        assert row["rss_ratio"] is None
        assert row["notes"] == "id-50pct"
        assert any(f"{variant} id-50pct status={status}" == gap["issue"] for gap in axis["gaps"])
