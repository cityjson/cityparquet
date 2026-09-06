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
        "ordering",
        "scaling",
    ):
        assert key in data, f"bench_data.json lacks '{key}'"
    assert set(data["scaling"]) == {"read", "sizes", "ordering", "codec", "rowgroup"}
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


def test_axis_records_are_baselined_against_the_default_variant(tmp_path):
    """Both configuration axes load through one loader, against `cityparquet`.

    Ratios are baseline over variant (above 1x is faster, leaner, smaller), the
    variant order is the CSV's own, and the write row is a measure like any
    other. The fixture is a measured delft run, so every number here is real.
    """
    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    for key, expected_variants in (
        ("codec", ["cityparquet", "cityparquet+zstd1", "cityparquet+lz4"]),
        ("rowgroup", ["cityparquet", "cityparquet+rg512", "cityparquet+rg2048"]),
    ):
        axis = data["scaling"][key]
        assert axis["variants"] == expected_variants
        assert axis["gaps"] == []
        by = {(r["variant"], r["measure"]): r for r in axis["records"]}
        assert set(m for _, m in by) == set(prep.AXIS_MEASURES)
        base_write = by[("cityparquet", "write")]
        assert base_write["kind"] == "default"
        assert base_write["objects"] == 2231
        assert base_write["time_ratio"] == 1.0
        assert base_write["rss_ratio"] == 1.0
        variant_write = by[(expected_variants[1], "write")]
        assert variant_write["kind"] == "variant"
        assert variant_write["base_time_s"] == base_write["time_s"]
        assert variant_write["time_ratio"] == pytest.approx(
            base_write["time_s"] / variant_write["time_s"]
        )
        assert isinstance(variant_write["below_floor"], bool)
        assert by[(expected_variants[1], "bbox-5pct")]["dataset"] == "delft"

        sizes = {s["variant"]: s for s in axis["sizes"]}
        assert set(sizes) == set(expected_variants)
        assert sizes["cityparquet"]["size_ratio"] == 1.0
        assert sizes["cityparquet"]["objects"] == 2231
        assert sizes[expected_variants[1]]["size_ratio"] == pytest.approx(
            sizes["cityparquet"]["bytes"] / sizes[expected_variants[1]]["bytes"]
        )
    assert data["meta"]["axis_baseline"] == "cityparquet"
    assert data["meta"]["sources"]["codec"].endswith("scaling_codec_results")
    assert "zstd" in data["meta"]["codec_level_note"]


def test_a_corpus_with_no_axis_run_is_stated_not_crashed(tmp_path):
    """Absence is normal, as for compression before it and ordering beside it."""
    from benchviz import figures

    bench = _bench_dir(tmp_path)
    shutil.rmtree(bench / "scaling_codec_results")
    data, _ = prep.build(prep.Inputs(bench))
    assert data["scaling"]["codec"] == {"records": [], "sizes": [], "gaps": [], "variants": []}
    assert data["scaling"]["rowgroup"]["records"]
    assert data["meta"]["machine"]["codec"] is None

    data_path = tmp_path / "no_codec.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")
    written = sorted(
        p.name for p in figures.main(data_path=data_path, out_dir=tmp_path / "f").glob("*")
    )
    assert "codec.svg" not in written
    assert "rowgroup.svg" in written
    assert "compression.svg" not in written


def test_axis_gaps_are_named_not_dropped(tmp_path):
    bench = _bench_dir(tmp_path)
    axis_dir = bench / "scaling_codec_results"
    header = (axis_dir / "delft.csv").read_text(encoding="utf-8").splitlines()[0]
    (axis_dir / "empty.csv").write_text(header + "\n", encoding="utf-8")
    rows = (axis_dir / "delft.csv").read_text(encoding="utf-8").splitlines()
    kept = [r for r in rows if not r.startswith("delft.city.jsonl,cityparquet,")]
    (axis_dir / "nobase.csv").write_text("\n".join(kept) + "\n", encoding="utf-8")
    (axis_dir / "MACHINE.md").write_text("# Measurement host\n\nfake\n", encoding="utf-8")

    data, _ = prep.build(prep.Inputs(bench))
    gaps = {(g["dataset"], g["issue"]) for g in data["scaling"]["codec"]["gaps"]}
    assert ("empty", "CSV present but header-only") in gaps
    assert ("nobase", "no 'cityparquet' baseline rows") in gaps
    assert data["meta"]["machine"]["codec"].startswith("# Measurement host")


