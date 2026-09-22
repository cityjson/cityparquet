"""3DCityDB v5 SQL per scenario.

v5 replaced v4's per-class tables with a single `feature` table plus a
single `property` table holding most attributes and associations. Attribute
access is therefore an EAV join, which is a genuine structural difference
worth measuring rather than a handicap to engineer around.

The constants below are filled in from docs/3dcitydb-v5-schema.md, which is
captured from a real import rather than read from prose documentation.
"""

from __future__ import annotations

from citybench.config import BBox, BboxWindow, Params
from citybench.scenarios.registry import ScenarioUnavailable

SCHEMA = "citydb"

# --- VERIFIED against a live 3dcitydb-pg:16-3.4-5.1.2-alpine instance ---
# These are confirmed, not guessed (see Task 5, re-confirmed here). Do not
# change them without re-inspecting the database and saying so in the
# schema doc.
CAPTURED_ID_COLUMN = "objectid"           # feature's CityObject identifier
CAPTURED_CLASS_COLUMN = "objectclass_id"  # feature's class discriminator
CAPTURED_ENVELOPE_COLUMN = "envelope"     # feature's spatial extent
CAPTURED_PROPERTY_NAME_COLUMN = "name"    # property's attribute-name column
CAPTURED_PROPERTY_FK = "feature_id"       # property -> feature foreign key

# The CityObject-granularity predicate, established empirically in Task 5
# and recorded under "Recommended predicate — use this one" in
# docs/3dcitydb-v5-schema.md: `is_toplevel = 1 OR NOT <descends from
# AbstractSpaceBoundary, objectclass id 13>`. This is the DEFENSIVE form,
# not the two narrower alternatives the schema doc also records — see that
# doc's "Two narrower alternatives" section for exactly why each of those
# is unsafe to reuse without re-verification against a given dataset.
#
# The schema doc states the predicate as a complete standalone statement
# (its own `WITH RECURSIVE`, a `JOIN` to `objectclass` aliased `oc`). This
# module instead needs a single, self-contained boolean EXPRESSION that can
# be dropped into any scenario's `WHERE` clause via plain string
# interpolation — including queries where `feature` is unaliased (`count`),
# aliased `f` (`full-read`, `attr-stats`, `lod-extract`), or not the only
# table in the FROM list (`attr-stats` and `lod-extract` both join
# `property`, which has no `objectclass_id` column of its own, so the bare
# column name below resolves unambiguously to `feature`'s regardless of its
# alias). So the doc's outer `JOIN objectclass oc` + `oc.is_toplevel` is
# rewritten as an uncorrelated `IN (SELECT id FROM objectclass WHERE
# is_toplevel = 1)` subquery, and the recursive CTE is nested INSIDE the
# `NOT IN (...)` subquery rather than prefixing the whole statement —
# PostgreSQL permits `WITH RECURSIVE ... SELECT ...` as a parenthesised
# subquery expression, confirmed directly against a live import. Both
# rewrites are uncorrelated (no reference to the outer query's columns), so
# PostgreSQL evaluates each once and reuses it ("hashed SubPlan" in
# EXPLAIN) rather than once per row — confirmed by EXPLAIN, not assumed;
# see the Task 9 report.
#
# Verified to reproduce the doc's own reference count exactly: 2231 against
# `SELECT count(*) FROM citydb.feature WHERE <this predicate>`.
#
# Built by _cityobject_predicate() rather than as a single fixed string so
# the SAME predicate can be alias-qualified where a query has more than one
# `feature` reference in scope — see `hierarchy` below, the one branch that
# needs the qualified form.
#
# `column` defaults to CAPTURED_CLASS_COLUMN ("objectclass_id" — the
# `feature`-table discriminator every scenario query filters on) but can be
# overridden to "id" to evaluate the SAME logical predicate against
# `objectclass` itself, where the class catalogue's own primary key IS the
# discriminator — see `resolve_cityobject_class_ids()` below, the ONLY
# other caller that overrides it. This keeps the recursive-CTE/OR-of-
# subqueries logic defined in exactly one place rather than duplicated
# between "filter `feature` rows" and "enumerate qualifying classes".
def _cityobject_predicate(alias: str | None = None, *,
                           column: str = CAPTURED_CLASS_COLUMN) -> str:
    col = f"{alias}.{column}" if alias else column
    return (
        f"({col} IN (SELECT id FROM {SCHEMA}.objectclass WHERE is_toplevel = 1) "
        f"OR {col} NOT IN ("
        "WITH RECURSIVE class_chain AS ("
        f"SELECT id, superclass_id, id AS leaf_id FROM {SCHEMA}.objectclass "
        "UNION ALL "
        "SELECT oc.id, oc.superclass_id, cc.leaf_id "
        f"FROM {SCHEMA}.objectclass oc "
        "JOIN class_chain cc ON oc.id = cc.superclass_id"
        ") SELECT leaf_id FROM class_chain WHERE id = 13"
        "))"
    )


CAPTURED_CITYOBJECT_PREDICATE = _cityobject_predicate()


