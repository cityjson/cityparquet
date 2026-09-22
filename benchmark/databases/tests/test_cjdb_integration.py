"""Requires `just up` and an imported fixture. Run with `-m integration`."""

from pathlib import Path

import pytest

from citybench.config import Dataset
from citybench.params import derive
from citybench.systems.cjdb import CjdbSystem

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
    s = CjdbSystem()
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


def test_count_matches_source_city_object_total(system):
    params = _params()
    m = system.run("count", params, repeat=1)
    assert m.result_count == params.total_city_objects


def test_every_non_windowed_read_scenario_runs(system):
    params = _params()
    for scenario in ("geometry-scan", "count", "attr-filter", "attr-range",
                     "attr-stats", "id-lookup", "point-query"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s and m.times_s[0] > 0
        assert m.server_times_s and m.server_times_s[0] > 0


def test_bbox_scenarios_increase_with_the_window(system):
    params = _params()
    for scenario in ("bbox-query", "bbox-fetch"):
        counts = [
            system.run(scenario, params, repeat=1, window=w).result_count
            for w in params.windows
        ]
        assert counts[0] <= counts[1] <= counts[2], scenario


def test_the_write_tier_runs_cjdbs_own_queries_and_carries_no_server_time(system):
    """Q6/Q7/Q8 in order: each leaves the state the next expects.
    `server_time_s` is deliberately empty — obtaining it would mean a second
    `EXPLAIN ANALYZE` execution of the mutation itself, which would apply
    Q6 twice and increment Q7 by 20 rather than 10."""
    params = _params()
    for scenario in ("attr-add", "attr-update", "attr-delete"):
        m = system.run(scenario, params, repeat=1)
        assert m.times_s[0] > 0
        assert m.server_times_s == []
        assert m.result_count is not None and m.result_count > 0


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


def test_parts_per_building_reports_one_row_per_building(system):
    """CJDB Q4 keeps childless Buildings, so this is a row count of the
    Buildings themselves, never of the parent/child pairs."""
    params = _params()
    m = system.run("parts-per-building", params, repeat=1)
    assert m.result_count is not None
    assert m.result_count > 0


def test_lod_extract_and_semantic_surface_run_without_error(system):
    params = _params()
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


def test_geometry_scan_counts_every_city_object(system):
    # `geometry-scan` narrows what is DECODED (the geometry column alone,
    # not also attributes and ground_geometry) but must not change WHAT is
    # counted. A different count here would mean the narrowed query
    # silently dropped rows — `coalesce` on the length keeps a NULL
    # geometry's row in the count.
    params = _params()
    m = system.run("geometry-scan", params, repeat=1)
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

    params = _params()
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
