"""Whole-solid metrics in SQL via duckdb-3d, one row per Building."""
from __future__ import annotations

import re

import duckdb
import pyarrow as pa

from .errors import MissingColumns, MissingLoD

_REFS = ("b3_volume_lod22", "b3_opp_dak_plat", "b3_opp_dak_schuin",
         "b3_opp_grond", "b3_opp_buitenmuur", "b3_opp_scheidingsmuur")


def lod_to_suffix(lod: str) -> str:
    if not re.fullmatch(r"\d\.\d", lod):
        raise MissingLoD(f"malformed LoD {lod!r}; expected e.g. '2.2'")
    return "lod" + lod.replace(".", "_")


def available_lods(con: duckdb.DuckDBPyConnection, input_glob: str) -> list[str]:
    cols = [d[0] for d in con.sql(
        "SELECT * FROM read_parquet(?) LIMIT 0", params=[input_glob]
    ).description]
    lods = sorted(m.group(1) + "." + m.group(2)
                  for c in cols
                  if (m := re.fullmatch(r"geometry_lod(\d)_(\d)", c)))
    return lods


def build_features(con: duckdb.DuckDBPyConnection,
                   input_glob: str, lod: str) -> pa.Table:
    suffix = lod_to_suffix(lod)
    lods = available_lods(con, input_glob)
    if lod not in lods:
        raise MissingLoD(
            f"LoD {lod} not in this package; available: {', '.join(lods)}"
        )

    cols = [d[0] for d in con.sql(
        "SELECT * FROM read_parquet(?) LIMIT 0", params=[input_glob]
    ).description]
    required = ("oorspronkelijkbouwjaar", *_REFS)
    missing = [c for c in required if c not in cols]
    if missing:
        raise MissingColumns(
            f"input is missing columns: {', '.join(missing)}; "
            "energy features requires 3DBAG-as-CityParquet input "
            "(the b3_* reference columns and oorspronkelijkbouwjaar)"
        )

    geom = f"geometry_{suffix}"
    refs = ", ".join(f"any_value(b.{r}) AS {r}" for r in _REFS)
    query = f"""
        WITH parts AS (
          SELECT p.parents[1] AS building_id,
                 ST_3DTryFromWKB(p.{geom}) AS solid
          FROM read_parquet($input) p
          WHERE p.object_type = 'BuildingPart' AND p.{geom} IS NOT NULL
        ),
        metrics AS (
          SELECT building_id,
                 ST_3DVolume(solid) AS v,
                 ST_3DSurfaceArea(solid) AS a,
                 ST_3DFootprintArea(solid) AS fp,
                 ST_3DZMin(solid) AS zmin,
                 ST_3DZMax(solid) AS zmax,
                 ST_3DIsClosed(solid) AS closed
          FROM parts
          WHERE solid IS NOT NULL
        )
        SELECT m.building_id,
               any_value(b.oorspronkelijkbouwjaar) AS year,
               count(*) AS n_parts,
               sum(m.v) AS volume_m3,
               sum(m.a) AS envelope_m2,
               sum(m.a) / nullif(sum(m.v), 0) AS sv_ratio,
               sum(m.fp) AS footprint_m2,
               max(m.zmax) - min(m.zmin) AS height_m,
               bool_and(m.closed) AS is_closed,
               {refs}
        FROM metrics m
        JOIN read_parquet($input) b
          ON b.id = m.building_id AND b.object_type = 'Building'
        GROUP BY m.building_id
        ORDER BY m.building_id
    """
    return con.sql(query, params={"input": input_glob}).to_arrow_table()
