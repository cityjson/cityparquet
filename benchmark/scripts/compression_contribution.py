#!/usr/bin/env python3
"""Break each prepared CityParquet package down by column group and by column.

For every dataset the script reads each object table of the package with
DuckDB's ``parquet_metadata()`` and sums, over all row groups and all object
tables, the compressed and the uncompressed bytes of every column chunk. A
nested column (a struct, list or map) rolls up to its top-level column, and
each top-level column falls into one group by its specification name:

- ``geometry``: ``geometry_lod*``
- ``geometry_properties``: ``geometry_properties_lod*``
- ``appearance``: ``material_lod*`` and ``texture_lod*``
- ``identifiers_structure``: ``id``, ``feature_id``, ``object_type``,
  ``parents``, ``children``, ``children_roles``
- ``bbox``: ``bbox``
- ``other``: ``other`` and ``implicit_geometry``
- ``attributes``: ``address`` and every column that is not reserved

"Uncompressed" is Parquet's ``total_uncompressed_size``: the size after the
column's encoding (for example dictionary or delta encoding) and before the
codec. For a plain byte-array column such as WKB geometry it is close to the
raw value bytes; for a dictionary-encoded column it is already the encoded
size, so the ratio understates the total saving over the raw values.

The rest of the package is reported as parts, so that column bytes and parts
add up to the package size ``measure_sizes.artefact_bytes`` reports:
``bloom_filters`` (the object tables' Bloom filters), ``footer_and_page_indexes``
(the remainder of each object table: footer, page indexes, magic bytes),
``sidecar_tables`` (the ``cityparquet-sidecar`` assets, whole files),
``metadata`` (``metadata.json``) and ``other_files``. Object tables and sidecars
are told apart by their asset roles in ``metadata.json``.

Run it in benchmark/databases' uv environment, which provides DuckDB::

    uv run --project benchmark/databases python benchmark/scripts/compression_contribution.py \\
        --prepared DIR --out DIR [--dataset NAME ...]

Without ``--dataset`` it covers every dataset in ``--prepared``: each
``<name>.parquet`` package and each ``<name>.city.jsonl`` stream, so a dataset
whose package is missing is reported as missing. A missing package gets one row
with status ``missing`` and empty values, never zeros, and the script exits 1
after writing the CSV.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from measure_sizes import artefact_bytes

OUTPUT_NAME = "compression.csv"
HEADER = [
    "dataset",
    "status",
    "level",
    "name",
    "group",
    "encodings",
    "compressed_bytes",
    "uncompressed_bytes",
    "compressed_mb_decimal",
    "share_of_column_bytes",
    "share_of_package_bytes",
    "compression_ratio",
]
GROUPS = ["geometry", "geometry_properties", "appearance", "attributes", "identifiers_structure", "bbox", "other"]
PARTS = ["bloom_filters", "footer_and_page_indexes", "sidecar_tables", "metadata", "other_files"]
IDENTIFIERS_STRUCTURE = {"id", "feature_id", "object_type", "parents", "children", "children_roles"}
BYTES_PER_MB = 1_000_000
OBJECTS_ROLE = "cityparquet-objects"
SIDECAR_ROLE = "cityparquet-sidecar"
# DuckDB joins the names along a leaf's path with ", ".
PATH_SEPARATOR = ", "


def column_group(name: str) -> str:
    """The group a top-level object-table column belongs to."""
    if re.fullmatch(r"geometry_properties_lod.+", name):
        return "geometry_properties"
    if re.fullmatch(r"geometry_lod.+", name):
        return "geometry"
    if re.fullmatch(r"(material|texture)_lod.+", name):
        return "appearance"
    if name in IDENTIFIERS_STRUCTURE:
        return "identifiers_structure"
    if name == "bbox":
        return "bbox"
    if name in {"other", "implicit_geometry"}:
        return "other"
    return "attributes"


def top_level_column(path_in_schema: str, columns: list[str]) -> str:
    """The top-level column a leaf path belongs to; the longest matching name wins."""
    matches = [name for name in columns if path_in_schema == name or path_in_schema.startswith(name + PATH_SEPARATOR)]
    if not matches:
        raise ValueError(f"leaf {path_in_schema!r} matches no top-level column")
    return max(matches, key=len)


def assets_by_role(package: Path) -> tuple[list[Path], list[Path]]:
    """The package's object tables and sidecar tables, from ``metadata.json``."""
    metadata = json.loads((package / "metadata.json").read_text(encoding="utf-8"))
    objects, sidecars = set(), set()
    for asset in metadata["assets"].values():
        roles = asset.get("roles", [])
        path = (package / asset["href"]).resolve()
        if OBJECTS_ROLE in roles:
            objects.add(path)
        elif SIDECAR_ROLE in roles:
            sidecars.add(path)
    return sorted(objects), sorted(sidecars)


