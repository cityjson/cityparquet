"""Degree-day heat-loss screen and retrofit ranking over a feature table."""
from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path

import duckdb
import pyarrow as pa

_DEFAULT_PARAMS = Path(__file__).parent / "params" / "u_values.toml"


@dataclass
class Band:
    name: str
    max_year: int | None
    u_roof: float
    u_wall: float
    u_ground: float


def load_params(path: str | None = None) -> list[Band]:
    with open(path or _DEFAULT_PARAMS, "rb") as fh:
        raw = tomllib.load(fh)
    bands = [Band(b["name"], b.get("max_year"), b["u_roof"], b["u_wall"], b["u_ground"])
             for b in raw["bands"]]
    bands.sort(key=lambda b: (b.max_year is None, b.max_year))
    return bands


def band_for_year(bands: list[Band], year: int | None) -> Band:
    if year is None:
        return bands[0]
    for band in bands:
        if band.max_year is not None and year <= band.max_year:
            return band
    return bands[-1]


def screen_features(features_path: str, bands: list[Band], hdd: float,
                    year_before: int | None, sv_above: float | None,
                    top: int | None) -> pa.Table:
    """Compute heat transfer and rank buildings by annual heating demand.

    Filter semantics:
    - year_before: KEEP rows with year=None (unknown age treated as oldest);
                   drop rows where year is not None and year >= year_before.
    - sv_above:    DROP rows with sv_ratio=None (missing volume cannot be asserted
                   above threshold); keep others where sv_ratio > sv_above.
    """
    con = duckdb.connect()
    rows = con.sql("SELECT * FROM read_parquet(?)",
                   params=[features_path]).arrow().read_all().to_pylist()

    # Build schema to ensure empty results preserve column structure.
    input_schema = con.sql("SELECT * FROM read_parquet(?)",
                           params=[features_path]).arrow().read_all().schema
    output_fields = list(input_schema) + [
        pa.field("u_roof", pa.float64()),
        pa.field("u_wall", pa.float64()),
        pa.field("u_ground", pa.float64()),
        pa.field("h_t_w_per_k", pa.float64()),
        pa.field("annual_kwh", pa.float64()),
        pa.field("rank", pa.int64()),
    ]
    output_schema = pa.schema(output_fields)

    out = []
    for r in rows:
        # year_before: keep year=None (unknown→oldest), drop year>=year_before
        if year_before is not None and r["year"] is not None and r["year"] >= year_before:
            continue
        # sv_above: drop sv_ratio=None (missing volume), keep sv_ratio>sv_above
        if sv_above is not None:
            if r["sv_ratio"] is None or not (r["sv_ratio"] > sv_above):
                continue
        band = band_for_year(bands, r["year"])
        h_t = (band.u_roof * (r["a_roof_flat_m2"] + r["a_roof_pitched_m2"])
               + band.u_wall * r["a_wall_m2"]
               + band.u_ground * r["a_ground_m2"])
        out.append({**r, "u_roof": band.u_roof, "u_wall": band.u_wall,
                    "u_ground": band.u_ground, "h_t_w_per_k": h_t,
                    "annual_kwh": h_t * hdd * 24.0 / 1000.0})
    out.sort(key=lambda r: r["annual_kwh"], reverse=True)
    for i, r in enumerate(out):
        r["rank"] = i + 1
    if top is not None:
        out = out[:top]
    return pa.Table.from_pylist(out, schema=output_schema)
