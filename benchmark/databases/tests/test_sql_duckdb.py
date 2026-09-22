"""Pure-function tests for the DuckDB SQL builder.

``sql_for`` never touches a database, so every scenario branch is testable
without a running engine or an on-disk CityParquet package.
"""

import pytest

from citybench.config import Params
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_duckdb import (
    BUILDING_PART_TYPE, BUILDING_TYPE, geometry_byte_length, sql_for,
    write_reset_statements, write_statements,
)
from conftest import ge_attr_filter, make_params

TABLE = "read_parquet('/data/example/building.parquet')"

#: A delft-shaped package schema: a native `GEOMETRY` footprint and three
#: `BLOB` solids, which is what a real package returns (measured — see
#: `geometry_byte_length`).
COLUMNS = {
    "id": "VARCHAR", "object_type": "VARCHAR", "parents": "VARCHAR[]",
    "children": "VARCHAR[]", "bbox": "STRUCT(xmin DOUBLE, ...)",
    "geometry_lod0_0": "GEOMETRY('EPSG:7415')",
    "geometry_lod1_2": "BLOB", "geometry_lod1_3": "BLOB",
    "geometry_lod2_2": "BLOB",
    "geometry_properties_lod0_0": "STRUCT(surfaces VARCHAR)",
    "geometry_properties_lod1_2": "STRUCT(surfaces VARCHAR)",
    "geometry_properties_lod1_3": "STRUCT(surfaces VARCHAR)",
    "geometry_properties_lod2_2": "STRUCT(surfaces VARCHAR)",
    "b3_dak_type": "VARCHAR", "b3_h_dak_max": "DOUBLE", "h_dak_max": "DOUBLE",
}


def _params(**overrides) -> Params:
    return make_params(**overrides)


def _window(params: Params, tag: str = "bbox-25pct"):
    return params.window(tag)


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # A dataset with no numeric attribute at all (Montreal, lod3_railway —
    # see params.py) is a legitimate dataset property, not a query bug.
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        sql_for("attr-stats", _params(numeric_column=None), TABLE)


def test_attr_range_raises_scenario_unavailable_without_a_numeric_attribute():
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        sql_for("attr-range", _params(attr_range=None), TABLE)


def test_attr_filter_raises_scenario_unavailable_without_a_predicate():
    with pytest.raises(ScenarioUnavailable, match="attr-filter predicate"):
        sql_for("attr-filter", _params(attr_filter=None), TABLE)


def test_parts_per_building_keeps_childless_buildings():
    """CJDB Q4 reports one row per Building INCLUDING childless ones, so
    the natural form must coalesce a NULL `children` array to 0 rather than
    return NULL where the join form returns 0."""
    sql, args = sql_for("parts-per-building", _params(), TABLE)
    assert "coalesce(len(children), 0)" in sql
    assert args == (BUILDING_TYPE,)
    assert "HAVING" not in sql.upper()


def test_parts_per_building_join_form_left_joins_so_no_building_is_dropped():
    sql, args = sql_for("parts-per-building-join", _params(), TABLE)
    assert "LEFT JOIN" in sql
    assert "unnest(parents)" in sql
    assert "GROUP BY b.id" in sql
    assert args == (BUILDING_PART_TYPE, BUILDING_TYPE)


def test_count_from_first_column_scenarios_select_count_as_first_column():
    # The registry's cross-system count comparison depends on this shape:
    # geometry-scan and attr-stats must not put the aggregate first.
    sql, _ = sql_for("geometry-scan", _params(), TABLE, columns=COLUMNS)
    assert sql.strip().upper().startswith("SELECT COUNT(*)")

    sql, _ = sql_for("attr-stats", _params(), TABLE)
    assert sql.strip().upper().startswith("SELECT COUNT(")


def test_attr_stats_references_the_column_bare_not_under_an_attributes_struct():
    # Attribute columns are flattened at the object table's top level —
    # `attributes."h_dak_max"` does not resolve (a real Binder Error
    # caught by running this SQL against an actual converted package,
    # since `attributes` is not a struct/table alias in this schema).
    sql, _ = sql_for("attr-stats", _params(), TABLE)
    assert '"h_dak_max"' in sql
    assert "attributes." not in sql


