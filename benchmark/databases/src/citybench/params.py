"""Derive query parameters once, from the CityJSON source.

Every system under test is then handed these values verbatim. This is the
mechanism that makes the comparison honest: the systems are provably being
asked the same question, rather than each deriving its own idea of "a 5%
window" or "a typical building".

Derivation is deterministic — ties are broken by sorting — so the committed
params file is reproducible from the committed input.
"""

from __future__ import annotations

import dataclasses
import json
from collections import Counter
from pathlib import Path
from typing import Any, Iterator

import duckdb

from citybench.config import (
    BBOX_TARGETS, AttrFilter, AttrRange, BBox, BboxWindow, Params,
    object_table_files, window_from_halves,
)

# --- the `attr-filter` predicate, shared with the format harness ---------
#
# Ported verbatim from `benchmark/readbench/src/params.rs` (`HAND_PICKED`,
# `fallback_attr_filter`) so that a `attr-filter` row in either benchmark
# family asks the same question of the same dataset. Before this port the
# database family filtered on `object_type` — a reserved structural column,
# not a CityJSON attribute — which is both an unnatural query and the exact
# predicate that made every FlatCityBuf row in the format family fall back to
# a full walk (`notes/benchmark-fairness-review-2026-09-22.md` §0).
#
# Each entry names a column that is a member of the CityJSON `attributes`
# map and that every format carries. `family` matches any dataset name with
# that prefix (the 3DBAG scaling slices are prefixes of one stream and so
# share their attributes); `dataset` matches that exact name or a
# `-<suffix>` ordering variant of it (`rotterdam_delfshaven-hilbert`).
HAND_PICKED: tuple[tuple[str, str, str, tuple[str, Any]], ...] = (
    # The natural roof-type query; 3DBAG carries it on the Building, one
    # per feature.
    ("family", "3dbag_", "b3_dak_type", ("eq", "slanted")),
    ("dataset", "zurich_building_lod2", "class", ("eq", "BB01")),
    ("dataset", "vienna_102081", "roofType", ("eq", "FLACHDACH")),
    ("dataset", "ingolstadt", "klumMaterialClass", ("eq", "Wood")),
    # NYC's only categorical attributes are identifiers; `1000000` is the
    # placeholder BIN, a legitimate low-selectivity equality.
    ("dataset", "nyc_da13_buildings", "BIN", ("eq", "1000000")),
    # Rotterdam's string attributes are constant, so a numeric range is the
    # only selective predicate it has.
    ("dataset", "rotterdam_delfshaven", "TerrainHeight", ("quantile", 0.75)),
)

#: The share of rows the fallback rule's string branch aims a predicate at —
#: selective enough that an index can help, common enough that the result is
#: not a rounding error. `params.rs::FALLBACK_TARGET_SHARE`.
FALLBACK_TARGET_SHARE = 0.25
#: The quantile the fallback's numeric branch (and Rotterdam's hand-picked
#: entry) thresholds at. `params.rs::FALLBACK_QUANTILE`.
FALLBACK_QUANTILE = 0.75
#: A string attribute needs at least this many distinct values to be a
#: candidate, and at most this many before it is an identifier rather than a
#: category. `params.rs::FALLBACK_MIN_DISTINCT` / `_MAX_DISTINCT`.
FALLBACK_MIN_DISTINCT = 2
FALLBACK_MAX_DISTINCT = 1000

#: `attr-range` (CJDB Q1) prefers this column where the dataset has it —
#: CJDB's own Q1 is `b3_h_dak_max > 20` — and falls back to the dataset's
#: `numeric_column` otherwise.
ATTR_RANGE_PREFERRED_COLUMN = "b3_h_dak_max"
#: The quantile `attr-range`'s threshold is taken at. CJDB's literal 20 on
#: 3DBAG selects 20.5 % of rows; the 0.8 quantile reproduces that
#: selectivity without hard-coding a value that means nothing elsewhere.
ATTR_RANGE_QUANTILE = 0.8

#: Characters that would corrupt the results CSV's `notes` column if they
#: appeared in a chosen value. `params.rs::NOTES_HOSTILE`.
NOTES_HOSTILE = (";", ",", '"', "\n", "\r")


