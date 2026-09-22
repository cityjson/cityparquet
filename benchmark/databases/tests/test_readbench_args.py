from dataclasses import replace

import pytest

from citybench.config import BBox, Dataset, Params
from citybench.systems import readbench
from citybench.systems.readbench import ReadbenchSystem, build_child_args, parse_child_stdout
from conftest import ge_attr_filter, make_params

PARAMS = make_params()


def test_count_args_are_minimal():
    args = build_child_args("count", PARAMS, "/pkg")
    assert "--child" in args
    assert "--format" in args and "cityparquet" in args
    assert "--scenario" in args and "count" in args
    assert "--input" in args and "/pkg" in args


def test_bbox_args_pass_six_comma_separated_ordinates():
    window = PARAMS.window("bbox-25pct")
    args = build_child_args("bbox-query", PARAMS, "/pkg", window)
    i = args.index("--bbox")
    # The resolved window verbatim — the SAME six numbers every SQL system
    # binds, never a re-derivation on this side.
    assert args[i + 1] == ",".join(str(v) for v in window.window.as_cli_list())
    # z is never narrowed: the window spans the dataset's full z range.
    assert args[i + 1].split(",")[2] == str(PARAMS.bbox_full.minz)


def test_bbox_args_carry_a_selectivity_tag():
    args = build_child_args("bbox-query", PARAMS, "/pkg",
                            PARAMS.window("bbox-5pct"))
    i = args.index("--selectivity-tag")
    assert args[i + 1] == "bbox-5pct"


def test_attr_filter_passes_the_derived_attribute_not_a_structural_column():
    args = build_child_args("attr-filter", PARAMS, "/pkg")
    assert args[args.index("--attr-column") + 1] == "b3_dak_type"
    assert args[args.index("--attr-eq") + 1] == "slanted"
    # `object_type` is not a CityJSON attribute, so FlatCityBuf's B+-tree
    # can never index it — the defect that made every FCB `attr-filter` row
    # a full walk (review §0).
    assert "object_type" not in args


def test_attr_filter_passes_the_numeric_lower_bound_form():
    args = build_child_args(
        "attr-filter", replace(PARAMS, attr_filter=ge_attr_filter()), "/pkg"
    )
    assert args[args.index("--attr-column") + 1] == "TerrainHeight"
    assert args[args.index("--attr-ge") + 1] == "2.45"
    assert "--attr-eq" not in args


def test_attr_stats_uses_the_numeric_column():
    args = build_child_args("attr-stats", PARAMS, "/pkg")
    assert args[args.index("--attr-column") + 1] == "h_dak_max"


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # Mirrors every sql_*.sql_for's own parent_id-is-None guard for
    # hierarchy: a dataset with no numeric attribute at all (Montreal,
    # lod3_railway — see params.py's derive()) must not silently pass the
    # string "None" as --attr-column to the Rust child.
    from citybench.scenarios.registry import ScenarioUnavailable
    no_numeric = replace(PARAMS, numeric_column=None)
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        build_child_args("attr-stats", no_numeric, "/pkg")


def test_id_lookup_passes_target_id():
    args = build_child_args("id-lookup", PARAMS, "/pkg")
    assert args[args.index("--target-id") + 1] == "obj-1"


def test_tier2_scenarios_are_rejected():
    # The Rust child implements only the inherited seven.
    with pytest.raises(ValueError):
        build_child_args("hierarchy", PARAMS, "/pkg")


def test_parse_child_stdout_local_four_fields():
    # "<time_s> <peak_heap_bytes> <ru_maxrss_bytes> <result_count>"
    count, time_s, heap, rss = parse_child_stdout("0.012345 4096 20480 2231\n")
    assert count == 2231
    assert time_s == 0.012345
    assert heap == 4096
    assert rss == 20480


def test_parse_child_stdout_http_six_fields_ignores_trailing_two():
    count, time_s, heap, rss = parse_child_stdout("0.5 1 2 7 999 3")
    assert (count, time_s, heap, rss) == (7, 0.5, 1, 2)


def test_parse_child_stdout_uses_last_line():
    count, _, _, _ = parse_child_stdout("warning: something\n0.1 1 2 42\n")
    assert count == 42


def test_parse_child_stdout_rejects_unexpected_field_count():
    with pytest.raises(ValueError):
        parse_child_stdout("0.1 1 2")


# --- Beyond the brief -------------------------------------------------
#
# The brief's own tests above are a floor, not a ceiling (see the task
# instructions). Everything below covers a branch or piece of state that
# the tests above happen not to exercise, but that a typo or inverted
# condition would slip straight through.


def test_bbox_query_without_a_window_raises():
    # `window` defaults to None; every non-windowed scenario tolerates
    # that, but bbox-query has nothing to build a window from without it.
    with pytest.raises(ValueError):
        build_child_args("bbox-query", PARAMS, "/pkg")


def test_bbox_args_carry_the_1pct_selectivity_tag_too():
    args = build_child_args("bbox-query", PARAMS, "/pkg",
                            PARAMS.window("bbox-1pct"))
    assert args[args.index("--selectivity-tag") + 1] == "bbox-1pct"


