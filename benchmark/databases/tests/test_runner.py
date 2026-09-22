from citybench.config import BBox, Measurement, Params
from citybench.runner import (
    NO_SELECTIVITY_SCENARIOS,
    _add_note,
    _failed,
    _selectivity,
    cross_check,
    run_matrix,
)
from citybench.scenarios.registry import (
    ALL, SELECTIVITY_SCENARIOS, ScenarioUnavailable, systems_for,
)
from conftest import ge_attr_filter, make_params

PARAMS = make_params(numeric_column="h", target_id="a")


class FakeSystem:
    def __init__(self, tag, count):
        self.tag = tag
        self._count = count

    def prepare(self): ...
    def teardown(self): ...

    def run(self, scenario, params, repeat, window=None):
        return Measurement(
            result_count=self._count,
            times_s=[0.1] * repeat,
            server_times_s=[],
            peak_rss_bytes=None,
        )


class RaisingSystem:
    """A fake whose `run` always raises `exc`, e.g. to exercise error paths."""

    def __init__(self, tag, exc):
        self.tag = tag
        self._exc = exc

    def prepare(self): ...
    def teardown(self): ...

    def run(self, scenario, params, repeat, window=None):
        raise self._exc


def test_cross_check_passes_when_all_agree():
    assert cross_check({"cjdb": 10, "3dcitydb": 10, "duckdb-cityparquet": 10}) == ("", "ok")


def test_cross_check_flags_disagreement_with_all_values():
    note, status = cross_check({"cjdb": 10, "3dcitydb": 9})
    assert note.startswith("count-mismatch")
    assert "cjdb=10" in note
    assert "3dcitydb=9" in note
    assert status == "mismatch"          # 10 % spread, far above tolerance


def test_cross_check_of_single_system_is_vacuously_fine():
    assert cross_check({"cjdb": 10}) == ("", "ok")


def test_cross_check_of_empty_dict_is_vacuously_fine():
    # run_matrix can reach this when every system for a scenario is
    # skipped or errored, leaving nothing to compare.
    assert cross_check({}) == ("", "ok")


def test_cross_check_publishes_a_small_spread_as_an_explained_deviation():
    """The committed 3DBAG bbox counts differ by 0.03-0.04 % for reasons
    that are properties of the compared systems, not of the query: cjdb's
    importer drops 2/16/60 BuildingPart footprints whose non-vertical faces
    share one Z, and PostGIS's float4 `&&` admits 4 extra objects at the
    25 % window (review §5). The decomposition is what a reader can act on;
    a binary pass/fail was not."""
    note, status = cross_check({"duckdb-cityparquet": 221005, "cjdb": 220949,
                                "3dcitydb": 221008})
    assert status == "ok-deviation"
    assert note.startswith("count-mismatch")
    assert "spread=" in note             # the decomposition stays in `notes`


def test_cross_check_keeps_a_large_spread_a_mismatch():
    _, status = cross_check({"cjdb": 99, "3dcitydb": 100})
    assert status == "mismatch"


def test_cross_check_tolerance_is_a_run_level_option():
    counts = {"cjdb": 99, "3dcitydb": 100}
    assert cross_check(counts, tolerance=0.02)[1] == "ok-deviation"
    assert cross_check(counts, tolerance=0.001)[1] == "mismatch"


def test_cross_check_does_not_divide_by_a_zero_maximum():
    # Defensive: a scenario legitimately returning zero rows everywhere
    # agrees and never reaches the spread, but a negative count would.
    assert cross_check({"cjdb": 0, "3dcitydb": 0}) == ("", "ok")


def test_run_matrix_tags_every_row_when_counts_disagree():
    systems = [FakeSystem("cjdb", 10), FakeSystem("3dcitydb", 9)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=3, scenarios=("count",))
    assert len(rows) == 2
    assert all(r["notes"].startswith("count-mismatch") for r in rows)
    assert all(r["status"] == "mismatch" for r in rows)


def test_run_matrix_marks_a_within_tolerance_deviation_without_failing_it():
    systems = [FakeSystem("cjdb", 220949), FakeSystem("3dcitydb", 221005)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1, scenarios=("count",))
    assert all(r["status"] == "ok-deviation" for r in rows)
    # The detail is NOT dropped: it is the whole value of the row.
    assert all("count-mismatch" in r["notes"] for r in rows)


def test_run_matrix_tolerance_is_threaded_through_from_the_caller():
    systems = [FakeSystem("cjdb", 99), FakeSystem("3dcitydb", 100)]
    loose = run_matrix(systems, PARAMS, "delft", repeat=1,
                       scenarios=("count",), tolerance=0.02)
    assert all(r["status"] == "ok-deviation" for r in loose)
    strict = run_matrix(systems, PARAMS, "delft", repeat=1,
                        scenarios=("count",), tolerance=0.0003)
    assert all(r["status"] == "mismatch" for r in strict)


def test_run_matrix_stamps_the_thread_configuration_onto_every_row():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1, scenarios=("count",),
                      run_note="threads=single")
    assert rows[0]["notes"].startswith("threads=single")