def _iter_json_lines(source: Path) -> Iterator[dict[str, Any]]:
    """Yield each parsed JSON object from a CityJSON or CityJSONSeq file.

    A plain CityJSON file is a single JSON document (possibly pretty-printed
    across many lines), so the whole file is tried as one object first. If
    that fails to parse — because the file is actually CityJSONSeq, one
    JSON object per line (a header line followed by feature lines) — fall
    back to reading it line by line.
    """
    text = source.read_text()
    try:
        yield json.loads(text)
        return
    except json.JSONDecodeError:
        pass  # not a single JSON document: fall through to line-delimited

    for line in text.splitlines():
        line = line.strip()
        if line:
            yield json.loads(line)


def source_facts(source: Path) -> tuple[int, str, str | None]:
    """``(total CityObjects, lexicographically first id, numeric column)``.

    These three stay source-derived rather than package-derived so that the
    selectivity denominator and `id-lookup`'s probe describe the CityJSON
    the PostgreSQL systems were actually fed, not the converted artefact
    only DuckDB reads. The extent is no longer derived here: the query
    windows are searched over the package's own per-row `bbox` column, as
    the format harness does, so dequantising every vertex of a multi-
    gigabyte source in Python bought nothing and cost minutes.
    """
    numeric_counts: Counter[str] = Counter()
    all_ids: list[str] = []
    total_objects = 0

    for doc in _iter_json_lines(source):
        for obj_id, obj in doc.get("CityObjects", {}).items():
            total_objects += 1
            all_ids.append(obj_id)
            for key, value in (obj.get("attributes") or {}).items():
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    numeric_counts[key] += 1

    if total_objects == 0:
        raise ValueError(f"{source}: no CityObjects found")

    # Ties broken by name so the result is deterministic.
    #
    # An earlier version of this function raised ValueError when a dataset
    # carried no numeric attribute at all. Discovered to be too broad
    # against the heterogeneity corpus (Task 14): Montreal's 294 Buildings
    # carry NO "attributes" object whatsoever, and lod3_railway's 121
    # CityObjects across 14 CityGML types carry only categorical attributes
    # ("function"/"class"/"species") — genuine, legitimate properties of
    # those datasets, not malformed input. The old raise aborted derivation
    # of EVERY scenario's parameters for the sake of the one scenario
    # (attr-stats) that actually needs a numeric column. So `numeric_column`
    # is simply None, and only attr-stats (guarded by a
    # `ScenarioUnavailable` in each `sql_*.sql_for`) is affected, recorded
    # downstream as a `skipped:` row.
    numeric_column = (
        min(numeric_counts.items(), key=lambda kv: (-kv[1], kv[0]))[0]
        if numeric_counts else None
    )
    return total_objects, sorted(all_ids)[0], numeric_column


def package_attributes(files: list[str], conn: duckdb.DuckDBPyConnection) -> list[str]:
    """Every CityJSON ATTRIBUTE name the package's object tables declare.

    Read from each Parquet file's own `city` footer (`attributes`), the same
    list `params.rs` reads through `CityMetadata`, rather than inferred by
    subtracting a hard-coded set of structural column names. A structural
    column (`object_type`, `bbox`, `parents`, ...) is therefore never
    eligible as an `attr-filter` predicate, which is the whole point of the
    port — see this module's `HAND_PICKED` note.
    """
    names: list[str] = []
    for path in files:
        rows = conn.execute(
            "SELECT value FROM parquet_kv_metadata(?) WHERE key = 'city'", [path]
        ).fetchall()
        for (value,) in rows:
            if isinstance(value, (bytes, bytearray)):
                value = value.decode()
            for name in json.loads(value).get("attributes", []):
                if name not in names:
                    names.append(name)
    return sorted(names)


def _column_types(table: str, conn: duckdb.DuckDBPyConnection) -> dict[str, str]:
    rows = conn.execute(f"DESCRIBE SELECT * FROM {table}").fetchall()
    return {row[0]: row[1] for row in rows}


def _is_string(duck_type: str) -> bool:
    return duck_type == "VARCHAR" or duck_type.startswith("ENUM")


def _is_numeric(duck_type: str) -> bool:
    return duck_type in {"BIGINT", "INTEGER", "DOUBLE", "FLOAT", "SMALLINT", "TINYINT"}


