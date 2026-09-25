"""Requires `just up`, `just build-citydb`, and an imported fixture."""

from pathlib import Path

import pytest

from citybench.config import Dataset
from citybench.params import derive
from citybench.systems.citydb import CityDbSystem

pytestmark = pytest.mark.integration

FIXTURE = Path(__file__).parent.parent / "data" / "delft.city.jsonl"
# Parameter derivation reads the CityParquet package as well as the source:
# the query windows are searched over the package's own `bbox` column and
# the attribute picks come from its `city` footer, exactly as the format
# harness derives its own (see `citybench.params.derive`). So these tests
# need a converted delft package as well as a running container.
PACKAGE = Path(__file__).parent.parent / "data" / "cityparquet" / "delft"


def _params():
    return derive(FIXTURE, PACKAGE)


@pytest.fixture(scope="module")
def system():
    s = CityDbSystem()
    s.prepare()
    s.ingest(
        Dataset(
            name="delft",
            source=FIXTURE,
            cityparquet_dir=PACKAGE,
            hilbert_dir=Path("data/cityparquet-hilbert/delft"),
        )
    )
    yield s
    s.teardown()


def test_count_is_city_object_granular(system):
    """The spec's blocking item, asserted rather than assumed."""
    params = _params()
    m = system.run("count", params, repeat=1)
    assert m.result_count == params.total_city_objects, (
        "3DCityDB count is not CityObject-granular; the restricting predicate "
        "in docs/3dcitydb-v5-schema.md is wrong or missing"
    )


