"""`benchviz` reads finished benchmark CSVs and writes where it is told.

The CSVs come from `tests/fixtures/benchviz/` — three datasets of a real,
pinned run — not from the live `benchmark/formats/read_results`, so these tests say the
same thing after the next benchmark run replaces those CSVs with a different
corpus. The methodology documents are the LIVE ones: caveat extraction is
supposed to break when `READ_BENCHMARK.md` grows a twelfth fairness caveat,
because the page quotes them verbatim and must not fall behind.

No test renders the HTML or the figures — that needs matplotlib and tens of
seconds; `just plot-pretty` is the check for those.
"""

import re
import shutil
from pathlib import Path

import pytest

from benchviz import __main__ as cli
from benchviz import prep

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "benchviz"
# benchmark/plot/tests/ -> benchmark/. Everything this reads is a sibling: the
# read benchmark's evidence and methodology docs in formats/, the harness crate
# they must agree with in readbench/.
BENCHMARK_ROOT = Path(__file__).resolve().parents[2]
LIVE_BENCH_DIR = BENCHMARK_ROOT / "formats"
READBENCH_SRC = BENCHMARK_ROOT / "readbench" / "src"


def _bench_dir(tmp_path: Path) -> Path:
    """A `benchmark/formats/` made of the fixture CSVs and the live methodology docs.

    Laid out under the real two-component name rather than a flat `bench/`,
    because one assertion below is about the SHAPE of the source label prep
    records — a fixture rooted somewhere that cannot occur in the repository
    would pass that assertion by accident.
    """
    root = tmp_path / "benchmark" / "formats"
    shutil.copytree(FIXTURE, root)
    for name in ("READ_BENCHMARK.md", "README.md"):
        shutil.copy(LIVE_BENCH_DIR / name, root / name)
    return root


def test_prep_builds_the_design_contract_from_result_csvs(tmp_path):
    out = prep.main(
        prep.Inputs(_bench_dir(tmp_path)), out_path=tmp_path / "bench_data.json"
    )

    assert out.exists()
    data = prep.json.loads(out.read_text(encoding="utf-8"))
    for key in (
        "meta",
        "datasets",
        "read",
        "sizes",
        "compression",
        "compression_gaps",
        "ordering",
        "scaling",
    ):
        assert key in data, f"bench_data.json lacks '{key}'"
    assert [d["id"] for d in data["datasets"]] == ["Zurich", "delft", "Ingolstadt"]
    assert data["read"]
    # The source labels stay repo-qualified (".../benchmark/formats/read_results"), so the
    # page names its inputs the same way wherever the renderer was invoked from.
    assert data["meta"]["sources"]["read"].endswith("benchmark/formats/read_results")
    assert data["meta"]["caveats_read"], "no fairness caveats extracted"

    # A format this package has no visual vocabulary for is EXCLUDED AND SAID
    # SO — never quietly averaged in, never quietly dropped. The corpus grows
    # formats (a CityGML-native column arrived with the CityGML reader), so the
    # tally is what keeps the page's own coverage note honest about them.
    for entry in data["meta"]["excluded_formats"]:
        assert entry["format"] not in prep.KNOWN_FORMATS
        assert entry["rows"] > 0
    for record in data["read"] + data["sizes"]:
        assert record["format"] in prep.KNOWN_FORMATS


def test_prep_records_the_compression_gaps_rather_than_dropping_them(tmp_path):
    """The two known-bad compression inputs must survive as stated gaps.

    Railway's CSV is header-only and every Ingolstadt row has
    `roundtrip_equal=false`. Both are undocumented in `benchmark/formats/README.md`, and a
    reader who cannot see them would read the codec panels as citable.
    """
    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))

    gaps = {g["dataset"]: g["issue"] for g in data["compression_gaps"]}
    assert "header-only" in gaps["Railway"]
    assert "roundtrip_equal=false" in gaps["Ingolstadt"]


