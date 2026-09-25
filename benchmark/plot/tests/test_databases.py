"""The rebuilt database family, and the format family's retired `project` rows."""

import json
import shutil
from pathlib import Path

import matplotlib.pyplot as plt

from benchviz import figures, html, prep

FIXTURES = Path(__file__).parent / "fixtures"
LIVE = Path(__file__).parents[2] / "formats"


def _bench(tmp_path: Path, *, databases: bool) -> Path:
    bench = tmp_path / "benchmark" / "formats"
    shutil.copytree(FIXTURES / "benchviz", bench)
    shutil.copy(LIVE / "READ_BENCHMARK.md", bench / "READ_BENCHMARK.md")
    if databases:
        shutil.copytree(FIXTURES / "benchviz-databases", tmp_path / "benchmark" / "databases")
    return bench


def _records(db: dict, **match) -> list[dict]:
    return [r for r in db["records"] if all(r.get(k) == v for k, v in match.items())]


def test_every_row_is_keyed_by_system_scenario_and_thread_configuration(tmp_path: Path):
    db = prep.load_databases(prep.Inputs(_bench(tmp_path, databases=True)))
    assert db["dataset"] == "3dbag_n10000" and db["objects"] == 10004
    keys = [(r["format"], r["scenario"], r["threads"]) for r in db["records"]]
    assert len(keys) == len(set(keys))
    reads = _records(db, tier="read")
    assert {r["threads"] for r in reads} == {"single", "parallel"}
    # Four id probes per system and configuration, as the format family has.
    for config in ("single", "parallel"):
        probes = {r["scenario"] for r in _records(db, format="cjdb", threads=config)}
        assert {"id-10pct", "id-50pct", "id-90pct", "id-miss"} <= probes
    # The source-order control answers the windows only; the join control is DuckDB's.
    assert {r["scenario"] for r in _records(db, format="duckdb-cityparquet-source")} == {
        "bbox-1pct",
        "bbox-5pct",
        "bbox-25pct",
    }
    assert {r["format"] for r in _records(db, scenario="parts-per-building-join")} == {
        "duckdb-cityparquet"
    }
    window = _records(db, format="cjdb", scenario="bbox-5pct", threads="single")[0]
    assert window["achieved"] == 0.049980


def test_the_write_tier_is_its_own_tier_and_storage_comes_from_reads(tmp_path: Path):
    db = prep.load_databases(prep.Inputs(_bench(tmp_path, databases=True)))
    writes = _records(db, tier="write")
    assert {r["scenario"] for r in writes} == set(prep.DB_WRITE_SCENARIOS)
    assert {r["format"] for r in writes} == {
        "duckdb-cityparquet",
        "duckdb-cityparquet-writeback",
        "cjdb",
        "3dcitydb",
    }
    assert "duckdb-cityparquet-writeback" not in {s["format"] for s in db["sizes"]}
    error = _records(db, format="duckdb-cityparquet", scenario="append-object")[0]
    assert error["status"] == "error" and error["time_s"] is None


def test_an_ok_deviation_row_is_citable_and_footnoted(tmp_path: Path):
    db = prep.load_databases(prep.Inputs(_bench(tmp_path, databases=True)))
    deviated = _records(db, scenario="bbox-25pct", threads="single")
    assert {r["status"] for r in deviated} == {"ok-deviation"}
    assert all(r["deviation"].endswith("spread=0.000400") for r in deviated)
    assert all(r["time_s"] is not None for r in deviated)
    assert _records(db, scenario="bbox-25pct", threads="parallel")[0]["status"] == "ok"


def test_database_conditions_carry_the_windows_and_predicates(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path, databases=True)))
    conditions = data["meta"]["conditions"]
    assert "databases-write" not in conditions
    lines = conditions["databases"]
    windows = next(line for line in lines if line.startswith("Spatial windows"))
    assert "bbox-1pct achieved 1 %" in windows and "bbox-25pct achieved 25 %" in windows
    assert any("b3_dak_type = 'slanted'" in line for line in lines)
    assert any(line.startswith("Attribute range: b3_h_dak_max > ") for line in lines)
    # The write tier's conditions follow the reads' in the same list.
    assert any(line.startswith("Write rows: different operations, not one scale") for line in lines)
    assert any("feature-rows-added 9" in line for line in lines)


def _row(layout: dict, scenario: str) -> int:
    return layout["rows"].index(scenario)


