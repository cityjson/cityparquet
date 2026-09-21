import duckdb


def test_fixture_has_buildings_and_parts(fixture_path):
    con = duckdb.connect()
    n = con.sql(
        "SELECT object_type, count(*) FROM read_parquet(?) GROUP BY 1 ORDER BY 1",
        params=[str(fixture_path)],
    ).fetchall()
    kinds = dict(n)
    assert kinds.get("Building", 0) == 150
    assert kinds.get("BuildingPart", 0) >= 150


def test_every_part_links_to_a_building(fixture_path):
    con = duckdb.connect()
    orphans = con.sql(
        """
        SELECT count(*) FROM read_parquet($p) p
        WHERE p.object_type = 'BuildingPart'
          AND p.parents[1] NOT IN
              (SELECT id FROM read_parquet($p) WHERE object_type = 'Building')
        """,
        params={"p": str(fixture_path)},
    ).fetchone()[0]
    assert orphans == 0


def test_parts_have_geometry_and_buildings_have_references(fixture_path):
    con = duckdb.connect()
    row = con.sql(
        """
        SELECT
          count(*) FILTER (object_type = 'BuildingPart' AND geometry_lod2_2 IS NULL),
          count(*) FILTER (object_type = 'Building' AND b3_volume_lod22 IS NULL)
        FROM read_parquet(?)
        """,
        params=[str(fixture_path)],
    ).fetchone()
    assert row == (0, 0)
