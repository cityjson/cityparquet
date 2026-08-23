#!/usr/bin/env python3
"""Resolve a CityParquet package's object tables from its STAC `metadata.json`.

The shell side of the benchmark harness needs the same table list the Rust
reader derives, and there is exactly one right way to derive it. Since
2026-07-21 a package's `metadata.json` IS a STAC Item (there is no top-level
`tables` key any more): object tables are the assets carrying the
`cityparquet-objects` role, sidecars carry `cityparquet-sidecar`.

This mirrors `PackageTables::open`
(`crates/cityparquet/src/stac/properties.rs`) deliberately and exactly:

  * assets are visited in the Item's asset order (`json.load` preserves object
    insertion order), which is first-appearance table order;
  * an asset's `href` is the authoritative locator, not its map key — a
    foreign writer may key its assets differently — with a leading `./`
    stripped;
  * a package naming the same object table twice is corrupt (every object in
    it would be counted twice) and is rejected, not silently deduplicated;
  * a package with no object-table asset is rejected.

Usage:
    package_tables.py PACKAGE_DIR              # every object table, one per line
    package_tables.py PACKAGE_DIR --single     # exactly one, else exit 1
    package_tables.py PACKAGE_DIR --sidecars   # sidecar files instead

Prints file names relative to the package directory. Exits 1 with a message on
stderr for a missing/unreadable manifest, a duplicate table, an empty table
set, or (under `--single`) a multi-table package.
"""

import json
import os
import sys

ROLE_OBJECT_TABLE = "cityparquet-objects"
ROLE_SIDECAR = "cityparquet-sidecar"


def resolve(package_dir):
    """Return (tables, sidecars) as file-name lists. Raises ValueError."""
    manifest_path = os.path.join(package_dir, "metadata.json")
    try:
        with open(manifest_path) as fh:
            item = json.load(fh)
    except OSError as e:
        raise ValueError(f"cannot read {manifest_path}: {e}") from e
    except json.JSONDecodeError as e:
        raise ValueError(f"{manifest_path} is not valid JSON: {e}") from e

    tables, sidecars, seen = [], [], set()
    for asset in item.get("assets", {}).values():
        roles = asset.get("roles", []) or []
        is_table = ROLE_OBJECT_TABLE in roles
        is_sidecar = ROLE_SIDECAR in roles
        if not is_table and not is_sidecar:
            continue
        href = asset.get("href")
        if not href:
            raise ValueError(f"{manifest_path}: asset with roles {roles} has no href")
        name = href[2:] if href.startswith("./") else href
        if is_table:
            if name in seen:
                raise ValueError(
                    f"{manifest_path}: package lists duplicate object table '{name}'"
                )
            seen.add(name)
            tables.append(name)
        else:
            sidecars.append(name)

    if not tables:
        raise ValueError(
            f"{manifest_path}: package lists no object tables "
            f"(no asset carries the {ROLE_OBJECT_TABLE} role)"
        )
    return tables, sidecars


def main(argv):
    args = [a for a in argv[1:] if not a.startswith("--")]
    flags = {a for a in argv[1:] if a.startswith("--")}
    unknown = flags - {"--single", "--sidecars"}
    if len(args) != 1 or unknown:
        print(
            "usage: package_tables.py PACKAGE_DIR [--single] [--sidecars]",
            file=sys.stderr,
        )
        return 2

    try:
        tables, sidecars = resolve(args[0])
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    if "--sidecars" in flags:
        print("\n".join(sidecars))
        return 0

    if "--single" in flags and len(tables) != 1:
        print(
            f"error: {args[0]} lists {len(tables)} object tables ({tables}); "
            "this caller only supports single-table (single-family) packages, "
            "not multi-table by-type packages",
            file=sys.stderr,
        )
        return 1

    print("\n".join(tables))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
