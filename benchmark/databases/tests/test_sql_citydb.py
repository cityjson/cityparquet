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
    index_ddl,
    sql_for,
)

PARAMS = Params(
    bbox_full=BBox(0.0, 0.0, 0.0, 100.0, 100.0, 10.0),
    attr_column="object_type",
    attr_eq="BuildingPart",
    numeric_column="b3_h_dak_50p",
    target_id="NL.IMBAG.Pand.0503100000000010",
    parent_id="NL.IMBAG.Pand.0503100000000010",
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


def _params(*, parent_id: str | None = "NL.IMBAG.Pand.0503100000000010",
            numeric_column: str | None = "b3_h_dak_50p") -> Params:
    return Params(
        bbox_full=BBox(minx=0.0, miny=0.0, minz=0.0, maxx=100.0, maxy=100.0, maxz=10.0),
        attr_column="object_type",
        attr_eq="BuildingPart",
        numeric_column=numeric_column,
        target_id="NL.IMBAG.Pand.0503100000000010",
        parent_id=parent_id,
        total_city_objects=2231,
    )


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # Mirrors this module's own hierarchy parent_id-is-None guard: a
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
    sql, args = sql_for("bbox-query", PARAMS, selectivity=0.25, cityobject_class_ids=IDS)
    assert "&&" in sql
    assert "envelope" in sql
    assert len(args) == 5  # four ordinates plus the SRID


def test_attr_filter_resolves_classname_via_objectclass():
    sql, args = sql_for("attr-filter", PARAMS, cityobject_class_ids=IDS)
    assert "objectclass" in sql
    assert "classname" in sql
    assert args == ("BuildingPart",)


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


def test_hierarchy_joins_property_via_val_feature_id():
    sql, args = sql_for("hierarchy", PARAMS, cityobject_class_ids=IDS)
    assert "val_feature_id" in sql
    assert args == ("NL.IMBAG.Pand.0503100000000010",)


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


@pytest.mark.parametrize("scenario, kwargs", [
    ("count", {}),
    ("full-read", {}),
    ("bbox-query", {"selectivity": 0.25}),
    ("attr-filter", {}),
    ("attr-stats", {}),
    ("project", {}),
    ("lod-extract", {}),
])
def test_scenario_uses_a_static_resolved_id_list_not_a_correlated_subquery(
    scenario, kwargs,
):
    sql, _ = sql_for(scenario, PARAMS, cityobject_class_ids=IDS, **kwargs)
    assert "objectclass_id IN (100, 901, 902)" in sql
    # None of the OLD, correlated-subquery predicate's own vocabulary may
    # appear anywhere in a scenario query any more — that shape is what
    # made every one of these scenarios full-scan `feature` (C1, the final
    # whole-branch review's critical finding).
    assert "is_toplevel" not in sql
    assert "WITH RECURSIVE" not in sql
    assert "NOT IN" not in sql


def test_semantic_surface_uses_a_static_resolved_id_list_qualified_to_owner():
    sql, args = sql_for("semantic-surface", PARAMS, cityobject_class_ids=IDS)
    assert "owner.objectclass_id IN (100, 901, 902)" in sql
    assert "is_toplevel" not in sql
    assert "WITH RECURSIVE" not in sql
    assert args == ("RoofSurface",)


def test_hierarchy_uses_a_static_resolved_id_list_qualified_to_child():
    sql, args = sql_for("hierarchy", _params(), cityobject_class_ids=IDS)
    assert "child.objectclass_id IN (100, 901, 902)" in sql
    assert "is_toplevel" not in sql
    assert "WITH RECURSIVE" not in sql
    assert args == ("NL.IMBAG.Pand.0503100000000010",)


def test_id_lookup_does_not_reference_the_granularity_predicate_at_all():
    # A known CityObject id is already CityObject-granular by construction
    # (see sql_citydb.py's comment on this branch) — no predicate needed,
    # static or otherwise.
    sql, _ = sql_for("id-lookup", PARAMS, cityobject_class_ids=IDS)
    assert "objectclass_id IN" not in sql
    assert "is_toplevel" not in sql


def test_full_read_selects_count_first_then_a_forcing_checksum():
    # count_mode("full-read") == "first-column": the registry's
    # cross-system comparison depends on count(*) being the FIRST selected
    # column. This query has a leading WITH clause (geom_len/prop_len), so
    # count(*) is not the first TOKEN of the string — instead, assert it is
    # the outer query's SELECT (paren-depth 0, i.e. not inside one of the
    # WITH clause's own subqueries).
    sql, args = sql_for("full-read", _params(), cityobject_class_ids=IDS)
    idx = sql.upper().index("SELECT COUNT(*)")
    depth = sql[:idx].count("(") - sql[:idx].count(")")
    assert depth == 0, f"count(*) is nested {depth} parens deep, not the outer SELECT"
    # And it must be the FIRST such occurrence at depth 0 — i.e. nothing
    # else is selected before it in the outer query.
    assert "SELECT count(*)" in sql
    assert args == ()


def test_full_read_forces_geometry_and_property_and_envelope():
    # Parity with sql_cjdb's full-read (geometry + attributes +
    # ground_geometry) and sql_duckdb's (hash of every column): 3DCityDB's
    # normalised schema splits "attributes" across many property rows and
    # "geometry" across many geometry_data rows per feature, so both must
    # be aggregated and forced, not just one.
    sql, _ = sql_for("full-read", _params(), cityobject_class_ids=IDS)
    assert "geometry_data" in sql
    assert "citydb.property" in sql
    assert "envelope" in sql
    assert sql.count("::text") >= 3  # envelope, geometry_data row, property row


def test_full_read_does_not_explode_the_count_by_joining_geometry_directly():
    # A naive `JOIN geometry_data` (one CityObject can own several
    # geometry_data rows, one per LoD) would inflate count(*) past the
    # CityObject total and mismatch cjdb's/duckdb's own full-read count —
    # exactly the bug this task's design avoids via pre-aggregating
    # geometry_data down to one row per feature_id first.
    sql, _ = sql_for("full-read", _params(), cityobject_class_ids=IDS)
    assert "GROUP BY feature_id" in sql
    assert "JOIN citydb.geometry_data" not in sql  # only inside the geom_len CTE
    assert "LEFT JOIN geom_len" in sql


def test_bbox_query_parameterises_the_window_and_srid_in_order():
    sql, args = sql_for(
        "bbox-query", _params(), selectivity=0.25, srid=7415, cityobject_class_ids=IDS,
    )
    win = _params().bbox_full.window(0.25)
    assert args == (win.minx, win.miny, win.maxx, win.maxy, 7415)


def test_bbox_query_defaults_to_zero_srid_when_none_is_given():
    _, args = sql_for("bbox-query", _params(), selectivity=0.25, cityobject_class_ids=IDS)
    assert args[-1] == 0


def test_attr_filter_parameterises_rather_than_interpolating_the_value():
    sql, args = sql_for("attr-filter", _params(), cityobject_class_ids=IDS)
    assert args == ("BuildingPart",)
    assert "'BuildingPart'" not in sql  # would be a SQL-injection-shaped bug if it were


def test_attr_stats_selects_count_first_per_registry_convention():
    sql, _ = sql_for("attr-stats", _params(), cityobject_class_ids=IDS)
    assert sql.strip().upper().startswith(
        "SELECT COUNT(COALESCE(PR.VAL_DOUBLE, PR.VAL_INT))"
    )


def test_id_lookup_filters_on_objectid_not_the_internal_pk():
    sql, args = sql_for("id-lookup", _params(), cityobject_class_ids=IDS)
    assert "objectid" in sql
    assert args == ("NL.IMBAG.Pand.0503100000000010",)
    assert "NL.IMBAG.Pand.0503100000000010" not in sql  # bound parameter, not a literal


def test_project_counts_objectclass_id():
    sql, args = sql_for("project", _params(), cityobject_class_ids=IDS)
    assert "count(objectclass_id)" in sql
    assert args == ()


def test_lod_extract_targets_the_truncated_integer_lod_not_the_cityjson_notation():
    # v5's importer truncates "1.2"/"1.3" to "1" (docs/3dcitydb-v5-schema.md,
    # "LoD value format") — querying the literal CityJSON tag "1.2" here
    # would silently match zero rows.
    sql, args = sql_for("lod-extract", _params(), cityobject_class_ids=IDS)
    assert CAPTURED_LOD_TARGET == "1"
    assert args == ("1",)
    assert "1.2" not in sql


def test_lod_extract_filters_on_val_lod_and_val_geometry_id():
    sql, _ = sql_for("lod-extract", _params(), cityobject_class_ids=IDS)
    assert "val_lod" in sql
    assert "val_geometry_id IS NOT NULL" in sql


def test_lod_extract_does_not_reach_for_a_geometry_data_lod_column():
    # geometry_data has no `lod` column at all (Task 5, re-confirmed here).
    sql, _ = sql_for("lod-extract", _params(), cityobject_class_ids=IDS)
    assert "g.lod" not in sql
    assert "geometry_data.lod" not in sql


def test_semantic_surface_filters_on_roofsurface_via_objectclass():
    sql, args = sql_for("semantic-surface", _params(), cityobject_class_ids=IDS)
    assert "objectclass" in sql
    assert "classname" in sql
    assert args == ("RoofSurface",)


def test_semantic_surface_is_not_restricted_to_a_single_lod():
    # Cross-system comparability requirement (review-caught): this query
    # must ask the SAME any-LoD question sql_cjdb.py's and (the fixed)
    # sql_duckdb.py's semantic-surface both ask — see
    # tests/test_semantic_surface_lod_scope.py for the live-data proof
    # against cjdb.
    #
    # This is a DELIBERATE CHOICE, not the only option 3DCityDB v5's schema
    # allows — an earlier version of this comment claimed the latter, and
    # that was an overclaim caught by a second review round. A LoD-scoped
    # query genuinely IS expressible here: `lod1MultiSurface`/
    # `lod2MultiSurface` `property` rows are owned DIRECTLY by the
    # boundary-surface feature itself (`property.feature_id = <the
    # RoofSurface's own id>`, not the Solid) and DO carry `val_lod` —
    # confirmed live, 1116 `lod1MultiSurface` + 1116 `lod2MultiSurface`
    # rows, one of each per RoofSurface feature. A query joining on
    # `lod_pr.feature_id = rs.id AND lod_pr.val_lod = ?` was written and
    # run; it returns 1116 for LoD1 and 1116 for LoD2 — both sensible. See
    # `sql_duckdb.py`'s own `semantic-surface` comment for the full
    # investigation and the real reason any-LoD was still chosen: it is
    # the more natural question ("does this object have a roof surface
    # classified at all"), and picking one specific LoD to scope to would
    # mean picking WHICH LoD — a choice that risks privileging whichever
    # tier each system's own storage model happens to represent most
    # naturally, an easy way for a benchmark to be self-serving toward its
    # own format without meaning to.
    sql, _ = sql_for("semantic-surface", _params(), cityobject_class_ids=IDS)
    assert "val_lod" not in sql


def test_semantic_surface_is_a_cityobject_granular_presence_count():
    # A real cross-system count-mismatch (3dcitydb=2232 vs
    # cjdb=duckdb-cityparquet=1116), caught by this task's smoke target
    # the first time all three ran together: 3DCityDB gives every
    # BuildingPart two solids (lod1Solid + lod2Solid), each with its own
    # boundary-linked RoofSurface feature, so a raw `feature`-row count is
    # NOT the same question cjdb/duckdb-cityparquet ask ("does this
    # CityObject have >=1 RoofSurface"). Fixed to count DISTINCT owning
    # features instead, and gated by the canonical CityObject-granularity
    # predicate — applied to the OWNER, not to the RoofSurface feature
    # itself (which would legitimately return 0: a RoofSurface descends
    # from AbstractSpaceBoundary and never itself passes the predicate).
    sql, args = sql_for("semantic-surface", _params(), cityobject_class_ids=IDS)
    assert "count(DISTINCT pr.feature_id)" in sql
    assert "owner.objectclass_id IN" in sql
    assert args == ("RoofSurface",)


def test_hierarchy_joins_parent_via_the_property_fk_and_child_via_val_feature_id():
    sql, args = sql_for("hierarchy", _params(), cityobject_class_ids=IDS)
    assert "citydb.feature child" in sql
    assert "citydb.feature parent" in sql
    assert "pr.val_feature_id = child.id" in sql
    assert "parent.id = pr.feature_id" in sql
    assert args == ("NL.IMBAG.Pand.0503100000000010",)


def test_hierarchy_raises_scenario_unavailable_when_dataset_has_no_parent_id():
    with pytest.raises(ScenarioUnavailable, match="dataset has no parent/child hierarchy"):
        sql_for("hierarchy", _params(parent_id=None), cityobject_class_ids=IDS)


def test_hierarchy_applies_the_resolved_predicate_qualified_to_child_only():
    # Fix, coordinator review round 1: an undefended "no predicate needed"
    # relied on a delft-specific fact (params.parent_id is always
    # Building-typed, and no 'boundary'-named association ever originates
    # from a Building) with nothing in code to protect a dataset where
    # that does not hold. The predicate must be qualified to `child`
    # specifically — `parent` also has an unqualified `objectclass_id` in
    # scope, so a bare, unqualified predicate here would be ambiguous SQL,
    # not merely imprecise.
    sql, args = sql_for("hierarchy", _params(), cityobject_class_ids=IDS)
    assert "child.objectclass_id IN (100, 901, 902)" in sql
    # `parent` must never carry an unqualified or wrongly-qualified copy.
    assert "parent.objectclass_id IN" not in sql
    assert args == ("NL.IMBAG.Pand.0503100000000010",)


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
