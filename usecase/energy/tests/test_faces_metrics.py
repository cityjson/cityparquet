import math

import numpy as np

from energy.faces import face_metrics, parse_wkb_polyhedral
from .test_faces_wkb import UNIT_CUBE, make_polyhedral_wkb


def _cube_faces():
    return parse_wkb_polyhedral(make_polyhedral_wkb(UNIT_CUBE))


def test_cube_top_face():
    m = face_metrics(_cube_faces()[1])
    assert math.isclose(m.area, 1.0, rel_tol=1e-12)
    assert math.isclose(m.tilt_deg, 0.0, abs_tol=1e-9)
    assert m.azimuth_deg is None
    np.testing.assert_allclose(m.normal, [0, 0, 1], atol=1e-12)
    np.testing.assert_allclose(m.centroid, [0.5, 0.5, 1.0], atol=1e-12)


def test_cube_bottom_face_points_down():
    m = face_metrics(_cube_faces()[0])
    assert math.isclose(m.tilt_deg, 180.0, abs_tol=1e-9)
    np.testing.assert_allclose(m.normal, [0, 0, -1], atol=1e-12)


def test_cube_wall_azimuths():
    faces = _cube_faces()
    # back face (+y normal) → azimuth 0 (north); right face (+x) → 90 (east)
    assert math.isclose(face_metrics(faces[3]).azimuth_deg, 0.0, abs_tol=1e-9)
    assert math.isclose(face_metrics(faces[5]).azimuth_deg, 90.0, abs_tol=1e-9)
    assert math.isclose(face_metrics(faces[2]).azimuth_deg, 180.0, abs_tol=1e-9)
    assert math.isclose(face_metrics(faces[4]).azimuth_deg, 270.0, abs_tol=1e-9)
    for i in (2, 3, 4, 5):
        assert math.isclose(face_metrics(faces[i]).tilt_deg, 90.0, abs_tol=1e-9)


def test_pitched_roof_45_degrees():
    ring = np.array([(0, 0, 0), (1, 0, 0), (1, 1, 1), (0, 1, 1)], dtype=float)
    m = face_metrics([ring])
    assert math.isclose(m.tilt_deg, 45.0, abs_tol=1e-9)
    assert math.isclose(m.area, math.sqrt(2.0), rel_tol=1e-12)


def test_hole_subtracts_area():
    exterior = np.array([(0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)], dtype=float)
    hole = np.array([(1, 1, 0), (1, 2, 0), (2, 2, 0), (2, 1, 0)], dtype=float)
    m = face_metrics([exterior, hole])
    assert math.isclose(m.area, 16.0 - 1.0, rel_tol=1e-12)


def test_concave_face_centroid_is_area_weighted():
    # L-shaped face (two 1×1 squares joined)
    ring = np.array([(0, 0, 0), (2, 0, 0), (2, 1, 0), (1, 1, 0), (1, 2, 0), (0, 2, 0)], dtype=float)
    m = face_metrics([ring])
    assert math.isclose(m.area, 3.0, rel_tol=1e-12)
    # Area-weighted centroid: (2.5/3, 2.5/3, 0)
    expected_centroid = np.array([2.5/3.0, 2.5/3.0, 0.0])
    np.testing.assert_allclose(m.centroid, expected_centroid, atol=1e-9)
    # Verify it is NOT the vertex mean (which would be (1, 1, 0))
    vertex_mean = ring.mean(axis=0)
    assert not np.allclose(m.centroid, vertex_mean, atol=1e-9)


def test_hole_shifts_centroid():
    # 4×4 square with 1×1 hole in the middle-left
    exterior = np.array([(0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)], dtype=float)
    hole = np.array([(1, 1, 0), (1, 2, 0), (2, 2, 0), (2, 1, 0)], dtype=float)
    m = face_metrics([exterior, hole])
    assert math.isclose(m.area, 15.0, rel_tol=1e-12)
    # Net area 15: exterior 16 at centroid (2, 2), hole 1 at centroid (1.5, 1.5)
    # Weighted centroid = (16 * 2 - 1 * 1.5) / 15 = (32 - 1.5) / 15 = 30.5 / 15 ≈ 2.033333
    expected_centroid_x = 30.5 / 15.0
    expected_centroid_y = 30.5 / 15.0
    np.testing.assert_allclose(m.centroid[:2], [expected_centroid_x, expected_centroid_y], atol=1e-9)
