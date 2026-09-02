"""ST_3DFaces prototype: per-face metrics from CityParquet WKB + semantics.

This module deliberately reimplements, in Python, what the planned
`ST_3DFaces` table function should do inside duckdb-3d. Its output schema is
the working contract for that primitive; when it lands, this module reduces
to a SQL query.
"""
from __future__ import annotations

import struct

import numpy as np

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