def read_columns(connection, table: Path) -> tuple[dict[str, dict], int]:
    """Per top-level column bytes and encodings of one table, and its Bloom bytes."""
    columns = [item[0] for item in connection.execute("SELECT * FROM read_parquet(?) LIMIT 0", [str(table)]).description]
    chunks = connection.execute(
        "SELECT path_in_schema, encodings, total_compressed_size, total_uncompressed_size, coalesce(bloom_filter_length, 0) FROM parquet_metadata(?)",
        [str(table)],
    ).fetchall()
    totals: dict[str, dict] = {}
    bloom = 0
    for path, encodings, compressed, uncompressed, bloom_length in chunks:
        name = top_level_column(path, columns)
        entry = totals.setdefault(name, {"compressed": 0, "uncompressed": 0, "encodings": set()})
        entry["compressed"] += compressed
        entry["uncompressed"] += uncompressed
        entry["encodings"].update(item.strip() for item in encodings.split(",") if item.strip())
        bloom += bloom_length
    return totals, bloom


def ratio(uncompressed: int, compressed: int) -> str:
    return f"{uncompressed / compressed:.4f}" if compressed else ""


def share(part: int, whole: int) -> str:
    return f"{part / whole:.6f}" if whole else ""


def mb_decimal(count: int) -> str:
    return f"{count / BYTES_PER_MB:.6f}"


def package_rows(dataset: str, package: Path) -> list[dict]:
    """Group, column, part and package rows for one package."""
    import duckdb

    objects, sidecars = assets_by_role(package)
    connection = duckdb.connect()
    columns: dict[str, dict] = {}
    bloom = 0
    remainder = 0
    for table in objects:
        totals, table_bloom = read_columns(connection, table)
        bloom += table_bloom
        chunk_bytes = sum(entry["compressed"] for entry in totals.values())
        left = table.stat().st_size - chunk_bytes - table_bloom
        if left < 0:
            raise SystemExit(f"{table}: column chunks and Bloom filters exceed the file size")
        remainder += left
        for name, entry in totals.items():
            merged = columns.setdefault(name, {"compressed": 0, "uncompressed": 0, "encodings": set()})
            merged["compressed"] += entry["compressed"]
            merged["uncompressed"] += entry["uncompressed"]
            merged["encodings"] |= entry["encodings"]
    total = artefact_bytes(package)
    column_bytes = sum(entry["compressed"] for entry in columns.values())
    known = set(objects) | set(sidecars) | {(package / "metadata.json").resolve()}
    others = sum(item.stat().st_size for item in package.rglob("*") if item.is_file() and item.resolve() not in known)
    parts = {
        "bloom_filters": bloom,
        "footer_and_page_indexes": remainder,
        "sidecar_tables": sum(path.stat().st_size for path in sidecars),
        "metadata": (package / "metadata.json").stat().st_size,
        "other_files": others,
    }
    if column_bytes + sum(parts.values()) != total:
        raise SystemExit(f"{package}: column bytes and parts do not add up to the package size")

    def row(level: str, name: str, group: str, compressed: int, uncompressed: int | None, encodings: str = "") -> dict:
        is_column = uncompressed is not None
        return {
            "dataset": dataset,
            "status": "ok",
            "level": level,
            "name": name,
            "group": group,
            "encodings": encodings,
            "compressed_bytes": compressed,
            "uncompressed_bytes": uncompressed if is_column else "",
            "compressed_mb_decimal": mb_decimal(compressed),
            "share_of_column_bytes": share(compressed, column_bytes) if is_column else "",
            "share_of_package_bytes": share(compressed, total),
            "compression_ratio": ratio(uncompressed, compressed) if is_column else "",
        }

    rows = []
    by_group: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for name, entry in columns.items():
        by_group[column_group(name)][0] += entry["compressed"]
        by_group[column_group(name)][1] += entry["uncompressed"]
    for group in GROUPS:
        if group in by_group:
            compressed, uncompressed = by_group[group]
            rows.append(row("group", group, group, compressed, uncompressed))
    for part in PARTS:
        rows.append(row("part", part, "", parts[part], None))
    rows.append(row("package", dataset, "", total, None))
    for name, entry in sorted(columns.items(), key=lambda item: (GROUPS.index(column_group(item[0])), -item[1]["compressed"], item[0])):
        rows.append(row("column", name, column_group(name), entry["compressed"], entry["uncompressed"], "|".join(sorted(entry["encodings"]))))
    return rows