def test_scenarios_the_rust_child_does_not_implement_are_refused_loudly():
    """`geometry-scan`, `bbox-fetch`, `point-query`, `attr-range` and the
    write tier have no counterpart in the child's own `Scenario` enum, and
    the read harness is not this family's to extend. The registry never
    asks the native readers for them (`READBENCH_SCENARIOS`); this guard is
    the second line of defence if it ever did."""
    for scenario in ("geometry-scan", "bbox-fetch", "point-query",
                     "attr-range", "parts-per-building", "attr-add"):
        with pytest.raises(ValueError, match="not implemented by the readbench child"):
            build_child_args(scenario, PARAMS, "/pkg")


def _dataset(tmp_path) -> Dataset:
    return Dataset(
        name="delft",
        source=tmp_path / "delft.city.jsonl",
        cityparquet_dir=tmp_path / "cityparquet" / "delft",
        hilbert_dir=tmp_path / "cityparquet" / "delft-hilbert",
    )


def test_tag_reflects_the_hilbert_flag(tmp_path):
    # ReadbenchSystem serves two registry tags off one class (hence it is
    # deliberately not @register-decorated); each constructor argument
    # must land on the right one.
    assert ReadbenchSystem(binary=tmp_path / "bin").tag == "cityparquet"
    assert ReadbenchSystem(binary=tmp_path / "bin", hilbert=True).tag == "cityparquet-hilbert"


def test_prepare_raises_file_not_found_when_binary_is_missing(tmp_path):
    system = ReadbenchSystem(binary=tmp_path / "no-such-binary")
    with pytest.raises(FileNotFoundError):
        system.prepare()


def test_prepare_does_not_raise_when_binary_exists(tmp_path):
    binary = tmp_path / "cityparquet-readbench"
    binary.write_bytes(b"")
    ReadbenchSystem(binary=binary).prepare()  # must not raise


def test_size_sums_every_file_under_the_ingested_package(tmp_path):
    package = tmp_path / "cityparquet" / "delft"
    package.mkdir(parents=True)
    (package / "building.parquet").write_bytes(b"x" * 100)
    sidecars = package / "sidecars"
    sidecars.mkdir()
    (sidecars / "materials.parquet").write_bytes(b"y" * 50)

    system = ReadbenchSystem(binary=tmp_path / "bin")
    system.ingest(_dataset(tmp_path))
    report = system.size()

    assert report.size_bytes == 150
    assert report.size_bytes_no_index == 150


class _FakeCompletedProcess:
    def __init__(self, stdout: str) -> None:
        self.stdout = stdout
        self.stderr = ""


def test_ingest_routes_a_hilbert_system_to_the_hilbert_package_and_run_reports_it(
    tmp_path, monkeypatch
):
    dataset = _dataset(tmp_path)
    captured: dict = {}

    def fake_run(argv, **kwargs):
        captured["argv"] = argv
        return _FakeCompletedProcess("0.1 100 200 5\n")

    monkeypatch.setattr(readbench.subprocess, "run", fake_run)

    system = ReadbenchSystem(binary=tmp_path / "bin", hilbert=True)
    ingest_result = system.ingest(dataset)
    assert ingest_result.wall_clock_s == 0.0

    system.run("count", PARAMS, repeat=1)

    # ingest() must have pointed --input at hilbert_dir, not cityparquet_dir.
    argv = captured["argv"]
    assert str(dataset.hilbert_dir) in argv
    assert str(dataset.cityparquet_dir) not in argv
    # ...and --format must match, since this is the flag that tells the
    # child which artefact layout it is opening.
    assert argv[argv.index("--format") + 1] == "cityparquet-hilbert"


def test_ingest_routes_a_plain_system_to_the_source_ordered_package(tmp_path, monkeypatch):
    dataset = _dataset(tmp_path)
    captured: dict = {}

    def fake_run(argv, **kwargs):
        captured["argv"] = argv
        return _FakeCompletedProcess("0.1 100 200 5\n")

    monkeypatch.setattr(readbench.subprocess, "run", fake_run)

    system = ReadbenchSystem(binary=tmp_path / "bin", hilbert=False)
    system.ingest(dataset)
    system.run("count", PARAMS, repeat=1)

    argv = captured["argv"]
    assert str(dataset.cityparquet_dir) in argv
    assert str(dataset.hilbert_dir) not in argv
    assert argv[argv.index("--format") + 1] == "cityparquet"


def test_run_discards_the_warmup_and_reports_the_repeats_own_peak(tmp_path, monkeypatch):
    # The warm-up's numbers (9.0s, 9000 bytes peak heap, 9000 bytes peak
    # rss, a bogus count of 999) are deliberately the most extreme of the
    # four responses: if the warm-up were folded into the reported samples
    # instead of discarded, or if peak_*_bytes picked the wrong sample
    # instead of the max, either bug would leak the warmup's numbers into
    # the assertions below.
    responses = iter([
        "9.000000 9000 9000 999\n",  # discarded warm-up
        "0.100000 100 200 5\n",
        "0.200000 150 180 5\n",
        "0.050000 120 220 5\n",
    ])

    def fake_run(argv, **kwargs):
        return _FakeCompletedProcess(next(responses))

    monkeypatch.setattr(readbench.subprocess, "run", fake_run)

    system = ReadbenchSystem(binary=tmp_path / "bin")
    system.ingest(_dataset(tmp_path))
    measurement = system.run("count", PARAMS, repeat=3)

    assert measurement.result_count == 5
    assert measurement.times_s == [0.1, 0.2, 0.05]
    assert measurement.peak_heap_bytes == 150  # max(100, 150, 120), never 9000
    assert measurement.peak_rss_bytes == 220   # max(200, 180, 220), never 9000
    assert measurement.server_times_s == []
