import csv

from citybench.config import Measurement
from citybench.report import COLUMNS, row_from_measurement, write_csv


def test_columns_match_the_inherited_contract_exactly():
    assert COLUMNS == (
        "dataset", "format", "scenario", "selectivity", "result_count",
        "time_s", "time_mad_s", "peak_heap_bytes", "peak_rss_bytes",
        "repeat", "notes", "bytes_read", "http_requests",
        "server_time_s", "size_bytes", "size_bytes_no_index",
    )


def test_size_columns_are_stamped_onto_the_row():
    m = Measurement(result_count=1, times_s=[1.0], server_times_s=[], peak_rss_bytes=None)
    row = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="count", measurement=m, selectivity=None,
        size_bytes=5000, size_bytes_no_index=4000,
    )
    assert row["size_bytes"] == "5000"
    assert row["size_bytes_no_index"] == "4000"


def test_size_columns_blank_when_unknown():
    m = Measurement(result_count=1, times_s=[1.0], server_times_s=[], peak_rss_bytes=None)
    row = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="count", measurement=m, selectivity=None,
    )
    assert row["size_bytes"] == ""
    assert row["size_bytes_no_index"] == ""


def test_row_reports_median_and_mad_at_six_decimals():
    m = Measurement(
        result_count=42,
        times_s=[0.1, 0.2, 0.3],
        server_times_s=[],
        peak_rss_bytes=None,
    )
    row = row_from_measurement(
        dataset="delft", fmt="cjdb", scenario="count",
        measurement=m, selectivity=None,
    )
    assert row["time_s"] == "0.200000"
    assert row["time_mad_s"] == "0.100000"
    assert row["result_count"] == "42"
    assert row["repeat"] == "3"


def test_local_transport_columns_are_always_empty():
    m = Measurement(result_count=1, times_s=[1.0], server_times_s=[], peak_rss_bytes=None)
    row = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="count", measurement=m, selectivity=None,
    )
    assert row["bytes_read"] == ""
    assert row["http_requests"] == ""


def test_server_time_reported_when_present():
    m = Measurement(
        result_count=1, times_s=[1.0], server_times_s=[0.4, 0.6], peak_rss_bytes=None,
    )
    row = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="count", measurement=m, selectivity=None,
    )
    assert row["server_time_s"] == "0.500000"


def test_selectivity_formatted_or_blank():
    m = Measurement(result_count=1, times_s=[1.0], server_times_s=[], peak_rss_bytes=None)
    with_sel = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="bbox-query", measurement=m, selectivity=0.25,
    )
    assert with_sel["selectivity"] == "0.250000"
    without = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="count", measurement=m, selectivity=None,
    )
    assert without["selectivity"] == ""


def test_write_csv_roundtrips(tmp_path):
    m = Measurement(result_count=7, times_s=[0.5], server_times_s=[], peak_rss_bytes=None)
    row = row_from_measurement(
        dataset="d", fmt="cjdb", scenario="count", measurement=m, selectivity=None,
    )
    out = tmp_path / "r.csv"
    write_csv(out, [row])
    with out.open() as fh:
        got = list(csv.DictReader(fh))
    assert got[0]["result_count"] == "7"
    assert list(got[0].keys()) == list(COLUMNS)
