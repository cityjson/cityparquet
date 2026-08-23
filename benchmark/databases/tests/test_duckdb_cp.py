"""Pure-function tests for the duckdb-cityparquet table resolution.

``object_table_files``/``_table`` only ever read ``metadata.json`` off
disk — no DuckDB connection or real Parquet data is needed to exercise the
branch that matters: single-family packages (delft-shaped, one object
table) versus multi-family, by-type packages (lod3_railway-shaped,
several object tables with no Building table at all).
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from citybench.systems.duckdb_cp import DuckDBCityParquet, object_table_files


def _write_manifest(package: Path, assets: dict) -> None:
    package.mkdir(parents=True, exist_ok=True)
    (package / "metadata.json").write_text(json.dumps({"assets": assets}))


def test_single_family_package_resolves_its_one_object_table(tmp_path):
    package = tmp_path / "delft"
    _write_manifest(package, {
        "data": {"href": "./building.parquet", "roles": ["data"]},
        "building.parquet": {
            "href": "./building.parquet",
            "roles": ["data", "cityparquet-objects"],
        },
    })
    files = object_table_files(package)
    assert files == [str((package / "building.parquet").resolve())]


def test_multi_family_package_resolves_every_object_table(tmp_path):
    # lod3_railway-shaped: no Building table exists at all — an EARLIER
    # version of this system hardcoded 'building.parquet', which would
    # have failed outright here (see object_table_files's own docstring).
    package = tmp_path / "lod3_railway"
    _write_manifest(package, {
        "data": {"href": "./railway.parquet", "roles": ["data"]},
        "railway.parquet": {
            "href": "./railway.parquet",
            "roles": ["data", "cityparquet-objects"],
        },
        "bridge.parquet": {
            "href": "./bridge.parquet",
            "roles": ["cityparquet-objects"],
        },
        "tunnel.parquet": {
            "href": "./tunnel.parquet",
            "roles": ["cityparquet-objects"],
        },
        "materials.parquet": {
            "href": "./materials.parquet",
            "roles": ["cityparquet-materials"],
        },
    })
    files = object_table_files(package)
    assert files == sorted(
        str((package / name).resolve())
        for name in ("railway.parquet", "bridge.parquet", "tunnel.parquet")
    )
    assert not any("materials" in f for f in files)


def test_no_object_table_asset_raises_clearly(tmp_path):
    package = tmp_path / "broken"
    _write_manifest(package, {
        "data": {"href": "./building.parquet", "roles": ["data"]},
    })
    with pytest.raises(ValueError, match="cityparquet-objects"):
        object_table_files(package)


def test_table_sql_is_a_bare_read_parquet_for_a_single_object_table(tmp_path):
    package = tmp_path / "delft"
    _write_manifest(package, {
        "building.parquet": {
            "href": "./building.parquet",
            "roles": ["cityparquet-objects"],
        },
    })
    system = DuckDBCityParquet()
    system._package = package
    sql = system._table()
    assert sql.startswith("read_parquet('")
    assert "union_by_name" not in sql
    assert "building.parquet" in sql


def test_table_sql_unions_by_name_across_every_object_table(tmp_path):
    package = tmp_path / "lod3_railway"
    _write_manifest(package, {
        "railway.parquet": {"href": "./railway.parquet", "roles": ["cityparquet-objects"]},
        "bridge.parquet": {"href": "./bridge.parquet", "roles": ["cityparquet-objects"]},
    })
    system = DuckDBCityParquet()
    system._package = package
    sql = system._table()
    assert sql.startswith("read_parquet([")
    assert "union_by_name = true" in sql
    assert "railway.parquet" in sql
    assert "bridge.parquet" in sql


def test_column_names_discovers_the_real_schema_and_caches_it(tmp_path):
    # A real DuckDB connection this time: `_column_names` exists precisely
    # so `run()` can build LoD-aware SQL against THIS package's own
    # columns (see sql_duckdb.sql_for's own docstring for the Montreal
    # BinderException this was written to fix) — that requires an actual
    # schema lookup, not just path bookkeeping.
    import duckdb as duckdb_module

    package = tmp_path / "montreal-shaped"
    package.mkdir()
    duckdb_module.connect().execute(
        "COPY (SELECT 'a' AS id, 'x'::BLOB AS geometry_lod0_0) "
        f"TO '{package / 'building.parquet'}' (FORMAT PARQUET)"
    )
    _write_manifest(package, {
        "building.parquet": {"href": "./building.parquet", "roles": ["cityparquet-objects"]},
    })

    system = DuckDBCityParquet()
    system.prepare()
    system._package = package
    try:
        columns = system._column_names()
        assert columns == frozenset({"id", "geometry_lod0_0"})

        # Caching: a second call must not re-run DESCRIBE against the
        # connection — proven by closing it first and confirming no error
        # (a real re-query against a closed DuckDB connection raises).
        system._conn.close()
        assert system._column_names() == columns
    finally:
        if system._conn is not None:
            system._conn.close()
