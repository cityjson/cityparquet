"""DuckDB SQL over a CityParquet package.

Each query uses the mechanism a competent DuckDB user would reach for —
never a hand-tuned shortcut, and never a shape contrived to match another
system's plan.

`building.parquet` is the per-module object table CityParquet writes; the
adapter substitutes the real path at call time.

Written and verified against the REAL package `cityparquet convert`
produces for the delft fixture (this task's own smoke target is the first
time this module was ever run against an actual CityParquet file rather
than an assumed schema) — not against `documents/docs/03-specification/`
prose alone. Two shapes below differ from an earlier, unverified draft
because the real schema differs from what that draft assumed:

- Attribute columns are FLATTENED at the top level of the object table
  (`"b3_h_dak_max"`, ...), matching
  `03-specification/02-object-table-schema.mdx`'s "inferred typed
  attribute columns" — there is no nested `attributes` STRUCT to qualify
  through. `attr-stats` below reads the column bare.
- `parents`/`children` are `VARCHAR[]` arrays (a CityObject's own list of
  its parent ids), not a scalar `parent_id` column — `hierarchy` below
  filters with `list_contains`.
"""

from __future__ import annotations

from citybench.config import BBox, BboxWindow, Params
from citybench.scenarios.registry import ScenarioUnavailable

# The LoD columns `semantic-surface` checks — every `geometry_properties_lod*`
# column delft's CityParquet package carries. ANY of them containing a
# RoofSurface counts; see the `semantic-surface` branch below for why this
# must be "any LoD", not one specific LoD, and why the set is hardcoded to
# this dataset's own LoD tiers rather than derived generically (matching
# `lod-extract`'s own pre-existing convention of naming a specific LoD
# column rather than discovering the schema at query-build time).
_SEMANTIC_SURFACE_LOD_COLUMNS: tuple[str, ...] = (
    "geometry_properties_lod0_0",
    "geometry_properties_lod1_2",
    "geometry_properties_lod1_3",
    "geometry_properties_lod2_2",
)


#: The CityObject type `bbox-fetch`, `point-query` and `parts-per-building`
#: restrict to, matching CJDB's Q2/Q3/Q4, all of which are Building-grained
#: (`WHERE type = 'Building'`). A dataset with no Building returns no rows —
#: a real answer, the same one cjdb and 3DCityDB give it.
BUILDING_TYPE = "Building"
#: The child type `parts-per-building`'s join form counts. CityJSON's own
#: parent/child relationship for a Building.
BUILDING_PART_TYPE = "BuildingPart"
#: The LoD0 footprint column `bbox-fetch`/`point-query` return alongside the
#: id, and `attr-add` takes its area from.
FOOTPRINT_COLUMN = "geometry_lod0_0"


def geometry_byte_length(column: str, duck_type: str) -> str:
    """The stored byte length of one geometry column.

    Most `geometry_lod*` columns come back as BLOB and `octet_length` reads
    their stored WKB directly. `geometry_lod0_0` does NOT: a CityParquet
    footprint is written with Parquet's own GEOMETRY logical type, which
    DuckDB 1.5 decodes to its native `GEOMETRY` regardless of
    `enable_geoparquet_conversion` (measured — the setting governs the
    `geo`-footer path, not the logical type), and `octet_length` does not
    bind against it. `ST_AsWKB` re-encodes it, so that one column's term is
    a re-serialisation rather than a stored length. Disclosed in README
    Caveat 18 rather than papered over.
    """
    expression = column if duck_type.upper().startswith("BLOB") else f"ST_AsWKB({column})"
    return f"coalesce(octet_length({expression}), 0)"


