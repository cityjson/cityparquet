"""Pure-function tests for the 3DCityDB v5 SQL builder.

``sql_for`` and ``index_ddl`` never touch a database, so every scenario
branch is testable without a running engine or an imported schema. The
brief's own tests are a floor, not a ceiling — see the sibling
``test_sql_cjdb.py`` for the same principle applied to cjdb; the sections
below follow the same shape, adapted to 3DCityDB v5's EAV schema.

C1 fix (final whole-branch review): ``sql_for`` now takes a required
``cityobject_class_ids`` keyword argument — a pre-resolved, static tuple of
qualifying ``objectclass_id`` values — instead of building the old
correlated-subquery predicate itself on every call. Every test below that
exercises the CityObject-granularity predicate now asserts the STATIC
``objectclass_id IN (...)`` shape, not "is_toplevel"/"NOT IN"/"WITH
RECURSIVE" text, because that dynamic text no longer appears anywhere in
``sql_for``'s own output — it lives only in ``_cityobject_predicate``/
``resolve_cityobject_class_ids``, the one-time resolution path (see
``test_citydb_integration.py`` for the live proof that the resolved ids and
the dynamic predicate agree on real data).
"""

from __future__ import annotations

import pytest

from citybench.config import BBox, Params
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_citydb import (
    CAPTURED_LOD_TARGET,
    _cityobject_predicate,
    _static_predicate,
    append_reset_sql,
    append_watermark_sql,
    index_ddl,
    sql_for,
    write_reset_sql,
    write_sql,
)
from conftest import ge_attr_filter, make_params, make_probes

PARAMS = make_params(
    numeric_column="b3_h_dak_50p",
    id_probes=make_probes("NL.IMBAG.Pand.0503100000000010"),
    total_city_objects=2231,
)

# A small, deliberately-unsorted stand-in for the real, live-resolved set
# (89 ids on the current schema — see resolve_cityobject_class_ids's own
# docstring). What matters for these pure-function tests is the SHAPE
# sql_for renders around whatever ids it is given, not the real values —
# the real values are proven correct only against a live import, in
# test_citydb_integration.py. Deliberately out of order and including a
# multi-digit and a single-digit id so sort-order and formatting are both
# exercised.
IDS = (901, 100, 902)


#: A stand-in for the live-resolved `objectclass_id` of `Building`, which a
#: real run reads once per ingest via `resolve_class_id`.
BUILDING_ID = 901


def _params(**overrides) -> Params:
    defaults = dict(
        numeric_column="b3_h_dak_50p",
        id_probes=make_probes("NL.IMBAG.Pand.0503100000000010"),
        total_city_objects=2231,
    )
    defaults.update(overrides)
    return make_params(**defaults)


def _window(params: Params, tag: str = "bbox-25pct"):
    return params.window(tag)


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # Mirrors the guards the other two SQL modules apply: a
    # dataset with no numeric attribute at all (Montreal, lod3_railway —
    # see params.py's derive()) is a legitimate dataset property, not a
    # query bug.
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        sql_for("attr-stats", _params(numeric_column=None), cityobject_class_ids=IDS)


# --- cityobject_class_ids is required -------------------------------------


def test_sql_for_requires_cityobject_class_ids_keyword():
    # No default: a silent fallback (e.g. an empty tuple) would risk
    # quietly reintroducing the exact fairness defect the C1 fix corrects.
    # Every caller must resolve the id set once and pass it explicitly.
    with pytest.raises(TypeError):
        sql_for("count", PARAMS)  # type: ignore[call-arg]


def test_sql_for_rejects_an_empty_cityobject_class_ids():
    with pytest.raises(ValueError, match="cityobject_class_ids is empty"):
        sql_for("count", PARAMS, cityobject_class_ids=())


# --- The brief's own floor -------------------------------------------------


def test_count_targets_feature_and_uses_a_static_resolved_id_list():
    sql, args = sql_for("count", PARAMS, cityobject_class_ids=IDS)
    assert "citydb.feature" in sql
    assert "objectclass_id IN (100, 901, 902)" in sql  # sorted, not input order
    assert args == ()


