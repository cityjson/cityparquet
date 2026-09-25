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
    read = data["meta"]["conditions"]["databases"]
    windows = next(line for line in read if line.startswith("Spatial windows"))
    assert "bbox-1pct achieved 1 %" in windows and "bbox-25pct achieved 25 %" in windows
    assert any("b3_dak_type = 'slanted'" in line for line in read)
    assert any(line.startswith("Attribute range: b3_h_dak_max > ") for line in read)
    write = data["meta"]["conditions"]["databases-write"]
    assert write[0].startswith("Different operations, not one scale")
    assert any("feature-rows-added 9" in line for line in write)


def test_database_figures_split_threads_and_keep_writes_apart(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path, databases=True)))
    out = tmp_path / "figures"
    with plt.rc_context({"svg.fonttype": "none"}):
        figures.databases(data, out)
        figures.databases_write(data, out)
    reads = (out / "databases.svg").read_text(encoding="utf-8")
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
    ):
        assert text in reads, text
    # No write scenario leaks into the read heatmap.
    assert "Add attribute" not in reads and "Append one building" not in reads
    writes = (out / "databases-write.svg").read_text(encoding="utf-8")
    for text in ("not one scale", "Add attribute", "Append one building", "write-back)", ">error<"):
        assert text in writes, text


def test_the_page_lists_both_database_figures_with_their_conditions(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path, databases=True)))
    data_path = tmp_path / "bench_data.json"
    data_path.write_text(json.dumps(data), encoding="utf-8")
    figures_dir = tmp_path / "figures"
    figures.databases(data, figures_dir)
    figures.databases_write(data, figures_dir)
    page = html.main(data_path, tmp_path / "index.html", figures_dir).read_text(encoding="utf-8")
    section = page.split("<h2>Database write tier</h2>", 1)[1].split("</section>", 1)[0]
    assert "<img" in section and "not one scale" in section
    section = page.split("<h2>Database comparison</h2>", 1)[1].split("</section>", 1)[0]
    assert "bbox-5pct achieved" in section


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