def test_the_axis_sheets_render_from_the_measured_fixture(tmp_path):
    """Both configuration axes draw from one function, one slice or many."""
    from benchviz import figures

    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    data_path = tmp_path / "axes.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")
    written = sorted(
        p.name for p in figures.main(data_path=data_path, out_dir=tmp_path / "f").glob("*")
    )
    for name in ("codec.svg", "codec.png", "rowgroup.svg", "rowgroup.png"):
        assert name in written
    assert "compression.svg" not in written

    title, subtitle = figures._axis_headline(data, "rowgroup")
    assert "512" in title or "2048" in title or "floor" in title
    assert "2231" in subtitle.replace(",", "") or "1 slice" in subtitle

    # The corpus names itself from the slice ids. Hand-typing "3DBAG" into a
    # computed subtitle survives a run of a different corpus unchanged, which
    # is a caption asserting something the records never said.
    assert "(delft)" in subtitle
    assert "3DBAG" not in subtitle

    # Every ratio in the axis data is baseline over variant, so zstd 1 writing a
    # BIGGER file than the default reads as a size_ratio below 1x and its bar
    # points left. A sentence phrased "of the default's bytes" is the other way
    # up, and has said so backwards once already.
    codec_title, _ = figures._axis_headline(data, "codec")
    default_bytes, zstd1_bytes = (
        next(
            s["bytes"]
            for s in data["scaling"]["codec"]["sizes"]
            if s["variant"] == variant
        )
        for variant in ("cityparquet", "cityparquet+zstd1")
    )
    assert zstd1_bytes > default_bytes
    assert figures._times(zstd1_bytes / default_bytes) in codec_title


def test_the_axis_vocabulary_survives_the_real_recipes(tmp_path):
    """Two shapes the pinned fixture's short variant lists cannot show.

    `just rowgroup-bench` sweeps the group sizes DOWNWARD, so a ramp that
    follows the order the CSVs arrive in paints the largest group lightest —
    backwards, for a sequential hue. And `just codec-bench` carries an
    uncompressed variant, whose key-strip label ("none") says the opposite of
    what it means once a sentence puts it in subject position.
    """
    from benchviz import figures

    recipe = [
        "cityparquet",
        "cityparquet+rg32768",
        "cityparquet+rg8192",
        "cityparquet+rg2048",
        "cityparquet+rg512",
    ]
    palette = figures._axis_palette("rowgroup", recipe)
    by_size = sorted(recipe[1:], key=lambda v: int(v.removeprefix("cityparquet+rg")))
    lightness = [sum(figures.mcolors.to_rgb(palette[v])) for v in by_size]
    assert lightness == sorted(lightness, reverse=True), (
        f"small groups must be lightest: {list(zip(by_size, lightness, strict=True))}"
    )

    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    axis = data["scaling"]["codec"]
    # The fixture's fastest full read, renamed to the variant the recipe carries.
    axis["variants"] = [
        "cityparquet+uncompressed" if v == "cityparquet+lz4" else v
        for v in axis["variants"]
    ]
    for row in axis["records"] + axis["sizes"]:
        if row["variant"] == "cityparquet+lz4":
            row["variant"] = "cityparquet+uncompressed"
    fastest = max(
        (r["time_ratio"], r["variant"])
        for r in axis["records"]
        if r["measure"] == "full-read" and r["time_ratio"]
    )[1]
    assert fastest == "cityparquet+uncompressed"

    title, _ = figures._axis_headline(data, "codec")
    assert "uncompressed reads fastest" in title
    assert "none reads" not in title
    # Beside its swatch, where the column of codec names is the context, it
    # stays the word the key strip has room for.
    assert figures._variant_label("cityparquet+uncompressed") == "none"