def test_bbox_query_uses_postgis_operator_for_index_use():
    sql, args = sql_for("bbox-query", PARAMS, _window(PARAMS),
                        cityobject_class_ids=IDS)
    assert "&&" in sql
    assert "envelope" in sql
    assert len(args) == 5  # four ordinates plus the SRID


def test_attr_filter_joins_property_on_name_and_val_string():
    sql, args = sql_for("attr-filter", PARAMS, cityobject_class_ids=IDS)
    assert "citydb.property" in sql
    assert "pr.name = %s" in sql
    assert "pr.val_string = %s" in sql
    assert sql.strip().startswith("SELECT f.objectid")
    assert args == ("b3_dak_type", "slanted")
    # The retired form resolved a CLASSNAME through `objectclass`, because
    # `attr-filter` used to filter on `object_type`. It now filters on a
    # real CityJSON attribute, like the other two systems.
    assert "classname" not in sql


def test_attr_filter_numeric_bound_form_coalesces_the_typed_value_columns():
    sql, args = sql_for("attr-filter", _params(attr_filter=ge_attr_filter()),
                        cityobject_class_ids=IDS)
    assert "coalesce(pr.val_double, pr.val_int) >= %s" in sql
    assert args == ("TerrainHeight", 2.45)


def test_attr_range_is_cjdb_q1_over_the_eav_property_table():
    sql, args = sql_for("attr-range", PARAMS, cityobject_class_ids=IDS)
    assert "coalesce(pr.val_double, pr.val_int) > %s" in sql
    assert sql.strip().startswith("SELECT f.objectid")
    assert args == ("b3_h_dak_max", 20.0)


def test_parts_per_building_without_a_resolved_building_class_is_a_loud_failure():
    # A silently wrong class id would make every Building-grained scenario
    # match nothing while still producing a plausible-looking zero.
    with pytest.raises(ValueError, match="Building objectclass_id"):
        sql_for("parts-per-building", PARAMS, cityobject_class_ids=IDS)


def test_attr_stats_joins_property_to_feature():
    sql, args = sql_for("attr-stats", PARAMS, cityobject_class_ids=IDS)
    assert "citydb.property" in sql
    assert "val_double" in sql
    assert args == ("b3_h_dak_50p",)


def test_attr_stats_coalesces_val_double_and_val_int():
    # Zurich's "Geomtype" is a real numeric attribute (params.derive() only
    # checks isinstance(value, (int, float))) but is JSON-integer-valued,
    # landing entirely in property.val_int with val_double NULL on every
    # row — verified live against a real import (see sql_citydb.py's own
    # attr-stats docstring). A val_double-only query matches zero rows for
    # such an attribute; coalescing across both columns is what a
    # competent query does regardless of which numeric JSON subtype the
    # attribute happens to be.
    sql, _ = sql_for("attr-stats", PARAMS, cityobject_class_ids=IDS)
    assert "coalesce(pr.val_double, pr.val_int)" in sql


def test_parts_per_building_left_joins_property_via_val_feature_id():
    sql, args = sql_for("parts-per-building", PARAMS,
                        cityobject_class_ids=IDS, building_class_id=BUILDING_ID)
    assert "val_feature_id" in sql
    assert sql.count("LEFT JOIN") == 2      # childless Buildings are kept
    assert "count(child.id)" in sql         # NOT count(*), which reports 1
    assert "GROUP BY parent.objectid" in sql
    assert args == (BUILDING_ID,)


def test_unknown_scenario_raises():
    with pytest.raises(KeyError):
        sql_for("nonsense", PARAMS, cityobject_class_ids=IDS)


# --- Beyond the brief --------------------------------------------------
#
# Every scenario branch in sql_for, plus index_ddl, gets its own assertion,
# per the task's explicit instruction that the brief's test file is a floor:
# a database is never involved in this file, so an untested branch here is
# purely a missed opportunity to catch a wrong column name before it ever
# reaches a live container.