def test_geometry_scan_sums_every_geometry_column_and_no_attribute_column():
    """The replacement for `full-read`, which hashed all 84 columns on this
    side while cjdb serialised three and 3DCityDB cast whole records through
    two CTEs. All three now scan the same thing: the geometry."""
    sql, args = sql_for("geometry-scan", _params(), TABLE, columns=COLUMNS)
    for column in ("geometry_lod0_0", "geometry_lod1_2", "geometry_lod1_3",
                   "geometry_lod2_2"):
        assert column in sql
    assert "hash(" not in sql
    assert "b3_dak_type" not in sql
    assert "::HUGEINT" in sql
    assert args == ()


def test_geometry_scan_re_encodes_only_the_native_geometry_column():
    """`octet_length` does not bind against DuckDB's native `GEOMETRY`, which
    is what CityParquet's LoD0 footprint decodes to whatever
    `enable_geoparquet_conversion` says. That one term is therefore a
    re-serialisation, and the rest are stored lengths — README Caveat 18."""
    assert geometry_byte_length("geometry_lod1_2", "BLOB") == (
        "coalesce(octet_length(geometry_lod1_2), 0)"
    )
    assert geometry_byte_length("geometry_lod0_0", "GEOMETRY('EPSG:7415')") == (
        "coalesce(octet_length(ST_AsWKB(geometry_lod0_0)), 0)"
    )
    sql, _ = sql_for("geometry-scan", _params(), TABLE, columns=COLUMNS)
    assert sql.count("ST_AsWKB(") == 1


def test_bbox_query_binds_the_resolved_window():
    params = _params()
    window = _window(params)
    sql, args = sql_for("bbox-query", params, TABLE, window)
    w = window.window
    assert args == (w.minx, w.maxx, w.miny, w.maxy)
    assert "bbox.xmax" in sql
    assert sql.strip().upper().startswith("SELECT COUNT(*)")


def test_bbox_query_without_a_window_is_a_programming_error_not_a_default():
    with pytest.raises(ValueError, match="resolved BboxWindow"):
        sql_for("bbox-query", _params(), TABLE)


def test_bbox_fetch_returns_id_and_footprint_for_buildings_only():
    params = _params()
    window = _window(params)
    sql, args = sql_for("bbox-fetch", params, TABLE, window, columns=COLUMNS)
    w = window.window
    assert sql.strip().startswith("SELECT id, geometry_lod0_0")
    assert args == (BUILDING_TYPE, w.minx, w.maxx, w.miny, w.maxy)
    assert "count(" not in sql


def test_bbox_fetch_degrades_to_a_null_footprint_without_a_lod0_column():
    params = _params()
    sql, _ = sql_for("bbox-fetch", params, TABLE, _window(params),
                     columns={"id": "VARCHAR", "object_type": "VARCHAR"})
    assert "SELECT id, NULL" in sql
    assert "geometry_lod0_0" not in sql


def test_point_query_is_a_degenerate_window_at_the_median_centre():
    params = _params()
    sql, args = sql_for("point-query", params, TABLE, columns=COLUMNS)
    x, y = params.point_xy
    assert args == (BUILDING_TYPE, x, x, y, y)
    assert "bbox.xmin <= ?" in sql and "bbox.xmax >= ?" in sql


def test_count_counts_every_row_unconditionally():
    sql, args = sql_for("count", _params(), TABLE)
    assert "count(*)" in sql
    assert "WHERE" not in sql.upper()  # unfiltered: no predicate to parameterise
    assert args == ()


def test_attr_filter_uses_the_derived_attribute_and_returns_ids():
    sql, args = sql_for("attr-filter", _params(), TABLE)
    assert sql.strip().startswith("SELECT id")
    assert '"b3_dak_type" = ?' in sql
    assert args == ("slanted",)   # bound, not interpolated
    assert "slanted" not in sql
    assert "object_type" not in sql   # never the structural column again