def test_object_counts_survive_a_run_that_measured_only_hilbert(tmp_path):
    """A CityParquet run under one ordering still yields the CityObject count.

    The count is a property of the dataset, not of the row order, and a run may
    measure `cityparquet-hilbert` alone (the 2026-08-17 corpus run did). Before
    this, prep refused such a run outright: no plain `cityparquet` full-read
    row, no dataset subtitle, no page.
    """
    bench = _bench_dir(tmp_path)
    for csv_path in (bench / "read_results").glob("*.csv"):
        if csv_path.name == "sizes.csv":
            continue
        kept = [
            line
            for line in csv_path.read_text(encoding="utf-8").splitlines()
            if ",cityparquet," not in line
        ]
        csv_path.write_text("\n".join(kept) + "\n", encoding="utf-8")

    data, _ = prep.build(prep.Inputs(bench))

    by_id = {d["id"]: d for d in data["datasets"]}
    assert by_id["Zurich"]["objects"] == 198699
    assert not any(r["format"] == "cityparquet" for r in data["read"])


def test_a_renamed_column_is_still_an_error(tmp_path):
    """Tolerating APPENDED columns must not tolerate a changed contract.

    The coordinator appends columns as the harness grows (`bytes_read` and
    `http_requests` arrived with the HTTP transport), and a reader that dies on
    those is a reader that goes stale after every harness change. A column that
    was *renamed* or *dropped* is the opposite case: the numbers no longer mean
    what this code thinks they mean, so it must refuse.
    """
    csv_path = tmp_path / "renamed.csv"
    header = list(prep.READ_COLUMNS)
    header[5] = "seconds"  # was time_s
    csv_path.write_text(",".join(header) + "\n", encoding="utf-8")

    with pytest.raises(prep.PrepError):
        prep._read_rows(csv_path, prep.READ_COLUMNS)


def test_a_corpus_with_no_compression_run_is_stated_not_crashed(tmp_path):
    """Read-only corpora are normal: compression is a separate, slower run.

    A corpus measured for reads but never for compression must still produce a
    page — with the compression view saying so — and the compression figure must
    be skipped rather than drawn empty or raised as a contract error.
    """
    from benchviz import figures

    bench = _bench_dir(tmp_path)
    shutil.rmtree(bench / "compression_results")

    data, _ = prep.build(prep.Inputs(bench))
    assert data["compression"] == []
    assert data["compression_gaps"] == []

    data_path = tmp_path / "no_compression.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")
    written = sorted(
        p.name for p in figures.main(data_path=data_path, out_dir=tmp_path / "f").glob("*")
    )
    assert "compression.svg" not in written
    assert "sizes.svg" in written


def test_figures_refuse_a_corpus_larger_than_their_panel_grid(tmp_path):
    """More datasets than panels must be a stated refusal, not a crash.

    The static figures are print artefacts: small-multiple grids that grow to a
    5x5 sheet and stop there. Beyond that the panels are too small to carry even
    a pattern, and the honest move is to say so — a corpus that big needs a
    different kind of figure, not a finer grid. It used to run off the end of the
    axes array and die with an IndexError deep inside the Pareto builder, which
    reads as a bug in the plotting code. The HTML page has no such limit.
    """
    from benchviz import figures

    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    template = data["datasets"][0]
    data["datasets"] = [dict(template, id=f"d{i}") for i in range(figures.MAX_PANELS + 1)]
    data_path = tmp_path / "too_many.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")

    with pytest.raises(SystemExit, match="panel grid"):
        figures.main(data_path=data_path, out_dir=tmp_path / "figures")


def _rust_format_set(name: str) -> list[str]:
    """A `Format::*_SET` const, read out of the harness's own source.

    `format.rs` is the authority on what a benchmark run measures and why, and
    it states the reasoning in full: one tag per format family on the format
    axis, with `cityjsonseq-gz` (a compression variant of a format already in
    the set) and `duckdb-parquet` (an SQL-engine baseline) opt-in because
    neither is a format. Restating that list here is how the views would come to
    plot a different comparison than the CSVs answer.
    """
    src = (READBENCH_SRC / "format.rs").read_text()
    body = re.search(rf"pub const {name}: \[Format; \d+\] = \[(.*?)\];", src, re.S)
    assert body, f"could not find {name} in format.rs"
    tags = re.findall(r"Format::(\w+)", body.group(1))
    # `Format::as_str` is the authority on the CSV/CLI spelling of each variant
    # (CityJsonSeq is "cityjsonseq", not "cityjson-seq"), so read it rather than
    # deriving one from the variant name.
    spelling = dict(re.findall(r'Format::(\w+) => "([a-z0-9-]+)"', src))
    return [spelling[t] for t in tags]


