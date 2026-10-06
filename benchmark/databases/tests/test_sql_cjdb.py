"""Pure-function tests for the cjdb SQL builder.

``sql_for`` never touches a database, so every scenario branch is testable
without a running engine or an imported schema. The brief's own tests below
are a floor, not a ceiling: the sections after them cover branches a typo
or a wrong column name would slip straight through otherwise.
"""

import pytest

from citybench.config import Params
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_cjdb import (
    append_reset_sql, append_watermark_sql, index_ddl, sql_for,
    write_reset_sql, write_sql,
)
from conftest import ge_attr_filter, make_params, make_probes

PARAMS = make_params(
    id_probes=make_probes("NL.IMBAG.Pand.1"), total_city_objects=2231,
)


def test_count_targets_city_object():
    sql, args = sql_for("count", PARAMS)
    assert "city_object" in sql
    assert args == ()


def test_bbox_query_uses_postgis_operator_for_index_use():
    sql, args = sql_for("bbox-query", PARAMS, _window(PARAMS))
    # The && operator is what engages the GiST index; ST_Intersects on
    # raw geometry without && would not.
    assert "&&" in sql
    assert "ground_geometry" in sql
    assert len(args) == 9  # the probe's four ordinates and SRID, then the recheck's four


def test_unknown_scenario_raises():
    with pytest.raises(KeyError):
        sql_for("nonsense", PARAMS)


# --- Beyond the brief ---------------------------------------------------
#
# Every scenario branch in sql_for, plus index_ddl and the write tier, gets
# its own assertion: a database is never involved, so there is no excuse for
# an untested branch here to be the one that ships a wrong column name.


def _params(**overrides) -> Params:
    return make_params(id_probes=make_probes("NL.IMBAG.Pand.1"), total_city_objects=2231,
                       **overrides)


def _window(params: Params, tag: str = "bbox-25pct"):
    return params.window(tag)


def test_attr_stats_raises_scenario_unavailable_when_dataset_has_no_numeric_column():
    # A dataset with no numeric attribute at all (Montreal, lod3_railway —
    # see params.py) is a legitimate dataset property, not a query bug.
    with pytest.raises(ScenarioUnavailable, match="dataset has no numeric attribute"):
        sql_for("attr-stats", _params(numeric_column=None))


def test_bbox_query_parameterises_the_window_and_srid_in_order():
    params = _params()
    window = _window(params)
    sql, args = sql_for("bbox-query", params, window, srid=7415)
    w = window.window
    assert args == (w.minx, w.miny, w.maxx, w.maxy, 7415,
                    w.minx, w.maxx, w.miny, w.maxy)


def test_bbox_query_defaults_to_the_srid_placeholder_when_none_is_given():
    params = _params()
    _, args = sql_for("bbox-query", params, _window(params))
    assert args[4] == 0  # SRID_PLACEHOLDER, unmistakably not a real SRID


def test_attr_filter_reaches_the_attribute_through_the_jsonb_document():
    sql, args = sql_for("attr-filter", _params())
    assert "attributes ->> 'b3_dak_type' = %s" in sql
    assert sql.strip().startswith("SELECT object_id")
    assert args == ("slanted",)
    assert "slanted" not in sql  # bound, not interpolated
    assert '"type" = %s' not in sql   # no longer the structural column


def test_attr_filter_supports_the_numeric_lower_bound_form():
    sql, args = sql_for("attr-filter", _params(attr_filter=ge_attr_filter()))
    assert "(attributes ->> 'TerrainHeight')::float >= %s" in sql
    assert args == (2.45,)


def test_attr_range_is_cjdb_q1():
    sql, args = sql_for("attr-range", _params())
    assert "(attributes ->> 'b3_h_dak_max')::float > %s" in sql
    assert sql.strip().startswith("SELECT object_id")
    assert args == (20.0,)