def sql_for(scenario: str, params: Params, table: str,
            window: BboxWindow | None = None, *,
            columns: dict[str, str] | None = None) -> tuple[str, tuple]:
    """Return ``(sql, args)`` for ``scenario``. ``table`` is a read_parquet call.

    ``window`` is the resolved bbox window for a windowed scenario
    (`bbox-query`, `bbox-fetch`), derived once by `citybench.params` and
    handed to every system verbatim; `point-query` ignores it and uses
    `params.point_xy`.

    ``columns`` maps the real column names of ``table`` to their DuckDB
    types (a live schema lookup only the caller can do — ``sql_for`` itself
    never touches a database, matching this module's own docstring and test
    file). Defaults to ``None``, which keeps every branch that only needs
    delft-shaped columns working for callers that do not pass it.

    Discovered running Task 14's heterogeneity corpus: delft's LoD tiers
    (0.0/1.2/1.3/2.2) are NOT universal. Montreal's real converted package
    carries only `geometry_lod0_0`/`geometry_lod2_0` — no `1_2`/`1_3`
    column exists at all — so the OLD, hardcoded-column-name SQL below
    (`geometry_lod1_2`, `_SEMANTIC_SURFACE_LOD_COLUMNS`) raised a DuckDB
    `BinderException` outright against Montreal, rather than the `0`/`294`
    cjdb and 3DCityDB (schema-flexible JSONB/EAV storage, immune to this
    because a query for an absent LoD simply matches nothing there) both
    correctly computed for the same two scenarios. ``columns`` lets the
    caller supply the package's REAL schema so this module can build SQL
    that degrades to "no such column, so the answer is 0" instead of
    erroring, closing that gap without changing either scenario's
    definition.
    """
    p = params

    if scenario == "count":
        return f"SELECT count(*) FROM {table}", ()

    if scenario == "geometry-scan":
        # Replaces `full-read`, which was three different operations under
        # one name: 84 hashed columns here, a JSONB text serialisation on
        # cjdb, and whole-record text casts through two CTEs on 3DCityDB
        # (`notes/benchmark-fairness-review-2026-09-22.md` §4.2). All three
        # now scan the same thing — every object's geometry — and report
        # `(count, bytes)`.
        #
        # This is FAIRER but it is not NEUTRAL, and the README says so
        # (Caveat 18): DuckDB reads stored binary lengths (bar the LoD0
        # column, see `geometry_byte_length`) while both PostgreSQL sides
        # serialise to text first. A byte length also does not prove a
        # geometry was decoded. The asymmetry is disclosed, not removed.
        geometry_columns = sorted(
            c for c in (columns or {}) if c.startswith("geometry_lod")
        ) or ["geometry_lod1_2"]
        types = columns or {}
        terms = " + ".join(
            geometry_byte_length(c, types.get(c, "BLOB")) for c in geometry_columns
        )
        return (f"SELECT count(*), sum({terms})::HUGEINT FROM {table}", ())

    if scenario == "bbox-query":
        win = _window(window)
        return (
            f"SELECT count(*) FROM {table} "
            "WHERE bbox.xmax >= ? AND bbox.xmin <= ? "
            "AND bbox.ymax >= ? AND bbox.ymin <= ?",
            (win.minx, win.maxx, win.miny, win.maxy),
        )

    if scenario == "bbox-fetch":
        # CJDB Q2 proper: id plus footprint, Buildings only. CJDB's own Q2
        # is `ST_Contains(window, ground_geometry)` — containment, not the
        # overlap every system here runs. Overlap is what `bbox-query`
        # already asks and what a bbox index can answer on all three, so it
        # is kept and the difference is stated (README, "Mapping to the
        # CJDB paper") rather than the harness quietly claiming fidelity.
        win = _window(window)
        return (
            f"SELECT id, {_footprint(columns)} FROM {table} "
            "WHERE object_type = ? "
            "AND bbox.xmax >= ? AND bbox.xmin <= ? "
            "AND bbox.ymax >= ? AND bbox.ymin <= ?",
            (BUILDING_TYPE, win.minx, win.maxx, win.miny, win.maxy),
        )

    if scenario == "point-query":
        # CJDB Q3: a bbox overlap with a POINT, not a point-in-polygon test,
        # and not guaranteed to return exactly one object. The point is the
        # median row centre the windows are built around, so it lands where
        # the data is.
        x, y = p.point_xy
        return (
            f"SELECT id, {_footprint(columns)} FROM {table} "
            "WHERE object_type = ? "
            "AND bbox.xmin <= ? AND bbox.xmax >= ? "
            "AND bbox.ymin <= ? AND bbox.ymax >= ?",
            (BUILDING_TYPE, x, x, y, y),
        )

    if scenario == "attr-filter":
        # A real CityJSON ATTRIBUTE, picked per dataset exactly as the
        # format family picks it (`params.HAND_PICKED`), and returning ids
        # rather than a count.
        if p.attr_filter is None:
            raise ScenarioUnavailable("dataset has no attr-filter predicate")
        spec = p.attr_filter
        if spec.op == "eq":
            return (
                f'SELECT id FROM {table} WHERE "{spec.column}" = ?',
                (spec.eq_value,),
            )
        return (
            f'SELECT id FROM {table} WHERE "{spec.column}" >= ?',
            (spec.ge_bound,),
        )

    if scenario == "attr-range":
        # CJDB Q1. A typed DOUBLE column with row-group statistics against
        # a JSONB cast and an EAV join: an expected advantage, stated in
        # the README rather than presented as a surprise.
        if p.attr_range is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        return (
            f'SELECT id FROM {table} WHERE "{p.attr_range.column}" > ?',
            (p.attr_range.threshold,),
        )

    if scenario == "attr-stats":
        # Mirrors `hierarchy`'s own `parent_id is None` guard below: a
        # dataset with no numeric attribute at all (Montreal's 294
        # attribute-less Buildings; lod3_railway's categorical-only
        # "function"/"class"/"species") is a legitimate dataset property,
        # not a query bug, so this is raised before any SQL references
        # `None` as a column name.
        if p.numeric_column is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        # Bare column reference, NOT `attributes."{col}"`: attribute
        # columns are flattened at the top level of the object table (see
        # this module's docstring) — there is no `attributes` struct to
        # qualify through. Confirmed against the real package: `attributes`
        # does not resolve as a table/struct alias at all
        # (`Binder Error: Referenced table "attributes" not found!`).
        col = f'"{p.numeric_column}"'
        # count first, per the registry's first-column convention.
        return (
            f"SELECT count({col}), min({col}), max({col}), sum({col}) FROM {table}",
            (),
        )

    if scenario == "id-lookup":
        return f"SELECT * FROM {table} WHERE id = ?", (p.target_id,)

    if scenario == "lod-extract":
        # Only the LoD1.2 geometry column is projected; the LoD2 column's
        # bytes are never read. This is the projection-pushdown claim.
        #
        # A dataset that never carries an LoD1.2 geometry at all (e.g.
        # Montreal: geometry_lod0_0/geometry_lod2_0 only, no lod1_2
        # column) has no `geometry_lod1_2` column to project in the first
        # place. cjdb's/3dcitydb's own `lod-extract` SQL is a FIXED
        # question ("count of objects carrying an LoD1.2 geometry",
        # hardcoded "1.2"/"1" respectively — see sql_cjdb.py/sql_citydb.py)
        # that still runs, correctly returning 0, against such a dataset
        # (schema-flexible JSONB/EAV storage tolerates a filter that
        # matches nothing). The comparable DuckDB answer when the column
        # is absent is therefore also 0 — real objects, zero of which
        # carry a geometry in a column that does not exist — not an
        # error and not a skip.
        #
        # I2 (final whole-branch review): `WHERE FALSE` is constant-folded
        # by DuckDB at plan time -- this branch performs NO scan at all on
        # a dataset that hits it, unlike cjdb's/3dcitydb's own unconditional
        # SQL, which genuinely executes and happens to match zero rows. On
        # this corpus that is 4 of 5 datasets (only `delft` carries
        # `geometry_lod1_2`), so the published `duckdb-cityparquet`
        # `lod-extract` timing on those four rows is not a measurement of
        # projection pushdown or of anything else -- see README Caveat 17.
        if columns is not None and "geometry_lod1_2" not in columns:
            return f"SELECT id FROM {table} WHERE FALSE", ()
        # Returns IDS, as CJDB Q5 does — not `count(col)`, which on a
        # Parquet reader is answerable from the definition levels alone and
        # so measured almost nothing.
        return (
            f"SELECT id FROM {table} WHERE geometry_lod1_2 IS NOT NULL",
            (),
        )

    if scenario == "semantic-surface":
        # `geometry_properties_lod*.surfaces` is a JSON-encoded VARCHAR (a
        # list of `{"type": ..., ...}` objects — the per-face-group
        # semantic surfaces, one entry per distinct semantic surface, not
        # one per geometry face), NOT a nested LIST<STRUCT> — confirmed
        # against the real package: `surfaces.type` fails to bind
        # ("Cannot extract field 'type' ... because it is not a struct").
        # `json_extract_string(..., '$[*].type')` pulls every element's
        # `type` field out as a `VARCHAR[]`.
        #
        # ANY-LoD, deliberately, not "LoD2.2 only" (an earlier version of
        # this branch WAS LoD2.2-only, and it was a real bug: caught by
        # review, not by the cross-system count-check, because on delft
        # every BuildingPart with a RoofSurface at LoD2.2 also happens to
        # have one at every other LoD — the check's silence was a property
        # of this fixture, not proof of correctness). This branch now
        # checks every LoD column CityParquet wrote for this dataset
        # (`geometry_properties_lod0_0`/`lod1_2`/`lod1_3`/`lod2_2`; LoD0's
        # own `surfaces` is always NULL in practice — a footprint carries
        # no semantic classification — but it costs nothing to include and
        # keeps the definition literally "any LoD", not "any LoD that
        # usually has semantics"), OR'd together.
        #
        # This is the SAME question cjdb's `semantic-surface` already asks
        # (its jsonpath `$[*].semantics...` iterates cjdb's own `geometry`
        # JSONB array across every LoD, unconditionally — verified against
        # a live import: cjdb stores all three of delft's LoDs per object)
        # and the same question `sql_citydb.py`'s (Task 12-fixed)
        # `semantic-surface` asks.
        #
        # An EARLIER version of this comment claimed any-LoD was 3DCityDB's
        # ONLY implementable option — reasoned from the `boundary`-linked
        # `property` row alone (owned by the Solid, `parent_id IS NULL`,
        # no `val_lod`). That was an overclaim, caught by review: it never
        # examined `lod1MultiSurface`/`lod2MultiSurface` — separate
        # `property` rows owned DIRECTLY by the boundary-surface feature
        # itself (`property.feature_id = <the RoofSurface's own id>`),
        # which DO carry `val_lod` (already documented, unconnected to this
        # question at the time, in `docs/3dcitydb-v5-schema.md`'s "LoD
        # value format" section and in `sql_citydb.py`'s own `lod-extract`
        # comment). Confirmed live: every RoofSurface feature owns exactly
        # one `lod1MultiSurface` row (`val_lod='1'`) and one
        # `lod2MultiSurface` row (`val_lod='2'`) — 1116 of each on delft.
        # A LoD-scoped query IS expressible —
        # `JOIN property lod_pr ON lod_pr.feature_id = rs.id AND
        # lod_pr.val_lod = ?` — and was written and run: it returns 1116
        # for LoD1 and 1116 for LoD2, both sensible.
        #
        # So any-LoD here is a DELIBERATE CHOICE, not a forced one. Two
        # reasons, in order of how much weight they carry: (1) it is the
        # more natural, general question a benchmark scenario named
        # "semantic-surface" should ask — "does this object have a roof
        # surface classified at all", independent of which LoD tier
        # happens to carry that classification — rather than requiring
        # every system to agree on picking one specific tier first, which
        # is itself an arbitrary decision a real query author would rarely
        # need to make; (2) picking one specific LoD to scope to would mean
        # picking WHICH LoD, and any such pick risks privileging whichever
        # tier each system's own storage model happens to represent most
        # naturally or richly — a self-serving choice to make in a
        # benchmark where one of the participating systems is this
        # project's own format. Any-LoD sidesteps the question entirely.
        # Worth being honest about the limit of this finding too: on delft
        # specifically, the choice does not even move a published number —
        # a LoD1-scoped or LoD2-scoped query returns the same 1116 as the
        # any-LoD query, since every BuildingPart here has a RoofSurface at
        # every LoD it stores. The reasoning above is about which QUESTION
        # this scenario states it is asking, not about a number this
        # fixture could have caught being wrong.
        #
        # Pinned by test_sql_duckdb.py (every LoD column referenced) and by
        # test_semantic_surface_lod_scope.py, which proves the point with
        # data: a fixture object carrying a RoofSurface at LoD1.2 only (no
        # semantics at all at LoD2.2) — a LoD2.2-only query returns 0 for
        # it (a false negative against the "does it have a roof at all"
        # question this scenario deliberately asks instead), the any-LoD
        # query returns 1, matching cjdb's own query against the same
        # fixture. (3DCityDB was not run against this fixture — a separate,
        # disclosed infrastructure constraint, not evidence for or against
        # this section's claim.)
        #
        # `_SEMANTIC_SURFACE_LOD_COLUMNS` is delft's own LoD tier set
        # (0.0/1.2/1.3/2.2), hardcoded — a real bug against any dataset
        # whose LoD tiers differ (Montreal: only lod0_0/lod2_0 exist;
        # referencing `geometry_properties_lod1_2` raised a DuckDB
        # `BinderException` outright, discovered running Task 14's
        # heterogeneity corpus). When the caller supplies the package's
        # real ``columns``, this branch instead ORs across every
        # `geometry_properties_lod*` column the package ACTUALLY has —
        # still "any LoD", now genuinely dataset-agnostic rather than
        # delft-shaped. Falls back to the old hardcoded list when
        # ``columns`` is omitted, so every existing caller/test that never
        # passes it is unaffected.
        lod_cols = (
            sorted(c for c in columns if c.startswith("geometry_properties_lod"))
            if columns is not None else list(_SEMANTIC_SURFACE_LOD_COLUMNS)
        )
        if not lod_cols:
            # No geometry_properties_lod* column exists at all: no object
            # can carry a RoofSurface classification anywhere, so the
            # correct answer is 0 — a real (if degenerate) query against
            # `table`, not an error.
            return f"SELECT count(*) FROM {table} WHERE FALSE", ()
        return (
            f"SELECT count(*) FROM {table} WHERE "
            + " OR ".join(
                "list_contains(json_extract_string("
                f"{col}.surfaces, '$[*].type'), 'RoofSurface')"
                for col in lod_cols
            ),
            (),
        )

    if scenario == "parts-per-building":
        # CJDB Q4, in the shape CityParquet's own storage makes natural:
        # the child list is a `VARCHAR[]` on the row, so the answer is a
        # column read with no join at all.
        #
        # `coalesce(len(children), 0)`, not `len(children)`: a Building
        # with no parts has a NULL `children` array, and Q4's whole point
        # (Codex's correction to the mapping) is that its LEFT JOIN KEEPS
        # childless Buildings — reporting NULL for them here while the join
        # form below reports 0 would make the two forms disagree on values
        # while agreeing on rows.
        return (
            f"SELECT id, coalesce(len(children), 0) FROM {table} "
            "WHERE object_type = ?",
            (BUILDING_TYPE,),
        )

    if scenario == "parts-per-building-join":
        # The SAME question asked the way a normalised store must ask it —
        # published beside the natural form so the cost of the join is
        # visible rather than asserted. `LEFT JOIN`, so childless Buildings
        # stay in the result and the two forms return identical row sets.
        return (
            f"SELECT b.id, count(p.id) FROM {table} b "
            f"LEFT JOIN (SELECT unnest(parents) AS parent, id FROM {table} "
            "WHERE object_type = ?) p ON p.parent = b.id "
            "WHERE b.object_type = ? GROUP BY b.id",
            (BUILDING_PART_TYPE, BUILDING_TYPE),
        )

    raise KeyError(f"unknown scenario: {scenario}")


