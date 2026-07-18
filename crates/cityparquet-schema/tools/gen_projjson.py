#!/usr/bin/env python3
"""Generate the vendored EPSG -> PROJJSON lookup table (spec §13.3, gap G1).

Offline, checked-in generator. NOT run at build time — its gzipped output is
committed so `cargo build` stays hermetic (no C toolchain, no network).

Emits a gzip of a JSON object:
  {"_meta": {"proj_version": ..., "pyproj_version": ..., "generated_codes": N},
   "<epsg_code>": <PROJJSON object>, ...}

Every PROJJSON is exactly what pyproj/PROJ emit, so it is byte-for-byte the
same definition GDAL/GeoPandas write (same proj.db lineage). Run:
    python3 crates/cityparquet-schema/tools/gen_projjson.py
"""
import gzip
import json
import pathlib

import pyproj
from pyproj.database import query_crs_info

OUT = pathlib.Path(__file__).resolve().parent.parent / "assets" / "epsg_projjson.json.gz"


def main() -> None:
    table = {
        "_meta": {
            "proj_version": pyproj.proj_version_str,
            "pyproj_version": pyproj.__version__,
        }
    }
    # Every EPSG CRS, including deprecated (old datasets cite deprecated codes).
    infos = query_crs_info(auth_name="EPSG", allow_deprecated=True)
    ok = 0
    for info in infos:
        try:
            crs = pyproj.CRS.from_authority("EPSG", info.code)
            table[str(info.code)] = json.loads(crs.to_json())
            ok += 1
        except Exception:
            continue
    # A couple of common OGC-authority CRSs a CityJSON URL can name.
    for auth, code in (("OGC", "CRS84"), ("OGC", "CRS84h")):
        try:
            table[f"{auth}:{code}"] = json.loads(
                pyproj.CRS.from_authority(auth, code).to_json()
            )
        except Exception:
            pass
    table["_meta"]["generated_codes"] = ok
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(OUT, "wt", encoding="utf-8", compresslevel=9) as f:
        json.dump(table, f, separators=(",", ":"))
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes, {ok} EPSG codes)")


if __name__ == "__main__":
    main()