# --- _static_predicate: the resolved, sargable form (C1 fix) --------------


def test_static_predicate_renders_a_plain_in_list():
    assert _static_predicate((901, 902)) == "objectclass_id IN (901, 902)"


def test_static_predicate_sorts_ids_for_a_deterministic_query_shape():
    assert _static_predicate((902, 100, 901)) == "objectclass_id IN (100, 901, 902)"


def test_static_predicate_qualifies_the_column_with_an_alias():
    assert _static_predicate((901,), "child") == "child.objectclass_id IN (901)"


def test_static_predicate_rejects_an_empty_id_tuple():
    # An empty IN (...) is silently always-false in PostgreSQL — every
    # scenario would match zero rows with no error at all. Raising here
    # turns a silent wrong-answer into a loud failure at query-build time.
    with pytest.raises(ValueError, match="cityobject_class_ids is empty"):
        _static_predicate(())


def test_static_predicate_never_emits_a_correlated_subquery():
    # The C1 fix's single most important requirement, pinned directly on
    # the primitive that renders it: the resolved form must be a flat
    # literal list, never the OR-of-two-subqueries shape
    # _cityobject_predicate builds.
    sql = _static_predicate(IDS)
    assert "SELECT" not in sql
    assert "WITH RECURSIVE" not in sql
    assert "is_toplevel" not in sql


# --- C1 fix: sql_for uses the static id list, not a correlated subquery ---
#
# The task's explicit requirement: pin that the generated SQL uses a
# static id list, not a correlated subquery, for every scenario that
# applies the CityObject-granularity predicate.


@pytest.mark.parametrize("scenario, args", [
    ("count", ()),
    ("geometry-scan", ()),
    ("bbox-query", (PARAMS.window("bbox-25pct"),)),
    ("attr-filter", ()),
    ("attr-range", ()),
    ("attr-stats", ()),
    ("lod-query", ()),
])
def test_scenario_uses_a_static_resolved_id_list_not_a_correlated_subquery(
    scenario, args,
):
    sql, _ = sql_for(scenario, PARAMS, *args, cityobject_class_ids=IDS)
    assert "objectclass_id IN (100, 901, 902)" in sql
    # None of the OLD, correlated-subquery predicate's own vocabulary may
    # appear anywhere in a scenario query any more — that shape is what
    # made every one of these scenarios full-scan `feature` (C1, the final
    # whole-branch review's critical finding).
    assert "is_toplevel" not in sql
    assert "WITH RECURSIVE" not in sql
    assert "NOT IN" not in sql


def test_parts_per_building_uses_a_static_resolved_id_list_qualified_to_child():
    sql, args = sql_for("parts-per-building", _params(),
                        cityobject_class_ids=IDS, building_class_id=BUILDING_ID)
    assert "child.objectclass_id IN (100, 901, 902)" in sql
    assert "is_toplevel" not in sql
    assert "WITH RECURSIVE" not in sql
    assert args == (BUILDING_ID,)


def test_id_lookup_does_not_reference_the_granularity_predicate_at_all():
    # A known CityObject id is already CityObject-granular by construction
    # (see sql_citydb.py's comment on this branch) — no predicate needed,
    # static or otherwise.
    sql, _ = sql_for("id-lookup", PARAMS, cityobject_class_ids=IDS,
                     probe=PARAMS.id_probes[0])
    assert "objectclass_id IN" not in sql
    assert "is_toplevel" not in sql


def test_geometry_scan_selects_a_cityobject_grain_count_first():
    """count_mode("geometry-scan") == "first-column", and the count must be
    CityObject-grain. One CityObject owns several `geometry_data` rows (one
    per LoD), so a plain `count(*)` over this join would report geometry
    rows and every row of the scenario would be a false count-mismatch."""
    sql, args = sql_for("geometry-scan", _params(), cityobject_class_ids=IDS)
    assert sql.strip().upper().startswith("SELECT COUNT(DISTINCT F.ID)")
    assert args == ()