def _quantile(table: str, column: str, quantile: float,
              conn: duckdb.DuckDBPyConnection) -> float | None:
    """The column's own `quantile`, by linear interpolation over its
    non-NULL values — `quantile_cont`, which is exactly the definition
    `params.rs::quantile_of` implements, so both families threshold at the
    same value."""
    row = conn.execute(
        f'SELECT quantile_cont("{column}", {quantile}) FROM {table}'
    ).fetchone()
    return None if row is None or row[0] is None else float(row[0])


def resolve_attr_filter(dataset: str, table: str, attributes: list[str],
                        types: dict[str, str],
                        conn: duckdb.DuckDBPyConnection) -> AttrFilter | None:
    """The `attr-filter` predicate for ``dataset``.

    The `HAND_PICKED` entry when there is one and the package actually
    carries a column of that name and type; otherwise the derived rule (the
    string attribute whose most frequent value's share lands closest to
    `FALLBACK_TARGET_SHARE`, else the alphabetically first numeric attribute
    at `FALLBACK_QUANTILE`); otherwise `None`, and `attr-filter` is skipped
    rather than fabricated.

    ``dataset`` is the dataset NAME as the CSV reports it, so an ordering
    variant shares its source dataset's pick: `3dbag_n1000-hilbert` is the
    same data as `3dbag_n1000` in a different row order, and measuring the
    two with different predicates would compare nothing.
    """
    pick = _hand_picked_for(dataset)
    if pick is not None:
        column, (kind, value) = pick
        wanted = _is_string if kind == "eq" else _is_numeric
        if column in attributes and wanted(types.get(column, "")):
            spec = _resolve_pick(table, column, kind, value, True, conn)
            if spec is not None:
                return spec
    return _fallback_attr_filter(table, attributes, types, conn)


def _hand_picked_for(dataset: str) -> tuple[str, tuple[str, Any]] | None:
    for kind, key, column, pick in HAND_PICKED:
        if kind == "family" and dataset.startswith(key):
            return column, pick
        if kind == "dataset" and (dataset == key or dataset.startswith(f"{key}-")):
            return column, pick
    return None


def _resolve_pick(table: str, column: str, kind: str, value: Any,
                  hand_picked: bool,
                  conn: duckdb.DuckDBPyConnection) -> AttrFilter | None:
    """Counts what one pick matches, computing its threshold first for a
    quantile pick. `None` when the pick matches nothing at all — a
    zero-result `attr-filter` measures the cost of proving an absence, not
    of attribute access."""
    total = conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
    if kind == "eq":
        matched = conn.execute(
            f'SELECT count(*) FROM {table} WHERE "{column}" = ?', [value]
        ).fetchone()[0]
        if not matched:
            return None
        return AttrFilter(
            column=column, op="eq", eq_value=str(value), ge_bound=None,
            matched=int(matched), share=matched / max(total, 1),
            hand_picked=hand_picked,
        )

    bound = _quantile(table, column, float(value), conn)
    if bound is None:
        return None
    matched = conn.execute(
        f'SELECT count(*) FROM {table} WHERE "{column}" >= ?', [bound]
    ).fetchone()[0]
    if not matched:
        return None
    return AttrFilter(
        column=column, op="ge", eq_value=None, ge_bound=float(bound),
        matched=int(matched), share=matched / max(total, 1),
        hand_picked=hand_picked,
    )


