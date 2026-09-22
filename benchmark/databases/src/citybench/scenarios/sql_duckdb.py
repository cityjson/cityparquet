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
  its parent ids), not a scalar `parent_id` column — `parts-per-building`
  below reads the `children` array directly.
"""

from __future__ import annotations

from citybench.config import AppendSpec, BBox, BboxWindow, IdProbe, Params
from citybench.scenarios.registry import ScenarioUnavailable

#: The CityObject type `parts-per-building` and the write tier restrict to,
#: matching CJDB's Q4/Q6-Q8, all of which are Building-grained (`WHERE type
#: = 'Building'`). A dataset with no Building returns no rows — a real
#: answer, the same one cjdb and 3DCityDB give it.
BUILDING_TYPE = "Building"
#: The child type `parts-per-building`'s join form counts. CityJSON's own
#: parent/child relationship for a Building.
BUILDING_PART_TYPE = "BuildingPart"
#: The LoD0 footprint column `attr-add` takes its area from.
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
            probe: IdProbe | None = None,
            columns: dict[str, str] | None = None) -> tuple[str, tuple]:
    """Return ``(sql, args)`` for ``scenario``. ``table`` is a read_parquet call.

    ``window`` is the resolved bbox window for `bbox-query`, derived once by
    `citybench.params` and handed to every system verbatim. ``probe`` is
    the resolved `id-lookup` target, one of the four the runner expands that
    scenario into; both are supplied by the runner, never chosen here.

    ``columns`` maps the real column names of ``table`` to their DuckDB
    types (a live schema lookup only the caller can do — ``sql_for`` itself
    never touches a database, matching this module's own docstring and test
    file). Defaults to ``None``, which keeps every branch that only needs
    delft-shaped columns working for callers that do not pass it.

    Discovered running Task 14's heterogeneity corpus: delft's LoD tiers
    (0.0/1.2/1.3/2.2) are NOT universal. Montreal's real converted package
    carries only `geometry_lod0_0`/`geometry_lod2_0` — no `1_2`/`1_3`
    column exists at all — so hardcoded-column-name SQL (`geometry_lod1_2`)
    raised a DuckDB `BinderException` outright against Montreal, rather
    than the `0` cjdb and 3DCityDB (schema-flexible JSONB/EAV storage,
    immune to this because a query for an absent LoD simply matches nothing
    there) both correctly computed for `lod-query`. ``columns`` lets the
    caller supply the package's REAL schema so this module can build SQL
    that degrades to "no such column, so the answer is 0 rows" instead of
    erroring, closing that gap without changing the scenario's definition.
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
        # Mirrors the guards the other two modules apply: a
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
        # One of the four probes the runner expands this scenario into: the
        # ids at 10 %, 50 % and 90 % of the canonical stream order, plus one
        # verified absent. `SELECT *` is the whole object row, geometry
        # included, materialised inside the timed window.
        return f"SELECT * FROM {table} WHERE id = ?", (_probe(probe).id,)

    if scenario == "lod-query":
        # "Retrieve all buildings having a specific LoD geometry"
        # (`notes/benchmark-queries.md` B12, CJDB's Q5). WHOLE ROWS, not
        # ids: `SELECT *` hands back the object — every column, every LoD
        # geometry among them — which is what a client asking for "the
        # buildings with an LoD1.2 geometry" receives. Returning ids alone
        # would let a Parquet reader answer from one column's definition
        # levels and measure almost nothing.
        #
        # The rows are fetched to Arrow INSIDE the timed window, as every
        # other row-returning scenario here is (see `DuckDBCityParquet.run`)
        # so no engine wins by handing back a lazy cursor.
        #
        # A dataset that never carries an LoD1.2 geometry at all (e.g.
        # Montreal: geometry_lod0_0/geometry_lod2_0 only, no lod1_2
        # column) has no `geometry_lod1_2` column to filter on in the first
        # place. cjdb's/3dcitydb's own `lod-query` SQL is a FIXED question
        # ("the objects carrying an LoD1.2 geometry", hardcoded "1.2"/"1"
        # respectively — see sql_cjdb.py/sql_citydb.py) that still runs,
        # correctly returning 0 rows, against such a dataset
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
        # `lod-query` timing on those four rows is not a measurement of
        # anything -- see README Caveat 15.
        if columns is not None and "geometry_lod1_2" not in columns:
            return f"SELECT * FROM {table} WHERE FALSE", ()
        return (
            f"SELECT * FROM {table} WHERE geometry_lod1_2 IS NOT NULL",
            (),
        )

    if scenario == "parts-per-building":
        # CJDB Q4 and catalogue B11 — "for each building, how many parts
        # does it have? CityParquet: the length of each Building row's
        # `children` list". The NATURAL form is the measured row: the child
        # list is a `VARCHAR[]` on the row, so the answer is a column read
        # with no join at all. The join form below is published beside it as
        # a labelled control, never as the CityParquet number.
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


