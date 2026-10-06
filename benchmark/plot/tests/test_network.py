"""The network family: discovery, the factor table, the figures and the page."""

import csv
import json
from pathlib import Path

from benchviz import html, network

HEADER = (
    "dataset,format,scenario,selectivity,result_count,time_mean_s,time_std_s,time_median_s,"
    "time_min_s,time_max_s,time_q1_s,time_q3_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,"
    "bytes_read,http_requests,row_groups_total,bloom_pruned,stats_pruned,filter_bytes"
).split(",")


def _row(fmt, scenario, time, nbytes, requests, notes="", selectivity="1.0"):
    row = dict.fromkeys(HEADER, "")
    row.update(
        dataset="rotterdam.city.json",
        format=fmt,
        scenario=scenario,
        selectivity=selectivity,
        time_mean_s=time,
        time_median_s=time,
        repeat="3",
        notes=notes,
        bytes_read=nbytes,
        http_requests=requests,
    )
    return row


def _write(path: Path, rows: list[dict], header=HEADER) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=header)
        writer.writeheader()
        writer.writerows(rows)


def _fixture(root: Path) -> Path:
    """data_root/network/full/typical/{rotterdam.csv, params, model, bloom/}."""
    folder = root / "network" / "full" / "typical"
    _write(
        folder / "rotterdam.csv",
        [
            _row("citygml", "full-read", "1.0", "14000000", "1"),
            _row("cityparquet", "full-read", "0.25", "700000", "4"),
            _row("citygml", "attr-filter", "1.2", "14000000", "1"),
            _row("cityparquet", "attr-filter", "", "", "", notes="skipped:budget"),
        ],
    )
    params = {
        "network": {
            "profile": "typical",
            "target": "simulated",
            "bandwidth_mbps": 100.0,
            "latency_ms": 20.0,
            "burst_ms": 2.0,
            "clients": {"requests": 6, "bytes_read": 28700000},
            "server": {"requests": 6, "body_bytes": 28700000, "connections": 3},
        }
    }
    (folder / "rotterdam.csv.params.json").write_text(json.dumps(params))
    model_header = (
        "dataset,format,scenario,notes,network_profile,target,bandwidth_mbps,latency_ms,"
        "bytes_read,http_requests,time_median_s,model_s"
    ).split(",")
    model = [
        dict(zip(model_header, r))
        for r in [
            ("rotterdam.city.json", "citygml", "full-read", "", "typical", "simulated",
             "100.0", "20.0", "14000000", "1", "1.0", "1.14"),
            ("rotterdam.city.json", "cityparquet", "full-read", "", "typical", "simulated",
             "100.0", "20.0", "700000", "4", "0.25", "0.136"),
        ]
    ]
    _write(folder / "rotterdam.model.csv", model, model_header)
    _write(
        folder / "bloom" / "rotterdam.csv",
        [
            _row("cityparquet", "id-lookup", "0.10", "50000", "3", notes="id-50pct"),
            _row("cityparquet+nobloom", "id-lookup", "0.30", "400000", "6", notes="id-50pct"),
        ],
    )
    return root


def test_absent_family_is_not_measured(tmp_path):
    block = network.load(tmp_path, "full")
    assert block["measured"] is False
    assert block["records"] == []
    data = {"network": block, "meta": {}}
    names = [s[0] for s in html.sections(data)]
    assert network.NOT_MEASURED_SECTION in names


def test_load_discovers_profiles_and_keeps_missing_cells_explicit(tmp_path):
    block = network.load(_fixture(tmp_path), "full")
    assert block["measured"] is True
    (profile,) = block["profiles"]
    assert profile["name"] == "typical"
    assert profile["bandwidth_mbps"] == 100.0 and profile["latency_ms"] == 20.0
    assert profile["totals_match"] is True
    cells = {(r["format"], r["query"]): r for r in block["records"]}
    assert cells[("cityparquet", "full-read")]["model_time_s"] == 0.136
    assert cells[("cityparquet", "full-read")]["http_requests"] == 4
    skipped = cells[("cityparquet", "attr-filter")]
    assert skipped["time_s"] is None and skipped["unavailable"] == "skipped:budget"
    lookups = {(r["format"], r["query"]) for r in block["bloom"]}
    assert lookups == {("cityparquet", "id-50pct"), ("cityparquet+nobloom", "id-50pct")}


def test_factor_table_and_figures(tmp_path):
    block = network.load(_fixture(tmp_path / "data"), "full")
    out = tmp_path / "figures"
    written = network.render({"network": block, "statistic": "median"}, out)
    table = out / "network" / "network_factors.csv"
    assert table in written
    rows = {(r["format"], r["query"]): r for r in csv.DictReader(table.open())}
    assert float(rows[("cityparquet", "full-read")]["time_factor_vs_citygml"]) == 4.0
    assert float(rows[("cityparquet", "full-read")]["bytes_factor_vs_citygml"]) == 20.0
    assert rows[("cityparquet", "attr-filter")]["time_s"] == ""
    assert rows[("cityparquet", "attr-filter")]["note"] == "skipped:budget"
    assert (out / "network" / "typical" / "rotterdam" / "network.svg").exists()
    assert (out / "network" / "typical" / "rotterdam" / "network.png").exists()
    assert (out / "network" / "typical" / "network_bloom.csv").exists()
    caption = network.caption(block["profiles"][0], "median")
    assert "simulated" in caption and "100 Mbps" in caption and "20 ms" in caption
