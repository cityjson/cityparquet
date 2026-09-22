import pytest

from citybench.scenarios import sql_citydb, sql_cjdb, sql_duckdb
from citybench.scenarios.registry import (
    ALL,
    ScenarioUnavailable,
    COUNT_FROM_FIRST_COLUMN,
    COUNT_FROM_ROWCOUNT,
    COUNT_FROM_WRITE_ROWCOUNT,
    ORDERING_SCENARIOS,
    READBENCH_SCENARIOS,
    READ_SCENARIOS,
    SELECTIVITY_SCENARIOS,
    TIER1,
    TIER2,
    TIER3,
    count_mode,
    systems_for,
)
from conftest import make_params


def test_the_scenario_set_is_the_read_tiers_plus_the_write_tier():
    assert ALL == TIER1 + TIER2 + TIER3
    assert READ_SCENARIOS == TIER1 + TIER2
    assert len(ALL) == 16


def test_tier1_carries_the_cjdb_mapped_read_scenarios():
    assert TIER1 == (
        "geometry-scan", "count", "bbox-query", "bbox-fetch", "point-query",
        "attr-filter", "attr-range", "attr-stats", "id-lookup",
    )
    # `full-read` and `project` are gone: the first was three different
    # operations under one name, the second duplicated `attr-filter`'s
    # column read (review §4.2 and §4.1).
    assert "full-read" not in ALL
    assert "project" not in ALL
    assert "hierarchy" not in ALL      # subsumed by parts-per-building


def test_tier2_is_the_semantic_and_hierarchy_scenarios():
    assert TIER2 == (
        "lod-extract", "semantic-surface", "parts-per-building",
        "parts-per-building-join",
    )


def test_tier3_runs_last_and_in_add_update_delete_order():
    """`attr-add` creates the attribute `attr-update` increments and
    `attr-delete` removes, so the order is load-bearing — and the tier is
    last because its mutations leave behind bloat a later read pass would
    measure as if it were the steady state."""
    assert TIER3 == ("attr-add", "attr-update", "attr-delete")
    assert ALL[-3:] == TIER3


def test_both_windowed_scenarios_expand_but_the_point_query_does_not():
    assert SELECTIVITY_SCENARIOS == frozenset({"bbox-query", "bbox-fetch"})
    # `point-query`'s window is a single point: one row, not three.
    assert "point-query" not in SELECTIVITY_SCENARIOS


def test_the_source_order_package_is_published_for_the_ordering_scenarios_only():
    assert ORDERING_SCENARIOS == frozenset(
        {"bbox-query", "bbox-fetch", "point-query"}
    )
    for scenario in ORDERING_SCENARIOS:
        assert "duckdb-cityparquet-source" in systems_for(scenario)
    assert "duckdb-cityparquet-source" not in systems_for("attr-stats")


def test_native_readers_only_run_what_the_rust_child_implements():
    # `geometry-scan`, `bbox-fetch`, `point-query` and `attr-range` have no
    # counterpart in the child's own Scenario enum, and the read harness is
    # not this family's to extend.
    assert READBENCH_SCENARIOS == frozenset(
        {"count", "bbox-query", "attr-filter", "attr-stats", "id-lookup"}
    )
    assert systems_for("count")[:2] == ("cityparquet", "cityparquet-hilbert")
    assert "cityparquet" not in systems_for("geometry-scan")
    assert "cityparquet" not in systems_for("point-query")


def test_tier2_runs_on_sql_systems_only():
    assert systems_for("semantic-surface") == (
        "duckdb-cityparquet", "cjdb", "3dcitydb"
    )


def test_the_join_form_of_parts_per_building_is_a_duckdb_only_control():
    assert systems_for("parts-per-building-join") == ("duckdb-cityparquet",)


def test_the_write_tier_publishes_both_cityparquet_cost_rows():
    """One row for the in-engine mutation and one that also pays the
    package write-back, so a reader sees both costs rather than a blend."""
    for scenario in TIER3:
        tags = systems_for(scenario)
        assert "duckdb-cityparquet" in tags
        assert "duckdb-cityparquet-writeback" in tags
        assert "cityparquet" not in tags


def test_unknown_scenario_raises():
    with pytest.raises(KeyError):
        systems_for("nonsense")


@pytest.mark.parametrize("scenario", sorted(COUNT_FROM_FIRST_COLUMN))
def test_count_mode_is_first_column_for_first_column_scenarios(scenario):
    assert count_mode(scenario) == "first-column"


@pytest.mark.parametrize("scenario", sorted(COUNT_FROM_ROWCOUNT))
def test_count_mode_is_rowcount_for_rowcount_scenarios(scenario):
    assert count_mode(scenario) == "rowcount"


def test_count_mode_unknown_scenario_raises():
    with pytest.raises(KeyError):
        count_mode("nonsense")


@pytest.mark.parametrize("scenario", sorted(COUNT_FROM_WRITE_ROWCOUNT))
def test_count_mode_is_write_rowcount_for_the_write_tier(scenario):
    assert count_mode(scenario) == "write-rowcount"


