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

from citybench.config import Params
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


def sql_for(scenario: str, params: Params, table: str,
            selectivity: float | None = None, *,
            columns: frozenset[str] | None = None) -> tuple[str, tuple]:
    """Return ``(sql, args)`` for ``scenario``. ``table`` is a read_parquet call.

    ``columns`` is the real column-name set of ``table`` (a live schema
    lookup only the caller can do — ``sql_for`` itself never touches a
    database, matching this module's own docstring and test file), used
    ONLY by ``lod-extract``/``semantic-surface`` below. Defaults to
    ``None``, which keeps this function's behaviour exactly as before for
    every existing caller (delft's own package, and this module's tests)
    that does not pass it.

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

    if scenario == "full-read":
        # count(*) first (the comparable quantity), then a checksum over
        # every column to force a full decode — matching the existing
        # harness's duckdb-parquet baseline rather than counting rows only.
        #
        # `COLUMNS(*)` is a star expression: DuckDB expands
        # `sum(hash(COLUMNS(*)))::HUGEINT` into one
        # `sum(hash(<col>))::HUGEINT` PER COLUMN, not one combined hash —
        # confirmed empirically (77 columns in delft's schema -> a 78-item
        # row: count(*) plus one sum-of-hashes per column). That is a
        # STRONGER full-decode guarantee than a single merged checksum
        # would give, so it is kept rather than collapsed to one value.
        # `::HUGEINT`, not `::BIGINT`: verified against the real package
        # that a per-column `sum(hash(...))` — hash() returns a 64-bit
        # value, but DuckDB's SUM accumulates in 128 bits internally to
        # avoid overflow — genuinely exceeds INT64 range on this table
        # (`Conversion Error: ... value ... can't be cast ... to BIGINT`
        # on delft's `id` column alone). HUGEINT is wide enough for the
        # accumulator DuckDB already uses internally, so the cast becomes
        # a no-op rather than a lossy narrowing.
        return (
            f"SELECT count(*), sum(hash(COLUMNS(*)))::HUGEINT FROM {table}",
            (),
        )

    if scenario == "bbox-query":
        win = p.bbox_full.window(selectivity)
        return (
            f"SELECT count(*) FROM {table} "
            "WHERE bbox.xmax >= ? AND bbox.xmin <= ? "
            "AND bbox.ymax >= ? AND bbox.ymin <= ?",
            (win.minx, win.maxx, win.miny, win.maxy),
        )

    if scenario == "attr-filter":
        return f"SELECT count(*) FROM {table} WHERE object_type = ?", (p.attr_eq,)

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

    if scenario == "project":
        return f"SELECT count(object_type) FROM {table}", ()

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
            return f"SELECT count(*) FROM {table} WHERE FALSE", ()
        return (
            f"SELECT count(geometry_lod1_2) FROM {table} "
            "WHERE geometry_lod1_2 IS NOT NULL",
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

    if scenario == "hierarchy":
        if p.parent_id is None:
            raise ScenarioUnavailable("dataset has no parent/child hierarchy")
        # `parents` is a `VARCHAR[]` (a CityObject's own list of its
        # parent ids), not a scalar `parent_id` column — confirmed against
        # the real package ("Referenced column 'parent_id' not found").
        # `list_contains` counts the CHILDREN of `p.parent_id`: every row
        # whose own `parents` array names it, mirroring cjdb's
        # `city_object_relationships` join and 3DCityDB's
        # `val_feature_id` join, both of which answer the same "how many
        # children does this parent have" question.
        return (
            f"SELECT count(*) FROM {table} WHERE list_contains(parents, ?)",
            (p.parent_id,),
        )

    raise KeyError(f"unknown scenario: {scenario}")
