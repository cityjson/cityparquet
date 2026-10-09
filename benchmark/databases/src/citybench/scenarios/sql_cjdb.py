"""cjdb SQL per scenario.

Schema (captured in docs/cjdb-schema.md):
  city_object(id, cj_metadata_id, type, object_id,
              attributes JSONB, geometry JSONB, ground_geometry GEOMETRY)
  city_object_relationships(id, parent_id, child_id)

cjdb keeps full geometry as JSONB and only the 2D footprint as a PostGIS
geometry. Spatial queries therefore run against ground_geometry and are
2D — the same limitation FlatCityBuf's R-tree has, and disclosed the same
way.
"""

from __future__ import annotations

import hashlib
import re

from citybench.config import BBox, BboxWindow, IdProbe, Params
from citybench.scenarios import registry
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.scenarios.sql_citydb import exact_box_args, exact_box_predicate

SCHEMA = "cjdb"
SRID_PLACEHOLDER = 0  # replaced by the adapter with the dataset's real SRID


#: The CityObject type the Building-grained CJDB queries (Q4, Q6-Q8)
#: restrict to. `sql_duckdb.BUILDING_TYPE`'s counterpart.
BUILDING_TYPE = "Building"


def sql_for(scenario: str, params: Params, window: BboxWindow | None = None,
            srid: int = SRID_PLACEHOLDER, *,
            probe: IdProbe | None = None) -> tuple[str, tuple]:
    p = params
    t = f"{SCHEMA}.city_object"

    if scenario == "geometry-scan":
        # Every object's id and its whole geometry JSONB, fetched in
        # PostgreSQL's binary wire format (no text cast, no byte sum).
        return f"SELECT object_id, geometry FROM {t}", ()

    if scenario == "bbox-query":
        # Ids plus the highest-LoD element of each matching object's
        # geometry array, ordered numerically on the dotted LoD ("2.2" >
        # "1.3" > "0").
        win = _window(window)
        return (
            "SELECT object_id, (SELECT g FROM jsonb_array_elements(geometry) g "
            "ORDER BY string_to_array(g ->> 'lod', '.')::int[] DESC LIMIT 1) AS geometry "
            f"FROM {t} "
            f"WHERE {exact_box_predicate('ground_geometry')}",
            exact_box_args(win, srid),
        )

    if scenario == "attr-filter":
        # The same per-dataset CityJSON attribute the format family filters
        # on, reached through the JSONB document; served by the expression
        # index `attribute_index_ddl()` builds for this dataset's predicate.
        if p.attr_filter is None:
            raise ScenarioUnavailable("dataset has no attr-filter predicate")
        spec = p.attr_filter
        if spec.op == "eq":
            return (
                f"SELECT object_id FROM {t} "
                f"WHERE attributes ->> '{spec.column}' = %s",
                (spec.eq_value,),
            )
        return (
            f"SELECT object_id FROM {t} "
            f"WHERE (attributes ->> '{spec.column}')::float >= %s",
            (spec.ge_bound,),
        )

    if scenario == "attr-range":
        # CJDB Q1, in cjdb's own idiom.
        if p.attr_range is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        return (
            f"SELECT object_id FROM {t} "
            f"WHERE (attributes ->> '{p.attr_range.column}')::float > %s",
            (p.attr_range.threshold,),
        )

    if scenario == "attr-stats":
        # Mirrors the guards the other two modules apply: a
        # dataset with no numeric attribute at all is a legitimate dataset
        # property (see sql_duckdb.py's equivalent guard for the two
        # heterogeneity-corpus datasets that hit this), not a query bug —
        # raised before `None` could be interpolated into the JSONB key.
        if p.numeric_column is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        col = f"(attributes ->> '{p.numeric_column}')::float8"
        # min, max, sum, count: the registry's last-column convention.
        return (
            f"SELECT min({col}), max({col}), sum({col}), count({col}) FROM {t}",
            (),
        )

    if scenario == "id-lookup":
        # One of the runner's two probes — the middle of the canonical
        # stream order, or a verified-absent id. The row `SELECT *`
        # materialises includes the geometry JSONB.
        return f"SELECT * FROM {t} WHERE object_id = %s", (_probe(probe).id,)

    if scenario == "lod-query":
        # Catalogue B12 / CJDB Q5, "the objects carrying an LoD 2.2
        # geometry": each one's id and that geometry element of the
        # `geometry` JSONB array, cjdb's native form, fetched in binary.
        #
        # The WHERE keeps the @? jsonpath-match OPERATOR rather than
        # jsonb_path_exists(...): PostgreSQL 16's planner reaches cjdb's
        # `lod` GIN(geometry) index through the operator form only (a
        # Bitmap Index Scan, confirmed by EXPLAIN on a live import); the
        # function form falls back to a Seq Scan. @? also suppresses
        # structural errors (a missing key) and returns false.
        registry.require_lod_query_target(p)
        return (
            "SELECT object_id, jsonb_path_query_first(geometry, "
            "'$[*] ? (@.lod == \"2.2\")') "
            f"FROM {t} WHERE geometry @? '$[*] ? (@.lod == \"2.2\")'",
            (),
        )

    if scenario == "parts-per-building":
        # CJDB Q4, adapted to cjdb 2.2.0's actual schema. The paper joins
        # `city_object_relationships.parent_id` to the city object, but in
        # 2.2.0 `parent_id` is the INTEGER surrogate `city_object.id`, not
        # the textual `object_id` (`docs/cjdb-schema.md`), so the join is on
        # `co.id` and the grouping on `co.object_id`.
        #
        # `LEFT JOIN`, and no `HAVING`: Q4 reports one row per Building
        # INCLUDING childless ones. Dropping them would compare a different
        # result set against the other systems' — the mapping error Codex
        # caught in the review's §7.
        return (
            f'SELECT co.object_id, count(cor.child_id) FROM {t} co '
            f"LEFT JOIN {SCHEMA}.city_object_relationships cor "
            "ON cor.parent_id = co.id "
            'WHERE co."type" = %s GROUP BY co.object_id',
            (BUILDING_TYPE,),
        )

    raise KeyError(f"unknown scenario: {scenario}")