def test_geometry_scan_touches_the_geometry_and_not_the_property_records():
    """The retired `full-read` cast every `property` row's whole COMPOSITE
    RECORD to text through two GROUP BY CTEs over the entire tables — ~20
    NULL fields per row, and 276 s against DuckDB's 1.2 s. That was largely
    artefact, not architecture."""
    sql, _ = sql_for("geometry-scan", _params(), cityobject_class_ids=IDS)
    assert "length(gd.geometry::text)" in sql
    assert "citydb.property" not in sql
    assert "WITH " not in sql.upper()          # no CTEs at all
    assert "envelope::text" not in sql


def test_bbox_query_parameterises_the_window_and_srid_in_order():
    params = _params()
    window = _window(params)
    sql, args = sql_for(
        "bbox-query", params, window, 7415, cityobject_class_ids=IDS,
    )
    w = window.window
    assert args == (w.minx, w.miny, w.maxx, w.maxy, 7415)


def test_bbox_query_defaults_to_zero_srid_when_none_is_given():
    params = _params()
    _, args = sql_for("bbox-query", params, _window(params),
                      cityobject_class_ids=IDS)
    assert args[-1] == 0


def test_attr_filter_parameterises_rather_than_interpolating_the_value():
    sql, args = sql_for("attr-filter", _params(), cityobject_class_ids=IDS)
    assert args == ("b3_dak_type", "slanted")
    assert "'slanted'" not in sql  # would be a SQL-injection-shaped bug if it were


def test_attr_stats_selects_count_first_per_registry_convention():
    sql, _ = sql_for("attr-stats", _params(), cityobject_class_ids=IDS)
    assert sql.strip().upper().startswith(
        "SELECT COUNT(COALESCE(PR.VAL_DOUBLE, PR.VAL_INT))"
    )


def test_id_lookup_asks_for_the_probe_it_was_handed_on_objectid():
    """One call per probe — 10/50/90 % of the canonical stream order plus a
    verified-absent id — against `objectid`, the CityObject identifier,
    never the internal integer `id`."""
    params = _params()
    for probe in params.id_probes:
        sql, args = sql_for("id-lookup", params, cityobject_class_ids=IDS,
                            probe=probe)
        assert "objectid" in sql
        assert args == (probe.id,)
        assert probe.id not in sql   # bound parameter, not a literal


def test_id_lookup_without_a_probe_is_a_loud_failure():
    with pytest.raises(ValueError):
        sql_for("id-lookup", _params(), cityobject_class_ids=IDS)


def test_write_tier_inserts_updates_and_deletes_a_property_row():
    """v5 has no `cityobject_genericattrib` table, so CJDB's Q6 becomes an
    INSERT into the EAV `property` table. `datatype_id` is NOT NULL and is
    resolved live; `namespace_id` is nullable and left unset."""
    add, add_args = write_sql("attr-add", BUILDING_ID, 7)
    assert "INSERT INTO citydb.property" in add
    assert "(feature_id, name, datatype_id, val_double)" in add
    # The CJDB paper's own Q6 computes ENVELOPE area on 3DCityDB against
    # FOOTPRINT area on cjdb; the asymmetry is inherited and disclosed.
    assert "ST_Area(envelope)" in add
    assert "namespace_id" not in add
    assert add_args == (7, BUILDING_ID)

    update, _ = write_sql("attr-update", BUILDING_ID, 7)
    assert "SET val_double = val_double + 10" in update

    delete, _ = write_sql("attr-delete", BUILDING_ID, 7)
    assert delete.startswith("DELETE FROM citydb.property")


def test_write_resets_undo_the_non_idempotent_insert():
    """`attr-add`'s INSERT is not idempotent: without the reset a second
    sample would leave two `footprint_area` rows per Building and make
    `attr-update` touch twice as many rows."""
    reset = write_reset_sql("attr-add", BUILDING_ID, 7)
    assert reset[0][0].startswith("DELETE FROM citydb.property")
    assert write_reset_sql("attr-update", BUILDING_ID, 7) == []
    assert write_reset_sql("attr-delete", BUILDING_ID, 7) == [
        write_sql("attr-add", BUILDING_ID, 7)
    ]


