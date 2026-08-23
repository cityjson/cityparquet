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

from citybench.config import Params
from citybench.scenarios.registry import ScenarioUnavailable

SCHEMA = "cjdb"
SRID_PLACEHOLDER = 0  # replaced by the adapter with the dataset's real SRID


def sql_for(scenario: str, params: Params, selectivity: float | None = None,
            srid: int = SRID_PLACEHOLDER) -> tuple[str, tuple]:
    p = params
    t = f"{SCHEMA}.city_object"

    if scenario == "count":
        return f"SELECT count(*) FROM {t}", ()

    if scenario == "full-read":
        # count(*) first (the comparable quantity), then a checksum that
        # touches every SUBSTANTIAL column — geometry, attributes and
        # ground_geometry — not just geometry. sql_duckdb's counterpart
        # hashes across every column (hash(COLUMNS(*))); decoding only one
        # column here would let cjdb look artificially fast on this Tier-1
        # headline scenario purely because it is asked to do less work.
        # Each column's length is coalesced to 0 individually so a row
        # with one NULL column does not null out the whole row's
        # contribution to the sum (and so silently vanish from the total).
        return (
            f"SELECT count(*), sum("
            "coalesce(length(geometry::text), 0) + "
            "coalesce(length(attributes::text), 0) + "
            "coalesce(length(ground_geometry::text), 0)"
            f")::bigint FROM {t}",
            (),
        )

    if scenario == "bbox-query":
        win = p.bbox_full.window(selectivity)
        return (
            f"SELECT count(*) FROM {t} "
            "WHERE ground_geometry && ST_MakeEnvelope(%s, %s, %s, %s, %s)",
            (win.minx, win.miny, win.maxx, win.maxy, srid),
        )

    if scenario == "attr-filter":
        return f'SELECT count(*) FROM {t} WHERE "type" = %s', (p.attr_eq,)

    if scenario == "attr-stats":
        # Mirrors `hierarchy`'s own `parent_id is None` guard below: a
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
        return f"SELECT * FROM {t} WHERE object_id = %s", (p.target_id,)

    if scenario == "project":
        return f'SELECT count("type") FROM {t}', ()

    if scenario == "lod-extract":
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
        # form ever hits this, which is why the 1116/1116 count check could
        # not have detected a divergence either way. Datasets with less
        # regular geometry (mixed CityGML modules, sparse/optional
        # semantics) should be watched for this the first time this SQL
        # runs against them.
        return (
            f"SELECT count(*) FROM {t} "
            "WHERE geometry @? '$[*] ? (@.lod == \"1.2\")'",
            (),
        )

    if scenario == "semantic-surface":
        # See the @? note on lod-extract above — it applies here too.
        return (
            f"SELECT count(*) FROM {t} "
            "WHERE geometry @? "
            "'$[*].semantics.surfaces[*] ? (@.type == \"RoofSurface\")'",
            (),
        )

    if scenario == "hierarchy":
        # Mirrors sql_duckdb's own hierarchy branch: an absent parent/child
        # pair is a property of the dataset, not a query bug, so this is
        # raised before any SQL is built rather than sent as a query that
        # would either error against NULL or silently match nothing.
        if p.parent_id is None:
            raise ScenarioUnavailable("dataset has no parent/child hierarchy")
        return (
            f"SELECT count(*) FROM {SCHEMA}.city_object_relationships r "
            f"JOIN {t} parent ON parent.id = r.parent_id "
            "WHERE parent.object_id = %s",
            (p.parent_id,),
        )

    raise KeyError(f"unknown scenario: {scenario}")


def index_ddl() -> list[str]:
    """The index set genuinely MISSING from cjdb's own defaults.

    cjdb (per docs/cjdb-schema.md, captured from a real import) already
    creates, unasked:
      - city_object_ground_gix / idx_city_object_ground_geometry — both
        GIST(ground_geometry), covering bbox-query.
      - city_object_type_idx — btree("type"), covering attr-filter/project.
      - lod — GIN(geometry) using the DEFAULT jsonb_ops opclass, covering
        lod-extract/semantic-surface's `geometry @? path` predicate.
        Verified empirically (EXPLAIN, default planner settings, against a
        live import with ONLY cjdb's own indexes present): jsonb_ops
        supports the @? jsonpath-match operator just as well as the more
        specialised jsonb_path_ops opclass does for this query shape —
        both give a Bitmap Index Scan. A second GIN index here would be a
        genuinely redundant index object, not a fairness improvement.
      - city_object_relationships_parent_idx / _child_idx — btree on
        parent_id/child_id, covering hierarchy.
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
    project's own format is compared against. attributes has no dedicated
    index either, for a different reason: no scenario query filters on it
    (attr-stats aggregates over the WHOLE table, unconditionally), so a
    GIN(attributes) index would sit unused and only add to cjdb's size.

    Only what is genuinely missing is created here; this DDL is committed
    alongside the results.
    """
    t = f"{SCHEMA}.city_object"
    return [
        f"CREATE INDEX IF NOT EXISTS ix_co_object_id ON {t} (object_id)",
    ]