def test_id_lookup_asks_for_the_probe_it_was_handed():
    """One call per probe — the ids at 10/50/90 % of the canonical stream
    order plus a verified-absent one — and `object_id`, cjdb's textual
    CityJSON identifier, never the internal integer `id`."""
    params = _params()
    for probe in params.id_probes:
        sql, args = sql_for("id-lookup", params, probe=probe)
        assert "object_id = %s" in sql
        assert args == (probe.id,)
        assert probe.id not in sql
    assert sql.strip().startswith("SELECT *")


def test_id_lookup_without_a_probe_is_a_loud_failure():
    with pytest.raises(ValueError):
        sql_for("id-lookup", _params())


def test_parts_per_building_joins_on_the_integer_surrogate_key():
    """CJDB's Q4 joins `city_object_relationships.parent_id`, which in cjdb
    2.2.0 is the INTEGER `city_object.id`, not the textual `object_id`
    (docs/cjdb-schema.md). Joining on `object_id` would compare a text
    column with an integer one."""
    sql, args = sql_for("parts-per-building", _params())
    assert "cor.parent_id = co.id" in sql
    assert "cor.parent_id = co.object_id" not in sql
    assert "LEFT JOIN" in sql          # childless Buildings are kept
    assert "count(cor.child_id)" in sql
    assert "GROUP BY co.object_id" in sql
    assert args == ("Building",)


def test_write_tier_is_the_cjdb_papers_own_sql():
    add, add_args = write_sql("attr-add")
    assert "jsonb_set(attributes::jsonb, '{footprint_area}'" in add
    assert "to_jsonb(ST_Area(ground_geometry))" in add
    # cjdb 2.2.0 stores `attributes` as jsonb, and PostgreSQL registers no
    # assignment cast from json to jsonb, so the paper's trailing ::json is
    # the one deliberate deviation — documented in write_sql's docstring.
    assert "::json " not in add and not add.rstrip().endswith("::json")
    assert add_args == ("Building",)

    update, _ = write_sql("attr-update")
    assert "+ 10.0" in update

    delete, _ = write_sql("attr-delete")
    assert "jsonb_set_lax" in delete and "'delete_key'" in delete


def test_write_resets_restore_the_state_each_sample_expects():
    assert write_reset_sql("attr-add") == [write_sql("attr-delete")]
    assert write_reset_sql("attr-update") == []
    assert write_reset_sql("attr-delete") == [write_sql("attr-add")]





def test_lod_query_uses_the_operator_form_not_the_function_form():
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
    sql, _ = sql_for("lod-query", _params())
    assert "jsonb_path_exists" not in sql
    assert "@?" in sql


def test_the_append_reset_empties_every_table_the_import_writes_to():
    """`append-object`'s untimed reset. The `cj_metadata` row matters as
    much as the objects: cjdb's importer PROMPTS ON STDIN when a file of
    the same name was imported before, which in a benchmark run is a hang,
    not a question."""
    marks = {table: 10 for table, _ in append_watermark_sql()}
    statements = append_reset_sql(marks)
    tables = [sql for sql, _ in statements]
    assert [sql.count("%s") for sql, _ in statements] == [1] * len(statements)
    assert any("city_object_relationships" in sql for sql in tables)
    assert any("cj_metadata" in sql for sql in tables)
    # Relationships before the objects they reference, metadata last.
    assert tables[0].index("city_object_relationships") >= 0
    assert "cj_metadata" in tables[-1]
    assert all(args == (10,) for _, args in statements)


def test_attr_filter_raises_scenario_unavailable_without_a_predicate():
    # A dataset carrying no attribute the pick rule can build a selective
    # predicate from is a dataset property, not a query bug — intercepted
    # before a query is built, never sent with a None column name.
    with pytest.raises(ScenarioUnavailable, match="attr-filter predicate"):
        sql_for("attr-filter", _params(attr_filter=None))


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
    # operator lod-query uses just as well as a second,
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