def test_the_codec_headline_crowns_a_winner_only_outside_the_noise(tmp_path):
    """A 4 % lead over a 3-5 % dispersion is not a fastest codec.

    On the measured 1M slice the default full read takes 50.93 s with a MAD of
    1.73 and snappy 49.17 s with a MAD of 2.36. Ranking the two names the top
    of a list the run cannot order, so the sentence may name a codec only when
    its lead over the default clears both MADs together — and must otherwise
    say that the axis found no separation.
    """
    from benchviz import figures

    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    axis = data["scaling"]["codec"]
    reads = [r for r in axis["records"] if r["measure"] == "full-read"]
    assert all(r["time_mad_s"] is not None for r in reads)

    # As measured on the fixture slice, lz4 leads by 0.030 s over MADs summing
    # to 0.009 — separable, and named.
    title, _ = figures._axis_headline(data, "codec")
    assert "lz4 reads fastest" in title

    # The same times read with a dispersion that swallows every lead.
    for record in reads:
        record["base_time_mad_s"] = record["time_mad_s"] = 0.5
    noisy, _ = figures._axis_headline(data, "codec")
    assert "every codec reads within measurement noise of the default" in noisy
    assert "reads fastest" not in noisy
    assert "except" not in noisy

    # A codec slower than the default by more than the two MADs is still an
    # effect, and the clause names it rather than dropping it into the parity.
    slow = next(r for r in reads if r["variant"] == "cityparquet+lz4")
    slow["time_s"] = slow["base_time_s"] * 10
    slow["time_ratio"] = 0.1
    named, _ = figures._axis_headline(data, "codec")
    assert f"except lz4, at {figures._times(0.1)}" in named


def test_the_machine_line_describes_the_host_without_naming_it(tmp_path):
    """The footer answers "what was this measured on", not "where does it live".

    `machine_record.sh` captures `uname -srm`, so no hostname is recorded; the
    line is assembled from the CPU model, the core count and the memory total
    rather than quoted off the top of the file, which used to print an internal
    FQDN onto a figure bound for publication.
    """
    from benchviz import figures

    record = (
        "# Measurement host\n\n"
        "Captured by benchmark/scripts/machine_record.sh at 2026-09-06T02:15:33Z.\n\n"
        "```\n"
        "Linux 6.8.0-136-generic x86_64\n"
        "Architecture:                            x86_64\n"
        "CPU(s):                                  128\n"
        "On-line CPU(s) list:                     0-127\n"
        "Vendor ID:                               AuthenticAMD\n"
        "Model name:                              AMD EPYC 7542 32-Core Processor\n"
        "               total        used        free\n"
        "Mem:     540725092352 67899543552  6151950336\n"
        "rustc 1.93.1 (01f6ddf75 2026-02-11)\n"
        "```\n"
    )
    note = figures._machine_note(record)
    assert "AMD EPYC 7542 32-Core Processor" in note
    assert "128 CPUs" in note
    assert "541 GB" in note
    assert "Linux 6.8.0-136-generic x86_64" in note
    assert "gilfoyle" not in note and ".tudelft.nl" not in note

    assert figures._machine_note(None) == "No machine record for this run."
    without_memory = "\n".join(
        ln for ln in record.splitlines() if not ln.startswith("Mem:")
    )
    assert figures._machine_note(without_memory) == "No machine record for this run."


