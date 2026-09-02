"""ST_3DFaces prototype: per-face metrics from CityParquet WKB + semantics.

This module deliberately reimplements, in Python, what the planned
`ST_3DFaces` table function should do inside duckdb-3d. Its output schema is
the working contract for that primitive; when it lands, this module reduces
to a SQL query.
"""
from __future__ import annotations

import json
import math
import struct
from dataclasses import dataclass, fields

import duckdb
import numpy as np
import pyarrow as pa

_Z_FLAG = 0x80000000
_SURFACE_TYPES = {15, 6}   # PolyhedralSurface, MultiPolygon
_POLYGON_TYPES = {3}


def _base_type(raw: int) -> int:
    if raw & _Z_FLAG:                      # EWKB Z flag
        return raw & 0x0FFFFFFF
    return raw % 1000 if raw >= 1000 else raw   # ISO: 1015 → 15


class _Reader:
    def __init__(self, buf: bytes) -> None:
        self.buf = buf
        self.pos = 0

    def _take(self, fmt: str):
        vals = struct.unpack_from(fmt, self.buf, self.pos)
        self.pos += struct.calcsize(fmt)
        return vals

    def header(self) -> int:
        (bo,) = self._take("<B")
        (raw,) = self._take("<I" if bo == 1 else ">I")
        self.order = "<" if bo == 1 else ">"
        return _base_type(raw)

    def u32(self) -> int:
        return self._take(f"{self.order}I")[0]

    def ring(self) -> np.ndarray:
        n = self.u32()
        flat = self._take(f"{self.order}{3 * n}d")
        pts = np.asarray(flat, dtype=np.float64).reshape(n, 3)
        if n > 1 and np.array_equal(pts[0], pts[-1]):
            pts = pts[:-1]
        return pts


def parse_wkb_polyhedral(wkb: bytes) -> list[list[np.ndarray]]:
    reader = _Reader(wkb)
    outer = reader.header()
    if outer not in _SURFACE_TYPES:
        raise ValueError(f"unsupported WKB type {outer}; expected PolyhedralSurface/MultiPolygon")
    faces: list[list[np.ndarray]] = []
    for _ in range(reader.u32()):
        inner = reader.header()
        if inner not in _POLYGON_TYPES:
            raise ValueError(f"unsupported WKB type {inner}; expected Polygon")
        faces.append([reader.ring() for _ in range(reader.u32())])
    return faces


@dataclass
class FaceMetrics:
    normal: np.ndarray
    area: float
    tilt_deg: float
    azimuth_deg: float | None
    centroid: np.ndarray


def _newell(ring: np.ndarray) -> np.ndarray:
    nxt = np.roll(ring, -1, axis=0)
    return np.array([
        np.sum((ring[:, 1] - nxt[:, 1]) * (ring[:, 2] + nxt[:, 2])),
        np.sum((ring[:, 2] - nxt[:, 2]) * (ring[:, 0] + nxt[:, 0])),
        np.sum((ring[:, 0] - nxt[:, 0]) * (ring[:, 1] + nxt[:, 1])),
    ])


def _fan_centroid(ring: np.ndarray, unit_normal: np.ndarray) -> tuple[np.ndarray, float]:
    """Accumulate the area-weighted centroid of a ring via fan triangulation
    from v0.

    Returns (unnormalised_centroid_acc, signed_area_sum): the first is the
    sum of each triangle's centroid weighted by its signed area, not yet
    divided by the total weight, so the caller can combine several rings
    (e.g. subtracting holes) before normalising. Divide by signed_area_sum
    to get the actual area-weighted centroid.
    """
    if len(ring) < 3:
        return ring.mean(axis=0), 0.0

    v0 = ring[0]
    centroid_acc = np.array([0.0, 0.0, 0.0])
    area_acc = 0.0

    for i in range(1, len(ring) - 1):
        vi = ring[i]
        vi1 = ring[i + 1]

        # Triangle vertices
        edge1 = vi - v0
        edge2 = vi1 - v0

        # Signed area = dot(cross(edge1, edge2), unit_normal) / 2
        cross = np.cross(edge1, edge2)
        signed_area = float(np.dot(cross, unit_normal)) / 2.0

        # Triangle centroid
        tri_centroid = (v0 + vi + vi1) / 3.0

        # Accumulate weighted by signed area
        centroid_acc += signed_area * tri_centroid
        area_acc += signed_area

    return centroid_acc, area_acc


