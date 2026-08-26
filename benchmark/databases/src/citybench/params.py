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

from citybench.config import BBox, Params


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


def derive(source: Path) -> Params:
    """Read ``source`` once and produce the shared query parameters."""
    minx = miny = minz = float("inf")
    maxx = maxy = maxz = float("-inf")

    type_counts: Counter[str] = Counter()
    numeric_counts: Counter[str] = Counter()
    all_ids: list[str] = []
    parent_ids: list[str] = []
    total_objects = 0
    transform = {"scale": [1.0, 1.0, 1.0], "translate": [0.0, 0.0, 0.0]}

    for doc in _iter_json_lines(source):
        # The CityJSON header line carries the transform. It must be
        # captured here, before any vertices below are dequantised, since
        # CityJSONSeq feature lines that follow rely on this same transform
        # and carry none of their own.
        if doc.get("type") == "CityJSON" and "transform" in doc:
            transform = doc["transform"]

        scale = transform["scale"]
        translate = transform["translate"]
        for vx, vy, vz in doc.get("vertices", []):
            x = vx * scale[0] + translate[0]
            y = vy * scale[1] + translate[1]
            z = vz * scale[2] + translate[2]
            minx, maxx = min(minx, x), max(maxx, x)
            miny, maxy = min(miny, y), max(maxy, y)
            minz, maxz = min(minz, z), max(maxz, z)

        for obj_id, obj in doc.get("CityObjects", {}).items():
            total_objects += 1
            all_ids.append(obj_id)
            type_counts[obj["type"]] += 1
            if obj.get("children"):
                parent_ids.append(obj_id)
            for key, value in (obj.get("attributes") or {}).items():
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    numeric_counts[key] += 1

    if total_objects == 0:
        raise ValueError(f"{source}: no CityObjects found")

    # Ties broken by name so the result is deterministic.
    attr_eq = min(type_counts.items(), key=lambda kv: (-kv[1], kv[0]))[0]
    # An earlier version of this function raised ValueError here when a
    # dataset carried no numeric attribute at all. Discovered to be too
    # broad against the heterogeneity corpus (Task 14): Montreal's 294
    # Buildings carry NO "attributes" object whatsoever, and lod3_railway's
    # 121 CityObjects across 14 CityGML types carry only categorical
    # attributes ("function"/"class"/"species") — genuine, legitimate
    # properties of those datasets, not malformed input. The old raise
    # aborted derivation of EVERY scenario's parameters (bbox, attr-filter,
    # id-lookup, hierarchy, ...) for the sake of the one scenario
    # (attr-stats) that actually needs a numeric column, exactly the
    # failure mode `parent_id` below was already written to avoid for a
    # missing parent/child pair. So this mirrors that same precedent:
    # `numeric_column` is simply None, and only attr-stats (guarded by a
    # `ScenarioUnavailable` in each `sql_*.sql_for`/`build_child_args`, the
    # same mechanism `hierarchy` already uses for `parent_id is None`) is
    # affected, recorded downstream as a `skipped:` row rather than
    # blocking the other eight scenarios for every system on the dataset.
    numeric_column = (
        min(numeric_counts.items(), key=lambda kv: (-kv[1], kv[0]))[0]
        if numeric_counts else None
    )

    # Unlike the two checks above, an absent parent-child pair is a
    # legitimate property of a dataset (e.g. a railway or terrain corpus
    # with no BuildingPart-style children), not an input the derivation
    # cannot proceed without: bbox, attr-filter, attr-stats and id-lookup
    # are all still meaningful without it. So this does not raise —
    # `parent_id` is simply None, and only the one scenario that reads it
    # (`hierarchy`, itself run against 3 of the 5 systems) is affected;
    # it is recorded downstream as a skip for this dataset rather than
    # aborting derivation of the other nine scenarios for everyone.
    parent_id = sorted(parent_ids)[0] if parent_ids else None

    return Params(
        bbox_full=BBox(
            minx=minx, miny=miny, minz=minz, maxx=maxx, maxy=maxy, maxz=maxz
        ),
        attr_column="object_type",
        attr_eq=attr_eq,
        numeric_column=numeric_column,
        target_id=sorted(all_ids)[0],
        parent_id=parent_id,
        total_city_objects=total_objects,
    )


def to_json(p: Params) -> str:
    """Serialise for committing to params/<dataset>.json."""
    payload = {
        "attr_column": p.attr_column,
        "attr_eq": p.attr_eq,
        "bbox_full": dataclasses.asdict(p.bbox_full),
        "numeric_column": p.numeric_column,
        "parent_id": p.parent_id,
        "target_id": p.target_id,
        "total_city_objects": p.total_city_objects,
    }
    return json.dumps(payload, indent=2, sort_keys=True) + "\n"


def from_json(text: str) -> Params:
    d = json.loads(text)
    return Params(
        bbox_full=BBox(**d["bbox_full"]),
        attr_column=d["attr_column"],
        attr_eq=d["attr_eq"],
        numeric_column=d["numeric_column"],
        target_id=d["target_id"],
        parent_id=d["parent_id"],
        total_city_objects=d["total_city_objects"],
    )
