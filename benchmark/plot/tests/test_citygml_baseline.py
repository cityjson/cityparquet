"""The format comparison against its CityGML baseline: factors, tables, layout.

Every relative value answers "how many times faster, or smaller, than
CityGML": a factor is CityGML's value divided by the format's, so a larger
factor is better. A CityGML cell that is missing, skipped or failed leaves the
factor unavailable — never zero, and never a ratio against another format.
"""

import csv
import json
import shutil
from pathlib import Path

from benchviz import figures, html, prep, tables

LIVE = Path(__file__).parents[2] / "formats"
HEADER = (
    "dataset,format,scenario,selectivity,result_count,time_mean_s,time_std_s,time_median_s,"
    "time_min_s,time_max_s,time_q1_s,time_q3_s,peak_heap_bytes,"
    "peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,"
    "filter_bytes"
)
FORMATS = ("citygml", "cityjson", "cityjsonseq", "flatcitybuf", "cityparquet")


def _row(dataset, fmt, scenario, count, time, rss, notes=""):
    return f"{dataset},{fmt},{scenario},,{count},{time},0.001,{time},{time},{time},{time},{time},,{rss},7,{notes},,,,,"


def _bench(tmp_path: Path) -> Path:
    """Three datasets in the coordinator's shape.

    `alpha` is complete. `beta`'s CityGML `full-read` failed, its CityGML
    `bbox-1pct` row is absent, its FlatCityBuf `bbox-1pct` was skipped, and
    its CityGML size is missing. `gamma` is complete and smaller.
    """
    bench = tmp_path / "benchmark" / "formats"
    results = bench / "results"
    results.mkdir(parents=True)
    shutil.copy(LIVE / "READ_BENCHMARK.md", bench / "READ_BENCHMARK.md")
    times = {fmt: 0.5 * (len(FORMATS) - i) for i, fmt in enumerate(FORMATS)}
    rss = {fmt: 1_000_000 * (len(FORMATS) - i) for i, fmt in enumerate(FORMATS)}
    for name, count in (("alpha", 2000), ("beta", 1000), ("gamma", 500)):
        lines = [HEADER]
        for fmt in FORMATS:
            for scenario, notes in (("full-read", ""), ("bbox-query", "bbox-1pct")):
                if name == "beta" and fmt == "citygml" and notes == "bbox-1pct":
                    continue
                if name == "beta" and fmt == "citygml" and scenario == "full-read":
                    lines.append(_row(name, fmt, scenario, 0, 0.0, 0, "error: out of memory"))
                    continue
                if name == "beta" and fmt == "flatcitybuf" and notes == "bbox-1pct":
                    lines.append(
                        _row(name, fmt, scenario, 0, 0.0, 0, "bbox-1pct;skipped: 2D index")
                    )
                    continue
                lines.append(_row(name, fmt, scenario, count, times[fmt], rss[fmt], notes))
        (results / f"{name}.csv").write_text("\n".join(lines) + "\n")
    sizes = ["dataset,format,bytes,mb_decimal"]
    for name, scale in (("alpha", 10), ("beta", 5), ("gamma", 1)):
        for i, fmt in enumerate(FORMATS):
            if name == "beta" and fmt == "citygml":
                continue
            factor = 8 if name == "gamma" else 4
            size = scale * 1000 * (factor if fmt == "citygml" else len(FORMATS) - i)
            sizes.append(f"{name},{fmt},{size},0.000001")
    (results / "sizes.csv").write_text("\n".join(sizes) + "\n")
    return bench


def _record(data, dataset, fmt, key):
    return next(
        r
        for r in data["read"]
        if r["dataset"] == dataset and r["format"] == fmt and r["scenario_key"] == key
    )


