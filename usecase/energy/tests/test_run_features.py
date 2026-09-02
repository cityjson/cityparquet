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


@requires_extensions
def test_multipart_building_aggregates(fixture_path, tmp_path):
    """The fixture and the evidence tile are both 1:1 Building:BuildingPart,
    so features.py's per-building SQL aggregation (sum volumes/areas,
    bool_and(closed), max(zmax)-min(zmin)) and faces.py's per_building
    class-area accumulation have never executed with more than one part per
    building. Give one building a duplicated part and check both metric
    paths sum across parts rather than only ever seeing one."""
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

    # Baseline: this building's single-part metrics on the unmodified fixture.
    baseline_out = tmp_path / "baseline.parquet"
    run_features(str(fixture_path), "2.2", str(baseline_out),
                 faces_out=None, validate_out=None, flat_tilt_deg=5.0)
    base_volume, base_wall, base_ground = con.sql(
        "SELECT volume_m3, a_wall_m2, a_ground_m2 FROM read_parquet($p) "
        "WHERE building_id = $b",
        params={"p": str(baseline_out), "b": building_id},
    ).fetchone()

    # Give the chosen building a second part: a copy of its one BuildingPart
    # row, same parents, same geometry, new id.
    new_part_id = part_id + "-dup"
    tmp_input = tmp_path / "with_dup_part.parquet"
    con.execute(
        """
        COPY (
          SELECT * FROM read_parquet($fixture)
          UNION ALL BY NAME
          SELECT * REPLACE ($np AS id)
          FROM read_parquet($fixture) WHERE id = $p AND object_type = 'BuildingPart'
        ) TO $out (FORMAT PARQUET, COMPRESSION ZSTD)
        """,
        {"fixture": str(fixture_path), "p": part_id, "np": new_part_id,
         "out": str(tmp_input)},
    )

    out = tmp_path / "features.parquet"
    summary = run_features(str(tmp_input), "2.2", str(out),
                           faces_out=None, validate_out=None, flat_tilt_deg=5.0)
    assert summary.n_buildings == 150
    assert summary.n_buildings_missing_geometry == 0

    table = duckdb.sql(f"FROM '{out}'").to_arrow_table()
    assert table.num_rows == 150

    n_parts, volume_m3, a_wall_m2, a_ground_m2 = con.sql(
        "SELECT n_parts, volume_m3, a_wall_m2, a_ground_m2 "
        "FROM read_parquet($p) WHERE building_id = $b",
        params={"p": str(out), "b": building_id},
    ).fetchone()
    assert n_parts == 2
    assert volume_m3 == pytest.approx(2 * base_volume, rel=1e-6)
    assert a_wall_m2 == pytest.approx(2 * base_wall, rel=1e-6)
    assert a_ground_m2 == pytest.approx(2 * base_ground, rel=1e-6)


@requires_extensions
def test_run_features_validate_report(fixture_path, tmp_path):
    import json

    from energy.run import run_features

    report_path = tmp_path / "report.json"
    run_features(str(fixture_path), "2.2", str(tmp_path / "f.parquet"),
                 faces_out=None, validate_out=str(report_path), flat_tilt_deg=5.0)
    report = json.loads(report_path.read_text())
    assert report["volume"]["median_rel_err_pct"] < 1.0
    assert report["ground"]["median_rel_err_pct"] < 5.0
