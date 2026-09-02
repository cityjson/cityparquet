from pathlib import Path

import pytest

from energy.db import find_extensions
from energy.errors import ExtensionsNotFound


def test_find_extensions_in_explicit_dir(tmp_path):
    (tmp_path / "cityjson.duckdb_extension").touch()
    (tmp_path / "three_d.duckdb_extension").touch()
    found = find_extensions(ext_dir=tmp_path)
    assert found["cityjson"] == tmp_path / "cityjson.duckdb_extension"
    assert found["three_d"] == tmp_path / "three_d.duckdb_extension"


def test_missing_extensions_error_names_paths(tmp_path):
    with pytest.raises(ExtensionsNotFound) as exc:
        find_extensions(ext_dir=tmp_path)
    msg = str(exc.value)
    assert str(tmp_path / "cityjson.duckdb_extension") in msg
    assert str(tmp_path / "three_d.duckdb_extension") in msg


from .conftest import requires_extensions


@requires_extensions
def test_connect_loads_both_extensions():
    from energy.db import connect

    con = connect()
    fns = {r[0] for r in con.sql(
        "SELECT function_name FROM duckdb_functions() "
        "WHERE function_name IN ('st_3dvolume', 'cityjson_metadata')"
    ).fetchall()}
    assert fns == {"st_3dvolume", "cityjson_metadata"}