def test_attr_filter_supports_the_numeric_lower_bound_form():
    sql, args = sql_for(
        "attr-filter", _params(attr_filter=ge_attr_filter()), TABLE
    )
    assert '"TerrainHeight" >= ?' in sql
    assert args == (2.45,)


def test_attr_range_is_a_strict_inequality_on_the_derived_threshold():
    sql, args = sql_for("attr-range", _params(), TABLE)
    assert sql.strip().startswith("SELECT id")
    assert '"b3_h_dak_max" > ?' in sql
    assert args == (20.0,)


def test_id_lookup_parameterises_the_target_id_rather_than_interpolating_it():
    sql, args = sql_for("id-lookup", _params(), TABLE)
    assert "id = ?" in sql
    assert args == ("obj-1",)
    assert "obj-1" not in sql  # must travel as a bound parameter, not literal text


def test_lod_extract_returns_ids_from_the_lod1_geometry_column_not_lod2():
    sql, args = sql_for("lod-extract", _params(), TABLE)
    assert sql.strip().startswith("SELECT id")
    assert "geometry_lod1_2 IS NOT NULL" in sql
    # `count(col)` was answerable from the definition levels alone, which
    # measured almost nothing; CJDB's Q5 returns ids.
    assert "count(" not in sql
    assert "geometry_lod2" not in sql
    assert args == ()


def test_write_statements_follow_cjdbs_add_update_delete_order():
    add = write_statements("attr-add", "pkg", "ST_Area(geometry_lod0_0)")
    assert "ADD COLUMN footprint_area DOUBLE" in add[0][0]
    assert "SET footprint_area = ST_Area(geometry_lod0_0)" in add[1][0]

    update = write_statements("attr-update", "pkg", "ignored")
    assert "footprint_area = footprint_area + 10.0" in update[0][0]

    delete = write_statements("attr-delete", "pkg", "ignored")
    assert "DROP COLUMN footprint_area" in delete[0][0]


def test_write_resets_make_every_sample_measure_the_same_work():
    """`ADD COLUMN` errors the second time and `DROP COLUMN` has nothing
    left to drop, so `repeat` > 1 needs an untimed reset — not a warm-up."""
    assert "DROP COLUMN IF EXISTS" in write_reset_statements(
        "attr-add", "pkg", "x")[0][0]
    assert write_reset_statements("attr-update", "pkg", "x") == []
    reset = write_reset_statements("attr-delete", "pkg", "x")
    assert "ADD COLUMN IF NOT EXISTS" in reset[0][0]


def test_unknown_write_scenario_raises_key_error():
    with pytest.raises(KeyError):
        write_statements("nonsense", "pkg", "x")


def test_semantic_surface_filters_on_the_lod2_surface_type_list():
    # `surfaces` is a JSON-encoded VARCHAR, not a nested LIST<STRUCT> —
    # `.type` cannot be dot-accessed on it directly (a real binder error
    # caught by running this SQL against an actual converted package).
    # `json_extract_string(..., '$[*].type')` is the fix.
    sql, args = sql_for("semantic-surface", _params(), TABLE)
    assert "json_extract_string(geometry_properties_lod2_2.surfaces" in sql
    assert "'$[*].type'" in sql
    assert "RoofSurface" in sql
    assert "list_contains" in sql
    assert args == ()


def test_semantic_surface_checks_every_lod_column_not_lod2_2_alone():
    # A real cross-system comparability defect (review-caught, not caught
    # by delft's own count-check — see sql_duckdb.py's module comment on
    # this branch and tests/test_semantic_surface_lod_scope.py for the
    # full story): an earlier version of this query was scoped to
    # geometry_properties_lod2_2 ALONE, silently asking a narrower
    # question than cjdb's and sql_citydb.py's own (any-LoD)
    # semantic-surface queries. Pinned here at the text level so a
    # regression back to a single LoD column is caught immediately,
    # without needing a live database — test_semantic_surface_lod_scope.py
    # additionally proves this matters with real divergent data.
    sql, _ = sql_for("semantic-surface", _params(), TABLE)
    for col in ("geometry_properties_lod0_0", "geometry_properties_lod1_2",
                "geometry_properties_lod1_3", "geometry_properties_lod2_2"):
        assert col in sql, f"{col} missing from semantic-surface SQL: {sql}"
    # Four independent list_contains(...) checks OR'd together, not one.
    assert sql.count("list_contains(") == 4
    assert sql.count(" OR ") == 3


