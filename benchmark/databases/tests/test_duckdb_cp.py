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

from citybench.config import Dataset
from citybench.systems import duckdb_cp
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
        columns = system._column_types()
        # Names AND types: `geometry-scan` needs the types, because
        # `octet_length` binds against a BLOB column but not against
        # DuckDB's native GEOMETRY.
        assert set(columns) == {"id", "geometry_lod0_0"}
        assert columns["id"] == "VARCHAR"

        # Caching: a second call must not re-run DESCRIBE against the
        # connection — proven by closing it first and confirming no error
        # (a real re-query against a closed DuckDB connection raises).
        system._conn.close()
        assert system._column_types() == columns
    finally:
        if system._conn is not None:
            system._conn.close()


def test_prepare_sets_duckdb_temporary_directory_explicitly(tmp_path, monkeypatch):
    commands = []

    class Connection:
        def execute(self, sql):
            commands.append(sql)

    monkeypatch.setenv("CITYBENCH_DUCKDB_TMPDIR", str(tmp_path))
    monkeypatch.setattr(duckdb_cp.duckdb, "connect", lambda: Connection())

    DuckDBCityParquet().prepare()

    assert f"SET temp_directory = '{tmp_path.resolve()}'" in commands


def _dataset_for_package(tmp_path: Path, package: Path) -> Dataset:
    """Both package slots point at the same directory.

    `duckdb-cityparquet` reads the HILBERT package by default now — the one
    the format family's figures call "CityParquet" — and only the
    `duckdb-cityparquet-source` tag reads the source-order one. A fixture
    that filled just `cityparquet_dir` would exercise neither default.
    """
    source = tmp_path / "source.city.jsonl"
    source.write_text("")
    return Dataset(
        name="fixture",
        source=source,
        cityparquet_dir=package,
        hilbert_dir=package,
    )


def test_the_default_package_is_hilbert_and_the_source_tag_reads_source_order(tmp_path):
    hilbert = tmp_path / "hilbert"
    source_order = tmp_path / "source-order"
    for package in (hilbert, source_order):
        _write_manifest(package, {
            "building.parquet": {"href": "./building.parquet",
                                 "roles": ["cityparquet-objects"]},
        })
        (package / "building.parquet").write_bytes(b"")
    dataset = Dataset(
        name="fixture", source=tmp_path / "s.city.jsonl",
        cityparquet_dir=source_order, hilbert_dir=hilbert,
    )
    (tmp_path / "s.city.jsonl").write_text("")

    default = DuckDBCityParquet()
    assert default.tag == "duckdb-cityparquet"
    default.ingest(dataset)
    assert default._package == hilbert

    ordering = DuckDBCityParquet(package="source")
    assert ordering.tag == "duckdb-cityparquet-source"
    ordering.ingest(dataset)
    assert ordering._package == source_order

    writeback = DuckDBCityParquet(writeback=True)
    assert writeback.tag == "duckdb-cityparquet-writeback"
    writeback.ingest(dataset)
    assert writeback._package == hilbert


def test_prepare_turns_off_the_geoparquet_footer_conversion(tmp_path, monkeypatch):
    """The committed run left this at its default (true). It governs the
    `geo`-footer conversion path; it does NOT re-type `geometry_lod0_0`,
    which CityParquet writes with Parquet's own GEOMETRY logical type
    (measured — README Caveat 18)."""
    commands = []

    class Connection:
        def execute(self, sql):
            commands.append(sql)

    monkeypatch.setattr(duckdb_cp.duckdb, "connect", lambda: Connection())
    DuckDBCityParquet().prepare()
    assert "SET enable_geoparquet_conversion = false" in commands


