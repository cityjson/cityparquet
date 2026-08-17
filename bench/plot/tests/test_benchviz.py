"""`benchviz` reads finished benchmark CSVs and writes where it is told.

The CSVs come from `tests/fixtures/benchviz/` — three datasets of a real,
pinned run — not from the live `bench/read_results`, so these tests say the
same thing after the next benchmark run replaces those CSVs with a different
corpus. The methodology documents are the LIVE ones: caveat extraction is
supposed to break when `READ_BENCHMARK.md` grows a twelfth fairness caveat,
because the page quotes them verbatim and must not fall behind.

No test renders the HTML or the figures — that needs matplotlib and tens of
seconds; `just plot-pretty` is the check for those.
"""

import shutil
from pathlib import Path

import pytest

from benchviz import __main__ as cli
from benchviz import prep

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "benchviz"
LIVE_BENCH_DIR = Path(__file__).resolve().parents[2]


def _bench_dir(tmp_path: Path) -> Path:
    """A `bench/` made of the fixture CSVs and the live methodology docs."""
    root = tmp_path / "bench"
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
    for key in ("meta", "datasets", "read", "sizes", "compression", "compression_gaps"):
        assert key in data, f"bench_data.json lacks '{key}'"
    assert [d["id"] for d in data["datasets"]] == ["Zurich", "delft", "Ingolstadt"]
    assert data["read"]
    # The source labels stay repo-qualified (".../bench/read_results"), so the
    # page names its inputs the same way wherever the renderer was invoked from.
    assert data["meta"]["sources"]["read"].endswith("bench/read_results")
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
    `roundtrip_equal=false`. Both are undocumented in `bench/README.md`, and a
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
