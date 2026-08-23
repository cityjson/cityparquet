"""Pure-function tests for the DuckDB SQL builder.

``sql_for`` never touches a database, so every scenario branch is testable
without a running engine or an on-disk CityParquet package.
"""

import pytest

from citybench.config import BBox, Params
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_duckdb import sql_for

TABLE = "read_parquet('/data/example/building.parquet')"


def _params(*, parent_id: str | None = "parent-1",
            numeric_column: str | None = "h_dak_max") -> Params:
    return Params(
        bbox_full=BBox(minx=0, miny=0, minz=0, maxx=100, maxy=100, maxz=10),
        attr_column="object_type",
        attr_eq="Building",
        numeric_column=numeric_column,
        target_id="obj-1",
        parent_id=parent_id,
        total_city_objects=10,
    )


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # Mirrors hierarchy's own parent_id-is-None guard: a dataset with no
    # numeric attribute at all (Montreal, lod3_railway — see params.py's
    # derive()) is a legitimate dataset property, not a query bug.
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        sql_for("attr-stats", _params(numeric_column=None), TABLE)


def test_hierarchy_raises_scenario_unavailable_when_dataset_has_no_parent_id():
    with pytest.raises(ScenarioUnavailable):
        sql_for("hierarchy", _params(parent_id=None), TABLE)


def test_hierarchy_scenario_unavailable_message_explains_the_dataset_gap():
    # Guards against a regression where the None slips through into the
    # query args instead of being intercepted before SQL is built, and
    # pins the message so a caller catching this can surface it verbatim.
    with pytest.raises(ScenarioUnavailable, match="dataset has no parent/child hierarchy"):
        sql_for("hierarchy", _params(parent_id=None), TABLE)


def test_hierarchy_builds_sql_with_parent_id_when_present():
    # `parents` is a VARCHAR[] on each row (its own parent ids), not a
    # scalar `parent_id` column — a real column-name mismatch caught by
    # running this SQL against an actual converted package (see
    # sql_duckdb.py's module docstring).
    sql, args = sql_for("hierarchy", _params(parent_id="parent-1"), TABLE)
    assert args == ("parent-1",)
    assert "list_contains(parents, ?)" in sql


def test_count_from_first_column_scenarios_select_count_as_first_column():
    # The registry's cross-system count comparison depends on this shape:
    # full-read and attr-stats must not put the checksum/aggregate first.
    sql, _ = sql_for("full-read", _params(), TABLE)
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


def test_full_read_casts_the_per_column_checksum_to_hugeint_not_bigint():
    # sum(hash(...))::BIGINT genuinely overflows on delft's real data (a
    # Conversion Error caught by running this SQL against an actual
    # converted package) — DuckDB's SUM accumulates hashes in 128 bits
    # internally, and BIGINT is too narrow to hold the result.
    sql, _ = sql_for("full-read", _params(), TABLE)
    assert "::HUGEINT" in sql
    assert "::BIGINT" not in sql


def test_bbox_query_uses_the_selectivity_window():
    sql, args = sql_for("bbox-query", _params(), TABLE, selectivity=0.25)
    assert args == (0.0, 50.0, 0.0, 50.0)
    assert "bbox.xmax" in sql


def test_count_counts_every_row_unconditionally():
    sql, args = sql_for("count", _params(), TABLE)
    assert "count(*)" in sql
    assert "WHERE" not in sql.upper()  # unfiltered: no predicate to parameterise
    assert args == ()


def test_attr_filter_parameterises_the_categorical_value_on_object_type():
    sql, args = sql_for("attr-filter", _params(), TABLE)
    assert "object_type" in sql
    assert args == ("Building",)  # attr_eq from _params(), not interpolated into sql
    assert "Building" not in sql  # would be a SQL-injection-shaped bug if it were


def test_id_lookup_parameterises_the_target_id_rather_than_interpolating_it():
    sql, args = sql_for("id-lookup", _params(), TABLE)
    assert "id = ?" in sql
    assert args == ("obj-1",)
    assert "obj-1" not in sql  # must travel as a bound parameter, not literal text


def test_project_counts_the_attribute_column_rather_than_star():
    sql, args = sql_for("project", _params(), TABLE)
    assert "count(object_type)" in sql
    assert "count(*)" not in sql
    assert args == ()


def test_lod_extract_names_the_lod1_geometry_column_not_lod2():
    sql, args = sql_for("lod-extract", _params(), TABLE)
    assert "geometry_lod1_2" in sql
    assert "geometry_lod2" not in sql  # the whole point is LoD2 bytes are never read
    assert args == ()


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
                         columns=frozenset({"geometry_lod1_2", "id"}))
    assert "geometry_lod1_2" in sql
    assert args == ()


def test_lod_extract_returns_a_real_zero_query_when_lod1_2_column_is_absent():
    # Montreal-shaped: only geometry_lod0_0/geometry_lod2_0 exist.
    sql, args = sql_for(
        "lod-extract", _params(), TABLE,
        columns=frozenset({"geometry_lod0_0", "geometry_lod2_0", "id"}),
    )
    assert sql.strip().upper().startswith("SELECT COUNT(*)")
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
        columns=frozenset({
            "geometry_properties_lod0_0", "geometry_properties_lod2_0", "id",
        }),
    )
    assert "geometry_properties_lod0_0" in sql
    assert "geometry_properties_lod2_0" in sql
    assert "geometry_properties_lod1_2" not in sql
    assert sql.count("list_contains(") == 2
    assert sql.count(" OR ") == 1


def test_semantic_surface_falls_back_to_select_false_when_no_lod_properties_column_exists():
    sql, args = sql_for("semantic-surface", _params(), TABLE,
                         columns=frozenset({"id", "object_type"}))
    assert sql.strip().upper().startswith("SELECT COUNT(*)")
    assert "WHERE FALSE" in sql.upper()
    assert args == ()


def test_semantic_surface_without_columns_keeps_the_old_hardcoded_four():
    # columns=None (the default) must reproduce the exact pre-fix SQL.
    sql, _ = sql_for("semantic-surface", _params(), TABLE)
    for col in ("geometry_properties_lod0_0", "geometry_properties_lod1_2",
                "geometry_properties_lod1_3", "geometry_properties_lod2_2"):
        assert col in sql
