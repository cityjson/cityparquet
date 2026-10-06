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
    expected = {f"{name}.{kind}" for name in ("bloom", "bloom-corpus") for kind in ("svg", "png")}
    assert expected <= names
    assert {"sizes.svg", "sizes.png"} <= {p.name for p in (output / "formats").glob("*")}
    assert not any("pareto" in name or "heatmap" in name for name in names)


def test_every_dataset_gets_its_own_formats_folder(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    path = tmp_path / "bench_data.json"
    path.write_text(prep.json.dumps(data), encoding="utf-8")
    output = figures.main(path, tmp_path / "figures")
    ids = [dataset["id"] for dataset in data["datasets"]]
    assert ids
    for i in ids:
        names = {p.name for p in (output / "formats" / i).glob("*")}
        assert {f"{m}.{k}" for m in ("time", "rss") for k in ("svg", "png")} <= names


def test_cityparquet_label_and_missing_database_are_honest(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    assert figures._label("cityparquet") == "CityParquet"
    assert data["databases"] == {"baseline": "3dcitydb", "records": [], "sizes": []}


DB_COLUMNS = [
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    "time_s",
    "time_std_s",
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


def test_bloom_axis_keys_the_lookup_probes_and_carries_the_counters(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    axis = data["bloom"]
    assert axis["variants"] == ["cityparquet", "cityparquet+nobloom"]
    assert {r["measure"] for r in axis["records"]} == set(prep.BLOOM_MEASURES)

    def record(variant: str, measure: str) -> dict:
        return next(
            r for r in axis["records"] if r["variant"] == variant and r["measure"] == measure
        )

    on = record("cityparquet", "id-miss")
    assert (on["row_groups_total"], on["bloom_pruned"]) == (1, 1)
    assert on["filter_bytes"] == 8192
    off = record("cityparquet+nobloom", "id-miss")
    assert (off["bloom_pruned"], off["filter_bytes"]) == (0, 0)
    assert off["time_ratio"] == 0.0049 / 0.0021


def test_bloom_objects_come_from_the_parameter_sidecar_not_a_lookup_count(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    axis = data["bloom"]
    assert {r["objects"] for r in axis["records"] + axis["sizes"]} == {2231}
    # Without the sidecar the lookups' result counts (0, 1, 2) say nothing
    # about the dataset's size, so the count is unknown rather than wrong.
    (bench / "bloom_results" / "delft.csv.params.json").unlink()
    data, _ = prep.build(prep.Inputs(bench))
    axis = data["bloom"]
    assert {r["objects"] for r in axis["records"]} == {None}


SLICE = "3dbag_n1000000"


def _mixed_bloom_fixture(bench: Path) -> None:
    """The slice and two corpus datasets in one bloom directory.

    The slice reports 1,000,004 objects; the corpus `rotterdam_delfshaven`
    and `ingolstadt` report 2,231 and 379.
    """
    directory = bench / "bloom_results"
    template = (directory / "delft.csv").read_text().splitlines()
    (directory / "delft.csv").unlink()
    (directory / "delft.csv.params.json").unlink()
    counts = {SLICE: 1_000_004, "rotterdam_delfshaven": 2231, "ingolstadt": 379}
    sizes = ["dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline"]
    for i, (name, count) in enumerate(counts.items()):
        rows = [template[0]]
        for line in template[1:]:
            cells = line.split(",")
            cells[0] = f"{name}.city.jsonl"
            cells[5] = f"{float(cells[5]) * (i + 1):.6f}"
            rows.append(",".join(cells))
        (directory / f"{name}.csv").write_text("\n".join(rows) + "\n")
        (directory / f"{name}.csv.params.json").write_text(f'{{"cp_object_total": {count}}}')
        sizes.append(f"{name},cityparquet,{1000 * (i + 1)},0.1,1.0,cityparquet,1.0")
        sizes.append(f"{name},cityparquet+nobloom,{900 * (i + 1)},0.1,1.0,cityparquet,0.9")
    (directory / "sizes.csv").write_text("\n".join(sizes) + "\n")


def test_the_bloom_headline_is_the_manifest_slice_and_the_corpus_stands_apart(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    _mixed_bloom_fixture(bench)
    data, _ = prep.build(prep.Inputs(bench))
    assert data["meta"]["slice_dataset"] == SLICE
    axis = data["bloom"]
    assert {r["dataset"] for r in axis["records"]} == {SLICE, "rotterdam_delfshaven", "ingolstadt"}
    # The corpus is every measured dataset but the slice, smallest first.
    assert figures._corpus_datasets(axis["records"], SLICE) == [
        "ingolstadt",
        "rotterdam_delfshaven",
    ]

    output = figures.main(_dump(data, tmp_path), tmp_path / "figures")
    names = {p.name for p in output.glob("*")}
    assert {"bloom.svg", "bloom-corpus.svg"} <= names


def test_an_unmeasured_slice_leaves_the_bloom_headline_a_placeholder(tmp_path: Path):
    """The fixture's one bloom dataset is a corpus model, not the slice: the
    headline says so instead of promoting a corpus city to it."""
    import matplotlib.pyplot as plt

    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    data["meta"]["slice_dataset"] = SLICE
    with plt.rc_context({"svg.fonttype": "none"}):
        figures._axis_main(data, "bloom", tmp_path / "figures")
    text = (tmp_path / "figures" / "bloom.svg").read_text(encoding="utf-8")
    assert f"the slice dataset ({SLICE}) was not measured" in text
    assert figures._corpus_datasets(data["bloom"]["records"], SLICE) == ["delft"]


def _dump(data: dict, tmp_path: Path) -> Path:
    path = tmp_path / "bench_data.json"
    path.write_text(prep.json.dumps(data), encoding="utf-8")
    return path


def test_a_rerun_without_corpus_data_leaves_no_stale_corpus_figure(tmp_path: Path):
    from benchviz import html

    bench = fixture_bench(tmp_path)
    _mixed_bloom_fixture(bench)
    figures_dir = tmp_path / "figures"
    data, _ = prep.build(prep.Inputs(bench))
    figures.main(_dump(data, tmp_path), figures_dir)
    assert (figures_dir / "bloom-corpus.svg").exists()

    # The same output directory, re-used by a run whose bloom axis measured
    # the slice only: the earlier corpus figure must not survive into the page.
    for key in ("records", "sizes"):
        data["bloom"][key] = [r for r in data["bloom"][key] if r["dataset"] == SLICE]
    data_path = _dump(data, tmp_path)
    figures.main(data_path, figures_dir)
    assert not (figures_dir / "bloom-corpus.svg").exists()
    assert not (figures_dir / "bloom-corpus.png").exists()
    page = html.main(data_path=data_path, out_path=tmp_path / "index.html", figures_dir=figures_dir)
    text = page.read_text(encoding="utf-8")
    corpus_section = text.split("<h2>Bloom filters on the corpus</h2>", 1)[1].split(
        "</section>", 1
    )[0]
    assert "<img" not in corpus_section
    assert "Not rendered" in corpus_section


def test_the_rendered_page_carries_the_bloom_and_predate_caveats(tmp_path: Path):
    """Every caveat the bloom figures need reaches the page, because it is a
    numbered fairness caveat in the LIVE `READ_BENCHMARK.md`.

    `prep.read_caveats` extracts that one list and `html.main` is the only
    thing that prints caveats — it reads `meta.caveats_read` and nothing else.
    So a bloom caveat kept in `benchmark/formats/README.md` instead would leave
    the bloom and bloom-corpus figures on the page with no warning beside them
    at all. Phrases, not a count: the list is allowed to
    grow (see `prep.read_caveats`), and each phrase is one source line with no
    character `html.escape` rewrites.
    """
    from benchviz import html

    bench = fixture_bench(tmp_path)
    _mixed_bloom_fixture(bench)
    data, _ = prep.build(prep.Inputs(bench))
    data_path = _dump(data, tmp_path)
    figures_dir = tmp_path / "figures"
    figures.main(data_path, figures_dir)
    page = html.main(data_path=data_path, out_path=tmp_path / "index.html", figures_dir=figures_dir)
    text = page.read_text(encoding="utf-8")

    # The premise: the page really is showing bloom figures.
    for title in (
        "Bloom-filter configuration",
        "Bloom filters on the corpus",
    ):
        section = text.split(f"<h2>{title}</h2>", 1)[1].split("</section>", 1)[0]
        assert "<img" in section, title

    caveats = text.split("<h2>Measurement caveats</h2>", 1)[1]
    for phrase in (
        # 24-29: the bloom family's own.
        "about how pruning scales",
        "a miss can still keep a row group",
        "reads the footer twice per lookup",
        "measures single-table packages only",
        "the same verified-absent string as `id-miss`",
        "counts the requests the reader made after",
        # 30-31: the evidence carries default-on filters and one timing statistic.
        "The current evidence was measured on bloom-enabled packages.",
        "One generation of results, one statistic, one header.",
        "refuses to",
    ):
        assert phrase in caveats, phrase


def test_a_single_statistic_csv_is_refused_loudly_in_both_loaders(tmp_path: Path):
    """A CSV with one `time_s` column instead of the seven-column timing
    block cannot say which statistic it holds: both the format and the bloom
    loaders refuse it rather than plotting it or skipping it quietly."""
    import pytest

    legacy = (
        "dataset,format,scenario,selectivity,result_count,time_s,time_std_s,"
        "peak_heap_bytes,peak_rss_bytes,repeat,notes\n"
        "delft.city.jsonl,cityparquet,full-read,,2231,0.1,0.01,1,2,7,\n"
    )
    for directory in ("results", "bloom_results"):
        bench = fixture_bench(tmp_path / directory)
        (bench / directory / "delft.csv").write_text(legacy, encoding="utf-8")
        with pytest.raises(prep.PrepError, match="unexpected columns"):
            prep.build(prep.Inputs(bench))