def _window(window: BboxWindow | None) -> BBox:
    if window is None:
        raise ValueError("a windowed scenario needs its resolved BboxWindow")
    return window.window


def _footprint(columns: dict[str, str] | None) -> str:
    """The footprint expression `bbox-fetch`/`point-query` return.

    A package with no LoD0 column at all (Montreal carries `lod0_0` and
    `lod2_0`, but a by-type package need not) returns a NULL footprint
    rather than failing to bind — the same degradation `lod-extract`
    already makes, and recorded by README Caveat 15.
    """
    if columns is not None and FOOTPRINT_COLUMN not in columns:
        return "NULL"
    return FOOTPRINT_COLUMN


def write_statements(scenario: str, schema: str, footprint_area: str
                     ) -> list[tuple[str, tuple]]:
    """The TIMED statements of one write scenario, against the package
    schema `cityparquet_read` loaded (`<schema>.building`, ...).

    CityParquet has no in-place update path of its own — a Parquet file's
    smallest rewritable unit is a column chunk — so the comparable operation
    is the one the DuckDB extension's package model offers: load the package
    into DuckDB tables (untimed), mutate them, and write the package back.
    `PRAGMA cityparquet_read` is the load; `cityparquet_write` is the
    write-back, and it is timed only on the `-writeback` system tag so the
    reader sees the in-engine cost and the file cost separately.

    `footprint_area` is the expression the caller resolved for the object's
    plan area — `ST_Area(geometry_lod0_0)` where DuckDB's spatial extension
    and the package's LoD0 column are both available, and the `bbox` plan
    area otherwise. Which one was used is stamped into the row's `notes`,
    because they are not the same quantity.

    There is deliberately no `cityparquet_reconcile` call: an attribute edit
    touches no derived state (`lib/duckdb-cityjson/docs/FUNCTIONS.md`,
    "Mutation" — "Attribute edits are ordinary `UPDATE` and need no
    wrapper"), so reconciling would time work the operation does not need.
    """
    table = f"{schema}.building"
    if scenario == "attr-add":
        # CJDB Q6. Two statements, both timed: the column must exist before
        # it can be filled, and cjdb's own Q6 likewise adds the key and its
        # value in one `jsonb_set`.
        return [
            (f"ALTER TABLE {table} ADD COLUMN footprint_area DOUBLE", ()),
            (f"UPDATE {table} SET footprint_area = {footprint_area} "
             f"WHERE object_type = '{BUILDING_TYPE}'", ()),
        ]
    if scenario == "attr-update":
        # CJDB Q7: `+ 10.0` over every row carrying the attribute.
        return [
            (f"UPDATE {table} SET footprint_area = footprint_area + 10.0 "
             f"WHERE object_type = '{BUILDING_TYPE}'", ()),
        ]
    if scenario == "attr-delete":
        # CJDB Q8. DuckDB's `DROP COLUMN` reports no rowcount, so the row's
        # `result_count` is defined as the Building count the other two
        # systems' `DELETE`/`jsonb_set_lax` touch — a DEFINITION, stated in
        # the README, not a measurement.
        return [(f"ALTER TABLE {table} DROP COLUMN footprint_area", ())]
    raise KeyError(f"unknown write scenario: {scenario}")


def write_reset_statements(scenario: str, schema: str, footprint_area: str
                           ) -> list[tuple[str, tuple]]:
    """The UNTIMED statements that put the schema back into the state
    `scenario` expects, so a second timed sample measures the same work as
    the first.

    Without this, `repeat` > 1 would measure something different on every
    sample: `ALTER TABLE ... ADD COLUMN` errors the second time, and
    `DROP COLUMN` has nothing left to drop.
    """
    table = f"{schema}.building"
    if scenario == "attr-add":
        return [(f"ALTER TABLE {table} DROP COLUMN IF EXISTS footprint_area", ())]
    if scenario == "attr-update":
        return []          # an increment is repeatable as it stands
    if scenario == "attr-delete":
        return [
            (f"ALTER TABLE {table} ADD COLUMN IF NOT EXISTS footprint_area DOUBLE", ()),
            (f"UPDATE {table} SET footprint_area = {footprint_area} "
             f"WHERE object_type = '{BUILDING_TYPE}'", ()),
        ]
    raise KeyError(f"unknown write scenario: {scenario}")
