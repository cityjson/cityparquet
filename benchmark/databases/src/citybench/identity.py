"""The identifier-set cross-check: the same objects, not just as many.

The count cross-check (`runner.cross_check`) compares how many rows each
system returned. This one compares WHICH objects, for the scenarios that
return object rows (`IDENTITY_SCENARIOS`), and, where those rows carry
geometry, how many non-null geometries came back.

It is an UNTIMED verification pass: after a scenario's timed samples, each
system executes the same SQL once more (`verify_rows`) and the rows are
summarised here. Nothing inside the timed window changes.

Identifier mapping. Every system's scenario SQL returns the object's
CityJSON identifier, under each schema's own name:

- the CityParquet package (DuckDB): `id`;
- cjdb: `city_object.object_id`;
- 3DCityDB v5: `feature.objectid`, which `citydb import cityjson` fills
  with the CityJSON identifier.

The row scenarios return it as their first column; `id-lookup` returns the
whole object row, so the identifier is read by the column name above.
Identifiers are compared as strings, unaltered.

Geometry counted per system, for `id-lookup`: DuckDB's non-null
`geometry_lod*` columns (one per LoD), cjdb's `geometry` JSON array
length, 3DCityDB's `geometries` array length. For the row scenarios that
return geometry (`bbox-query`, `lod-query`), the non-null second column.
"""
from __future__ import annotations

import json
from dataclasses import dataclass

#: Scenarios whose rows are objects, and so whose identifier sets compare.
IDENTITY_SCENARIOS = frozenset(
    {"bbox-query", "attr-filter", "attr-range", "lod-query", "id-lookup"}
)

#: The identifier column of each schema (module docstring), most specific
#: first: cjdb's and 3DCityDB's tables also carry an `id`, their surrogate
#: key, which must never be taken for the object's identifier.
ID_COLUMNS = ("object_id", "objectid", "id")

#: How many differing identifiers a mismatch note names.
SHOWN = 5


@dataclass(frozen=True)
class Identity:
    ids: frozenset[str]
    geometries: int | None


def _geometry_count(columns: list[str], row: tuple) -> int:
    by_name = dict(zip(columns, row))
    count = sum(
        1 for name, value in by_name.items()
        if name.startswith("geometry_lod") and value is not None
    )
    for name in ("geometry", "geometries"):
        value = by_name.get(name)
        if isinstance(value, str):
            if value.startswith("["):
                value = json.loads(value)
            elif value.startswith("{"):
                # A PostgreSQL array literal (text fetch, no loader for
                # `geometry`): hex WKB elements hold no commas.
                value = [None if e == "NULL" else e
                         for e in value[1:-1].split(",") if e]
            else:
                value = [value]
        if isinstance(value, (list, tuple)):
            count += sum(1 for g in value if g is not None)
    return count


def summarise(scenario: str, columns: list[str], rows: list[tuple]) -> Identity:
    """The identifier set and non-null geometry count of one result."""
    if scenario == "id-lookup":
        key = next((c for c in ID_COLUMNS if c in columns), None)
        if key is None:
            raise ValueError(f"id-lookup returned no identifier column: {columns}")
        at = columns.index(key)
        return Identity(
            frozenset(str(row[at]) for row in rows),
            sum(_geometry_count(columns, row) for row in rows),
        )
    geometries = (
        sum(1 for row in rows if row[1] is not None) if len(columns) > 1 else None
    )
    return Identity(frozenset(str(row[0]) for row in rows), geometries)


def compare(identities: dict[str, Identity], tolerance: float = 0.0) -> str | None:
    """``None`` when every system agrees, else a note naming the first disagreement.

    Each system is compared against the first. ``tolerance`` is the
    fraction of the larger set the symmetric difference may reach; it is
    0 unless the count cross-check already accepted a deviation for this
    row, so an agreeing count never hides a different set.
    """
    if len(identities) < 2:
        return None
    (ref_tag, ref), *others = identities.items()
    problems = []
    for tag, other in others:
        only_ref = sorted(ref.ids - other.ids)
        only_other = sorted(other.ids - ref.ids)
        largest = max(len(ref.ids), len(other.ids), 1)
        if (only_ref or only_other) and (
                len(only_ref) + len(only_other)) / largest > tolerance:
            problems.append(
                f"{ref_tag} vs {tag}: only in {ref_tag} {only_ref[:SHOWN]} "
                f"({len(only_ref)}), only in {tag} {only_other[:SHOWN]} "
                f"({len(only_other)})"
            )
        if (ref.geometries is not None and other.geometries is not None
                and ref.geometries != other.geometries
                and abs(ref.geometries - other.geometries)
                / max(ref.geometries, other.geometries, 1) > tolerance):
            problems.append(
                f"geometries {ref_tag}={ref.geometries} {tag}={other.geometries}"
            )
    return ("id-mismatch: " + "; ".join(problems)) if problems else None