def _probe(probe: IdProbe | None) -> IdProbe:
    if probe is None:
        raise ValueError("id-lookup needs its resolved IdProbe")
    return probe


def _append(append: AppendSpec | None) -> AppendSpec:
    if append is None:
        raise ScenarioUnavailable(
            "no one-feature append file was derived for this dataset"
        )
    return append


def write_statements(scenario: str, schema: str, footprint_area: str,
                     append: AppendSpec | None = None
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

    `append` is the derived one-feature CityJSONSeq file `append-object`
    imports; `ScenarioUnavailable` when the dataset has none.

    There is deliberately no `cityparquet_reconcile` call on the three
    attribute scenarios: an attribute edit touches no derived state
    (`lib/duckdb-cityjson/docs/FUNCTIONS.md`, "Mutation" — "Attribute edits
    are ordinary `UPDATE` and need no wrapper"), so reconciling would time
    work the operation does not need. `insert_cityjsonseq` needs no
    reconcile call either, for the opposite reason: it re-derives
    `feature_id`, the reciprocal hierarchy and `bbox` itself, inside the one
    call, which is part of what the row is measuring.
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
    if scenario == "append-object":
        # Catalogue B18: "add one new building, with its parts and
        # geometry, to the dataset", through the extension's own importer
        # rather than a hand-written INSERT. One call routes every object of
        # the file to its module table and re-derives `feature_id`, the
        # reciprocal hierarchy and `bbox` afterwards
        # (`lib/duckdb-cityjson/docs/FUNCTIONS.md`, "Adding a CityJSON
        # file") — exactly the index/derived-state maintenance the other two
        # systems' importers also do, differently, and the point of the row.
        path = _append(append).path.replace("'", "''")
        return [(f"PRAGMA insert_cityjsonseq('{schema}', '{path}')", ())]
    raise KeyError(f"unknown write scenario: {scenario}")


def write_reset_statements(scenario: str, schema: str, footprint_area: str,
                           append: AppendSpec | None = None
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
    if scenario == "append-object":
        # The appended objects are removed again, UNTIMED, so every sample
        # inserts into the same state — and because "an incoming id already
        # in the destination refuses the entire insert" (`FUNCTIONS.md`,
        # "Adding a CityJSON file"), a second sample would otherwise fail
        # rather than measure. Every id in the appended file carries the
        # suffix, so the predicate names exactly them and nothing else;
        # `cascade` is left at its default and walks `children`, which are
        # suffixed too. The predicate is a SQL fragment inside a SQL
        # string, so its own quotes are doubled twice.
        suffix = _append(append).suffix.replace("'", "''''")
        return [
            (f"PRAGMA cityparquet_delete('{schema}', "
             f"'id LIKE ''%{suffix}''')", ()),
        ]
    raise KeyError(f"unknown write scenario: {scenario}")