def test_count_mode_sets_partition_all_scenarios_exactly():
    # Every scenario in ALL must be in exactly one of the three sets: none
    # missing (count_mode would raise for a real scenario), none in two
    # (extract_count would be told contradictory things about the same
    # scenario). This is the invariant a newly added scenario could break
    # silently if only added to ALL and forgotten here.
    sets = (COUNT_FROM_FIRST_COLUMN, COUNT_FROM_ROWCOUNT,
            set(COUNT_FROM_WRITE_ROWCOUNT))
    assert set().union(*sets) == set(ALL)
    assert sum(len(s) for s in sets) == len(ALL)


TABLE = "read_parquet('/data/example/building.parquet')"
CLASS_IDS = (100, 901, 902)
BUILDING_ID = 901
COLUMNS = {
    "id": "VARCHAR", "object_type": "VARCHAR", "parents": "VARCHAR[]",
    "children": "VARCHAR[]", "bbox": "STRUCT(xmin DOUBLE, ...)",
    "geometry_lod0_0": "GEOMETRY('EPSG:7415')", "geometry_lod1_2": "BLOB",
    "geometry_properties_lod2_2": "STRUCT(surfaces VARCHAR)",
    "b3_dak_type": "VARCHAR", "b3_h_dak_max": "DOUBLE", "h_dak_max": "DOUBLE",
}


def _build(scenario, system, params, window):
    """The `(sql, args)` one system would send for one scenario."""
    if system == "cjdb":
        return sql_cjdb.sql_for(scenario, params, window, srid=7415)
    if system == "3dcitydb":
        return sql_citydb.sql_for(
            scenario, params, window, 7415,
            cityobject_class_ids=CLASS_IDS, building_class_id=BUILDING_ID,
        )
    return sql_duckdb.sql_for(scenario, params, TABLE, window, columns=COLUMNS)


def _placeholders(sql: str, system: str) -> int:
    return sql.count("%s") if system in {"cjdb", "3dcitydb"} else sql.count("?")


@pytest.mark.parametrize("scenario", READ_SCENARIOS)
def test_every_read_scenario_builds_on_every_system_that_runs_it(scenario):
    """The cross-product sweep: for every read scenario, every system the
    registry says answers it must build real SQL whose placeholder count
    matches the arguments bound to it.

    A mismatch here is the failure mode a per-branch test misses — the SQL
    itself looks right, and only psycopg or DuckDB, at run time against a
    live database, reports that it was given the wrong number of
    parameters. That would surface as an `error:` row in a multi-hour run.
    """
    params = make_params()
    for system in systems_for(scenario):
        if system.startswith("cityparquet"):
            continue          # the native child takes argv, not SQL
        windows = (
            params.windows if scenario in SELECTIVITY_SCENARIOS else (None,)
        )
        for window in windows:
            sql, args = _build(scenario, system, params, window)
            assert sql.strip(), f"{system}/{scenario} built empty SQL"
            assert _placeholders(sql, system) == len(args), (
                f"{system}/{scenario} binds {len(args)} args into "
                f"{_placeholders(sql, system)} placeholders: {sql}"
            )


@pytest.mark.parametrize("scenario", TIER3)
def test_every_write_scenario_builds_on_every_system_that_runs_it(scenario):
    for system in systems_for(scenario):
        if system == "cjdb":
            sql, args = sql_cjdb.write_sql(scenario)
            assert sql.count("%s") == len(args)
            for reset_sql, reset_args in sql_cjdb.write_reset_sql(scenario):
                assert reset_sql.count("%s") == len(reset_args)
        elif system == "3dcitydb":
            sql, args = sql_citydb.write_sql(scenario, BUILDING_ID, 7)
            assert sql.count("%s") == len(args)
            for reset_sql, reset_args in sql_citydb.write_reset_sql(
                scenario, BUILDING_ID, 7
            ):
                assert reset_sql.count("%s") == len(reset_args)
        else:
            statements = sql_duckdb.write_statements(scenario, "pkg", "ST_Area(g)")
            assert statements
            for sql, args in statements:
                assert sql.count("?") == len(args)


def test_an_attribute_less_dataset_skips_rather_than_errors_on_every_system():
    """A dataset carrying neither an `attr-filter` predicate nor a numeric
    attribute is a legitimate input (Montreal, lod3_railway). Every system
    must raise `ScenarioUnavailable` — recorded as `skipped:` — not an
    arbitrary exception recorded as `error:`."""
    params = make_params(attr_filter=None, attr_range=None, numeric_column=None)
    for scenario in ("attr-filter", "attr-range", "attr-stats"):
        for system in systems_for(scenario):
            if system.startswith("cityparquet"):
                continue
            with pytest.raises(ScenarioUnavailable):
                _build(scenario, system, params, None)


def test_scenarios_returning_rows_use_rowcount_not_a_first_column():
    """Every scenario that returns ids — as CJDB's own queries do — is
    counted by the rows it materialised, so no engine can win by returning
    a lazy cursor or by answering a count from metadata."""
    for scenario in ("bbox-fetch", "point-query", "attr-filter", "attr-range",
                     "id-lookup", "lod-extract", "parts-per-building",
                     "parts-per-building-join"):
        assert count_mode(scenario) == "rowcount", scenario
    for scenario in ("count", "geometry-scan", "bbox-query", "attr-stats",
                     "semantic-surface"):
        assert count_mode(scenario) == "first-column", scenario