def face_metrics(rings: list[np.ndarray]) -> FaceMetrics:
    n = _newell(rings[0])
    norm = np.linalg.norm(n)
    unit = n / norm if norm > 0 else np.array([0.0, 0.0, 0.0])

    # Compute area
    exterior_area = float(np.linalg.norm(n)) / 2.0
    holes = sum(float(np.linalg.norm(_newell(r))) / 2.0 for r in rings[1:])
    area = max(exterior_area - holes, 0.0)

    # Compute area-weighted centroid
    ext_centroid_acc, ext_area_sum = _fan_centroid(rings[0], unit)

    # Accumulate weighted centroid
    total_weight = ext_area_sum
    weighted_centroid = ext_centroid_acc

    for hole_ring in rings[1:]:
        hole_centroid_acc, hole_area_sum = _fan_centroid(hole_ring, unit)
        # Subtract hole's contribution
        # Note: hole_area_sum may be negative due to winding; use absolute value for weight
        hole_abs_area = abs(hole_area_sum)
        if hole_abs_area > 1e-12:
            hole_centroid = hole_centroid_acc / hole_area_sum  # divide by signed area to get actual centroid
        else:
            hole_centroid = hole_ring.mean(axis=0)
        total_weight -= hole_abs_area
        weighted_centroid -= hole_abs_area * hole_centroid

    # Compute final centroid
    if abs(total_weight) < 1e-12:
        # Degenerate face: fall back to vertex mean
        centroid = rings[0].mean(axis=0)
    else:
        centroid = weighted_centroid / total_weight

    tilt = float(np.degrees(np.arccos(np.clip(unit[2], -1.0, 1.0))))
    horiz = math.hypot(unit[0], unit[1])
    azimuth = None if horiz < 1e-9 else float(np.degrees(np.arctan2(unit[0], unit[1])) % 360.0)
    return FaceMetrics(
        normal=unit,
        area=area,
        tilt_deg=tilt,
        azimuth_deg=azimuth,
        centroid=centroid,
    )


def resolve_semantics(face_semantics, surfaces_json, n_faces: int) -> list[str]:
    labels = ["Unknown"] * n_faces
    if face_semantics is None or surfaces_json is None:
        return labels
    try:
        surfaces = json.loads(surfaces_json)
    except (TypeError, ValueError):
        return labels
    if not isinstance(surfaces, list):
        return labels
    for i, idx in enumerate(face_semantics[:n_faces]):
        if idx is not None and 0 <= idx < len(surfaces):
            if isinstance(surfaces[idx], dict):
                labels[i] = surfaces[idx].get("type", "Unknown")
    return labels


@dataclass
class FaceRecord:
    building_id: str
    part_id: str
    face_idx: int
    semantic: str
    nx: float
    ny: float
    nz: float
    tilt_deg: float
    azimuth_deg: float | None
    area_m2: float
    cx: float
    cy: float
    cz: float


def faces_for_part(building_id, part_id, wkb, face_semantics, surfaces_json):
    faces = parse_wkb_polyhedral(wkb)
    labels = resolve_semantics(face_semantics, surfaces_json, len(faces))
    records = []
    for idx, (rings, label) in enumerate(zip(faces, labels)):
        m = face_metrics(rings)
        records.append(FaceRecord(
            building_id=building_id, part_id=part_id, face_idx=idx, semantic=label,
            nx=float(m.normal[0]), ny=float(m.normal[1]), nz=float(m.normal[2]),
            tilt_deg=m.tilt_deg, azimuth_deg=m.azimuth_deg, area_m2=m.area,
            cx=float(m.centroid[0]), cy=float(m.centroid[1]), cz=float(m.centroid[2]),
        ))
    return records


def class_areas(records, flat_tilt_deg: float = 5.0) -> dict[str, float]:
    out = {"a_roof_flat_m2": 0.0, "a_roof_pitched_m2": 0.0,
           "a_wall_m2": 0.0, "a_ground_m2": 0.0, "a_other_m2": 0.0}
    for r in records:
        if r.semantic == "RoofSurface":
            key = "a_roof_flat_m2" if r.tilt_deg <= flat_tilt_deg else "a_roof_pitched_m2"
        elif r.semantic == "WallSurface":
            key = "a_wall_m2"
        elif r.semantic == "GroundSurface":
            key = "a_ground_m2"
        else:
            key = "a_other_m2"
        out[key] += r.area_m2
    return out


def compute_face_tables(input_glob: str, lod_suffix: str,
                        flat_tilt_deg: float) -> tuple[pa.Table, pa.Table]:
    con = duckdb.connect()
    rows = con.sql(
        f"""
        SELECT p.parents[1] AS building_id, p.id AS part_id,
               p.geometry_{lod_suffix} AS wkb,
               p.geometry_properties_{lod_suffix}.face_semantics AS fs,
               CAST(p.geometry_properties_{lod_suffix}.surfaces AS VARCHAR) AS sj
        FROM read_parquet($input) p
        WHERE p.object_type = 'BuildingPart' AND p.geometry_{lod_suffix} IS NOT NULL
        ORDER BY building_id, part_id
        """,
        params={"input": input_glob},
    ).fetchall()

    all_records = []
    per_building: dict[str, dict[str, float]] = {}
    for building_id, part_id, wkb, fs, sj in rows:
        records = faces_for_part(building_id, part_id, bytes(wkb), fs, sj)
        all_records.extend(records)
        areas = class_areas(records, flat_tilt_deg)
        acc = per_building.setdefault(building_id, dict.fromkeys(areas, 0.0))
        for key, val in areas.items():
            acc[key] += val

    face_cols = [f.name for f in fields(FaceRecord)]
    faces_table = pa.table(
        {c: [getattr(r, c) for r in all_records] for c in face_cols}
    )
    classes_table = pa.table({
        "building_id": list(per_building),
        **{k: [v[k] for v in per_building.values()]
           for k in ("a_roof_flat_m2", "a_roof_pitched_m2",
                      "a_wall_m2", "a_ground_m2", "a_other_m2")},
    })
    return faces_table, classes_table