def _fallback_attr_filter(table: str, attributes: list[str],
                          types: dict[str, str],
                          conn: duckdb.DuckDBPyConnection) -> AttrFilter | None:
    total = conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
    best: tuple[float, str, str, int] | None = None
    for column in sorted(c for c in attributes if _is_string(types.get(c, ""))):
        distinct = conn.execute(
            f'SELECT count(DISTINCT "{column}") FROM {table}'
        ).fetchone()[0]
        if not (FALLBACK_MIN_DISTINCT <= distinct <= FALLBACK_MAX_DISTINCT):
            continue
        row = conn.execute(
            f'SELECT "{column}", count(*) AS n FROM {table} '
            f'WHERE "{column}" IS NOT NULL AND "{column}" <> \'\' '
            f'GROUP BY 1 ORDER BY n DESC, 1 ASC LIMIT 1'
        ).fetchone()
        if row is None or row[0] is None:
            continue
        value, count = str(row[0]), int(row[1])
        if any(ch in value for ch in NOTES_HOSTILE):
            continue
        share = count / max(total, 1)
        key = abs(share - FALLBACK_TARGET_SHARE)
        if best is None or (key, column) < (best[0], best[1]):
            best = (key, column, value, count)
    if best is not None:
        _, column, value, count = best
        return AttrFilter(
            column=column, op="eq", eq_value=value, ge_bound=None,
            matched=count, share=count / max(total, 1), hand_picked=False,
        )

    numeric = sorted(c for c in attributes if _is_numeric(types.get(c, "")))
    if not numeric:
        return None
    return _resolve_pick(table, numeric[0], "quantile", FALLBACK_QUANTILE, False, conn)


def resolve_attr_range(table: str, attributes: list[str], types: dict[str, str],
                       numeric_column: str | None,
                       conn: duckdb.DuckDBPyConnection) -> AttrRange | None:
    """CJDB Q1's `column > threshold`, derived deterministically.

    `b3_h_dak_max` when the package carries it as a numeric attribute (the
    column CJDB's own Q1 names), otherwise the dataset's `numeric_column` —
    the same column `attr-stats` aggregates, so the three systems are known
    to agree on it. The threshold is the column's `ATTR_RANGE_QUANTILE`
    quantile, and the matched count is recorded beside it.
    """
    column = None
    if ATTR_RANGE_PREFERRED_COLUMN in attributes and _is_numeric(
        types.get(ATTR_RANGE_PREFERRED_COLUMN, "")
    ):
        column = ATTR_RANGE_PREFERRED_COLUMN
    elif numeric_column and _is_numeric(types.get(numeric_column, "")):
        column = numeric_column
    if column is None:
        return None

    threshold = _quantile(table, column, ATTR_RANGE_QUANTILE, conn)
    if threshold is None:
        return None
    matched = conn.execute(
        f'SELECT count(*) FROM {table} WHERE "{column}" > ?', [threshold]
    ).fetchone()[0]
    return AttrRange(
        column=column, quantile=ATTR_RANGE_QUANTILE,
        threshold=float(threshold), matched=int(matched),
    )


def resolve_windows(table: str, conn: duckdb.DuckDBPyConnection
                    ) -> tuple[BBox, tuple[float, float], tuple[BboxWindow, ...], int]:
    """The dataset extent, the median row centre, the three windows and the
    number of rows they were searched over.

    The row boxes never reach Python: `minimum_halves`' arithmetic is
    pushed into DuckDB (one sorted DOUBLE column instead of a million
    six-field structs) and only the bisection runs here, over exactly the
    same values `config.minimum_halves` would have produced.
    """
    row = conn.execute(
        f"SELECT min(bbox.xmin), min(bbox.ymin), min(bbox.zmin), "
        f"max(bbox.xmax), max(bbox.ymax), max(bbox.zmax), "
        f"median((bbox.xmin + bbox.xmax) / 2.0), "
        f"median((bbox.ymin + bbox.ymax) / 2.0), count(*) "
        f"FROM {table} WHERE bbox IS NOT NULL"
    ).fetchone()
    if row is None or row[8] == 0:
        raise ValueError(f"no row of {table} has a bbox — cannot derive a window")
    # float(), not the raw values: DuckDB returns DECIMAL for a bbox struct
    # whose members were built from decimal literals, and mixing Decimal
    # with float in `window_at` raises rather than quietly rounding.
    dataset = BBox(minx=float(row[0]), miny=float(row[1]), minz=float(row[2]),
                   maxx=float(row[3]), maxy=float(row[4]), maxz=float(row[5]))
    centre = (float(row[6]), float(row[7]))
    rows = int(row[8])

    span_x = dataset.maxx - dataset.minx
    span_y = dataset.maxy - dataset.miny
    terms = ["0.0"]
    if span_x:
        terms += [f"({centre[0]} - bbox.xmax) / {span_x}",
                  f"(bbox.xmin - {centre[0]}) / {span_x}"]
    if span_y:
        terms += [f"({centre[1]} - bbox.ymax) / {span_y}",
                  f"(bbox.ymin - {centre[1]}) / {span_y}"]
    halves = [
        float(v) for v in conn.execute(
            f"SELECT greatest({', '.join(terms)})::DOUBLE AS h FROM {table} "
            f"WHERE bbox IS NOT NULL ORDER BY h"
        ).to_arrow_table().column("h").to_pylist()
    ]

    windows = tuple(
        window_from_halves(halves, centre, dataset, target, tag)
        for target, tag in BBOX_TARGETS
    )
    return dataset, centre, windows, rows