# --- C1 fix (final whole-branch review) -------------------------------
#
# `CAPTURED_CITYOBJECT_PREDICATE` above — an OR across an uncorrelated `IN
# (subquery)` and an uncorrelated `NOT IN (WITH RECURSIVE subquery)` — was,
# until this fix, interpolated DIRECTLY into every scenario's WHERE clause
# and re-evaluated by PostgreSQL on every single query. That shape has no
# usable `Index Cond`: `EXPLAIN` shows it applied as a post-hoc `Filter` on
# top of an UNRESTRICTED index or table scan (a "hashed SubPlan" applied
# per candidate row), not as a restriction the scan itself can use to skip
# non-matching rows — see `index_ddl()`'s docstring below for the full
# EXPLAIN evidence and why this was missed by an earlier measurement. On
# Zurich (2,192,890 raw `feature` rows) this made every one of the 9
# scenarios using the predicate scan all 2.2M rows instead of the 198,699
# that actually qualify, which is a fairness defect biased TOWARD this
# project's own format (`duckdb-cityparquet`), the most damaging direction
# an error in this harness can point.
#
# The fix does not change what qualifies as a CityObject — it changes WHEN
# the predicate is evaluated. `resolve_cityobject_class_ids()` runs the
# identical logical predicate ONCE, against `citydb.objectclass` (a small,
# dataset-independent class catalogue — the same ~500-row table regardless
# of which dataset was imported), and returns the concrete set of
# `objectclass_id` values that qualify (89 of them, verified live against
# this schema). `_static_predicate()` then renders that resolved set as a
# plain `objectclass_id IN (1, 2, 3, ...)` — sargable, and routed by the
# planner through the SAME pre-existing `feature_objectclass_inx` index
# every scenario already had available, now as a genuine `Index Cond`
# rather than a post-scan `Filter`. No new index is required (`index_ddl()`
# correctly stays empty). Measured live on Zurich, `EXPLAIN (ANALYZE,
# BUFFERS)`, same warm cache: `count` 0.408s -> 0.013s (Index Cond
# restricts the scan to 198,699 rows outright, versus the old plan's
# `Index Only Scan` touching the full 2,192,890-row index followed by a
# `Filter` removing 1,994,191 of them) — the review's independently
# reproduced 32.2x. Every downstream count is identical: this is a plan
# change, not a semantics change, and every dataset re-run for this fix
# must reproduce its own previously-committed counts exactly to prove it.
def resolve_cityobject_class_ids(conn) -> tuple[int, ...]:
    """The concrete `objectclass_id` values that satisfy the canonical
    CityObject-granularity predicate, resolved ONCE against a live
    `citydb.objectclass` table rather than re-evaluated by PostgreSQL on
    every scenario query — see the C1 fix note above `sql_for` for why.

    `objectclass` is the fixed CityGML/CityJSON class catalogue 3DCityDB
    v5 ships with its schema, not data derived from the imported dataset,
    so this result is the SAME regardless of which dataset was imported —
    a system only needs to call this once per `ingest()`, not once per
    dataset-specific fact. Callers MUST call this after the schema exists
    (i.e. after the first container start has created it) but do not need
    to re-resolve it between datasets sharing a schema version.

    Uses `_cityobject_predicate(column="id")`: the identical OR-of-two-
    subqueries logic `CAPTURED_CITYOBJECT_PREDICATE` embodies, but
    evaluated against `objectclass.id` (the class catalogue's own primary
    key) instead of `feature.objectclass_id` (a `feature` row's class
    discriminator) — i.e. "which classes qualify", not "which feature rows
    qualify". Verified live against this schema to return exactly 89 ids.
    """
    predicate = _cityobject_predicate(column="id")
    query = f"SELECT id FROM {SCHEMA}.objectclass WHERE {predicate} ORDER BY id"
    with conn.cursor() as cur:
        cur.execute(query)
        return tuple(row[0] for row in cur.fetchall())


def _static_predicate(cityobject_class_ids: tuple[int, ...],
                       alias: str | None = None) -> str:
    """The RESOLVED, sargable form of the CityObject-granularity predicate:
    a plain `objectclass_id IN (...)` over a fixed, pre-resolved id list,
    in place of `_cityobject_predicate`'s OR-of-two-subqueries shape. See
    the C1 fix note above `resolve_cityobject_class_ids` for why this
    exists and what it measurably changes.

    `cityobject_class_ids` must be non-empty: a caller passing an empty
    tuple (e.g. a `resolve_cityobject_class_ids()` call against the wrong
    schema, or a stubbed-out test double) would otherwise get a silently
    wrong `IN ()`, which PostgreSQL evaluates as always-false — matching
    zero rows on every scenario without erroring. Raising here turns that
    into a loud failure at query-build time instead.
    """
    if not cityobject_class_ids:
        raise ValueError(
            "cityobject_class_ids is empty; resolve_cityobject_class_ids() "
            "must have returned at least one id before sql_for() can build "
            "a predicate. An empty IN (...) would silently match nothing."
        )
    column = f"{alias}.{CAPTURED_CLASS_COLUMN}" if alias else CAPTURED_CLASS_COLUMN
    id_list = ", ".join(str(i) for i in sorted(cityobject_class_ids))
    return f"{column} IN ({id_list})"