def _window(window: BboxWindow | None) -> BBox:
    if window is None:
        raise ValueError("a windowed scenario needs its resolved BboxWindow")
    return window.window


def _probe(probe: IdProbe | None) -> IdProbe:
    if probe is None:
        raise ValueError("id-lookup needs its resolved IdProbe")
    return probe


#: The tables `append-object`'s untimed reset empties back to its watermark,
#: in foreign-key order. `city_object_relationships` references
#: `city_object`, which references `cj_metadata`; the import adds rows to
#: all three (a fresh `cj_metadata` row per imported file), and leaving the
#: metadata row behind would make the NEXT sample prompt interactively —
#: cjdb's importer asks on stdin when a file of that name was imported
#: before (`cjdb/modules/importer.py`), which in a benchmark run is a hang,
#: not a question.
APPEND_RESET_TABLES: tuple[str, ...] = (
    "city_object_relationships", "city_object", "cj_metadata",
)


def append_watermark_sql() -> list[tuple[str, str]]:
    """`(table, sql)` for the highest id each append-affected table holds.

    Read UNTIMED before each sample. A watermark rather than a `LIKE` on the
    suffixed ids because the importer also writes rows that carry no id of
    ours at all — the relationship rows tying the appended Building to its
    parts, and its own `cj_metadata` row.
    """
    return [
        (table, f"SELECT coalesce(max(id), 0) FROM {SCHEMA}.{table}")
        for table in APPEND_RESET_TABLES
    ]


def append_reset_sql(watermarks: dict[str, int]) -> list[tuple[str, tuple]]:
    """The UNTIMED statements removing everything one append added.

    Run before every sample after the first, and once more after the last,
    so the schema every other scenario was measured against is the schema
    this one leaves behind.
    """
    return [
        (f"DELETE FROM {SCHEMA}.{table} WHERE id > %s", (watermarks[table],))
        for table in APPEND_RESET_TABLES
    ]


def write_sql(scenario: str) -> tuple[str, tuple]:
    """CJDB's own Q6/Q7/Q8, against cjdb 2.2.0's schema.

    One deliberate deviation from the paper's text, and only one: the
    paper's trailing ``::json`` cast is dropped, because cjdb 2.2.0 stores
    `city_object.attributes` as **jsonb** (`docs/cjdb-schema.md`) and
    PostgreSQL registers no assignment cast from `json` to `jsonb`. The
    `attributes::jsonb` the paper writes is kept verbatim and is a no-op on
    this schema.

    Two properties of the paper's own SQL that this harness does NOT
    "fix", because fixing them would benchmark a query CJDB never ran:

    - `jsonb_set` is STRICT, so a Building whose `ground_geometry` is NULL
      has its whole `attributes` document set to NULL by Q6. On 3DBAG the
      NULL footprints are all BuildingParts (review §5), which Q6's
      `type = 'Building'` predicate excludes, so no row is affected there;
      on another corpus it could be.
    - Q6 computes `ST_Area(ground_geometry)` — a footprint area — where
      3DCityDB's Q6 computes `ST_Area(envelope)`, an envelope area. That
      asymmetry is the CJDB paper's, inherited here and disclosed in the
      README rather than silently equalised.
    """
    t = f"{SCHEMA}.city_object"
    if scenario == "attr-add":
        return (
            f"UPDATE {t} SET attributes = jsonb_set(attributes::jsonb, "
            "'{footprint_area}', to_jsonb(ST_Area(ground_geometry))) "
            'WHERE "type" = %s',
            (BUILDING_TYPE,),
        )
    if scenario == "attr-update":
        return (
            f"UPDATE {t} SET attributes = jsonb_set(attributes::jsonb, "
            "'{footprint_area}', "
            "to_jsonb((attributes ->> 'footprint_area')::float + 10.0)) "
            'WHERE "type" = %s',
            (BUILDING_TYPE,),
        )
    if scenario == "attr-delete":
        return (
            f"UPDATE {t} SET attributes = jsonb_set_lax(attributes::jsonb, "
            "'{footprint_area}', NULL, true, 'delete_key') "
            'WHERE "type" = %s',
            (BUILDING_TYPE,),
        )
    raise KeyError(f"unknown write scenario: {scenario}")