def test_the_views_plot_the_format_axis_the_harness_measures():
    from benchviz import figures

    axis = _rust_format_set("DEFAULT_SET")
    assert axis, "no DEFAULT_SET parsed"
    for fmt in axis:
        assert fmt in prep.KNOWN_FORMATS, f"{fmt} would be excluded by prep"
        assert fmt in figures.FORMAT_STYLE, f"{fmt} has no marker/colour"
        assert fmt in figures.FORMAT_ORDER, f"{fmt} is missing from the Pareto panels"
        assert fmt in figures.HEATMAP_FORMATS, f"{fmt} is missing from the heatmap columns"

    # The opt-in tags are not formats, so they must not sit on a format axis —
    # plotting gzipped CityJSONSeq beside CityJSONSeq compares a codec, not a
    # format, and DuckDB compares an engine.
    for fmt in ("cityjsonseq-gz", "duckdb-parquet"):
        assert fmt not in figures.FORMAT_ORDER
        assert fmt not in figures.HEATMAP_FORMATS
        assert fmt not in figures.SIZE_FORMATS


def test_cityparquet_is_represented_by_the_configuration_the_axis_names():
    """On the format axis CityParquet is the Hilbert-ordered package.

    `DEFAULT_SET` says so — the format comparison must not be handicapped by an
    ordering choice no other format faces, and ordering is asked separately by
    `ORDERING_SET`. So where a run carries both packages, the sentences are
    about the Hilbert one.
    """
    from benchviz import figures

    assert "cityparquet-hilbert" in _rust_format_set("DEFAULT_SET")
    assert _rust_format_set("ORDERING_SET") == ["cityparquet", "cityparquet-hilbert"]
    data = {
        "read": [
            {"format": "cityparquet", "time_ratio": 1.0},
            {"format": "cityparquet-hilbert", "time_ratio": 1.0},
        ]
    }
    assert figures._primary_cityparquet(data) == "cityparquet-hilbert"


@pytest.mark.parametrize(
    ("panels", "expected"),
    [(3, (1, 4)), (11, (3, 4)), (12, (3, 4)), (13, (3, 5)), (22, (5, 5)), (25, (5, 5))],
)
def test_grid_shape(panels, expected):
    """Four columns up to a dozen panels, five beyond, never past 5x5.

    Four is what the figures were drawn at, so a corpus of the size they were
    designed for keeps its exact layout; a bigger one densifies instead of
    running off the sheet.
    """
    from benchviz import figures

    assert figures._grid(panels) == expected


def test_cli_prep_writes_to_the_requested_data_path(tmp_path):
    """The path flags are why this package can live inside the submodule.

    It used to resolve its inputs and outputs by counting parent directories up
    into the paper repository, which only worked from there.
    """
    target = tmp_path / "nested" / "data.json"

    assert (
        cli.main(
            ["prep", "--bench-dir", str(_bench_dir(tmp_path)), "--data", str(target)]
        )
        == 0
    )
    assert target.exists()


