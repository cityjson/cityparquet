import duckdb
import pytest

from energy.faces import compute_face_tables
from .conftest import requires_extensions

EXPECTED_COLS = {
    "building_id", "year", "n_parts", "volume_m3", "envelope_m2", "sv_ratio",
    "footprint_m2", "height_m", "is_closed",
    "a_roof_flat_m2", "a_roof_pitched_m2", "a_wall_m2", "a_ground_m2", "a_other_m2",
}


def test_compute_face_tables_shapes(fixture_path):
    faces, classes = compute_face_tables(str(fixture_path), "lod2_2", 5.0)
    assert classes.num_rows == 150
    assert faces.num_rows > classes.num_rows          # many faces per building
    assert set(classes.column_names) == {
        "building_id", "a_roof_flat_m2", "a_roof_pitched_m2",
        "a_wall_m2", "a_ground_m2", "a_other_m2",
    }


def test_class_areas_match_3dbag_references(fixture_path):
    _, classes = compute_face_tables(str(fixture_path), "lod2_2", 5.0)
    con = duckdb.connect()
    con.register("classes", classes)
    median_err = con.sql(
        """
        SELECT median(abs((c.a_ground_m2 - b.b3_opp_grond) / nullif(b.b3_opp_grond, 0)))
        FROM classes c
        JOIN read_parquet(?) b ON b.id = c.building_id
        WHERE b.object_type = 'Building'
        """,
        params=[str(fixture_path)],
    ).fetchone()[0]
    assert median_err < 0.05, f"median ground-area error {median_err:.4f}"


@requires_extensions
def test_run_features_end_to_end(fixture_path, tmp_path):
    from energy.run import run_features

    out = tmp_path / "features.parquet"
    summary = run_features(str(fixture_path), "2.2", str(out),
                           faces_out=None, validate_out=None, flat_tilt_deg=5.0)
    assert summary.n_buildings == 150
    assert summary.n_parts >= 150
    table = duckdb.sql(f"FROM '{out}'").to_arrow_table()
    assert set(table.column_names) == EXPECTED_COLS
    assert table.num_rows == 150


@requires_extensions
def test_run_features_writes_faces_table(fixture_path, tmp_path):
    from energy.run import run_features

    out = tmp_path / "features.parquet"
    faces_out = tmp_path / "faces.parquet"
    run_features(str(fixture_path), "2.2", str(out),
                 faces_out=str(faces_out), validate_out=None, flat_tilt_deg=5.0)
    faces = duckdb.sql(f"FROM '{faces_out}'").to_arrow_table()
    assert {"building_id", "part_id", "face_idx", "semantic",
            "tilt_deg", "azimuth_deg", "area_m2"} <= set(faces.column_names)


@requires_extensions
def test_missing_geometry_buildings_are_counted(fixture_path, tmp_path):
    """A Building whose parts ALL lack usable geometry at the requested LoD
    is dropped by build_features' inner join; run_features must count it."""
    from energy.run import run_features

    con = duckdb.connect()
    building_id = con.sql(
        "SELECT id FROM read_parquet($p) WHERE object_type = 'Building' "
        "ORDER BY id LIMIT 1",
        params={"p": str(fixture_path)},
    ).fetchone()[0]
    part_id = con.sql(
        "SELECT id FROM read_parquet($p) WHERE object_type = 'BuildingPart' "
        "AND parents[1] = $b LIMIT 1",
        params={"p": str(fixture_path), "b": building_id},
    ).fetchone()[0]

    new_building_id = building_id + "-nogeom"
    new_part_id = part_id + "-nogeom"
    tmp_input = tmp_path / "with_missing.parquet"
    con.execute(
        """
        COPY (
          SELECT * FROM read_parquet($fixture)
          UNION ALL BY NAME
          SELECT * REPLACE ($nb AS id)
          FROM read_parquet($fixture) WHERE id = $b AND object_type = 'Building'
          UNION ALL BY NAME
          SELECT * REPLACE ($np AS id, NULL AS geometry_lod2_2, [$nb] AS parents)
          FROM read_parquet($fixture) WHERE id = $p AND object_type = 'BuildingPart'
        ) TO $out (FORMAT PARQUET, COMPRESSION ZSTD)
        """,
        {"fixture": str(fixture_path), "b": building_id, "p": part_id,
         "nb": new_building_id, "np": new_part_id, "out": str(tmp_input)},
    )

    out = tmp_path / "features.parquet"
    summary = run_features(str(tmp_input), "2.2", str(out),
                           faces_out=None, validate_out=None, flat_tilt_deg=5.0)
    assert summary.n_buildings_missing_geometry == 1
    assert summary.n_buildings == 150