def derive(source: Path, package: Path) -> Params:
    """The shared query parameters: source facts plus package facts.

    ``package`` is a CityParquet package directory. It supplies the extent,
    the query windows, the `attr-filter` predicate and `attr-range`'s
    threshold — every parameter the format harness also derives from the
    package, so the two families ask the same questions. The source-order
    and Hilbert packages hold the same rows in a different order, so either
    yields identical parameters; the caller passes the source-order one for
    determinism.
    """
    total_objects, target_id, numeric_column = source_facts(source)

    conn = duckdb.connect()
    try:
        conn.execute("SET enable_geoparquet_conversion = false")
        files = object_table_files(package)
        if len(files) == 1:
            table = f"read_parquet('{files[0]}')"
        else:
            table = ("read_parquet([" + ", ".join(f"'{f}'" for f in files)
                     + "], union_by_name = true)")
        bbox_full, centre, windows, window_rows = resolve_windows(table, conn)
        attributes = package_attributes(files, conn)
        types = _column_types(table, conn)
        attr_filter = resolve_attr_filter(
            dataset_name_of(package), table, attributes, types, conn
        )
        attr_range = resolve_attr_range(
            table, attributes, types, numeric_column, conn
        )
    finally:
        conn.close()

    return Params(
        bbox_full=bbox_full,
        windows=windows,
        point_xy=centre,
        attr_filter=attr_filter,
        attr_range=attr_range,
        numeric_column=numeric_column,
        target_id=target_id,
        total_city_objects=total_objects,
        window_rows=window_rows,
    )


def dataset_name_of(package: Path) -> str:
    """The dataset name a package directory carries, for `HAND_PICKED`.

    `<name>.parquet` / `<name>-hilbert.parquet` both reduce to `<name>`'s
    own hand-picked entry, because `_hand_picked_for` matches a
    `-<suffix>` ordering variant of a dataset name as well as the name
    itself — the same rule `params.rs::hand_picked_for` applies.
    """
    return package.name[: -len(".parquet")] if package.name.endswith(".parquet") else package.name


def to_json(p: Params) -> str:
    """Serialise for committing to params/<dataset>.json."""
    payload = {
        "attr_filter": dataclasses.asdict(p.attr_filter) if p.attr_filter else None,
        "attr_range": dataclasses.asdict(p.attr_range) if p.attr_range else None,
        "bbox_full": dataclasses.asdict(p.bbox_full),
        "numeric_column": p.numeric_column,
        "point_xy": list(p.point_xy),
        "target_id": p.target_id,
        "total_city_objects": p.total_city_objects,
        "window_rows": p.window_rows,
        "windows": [
            {
                "tag": w.tag,
                "target": w.target,
                "achieved": w.achieved,
                "approx": w.approx,
                "window": dataclasses.asdict(w.window),
            }
            for w in p.windows
        ],
    }
    return json.dumps(payload, indent=2, sort_keys=True) + "\n"


def from_json(text: str) -> Params:
    d = json.loads(text)
    return Params(
        bbox_full=BBox(**d["bbox_full"]),
        windows=tuple(
            BboxWindow(
                tag=w["tag"], target=w["target"], achieved=w["achieved"],
                window=BBox(**w["window"]), approx=w["approx"],
            )
            for w in d["windows"]
        ),
        point_xy=tuple(d["point_xy"]),
        attr_filter=AttrFilter(**d["attr_filter"]) if d["attr_filter"] else None,
        attr_range=AttrRange(**d["attr_range"]) if d["attr_range"] else None,
        numeric_column=d["numeric_column"],
        target_id=d["target_id"],
        total_city_objects=d["total_city_objects"],
        window_rows=d["window_rows"],
    )
