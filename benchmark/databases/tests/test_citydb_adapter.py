"""Adapter-level tests for CityDbSystem: the subprocess/env plumbing around
`citydb-tool import cityjson`, and the System.run warm-up-discard contract.

sql_for's own branches are already exhaustively covered without a database
in test_sql_citydb.py; this file's job is everything sql_for cannot see —
the exact argv handed to the pinned `citydb-tool` container image (podman,
not docker; `--threads=4`; no positional path baked in twice), and how
CityDbSystem wires pg.time_query's return values into a Measurement. Fakes
stand in for psycopg and subprocess so none of this needs a running
container, unlike test_citydb_integration.py.
"""

from __future__ import annotations

import pytest

from citybench.config import BBox, Dataset, Params
from citybench.systems import citydb as citydb_module
from citybench.systems.citydb import CityDbSystem

PARAMS = Params(
    bbox_full=BBox(0.0, 0.0, 0.0, 100.0, 100.0, 10.0),
    attr_column="object_type",
    attr_eq="BuildingPart",
    numeric_column="b3_h_dak_50p",
    target_id="obj-1",
    parent_id="obj-0",
    total_city_objects=100,
)


# A small, arbitrary stand-in for the real, live-resolved objectclass id
# set (89 ids on the real schema — see sql_citydb.resolve_cityobject_class_ids's
# own docstring; test_citydb_integration.py proves the real value). What
# matters for these fake-conn adapter tests is only that ingest() ends up
# with SOME non-empty tuple cached on the instance, so run() can build a
# valid predicate afterwards.
_FAKE_CLASS_IDS = ((901,), (902,))


class _FakeCursor:
    def __init__(self, srid_row=(7415,), class_id_rows=_FAKE_CLASS_IDS):
        self.executed: list[tuple[str, tuple]] = []
        self._srid_row = srid_row
        self._class_id_rows = class_id_rows

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql, args=()):
        self.executed.append((sql, tuple(args) if args else ()))

    def fetchone(self):
        return self._srid_row

    def fetchall(self):
        # Only resolve_cityobject_class_ids() calls fetchall() on this fake
        # (the SRID lookup uses fetchone()) — always returns the same
        # canned class-id rows regardless of the query text, matching this
        # fake's existing "canned response regardless of SQL" style.
        return self._class_id_rows


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


def _system_with_fake_conn(monkeypatch, *, srid_row=(7415,)):
    system = CityDbSystem(port=55433, schema="citydb")
    fake_conn = _FakeConnection(srid_row)
    monkeypatch.setattr(citydb_module.pg, "connect", lambda port: fake_conn)
    monkeypatch.setattr(citydb_module.pg, "vacuum_analyze", lambda conn, schema: None)
    system.prepare()
    return system, fake_conn


def test_tag_is_3dcitydb():
    assert CityDbSystem.tag == "3dcitydb"


def test_ingest_invokes_podman_not_docker(tmp_path, monkeypatch):
    # GLOBAL CONSTRAINT: the container runtime is rootless podman, not
    # docker — the `docker` binary happens to exist on this host too
    # (a separate, unrelated rootful daemon), but the harness's stack is
    # brought up by `just up` -> podman-compose, so the CLI invocation
    # must agree.
    captured = {}

    def fake_run(argv, **kwargs):
        captured["argv"] = argv

    monkeypatch.setattr(citydb_module.subprocess, "run", fake_run)
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    assert captured["argv"][0] == "podman"
    assert "docker" not in captured["argv"]


def test_ingest_passes_threads_flag_to_avoid_exhausting_max_connections(tmp_path, monkeypatch):
    # Verified fact: citydb-tool's default thread count is host `nproc`
    # (128 here), which blows past the tuned `max_connections = 20` and
    # fails the import outright ("too many clients already"). --threads=4
    # is required, not optional.
    captured = {}
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda argv, **kw: captured.update(argv=argv))
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    assert "--threads=4" in captured["argv"]


def test_ingest_mounts_the_dataset_parent_directory_and_references_it_by_name(tmp_path, monkeypatch):
    captured = {}
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda argv, **kw: captured.update(argv=argv))
    system, _ = _system_with_fake_conn(monkeypatch)
    dataset = _dataset(tmp_path)

    system.ingest(dataset)

    argv = captured["argv"]
    assert f"{dataset.source.parent.resolve()}:/work" in argv
    assert f"/work/{dataset.source.name}" in argv
    # No positional path baked in twice.
    assert argv.count(f"/work/{dataset.source.name}") == 1


def test_ingest_invokes_import_cityjson_subcommand(tmp_path, monkeypatch):
    captured = {}
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda argv, **kw: captured.update(argv=argv))
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    argv = captured["argv"]
    assert "import" in argv
    assert "cityjson" in argv
    assert argv.index("import") < argv.index("cityjson")


