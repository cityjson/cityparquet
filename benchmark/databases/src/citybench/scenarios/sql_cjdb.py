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

from citybench.config import BBox, BboxWindow, IdProbe, Params
from citybench.scenarios.registry import ScenarioUnavailable

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

    if scenario == "count":
        return f"SELECT count(*) FROM {t}", ()

    if scenario == "geometry-scan":
        # The geometry column only, matching `sql_duckdb`'s own
        # `geometry-scan`: every object's geometry, once, reported as
        # `(count, bytes)`. The old `full-read` also summed `attributes`
        # and `ground_geometry`, which was a different amount of work from
        # the other two systems' rows.
        #
        # `length(geometry::text)` is a JSONB-to-text serialisation, not a
        # stored byte length: a client reading JSONB genuinely pays this,
        # but it is NOT the same operation DuckDB's `octet_length` performs
        # (README Caveat 18). `length()` then throws the string away.
        return (
            f"SELECT count(*), sum(coalesce(length(geometry::text), 0))::bigint "
            f"FROM {t}",
            (),
        )

    if scenario == "bbox-query":
        win = _window(window)
        return (
            f"SELECT count(*) FROM {t} "
            "WHERE ground_geometry && ST_MakeEnvelope(%s, %s, %s, %s, %s)",
            (win.minx, win.miny, win.maxx, win.maxy, srid),
        )

    if scenario == "attr-filter":
        # The same per-dataset CityJSON attribute the format family filters
        # on, reached through the JSONB document. cjdb builds no index on
        # `attributes` and this harness adds none: a GIN(attributes) index
        # would sit unused by every other scenario while inflating
        # `size_bytes`, the storage figure CityParquet is compared against.
        # Disclosed in the README rather than silently compensated.
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
        col = f"(attributes ->> '{p.numeric_column}')::numeric"
        # count first, per the registry's first-column convention.
        return (
            f"SELECT count({col}), min({col}), max({col}), sum({col}) FROM {t}",
            (),
        )

    if scenario == "id-lookup":
        # One of the runner's four probes — 10/50/90 % of the canonical
        # stream order plus a verified-absent id. The row `SELECT *`
        # materialises includes the geometry JSONB.
        return f"SELECT * FROM {t} WHERE object_id = %s", (_probe(probe).id,)

    if scenario == "lod-query":
        # Catalogue B12 / CJDB Q5: "retrieve all buildings having a specific
        # LoD geometry". WHOLE ROWS, as the other two systems return — and
        # on cjdb the row carries the whole `geometry` JSONB, so this
        # materialises every matching object's full geometry document.
        #
        # No per-LoD column exists: the LoD lives inside the geometry
        # JSONB, so every row's geometry must be visited and filtered.
        #
        # The @? jsonpath-match OPERATOR is used deliberately instead of
        # the jsonb_path_exists(...) FUNCTION. Confirmed by EXPLAIN against
        # a live import: Postgres 16's planner does not recognise the
        # function-call form as index-cooperating with cjdb's own `lod`
        # GIN(geometry) index and falls back to a Seq Scan even with the
        # index present and enable_seqscan forced off. The @? operator form
        # reaches the same index via a Bitmap Index Scan, chosen under
        # default planner settings. Using the function form here would
        # silently defeat the index this task's fairness constraint depends
        # on.
        #
        # NOT a drop-in equivalent, however: geometry @? path and
        # jsonb_path_exists(geometry, path) differ on rows with irregular
        # structure. @? (like @@) always suppresses structural errors
        # during path evaluation (a missing key, a type mismatch) and
        # returns false; jsonb_path_exists(...) without silent => true does
        # not — it raises. delft's geometry is regular enough that neither
        # form ever hits this, which is why the count check could not have
        # detected a divergence either way. Datasets with less regular
        # geometry (mixed CityGML modules, sparse/optional semantics)
        # should be watched for this the first time this SQL runs against
        # them.
        return (
            f"SELECT * FROM {t} "
            "WHERE geometry @? '$[*] ? (@.lod == \"1.2\")'",
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

    `attributes` has NO index, and this harness adds none. Two scenarios do
    now filter on it — `attr-filter` (`attributes ->> '<col>' = %s`) and
    `attr-range` (`(attributes ->> '<col>')::float > %s`) — so unlike the
    other columns this is a real, measurable disadvantage rather than a
    redundancy. It is left alone because a useful index here is a
    per-expression btree on the one attribute each dataset happens to be
    filtered by, which is a query-specific object cjdb's own importer never
    builds and a real cjdb deployment would not have; a blanket
    GIN(attributes) would not serve `attr-range`'s inequality at all while
    adding materially to `size_bytes`. The README states this beside both
    scenarios' numbers.

    Only what is genuinely missing is created here; this DDL is committed
    alongside the results.
    """
    t = f"{SCHEMA}.city_object"
    return [
        f"CREATE INDEX IF NOT EXISTS ix_co_object_id ON {t} (object_id)",
    ]
