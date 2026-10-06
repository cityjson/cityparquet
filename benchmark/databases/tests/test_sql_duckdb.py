"""Pure-function tests for the DuckDB SQL builder.

``sql_for`` never touches a database, so every scenario branch is testable
without a running engine or an on-disk CityParquet package.
"""

import pytest

from citybench.config import Params
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_duckdb import (
    BUILDING_PART_TYPE, BUILDING_TYPE, highest_lod_geometry, sql_for,
    write_reset_statements, write_statements,
)
from conftest import ge_attr_filter, make_params, make_probes

TABLE = "read_parquet('/data/example/building.parquet')"

#: A delft-shaped package schema: a native `GEOMETRY` footprint and three
#: `BLOB` solids, which is what a real package returns (measured).
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


def test_attr_stats_references_the_column_bare_not_under_an_attributes_struct():
    # Attribute columns are flattened at the object table's top level —
    # `attributes."h_dak_max"` does not resolve (a real Binder Error
    # caught by running this SQL against an actual converted package,
    # since `attributes` is not a struct/table alias in this schema).
    sql, _ = sql_for("attr-stats", _params(), TABLE)
    assert '"h_dak_max"' in sql
    assert "attributes." not in sql


def test_geometry_scan_returns_every_id_and_every_geometry_column_as_wkb():
    """Every object's id and geometry, in DuckDB's native binary: no byte
    lengths summed, no text, no attribute column."""
    sql, args = sql_for("geometry-scan", _params(), TABLE, columns=COLUMNS)
    assert sql.startswith("SELECT id, ")
    for column in ("geometry_lod0_0", "geometry_lod1_2", "geometry_lod1_3", "geometry_lod2_2"):
        assert column in sql
    for absent in ("octet_length", "sum(", "count(", "hash(", "b3_dak_type", "geometry_properties"):
        assert absent not in sql
    assert args == ()


def test_highest_lod_geometry_prefers_the_most_detailed_column():
    expr = highest_lod_geometry(COLUMNS)
    assert expr == ("coalesce(geometry_lod2_2, geometry_lod1_3, geometry_lod1_2, "
                    "ST_AsWKB(geometry_lod0_0))")


def test_bbox_query_returns_ids_and_the_highest_lod_geometry():
    sql, args = sql_for("bbox-query", _params(), TABLE, _window(_params()), columns=COLUMNS)
    assert sql.startswith("SELECT id, coalesce(geometry_lod2_2,")
    assert "count(" not in sql
    assert "bbox.xmax >= ?" in sql


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