def test_read_factors_are_citygml_over_the_format(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    assert data["meta"]["baseline"] == "citygml"
    record = _record(data, "alpha", "cityparquet", "full-read")
    assert record["time_factor"] == 2.5 / 0.5
    assert record["rss_factor"] == 5_000_000 / 1_000_000
    assert _record(data, "alpha", "citygml", "full-read")["time_factor"] == 1.0


def test_a_failed_or_missing_citygml_cell_leaves_the_factor_unavailable(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    for key in ("full-read", "bbox-1pct"):
        record = _record(data, "beta", "cityparquet", key)
        assert record["time_s"] == 0.5, "the absolute value is kept"
        assert record["time_factor"] is None and record["rss_factor"] is None
    # Never a fallback to another format as the baseline.
    assert _record(data, "beta", "cityjson", "full-read")["time_factor"] is None


def test_a_failed_format_cell_has_no_value_and_no_factor(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    skipped = _record(data, "beta", "flatcitybuf", "bbox-1pct")
    assert skipped["time_s"] is None and skipped["time_factor"] is None
    failed = _record(data, "beta", "citygml", "full-read")
    assert failed["time_s"] is None and failed["rss_b"] is None


def test_the_size_table_has_one_row_per_dataset_and_citygml_factors(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    rows = tables.size_table(data)
    assert [r["dataset"] for r in rows] == ["alpha", "beta", "gamma"]
    assert list(rows[0]) == [
        "dataset",
        "title",
        "city_objects",
        "citygml_bytes",
        "cityjson_bytes",
        "cityjson_factor",
        "cityjsonseq_bytes",
        "cityjsonseq_factor",
        "flatcitybuf_bytes",
        "flatcitybuf_factor",
        "cityparquet_bytes",
        "cityparquet_factor",
    ]
    alpha = rows[0]
    assert alpha["city_objects"] == 2000
    assert alpha["citygml_bytes"] == 40000 and alpha["cityparquet_bytes"] == 10000
    assert alpha["cityparquet_factor"] == 4.0
    beta = rows[1]
    assert beta["citygml_bytes"] is None
    assert beta["cityparquet_bytes"] == 5000
    assert all(beta[f"{f}_factor"] is None for f in tables.SIZE_FORMATS)


def test_the_size_extremes_name_the_best_and_worst_cityparquet_factor(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    extremes = tables.size_extremes(tables.size_table(data))
    assert [(r["rank"], r["dataset"], r["cityparquet_factor"]) for r in extremes] == [
        ("best", "gamma", 8.0),
        ("worst", "alpha", 4.0),
    ]


def test_the_query_table_orients_every_factor_against_citygml(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    rows = tables.query_table(data)
    assert list(rows[0]) == [
        "dataset",
        "query",
        "format",
        "format_id",
        "statistic",
        "time_value_s",
        "time_lo_s",
        "time_hi_s",
        "time_min_s",
        "time_max_s",
        "peak_rss_bytes",
        "citygml_time_s",
        "citygml_peak_rss_bytes",
        "time_factor_vs_citygml",
        "rss_factor_vs_citygml",
        "note",
    ]
    by = {(r["dataset"], r["query"], r["format_id"]): r for r in rows}
    parquet = by[("alpha", "full-read", "cityparquet")]
    assert parquet["format"] == "CityParquet"
    assert parquet["time_factor_vs_citygml"] == 5.0
    assert parquet["note"] == ""
    no_base = by[("beta", "full-read", "cityparquet")]
    assert no_base["time_value_s"] == 0.5 and no_base["time_factor_vs_citygml"] is None
    assert no_base["note"] == "CityGML unavailable: error: out of memory"
    assert by[("beta", "bbox-1pct", "cityjson")]["note"] == "CityGML unavailable: not measured"
    skipped = by[("beta", "bbox-1pct", "flatcitybuf")]
    assert skipped["time_value_s"] is None and skipped["note"] == "skipped: 2D index"


def test_the_csv_files_print_unavailable_as_an_empty_cell_never_zero(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    out = tmp_path / "figures"
    tables.write_tables(data, out)
    with (out / "formats" / "size_factors.csv").open(newline="") as handle:
        beta = next(r for r in csv.DictReader(handle) if r["dataset"] == "beta")
    assert beta["citygml_bytes"] == "" and beta["cityparquet_factor"] == ""
    assert beta["cityparquet_bytes"] == "5000"
    with (out / "formats" / "query_factors.csv").open(newline="") as handle:
        rows = list(csv.DictReader(handle))
    row = next(
        r
        for r in rows
        if (r["dataset"], r["query"], r["format_id"]) == ("beta", "full-read", "cityjson")
    )
    assert row["time_factor_vs_citygml"] == "" and row["time_value_s"] == "2.000000"
    assert (out / "formats" / "size_extremes.csv").is_file()


def test_a_cell_without_a_citygml_value_says_so(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    dataset = next(d for d in data["datasets"] if d["id"] == "beta")
    queries, cells = figures.format_cells(data, dataset, "time_s")
    row = cells[queries.index("full-read")]
    citygml, parquet = row[0], row[-1]
    assert citygml == (None, "failed")
    assert parquet[0] is None and parquet[1] == "0.5\n\u00d7 n/a"


def test_one_folder_per_dataset_with_a_time_and_a_memory_figure(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    data_path = tmp_path / "bench_data.json"
    data_path.write_text(json.dumps(data), encoding="utf-8")
    out = figures.main(data_path, tmp_path / "figures")
    for name in ("alpha", "beta", "gamma"):
        for metric in ("time", "rss"):
            for kind in ("svg", "png"):
                assert (out / "formats" / name / f"{metric}.{kind}").is_file()
    for path in ("sizes.svg", "sizes.png", "size_factors.csv", "query_factors.csv"):
        assert (out / "formats" / path).is_file()
    assert not list(out.glob("heatmap*")) and not (out / "sizes.svg").exists()

    page = html.main(data_path=data_path, out_path=tmp_path / "index.html", figures_dir=out)
    text = page.read_text(encoding="utf-8")
    for name in ("alpha", "beta", "gamma"):
        assert f"<h2>{name} — read time</h2>" in text
        assert f"<h2>{name} — read peak memory</h2>" in text
    assert "<h2>File size on disk</h2>" in text
    assert "Size factors against CityGML" in text and "<td>gamma</td>" in text


def test_a_slice_title_names_its_nominal_size_not_its_exact_count():
    slice_ = {"id": "3dbag_n1000000", "title": "3DBAG", "objects": 1_000_001}
    slice_["nominal_objects"] = 1_000_000
    assert figures._title(slice_) == "3DBAG — 1,000,000-object slice"
    assert "1,000,001" in figures.slice_note(slice_)
    corpus = {"id": "rotterdam", "title": "Rotterdam", "objects": 853}
    assert figures._title(corpus) == "Rotterdam — 853 CityObjects"
    assert figures.slice_note(corpus) == ""


def test_figures_dir_redirects_the_whole_formats_tree(tmp_path: Path):
    from benchviz import __main__ as cli

    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    data_path = tmp_path / "out" / "bench_data.json"
    data_path.parent.mkdir()
    data_path.write_text(json.dumps(data), encoding="utf-8")
    exported = tmp_path / "paper" / "assets" / "bench"
    assert cli.main(["figures", "--data", str(data_path), "--figures", str(exported)]) == 0
    assert (exported / "formats" / "alpha" / "time.svg").is_file()
    assert (exported / "formats" / "query_factors.csv").is_file()
    assert not (tmp_path / "out" / "figures").exists()


def test_a_manifest_dataset_without_results_stays_explicitly_missing(tmp_path: Path):
    data, _ = prep.build(prep.Inputs(_bench(tmp_path)))
    data["meta"]["dataset_labels"]["kyoto"] = {"role": "corpus", "title": "Kyoto"}
    data["meta"]["dataset_labels"]["3dbag_n1000000"] = {
        "role": "slice",
        "title": "3DBAG",
        "nominal_objects": 1_000_000,
    }
    data_path = tmp_path / "bench_data.json"
    data_path.write_text(json.dumps(data), encoding="utf-8")
    out = figures.main(data_path, tmp_path / "figures")
    page = html.main(data_path=data_path, out_path=tmp_path / "index.html", figures_dir=out)
    text = page.read_text(encoding="utf-8")
    section = text.split("<h2>Kyoto — read time</h2>", 1)[1].split("</section>", 1)[0]
    assert "Not rendered" in section and "<img" not in section
    # The slice is an expected dataset too: unmeasured, it is reported missing.
    section = text.split("<h2>3DBAG — read time</h2>", 1)[1].split("</section>", 1)[0]
    assert "Not rendered" in section and "<img" not in section
    assert not any(r["dataset"] == "kyoto" for r in tables.size_table(data))
