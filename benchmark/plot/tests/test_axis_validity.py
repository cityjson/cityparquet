"""Regression coverage for invalid configuration-axis measurements."""

import csv
from pathlib import Path

from benchviz import prep


def _row(format: str, *, time: str, rss: str, status: str = "") -> dict[str, str]:
    return {
        "dataset": "slice.city.jsonl",
        "format": format,
        "scenario": "full-read",
        "selectivity": "",
        "result_count": "1000",
        "time_s": time,
        "time_mad_s": "0.01",
        "peak_heap_bytes": "50",
        "peak_rss_bytes": rss,
        "repeat": "3",
        "notes": "probe-note",
        "status": status,
    }


def test_axis_invalid_statuses_are_null_and_reported_as_gaps(tmp_path: Path):
    output = tmp_path / "scaling_codec_results"
    output.mkdir()
    path = output / "slice.csv"
    fields = [*prep.READ_COLUMNS, "status"]
    with path.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerows(
            [
                _row("cityparquet", time="4.0", rss="400"),
                _row("cityparquet+zstd", time="2.0", rss="200"),
                _row("cityparquet+error", time="not-a-number", rss="bad", status="error"),
                _row("cityparquet+skipped", time="", rss="", status="skipped"),
                _row("cityparquet+mismatch", time="1.0", rss="100", status="mismatch"),
            ]
        )

    axis = prep.load_scaling_axis(output)
    by_variant = {row["variant"]: row for row in axis["records"]}
    successful = by_variant["cityparquet+zstd"]
    assert successful["time_ratio"] == 0.5
    assert successful["rss_ratio"] == 0.5
    for variant, status in (("cityparquet+error", "error"), ("cityparquet+skipped", "skipped"), ("cityparquet+mismatch", "mismatch")):
        row = by_variant[variant]
        assert row["status"] == status
        assert row["time_s"] is None
        assert row["rss_b"] is None
        assert row["time_ratio"] is None
        assert row["rss_ratio"] is None
        assert row["notes"] == "probe-note"
        assert any(f"{variant} full-read status={status}" == gap["issue"] for gap in axis["gaps"])