def test_ingest_passes_connection_flags(tmp_path, monkeypatch):
    captured = {}
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda argv, **kw: captured.update(argv=argv))
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    argv = captured["argv"]
    assert argv[argv.index("-H") + 1] == "localhost"
    assert argv[argv.index("-P") + 1] == "55433"
    assert argv[argv.index("-d") + 1] == "bench"
    assert argv[argv.index("-u") + 1] == "bench"
    assert argv[argv.index("-p") + 1] == "bench"
    assert argv[argv.index("-S") + 1] == "citydb"


def test_ingest_uses_check_true_so_a_failed_import_raises(tmp_path, monkeypatch):
    captured = {}
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda argv, **kw: captured.update(kwargs=kw))
    system, _ = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    assert captured["kwargs"]["check"] is True


def test_ingest_extracts_srid_from_database_srs_and_runs_index_ddl(tmp_path, monkeypatch):
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    system, fake_conn = _system_with_fake_conn(monkeypatch, srid_row=(7415,))

    system.ingest(_dataset(tmp_path))

    assert system._srid == 7415
    executed_sql = [sql for sql, _ in fake_conn.cur.executed]
    assert any("database_srs" in sql for sql in executed_sql)
    # index_ddl() is empty for 3DCityDB (see sql_citydb.py), so the loop
    # executes zero DDL statements — but every statement it WOULD return
    # must still be executed if that ever changes.
    for ddl in citydb_module.sql_citydb.index_ddl():
        assert ddl in executed_sql


def test_ingest_resolves_and_caches_the_cityobject_class_ids(tmp_path, monkeypatch):
    # C1 fix: ingest() must resolve the static id set ONCE (via
    # resolve_cityobject_class_ids, which fetchall()s against the fake's
    # canned _FAKE_CLASS_IDS rows) and cache it on the instance so run()
    # never needs to re-resolve it per query.
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    system, fake_conn = _system_with_fake_conn(monkeypatch)

    system.ingest(_dataset(tmp_path))

    assert system._cityobject_class_ids == (901, 902)
    executed_sql = [sql for sql, _ in fake_conn.cur.executed]
    assert any("objectclass" in sql and "is_toplevel" in sql for sql in executed_sql)


def test_ingest_defaults_srid_to_zero_when_database_srs_has_none(tmp_path, monkeypatch):
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch, srid_row=(None,))

    system.ingest(_dataset(tmp_path))

    assert system._srid == 0


def test_ingest_defaults_srid_to_zero_when_database_srs_is_empty(tmp_path, monkeypatch):
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch, srid_row=None)

    system.ingest(_dataset(tmp_path))

    assert system._srid == 0


def test_ingest_calls_vacuum_analyze_on_the_citydb_schema(tmp_path, monkeypatch):
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    calls = []
    system = CityDbSystem(port=55433, schema="citydb")
    fake_conn = _FakeConnection()
    monkeypatch.setattr(citydb_module.pg, "connect", lambda port: fake_conn)
    monkeypatch.setattr(
        citydb_module.pg, "vacuum_analyze",
        lambda conn, schema: calls.append((conn, schema)),
    )
    system.prepare()

    system.ingest(_dataset(tmp_path))

    assert calls == [(fake_conn, "citydb")]


def test_ingest_reports_the_import_wall_clock(tmp_path, monkeypatch):
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch)

    result = system.ingest(_dataset(tmp_path))

    assert result.wall_clock_s >= 0.0


def test_run_discards_the_warmup_and_reports_repeat_samples(monkeypatch):
    system, _ = _system_with_fake_conn(monkeypatch)
    # run() builds its SQL via sql_for(cityobject_class_ids=...); this test
    # exercises run()'s own warm-up/repeat plumbing (pg.time_query is
    # monkeypatched below), not ingest()'s resolution step, so the id set
    # is set directly rather than via a real ingest() call.
    system._cityobject_class_ids = (901, 902)
    responses = iter([
        (999, 9.0, 9.0),   # discarded warm-up: the most extreme values
        (5, 0.1, 0.05),
        (5, 0.2, 0.08),
        (5, 0.05, 0.02),
    ])
    monkeypatch.setattr(
        citydb_module.pg, "time_query",
        lambda conn, sql, args, count_mode: next(responses),
    )

    measurement = system.run("count", PARAMS, repeat=3)

    assert measurement.result_count == 5
    assert measurement.times_s == [0.1, 0.2, 0.05]
    assert measurement.server_times_s == [0.05, 0.08, 0.02]
    assert measurement.peak_rss_bytes is None
    assert measurement.peak_heap_bytes is None


def test_run_passes_the_stored_srid_into_sql_for(tmp_path, monkeypatch):
    monkeypatch.setattr(citydb_module.subprocess, "run", lambda *a, **kw: None)
    system, _ = _system_with_fake_conn(monkeypatch, srid_row=(28992,))
    system.ingest(_dataset(tmp_path))

    captured_args = {}

    def fake_time_query(conn, sql, args, count_mode):
        captured_args["args"] = args
        return (1, 0.01, 0.005)

    monkeypatch.setattr(citydb_module.pg, "time_query", fake_time_query)

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