def missing_row(dataset: str) -> dict:
    row = {key: "" for key in HEADER}
    row.update({"dataset": dataset, "status": "missing", "level": "package", "name": dataset})
    return row


def discover(prepared: Path) -> list[str]:
    """Every dataset with a package or a CityJSONSeq stream in ``prepared``."""
    names = {path.name[: -len(".parquet")] for path in prepared.glob("*.parquet") if path.is_dir()}
    names |= {path.name[: -len(".city.jsonl")] for path in prepared.glob("*.city.jsonl")}
    return sorted(names)


def print_table(rows: list[dict]) -> None:
    """The group and part rows of each dataset, readable on a terminal."""
    print(f"{'dataset':<24} {'level':<7} {'name':<24} {'MB':>10} {'of pkg':>7} {'ratio':>7}")
    for row in rows:
        if row["status"] == "missing":
            print(f"{row['dataset']:<24} MISSING")
            continue
        if row["level"] == "column":
            continue
        share_text = f"{float(row['share_of_package_bytes']) * 100:6.2f}%"
        ratio_text = f"{float(row['compression_ratio']):6.2f}x" if row["compression_ratio"] else ""
        print(f"{row['dataset']:<24} {row['level']:<7} {row['name']:<24} {float(row['compressed_mb_decimal']):>10.3f} {share_text:>7} {ratio_text:>7}")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    result.add_argument("--prepared", type=Path, required=True, help="directory holding the prepared <dataset>.parquet packages")
    result.add_argument("--out", type=Path, required=True, help=f"directory the {OUTPUT_NAME} is written to")
    result.add_argument("--dataset", action="append", default=[], help="dataset name; repeat it; default every dataset in --prepared")
    return result


def main() -> None:
    args = parser().parse_args()
    datasets = args.dataset or discover(args.prepared)
    if not datasets:
        raise SystemExit(f"no prepared datasets in {args.prepared}")
    rows = []
    missing = []
    for dataset in datasets:
        package = args.prepared / f"{dataset}.parquet"
        if package.is_dir():
            rows.extend(package_rows(dataset, package))
        else:
            missing.append(dataset)
            rows.append(missing_row(dataset))
    args.out.mkdir(parents=True, exist_ok=True)
    with (args.out / OUTPUT_NAME).open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=HEADER, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    print_table(rows)
    if missing:
        print(f"missing CityParquet package: {', '.join(missing)} (in {args.prepared})", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