def test_the_row_group_headline_names_the_best_trade_off(tmp_path):
    """The figure asks which group size and when, so the sentence names the winner.

    The LARGEST size that clears the floor is the smallest departure from the
    default, which is a different question. On the measured 1M slice 8,192 rows
    answer the window faster than 32,768 rows AND write faster, so a headline
    reaching for the largest clearing size names the size that lost on both
    counts.
    """
    from benchviz import figures

    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    axis = data["scaling"]["rowgroup"]
    dataset = figures._axis_slices(axis)[0]["id"]
    axis["variants"] = ["cityparquet", "cityparquet+rg32768", "cityparquet+rg8192"]
    spatial = {"cityparquet+rg8192": 2.29, "cityparquet+rg32768": 2.22}
    writes = {"cityparquet+rg8192": 1.01, "cityparquet+rg32768": 0.99}
    proto = axis["records"][0]
    # Seconds and MADs consistent with the ratios above, because the write
    # clause reads them: at a 2 s dispersion either way, a 1 % write ratio is
    # the writer's own spread and the sentence must not price it.
    axis["records"] = [
        dict(proto, dataset=dataset, variant=v, measure=measure,
             time_ratio=table[v], below_floor=False,
             base_time_s=100.0, time_s=100.0 / table[v],
             base_time_mad_s=mad, time_mad_s=mad)
        for measure, table, mad in (("bbox-5pct", spatial, 0.0), ("write", writes, 2.0))
        for v in spatial
    ]

    title, _ = figures._axis_headline(data, "rowgroup")
    assert "8,192 rows" in title
    assert "32,768" not in title
    assert figures._times(2.29) in title
    assert "for the same write time" in title

    # Tighten the write runs and the same 1 % is an effect, priced as one.
    for record in axis["records"]:
        if record["measure"] == "write":
            record["base_time_mad_s"] = record["time_mad_s"] = 0.0
    priced, _ = figures._axis_headline(data, "rowgroup")
    assert f"for {figures._times(1 / 1.01)} the write time" in priced

    # A tie goes the other way: the larger group is the smaller departure from
    # the default, so it wins when nothing separates them on the window.
    for record in axis["records"]:
        if record["measure"] == "bbox-5pct":
            record["time_ratio"] = 2.29
    tied, _ = figures._axis_headline(data, "rowgroup")
    assert "32,768 rows" in tied


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


# Two generations of id-lookup notes live in the committed results at once:
# `read_results/` carries the positional probes the current runner emits
# (READ_BENCHMARK.md's `id-10pct` / `id-50pct` / `id-90pct` / `id-miss` table),
# while `scaling_read_results/` and `scaling_ordering_results/` still hold the
# single `id=<identifier>` probe of the run that produced them. Those scaling
# artefacts are hours of measurement and are not regenerated to suit the
# renderer, so both shapes have to key.
def test_positional_id_probes_are_separate_scenarios(tmp_path):
    root = _bench_dir(tmp_path)
    csv_path = root / "read_results" / "Zurich.csv"
    lines = csv_path.read_text(encoding="utf-8").splitlines()
    head = [ln for ln in lines if not ln.split(",")[2:3] == ["id-lookup"]]
    probes = []
    for tag in ("id-10pct", "id-50pct", "id-90pct", "id-miss"):
        count = "0" if tag == "id-miss" else "1"
        for fmt, note in (
            ("cityjsonseq", tag),
            ("cityparquet", tag),
            # flatcitybuf discloses that its attribute index was not used; the
            # tag is a suffix on the SAME probe, not a probe of its own.
            ("flatcitybuf", f"{tag};no-attr-index"),
        ):
            probes.append(
                f"Zurich.city.jsonl,{fmt},id-lookup,0.000005,{count},"
                f"0.01,0.0001,8366366,21020672,7,{note}"
            )
    csv_path.write_text("\n".join(head + probes) + "\n", encoding="utf-8")

    out = prep.main(prep.Inputs(root), out_path=tmp_path / "bench_data.json")
    data = prep.json.loads(out.read_text(encoding="utf-8"))

    zurich = [r for r in data["read"] if r["dataset"] == "Zurich"]
    keys = {r["scenario_key"] for r in zurich}
    assert {"id-10pct", "id-50pct", "id-90pct", "id-miss"} <= keys
    assert "id-lookup" not in keys, "the positional probes must not collapse"

    # The disclosure suffix must not strand flatcitybuf in a baseline-less group
    # of its own -- it is measured against the same probe as every other format.
    at_10pct = {r["format"] for r in zurich if r["scenario_key"] == "id-10pct"}
    assert {"cityjsonseq", "cityparquet", "flatcitybuf"} <= at_10pct


def test_the_older_single_id_probe_still_keys_as_id_lookup(tmp_path):
    root = _bench_dir(tmp_path)
    out = prep.main(prep.Inputs(root), out_path=tmp_path / "bench_data.json")
    data = prep.json.loads(out.read_text(encoding="utf-8"))

    scaling = data["scaling"]["read"] if "read" in data["scaling"] else []
    assert any(r["scenario_key"] == "id-lookup" for r in scaling + data["read"])
