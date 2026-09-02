import duckdb

from energy.cli import main
from .conftest import requires_extensions


@requires_extensions
def test_features_then_screen_end_to_end(fixture_path, tmp_path, capsys):
    features = tmp_path / "features.parquet"
    rc = main(["features", "--input", str(fixture_path),
               "--output", str(features)])
    assert rc == 0
    assert "150 buildings" in capsys.readouterr().out

    screen = tmp_path / "screen.parquet"
    rc = main(["screen", "--features", str(features),
               "--year-before", "2100", "--top", "10",
               "--output", str(screen)])
    assert rc == 0
    table = duckdb.sql(f"FROM '{screen}'").to_arrow_table()
    assert table.num_rows == 10
    assert "annual_kwh" in table.column_names


def test_features_bad_lod_is_a_clean_error(fixture_path, capsys):
    rc = main(["features", "--input", str(fixture_path), "--lod", "9.9",
               "--output", "/dev/null"])
    assert rc == 1
    assert "available" in capsys.readouterr().err
