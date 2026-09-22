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
from conftest import ge_attr_filter, make_params, make_probes

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


def test_id_lookup_asks_for_the_probe_it_was_handed_and_binds_it():
    """One call per probe, and the id travels as a bound parameter rather
    than as literal text — the four probes differ only in that value, so
    interpolating it would also mean four different query plans."""
    params = _params()
    for probe in params.id_probes:
        sql, args = sql_for("id-lookup", params, TABLE, probe=probe)
        assert "id = ?" in sql
        assert args == (probe.id,)
        assert probe.id not in sql


def test_id_lookup_returns_the_whole_object_row():
    sql, _ = sql_for("id-lookup", _params(), TABLE,
                     probe=_params().id_probes[0])
    assert sql.strip().startswith("SELECT *")


def test_id_lookup_without_a_probe_is_a_loud_failure():
    """The runner always supplies one. A silent default would publish four
    identical rows under four different probe tags."""
    with pytest.raises(ValueError):
        sql_for("id-lookup", _params(), TABLE)


def test_lod_query_returns_whole_rows_from_the_lod1_geometry_column():
    """Catalogue B12: "retrieve all buildings having a specific LoD
    geometry" — the buildings, not their ids. A projection of ids alone is
    answerable from one column's definition levels and measures almost
    nothing."""
    sql, args = sql_for("lod-query", _params(), TABLE)
    assert sql.strip().startswith("SELECT *")
    assert "geometry_lod1_2 IS NOT NULL" in sql
    assert "count(" not in sql
    assert "geometry_lod2" not in sql
    assert args == ()


def test_append_object_calls_the_extensions_own_importer():
    """Catalogue B18 through `insert_cityjsonseq`, not a hand-written
    INSERT: the row is meant to include the derived-state maintenance an
    importer does (`feature_id`, the reciprocal hierarchy, `bbox`)."""
    append = _params().append
    statements = write_statements("append-object", "pkg", "ignored", append)
    assert len(statements) == 1
    sql, args = statements[0]
    assert sql.startswith("PRAGMA insert_cityjsonseq('pkg', ")
    assert append.path in sql
    assert args == ()


def test_append_object_without_an_append_file_is_skipped_not_errored():
    """A plain CityJSON source has no feature to cut out. That is a dataset
    property — a `skipped:` row — not a failure."""
    with pytest.raises(ScenarioUnavailable):
        write_statements("append-object", "pkg", "ignored", None)


def test_append_object_reset_deletes_exactly_the_appended_ids():
    """Untimed, between samples: the importer refuses an id it already
    holds, so without this a second sample would fail rather than measure.
    Every id in the appended file carries the suffix, so the predicate
    names them and nothing else."""
    reset = write_reset_statements(
        "append-object", "pkg", "ignored", _params().append
    )
    assert len(reset) == 1
    sql, args = reset[0]
    assert sql.startswith("PRAGMA cityparquet_delete('pkg', ")
    assert "id LIKE ''%-appended''" in sql
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
# the package's real column set, `lod-query` degrades to a real,
# zero-result query instead of erroring.


def test_lod_query_uses_the_real_column_when_present_in_columns():
    sql, args = sql_for("lod-query", _params(), TABLE,
                        columns={"geometry_lod1_2": "BLOB", "id": "VARCHAR"})
    assert "geometry_lod1_2" in sql
    assert args == ()


def test_lod_query_returns_a_real_zero_query_when_lod1_2_column_is_absent():
    # Montreal-shaped: only geometry_lod0_0/geometry_lod2_0 exist.
    sql, args = sql_for(
        "lod-query", _params(), TABLE,
        columns={"geometry_lod0_0": "GEOMETRY", "geometry_lod2_0": "BLOB",
                 "id": "VARCHAR"},
    )
    # Still the scenario's own row shape (whole rows), just an empty set.
    assert sql.strip().startswith("SELECT *")
    assert "WHERE FALSE" in sql.upper()
    assert "geometry_lod1_2" not in sql
    assert args == ()


def test_lod_query_without_columns_keeps_the_unconditional_query():
    sql, args = sql_for("lod-query", _params(), TABLE)
    assert "geometry_lod1_2" in sql
    assert "WHERE FALSE" not in sql.upper()
    assert args == ()