def test_unknown_scenario_raises_key_error():
    with pytest.raises(KeyError):
        sql_for("nonsense", _params(), TABLE)


# --- Schema-aware LoD columns (Task 14 fix) -----------------------------
#
# delft's LoD tiers (0.0/1.2/1.3/2.2) are not universal — Montreal's real
# converted package carries only geometry_lod0_0/geometry_lod2_0.
# Referencing a hardcoded, delft-shaped column name against that package
# raised a DuckDB BinderException outright, discovered running Task 14's
# heterogeneity corpus. These tests pin the fix: when the caller supplies
# the package's real column set, both scenarios degrade to a real,
# zero-result query instead of erroring.


def test_lod_extract_uses_the_real_column_when_present_in_columns():
    sql, args = sql_for("lod-extract", _params(), TABLE,
                         columns={"geometry_lod1_2": "BLOB", "id": "VARCHAR"})
    assert "geometry_lod1_2" in sql
    assert args == ()


def test_lod_extract_returns_a_real_zero_query_when_lod1_2_column_is_absent():
    # Montreal-shaped: only geometry_lod0_0/geometry_lod2_0 exist.
    sql, args = sql_for(
        "lod-extract", _params(), TABLE,
        columns={"geometry_lod0_0": "GEOMETRY", "geometry_lod2_0": "BLOB", "id": "VARCHAR"},
    )
    # Still the scenario's own row shape (ids), just an empty one.
    assert sql.strip().startswith("SELECT id")
    assert "WHERE FALSE" in sql.upper()
    assert "geometry_lod1_2" not in sql
    assert args == ()


def test_lod_extract_without_columns_keeps_the_old_unconditional_query():
    # columns=None (the default) must reproduce the exact pre-fix SQL, so
    # every existing caller/test that never passes it is unaffected.
    sql, args = sql_for("lod-extract", _params(), TABLE)
    assert "geometry_lod1_2" in sql
    assert "WHERE FALSE" not in sql.upper()
    assert args == ()


def test_semantic_surface_ors_across_whatever_lod_properties_columns_exist():
    # Montreal-shaped: only lod0_0/lod2_0 — neither is in delft's
    # hardcoded four-column list, so the old code would have referenced
    # zero real columns' worth of the WRONG names.
    sql, _ = sql_for(
        "semantic-surface", _params(), TABLE,
        columns={
            "geometry_properties_lod0_0": "STRUCT(surfaces VARCHAR)",
            "geometry_properties_lod2_0": "STRUCT(surfaces VARCHAR)",
            "id": "VARCHAR",
        },
    )
    assert "geometry_properties_lod0_0" in sql
    assert "geometry_properties_lod2_0" in sql
    assert "geometry_properties_lod1_2" not in sql
    assert sql.count("list_contains(") == 2
    assert sql.count(" OR ") == 1


def test_semantic_surface_falls_back_to_select_false_when_no_lod_properties_column_exists():
    sql, args = sql_for("semantic-surface", _params(), TABLE,
                         columns={"id": "VARCHAR", "object_type": "VARCHAR"})
    assert sql.strip().upper().startswith("SELECT COUNT(*)")
    assert "WHERE FALSE" in sql.upper()
    assert args == ()


def test_semantic_surface_without_columns_keeps_the_old_hardcoded_four():
    # columns=None (the default) must reproduce the exact pre-fix SQL.
    sql, _ = sql_for("semantic-surface", _params(), TABLE)
    for col in ("geometry_properties_lod0_0", "geometry_properties_lod1_2",
                "geometry_properties_lod1_3", "geometry_properties_lod2_2"):
        assert col in sql
