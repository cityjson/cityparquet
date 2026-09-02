import struct

import numpy as np
import pytest

from energy.faces import parse_wkb_polyhedral


def make_polyhedral_wkb(polygons, type_code=1015, polygon_code=1003):
    """polygons: list of faces; each face: list of rings; ring: list of (x, y, z)."""
    out = bytearray()
    out += struct.pack("<BII", 1, type_code, len(polygons))
    for rings in polygons:
        out += struct.pack("<BII", 1, polygon_code, len(rings))
        for ring in rings:
            pts = list(ring) + [ring[0]]  # WKB rings close explicitly
            out += struct.pack("<I", len(pts))
            for x, y, z in pts:
                out += struct.pack("<ddd", x, y, z)
    return bytes(out)


UNIT_CUBE = [
    [[(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)]],  # bottom, normal -z
    [[(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)]],  # top, normal +z
    [[(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)]],  # front y=0, normal -y
    [[(1, 1, 0), (0, 1, 0), (0, 1, 1), (1, 1, 1)]],  # back y=1, normal +y
    [[(0, 0, 0), (0, 0, 1), (0, 1, 1), (0, 1, 0)]],  # left x=0, normal -x
    [[(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)]],  # right x=1, normal +x
]


def test_parse_unit_cube():
    faces = parse_wkb_polyhedral(make_polyhedral_wkb(UNIT_CUBE))
    assert len(faces) == 6
    for face in faces:
        assert len(face) == 1               # exterior ring only
        assert face[0].shape == (4, 3)      # closing point removed
    np.testing.assert_allclose(faces[0][0][0], [0.0, 0.0, 0.0])
    np.testing.assert_allclose(faces[5][0][2], [1.0, 1.0, 1.0])


def test_parse_accepts_multipolygon_z():
    faces = parse_wkb_polyhedral(
        make_polyhedral_wkb(UNIT_CUBE, type_code=1006, polygon_code=1003)
    )
    assert len(faces) == 6


def test_parse_rejects_unknown_type():
    bad = struct.pack("<BII", 1, 1001, 0)  # PointZ
    with pytest.raises(ValueError, match="unsupported WKB type"):
        parse_wkb_polyhedral(bad)


def test_parse_keeps_interior_rings():
    face_with_hole = [[
        (0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)],
        [(1, 1, 0), (1, 2, 0), (2, 2, 0), (2, 1, 0)],
    ]
    faces = parse_wkb_polyhedral(make_polyhedral_wkb([face_with_hole]))
    assert len(faces) == 1
    assert len(faces[0]) == 2
