"""Adapter-level tests for CjdbSystem: the subprocess/env plumbing around
`cjdb import`, and the System.run warm-up-discard contract.

sql_for's own branches are already exhaustively covered without a database
in test_sql_cjdb.py; this file's job is everything sql_for cannot see —
the exact argv and environment handed to the `cjdb` CLI, and how
CjdbSystem wires pg.time_query's return values into a Measurement. Fakes
stand in for psycopg and subprocess so none of this needs a running
container, unlike test_cjdb_integration.py.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from citybench.config import BBox, Dataset, Params
from citybench.systems import cjdb as cjdb_module
from citybench.systems.cjdb import CjdbSystem

PARAMS = Params(
    bbox_full=BBox(0.0, 0.0, 0.0, 100.0, 100.0, 10.0),
    attr_column="object_type",
    attr_eq="Building",
    numeric_column="h_dak_max",
    target_id="obj-1",
    parent_id="obj-0",
    total_city_objects=100,
)


class _FakeCursor:
    def __init__(self, srid_row=(7415,)):
        self.executed: list[tuple[str, tuple]] = []
        self._srid_row = srid_row

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql, args=()):
        self.executed.append((sql, tuple(args) if args else ()))

    def fetchone(self):
        return self._srid_row


class _FakeConnection:
    def __init__(self, srid_row=(7415,)):
        self.cur = _FakeCursor(srid_row)
        self.closed = False

    def cursor(self):
        return self.cur

    def close(self):
        self.closed = True


def _dataset(tmp_path) -> Dataset:
    source = tmp_path / "delft.city.jsonl"
    if not source.exists():
        source.write_text("")
    return Dataset(
        name="delft",
        source=source,
        cityparquet_dir=tmp_path / "cityparquet" / "delft",
        hilbert_dir=tmp_path / "cityparquet-hilbert" / "delft",
    )


_FAKE_PATCHED_SOURCE = "/fake/patched/cjdb-2.2.0+deadbeefcafe"


def _stub_patched_source(monkeypatch, path=_FAKE_PATCHED_SOURCE):
    """CjdbSystem always drives a patched cjdb, never stock cjdb from PyPI
    (see cjdb.py's module docstring) -- prepare()/ingest() both resolve it
    via patched_cjdb_source(), which reads a build artefact
    (.cjdb-patched/current-path) these adapter-level tests must not depend
    on existing. Stubbed here so this file stays a hermetic, offline unit
    test, exactly like every other fake in this module.
    """
    monkeypatch.setattr(cjdb_module, "patched_cjdb_source", lambda: Path(path))


def _system_with_fake_conn(monkeypatch, *, srid_row=(7415,)):
    system = CjdbSystem(port=55432, schema="cjdb")
    fake_conn = _FakeConnection(srid_row)
    monkeypatch.setattr(cjdb_module.pg, "connect", lambda port: fake_conn)
    monkeypatch.setattr(cjdb_module.pg, "vacuum_analyze", lambda conn, schema: None)
    _stub_patched_source(monkeypatch)
    system.prepare()
    return system, fake_conn


def test_tag_is_cjdb():
    assert CjdbSystem.tag == "cjdb"


def test_ingest_invokes_cjdb_import_with_the_filepath_flag_not_positional(tmp_path, monkeypatch):
    # Task 5's verified fact: `cjdb import` has NO positional filepath
    # argument, only -f/--filepath. A regression here would silently pass
    # the source path where cjdb expects nothing, rather than raise.
    captured = {}

    def fake_run(argv, **kwargs):
        captured["argv"] = argv
        captured["kwargs"] = kwargs

    monkeypatch.setattr(cjdb_module.subprocess, "run", fake_run)
    system, _ = _system_with_fake_conn(monkeypatch)
    dataset = _dataset(tmp_path)

    system.ingest(dataset)

    argv = captured["argv"]
    assert argv[argv.index("-f") + 1] == str(dataset.source)
    # Never trust a positional: the path must not appear anywhere except
    # immediately after -f.
    assert argv.count(str(dataset.source)) == 1


def test_ingest_passes_pgpassword_via_environment_not_a_flag(tmp_path, monkeypatch):
    # Task 5's verified fact: cjdb needs PGPASSWORD in the environment; it
    # does not take a password flag.
    captured = {}

    def fake_run(argv, **kwargs):
        captured["argv"] = argv
        captured["kwargs"] = kwargs

    monkeypatch.setattr(cjdb_module.subprocess, "run", fake_run)
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    assert "-P" not in captured["argv"]
    assert "--password" not in captured["argv"]
    assert captured["kwargs"]["env"]["PGPASSWORD"] == "bench"
    assert captured["kwargs"]["check"] is True


def test_ingest_passes_overwrite_port_and_schema_flags(tmp_path, monkeypatch):
    captured = {}
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda argv, **kw: captured.update(argv=argv))
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    argv = captured["argv"]
    assert "--overwrite" in argv
    assert argv[argv.index("-s") + 1] == "cjdb"
    assert argv[argv.index("-p") + 1] == "55432"
    assert argv[argv.index("-d") + 1] == "bench"
    assert argv[argv.index("-U") + 1] == "bench"


def test_ingest_extracts_srid_from_cj_metadata_and_creates_every_index(tmp_path, monkeypatch):
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda *a, **kw: None)
    system, fake_conn = _system_with_fake_conn(monkeypatch, srid_row=(7415,))

    system.ingest(_dataset(tmp_path))

    assert system._srid == 7415
    executed_sql = [sql for sql, _ in fake_conn.cur.executed]
    assert any("cj_metadata" in sql for sql in executed_sql)
    for ddl in cjdb_module.sql_cjdb.index_ddl():
        assert ddl in executed_sql


def test_ingest_defaults_srid_to_zero_when_cj_metadata_has_none(tmp_path, monkeypatch):
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch, srid_row=(None,))

    system.ingest(_dataset(tmp_path))

    assert system._srid == 0


def test_ingest_defaults_srid_to_zero_when_cj_metadata_is_empty(tmp_path, monkeypatch):
    # No row at all (fetchone() -> None) is a distinct case from a NULL
    # srid column; both must fall back to 0 rather than raise.
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch, srid_row=None)

    system.ingest(_dataset(tmp_path))

    assert system._srid == 0


def test_ingest_calls_vacuum_analyze_on_the_cjdb_schema(tmp_path, monkeypatch):
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda *a, **kw: None)
    calls = []
    system = CjdbSystem(port=55432, schema="cjdb")
    fake_conn = _FakeConnection()
    monkeypatch.setattr(cjdb_module.pg, "connect", lambda port: fake_conn)
    monkeypatch.setattr(
        cjdb_module.pg, "vacuum_analyze",
        lambda conn, schema: calls.append((conn, schema)),
    )
    _stub_patched_source(monkeypatch)
    system.prepare()

    system.ingest(_dataset(tmp_path))

    assert calls == [(fake_conn, "cjdb")]


def test_ingest_reports_the_import_wall_clock(tmp_path, monkeypatch):
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch)

    result = system.ingest(_dataset(tmp_path))

    assert result.wall_clock_s >= 0.0


def test_run_discards_the_warmup_and_reports_repeat_samples(monkeypatch):
    system, _ = _system_with_fake_conn(monkeypatch)
    responses = iter([
        (999, 9.0, 9.0),   # discarded warm-up: the most extreme values
        (5, 0.1, 0.05),
        (5, 0.2, 0.08),
        (5, 0.05, 0.02),
    ])
    monkeypatch.setattr(
        cjdb_module.pg, "time_query",
        lambda conn, sql, args, count_mode: next(responses),
    )

    measurement = system.run("count", PARAMS, repeat=3)

    assert measurement.result_count == 5
    assert measurement.times_s == [0.1, 0.2, 0.05]
    assert measurement.server_times_s == [0.05, 0.08, 0.02]
    assert measurement.peak_rss_bytes is None
    assert measurement.peak_heap_bytes is None


def test_run_passes_the_stored_srid_into_sql_for(tmp_path, monkeypatch):
    # sql_for's bbox-query args end in the SRID; the adapter must forward
    # the value ingest() captured from cj_metadata, not the module-level
    # placeholder sql_cjdb.SRID_PLACEHOLDER.
    monkeypatch.setattr(cjdb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch, srid_row=(28992,))
    system.ingest(_dataset(tmp_path))

    captured_args = {}

    def fake_time_query(conn, sql, args, count_mode):
        captured_args["args"] = args
        return (1, 0.01, 0.005)

    monkeypatch.setattr(cjdb_module.pg, "time_query", fake_time_query)

    system.run("bbox-query", PARAMS, repeat=1, selectivity=0.25)

    assert captured_args["args"][-1] == 28992


def test_run_raises_scenario_unavailable_for_hierarchy_without_a_parent_id(monkeypatch):
    from citybench.scenarios.registry import ScenarioUnavailable

    system, _ = _system_with_fake_conn(monkeypatch)
    params = Params(
        bbox_full=PARAMS.bbox_full, attr_column=PARAMS.attr_column,
        attr_eq=PARAMS.attr_eq, numeric_column=PARAMS.numeric_column,
        target_id=PARAMS.target_id, parent_id=None,
        total_city_objects=PARAMS.total_city_objects,
    )
    with pytest.raises(ScenarioUnavailable):
        system.run("hierarchy", params, repeat=1)


def test_teardown_closes_the_connection_and_is_safe_to_call_twice(monkeypatch):
    system, fake_conn = _system_with_fake_conn(monkeypatch)
    system.teardown()
    assert fake_conn.closed is True
    system.teardown()  # must not raise on a second call


# --- Patched-cjdb wiring ----------------------------------------------
#
# CjdbSystem never drives stock cjdb from PyPI — see cjdb.py's module
# docstring and vendor/cjdb/README.md for why (a real footprint-dropping
# bug, patched and disclosed rather than silently benchmarked). These
# tests pin the wiring that makes that true, independent of
# test_cjdb_patch.py's own (network/build-dependent) proof that the patch
# itself works.


def test_ingest_uses_the_patched_source_not_the_bare_pypi_name(tmp_path, monkeypatch):
    captured = {}
    monkeypatch.setattr(
        cjdb_module.subprocess, "run", lambda argv, **kw: captured.update(argv=argv)
    )
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    argv = captured["argv"]
    assert argv[:4] == ["uv", "run", "--with", _FAKE_PATCHED_SOURCE]
    # The whole point: never the bare PyPI package name, which would
    # silently run stock (buggy) cjdb instead of the patched build.
    assert argv[3] != "cjdb"


def test_prepare_raises_when_the_patched_source_is_not_built(monkeypatch):
    # Mirrors ReadbenchSystem's own missing-binary check: fail fast, with
    # a clear "run this command" message, rather than a confusing failure
    # deep inside `cjdb import`.
    def _raise():
        raise FileNotFoundError("build the patched cjdb with `just patch-cjdb` first")

    monkeypatch.setattr(cjdb_module, "patched_cjdb_source", _raise)
    monkeypatch.setattr(cjdb_module.pg, "connect", lambda port: _FakeConnection())

    system = CjdbSystem()
    with pytest.raises(FileNotFoundError, match="just patch-cjdb"):
        system.prepare()


def test_ingest_raises_when_the_patched_source_is_not_built(tmp_path, monkeypatch):
    def _raise():
        raise FileNotFoundError("build the patched cjdb with `just patch-cjdb` first")

    monkeypatch.setattr(cjdb_module, "patched_cjdb_source", _raise)
    monkeypatch.setattr(cjdb_module.pg, "connect", lambda port: _FakeConnection())
    system = CjdbSystem()
    system.prepare = lambda: None  # bypass prepare's own check for this test
    with pytest.raises(FileNotFoundError, match="just patch-cjdb"):
        system.ingest(_dataset(tmp_path))


def test_patch_disclosure_reports_upstream_version_and_patch_file(monkeypatch):
    _stub_patched_source(monkeypatch)
    disclosure = cjdb_module.patch_disclosure()
    assert disclosure["upstream_version"] == cjdb_module.CJDB_UPSTREAM_VERSION
    assert disclosure["patched"] == "true"
    assert disclosure["patch_file"] == "vendor/cjdb/ground-surfaces-tie.patch"
    assert disclosure["built_from"] == _FAKE_PATCHED_SOURCE
