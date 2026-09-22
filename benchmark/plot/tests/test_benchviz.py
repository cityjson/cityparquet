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
        for name in (
            "sizes",
            "heatmap",
            "codec",
            "codec-scaling",
            "rowgroup",
            "rowgroup-scaling",
            "bloom",
            "bloom-scaling",
        )
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


def test_bloom_axis_keys_the_lookup_probes_and_carries_the_counters(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    axis = data["scaling"]["bloom"]
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
    assert record("cityparquet", "write")["row_groups_total"] is None


def _mixed_bloom_fixture(bench: Path) -> None:
    """Corpus and 3DBAG slices in one bloom directory, with colliding counts.

    `3dbag_n1000`, `3dbag_n5000` and the corpus `rotterdam_delfshaven` all
    report 2,231 objects; `3dbag_n10000` reports 10,004.
    """
    directory = bench / "scaling_bloom_results"
    template = (directory / "delft.csv").read_text().splitlines()
    (directory / "delft.csv").unlink()
    counts = {
        "3dbag_n1000": 2231,
        "3dbag_n5000": 2231,
        "3dbag_n10000": 10004,
        "rotterdam_delfshaven": 2231,
    }
    sizes = ["dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline"]
    for i, (name, count) in enumerate(counts.items()):
        rows = [template[0]]
        for line in template[1:]:
            cells = line.split(",")
            cells[0] = f"{name}.city.jsonl"
            if cells[2] == "write":
                cells[4] = str(count)
            cells[5] = f"{float(cells[5]) * (i + 1):.6f}"
            rows.append(",".join(cells))
        (directory / f"{name}.csv").write_text("\n".join(rows) + "\n")
        sizes.append(f"{name},cityparquet,{1000 * (i + 1)},0.1,1.0,cityparquet,1.0")
        sizes.append(f"{name},cityparquet+nobloom,{900 * (i + 1)},0.1,1.0,cityparquet,0.9")
    (directory / "sizes.csv").write_text("\n".join(sizes) + "\n")


def test_bloom_scaling_curve_holds_only_the_slices_and_the_corpus_stands_apart(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    _mixed_bloom_fixture(bench)
    data, _ = prep.build(prep.Inputs(bench))
    axis = data["scaling"]["bloom"]
    series = {r["dataset"]: r["series"] for r in axis["records"] + axis["sizes"]}
    assert series == {
        "3dbag_n1000": "scaling",
        "3dbag_n5000": "scaling",
        "3dbag_n10000": "scaling",
        "rotterdam_delfshaven": "corpus",
    }

    # Every slice keeps its own point, the two equal counts included, and the
    # corpus dataset of the same count joins neither the curve nor overwrites it.
    for source, measure in (
        (axis["records"], "write"),
        (axis["records"], "id-miss"),
        (axis["sizes"], None),
    ):
        for variant in axis["variants"]:
            points = figures._scaling_points(source, variant, measure)
            assert [r["dataset"] for r in points] == ["3dbag_n1000", "3dbag_n5000", "3dbag_n10000"]
    write = figures._scaling_points(axis["records"], "cityparquet", "write")
    assert [r["time_s"] for r in write] == [1.17, 2.34, 3.51]
    assert figures._corpus_datasets(axis["records"]) == ["rotterdam_delfshaven"]

    output = figures.main(_dump(data, tmp_path), tmp_path / "figures")
    names = {p.name for p in output.glob("*")}
    assert {"bloom-scaling.svg", "bloom-corpus.svg", "bloom.svg"} <= names


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
    # slices only: the earlier corpus figure must not survive into the page.
    for key in ("records", "sizes"):
        data["scaling"]["bloom"][key] = [
            r for r in data["scaling"]["bloom"][key] if r["series"] == "scaling"
        ]
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
    the bloom, bloom-scaling and bloom-corpus figures on the page with no
    warning beside them at all. Phrases, not a count: the list is allowed to
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
    page = html.main(
        data_path=data_path, out_path=tmp_path / "index.html", figures_dir=figures_dir
    )
    text = page.read_text(encoding="utf-8")

    # The premise: the page really is showing bloom figures.
    for title in (
        "Bloom-filter configuration",
        "Bloom-filter scaling",
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
        # 30-31: what predates default-on filters and must be re-run.
        "The committed codec and row-group CSVs predate bloom filters.",
        "Every other committed CSV predates bloom filters too.",
        "refuses to",
    ):
        assert phrase in caveats, phrase