def test_lod_query_targets_the_truncated_integer_lod_not_the_cityjson_notation():
    # v5's importer truncates "1.2"/"1.3" to "1" (docs/3dcitydb-v5-schema.md,
    # "LoD value format") — querying the literal CityJSON tag "1.2" here
    # would silently match zero rows. The tier-collapsing is disclosed in
    # the README, not corrected: 3DCityDB's "LoD 1" covers 1.2 AND 1.3.
    sql, args = sql_for("lod-query", _params(), cityobject_class_ids=IDS)
    assert CAPTURED_LOD_TARGET == "1"
    assert args == ("1",)
    assert "1.2" not in sql


def test_lod_query_filters_on_val_lod_and_val_geometry_id():
    sql, _ = sql_for("lod-query", _params(), cityobject_class_ids=IDS)
    assert "val_lod" in sql
    assert "val_geometry_id IS NOT NULL" in sql


def test_lod_query_does_not_reach_for_a_geometry_data_lod_column():
    # geometry_data has no `lod` column at all (Task 5, re-confirmed here).
    sql, _ = sql_for("lod-query", _params(), cityobject_class_ids=IDS)
    assert "gd.lod" not in sql
    assert "geometry_data.lod" not in sql


def test_lod_query_returns_the_whole_feature_row_and_its_lod1_geometry():
    """The other two systems hand back the object WITH its geometry; a
    `feature` row alone would be a different amount of object. The
    attributes still are not joined — that asymmetry is README Caveat
    16."""
    sql, _ = sql_for("lod-query", _params(), cityobject_class_ids=IDS)
    assert "f.*" in sql
    assert "gd.geometry" in sql
    assert "citydb.geometry_data gd ON gd.id = pr.val_geometry_id" in sql


def test_lod_query_stays_one_row_per_cityobject():
    """`DISTINCT ON (f.id)` is this query's `SELECT DISTINCT f.*`: several
    matching `property` rows of one feature must not become several rows,
    or the count would not be comparable with the other two systems'. It
    is scoped to the key rather than to the whole row so PostgreSQL never
    compares WKB geometries for equality."""
    sql, _ = sql_for("lod-query", _params(), cityobject_class_ids=IDS)
    assert sql.strip().startswith("SELECT DISTINCT ON (f.id)")
    assert sql.rstrip().endswith("ORDER BY f.id")


def test_the_append_reset_empties_the_tables_the_importer_writes_to():
    """`append-object`'s untimed reset. A watermark, not a predicate on
    `objectid`: citydb-tool also writes a `feature` row per boundary
    surface, whose `objectid` it invents, so no id-shaped predicate can
    name them."""
    marks = {table: 7 for table, _ in append_watermark_sql()}
    statements = append_reset_sql(marks)
    tables = [sql for sql, _ in statements]
    # property references both feature and geometry_data; geometry_data
    # references feature. Deleting in any other order fails the FK.
    assert "citydb.property" in tables[0]
    assert "citydb.geometry_data" in tables[1]
    assert "citydb.feature" in tables[2]
    assert all(args == (7,) for _, args in statements)
    assert all(sql.count("%s") == 1 for sql, _ in statements)


def test_parts_per_building_joins_parent_via_the_property_fk_and_child_via_val_feature_id():
    sql, args = sql_for("parts-per-building", _params(),
                        cityobject_class_ids=IDS, building_class_id=BUILDING_ID)
    assert "citydb.feature child" in sql
    assert "citydb.feature parent" in sql
    assert "child.id = pr.val_feature_id" in sql
    assert "pr.feature_id = parent.id" in sql
    assert args == (BUILDING_ID,)


