"""Requires `just up`, `just build-citydb`, and an imported fixture."""

from pathlib import Path

import pytest

from citybench.config import Dataset
from citybench.params import derive
from citybench.systems.citydb import CityDbSystem

pytestmark = pytest.mark.integration

FIXTURE = Path(__file__).parent.parent / "data" / "delft.city.jsonl"


@pytest.fixture(scope="module")
def system():
    s = CityDbSystem()
    s.prepare()
    s.ingest(
        Dataset(
            name="delft",
            source=FIXTURE,
            cityparquet_dir=Path("data/cityparquet/delft"),
            hilbert_dir=Path("data/cityparquet-hilbert/delft"),
        )
    )
    yield s
    s.teardown()


def test_count_is_city_object_granular(system):
    """The spec's blocking item, asserted rather than assumed."""
    params = derive(FIXTURE)
    m = system.run("count", params, repeat=1)
    assert m.result_count == params.total_city_objects, (
        "3DCityDB count is not CityObject-granular; the restricting predicate "
        "in docs/3dcitydb-v5-schema.md is wrong or missing"
    )


def test_every_tier1_scenario_runs_and_reports_server_time(system):
    params = derive(FIXTURE)
    for scenario in ("full-read", "count", "attr-filter", "attr-stats",
                     "id-lookup", "project"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s[0] > 0
        assert m.server_times_s[0] > 0


def test_bbox_query_selectivity_is_monotonic(system):
    params = derive(FIXTURE)
    counts = [
        system.run("bbox-query", params, repeat=1, selectivity=s).result_count
        for s in (0.01, 0.05, 0.25)
    ]
    assert counts[0] <= counts[1] <= counts[2]


def test_tier2_scenarios_run(system):
    params = derive(FIXTURE)
    for scenario in ("lod-extract", "semantic-surface", "hierarchy"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s[0] > 0


# --- Beyond the brief -----------------------------------------------------
#
# The tests above are the brief's floor. The ones below exercise the
# whole-window bbox-query's exact count against ground truth, full-read's
# own CityObject-granular count, every filtered query's actual EXPLAIN
# plan (not just that it runs without error), the empty index_ddl()
# against a live schema, and the size report — all real, all only
# checkable against a live container.


def test_bbox_query_at_full_window_matches_the_city_object_count(system):
    # The whole dataset's bbox covers every feature's envelope, so a
    # correctly CityObject-granular bbox-query at selectivity 1.0 must
    # equal the same 2231 `count` reports — not 10045 (every feature,
    # verified without the predicate: see docs/3dcitydb-v5-schema.md's
    # "Index coverage" section and the Task 9 report's EXPLAIN evidence).
    params = derive(FIXTURE)
    m = system.run("bbox-query", params, repeat=1, selectivity=1.0)
    assert m.result_count == params.total_city_objects


def test_full_read_result_count_is_city_object_granular_not_exploded_by_lod(system):
    # Each CityObject can own several geometry_data rows (one per LoD).
    # full-read's count(*) must stay at the CityObject total (matching
    # cjdb's and duckdb-cityparquet's own full-read count(*)), not the
    # exploded per-geometry row count — the bug this task's design
    # specifically avoids by pre-aggregating geometry_data/property down
    # to one row per feature_id before joining back to `feature`.
    params = derive(FIXTURE)
    m = system.run("full-read", params, repeat=1)
    assert m.result_count == params.total_city_objects


def test_lod_extract_matches_the_known_buildingpart_lod1_solid_count(system):
    # Verified directly against a live import (Task 9 report, and
    # docs/3dcitydb-v5-schema.md's "LoD value format" section): with the
    # CityObject-granularity predicate applied, val_lod='1' AND
    # val_geometry_id IS NOT NULL matches exactly one `lod1Solid` row per
    # BuildingPart — 1116 on this fixture, not the 4929 an unrestricted
    # query would return by also counting each solid's own boundary
    # surfaces' LoD1 geometry.
    params = derive(FIXTURE)
    m = system.run("lod-extract", params, repeat=1)
    assert m.result_count == 1116


def test_semantic_surface_matches_the_cityobject_granular_presence_count(system):
    # From the captured class breakdown (docs/3dcitydb-v5-schema.md):
    # objectclass 712 RoofSurface has 2232 rows in `feature` for this
    # fixture — but that raw feature count is NOT what this scenario
    # reports (a real count-mismatch caught by Task 12's smoke target: an
    # earlier version of this branch counted RoofSurface rows directly and
    # got 2232, while cjdb and duckdb-cityparquet both report 1116 for the
    # "same" scenario). Every BuildingPart genuinely owns two RoofSurface
    # rows (one from `lod1Solid`, one from `lod2Solid`), so the raw count
    # answers "how many RoofSurface features exist", not cjdb's/
    # duckdb-cityparquet's "how many CityObjects have >=1 RoofSurface".
    # Fixed to count DISTINCT owning CityObjects instead — see
    # sql_citydb.py's comment on this branch for the full investigation.
    params = derive(FIXTURE)
    m = system.run("semantic-surface", params, repeat=1)
    assert m.result_count == 1116


def test_index_ddl_is_empty_and_all_expected_default_indexes_are_present(system):
    # index_ddl() returns [] — see its docstring — because 3DCityDB's own
    # import-time indexes already cover every column this benchmark's
    # queries touch. This test pins BOTH halves of that claim against a
    # live, freshly-imported schema: the function itself is empty, and the
    # specific indexes its docstring cites as covering each scenario are
    # actually present in pg_indexes (not just assumed from the schema doc).
    from citybench.scenarios.sql_citydb import index_ddl

    assert index_ddl() == []

    with system._conn.cursor() as cur:
        cur.execute(
            "SELECT indexname FROM pg_indexes WHERE schemaname = %s",
            (system._schema,),
        )
        existing = {row[0] for row in cur.fetchall()}

    for name in (
        "feature_objectid_inx", "feature_objectclass_inx",
        "feature_envelope_spx", "property_name_inx",
        "property_val_geometry_fkx", "property_feature_fkx", "feature_pk",
    ):
        assert name in existing, f"{name} missing from pg_indexes after ingest"


def test_hierarchy_runs_when_the_dataset_has_a_parent_child_pair(system):
    params = derive(FIXTURE)
    if params.parent_id is None:
        pytest.skip("delft fixture has no parent/child pair to exercise hierarchy with")
    m = system.run("hierarchy", params, repeat=1)
    assert m.result_count is not None
    assert m.result_count >= 0


def test_size_reports_a_positive_byte_count(system):
    report = system.size()
    assert report.size_bytes > 0
    assert report.size_bytes_no_index is not None
    assert report.size_bytes >= report.size_bytes_no_index


# --- EXPLAIN-based regression guards --------------------------------------
#
# Mirrors test_cjdb_integration.py's
# test_lod_extract_and_semantic_surface_plans_use_a_bitmap_index_scan: the
# point is that the planner CHOOSES an index-based plan on its own under
# DEFAULT settings, not that it can be coerced into one. A regression that
# dropped an index, let statistics go stale, or reintroduced a
# non-sargable predicate would show up here even though every purely
# textual assertion in test_sql_citydb.py would still pass.


def _explain_text(system, sql: str, args: tuple) -> str:
    with system._conn.cursor() as cur:
        cur.execute(f"EXPLAIN {sql}", args)
        return "\n".join(row[0] for row in cur.fetchall())


def test_id_lookup_uses_the_objectid_index(system):
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    sql, args = sql_citydb.sql_for(
        "id-lookup", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "feature_objectid_inx" in plan
    assert "Seq Scan" not in plan


def test_bbox_query_uses_the_envelope_gist_index(system):
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    sql, args = sql_citydb.sql_for(
        "bbox-query", params, selectivity=0.01, srid=7415,
        cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "feature_envelope_spx" in plan
    assert "Seq Scan on feature" not in plan


def test_count_and_project_and_attr_filter_route_the_predicate_through_an_index(system):
    # The CityObject-granularity predicate's own supporting index
    # (feature_objectclass_inx) must be reachable, not merely present —
    # confirmed as an Index Cond for these three scenarios, all of which
    # apply the resolved static predicate standalone (no JOIN in the same
    # query). C1 fix (final whole-branch review): the OLD correlated
    # predicate genuinely could not reach this index inside a JOIN
    # (attr-stats/lod-extract/hierarchy's child side); the NEW static,
    # pre-resolved `objectclass_id IN (...)` form is sargable everywhere,
    # including inside a JOIN — see the scenarios below and
    # sql_citydb.index_ddl()'s docstring for the full, corrected picture.
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    for scenario, kwargs in (
        ("count", {}), ("project", {}), ("attr-filter", {}),
    ):
        sql, args = sql_citydb.sql_for(
            scenario, params, cityobject_class_ids=system._cityobject_class_ids, **kwargs,
        )
        plan = _explain_text(system, sql, args)
        assert "feature_objectclass_inx" in plan, f"{scenario}: {plan}"
        assert "Seq Scan on feature" not in plan, f"{scenario}: {plan}"


def test_attr_stats_property_side_uses_the_name_index(system):
    # The property-table half of the join (the scenario-distinguishing
    # predicate, pr.name = %s) must be index-driven — always true,
    # independent of the C1 fix below.
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    sql, args = sql_citydb.sql_for(
        "attr-stats", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "property_name_inx" in plan


def test_attr_stats_feature_side_no_longer_seq_scans_after_the_c1_fix(system):
    # C1 fix (final whole-branch review): before this fix, the
    # feature-table half of this join legitimately seq-scanned `feature`
    # for the OLD correlated `(objectclass_id IN (...) OR objectclass_id
    # NOT IN (...))` predicate — measured (coordinator review round 1,
    # Task 9) to be a genuine structural limitation of that predicate
    # shape inside a JOIN, not a cost-based choice avoiding a faster
    # index-driven alternative that existed. The NEW static, pre-resolved
    # id-list predicate IS sargable inside a JOIN — confirmed live against
    # Zurich-scale data (2,192,890 raw `feature` rows) to route through
    # `feature_objectclass_inx` as a genuine `Index Cond`. This test pins
    # the outcome that matters (no seq scan), without pinning to one
    # specific index name, since the planner may legitimately prefer a
    # different, even more selective index depending on the imported
    # dataset's own row-count distribution — see
    # sql_citydb.index_ddl()'s docstring for the full, data-scale-aware
    # picture.
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    sql, args = sql_citydb.sql_for(
        "attr-stats", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "Seq Scan on feature" not in plan, plan


def test_lod_extract_uses_an_index_and_no_longer_seq_scans_feature(system):
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    sql, args = sql_citydb.sql_for(
        "lod-extract", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    # The property-side driving index: either is a legitimate plan choice
    # (see sql_citydb.index_ddl()'s docstring — which one the planner
    # prefers can vary with the imported dataset's own LoD/class
    # distribution), so only presence of ONE of them, not a specific one,
    # is asserted.
    assert "property_val_geometry_fkx" in plan or "property_val_lod_inx" in plan, plan
    # C1 fix: no longer seq-scans `feature` for the granularity predicate
    # either (see test_attr_stats_feature_side_no_longer_seq_scans_after_the_c1_fix's
    # own docstring for the full before/after).
    assert "Seq Scan on feature" not in plan, plan


def test_hierarchy_uses_indexes_for_the_parent_lookup_and_join(system):
    # The `parent` half of the join chain — the lookup by objectid and the
    # property_feature_fkx join — stays fully index-driven regardless of
    # the child-side predicate.
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    if params.parent_id is None:
        pytest.skip("delft fixture has no parent/child pair to exercise hierarchy with")
    sql, args = sql_citydb.sql_for(
        "hierarchy", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "feature_objectid_inx" in plan
    assert "property_feature_fkx" in plan


def test_hierarchy_result_is_unchanged_by_the_child_granularity_predicate(system):
    # Coordinator review round 1, IMPORTANT 2: hierarchy's omission of the
    # CityObject-granularity predicate on `child` was previously safeguarded
    # only by a delft-specific fact recorded in a comment, not enforced in
    # code — a dataset where that fact doesn't hold could have silently
    # overcounted. Fixed by applying the predicate to `child`. This pins
    # that the fix does not change the answer on delft (where it was
    # already a no-op, per the investigation in sql_citydb.py's comment on
    # this branch) — a regression test for the fix itself, distinct from
    # `test_hierarchy_runs_when_the_dataset_has_a_parent_child_pair` above,
    # which only checks the scenario runs at all.
    params = derive(FIXTURE)
    if params.parent_id is None:
        pytest.skip("delft fixture has no parent/child pair to exercise hierarchy with")
    m = system.run("hierarchy", params, repeat=1)
    assert m.result_count == 1


def test_hierarchy_child_side_no_longer_seq_scans_after_the_c1_fix(system):
    # C1 fix (final whole-branch review): an earlier version of this test
    # asserted the OPPOSITE — "Seq Scan on feature child" IS in the plan —
    # as an accepted, understood trade-off of the OLD correlated predicate
    # shape. That predicate shape is gone; the resolved static id-list
    # predicate applied to `child` is sargable, so this now asserts the
    # fix actually landed here too, not just on the standalone scenarios.
    from citybench.scenarios import sql_citydb

    params = derive(FIXTURE)
    if params.parent_id is None:
        pytest.skip("delft fixture has no parent/child pair to exercise hierarchy with")
    sql, args = sql_citydb.sql_for(
        "hierarchy", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "Seq Scan on feature child" not in plan, plan
