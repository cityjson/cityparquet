"""Orchestrate the `features` subcommand: SQL metrics + face split → Parquet."""
from __future__ import annotations

from dataclasses import dataclass, field

from . import db
from .faces import compute_face_tables
from .features import build_features, lod_to_suffix

_REF_COLS = ("b3_volume_lod22", "b3_opp_dak_plat", "b3_opp_dak_schuin",
             "b3_opp_grond", "b3_opp_buitenmuur", "b3_opp_scheidingsmuur")


@dataclass
class RunSummary:
    n_buildings: int
    n_parts: int
    n_null_geometry: int
    n_open_solids: int
    n_buildings_missing_geometry: int
    outputs: list[str] = field(default_factory=list)


def run_features(input_glob: str, lod: str, output: str,
                 faces_out: str | None, validate_out: str | None,
                 flat_tilt_deg: float, ext_dir=None) -> RunSummary:
    con = db.connect(ext_dir, need_httpfs=input_glob.startswith("s3://"))
    features = build_features(con, input_glob, lod)
    faces, classes = compute_face_tables(input_glob, lod_to_suffix(lod), flat_tilt_deg)
    con.register("features_t", features)
    con.register("classes_t", classes)

    drop_refs = "" if validate_out else \
        "EXCLUDE (" + ", ".join(_REF_COLS) + ")"
    con.execute(
        f"""
        COPY (
          SELECT f.* {drop_refs},
                 coalesce(c.a_roof_flat_m2, 0)    AS a_roof_flat_m2,
                 coalesce(c.a_roof_pitched_m2, 0) AS a_roof_pitched_m2,
                 coalesce(c.a_wall_m2, 0)         AS a_wall_m2,
                 coalesce(c.a_ground_m2, 0)       AS a_ground_m2,
                 coalesce(c.a_other_m2, 0)        AS a_other_m2
          FROM features_t f LEFT JOIN classes_t c USING (building_id)
          ORDER BY f.building_id
        ) TO '{output}' (FORMAT PARQUET, COMPRESSION ZSTD)
        """
    )
    outputs = [output]

    if faces_out:
        con.register("faces_t", faces)
        con.execute(
            "COPY (SELECT * FROM faces_t ORDER BY building_id, part_id, face_idx)"
            f" TO '{faces_out}' (FORMAT PARQUET, COMPRESSION ZSTD)"
        )
        outputs.append(faces_out)

    geom = f"geometry_{lod_to_suffix(lod)}"
    n_null, n_parts = con.sql(
        f"""
        SELECT count(*) FILTER ({geom} IS NULL),
               count(*) FILTER ({geom} IS NOT NULL)
        FROM read_parquet($input) WHERE object_type = 'BuildingPart'
        """,
        params={"input": input_glob},
    ).fetchone()
    n_open = con.sql(
        "SELECT count(*) FROM features_t WHERE NOT is_closed"
    ).fetchone()[0]
    n_buildings_total = con.sql(
        """
        SELECT count(DISTINCT id) FROM read_parquet($input)
        WHERE object_type = 'Building'
        """,
        params={"input": input_glob},
    ).fetchone()[0]
    n_buildings_missing_geometry = n_buildings_total - features.num_rows
    return RunSummary(
        n_buildings=features.num_rows, n_parts=n_parts,
        n_null_geometry=n_null, n_open_solids=n_open,
        n_buildings_missing_geometry=n_buildings_missing_geometry,
        outputs=outputs,
    )
