"""Pure-function tests for the cjdb SQL builder.

``sql_for`` never touches a database, so every scenario branch is testable
without a running engine or an imported schema. The brief's own tests below
are a floor, not a ceiling: the sections after them cover branches a typo
or a wrong column name would slip straight through otherwise.
"""

import pytest

from citybench.config import BBox, Params
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_cjdb import index_ddl, sql_for

PARAMS = Params(
    bbox_full=BBox(0.0, 0.0, 0.0, 100.0, 100.0, 10.0),
    attr_column="object_type",
    attr_eq="Building",
    numeric_column="h_dak_max",
    target_id="NL.IMBAG.Pand.1",
    parent_id="NL.IMBAG.Pand.2",
    total_city_objects=2231,
)


def test_count_targets_city_object():
    sql, args = sql_for("count", PARAMS)
    assert "city_object" in sql
    assert args == ()


def test_bbox_query_uses_postgis_operator_for_index_use():
    sql, args = sql_for("bbox-query", PARAMS, selectivity=0.25)
    # The && operator is what engages the GiST index; ST_Intersects on
    # raw geometry without && would not.
    assert "&&" in sql
    assert "ground_geometry" in sql
    assert len(args) == 5  # four ordinates plus the SRID


def test_attr_filter_uses_the_type_column_not_jsonb():
    sql, args = sql_for("attr-filter", PARAMS)
    assert '"type"' in sql
    assert args == ("Building",)


def test_attr_stats_casts_jsonb_to_numeric():
    sql, args = sql_for("attr-stats", PARAMS)
    assert "attributes ->> 'h_dak_max'" in sql
    assert "::numeric" in sql
    assert args == ()


def test_hierarchy_uses_the_relationships_table():
    sql, args = sql_for("hierarchy", PARAMS)
    assert "city_object_relationships" in sql
    assert args == ("NL.IMBAG.Pand.2",)


def test_unknown_scenario_raises():
    with pytest.raises(KeyError):
        sql_for("nonsense", PARAMS)


# --- Beyond the brief ---------------------------------------------------
#
# Every scenario branch in sql_for, plus index_ddl, gets its own assertion:
# a database is never involved, so there is no excuse for an untested
# branch here to be the one that ships a wrong column name.


def _params(*, parent_id: str | None = "NL.IMBAG.Pand.2",
            numeric_column: str | None = "h_dak_max") -> Params:
    return Params(
        bbox_full=BBox(minx=0.0, miny=0.0, minz=0.0, maxx=100.0, maxy=100.0, maxz=10.0),
        attr_column="object_type",
        attr_eq="Building",
        numeric_column=numeric_column,
        target_id="NL.IMBAG.Pand.1",
        parent_id=parent_id,
        total_city_objects=2231,
    )


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # Mirrors hierarchy's own parent_id-is-None guard just below: a
    # dataset with no numeric attribute at all (Montreal, lod3_railway —
    # see params.py's derive()) is a legitimate dataset property, not a
    # query bug.
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        sql_for("attr-stats", _params(numeric_column=None))


def test_full_read_selects_count_first_then_a_forcing_checksum():
    # count_mode("full-read") == "first-column": the registry's cross-system
    # comparison depends on count(*) being the FIRST column, with the
    # forcing checksum after it.
    sql, args = sql_for("full-read", _params())
    assert sql.strip().upper().startswith("SELECT COUNT(*)")
    assert args == ()


def test_full_read_forces_every_substantial_column_not_just_geometry():
    # sql_duckdb's full-read hashes across EVERY column (hash(COLUMNS(*))).
    # A cjdb full-read that only decoded `geometry` would be doing strictly
    # less work than its DuckDB counterpart and would look artificially
    # fast on this Tier-1 headline scenario purely because it was asked a
    # cheaper question. attributes and ground_geometry must be forced too.
    sql, _ = sql_for("full-read", _params())
    assert "geometry::text" in sql
    assert "attributes::text" in sql
    assert "ground_geometry::text" in sql


def test_full_read_coalesces_each_column_so_one_null_does_not_zero_the_row():
    # A row with e.g. no attributes must still contribute its geometry and
    # ground_geometry length to the sum. Summing lengths without coalesce
    # would make `x + NULL` NULL for that whole row, silently dropping its
    # contribution — not a crash, just a quietly wrong number.
    sql, _ = sql_for("full-read", _params())
    assert sql.count("coalesce(") == 3


def test_bbox_query_parameterises_the_window_and_srid_in_order():
    sql, args = sql_for("bbox-query", _params(), selectivity=0.25, srid=7415)
    win = _params().bbox_full.window(0.25)
    assert args == (win.minx, win.miny, win.maxx, win.maxy, 7415)


def test_bbox_query_defaults_to_the_srid_placeholder_when_none_is_given():
    _, args = sql_for("bbox-query", _params(), selectivity=0.25)
    assert args[-1] == 0  # SRID_PLACEHOLDER, unmistakably not a real SRID


def test_attr_filter_parameterises_rather_than_interpolating_the_value():
    sql, args = sql_for("attr-filter", _params())
    assert args == ("Building",)
    assert "Building" not in sql  # would be a SQL-injection-shaped bug if it were


def test_id_lookup_filters_on_object_id_not_the_internal_pk():
    sql, args = sql_for("id-lookup", _params())
    assert "object_id" in sql
    assert args == ("NL.IMBAG.Pand.1",)
    assert "NL.IMBAG.Pand.1" not in sql  # bound parameter, not a literal


def test_project_counts_the_type_column_rather_than_star():
    sql, args = sql_for("project", _params())
    assert 'count("type")' in sql
    assert "count(*)" not in sql
    assert args == ()


