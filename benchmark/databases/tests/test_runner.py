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
    ALL, ID_PROBE_SCENARIOS, SELECTIVITY_SCENARIOS, ScenarioUnavailable,
    systems_for,
)
from conftest import ge_attr_filter, make_params

PARAMS = make_params(numeric_column="h")


class FakeSystem:
    def __init__(self, tag, count):
        self.tag = tag
        self._count = count

    def prepare(self): ...
    def teardown(self): ...

    def run(self, scenario, params, repeat, window=None, probe=None):
        return Measurement(
            result_count=self._count,
            times_s=[0.1] * repeat,
            server_times_s=[],
            peak_working_mem_bytes=None,
        )


class RaisingSystem:
    """A fake whose `run` always raises `exc`, e.g. to exercise error paths."""

    def __init__(self, tag, exc):
        self.tag = tag
        self._exc = exc

    def prepare(self): ...
    def teardown(self): ...

    def run(self, scenario, params, repeat, window=None, probe=None):
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
    """3DBAG bbox counts differ by a few hundredths of a percent for reasons
    that are properties of the compared systems, not of the query: cjdb's
    importer drops 2/16/60 BuildingPart footprints whose non-vertical faces
    share one Z, and PostGIS's float4 `&&` admits objects just outside a
    window (review §5); the counts below are fixture values of that shape. The decomposition is what a reader can act on;
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
    rows = run_matrix(systems, PARAMS, "delft", repeat=3, scenarios=("geometry-scan",))
    assert len(rows) == 2
    assert all(r["notes"].startswith("count-mismatch") for r in rows)
    assert all(r["status"] == "mismatch" for r in rows)


def test_run_matrix_marks_a_within_tolerance_deviation_without_failing_it():
    systems = [FakeSystem("cjdb", 220949), FakeSystem("3dcitydb", 221005)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1, scenarios=("geometry-scan",))
    assert all(r["status"] == "ok-deviation" for r in rows)
    # The detail is NOT dropped: it is the whole value of the row.
    assert all("count-mismatch" in r["notes"] for r in rows)


def test_run_matrix_tolerance_is_threaded_through_from_the_caller():
    systems = [FakeSystem("cjdb", 99), FakeSystem("3dcitydb", 100)]
    loose = run_matrix(systems, PARAMS, "delft", repeat=1,
                       scenarios=("geometry-scan",), tolerance=0.02)
    assert all(r["status"] == "ok-deviation" for r in loose)
    strict = run_matrix(systems, PARAMS, "delft", repeat=1,
                        scenarios=("geometry-scan",), tolerance=0.0003)
    assert all(r["status"] == "mismatch" for r in strict)


def test_run_matrix_stamps_the_thread_configuration_onto_every_row():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1, scenarios=("geometry-scan",),
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
    rows = run_matrix(systems, PARAMS, "delft", repeat=3, scenarios=("geometry-scan",))
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


def test_non_windowed_non_excluded_scenario_still_reports_selectivity():
    # The discriminating case: `attr-filter` has no window target (target
    # is None throughout), yet per the inherited CSV contract
    # ("selectivity = result_count / total_object_count, empty where N/A
    # (count, geometry-scan)") it MUST still report result_count/total. A rule
    # that gates on "has a window" rather than on scenario identity fails
    # this test while still passing every other selectivity test here.
    systems = [FakeSystem("cjdb", 25)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("attr-filter",))
    assert rows[0]["selectivity"] == "0.250000"


def test_other_non_windowed_scenarios_also_report_selectivity():
    # Covered individually so a rule that special-cases just one of them
    # cannot slip through.
    for scenario in ("attr-stats", "attr-range", "lod-query"):
        systems = [FakeSystem("cjdb", 10)]
        rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=(scenario,))
        assert rows[0]["selectivity"] == "0.100000", scenario


def test_no_selectivity_scenarios_is_the_whole_dataset_reads_plus_the_writes():
    # Locks the exclusion set itself: a scenario silently added to or
    # dropped from this constant would otherwise only be caught by chance.
    # `geometry-scan` answers over the whole dataset; the write
    # tier's result_count is rows TOUCHED by a mutation, which is not a
    # selection either.
    assert NO_SELECTIVITY_SCENARIOS == frozenset({
        "geometry-scan", "attr-add", "attr-update", "attr-delete",
        "append-object",
    })


def test_run_matrix_records_repeat_count():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=7, scenarios=("geometry-scan",))
    assert rows[0]["repeat"] == "7"


def test_run_matrix_stamps_sizes_onto_every_row_of_that_system():
    systems = [FakeSystem("cjdb", 10), FakeSystem("duckdb-cityparquet", 10)]
    rows = run_matrix(
        systems, PARAMS, "delft", repeat=2, scenarios=("geometry-scan", "attr-stats"),
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
    rows = run_matrix(systems, PARAMS, "delft", repeat=2, scenarios=("geometry-scan",))
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
    m = Measurement(result_count=1, times_s=[0.1], server_times_s=[], peak_working_mem_bytes=None)
    assert _add_note(m, "") is m


def test_add_note_sets_notes_when_previously_blank():
    m = Measurement(result_count=1, times_s=[0.1], server_times_s=[], peak_working_mem_bytes=None)
    assert _add_note(m, "bbox-1pct").notes == "bbox-1pct"


def test_add_note_appends_with_a_single_separating_space():
    m = Measurement(
        result_count=1, times_s=[0.1], server_times_s=[], peak_working_mem_bytes=None,
        notes="skipped: no hierarchy",
    )
    assert _add_note(m, "bbox-1pct").notes == "skipped: no hierarchy bbox-1pct"


def test_failed_produces_a_measurement_with_no_result_and_the_given_note():
    m = _failed("error: RuntimeError")
    assert m.result_count is None
    assert m.times_s == []
    assert m.server_times_s == []
    assert m.peak_working_mem_bytes is None
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
    # No `scenarios=` kwarg: every scenario cjdb answers runs, with
    # `bbox-query` expanding into three window rows and `id-lookup` into
    # four id-probe rows.
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1)
    answered = [s for s in ALL if "cjdb" in systems_for(s)]
    windowed = [s for s in answered if s in SELECTIVITY_SCENARIOS]
    probed = [s for s in answered if s in ID_PROBE_SCENARIOS]
    assert len(rows) == (
        len(answered) - len(windowed) - len(probed)
        + 3 * len(windowed) + 4 * len(probed)
    )


def test_id_lookup_expands_into_one_row_per_probe_tagged_in_notes():
    """Four probes, four rows, each naming which probe it asked for — a
    single `id-lookup` time would be a function of where that one id
    happened to sit in the stream."""
    systems = [FakeSystem("cjdb", 1)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1,
                      scenarios=("id-lookup",))
    assert len(rows) == 4
    assert [r["notes"] for r in rows] == [
        "id-10pct", "id-50pct", "id-90pct", "id-miss",
    ]


def test_each_id_probe_is_cross_checked_against_itself_not_against_the_others():
    """The miss probe legitimately returns 0 where the hits return 1. If
    the four probes shared one row, that difference would read as a
    cross-system count mismatch."""

    class ProbeAwareSystem(FakeSystem):
        def run(self, scenario, params, repeat, window=None, probe=None):
            self._count = 1 if probe is None or probe.present else 0
            return super().run(scenario, params, repeat, window, probe)

    systems = [ProbeAwareSystem("cjdb", 1), ProbeAwareSystem("3dcitydb", 1)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1,
                      scenarios=("id-lookup",))
    assert all(r["status"] == "ok" for r in rows)
    assert {r["result_count"] for r in rows} == {"1", "0"}


def test_run_matrix_dataset_name_is_stamped_onto_every_row():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "rotterdam", repeat=1, scenarios=("geometry-scan",))
    assert rows[0]["dataset"] == "rotterdam"


class FakeGate:
    def __init__(self, busy):
        self.busy = busy
        self.labels = []
        self.after = 0

    def before_cell(self, label):
        self.labels.append(label)
        return self.busy

    def after_cell(self):
        self.after += 1


def test_run_matrix_consults_the_load_gate_around_every_cell():
    gate = FakeGate(busy=False)
    systems = [FakeSystem("cjdb", 10), FakeSystem("3dcitydb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1, scenarios=("geometry-scan",),
                      run_note="threads=single", gate=gate)
    assert gate.labels == ["threads=single geometry-scan cjdb",
                           "threads=single geometry-scan 3dcitydb"]
    assert gate.after == 2
    assert all("busy" not in row["notes"].split() for row in rows)


def test_run_matrix_tags_a_cell_busy_when_the_gate_gave_up_waiting():
    systems = [FakeSystem("cjdb", 10)]
    rows = run_matrix(systems, PARAMS, "delft", repeat=1, scenarios=("geometry-scan",),
                      run_note="threads=single", gate=FakeGate(busy=True))
    assert "busy" in rows[0]["notes"].split()


# --- the identifier-set cross-check -----------------------------------------


class VerifyingSystem(FakeSystem):
    """A system that also answers the untimed verification pass."""

    def __init__(self, tag, ids, geometry=None):
        super().__init__(tag, len(ids))
        self._ids = ids
        self._geometry = geometry
        self.verified = 0

    def verify_rows(self, scenario, params, window=None, probe=None):
        self.verified += 1
        if self._geometry is None:
            return ["id"], [(i,) for i in self._ids]
        return ["id", "geometry"], [(i, self._geometry) for i in self._ids]


def test_same_counts_but_different_ids_fail_the_row_as_id_mismatch():
    systems = [VerifyingSystem("duckdb-cityparquet", ["a", "b"]),
               VerifyingSystem("cjdb", ["a", "c"])]
    rows = run_matrix(systems, make_params(), "d", repeat=1,
                      scenarios=["attr-filter"])
    assert {r["status"] for r in rows} == {"id-mismatch"}
    assert all("only in cjdb ['c']" in r["notes"] for r in rows)


def test_agreeing_ids_keep_the_count_status_and_verify_once_per_system():
    systems = [VerifyingSystem("duckdb-cityparquet", ["a", "b"]),
               VerifyingSystem("cjdb", ["b", "a"])]
    rows = run_matrix(systems, make_params(), "d", repeat=3,
                      scenarios=["attr-filter"])
    assert {r["status"] for r in rows} == {"ok"}
    assert [s.verified for s in systems] == [1, 1]


def test_non_object_scenarios_are_not_verified():
    systems = [VerifyingSystem("duckdb-cityparquet", ["a"]),
               VerifyingSystem("cjdb", ["a"])]
    run_matrix(systems, make_params(), "d", repeat=1, scenarios=["geometry-scan"])
    assert [s.verified for s in systems] == [0, 0]


# --- cjdb's footprint-only bbox answer, verified and decomposed ---

import pytest  # noqa: E402
from citybench import identity as _identity  # noqa: E402
from citybench.runner import explained_footprint_deviation  # noqa: E402


class _Footprint:
    tag = "cjdb"

    def __init__(self, null_ids):
        self.null_ids = null_ids

    def footprint_decomposition(self, ids):
        nulls = sum(1 for i in ids if i in self.null_ids)
        return {"null-footprint": nulls, "footprint-outside-window": len(ids) - nulls}


def _ids(*ids):
    return _identity.Identity(frozenset(ids), len(ids))


def test_a_footprint_subset_is_an_explained_deviation_when_the_others_agree():
    summaries = {"duckdb-cityparquet": _ids("a", "b", "c"), "cjdb": _ids("a"),
                 "3dcitydb": _ids("a", "b", "c")}
    note = explained_footprint_deviation("bbox-query", summaries, [_Footprint({"b"})])
    assert note == ("explained-deviation: cjdb tests its footprint, lacks 2 of 3 "
                    "(null-footprint=1 footprint-outside-window=1; README Caveats 10-11)")


@pytest.mark.parametrize("summaries", [
    # cjdb returns an id the reference lacks: not a footprint undercount
    {"duckdb-cityparquet": _ids("a", "b"), "cjdb": _ids("a", "z"), "3dcitydb": _ids("a", "b")},
    # the box-testing systems disagree with each other: a defect, not cjdb's
    {"duckdb-cityparquet": _ids("a", "b"), "cjdb": _ids("a"), "3dcitydb": _ids("a", "b", "c")},
])
def test_anything_else_stays_unexplained(summaries):
    assert explained_footprint_deviation("bbox-query", summaries, [_Footprint(set())]) is None


def test_only_bbox_query_is_explained():
    summaries = {"duckdb-cityparquet": _ids("a", "b"), "cjdb": _ids("a")}
    assert explained_footprint_deviation("attr-range", summaries, [_Footprint(set())]) is None