def test_the_write_tier_is_rows_of_the_single_panel_and_n_a_in_parallel(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path, databases=True)))
    layout = figures.database_blocks(data)
    rows, systems = layout["rows"], layout["systems"]
    # Reads, one blank separator, then the write tier in the catalogue's order.
    separator = rows.index(figures.DB_WRITE_SEPARATOR)
    assert rows[separator + 1 :] == list(prep.DB_WRITE_SCENARIOS)
    assert all(q not in prep.DB_WRITE_SCENARIOS for q in rows[:separator])
    # The write-back tag is stacked inside the CityParquet (DuckDB) cell, not a column.
    assert "duckdb-cityparquet-writeback" not in systems
    duck, base = systems.index("duckdb-cityparquet"), systems.index("3dcitydb")
    for field in ("time_s", "peak_rss_bytes"):
        single = layout["blocks"][(field, "single")]
        parallel = layout["blocks"][(field, "parallel")]
        for scenario in prep.DB_WRITE_SCENARIOS:
            assert {c[1] for c in parallel[_row(layout, scenario)]} == {"n/a"}
        ratio, text, stacked, low = single[_row(layout, "attr-add")][duck]
        assert stacked and text.count("\n") == 1 and text.count("\u00d7") == 2
        assert text.split("\n")[1].startswith("+wb ")
        assert ratio is not None and low is not None and low > ratio
        # The baseline's own write cell is coloured at a ratio of one.
        assert single[_row(layout, "attr-add")][base][0] == 1.0
    time = layout["blocks"][("time_s", "single")]
    assert time[_row(layout, "append-object")][duck][1] == "error\n+wb error"
    rss = layout["blocks"][("peak_rss_bytes", "single")]
    assert rss[_row(layout, "append-object")][base][1] == "not sampled"


def test_one_database_figure_carries_reads_writes_and_the_write_footnote(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path, databases=True)))
    out = tmp_path / "figures"
    with plt.rc_context({"svg.fonttype": "none"}):
        figures.databases(data, out)
    assert not (out / "databases-write.svg").exists()
    assert not hasattr(figures, "databases_write")
    svg = (out / "databases.svg").read_text(encoding="utf-8")
    for text in (
        "threads=single (primary)",
        "threads=parallel (disclosed second pass)",
        "Spatial 25 %",
        "Id lookup (miss)",
        "Parts per building (join)",
        "(DuckDB,",
        "source order)",
        "*1",
        "ok-deviation, Spatial 25 %, threads=single",
        ">mismatch<",
        ">skipped<",
        ">n/a<",
        "write tier",
        "Add attribute",
        "Update attribute",
        "Delete attribute",
        "Append one building",
        ">error<",
        "not one scale",
        "each system does different",
        "Area expression",
        "feature-rows-added 9",
        "duckdb-cityparquet-writeback",
    ):
        assert text in svg, text


def test_the_page_lists_one_database_figure_with_its_conditions(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path, databases=True)))
    data_path = tmp_path / "bench_data.json"
    data_path.write_text(json.dumps(data), encoding="utf-8")
    figures_dir = tmp_path / "figures"
    figures.databases(data, figures_dir)
    page = html.main(data_path, tmp_path / "index.html", figures_dir).read_text(encoding="utf-8")
    assert "Database write tier" not in page and "databases-write" not in html.ORDER
    section = page.split("<h2>Database comparison</h2>", 1)[1].split("</section>", 1)[0]
    assert "<img" in section and "bbox-5pct achieved" in section and "not one scale" in section


def test_retired_project_rows_are_ignored_not_fatal(tmp_path: Path):
    bench = _bench(tmp_path, databases=False)
    assert ",project," in (bench / "read_results" / "delft.csv").read_text()
    data, anomalies = prep.build(prep.Inputs(bench))
    assert "project" not in {r["scenario_key"] for r in data["read"]}
    assert any("retired scenario 'project'" in note for note in anomalies)
    assert "project" not in figures.QUERIES and "project" not in figures.SCENARIO_LABELS


def test_format_conditions_print_the_predicate_and_the_write_baseline(tmp_path: Path):
    bench = _bench(tmp_path, databases=False)
    inputs = prep.Inputs(bench)
    (inputs.read_dir / "delft.csv.params.json").write_text(
        json.dumps(
            {
                "attr_filter": {
                    "column": "TerrainHeight",
                    "pred": {"ge": 2.45},
                    "matched": 217,
                    "share": 0.2544,
                    "hand_picked": True,
                },
                "windows": [{"tag": "bbox-1pct", "achieved": 0.0106, "approx": False}],
            }
        )
    )
    records = [
        {"dataset": "delft", "scenario_key": "attr-filter", "notes": "attr=TerrainHeight>=2.45"},
        {
            "dataset": "delft",
            "scenario_key": "write",
            "notes": "canonical-cityjsonseq;cityjsonseq=readbench-reserialise",
        },
        {"dataset": "nyc", "scenario_key": "attr-filter", "notes": "attr=BIN=1000000;no-attr-index"},
    ]
    lines = prep.format_conditions(inputs, records)
    delft = next(line for line in lines if line.startswith("delft:"))
    assert "TerrainHeight >= 2.45 (217 matched, share 25.4 %, hand-picked)" in delft
    assert "bbox-1pct achieved 1.06 %" in delft
    assert "re-serialised by the read harness" in delft and "copy" not in delft
    # Without a sidecar the predicate comes from the notes, disclosures dropped.
    assert next(line for line in lines if line.startswith("nyc:")) == (
        "nyc: attribute filter attr=BIN=1000000"
    )
