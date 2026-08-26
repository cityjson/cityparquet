#!/usr/bin/env python3
"""Inject a CityJSON ``metadata.referenceSystem`` when the source lacks one.

Discovered while running Task 14 (the heterogeneity corpus): none of
Montreal.city.jsonl / Vienna.city.jsonl / Zurich.city.jsonl /
lod3_railway.city.json — as actually published at
storage.googleapis.com/cityjson/... — carry a ``metadata.referenceSystem``,
despite the task brief's own instruction to read the EPSG code from that
field. `cityparquet convert` (cityparquet-rs) enforces the spec's CRS rule
strictly and refuses outright to write a package for a CRS-less source
that nonetheless carries real coordinates ("source carries a CRS-bearing
coordinate ... but declares no CRS a writer can resolve to PROJJSON").
Without this step, three of the four corpus datasets simply cannot be
converted at all.

The EPSG code for each real-world dataset was independently determined by
transforming its ``geographicalExtent`` corners with PROJ (`cs2cs`) and
confirming the result lands at the real city's known location — see
`.superpowers/sdd/2026-07-31-database-benchmark-harness/task-14-report.md`
for the full derivation and cross-check for each dataset. lod3_railway's
extent is a ~12x7x1.5m box near the coordinate origin — not a real-world
location at all (a local/model-space LoD3 test scene) — so it is stamped
with the harness's own pre-existing default (EPSG:7415, matching
`docker/compose.yml`'s `CITYDB_SRID` default) purely to satisfy the
writer's/importers' hard CRS requirement, with no geographic meaning
implied.

This script never touches the PRISTINE, checksum-pinned downloads: it is a
separate, explicit, idempotent step run AFTER `fetch_corpus.sh`, on the
same on-disk path `fetch_corpus.sh` writes to. Re-running `fetch_corpus.sh`
re-downloads the pristine (CRS-less) bytes and re-verifies them against
`corpus.sha256` exactly as before — this script must be re-run afterwards
to restore the working, CRS-stamped copy `cityparquet convert`/`cjdb
import`/`citydb-tool import` all then read from.

For a CityJSONSeq file (Montreal/Vienna/Zurich), only the FIRST line (the
CityJSON header, which alone carries `metadata`) is parsed and rewritten;
every following CityJSONFeature line is streamed through byte-for-byte,
so a 259MB file (Zurich) is never fully loaded into memory and no
feature's geometry/vertex encoding is ever touched. For a single-document
CityJSON file (lod3_railway), the whole document is parsed, modified, and
rewritten — small enough (4.5MB) that this is cheap, and Python's `json`
module round-trips floats losslessly (shortest round-trip repr).
"""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path


def _crs_uri(epsg: int) -> str:
    return f"https://www.opengis.net/def/crs/EPSG/0/{epsg}"


def _inject(doc: dict, epsg: int) -> tuple[dict, bool]:
    """Returns (possibly-modified doc, whether a change was made).

    If `referenceSystem` is already present, it is left untouched and
    `False` is returned — this script never overwrites a source-declared
    CRS, only fills a genuine absence.
    """
    metadata = doc.get("metadata") or {}
    if metadata.get("referenceSystem"):
        return doc, False
    metadata = dict(metadata)
    metadata["referenceSystem"] = _crs_uri(epsg)
    doc = dict(doc)
    doc["metadata"] = metadata
    return doc, True


def stamp(path: Path, epsg: int) -> str:
    """Idempotently ensure ``path`` declares ``epsg`` as its CRS.

    Returns a one-line status message.
    """
    with path.open("rb") as f:
        first_line_bytes = f.readline()
        rest_offset = f.tell()
        file_size = f.seek(0, 2)

    try:
        header = json.loads(first_line_bytes)
        is_cityjson_header = header.get("type") == "CityJSON"
    except json.JSONDecodeError:
        header = None
        is_cityjson_header = False

    if is_cityjson_header and rest_offset < file_size:
        # CityJSONSeq: rewrite only the header line, stream the rest.
        new_header, changed = _inject(header, epsg)
        if not changed:
            return f"{path}: already declares a referenceSystem — unchanged"
        tmp = path.with_name(path.name + ".stamping-tmp")
        with tmp.open("wb") as out:
            out.write(json.dumps(new_header, separators=(",", ":")).encode("utf-8"))
            out.write(b"\n")
            with path.open("rb") as f:
                f.seek(rest_offset)
                shutil.copyfileobj(f, out)
        tmp.replace(path)
        return f"{path}: stamped EPSG:{epsg} onto the CityJSONSeq header (features untouched)"

    # Single-document CityJSON (or unparseable first line): parse and
    # rewrite the whole file. Only lod3_railway.city.json in this corpus
    # takes this path, and it is small (4.5MB).
    doc = json.loads(path.read_bytes())
    if doc.get("type") != "CityJSON":
        raise ValueError(f"{path}: not a CityJSON document (type={doc.get('type')!r})")
    new_doc, changed = _inject(doc, epsg)
    if not changed:
        return f"{path}: already declares a referenceSystem — unchanged"
    tmp = path.with_name(path.name + ".stamping-tmp")
    tmp.write_text(json.dumps(new_doc, separators=(",", ":")))
    tmp.replace(path)
    return f"{path}: stamped EPSG:{epsg} onto the single-document CityJSON file"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dataset", type=Path, help="path to the .city.json[l] file")
    parser.add_argument("epsg", type=int, help="EPSG code to inject if absent")
    args = parser.parse_args(argv)
    print(stamp(args.dataset, args.epsg))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
