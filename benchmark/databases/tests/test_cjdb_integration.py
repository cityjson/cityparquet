"""Requires `just up` and an imported fixture. Run with `-m integration`."""

from pathlib import Path

import pytest

from citybench.config import Dataset
from citybench.params import derive
from citybench.systems.cjdb import CjdbSystem

pytestmark = pytest.mark.integration

FIXTURE = Path(__file__).parent.parent / "data" / "delft.city.jsonl"


@pytest.fixture(scope="module")
def system():
    s = CjdbSystem()
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


def test_count_matches_source_city_object_total(system):
    params = derive(FIXTURE)
    m = system.run("count", params, repeat=1)
    assert m.result_count == params.total_city_objects


def test_every_tier1_scenario_runs(system):
    params = derive(FIXTURE)
    for scenario in ("full-read", "count", "attr-filter", "attr-stats",
                     "id-lookup", "project"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s and m.times_s[0] > 0
        assert m.server_times_s and m.server_times_s[0] > 0


def test_bbox_query_selectivity_increases_with_window(system):
    params = derive(FIXTURE)
    counts = [
        system.run("bbox-query", params, repeat=1, selectivity=s).result_count
        for s in (0.01, 0.05, 0.25)
    ]
    assert counts[0] <= counts[1] <= counts[2]


# --- Beyond the brief -----------------------------------------------------
#
# The three tests above are the brief's floor. The ones below exercise
# tier-2 scenarios, the index set actually landing in the schema, and the
# size report — all real, all only checkable against a live container.


def test_index_ddl_actually_creates_every_declared_index(system):
    from citybench.scenarios.sql_cjdb import index_ddl

    with system._conn.cursor() as cur:
        cur.execute(
            "SELECT indexname FROM pg_indexes WHERE schemaname = %s",
            (system._schema,),
        )
        existing = {row[0] for row in cur.fetchall()}

    for ddl in index_ddl():
        # Each statement is "CREATE INDEX IF NOT EXISTS <name> ON ...".
        name = ddl.split()[5]
        assert name in existing, f"{name} missing from pg_indexes after ingest"


def test_hierarchy_runs_when_the_dataset_has_a_parent_child_pair(system):
    params = derive(FIXTURE)
    if params.parent_id is None:
        pytest.skip("delft fixture has no parent/child pair to exercise hierarchy with")
    m = system.run("hierarchy", params, repeat=1)
    assert m.result_count is not None
    assert m.result_count >= 0


def test_lod_extract_and_semantic_surface_run_without_error(system):
    params = derive(FIXTURE)
    for scenario in ("lod-extract", "semantic-surface"):
        m = system.run(scenario, params, repeat=1)
        assert m.result_count is not None
        assert m.result_count >= 0


def test_size_reports_a_positive_byte_count_including_the_new_indexes(system):
    report = system.size()
    assert report.size_bytes > 0
    assert report.size_bytes_no_index is not None
    # The index set this task adds must be reflected: total size (indexes
    # included) must be at least the table-only size, never less.
    assert report.size_bytes >= report.size_bytes_no_index


def test_full_read_result_count_is_unchanged_by_forcing_more_columns(system):
    # Widening full-read's checksum to cover attributes and ground_geometry
    # as well as geometry must not change WHAT is counted — only how much
    # is decoded to produce the checksum. A different count here would mean
    # the widened predicate silently dropped or duplicated rows, which
    # would be a new correctness bug, not a fairness fix.
    params = derive(FIXTURE)
    m = system.run("full-read", params, repeat=1)
    assert m.result_count == params.total_city_objects


def test_lod_extract_and_semantic_surface_plans_use_a_bitmap_index_scan(system):
    # The point of the @? rewrite (see the fix report) is that the planner
    # CHOOSES an index-based plan on its own — not that it can be coerced
    # into one with enable_seqscan=off. Default planner settings only, so a
    # regression that keeps the @? syntax but loses the index (a dropped
    # index, stale statistics, a schema change) would show up here as a
    # missing "Bitmap Index Scan" and fail this test, even though every
    # purely textual assertion in test_sql_cjdb.py would still pass.
    from citybench.scenarios import sql_cjdb

    params = derive(FIXTURE)
    for scenario in ("lod-extract", "semantic-surface"):
        sql, args = sql_cjdb.sql_for(scenario, params)
        with system._conn.cursor() as cur:
            cur.execute(f"EXPLAIN {sql}", args)
            plan = "\n".join(row[0] for row in cur.fetchall())
        assert "Bitmap Index Scan" in plan, (
            f"{scenario}: expected a Bitmap Index Scan, got:\n{plan}"
        )
        assert "Seq Scan" not in plan, (
            f"{scenario}: fell back to a Seq Scan, got:\n{plan}"
        )