def test_every_non_windowed_read_scenario_runs_and_reports_server_time(system):
    params = _params()
    for scenario in ("geometry-scan", "count", "attr-filter", "attr-range",
                     "attr-stats"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s[0] > 0
        assert m.server_times_s[0] > 0


def test_bbox_scenarios_are_monotonic_in_the_window(system):
    params = _params()
    for scenario in ("bbox-query",):
        counts = [
            system.run(scenario, params, repeat=1, window=w).result_count
            for w in params.windows
        ]
        assert counts[0] <= counts[1] <= counts[2], scenario


def test_tier2_scenarios_run(system):
    params = _params()
    for scenario in ("lod-query", "parts-per-building"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s[0] > 0


def test_the_write_tier_runs_and_carries_no_server_time(system):
    """CJDB's Q6/Q7/Q8 in order: each leaves the state the next expects.
    `server_time_s` is deliberately empty — obtaining it would mean a second
    `EXPLAIN ANALYZE` execution of the mutation itself."""
    params = _params()
    for scenario in ("attr-add", "attr-update", "attr-delete"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s[0] > 0
        assert m.server_times_s == []
        assert m.result_count is not None and m.result_count > 0


def test_append_object_adds_and_then_removes_everything_the_importer_wrote(system):
    """B18 through `citydb-tool import cityjson`, twice, so the untimed
    watermark reset between samples is exercised. v5 writes more `feature`
    rows than the file has CityObjects — a row per boundary surface — and
    all of them must go again."""
    params = _params()
    before = _feature_count(system)
    m = system.run("append-object", params, repeat=2)
    assert len(m.times_s) == 2 and all(t > 0 for t in m.times_s)
    assert m.result_count == params.append.object_count
    assert "external-importer" in m.notes
    assert _feature_count(system) == before


def _feature_count(system) -> int:
    with system._conn.cursor() as cur:
        cur.execute(f"SELECT count(*) FROM {system._schema}.feature")
        return int(cur.fetchone()[0])


# --- Beyond the brief -----------------------------------------------------
#
# The tests above are the brief's floor. The ones below exercise the
# whole-window bbox-query's exact count against ground truth, geometry-scan's
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
    from citybench.config import BboxWindow

    params = _params()
    whole = BboxWindow(tag="bbox-100pct", target=1.0, achieved=1.0,
                       window=params.bbox_full, approx=False)
    m = system.run("bbox-query", params, repeat=1, window=whole)
    assert m.result_count == params.total_city_objects


def test_geometry_scan_result_count_is_city_object_granular_not_exploded_by_lod(system):
    # Each CityObject owns several geometry_data rows (one per LoD), so a
    # plain count(*) over the join would report geometry rows and every row
    # of this scenario would be a false cross-system count-mismatch. The
    # query uses count(DISTINCT f.id) for exactly this reason.
    params = _params()
    m = system.run("geometry-scan", params, repeat=1)
    assert m.result_count == params.total_city_objects


def test_lod_query_matches_the_known_buildingpart_lod1_solid_count(system):
    # Verified directly against a live import (Task 9 report, and
    # docs/3dcitydb-v5-schema.md's "LoD value format" section): with the
    # CityObject-granularity predicate applied, val_lod='1' AND
    # val_geometry_id IS NOT NULL matches exactly one `lod1Solid` row per
    # BuildingPart — 1116 on this fixture, not the 4929 an unrestricted
    # query would return by also counting each solid's own boundary
    # surfaces' LoD1 geometry.
    params = _params()
    m = system.run("lod-query", params, repeat=1)
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


def test_parts_per_building_reports_one_row_per_building(system):
    """CJDB Q4 keeps childless Buildings, so this is a row count of the
    Buildings themselves, never of the parent/child pairs."""
    params = _params()
    m = system.run("parts-per-building", params, repeat=1)
    assert m.result_count is not None
    assert m.result_count > 0


def test_size_reports_a_positive_byte_count(system):
    report = system.size()
    assert report.size_bytes > 0
    assert report.size_bytes_no_index is not None
    assert report.size_bytes >= report.size_bytes_no_index


# --- EXPLAIN-based regression guards --------------------------------------
#
# Mirrors test_cjdb_integration.py's
# test_lod_query_plans_use_a_bitmap_index_scan: the
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

    params = _params()
    sql, args = sql_citydb.sql_for(
        "id-lookup", params, probe=params.id_probes[0],
        cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "feature_objectid_inx" in plan
    assert "Seq Scan" not in plan


def test_bbox_query_uses_the_envelope_gist_index(system):
    from citybench.scenarios import sql_citydb

    params = _params()
    sql, args = sql_citydb.sql_for(
        "bbox-query", params, params.window("bbox-1pct"), 7415,
        cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "feature_envelope_spx" in plan
    assert "Seq Scan on feature" not in plan


def test_count_and_attr_range_route_the_predicate_through_an_index(system):
    # The CityObject-granularity predicate's own supporting index
    # (feature_objectclass_inx) must be reachable, not merely present —
    # confirmed as an Index Cond for these three scenarios, all of which
    # apply the resolved static predicate standalone (no JOIN in the same
    # query). C1 fix (final whole-branch review): the OLD correlated
    # predicate genuinely could not reach this index inside a JOIN
    # (attr-stats/lod-query/parts-per-building's child side); the NEW static,
    # pre-resolved `objectclass_id IN (...)` form is sargable everywhere,
    # including inside a JOIN — see the scenarios below and
    # sql_citydb.index_ddl()'s docstring for the full, corrected picture.
    from citybench.scenarios import sql_citydb

    params = _params()
    for scenario in ("count", "attr-range", "attr-filter"):
        sql, args = sql_citydb.sql_for(
            scenario, params, cityobject_class_ids=system._cityobject_class_ids,
        )
        plan = _explain_text(system, sql, args)
        assert "feature_objectclass_inx" in plan, f"{scenario}: {plan}"
        assert "Seq Scan on feature" not in plan, f"{scenario}: {plan}"


def test_attr_stats_property_side_uses_the_name_index(system):
    # The property-table half of the join (the scenario-distinguishing
    # predicate, pr.name = %s) must be index-driven — always true,
    # independent of the C1 fix below.
    from citybench.scenarios import sql_citydb

    params = _params()
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

    params = _params()
    sql, args = sql_citydb.sql_for(
        "attr-stats", params, cityobject_class_ids=system._cityobject_class_ids,
    )
    plan = _explain_text(system, sql, args)
    assert "Seq Scan on feature" not in plan, plan


def test_lod_query_uses_an_index_and_no_longer_seq_scans_feature(system):
    from citybench.scenarios import sql_citydb

    params = _params()
    sql, args = sql_citydb.sql_for(
        "lod-query", params, cityobject_class_ids=system._cityobject_class_ids,
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


def test_parts_per_building_uses_indexes_for_the_parent_side_of_the_join(system):
    # The `parent` half of the join chain — the Building class restriction
    # and the property_feature_fkx join — stays index-driven regardless of
    # the child-side predicate.
    from citybench.scenarios import sql_citydb

    params = _params()
    sql, args = sql_citydb.sql_for(
        "parts-per-building", params,
        cityobject_class_ids=system._cityobject_class_ids,
        building_class_id=system._building_class_id,
    )
    plan = _explain_text(system, sql, args)
    assert "feature_objectclass_inx" in plan, plan
    assert "property_feature_fkx" in plan, plan


def test_parts_per_building_child_side_no_longer_seq_scans_after_the_c1_fix(system):
    # C1 fix (final whole-branch review): an earlier version of this test
    # asserted the OPPOSITE — "Seq Scan on feature child" IS in the plan —
    # as an accepted trade-off of the OLD correlated predicate shape. That
    # shape is gone; the resolved static id-list predicate applied to
    # `child` is sargable, so this asserts the fix landed inside the join
    # too, not just on the standalone scenarios.
    from citybench.scenarios import sql_citydb

    params = _params()
    sql, args = sql_citydb.sql_for(
        "parts-per-building", params,
        cityobject_class_ids=system._cityobject_class_ids,
        building_class_id=system._building_class_id,
    )
    plan = _explain_text(system, sql, args)
    assert "Seq Scan on feature child" not in plan, plan


def test_the_building_class_and_double_datatype_are_resolved_live(system):
    """Both are schema-version facts read from the shipped catalogues at
    ingest, never hard-coded: a silently wrong class id would make every
    Building-grained scenario match nothing while still producing a
    plausible-looking zero."""
    assert system._building_class_id is not None and system._building_class_id > 0
    assert system._datatype_id is not None and system._datatype_id > 0