def test_run_matrix_only_asks_the_systems_the_registry_names_for_a_scenario():
    """`parts-per-building-join` is a DuckDB-only control, and the native
    Rust child implements none of the new scenarios: handing either a name
    it does not answer would be recorded as `error:`, indistinguishable
    from a real failure."""
    systems = [FakeSystem("duckdb-cityparquet", 10), FakeSystem("cjdb", 10),
               FakeSystem("cityparquet", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1,
                      scenarios=("parts-per-building-join",))
    assert [r["format"] for r in rows] == ["duckdb-cityparquet"]

    rows = run_matrix(systems, PARAMS, "delft", repeat=1,
                      scenarios=("geometry-scan",))
    assert "cityparquet" not in {r["format"] for r in rows}


def test_run_matrix_leaves_notes_clean_when_counts_agree():
    systems = [FakeSystem("cjdb", 10), FakeSystem("3dcitydb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=3, scenarios=("count",))
    assert all(r["notes"] == "" for r in rows)


def test_run_matrix_expands_bbox_into_three_window_rows():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("bbox-query",))
    assert len(rows) == 3
    # The three rows are distinguished by their window tag in `notes`,
    # matching the format harness's convention, and each one also carries
    # the fraction of rows the window ACTUALLY achieved — so "1 %" never
    # has to be taken on trust.
    tags = {r["notes"].split()[0] for r in rows}
    assert tags == {"bbox-1pct", "bbox-5pct", "bbox-25pct"}
    assert all("achieved=" in r["notes"] for r in rows)


def test_selectivity_column_is_result_count_over_total_not_the_window_target():
    # The spec defines selectivity as result_count / total_city_objects.
    # The fake returns 10 of 100 objects for every window, so all three
    # rows report 0.1 — the window target lives in `notes`, not here.
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("bbox-query",))
    assert {r["selectivity"] for r in rows} == {"0.100000"}


def test_scenarios_without_a_window_have_blank_selectivity():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("geometry-scan",))
    assert rows[0]["selectivity"] == ""


def test_count_scenario_also_has_blank_selectivity():
    # `full-read` alone cannot discriminate "blank because it has no
    # window" from "blank because it is in NO_SELECTIVITY_SCENARIOS" — both
    # rules agree on `full-read`. `count` is the second, independent member
    # of the inherited exclusion pair and must be blank too.
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("count",))
    assert rows[0]["selectivity"] == ""


def test_non_windowed_non_excluded_scenario_still_reports_selectivity():
    # The discriminating case: `attr-filter` has no window target (target
    # is None throughout), yet per the inherited CSV contract
    # ("selectivity = result_count / total_object_count, empty where N/A
    # (count, full-read)") it MUST still report result_count/total. A rule
    # that gates on "has a window" rather than on scenario identity fails
    # this test while still passing every other selectivity test here.
    systems = [FakeSystem("cjdb", 25)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("attr-filter",))
    assert rows[0]["selectivity"] == "0.250000"


def test_other_non_windowed_scenarios_also_report_selectivity():
    # Covered individually so a rule that special-cases just one of them
    # cannot slip through.
    for scenario in ("attr-stats", "id-lookup", "attr-range", "lod-extract"):
        systems = [FakeSystem("cjdb", 10)]
        rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=(scenario,))
        assert rows[0]["selectivity"] == "0.100000", scenario


def test_no_selectivity_scenarios_is_the_whole_dataset_reads_plus_the_writes():
    # Locks the exclusion set itself: a scenario silently added to or
    # dropped from this constant would otherwise only be caught by chance.
    # `count`/`geometry-scan` answer over the whole dataset; the write
    # tier's result_count is rows TOUCHED by a mutation, which is not a
    # selection either.
    assert NO_SELECTIVITY_SCENARIOS == frozenset({
        "count", "geometry-scan", "attr-add", "attr-update", "attr-delete",
    })


def test_run_matrix_records_repeat_count():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=7, scenarios=("count",))
    assert rows[0]["repeat"] == "7"


def test_run_matrix_stamps_sizes_onto_every_row_of_that_system():
    systems = [FakeSystem("cjdb", 10), FakeSystem("duckdb-cityparquet", 10)]
    rows = run_matrix(
        systems, PARAMS, "delft", repeat=2, scenarios=("count", "geometry-scan"),
        sizes={"cjdb": (900, 700), "duckdb-cityparquet": (400, 400)},
    )
    cjdb_rows = [r for r in rows if r["format"] == "cjdb"]
    assert len(cjdb_rows) == 2
    assert all(r["size_bytes"] == "900" for r in cjdb_rows)
    assert all(r["size_bytes_no_index"] == "700" for r in cjdb_rows)
    duck_rows = [r for r in rows if r["format"] == "duckdb-cityparquet"]
    assert all(r["size_bytes"] == "400" for r in duck_rows)


def test_run_matrix_leaves_sizes_blank_when_not_supplied():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("count",))
    assert rows[0]["size_bytes"] == ""


