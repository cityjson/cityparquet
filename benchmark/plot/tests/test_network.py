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


DERIVED = "derived-from=full-read;status=derived"


def _derived_fixture(root: Path) -> Path:
    """A run whose CityGML spatial cell is derived from its proven read all."""
    folder = root / "network" / "full" / "typical"
    derived = _row("citygml", "bbox-query", "", "14000000", "1", notes=f"bbox-1pct;{DERIVED}", selectivity="")
    derived["repeat"] = "0"
    _write(
        folder / "rotterdam.csv",
        [
            _row("citygml", "full-read", "1.0", "14000000", "1"),
            _row("cityparquet", "full-read", "0.25", "700000", "4"),
            _row("cityparquet", "bbox-query", "0.05", "70000", "3", notes="bbox-1pct"),
            derived,
        ],
    )
    return root


def test_derived_cell_keeps_transfer_and_no_time(tmp_path):
    block = network.load(_derived_fixture(tmp_path / "data"), "full")
    cell = next(r for r in block["records"] if r["format"] == "citygml" and r["query"] == "bbox-1pct")
    assert cell["derived"] is True
    assert cell["time_s"] is None and cell["unavailable"] is None
    assert (cell["bytes_read"], cell["http_requests"]) == (14000000, 1)
    measured = next(r for r in block["records"] if r["format"] == "cityparquet" and r["query"] == "bbox-1pct")
    assert measured["derived"] is False


def test_factor_table_marks_derived_cells(tmp_path):
    block = network.load(_derived_fixture(tmp_path / "data"), "full")
    out = tmp_path / "figures"
    network.render({"network": block, "statistic": "median"}, out)
    rows = {(r["format"], r["query"]): r for r in csv.DictReader((out / "network" / "network_factors.csv").open())}
    derived = rows[("citygml", "bbox-1pct")]
    assert derived["derived"] == "full-read"
    assert derived["time_s"] == "" and derived["time_factor_vs_citygml"] == ""
    assert derived["bytes_read"] == "14000000" and derived["http_requests"] == "1"
    assert "same transfer as read all" in derived["note"]
    measured = rows[("cityparquet", "bbox-1pct")]
    assert measured["derived"] == ""
    assert float(measured["bytes_factor_vs_citygml"]) == 200.0
    # The time factor needs a measured CityGML time; a derived one is not.
    assert measured["time_factor_vs_citygml"] == ""


def test_figure_shows_derived_time_as_transfer_marker(tmp_path):
    block = network.load(_derived_fixture(tmp_path / "data"), "full")
    grid = network.figure_grid(block["records"])
    time_text = grid["cells"][("time_s", "citygml", "bbox-1pct")]["text"]
    assert time_text == network.DERIVED_TIME_TEXT and "0" not in time_text
    assert grid["cells"][("bytes_read", "citygml", "bbox-1pct")]["text"].startswith("14 MB")
    assert grid["cells"][("http_requests", "citygml", "bbox-1pct")]["text"].startswith("1")
    # Derived rows sort with their query, not after every measured row.
    assert grid["queries"] == ["full-read", "bbox-1pct"]
    caption = network.caption(block["profiles"][0], "median", derived=True)
    assert network.DERIVED_MARK in caption and "one whole-object GET" in caption


def test_page_section_states_the_derived_rows(tmp_path):
    block = network.load(_derived_fixture(tmp_path / "data"), "full")
    (_key, _title, lines), = network.sections({"network": block, "statistic": "median"})
    assert network.DERIVED_FOOTNOTE in lines[0]
    # bench_data.json carries the block as loaded: every record says whether it is derived.
    assert json.loads(json.dumps(block))["records"][-1]["derived"] in (True, False)


def test_figure_uses_display_labels_and_the_factor_colours(tmp_path):
    from matplotlib import colors

    from benchviz import figures

    block = network.load(_derived_fixture(tmp_path / "data"), "full")
    grid = network.figure_grid(block["records"])
    assert grid["format_labels"] == ["CityGML", "CityParquet"]
    assert grid["query_labels"] == ["Read all", "Spatial 1 %"]
    cells = grid["cells"]

    def rgb(key):
        return colors.to_rgb(cells[key]["colour"])

    # CityGML is the baseline: 1x is the page colour.
    assert cells[("time_s", "citygml", "full-read")]["colour"] == figures.BG
    # Better than CityGML: teal (green dominates red); the absolute value stays printed.
    r, g, _b = rgb(("time_s", "cityparquet", "full-read"))
    assert g > r
    assert cells[("time_s", "cityparquet", "full-read")]["text"].startswith("0.25 s")
    # Request counts and derived times are never coloured as factors.
    assert cells[("http_requests", "cityparquet", "full-read")]["colour"] == figures.BG
    assert cells[("time_s", "citygml", "bbox-1pct")]["colour"] == figures.BG


def test_worse_than_citygml_is_the_warm_accent(tmp_path):
    from matplotlib import colors

    block = network.load(_fixture(tmp_path / "data"), "full")
    for r in block["records"]:
        if r["format"] == "cityparquet" and r["query"] == "full-read":
            r["bytes_read"] = 28000000  # twice CityGML's
    r, g, _b = colors.to_rgb(network.figure_grid(block["records"])["cells"][("bytes_read", "cityparquet", "full-read")]["colour"])
    assert r > g


def test_a_real_target_caption_says_one_snapshot_and_names_the_path():
    text = network.caption({"name": "real", "target": "real", "base_url": "https://h/v8", "host": "h", "resolved_ips": ["192.0.2.1"]}, "median")
    assert "real object storage, one snapshot" in text
    assert "https://h/v8" in text and "192.0.2.1" in text
    assert "real object storage, one snapshot" not in network.caption({"name": "typical", "target": "simulated", "bandwidth_mbps": 100, "latency_ms": 20}, "median")