# `property.val_lod` truncates CityJSON's fractional LoD notation to the
# integer tier — "1.2"/"1.3" both become "1" on import (see
# docs/3dcitydb-v5-schema.md's "LoD value format" section, a Task 9
# finding). Querying the brief's literal "1.2" against this column matches
# zero rows, silently, rather than erroring — the bad case the brief's own
# `lod-extract` comment warns about for a *different* substitution (the
# geometry_data.lod column that doesn't exist at all). "1" is the closest
# comparable tier to cjdb/CityParquet's LoD1.2/1.3.
CAPTURED_LOD_TARGET = "1"

_F = f"{SCHEMA}.feature"
_P = f"{SCHEMA}.property"


#: `property.val_lod` for the LoD0 footprint geometry — the integer tier
#: `citydb-tool` truncates CityJSON's `"0"` to (see CAPTURED_LOD_TARGET and
#: `docs/3dcitydb-v5-schema.md`, "LoD value format": the captured Delft
#: import holds 1115 rows with `val_lod = '0'`, one per Building).
CAPTURED_LOD0_TARGET = "0"

#: The CityGML class name `bbox-fetch`, `point-query`, `parts-per-building`
#: and the write tier restrict to, resolved to an `objectclass_id` once per
#: ingest by `resolve_class_id` rather than hard-coded: the catalogue's ids
#: are a schema-version fact, not a constant this module may assume.
BUILDING_CLASSNAME = "Building"

#: The `datatype.typename` the write tier's new `property` rows are given.
#: `property.datatype_id` is NOT NULL, so a value must be supplied;
#: `namespace_id` is nullable and is left unset, as an ad-hoc generic
#: attribute has no CityGML namespace.
DOUBLE_TYPENAME = "Double"


def resolve_class_id(conn, classname: str) -> int:
    """The `objectclass_id` for one CityGML class name, read live.

    Fails loudly rather than returning a default: a silently wrong class id
    would make every Building-grained scenario match nothing while still
    producing a plausible-looking zero.
    """
    with conn.cursor() as cur:
        cur.execute(
            f"SELECT id FROM {SCHEMA}.objectclass WHERE classname = %s", (classname,)
        )
        row = cur.fetchone()
    if row is None:
        raise ValueError(
            f"{SCHEMA}.objectclass has no class named {classname!r}; the "
            "Building-grained scenarios cannot be built against this schema"
        )
    return int(row[0])


def resolve_datatype_id(conn, typename: str = DOUBLE_TYPENAME) -> int:
    """The `datatype.id` the write tier stamps on its new `property` rows.

    Read live for the same reason as `resolve_class_id`: `datatype` is part
    of 3DCityDB's shipped catalogue, and its ids are not stable across
    schema versions.
    """
    with conn.cursor() as cur:
        cur.execute(
            f"SELECT id FROM {SCHEMA}.datatype WHERE typename = %s", (typename,)
        )
        row = cur.fetchone()
    if row is None:
        raise ValueError(
            f"{SCHEMA}.datatype has no type named {typename!r}; the write "
            "tier cannot insert a property row without a datatype_id"
        )
    return int(row[0])