# --- Tests beyond the brief -------------------------------------------
#
# Added per the task's testing instruction: every public function and
# every branch must be covered by a test that would catch a typo or an
# inverted condition, not merely whatever the brief happened to list.


def test_selectivity_is_none_when_result_count_is_none():
    assert _selectivity(None, PARAMS) is None


def test_selectivity_is_none_when_total_city_objects_is_zero():
    assert _selectivity(10, make_params(total_city_objects=0)) is None


def test_selectivity_divides_result_count_by_total():
    assert _selectivity(25, PARAMS) == 0.25


def test_add_note_returns_measurement_unchanged_when_note_is_empty():
    m = Measurement(result_count=1, times_s=[0.1], server_times_s=[], peak_rss_bytes=None)
    assert _add_note(m, "") is m


def test_add_note_sets_notes_when_previously_blank():
    m = Measurement(result_count=1, times_s=[0.1], server_times_s=[], peak_rss_bytes=None)
    assert _add_note(m, "bbox-1pct").notes == "bbox-1pct"


def test_add_note_appends_with_a_single_separating_space():
    m = Measurement(
        result_count=1, times_s=[0.1], server_times_s=[], peak_rss_bytes=None,
        notes="skipped: no hierarchy",
    )
    assert _add_note(m, "bbox-1pct").notes == "skipped: no hierarchy bbox-1pct"


def test_failed_produces_a_measurement_with_no_result_and_the_given_note():
    m = _failed("error: RuntimeError")
    assert m.result_count is None
    assert m.times_s == []
    assert m.server_times_s == []
    assert m.peak_rss_bytes is None
    assert m.notes == "error: RuntimeError"


def test_run_matrix_scenario_unavailable_is_recorded_as_skipped_not_error():
    systems = [RaisingSystem("cjdb", ScenarioUnavailable("dataset has no parent/child hierarchy"))]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("parts-per-building",))
    assert len(rows) == 1
    assert rows[0]["notes"].startswith("skipped: ")
    assert "dataset has no parent/child hierarchy" in rows[0]["notes"]
    assert rows[0]["result_count"] == ""


def test_run_matrix_general_exception_is_recorded_as_error_not_skipped():
    systems = [RaisingSystem("cjdb", RuntimeError("connection refused"))]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("parts-per-building",))
    assert len(rows) == 1
    assert rows[0]["notes"].startswith("error: RuntimeError")
    assert not rows[0]["notes"].startswith("skipped")
    assert rows[0]["result_count"] == ""


def test_run_matrix_skipped_system_does_not_join_the_count_cross_check():
    # One system is skipped (no count to offer); the other two systems
    # agree with each other. The skipped row must not manufacture a
    # mismatch out of thin air, and the agreeing rows must stay clean.
    systems = [
        RaisingSystem("cjdb", ScenarioUnavailable("no hierarchy")),
        FakeSystem("3dcitydb", 10),
        FakeSystem("duckdb-cityparquet", 10),
    ]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("parts-per-building",))
    by_tag = {r["format"]: r for r in rows}
    assert by_tag["cjdb"]["notes"].startswith("skipped: ")
    assert "count-mismatch" not in by_tag["cjdb"]["notes"]
    assert by_tag["3dcitydb"]["notes"] == ""
    assert by_tag["duckdb-cityparquet"]["notes"] == ""


def test_run_matrix_still_flags_mismatch_among_answering_systems_when_one_is_skipped():
    # A skipped system must not silently absorb a real disagreement between
    # the systems that DID answer.
    systems = [
        RaisingSystem("cjdb", ScenarioUnavailable("no hierarchy")),
        FakeSystem("3dcitydb", 10),
        FakeSystem("duckdb-cityparquet", 9),
    ]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("parts-per-building",))
    by_tag = {r["format"]: r for r in rows}
    assert by_tag["3dcitydb"]["notes"].startswith("count-mismatch")
    assert by_tag["duckdb-cityparquet"]["notes"].startswith("count-mismatch")


def test_run_matrix_all_systems_skipped_leaves_no_stray_mismatch():
    systems = [
        RaisingSystem("cjdb", ScenarioUnavailable("no hierarchy")),
        RaisingSystem("3dcitydb", ScenarioUnavailable("no hierarchy")),
    ]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("parts-per-building",))
    assert all(r["notes"].startswith("skipped: ") for r in rows)
    assert all("count-mismatch" not in r["notes"] for r in rows)


def test_run_matrix_default_scenarios_cover_the_full_registry():
    # No `scenarios=` kwarg: every scenario cjdb answers runs, with the two
    # windowed scenarios expanding into three window rows each.
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1)
    answered = [s for s in ALL if "cjdb" in systems_for(s)]
    windowed = [s for s in answered if s in SELECTIVITY_SCENARIOS]
    assert len(rows) == len(answered) - len(windowed) + 3 * len(windowed)


def test_run_matrix_dataset_name_is_stamped_onto_every_row():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "rotterdam", repeat=1, scenarios=("count",))
    assert rows[0]["dataset"] == "rotterdam"