def test_lod_extract_filters_on_the_lod_field_inside_the_geometry_jsonb():
    sql, args = sql_for("lod-extract", _params())
    assert "geometry @?" in sql
    assert '"1.2"' in sql  # the LoD tag, not the LoD2 geometry
    assert args == ()


def test_lod_extract_uses_the_operator_form_not_the_function_form():
    # Discriminating on purpose: for structurally regular JSON (which
    # delft's is), jsonb_path_exists(geometry, path) and geometry @? path
    # return the same boolean — but only the @? OPERATOR is recognised by
    # Postgres 16's planner as cooperating with cjdb's own `lod`
    # GIN(geometry) index, confirmed by EXPLAIN against a live import (see
    # the fix report). The function-call form silently forces a Seq Scan
    # even with the index present, which a test that only checked "the
    # right column is filtered on" would not catch. (The two forms are NOT
    # interchangeable on irregular geometry — @? suppresses structural
    # errors during path evaluation and jsonb_path_exists without
    # silent=>true does not — but that distinction is orthogonal to what
    # this test pins: the index-reachability of the syntax actually used.)
    sql, _ = sql_for("lod-extract", _params())
    assert "jsonb_path_exists" not in sql
    assert "@?" in sql


def test_semantic_surface_filters_on_the_roof_surface_type_inside_the_jsonb():
    sql, args = sql_for("semantic-surface", _params())
    assert "geometry @?" in sql
    assert "RoofSurface" in sql
    assert "semantics.surfaces" in sql
    assert args == ()


def test_semantic_surface_uses_the_operator_form_not_the_function_form():
    # See test_lod_extract_uses_the_operator_form_not_the_function_form —
    # the same index-defeating trap applies to this scenario's predicate.
    sql, _ = sql_for("semantic-surface", _params())
    assert "jsonb_path_exists" not in sql
    assert "@?" in sql


def test_semantic_surface_is_not_restricted_to_a_single_lod():
    # Cross-system comparability defect (review-caught): sql_duckdb.py's
    # semantic-surface scenario was once scoped to LoD2.2 alone while this
    # query and sql_citydb.py's both check any LoD — see
    # tests/test_semantic_surface_lod_scope.py for the live-data proof.
    # This query's OWN any-LoD-ness must not silently regress either: the
    # leading `$[*]` iterates cjdb's geometry array (one entry per LoD)
    # UNFILTERED — there must be no `@.lod ==` predicate narrowing which
    # LoD's semantics are inspected.
    sql, _ = sql_for("semantic-surface", _params())
    assert "$[*].semantics.surfaces" in sql
    assert "@.lod" not in sql


def test_hierarchy_raises_scenario_unavailable_when_dataset_has_no_parent_id():
    # Mirrors sql_duckdb's own guard: cjdb also has a relationships table
    # that a hierarchy-less dataset simply has no rows for, so this must be
    # intercepted before a query is built, not sent as a query that would
    # match a NULL parent-id filter.
    with pytest.raises(ScenarioUnavailable, match="dataset has no parent/child hierarchy"):
        sql_for("hierarchy", _params(parent_id=None))


def test_index_ddl_creates_the_one_index_cjdb_genuinely_lacks():
    # id-lookup filters on object_id alone. cjdb's own default is a UNIQUE
    # btree on the COMPOSITE (cj_metadata_id, object_id) — verified by
    # EXPLAIN against a live import that this does not serve a bare
    # object_id equality the way a dedicated index does (cost 44.52 / 24
    # buffer hits without one, vs cost 2.50 / 3 buffer hits with one, on
    # the small delft fixture — the gap only widens at real scale, since
    # every row here shares one cj_metadata_id and so the leading column
    # of the composite index does not discriminate at all). This is the
    # only column the benchmark queries that cjdb does not already index.
    ddl = index_ddl()
    assert len(ddl) == 1
    assert "object_id" in ddl[0]
    assert "city_object" in ddl[0]


def test_index_ddl_does_not_duplicate_indexes_cjdb_already_creates():
    # Four of cjdb's own defaults already cover four of the benchmark's
    # predicates (see docs/cjdb-schema.md, and index_ddl's own docstring
    # for the EXPLAIN evidence behind each): GIST(ground_geometry) x2,
    # btree("type"), GIN(geometry) [jsonb_ops — confirmed to serve the @?
    # operator lod-extract/semantic-surface use just as well as a second,
    # jsonb_path_ops-opclass index would], and btree(parent_id)/(child_id).
    # `CREATE INDEX IF NOT EXISTS` dedupes by NAME, not by definition, so
    # re-adding any of these under a different name would build a genuinely
    # redundant index object — inflating cjdb's on-disk size (a headline
    # metric this project's own format is compared against) for zero query
    # benefit. attributes is excluded for an unrelated reason: no scenario
    # query filters on it at all (attr-stats aggregates unconditionally),
    # so an index there would sit unused.
    ddl = "\n".join(index_ddl())
    assert "ground_geometry" not in ddl
    assert '"type"' not in ddl
    assert "attributes" not in ddl
    assert "GIN" not in ddl  # no geometry GIN index — cjdb's own `lod` covers @?
    assert "GIST" not in ddl
    assert "parent_id" not in ddl
    assert "child_id" not in ddl


def test_index_ddl_statements_are_idempotent_create_index_if_not_exists():
    # `just up` may be re-run against an already-imported schema; a bare
    # CREATE INDEX would then fail the second time round.
    for stmt in index_ddl():
        assert stmt.strip().upper().startswith("CREATE INDEX IF NOT EXISTS")


def test_index_ddl_takes_no_arguments():
    # Pinned per the task instructions: an earlier plan had this take an
    # unused `params` argument. The adapter calls it with none.
    ddl = index_ddl()
    assert isinstance(ddl, list)
    assert all(isinstance(stmt, str) for stmt in ddl)