def sql_for(scenario: str, params: Params, window: BboxWindow | None = None,
            srid: int = 0, *,
            cityobject_class_ids: tuple[int, ...],
            building_class_id: int | None = None) -> tuple[str, tuple]:
    """`cityobject_class_ids` is keyword-only and has no default,
    deliberately: every scenario branch except `id-lookup` needs the
    CityObject-granularity predicate, and a silent default (e.g. an empty
    tuple) would risk the exact fairness defect the C1 fix corrects —
    reintroduced quietly rather than loudly. Callers must resolve it once
    via `resolve_cityobject_class_ids()` and thread the result through.

    `_static_predicate(cityobject_class_ids)` is called LAZILY, inline in
    each branch that needs it (not once, eagerly, at the top) — so a
    scenario whose own guard rejects the request first (`attr-stats` with
    no numeric column, `hierarchy` with no parent id) raises that specific,
    informative error even if `cityobject_class_ids` also happens to be
    invalid, rather than an unrelated-looking `ValueError` from a predicate
    the branch was never going to reach anyway.
    """
    p = params

    if scenario == "count":
        return f"SELECT count(*) FROM {_F} WHERE {_static_predicate(cityobject_class_ids)}", ()

    if scenario == "geometry-scan":
        # The geometry only, matching the other two systems' own
        # `geometry-scan`. The retired `full-read` also cast every
        # `property` row's whole COMPOSITE RECORD to text through two
        # `GROUP BY` CTEs over the entire tables — ~20 NULL fields per row,
        # the only CTEs in the set, and 276 s against DuckDB's 1.2 s
        # (`notes/benchmark-fairness-review-2026-09-22.md` §4.2). That was
        # largely artefact, not architecture.
        #
        # `count(DISTINCT f.id)`, not `count(*)`: one CityObject owns
        # several `geometry_data` rows (one per LoD), so a plain `count(*)`
        # over the join would report geometry rows and every row of this
        # scenario would be a false count-mismatch.
        #
        # `length(gd.geometry::text)` is a PostGIS-to-EWKT serialisation
        # (README Caveat 18), not a stored byte length.
        return (
            f"SELECT count(DISTINCT f.id), "
            f"sum(coalesce(length(gd.geometry::text), 0))::bigint "
            f"FROM {_F} f "
            f"JOIN {SCHEMA}.geometry_data gd ON gd.feature_id = f.id "
            f"WHERE {_static_predicate(cityobject_class_ids)}",
            (),
        )

    if scenario == "bbox-query":
        win = _window(window)
        return (
            f"SELECT count(*) FROM {_F} "
            f"WHERE {_static_predicate(cityobject_class_ids)} AND {CAPTURED_ENVELOPE_COLUMN} "
            "&& ST_MakeEnvelope(%s, %s, %s, %s, %s)",
            (win.minx, win.miny, win.maxx, win.maxy, srid),
        )

    if scenario == "bbox-fetch":
        # CJDB Q2's result shape on v5's schema: the Building's own
        # `objectid` and the LoD0 geometry the `property` row points at.
        # `val_lod = '0'` is the integer tier `citydb-tool` writes for
        # CityJSON's own `"0"` (docs/3dcitydb-v5-schema.md, "LoD value
        # format"), and `val_geometry_id IS NOT NULL` restricts the join to
        # the geometry-bearing property rows. The class predicate is
        # `Building` alone, not the CityObject-granularity predicate:
        # CJDB's Q2 is Building-grained and so are the other two systems'
        # rows here.
        win = _window(window)
        return (
            f"SELECT f.{CAPTURED_ID_COLUMN}, gd.geometry FROM {_F} f "
            f"JOIN {_P} pr ON pr.{CAPTURED_PROPERTY_FK} = f.id "
            f"  AND pr.val_geometry_id IS NOT NULL AND pr.val_lod = %s "
            f"JOIN {SCHEMA}.geometry_data gd ON gd.id = pr.val_geometry_id "
            f"WHERE f.{CAPTURED_CLASS_COLUMN} = %s "
            f"AND f.{CAPTURED_ENVELOPE_COLUMN} && ST_MakeEnvelope(%s, %s, %s, %s, %s)",
            (CAPTURED_LOD0_TARGET, _building(building_class_id),
             win.minx, win.miny, win.maxx, win.maxy, srid),
        )

    if scenario == "point-query":
        # CJDB Q3 on v5: the same shape as bbox-fetch against a degenerate
        # window — `&&` with a POINT, a single GIST probe.
        x, y = p.point_xy
        return (
            f"SELECT f.{CAPTURED_ID_COLUMN}, gd.geometry FROM {_F} f "
            f"JOIN {_P} pr ON pr.{CAPTURED_PROPERTY_FK} = f.id "
            f"  AND pr.val_geometry_id IS NOT NULL AND pr.val_lod = %s "
            f"JOIN {SCHEMA}.geometry_data gd ON gd.id = pr.val_geometry_id "
            f"WHERE f.{CAPTURED_CLASS_COLUMN} = %s "
            f"AND f.{CAPTURED_ENVELOPE_COLUMN} "
            "&& ST_SetSRID(ST_MakePoint(%s, %s), %s)",
            (CAPTURED_LOD0_TARGET, _building(building_class_id), x, y, srid),
        )

    if scenario == "attr-filter":
        # The per-dataset CityJSON attribute, reached through v5's EAV
        # `property` table. A string predicate lands in `val_string` and a
        # numeric bound in `val_double`/`val_int` — the importer picks the
        # column from the JSON value's own type
        # (docs/3dcitydb-v5-schema.md), which is why `attr-stats` already
        # coalesces the two. Caveat 13 applies: a CityGML-recognised
        # attribute name may have been restructured by `citydb-tool`, in
        # which case no `property` row carries it and 3DCityDB answers 0
        # against the others' count — a disclosed architectural difference,
        # not something this query special-cases.
        if p.attr_filter is None:
            raise ScenarioUnavailable("dataset has no attr-filter predicate")
        spec = p.attr_filter
        if spec.op == "eq":
            return (
                f"SELECT f.{CAPTURED_ID_COLUMN} FROM {_P} pr "
                f"JOIN {_F} f ON f.id = pr.{CAPTURED_PROPERTY_FK} "
                f"WHERE {_static_predicate(cityobject_class_ids)} "
                f"AND pr.{CAPTURED_PROPERTY_NAME_COLUMN} = %s "
                "AND pr.val_string = %s",
                (spec.column, spec.eq_value),
            )
        return (
            f"SELECT f.{CAPTURED_ID_COLUMN} FROM {_P} pr "
            f"JOIN {_F} f ON f.id = pr.{CAPTURED_PROPERTY_FK} "
            f"WHERE {_static_predicate(cityobject_class_ids)} "
            f"AND pr.{CAPTURED_PROPERTY_NAME_COLUMN} = %s "
            "AND coalesce(pr.val_double, pr.val_int) >= %s",
            (spec.column, spec.ge_bound),
        )

    if scenario == "attr-range":
        # CJDB Q1 on v5's EAV schema. `coalesce(val_double, val_int)` for
        # the same reason `attr-stats` uses it.
        if p.attr_range is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        return (
            f"SELECT f.{CAPTURED_ID_COLUMN} FROM {_P} pr "
            f"JOIN {_F} f ON f.id = pr.{CAPTURED_PROPERTY_FK} "
            f"WHERE {_static_predicate(cityobject_class_ids)} "
            f"AND pr.{CAPTURED_PROPERTY_NAME_COLUMN} = %s "
            "AND coalesce(pr.val_double, pr.val_int) > %s",
            (p.attr_range.column, p.attr_range.threshold),
        )

    if scenario == "attr-stats":
        # Mirrors `hierarchy`'s own `parent_id is None` guard below: a
        # dataset with no numeric attribute at all is a legitimate dataset
        # property (see sql_duckdb.py's equivalent guard for the two
        # heterogeneity-corpus datasets that hit this), not a query bug —
        # raised before `None` could be bound as `pr.name`'s comparison value.
        if p.numeric_column is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        # coalesce(val_double, val_int), not val_double alone: `property`
        # carries the scalar in a TYPE-SPECIFIC column (val_int/val_double/
        # val_string — see docs/3dcitydb-v5-schema.md's captured `property`
        # columns), chosen by citydb-tool's importer from the JSON value's
        # OWN type, not from `numeric_column`'s status as "the derived
        # numeric attribute" alone. `params.derive()` only checks
        # `isinstance(value, (int, float))`, so a dataset whose numeric
        # attribute happens to be JSON-integer-valued (Zurich's "Geomtype",
        # an enum-coded 1/2, not a measurement) lands entirely in val_int,
        # with val_double NULL on every one of its rows. An earlier version
        # of this query read val_double alone, which is correct for a
        # genuinely fractional attribute (delft's b3_h_dak_50p) but matches
        # ZERO rows for an integer one — verified live against Zurich:
        # `SELECT val_int, val_double, count(*) FROM property WHERE
        # name='Geomtype' GROUP BY 1,2` returns two rows (val_int
        # 1/count 53384, val_int 2/count 92478 — summing to 145862, exactly
        # the other four systems' agreeing attr-stats count) with
        # val_double NULL on both. This is a genuine harness bug, not a
        # 3DCityDB architectural property to disclose and leave: the data
        # IS there, just typed differently, and coalescing across the two
        # numeric columns is what a competent query would do for "the
        # count/min/max/sum of this attribute" regardless of which JSON
        # numeric subtype it happens to be. Contrast Vienna's
        # `measuredHeight` (README Caveat 11): there, no `property` row is
        # named `measuredHeight` AT ALL — the value is nested one level
        # down under a differently-named child row (`name='value'`) — so no
        # coalesce over sibling val_* columns of the SAME row can reach it;
        # that one stays a genuine, undoctored architectural difference.
        col = "coalesce(pr.val_double, pr.val_int)"
        return (
            f"SELECT count({col}), min({col}), "
            f"max({col}), sum({col}) "
            f"FROM {_P} pr JOIN {_F} f ON f.id = pr.{CAPTURED_PROPERTY_FK} "
            f"WHERE {_static_predicate(cityobject_class_ids)} AND pr.{CAPTURED_PROPERTY_NAME_COLUMN} = %s",
            (p.numeric_column,),
        )

    if scenario == "id-lookup":
        # No CityObject-granularity predicate: `objectid` is a unique
        # identifier and `params.target_id` always names a genuine
        # CityObject (it comes from a CityJSON file's `CityObjects` keys —
        # 3DCityDB's semantic-surface features get their own internal
        # `objectid`s, never one of these), so the row this finds is
        # already CityObject-granular by construction. count_mode is
        # "rowcount" (0 or 1), not first-column.
        return (
            f"SELECT * FROM {_F} WHERE {CAPTURED_ID_COLUMN} = %s",
            (p.target_id,),
        )

    if scenario == "lod-extract":
        # NOTE: geometry_data has NO `lod` column — verified against a live
        # instance in Task 5. In v5 the LoD is carried on the PROPERTY row
        # that points at the geometry (`property.val_lod`, alongside
        # `val_geometry_id`). Reaching for `geometry_data.lod` fails
        # outright, which is the good case; the bad case would have been a
        # column that exists but means something else.
        #
        # The target value is CAPTURED_LOD_TARGET ("1"), not the CityJSON
        # notation "1.2" — see that constant's docstring and
        # docs/3dcitydb-v5-schema.md's "LoD value format" section: v5's
        # importer truncates the fractional LoD tag, so "1.2" matches zero
        # rows, silently.
        #
        # The CityObject-granularity predicate matters here independently
        # of `count`'s reason for it: without it, this also counts each
        # LoD1 solid's *boundary surfaces'* own LoD1 geometry rows
        # (`lod1MultiSurface`, one set per WallSurface/GroundSurface/
        # RoofSurface), not just the CityObject's own LoD1 solid
        # (`lod1Solid`) — 4929 rows unrestricted, 1116 restricted, on this
        # fixture. Only the restricted count is comparable to cjdb's
        # CityObject-level "does this CityObject have an LoD1.2 geometry"
        # count.
        # Returns IDS, as CJDB Q5 does; the other two systems' own
        # `lod-extract` made the same change.
        return (
            f"SELECT f.{CAPTURED_ID_COLUMN} FROM {_P} pr "
            f"JOIN {_F} f ON f.id = pr.{CAPTURED_PROPERTY_FK} "
            f"WHERE {_static_predicate(cityobject_class_ids)} AND pr.val_lod = %s AND pr.val_geometry_id IS NOT NULL",
            (CAPTURED_LOD_TARGET,),
        )

    if scenario == "semantic-surface":
        # CityObject-granular: count of top-level CityObjects (via the
        # canonical predicate, applied to the OWNING feature) that have AT
        # LEAST ONE boundary surface classified RoofSurface — a presence
        # check, matching cjdb's and duckdb-cityparquet's own
        # semantic-surface semantics (see sql_cjdb.py / sql_duckdb.py).
        #
        # An EARLIER version of this branch counted RoofSurface `feature`
        # rows directly and deliberately WITHOUT the granularity
        # predicate, reasoned at the time as "measuring the cost of the
        # semantic surfaces themselves". That reasoning was self-
        # consistent but was never cross-checked against cjdb's/duckdb's
        # OWN semantic-surface query — both of which ask "does this
        # CityObject have >=1 RoofSurface" (presence), not "how many
        # RoofSurface features exist" (a raw count) — until this task's
        # smoke target ran all three together for the first time and
        # `run_matrix`'s cross-check caught it: 3dcitydb=2232 vs
        # cjdb=duckdb-cityparquet=1116, a genuine count-mismatch, not a
        # measurement fluke. Investigated, not assumed: 3DCityDB's
        # importer gives every BuildingPart TWO solids (`lod1Solid` —
        # collapsing CityJSON's fractional 1.2/1.3 sub-tiers into one, per
        # `CAPTURED_LOD_TARGET`'s own docstring — and `lod2Solid`), each
        # with its OWN `boundary`-linked RoofSurface feature, so every one
        # of the 1116 BuildingParts genuinely owns exactly 2 RoofSurface
        # rows (`GROUP BY pr.feature_id HAVING count(*) > 1` returns all
        # 1116, each with count 2 — confirmed against a live import, not
        # inferred). The raw count and the presence count are BOTH
        # internally correct answers to DIFFERENT questions; since cjdb
        # and duckdb-cityparquet already ask the presence question, and
        # rewriting THEM to a raw, multi-solid surface count has no
        # natural equivalent in either engine's storage model (cjdb would
        # need to unnest a JSONB array across every LoD; duckdb-cityparquet
        # stores one semantic-surfaces list per LoD column, not per
        # solid), this branch is rewritten to ask the presence question
        # instead: `property.val_feature_id` -> the RoofSurface feature,
        # `property.feature_id` -> its owner, DISTINCT-counted. Verified
        # against a live import to return exactly 1116, matching both
        # other systems.
        owner_co = _static_predicate(cityobject_class_ids, "owner")
        return (
            f"SELECT count(DISTINCT pr.feature_id) "
            f"FROM {_P} pr "
            f"JOIN {_F} rs ON rs.id = pr.val_feature_id "
            f"JOIN {_F} owner ON owner.id = pr.feature_id "
            f"WHERE rs.{CAPTURED_CLASS_COLUMN} = ("
            f"  SELECT id FROM {SCHEMA}.objectclass WHERE classname = %s) "
            f"AND {owner_co}",
            ("RoofSurface",),
        )

    if scenario == "parts-per-building":
        # CJDB Q4 on v5: one row per Building, childless ones INCLUDED, so
        # `LEFT JOIN` on both hops and no `HAVING`.
        #
        # v5 models a parent -> child association as a `property` row on the
        # PARENT carrying `val_feature_id` (the child's `feature.id`); on
        # delft those rows come in exactly two kinds, discriminated by
        # `name` — `buildingPart` (the genuine Building -> BuildingPart
        # relationship) and `boundary` (a solid -> its semantic surfaces,
        # confirmed never to originate from a Building on that fixture).
        # Rather than depend on which `name` a given importer uses, the
        # child side carries the CityObject-granularity predicate, which
        # excludes boundary surfaces by class whatever they are named. That
        # keeps this query correct on a dataset whose hierarchy-bearing
        # type is not Building/BuildingPart.
        #
        # `count(child.id)`, not `count(*)`: `count(*)` over a LEFT JOIN
        # reports 1 for a childless Building, which is exactly the
        # semantics Q4 exists to measure.
        child_co = _static_predicate(cityobject_class_ids, "child")
        return (
            f"SELECT parent.{CAPTURED_ID_COLUMN}, count(child.id) "
            f"FROM {_F} parent "
            f"LEFT JOIN {_P} pr ON pr.{CAPTURED_PROPERTY_FK} = parent.id "
            f"  AND pr.val_feature_id IS NOT NULL "
            f"LEFT JOIN {_F} child ON child.id = pr.val_feature_id AND {child_co} "
            f"WHERE parent.{CAPTURED_CLASS_COLUMN} = %s "
            f"GROUP BY parent.{CAPTURED_ID_COLUMN}",
            (_building(building_class_id),),
        )

    raise KeyError(f"unknown scenario: {scenario}")