def test_the_primary_thread_configuration_is_one_thread(tmp_path, monkeypatch):
    """The committed run gave DuckDB 16 threads against a PostgreSQL with
    parallel query disabled, which concentrated a 5-8x advantage on exactly
    the headline rows (review §4.3). One thread is the primary figure; the
    parallel pass is a disclosed second column."""
    commands = []

    class Connection:
        def execute(self, sql):
            commands.append(sql)

    monkeypatch.setattr(duckdb_cp.duckdb, "connect", lambda: Connection())
    system = DuckDBCityParquet()
    system.prepare()
    assert "SET threads TO 1" in commands

    system.set_threads(16)
    assert "SET threads TO 16" in commands


def test_ingest_fails_before_other_systems_for_a_missing_package(tmp_path):
    system = DuckDBCityParquet()

    with pytest.raises(FileNotFoundError, match="metadata.json"):
        system.ingest(_dataset_for_package(tmp_path, tmp_path / "missing"))


def test_ingest_rejects_a_package_with_a_missing_object_table_asset(tmp_path):
    package = tmp_path / "prepared"
    _write_manifest(package, {
        "building.parquet": {
            "href": "./building.parquet",
            "roles": ["cityparquet-objects"],
        },
    })

    with pytest.raises(FileNotFoundError, match="building.parquet"):
        DuckDBCityParquet().ingest(_dataset_for_package(tmp_path, package))


def test_ingest_accepts_a_package_with_existing_object_table_assets(tmp_path):
    package = tmp_path / "prepared"
    _write_manifest(package, {
        "building.parquet": {
            "href": "./building.parquet",
            "roles": ["cityparquet-objects"],
        },
    })
    (package / "building.parquet").touch()
    system = DuckDBCityParquet()

    result = system.ingest(_dataset_for_package(tmp_path, package))

    assert result.wall_clock_s == 0.0
    assert system._package == package


# --- The two parts-per-building forms, on a real package ------------------
#
# CJDB's Q4 reports one row per Building INCLUDING childless ones. The
# natural form reads a stored `children` array; the join form unnests
# `parents` and LEFT JOINs the Buildings back. Publishing the two side by
# side as "the cost of normalisation" is only honest if they return the
# SAME rows, which the review's §7 specifically called out: an earlier
# proposal's join form dropped childless Buildings and so compared
# different result sets.

_REAL_PACKAGE = (
    Path(__file__).resolve().parents[2] / "runs" / "data" / "readbench"
    / "3dbag_n1000.parquet"
)


@pytest.mark.skipif(
    not (_REAL_PACKAGE / "metadata.json").exists(),
    reason=f"{_REAL_PACKAGE} not prepared; run `just bench-prep` first",
)
def test_both_parts_per_building_forms_return_identical_row_sets():
    from citybench.scenarios.sql_duckdb import sql_for
    from conftest import make_params

    system = DuckDBCityParquet()
    system.prepare()
    system._package = _REAL_PACKAGE
    try:
        params = make_params()
        table = system._table()
        columns = system._column_types()

        natural_sql, natural_args = sql_for(
            "parts-per-building", params, table, columns=columns
        )
        join_sql, join_args = sql_for(
            "parts-per-building-join", params, table, columns=columns
        )
        natural = system._conn.execute(natural_sql, list(natural_args)).fetchall()
        joined = system._conn.execute(join_sql, list(join_args)).fetchall()
    finally:
        if system._conn is not None:
            system._conn.close()

    assert natural, "the fixture must contain Buildings for this to mean anything"
    # Identical rows, not merely identical counts: same ids, same child
    # counts, childless Buildings included with 0 rather than NULL.
    assert sorted((row[0], int(row[1])) for row in natural) == sorted(
        (row[0], int(row[1])) for row in joined
    )
    # Honest about what this fixture does and does not exercise: every
    # Building in the `3dbag_n1000` slice has exactly one BuildingPart, so
    # the CHILDLESS case is covered by the pure-function tests
    # (`test_sql_duckdb.test_parts_per_building_keeps_childless_buildings`)
    # and by the `coalesce`/`LEFT JOIN`/`count(child.id)` shapes they pin,
    # not by this data.
    assert {count for _, count in ((r[0], int(r[1])) for r in natural)} == {1}