def test_parts_per_building_qualifies_the_granularity_predicate_to_child_only():
    # The predicate must be qualified to `child` specifically — `parent`
    # also has an unqualified `objectclass_id` in scope, so a bare
    # predicate here would be ambiguous SQL, not merely imprecise. It also
    # replaces any dependency on which `name` a given importer gives the
    # parent -> child association: boundary surfaces are excluded by CLASS.
    sql, args = sql_for("parts-per-building", _params(),
                        cityobject_class_ids=IDS, building_class_id=BUILDING_ID)
    assert "child.objectclass_id IN (100, 901, 902)" in sql
    # `parent` is restricted by the Building class alone, as CJDB's Q4 is.
    assert "parent.objectclass_id IN" not in sql
    assert "parent.objectclass_id = %s" in sql
    assert args == (BUILDING_ID,)


# --- _cityobject_predicate: kept as the resolution-time primitive ---------
#
# This function no longer appears in any scenario's own SQL (that is
# exactly the C1 fix), but it still exists and is still exercised directly:
# resolve_cityobject_class_ids() calls it (with column="id") to enumerate
# the qualifying objectclass ids from the objectclass catalogue itself, and
# CAPTURED_CITYOBJECT_PREDICATE (its column="objectclass_id" default) is
# still the recorded, canonical statement of the underlying logic these
# tests pin.


def test_cityobject_predicate_unqualified_matches_the_module_constant():
    from citybench.scenarios.sql_citydb import CAPTURED_CITYOBJECT_PREDICATE

    assert _cityobject_predicate() == CAPTURED_CITYOBJECT_PREDICATE
    assert "objectclass_id" in _cityobject_predicate()
    assert "." not in _cityobject_predicate().split("IN (SELECT")[0]


def test_cityobject_predicate_qualified_prefixes_every_column_reference():
    # Both occurrences of objectclass_id in the predicate (the IN branch
    # and the NOT IN branch) must be alias-qualified — qualifying only one
    # would leave the other ambiguous in a query with two `feature` aliases
    # in scope, exactly the bug this fix addresses.
    predicate = _cityobject_predicate("child")
    assert predicate.count("child.objectclass_id") == 2
    assert "(objectclass_id " not in predicate  # no unqualified occurrence slipped through


def test_cityobject_predicate_column_override_targets_id_not_objectclass_id():
    # resolve_cityobject_class_ids()'s own mechanism: the SAME logic,
    # evaluated against objectclass.id (the class catalogue's own primary
    # key) rather than feature.objectclass_id, to enumerate which CLASSES
    # qualify rather than which FEATURE ROWS do.
    predicate = _cityobject_predicate(column="id")
    assert "objectclass_id" not in predicate
    assert predicate.count(" id ") >= 1 or "(id IN" in predicate
    assert "(id IN (SELECT id FROM citydb.objectclass WHERE is_toplevel = 1)" in predicate


def test_cityobject_predicate_column_override_can_be_alias_qualified_too():
    # Alias deliberately not "oc" — the recursive CTE's own internal query
    # always aliases `objectclass` as `oc` regardless of this function's
    # `alias` argument, so "oc" would coincidentally inflate the count.
    predicate = _cityobject_predicate("cls", column="id")
    assert predicate.count("cls.id") == 2


def test_index_ddl_returns_an_empty_list():
    # 3DCityDB's own import-time indexes (verified: `citydb index create`
    # is a no-op against a freshly-imported schema — pg_indexes count is
    # unchanged, 59 before and after) already cover every column this
    # benchmark's queries touch — see index_ddl's own docstring and
    # docs/3dcitydb-v5-schema.md's "Index coverage" section for the
    # per-scenario mapping and EXPLAIN evidence. Adding a same-shape index
    # under a new name would be a genuinely redundant index object,
    # inflating size_bytes for zero query benefit — the same class of
    # mistake Task 8 found and fixed for cjdb. Still true after the C1 fix:
    # the static id list routes through the SAME pre-existing
    # feature_objectclass_inx index, so no new index is required either.
    assert index_ddl() == []


def test_index_ddl_takes_no_arguments():
    ddl = index_ddl()
    assert isinstance(ddl, list)
    assert all(isinstance(stmt, str) for stmt in ddl)
