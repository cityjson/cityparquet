import json
import math

from energy.faces import class_areas, faces_for_part, resolve_semantics
from .test_faces_wkb import UNIT_CUBE, make_polyhedral_wkb

CUBE_SURFACES = json.dumps([
    {"type": "GroundSurface"},
    {"type": "WallSurface", "on_footprint_edge": True},
    {"type": "RoofSurface", "b3_azimut": 120.0},
])
CUBE_SEMANTICS = [0, 2, 1, 1, 1, 1]  # bottom, top, 4 walls


def test_resolve_semantics_maps_indices():
    labels = resolve_semantics(CUBE_SEMANTICS, CUBE_SURFACES, 6)
    assert labels == ["GroundSurface", "RoofSurface", "WallSurface",
                      "WallSurface", "WallSurface", "WallSurface"]


def test_resolve_semantics_handles_missing():
    assert resolve_semantics(None, CUBE_SURFACES, 2) == ["Unknown", "Unknown"]
    assert resolve_semantics([0, 99], CUBE_SURFACES, 2) == ["GroundSurface", "Unknown"]
    assert resolve_semantics([0], CUBE_SURFACES, 3) == ["GroundSurface", "Unknown", "Unknown"]


def test_faces_for_part_full_cube():
    records = faces_for_part("B1", "P1", make_polyhedral_wkb(UNIT_CUBE),
                             CUBE_SEMANTICS, CUBE_SURFACES)
    assert len(records) == 6
    assert records[0].semantic == "GroundSurface"
    assert records[1].semantic == "RoofSurface"
    assert all(r.building_id == "B1" and r.part_id == "P1" for r in records)
    assert [r.face_idx for r in records] == list(range(6))
    assert math.isclose(sum(r.area_m2 for r in records), 6.0, rel_tol=1e-12)


def test_class_areas_cube():
    records = faces_for_part("B1", "P1", make_polyhedral_wkb(UNIT_CUBE),
                             CUBE_SEMANTICS, CUBE_SURFACES)
    areas = class_areas(records, flat_tilt_deg=5.0)
    assert math.isclose(areas["a_roof_flat_m2"], 1.0, rel_tol=1e-12)  # flat top
    assert math.isclose(areas["a_roof_pitched_m2"], 0.0, abs_tol=1e-12)
    assert math.isclose(areas["a_wall_m2"], 4.0, rel_tol=1e-12)
    assert math.isclose(areas["a_ground_m2"], 1.0, rel_tol=1e-12)
    assert math.isclose(areas["a_other_m2"], 0.0, abs_tol=1e-12)


def test_class_areas_unknown_goes_to_other():
    records = faces_for_part("B1", "P1", make_polyhedral_wkb(UNIT_CUBE), None, None)
    areas = class_areas(records)
    assert math.isclose(areas["a_other_m2"], 6.0, rel_tol=1e-12)