def _window(window: BboxWindow | None) -> BBox:
    if window is None:
        raise ValueError("a windowed scenario needs its resolved BboxWindow")
    return window.window


def _building(building_class_id: int | None) -> int:
    if building_class_id is None:
        raise ValueError(
            "a Building-grained scenario needs the resolved Building "
            "objectclass_id; call resolve_class_id() once per ingest"
        )
    return building_class_id


def write_sql(scenario: str, building_class_id: int,
              datatype_id: int) -> tuple[str, tuple]:
    """CJDB's Q6/Q7/Q8 on 3DCityDB v5.

    v5 has no `cityobject_genericattrib` table: an ad-hoc attribute is an
    ordinary `property` row, so Q6 is an INSERT rather than an UPDATE.
    `datatype_id` is NOT NULL and is resolved live by `resolve_datatype_id`;
    `namespace_id` is nullable and left unset, because a generic attribute
    belongs to no CityGML namespace.

    `ST_Area(envelope)` is what the CJDB paper's own Q6 computes on
    3DCityDB, against `ST_Area(ground_geometry)` — a footprint area — on
    cjdb. The two are NOT the same quantity: an envelope is the object's
    axis-aligned bounding rectangle, so its area is an upper bound on the
    footprint's. The asymmetry is inherited from the paper deliberately and
    disclosed in the README; equalising it would measure a query CJDB never
    published.
    """
    if scenario == "attr-add":
        return (
            f"INSERT INTO {_P} (feature_id, name, datatype_id, val_double) "
            f"SELECT id, 'footprint_area', %s, ST_Area({CAPTURED_ENVELOPE_COLUMN}) "
            f"FROM {_F} WHERE {CAPTURED_CLASS_COLUMN} = %s",
            (datatype_id, building_class_id),
        )
    if scenario == "attr-update":
        return (
            f"UPDATE {_P} SET val_double = val_double + 10 "
            "WHERE name = 'footprint_area'",
            (),
        )
    if scenario == "attr-delete":
        return (
            f"DELETE FROM {_P} WHERE name = 'footprint_area'",
            (),
        )
    raise KeyError(f"unknown write scenario: {scenario}")