def test_geometry_scan_returns_ids_and_the_geometry_document_without_text():
    from citybench.scenarios.sql_cjdb import sql_for
    sql, _ = sql_for("geometry-scan", make_params())
    assert sql.startswith("SELECT object_id, geometry FROM")
    assert "::text" not in sql and "length(" not in sql and "sum(" not in sql


def test_bbox_query_returns_ids_and_the_highest_lod_geometry_element():
    from citybench.scenarios.sql_cjdb import sql_for
    sql, _ = sql_for("bbox-query", make_params(), make_params().window("bbox-25pct"))
    assert sql.startswith("SELECT object_id, (SELECT g FROM jsonb_array_elements(geometry) g")
    assert "string_to_array(g ->> 'lod', '.')::int[] DESC" in sql
    assert "count(" not in sql


def test_attr_stats_casts_to_float8_like_attr_range():
    from citybench.scenarios.sql_cjdb import sql_for
    sql, _ = sql_for("attr-stats", make_params())
    assert "::float8" in sql and "::numeric" not in sql


def test_attr_stats_returns_min_max_sum_count_in_that_order():
    sql, _ = sql_for("attr-stats", make_params())
    select = sql.split(" FROM ")[0]
    positions = [select.index(f"{agg}(") for agg in ("min", "max", "sum", "count")]
    assert positions == sorted(positions), select


# --- Attribute indexes (index policy: every queried predicate indexed) ---

from citybench.scenarios.sql_cjdb import attribute_index_ddl  # noqa: E402


def test_attribute_index_ddl_indexes_the_eq_filter_text_expression():
    ddl = attribute_index_ddl(_params())
    text = [d for d in ddl if "::float8" not in d]
    assert len(text) == 1
    assert "((attributes ->> 'b3_dak_type'))" in text[0]
    assert "cjdb.city_object" in text[0]


def test_attribute_index_ddl_indexes_the_numeric_cast_of_the_range_column():
    p = _params()
    ddl = attribute_index_ddl(p)
    assert any(
        f"(((attributes ->> '{p.attr_range.column}'))::float8)" in d for d in ddl
    )


def test_attribute_index_ddl_ge_filter_and_range_on_one_column_build_one_index():
    import dataclasses
    base = _params()
    p = _params(attr_filter=ge_attr_filter(column="TerrainHeight"),
                attr_range=dataclasses.replace(base.attr_range, column="TerrainHeight"))
    ddl = attribute_index_ddl(p)
    assert len(ddl) == 1
    assert "::float8" in ddl[0]


def test_attribute_index_ddl_is_empty_without_attribute_predicates():
    assert attribute_index_ddl(_params(attr_filter=None, attr_range=None)) == []


def test_attribute_index_names_are_safe_identifiers():
    p = _params(attr_filter=ge_attr_filter(column="Weird-Col Name'x"))
    ddl = attribute_index_ddl(p)
    for d in ddl:
        name = d.split("IF NOT EXISTS ")[1].split(" ")[0]
        assert name.replace("_", "").isalnum() and len(name) <= 63
    assert any("'Weird-Col Name''x'" in d for d in ddl)


def test_bbox_query_rechecks_the_probe_in_double_precision():
    """`&&` compares PostGIS's cached float4 box: at EPSG:6697 magnitudes
    one float4 step is about a metre, so the index probe alone admitted
    objects outside the window (Tokyo: +14 of 499 on 3DCityDB). The exact,
    edge-inclusive test on the box's double-precision bounds follows it."""
    params = _params()
    sql, args = sql_for("bbox-query", params, _window(params))
    assert "ground_geometry && ST_MakeEnvelope" in sql
    for bound in ("ST_XMax(ground_geometry) >= %s", "ST_XMin(ground_geometry) <= %s",
                  "ST_YMax(ground_geometry) >= %s", "ST_YMin(ground_geometry) <= %s"):
        assert bound in sql