def write_reset_sql(scenario: str) -> list[tuple[str, tuple]]:
    """The UNTIMED statements restoring the state one write scenario
    expects, run before every sample after the first so that `repeat` > 1
    measures the same work each time.

    `attr-update` needs none: incrementing an existing key is the same
    amount of work however many times it has already been incremented.
    """
    if scenario == "attr-add":
        return [write_sql("attr-delete")]
    if scenario == "attr-update":
        return []
    if scenario == "attr-delete":
        return [write_sql("attr-add")]
    raise KeyError(f"unknown write scenario: {scenario}")


def index_ddl() -> list[str]:
    """The index set genuinely MISSING from cjdb's own defaults.

    cjdb (per docs/cjdb-schema.md, captured from a real import) already
    creates, unasked:
      - city_object_ground_gix / idx_city_object_ground_geometry — both
        GIST(ground_geometry), covering bbox-query.
      - city_object_type_idx — btree("type"), covering the `type =
        'Building'` restriction of parts-per-building and of every
        write-tier statement.
      - lod — GIN(geometry) using the DEFAULT jsonb_ops opclass, covering
        lod-query's `geometry @? path` predicate.
        Verified empirically (EXPLAIN, default planner settings, against a
        live import with ONLY cjdb's own indexes present): jsonb_ops
        supports the @? jsonpath-match operator just as well as the more
        specialised jsonb_path_ops opclass does for this query shape —
        both give a Bitmap Index Scan. A second GIN index here would be a
        genuinely redundant index object, not a fairness improvement.
      - city_object_relationships_parent_idx / _child_idx — btree on
        parent_id/child_id, covering parts-per-building.
      - city_object_cj_metadata_id_object_id_key — a UNIQUE btree on
        (cj_metadata_id, object_id). This does NOT cover id-lookup's
        `WHERE object_id = %s` the way a leading-column index would:
        verified by EXPLAIN that without a dedicated index, Postgres must
        apply the object_id equality while walking the ENTIRE composite
        index (cost 0.28..44.52, 24 buffer hits on the tiny delft fixture,
        since every row shares one cj_metadata_id and so the leading
        column does not discriminate); WITH a dedicated btree(object_id),
        the same query drops to cost 0.28..2.50, 3 buffer hits. This one
        genuinely is missing.

    Creating the other four (ground_geometry, type, geometry, parent/child)
    on top of cjdb's own would build duplicate index objects: no query
    benefit, but they DO inflate on-disk size — the very metric this
    project's own format is compared against.

    `attributes` gets no index from cjdb's importer. The attribute
    predicates are indexed separately, per dataset, by
    `attribute_index_ddl()` — see there.

    Only what is genuinely missing is created here; this DDL is committed
    alongside the results.
    """
    t = f"{SCHEMA}.city_object"
    return [
        f"CREATE INDEX IF NOT EXISTS ix_co_object_id ON {t} (object_id)",
    ]


def _quote_literal(value: str) -> str:
    return value.replace("'", "''")


def _index_name(prefix: str, column: str) -> str:
    """A safe identifier (<= 63 bytes) derived from an attribute name."""
    slug = re.sub(r"[^a-z0-9]+", "_", column.lower()).strip("_") or "attr"
    digest = hashlib.sha1(column.encode()).hexdigest()[:8]
    return f"{prefix}_{slug}"[: 63 - 9] + f"_{digest}"


def attribute_index_ddl(p: Params) -> list[str]:
    """Expression indexes on the attribute predicates this dataset queries.

    Index policy: every queried predicate is indexed where the system
    supports it. cjdb keeps attributes in one JSONB document, so the
    attribute scenarios are served by btree expression indexes on exactly
    the expressions the queries evaluate: the text value
    `attributes ->> col` for an equality filter, and its `float8` cast for
    a numeric bound (`attr-filter`'s `>=` form and `attr-range`). The
    attribute names come from the run's parameters, so the indexes are
    built per dataset after import; their build time is recorded apart
    from the import time, and their bytes count towards `size_bytes` but
    not `size_bytes_no_index`.
    """
    t = f"{SCHEMA}.city_object"
    text_cols: list[str] = []
    num_cols: list[str] = []
    if p.attr_filter is not None:
        (text_cols if p.attr_filter.op == "eq" else num_cols).append(p.attr_filter.column)
    if p.attr_range is not None:
        num_cols.append(p.attr_range.column)
    ddl: list[str] = []
    for col in dict.fromkeys(text_cols):
        ddl.append(
            f"CREATE INDEX IF NOT EXISTS {_index_name('ix_co_attr_txt', col)} "
            f"ON {t} (((attributes ->> '{_quote_literal(col)}')))"
        )
    for col in dict.fromkeys(num_cols):
        ddl.append(
            f"CREATE INDEX IF NOT EXISTS {_index_name('ix_co_attr_num', col)} "
            f"ON {t} ((((attributes ->> '{_quote_literal(col)}'))::float8))"
        )
    return ddl