def write_reset_sql(scenario: str, building_class_id: int,
                    datatype_id: int) -> list[tuple[str, tuple]]:
    """The UNTIMED statements restoring the state each write scenario
    expects, so every timed sample measures the same work.

    `attr-add`'s INSERT is not idempotent — running it twice would leave two
    `footprint_area` rows per Building and make `attr-update` touch twice as
    many rows — so the reset deletes them first.
    """
    if scenario == "attr-add":
        return [(f"DELETE FROM {_P} WHERE name = 'footprint_area'", ())]
    if scenario == "attr-update":
        return []
    if scenario == "attr-delete":
        return [write_sql("attr-add", building_class_id, datatype_id)]
    raise KeyError(f"unknown write scenario: {scenario}")


def index_ddl() -> list[str]:
    """Empty: every index this benchmark's queries need already exists.

    `citydb-tool index create` was investigated as instructed
    (`podman run --rm citybench/citydb-tool help index`) rather than
    reached for hand-written DDL by default. It creates 3DCityDB's own
    fixed set of 16 "content indexes" — but `citydb-tool import cityjson`
    already creates that same set automatically as part of import:
    `SELECT count(*) FROM pg_indexes WHERE schemaname='citydb'` read 59
    immediately after import, BEFORE `index create` was ever invoked, and
    still 59 after running it — confirming `index create` is a genuine
    no-op here, not a step `CityDbSystem.ingest()` needs to call.

    Every column a scenario query above filters, joins or aggregates on is
    already covered by one of those 59 (all pre-existing, none added by
    this task — see docs/3dcitydb-v5-schema.md's captured `## Indexes`
    listing and its "Index coverage" section for the full mapping and the
    EXPLAIN evidence, both under default planner settings):

      - `feature_objectid_inx` (btree objectid) — id-lookup, and
        hierarchy's parent lookup.
      - `feature_objectclass_inx` (btree objectclass_id) — the
        CityObject-granularity predicate itself, wherever it appears,
        NOW sargable (see below — an earlier version of this docstring
        reported it permanently unreachable for `attr-stats`/`lod-extract`/
        hierarchy's child side; that was true only of the old correlated
        predicate shape, not of this column in general). Directly the
        driving `Index Cond` for `count`/`project`/`attr-filter`/
        `semantic-surface`/`attr-stats`; for `lod-extract`/hierarchy's
        child side the planner instead drives from an even more selective
        index first and applies this one as a cheap `Filter` over the
        resulting small candidate set — both are genuinely index-driven
        plans, and which one the planner picks can legitimately vary with
        data distribution/scale, unlike the old shape's permanent seq scan.
      - `feature_envelope_spx` (GIST envelope) — bbox-query.
      - `property_name_inx` (btree name) — attr-stats.
      - `property_val_geometry_fkx` / `property_val_lod_inx` (btree
        val_geometry_id / val_lod) — lod-extract's other candidate driving
        index; which of the two (or `feature_objectclass_inx`) the planner
        picks depends on the imported dataset's own row-count distribution
        across LoDs and classes, observed to differ between delft-scale and
        Zurich-scale imports — not asserted to be fixed across datasets,
        only to always be index-driven.
      - `property_feature_fkx` (btree feature_id) + `feature_pk` (btree
        id) — the rest of hierarchy's join chain.

    An EARLIER version of this docstring concluded "no faster index-driven
    plan exists for this predicate shape" and left open, unresolved,
    exactly the question that turns out to falsify that conclusion: "a
    genuinely selective plan against `feature_objectclass_inx` would need
    a differently-shaped predicate (e.g. ... a static id list resolved at
    query-build time rather than a correlated `NOT IN (subquery)`)". A
    later whole-branch review (C1) picked up precisely that thread and
    measured it. What that earlier round actually established, restated
    honestly: with `SET enable_seqscan = off`, forcing PostgreSQL to route
    the ORIGINAL `(objectclass_id IN (subquery) OR objectclass_id NOT IN
    (WITH RECURSIVE subquery))` predicate through SOME index, the planner
    falls back to a full `feature_pk` scan, not `feature_objectclass_inx`
    — because that OR-of-two-subqueries SHAPE genuinely does not reduce to
    a usable `Index Cond`, no matter how the seq scan is discouraged. That
    is a true, still-correct finding about THAT predicate shape. But it
    answers "can this specific predicate be forced onto an index?" — a
    narrower question than "does a better predicate for the same result
    exist?", and the two were conflated in the earlier docstring's closing
    claim.

    A better predicate DOES exist, and this module now uses it
    exclusively: `resolve_cityobject_class_ids()` (above) evaluates the
    identical logical predicate ONCE against `citydb.objectclass` (a
    small, dataset-independent class catalogue — 89 qualifying ids on this
    schema version) and `sql_for` renders the result as a plain
    `objectclass_id IN (1, 2, 3, ...)` via `_static_predicate()`. Static
    membership against a literal list of 89 constants IS sargable — it is
    a fundamentally different shape from a correlated `NOT IN (subquery)`,
    not a forced version of the same one — and under DEFAULT planner
    settings, no scenario seq-scans `feature` for it any more, verified
    live by `EXPLAIN` for every scenario that uses it:

    - `count`, `project`, `attr-filter`, `attr-stats`, `semantic-surface`
      (its `owner` side): the planner chooses `Index Cond:
      (objectclass_id = ANY (...))` against `feature_objectclass_inx`
      directly — the three of these (`attr-stats`, and, in the JOIN cases
      below, `lod-extract`/`hierarchy`) an earlier round of this docstring
      reported as permanently seq-scan-bound are no longer seq-scanning at
      all.
    - `lod-extract` and `hierarchy`'s child side: here the planner
      instead drives from a DIFFERENT, even more selective index first
      (`property_val_lod_inx` for `lod-extract`; `feature_objectid_inx` on
      the parent lookup, joined through, for `hierarchy`) and applies
      `objectclass_id = ANY (...)` as a plain `Filter` over the resulting
      small candidate set — genuinely the cheaper plan once the join order
      is considered, and still never a full-table scan or a
      "Filter removes 1994191 rows"-shaped Index Only Scan the way the old
      correlated predicate produced. The planner is choosing between two
      GOOD index-driven plans here, not falling back to a bad one.

    Net effect: sargability, not "always routes through
    `feature_objectclass_inx` specifically", is what the fix delivers —
    and that is what eliminates the seq scans. Measured live on Zurich
    (2,192,890 raw `feature` rows), same warm cache, `EXPLAIN (ANALYZE,
    BUFFERS)`: `count` 0.408s -> 0.013s (32.2x), `project` 0.363s ->
    0.014s (25.6x), `attr-filter` 2.74x, `bbox-25pct` 2.07x —
    independently reproduced counts identical to the pre-fix committed
    CSVs throughout. See `resolve_cityobject_class_ids`'s and `sql_for`'s
    own docstrings (above) for the full mechanism.

    Because the fix works by routing through `feature_objectclass_inx`,
    which already existed, no new index is required here — `index_ddl()`
    correctly stays empty, exactly as before the fix, just for a
    genuinely resolved reason now rather than an unresolved one. Adding a
    same-shape index under a new name would build a genuinely redundant
    index object — no query in this file would ever prefer it over
    3DCityDB's own — while still inflating `size_bytes`, the published
    storage metric this project's own format is compared against. That is
    precisely the class of error Task 8 found and fixed for cjdb (four of
    seven hand-written indexes there duplicated cjdb's own defaults); the
    fix for 3DCityDB is simpler still: add nothing.
    """
    return []