def test_ordering_records_carry_their_own_dataset_shape(tmp_path):
    """The ordering corpus is not a subset of the read corpus.

    `just ordering-bench` runs over whatever inputs it is pointed at, routinely
    including datasets the read benchmark never measured, so an ordering record
    has to carry the object count and the baseline seconds a view needs rather
    than expecting to find a `datasets` entry to look them up in.
    """
    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))

    assert data["ordering"], "the fixture has an ordering run"
    read_ids = {d["id"] for d in data["datasets"]}
    ordering_ids = {r["dataset"] for r in data["ordering"]}
    assert not (ordering_ids & read_ids), (
        "the fixture is built so the two corpora are disjoint -- a view that "
        "joins ordering rows to `datasets` would silently draw nothing"
    )
    for record in data["ordering"]:
        assert record["objects"], "no CityObject count to title a panel with"
        assert record["base_time_s"] > 0
        assert record["below_floor"] in (True, False)
    # The baseline is the source-order package, NOT the read benchmark's
    # CityJSONSeq: an ordering run has no CityJSONSeq row to divide by.
    assert data["meta"]["ordering_baseline"] == "cityparquet"


def test_a_corpus_with_no_ordering_run_is_stated_not_crashed(tmp_path):
    """Ordering is a separate pass, so its absence is normal.

    Same contract as the compression run: the configuration figure is skipped
    rather than drawn empty, and every other figure is still written.
    """
    from benchviz import figures

    bench = _bench_dir(tmp_path)
    shutil.rmtree(bench / "ordering_results")

    data, _ = prep.build(prep.Inputs(bench))
    assert data["ordering"] == []

    data_path = tmp_path / "no_ordering.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")
    written = sorted(
        p.name for p in figures.main(data_path=data_path, out_dir=tmp_path / "f").glob("*")
    )
    assert "configuration.svg" not in written
    assert "formats.svg" in written


def test_both_panel_picks_refuse_a_degenerate_dataset(tmp_path):
    """A dataset too small to filter cannot carry a panel.

    The corpus deliberately holds a one-object tile (READ_BENCHMARK.md says so
    itself). Every selective scenario on it matches all of it or none of it, so
    a panel drawn from it shows fixed open cost, not a format or a
    configuration. Both picks must reach past it while it stays in the data.
    """
    from benchviz import figures

    tiny = {"id": "tiny", "objects": 1, "raw_mb": 0.1, "subtitle": ""}
    big = [
        {"id": f"d{i}", "objects": 10_000 - i, "raw_mb": 10.0, "subtitle": ""}
        for i in range(figures.FORMAT_PANELS)
    ]
    picked = figures._panel_pick([*big, tiny], figures.FORMAT_PANELS)
    assert tiny not in picked

    ordering = {
        "tiny": [{"objects": 1, "below_floor": True, "delta_s": 0.0}],
        "real": [{"objects": 5_000, "below_floor": True, "delta_s": 0.001}],
    }
    assert [ds for ds, _ in figures._ordering_pick(ordering)] == ["real"]


def test_scaling_slices_are_measured_not_named(tmp_path):
    """The scaling corpus is one model at several cardinalities.

    Its slices are NAMED for the size they were asked for and HOLD whatever a
    strict prefix of the source actually contains, so the object count has to
    come from the run. It also stays out of `datasets`: a slice is not a peer
    of Vienna, and letting one in would grow a synthetic panel onto every
    per-dataset grid on the page.
    """
    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    scaling = data["scaling"]

    assert scaling["read"], "the fixture has a scaling run"
    slice_ids = {r["dataset"] for r in scaling["read"]}
    assert not (slice_ids & {d["id"] for d in data["datasets"]})
    for record in scaling["read"]:
        assert record["objects"], "no CityObject count to place the slice on the x axis"
        # Absolutes, not only ratios: a trend view reads the SLOPE of time
        # against cardinality, which a ratio to a growing baseline destroys.
        assert record["time_s"] is not None
        assert record["rss_b"] is not None

    # sizes.csv sweeps the shared prepared-artefact directory, so it carries
    # rows for the catalogue corpus too. Only the measured slices survive.
    assert {r["dataset"] for r in scaling["sizes"]} <= slice_ids


def test_a_corpus_with_no_scaling_run_is_stated_not_crashed(tmp_path):
    """Same contract as compression and ordering: absence is normal."""
    bench = _bench_dir(tmp_path)
    shutil.rmtree(bench / "scaling_read_results")

    data, _ = prep.build(prep.Inputs(bench))
    assert data["scaling"]["read"] == []
    assert data["scaling"]["sizes"] == []
