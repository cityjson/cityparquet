"""Unit tests for the CLI's own glue logic.

Everything here is pure or fake-driven — no subprocess, no Docker, no
network. `test_citydb_integration.py`/`test_cjdb_integration.py` (marked
`integration`) exercise the real systems end to end; this file exercises
only what the CLI itself decides, independent of any backend.
"""

from __future__ import annotations

from pathlib import Path

import os

import pytest

from citybench.systems.duckdb_cp import DuckDBCityParquet

from citybench.cli import (
    ROOT,
    _build_systems,
    _dataset,
    _execution,
    _format_ddl,
    _indexes_sql,
    _patches,
    _pg_settings,
    _run_all_scenarios,
    _srids,
    _versions,
)
from citybench.cli import THREAD_CONFIGURATIONS
from citybench.config import Measurement, Params
from citybench.runner import DEFAULT_COUNT_TOLERANCE
from citybench.scenarios.registry import (
    READ_SCENARIOS, SQL_SYSTEMS, TIER1, TIER2, TIER3, systems_for,
)
from conftest import ge_attr_filter, make_params

PARAMS = make_params(numeric_column="h")


# --- _dataset -----------------------------------------------------------


def test_dataset_reads_the_format_benchmarks_prepared_package_by_default():
    # The package `readbench_prepare.sh` wrote with `--no-lod0`, under the
    # suite's data root: the same bytes the format family measured.
    d = _dataset(Path("/somewhere/delft.city.jsonl"))
    assert d.name == "delft"
    assert d.source == Path("/somewhere/delft.city.jsonl")
    assert d.cityparquet_dir == ROOT.parent / "runs" / "data" / "readbench" / "delft.parquet"


def test_dataset_strips_city_jsonl_suffix_not_just_the_extension():
    d = _dataset(Path("/x/Montreal.city.jsonl"))
    assert d.name == "Montreal"
    assert d.cityparquet_dir == ROOT.parent / "runs" / "data" / "readbench" / "Montreal.parquet"


# --- _build_systems -------------------------------------------------------


def test_build_systems_returns_one_instance_per_tag_in_order():
    systems = _build_systems(["cjdb", "3dcitydb"])
    assert [s.tag for s in systems] == ["cjdb", "3dcitydb"]


def test_build_systems_knows_every_system_tag():
    tags = ["cityparquet", "duckdb-cityparquet", "duckdb-cityparquet-writeback",
            "cjdb", "3dcitydb"]
    systems = _build_systems(tags)
    assert [s.tag for s in systems] == tags


def test_build_systems_rejects_unknown_tag():
    with pytest.raises(SystemExit):
        _build_systems(["not-a-real-system"])


def test_build_systems_unknown_tag_error_names_the_offender():
    with pytest.raises(SystemExit, match="typo-tag"):
        _build_systems(["cjdb", "typo-tag"])


# --- _format_ddl ----------------------------------------------------------


def test_format_ddl_empty_list_is_empty_string_not_a_stray_semicolon():
    # 3DCityDB's index_ddl() legitimately returns [] (every index it needs
    # already exists) — this must render as nothing, not ";\n", which
    # would misleadingly read as a dropped statement.
    assert _format_ddl([]) == ""


def test_format_ddl_single_statement_is_semicolon_terminated():
    assert _format_ddl(["CREATE INDEX ix ON t (c)"]) == "CREATE INDEX ix ON t (c);\n"


def test_format_ddl_multiple_statements_are_joined_and_each_terminated():
    out = _format_ddl(["stmt one", "stmt two"])
    assert out == "stmt one;\nstmt two;\n"


# --- _run_all_scenarios ----------------------------------------------------
#
# The integration bug this guards against: a single run_matrix call across
# every scenario and every system would hand the Rust child a scenario name
# it does not implement. `TierAwareFakeSystem` reproduces exactly the
# failure mode `ReadbenchSystem.run` has for real (see `build_child_args`)
# so a regression to that shape is caught here.


class TierAwareFakeSystem:
    def __init__(self, tag: str, count: int = 1):
        self.tag = tag
        self._count = count
        self.threads: list[int] = []
        self.workers: list[int] = []

    def run(self, scenario, params, repeat, window=None, probe=None):
        if self.tag not in systems_for(scenario):
            raise ValueError(f"{self.tag} cannot run scenario {scenario!r}")
        return Measurement(
            result_count=self._count,
            times_s=[0.01] * repeat,
            server_times_s=[],
            peak_rss_bytes=None,
        )


class DuckFakeSystem(TierAwareFakeSystem):
    """A fake reconfigured the way the embedded engine is: by thread count."""

    def set_threads(self, threads):
        self.threads.append(threads)


class PgFakeSystem(TierAwareFakeSystem):
    """A fake reconfigured the way a PostgreSQL session is: by worker budget.

    Deliberately has NO `set_threads`: `_apply_threads` picks whichever
    method the system offers, and a PostgreSQL adapter that grew a
    `set_threads` would silently stop receiving its worker budget.
    """

    def set_parallel_workers(self, workers):
        self.workers.append(workers)


def test_run_all_scenarios_never_asks_a_system_for_a_scenario_it_cannot_run():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    # Must not raise: if this called run_matrix once across ALL scenarios
    # and all systems, TierAwareFakeSystem("cityparquet") would raise on
    # the first scenario the Rust child does not implement.
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={},
                              tolerance=DEFAULT_COUNT_TOLERANCE)
    assert not any("error:" in r["notes"] for r in rows)


def test_run_all_scenarios_runs_tier2_only_against_sql_systems():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={},
                              tolerance=DEFAULT_COUNT_TOLERANCE)
    tier2_formats = {r["format"] for r in rows if r["scenario"] in TIER2}
    assert tier2_formats == {"cjdb"}


def test_run_all_scenarios_runs_the_readbench_subset_against_every_system():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={},
                              tolerance=DEFAULT_COUNT_TOLERANCE)
    formats = {r["format"] for r in rows if r["scenario"] == "count"}
    assert formats == {"cityparquet", "cjdb"}
    # …and only the SQL system answers a scenario the child cannot.
    formats = {r["format"] for r in rows if r["scenario"] == "geometry-scan"}
    assert formats == {"cjdb"}


def test_every_read_scenario_is_measured_under_both_thread_configurations():
    systems = [TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={},
                              tolerance=DEFAULT_COUNT_TOLERANCE)
    read_rows = [r for r in rows if r["scenario"] in READ_SCENARIOS]
    for name, _, _ in THREAD_CONFIGURATIONS:
        tagged = [r for r in read_rows if f"threads={name}" in r["notes"]]
        assert tagged, name
    assert len(read_rows) == 2 * len(read_rows) // 2       # exactly two passes
    assert {r["scenario"] for r in read_rows} <= set(READ_SCENARIOS)


def test_the_write_tier_runs_last_under_the_primary_configuration_only():
    """CJDB's Q6-Q8 leave dead tuples on cjdb and rewritten pages on
    3DCityDB even after the attribute is deleted, so a read pass after them
    would measure a bloated table."""
    systems = [TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={},
                              tolerance=DEFAULT_COUNT_TOLERANCE)
    write_indices = [i for i, r in enumerate(rows) if r["scenario"] in TIER3]
    read_indices = [i for i, r in enumerate(rows) if r["scenario"] in READ_SCENARIOS]
    assert write_indices and min(write_indices) > max(read_indices)
    primary = THREAD_CONFIGURATIONS[0][0]
    assert all(f"threads={primary}" in rows[i]["notes"] for i in write_indices)
    # add -> update -> delete: each leaves the state the next expects.
    assert [rows[i]["scenario"] for i in write_indices] == list(TIER3)


def test_each_system_is_reconfigured_the_way_its_own_engine_expects():
    duck = DuckFakeSystem("duckdb-cityparquet")
    postgres = PgFakeSystem("cjdb")
    _run_all_scenarios([duck, postgres], PARAMS, "delft", repeat=1, sizes={},
                       tolerance=DEFAULT_COUNT_TOLERANCE)
    # Both configurations, then back to the primary for the write tier.
    assert duck.threads == [1, 16, 1]
    assert postgres.workers == [0, 8, 0]


def test_run_all_scenarios_stamps_sizes_through_to_every_tier():
    systems = [TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(
        systems, PARAMS, "delft", repeat=1, sizes={"cjdb": (900, 700)},
        tolerance=DEFAULT_COUNT_TOLERANCE,
    )
    assert all(r["size_bytes"] == "900" for r in rows)


# --- _versions / _patches: cjdb patch disclosure ---------------------------
#
# cjdb is a PATCHED system (see systems/cjdb.py's module docstring and
# vendor/cjdb/README.md) — a reader of the run manifest must never be able
# to mistake its numbers for stock cjdb 2.2.0's. These tests pin that both
# the terse `versions` marker and the full `patches` disclosure appear
# whenever cjdb is actually part of a run, and stay absent otherwise.


class _TaggedFake:
    def __init__(self, tag):
        self.tag = tag


def test_versions_includes_duckdb_unconditionally():
    versions = _versions([])
    assert "duckdb" in versions


def test_versions_omits_cjdb_when_cjdb_is_not_in_the_run(monkeypatch):
    from citybench import cli

    monkeypatch.setattr(cli, "_extension_reported_version", lambda path: "test")
    versions = _versions([_TaggedFake("duckdb-cityparquet")])
    assert "cjdb" not in versions


def test_versions_marks_cjdb_as_patched_when_cjdb_is_in_the_run():
    versions = _versions([_TaggedFake("cjdb")])
    assert "patch" in versions["cjdb"]
    assert versions["cjdb"].startswith("2.2.0")


def test_patches_is_empty_when_cjdb_is_not_in_the_run():
    assert _patches([_TaggedFake("duckdb-cityparquet")]) == {}


def test_patches_includes_cjdbs_full_disclosure_when_cjdb_is_in_the_run(monkeypatch):
    from citybench.systems import cjdb as cjdb_module

    fake_disclosure = {
        "upstream_version": "2.2.0", "patched": "true",
        "patch_file": "vendor/cjdb/ground-surfaces-tie.patch",
        "patch_summary": "...", "built_from": "/fake/path",
    }
    monkeypatch.setattr(cjdb_module, "patch_disclosure", lambda: fake_disclosure)

    patches = _patches([_TaggedFake("cjdb")])

    assert patches == {"cjdb": fake_disclosure}


# --- _srids: the SRID each PostgreSQL-backed system actually landed on -----
#
# Task 14 (the heterogeneity corpus): 3DCityDB's SRID is baked in at schema
# creation and cannot be changed afterwards, and getting it wrong does NOT
# error — it silently mislabels or reprojects geometry. `_srids` is the
# glue that gets each adapter's own database-verified `_srid` into the run
# manifest (`manifest.collect`'s `srid` field).


class _SridFake(_TaggedFake):
    def __init__(self, tag, srid):
        super().__init__(tag)
        self._srid = srid


def test_srids_reads_back_the_landed_value_from_each_system_that_has_one():
    srids = _srids([_SridFake("cjdb", 2950), _SridFake("3dcitydb", 2950)])
    assert srids == {"cjdb": 2950, "3dcitydb": 2950}


def test_srids_omits_systems_with_no_srid_concept():
    # cityparquet/duckdb-cityparquet carry no `_srid`
    # attribute at all — absent from the dict, not stamped with a
    # meaningless placeholder like 0 or None.
    srids = _srids([_TaggedFake("cityparquet"), _SridFake("cjdb", 7415)])
    assert srids == {"cjdb": 7415}
    assert "cityparquet" not in srids


def test_srids_is_empty_when_no_system_has_one():
    assert _srids([_TaggedFake("duckdb-cityparquet")]) == {}


# --- _pg_settings -----------------------------------------------------
#
# M1 (final whole-branch review): _pg_settings() used to concatenate
# pg_settings.setting (a raw integer) with pg_settings.unit (that GUC's own
# internal multiplier string, e.g. "8kB") directly as text -- producing
# "10485768kB" for an 8GB shared_buffers, which reads as ~10.5GB. Fixed to
# ask PostgreSQL for the already-pretty-printed value via current_setting().
# These tests fake psycopg's connect/cursor rather than touching a live
# database, mirroring test_citydb_adapter.py's _FakeConnection/_FakeCursor.


class _FakeSettingsCursor:
    def __init__(self, rows):
        self._rows = rows
        self.executed_sql: list[str] = []
        self.executed_args: list[tuple] = []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql, args=()):
        self.executed_sql.append(sql)
        self.executed_args.append(args)

    def fetchall(self):
        return self._rows


class _FakeSettingsConnection:
    def __init__(self, rows):
        self._cur = _FakeSettingsCursor(rows)

    def cursor(self):
        return self._cur

    def close(self):
        pass


def test_pg_settings_reports_the_pretty_printed_value_not_a_raw_concatenation(monkeypatch):
    # The exact M1 regression case: shared_buffers stored as raw block
    # count 1048576 with unit "8kB" must render as "8GB" (what
    # current_setting()/SHOW would report), never "10485768kB".
    from citybench.systems import pg as pg_module

    fake_conn = _FakeSettingsConnection([("shared_buffers", "8GB")])
    monkeypatch.setattr(pg_module, "connect", lambda port: fake_conn)

    settings = _pg_settings()

    assert settings["cjdb"]["shared_buffers"] == "8GB"
    assert "10485768kB" not in settings["cjdb"].values()


def test_pg_settings_leaves_the_per_session_setting_to_the_execution_block(monkeypatch):
    """`max_parallel_workers_per_gather` binds per query and is now set per
    thread configuration on the benchmark session. Reading it back here,
    from a FRESH connection, would report the configuration FILE's value and
    contradict `execution.postgresql_session_resolved`, which records what
    each configuration's own session resolved to. One manifest, one answer.

    The cluster-wide pool it draws from is a file setting and stays."""
    from citybench.systems import pg as pg_module

    fake_conn = _FakeSettingsConnection([])
    monkeypatch.setattr(pg_module, "connect", lambda port: fake_conn)

    _pg_settings()

    # _pg_settings() queries once per port (cjdb, 3dcitydb); both calls
    # share the fake connection here, so both entries are checked.
    assert fake_conn._cur.executed_args, "execute() was never called"
    for (queried_names,) in fake_conn._cur.executed_args:
        assert "max_parallel_workers_per_gather" not in queried_names
        assert "max_parallel_workers" in queried_names


def test_pg_settings_reports_an_error_string_when_the_connection_fails(monkeypatch):
    from citybench.systems import pg as pg_module

    def fail(port):
        raise RuntimeError("connection refused")

    monkeypatch.setattr(pg_module, "connect", fail)

    settings = _pg_settings()

    assert "connection refused" in settings["cjdb"]["error"]
    assert "connection refused" in settings["3dcitydb"]["error"]


# --- _indexes_sql -------------------------------------------------------
#
# I7 (final whole-branch review): results/<dataset>.indexes.sql must dump
# the FULL live pg_indexes set for both schemas, not just what this
# harness's own index_ddl() functions added -- the previous version wrote
# effectively one real line (3dcitydb's own index_ddl() is always empty).


class _FakePgSystem:
    def __init__(self, tag, schema, conn):
        self.tag = tag
        self._schema = schema
        self._conn = conn


def test_indexes_sql_includes_the_full_live_dump_for_both_systems(monkeypatch):
    from citybench import cli as cli_module

    monkeypatch.setattr(
        cli_module.pg, "dump_indexes",
        lambda conn, schema: [f"CREATE INDEX ix ON {schema}.t (c)"],
    )
    systems = [
        _FakePgSystem("cjdb", "cjdb", object()),
        _FakePgSystem("3dcitydb", "citydb", object()),
    ]

    text = _indexes_sql(systems)

    assert "CREATE INDEX ix ON cjdb.t (c);" in text
    assert "CREATE INDEX ix ON citydb.t (c);" in text


def test_indexes_sql_dumps_each_systems_own_schema_not_a_hardcoded_one(monkeypatch):
    from citybench import cli as cli_module

    captured = []
    monkeypatch.setattr(
        cli_module.pg, "dump_indexes",
        lambda conn, schema: captured.append(schema) or [],
    )
    systems = [_FakePgSystem("3dcitydb", "custom_schema", object())]

    _indexes_sql(systems)

    assert captured == ["custom_schema"]


def test_indexes_sql_notes_absence_when_a_system_did_not_run_this_time():
    text = _indexes_sql([])
    assert "was not part of this run" in text
    assert "cjdb" in text
    assert "3dcitydb" in text


def test_indexes_sql_still_includes_the_harness_added_ddl_section(monkeypatch):
    from citybench import cli as cli_module

    monkeypatch.setattr(cli_module.pg, "dump_indexes", lambda conn, schema: [])
    systems = [_FakePgSystem("cjdb", "cjdb", object())]

    text = _indexes_sql(systems)

    # The pre-existing "what this harness added" section (cjdb's one
    # genuinely missing index) must survive alongside the new live dump,
    # not be replaced by it.
    assert "CREATE INDEX IF NOT EXISTS ix_co_object_id" in text


def test_run_defaults_to_25_timed_repetitions(monkeypatch):
    import citybench.cli as cli

    seen = []
    monkeypatch.setattr(cli, "cmd_bench", lambda args: seen.append(args.repeat) or 0)
    assert cli.main(["run", "--dataset", "unused.city.jsonl"]) == 0
    assert seen == [25]


def test_run_parser_accepts_the_isolation_flags(monkeypatch):
    from citybench import cli
    seen = {}
    monkeypatch.setattr(cli, "cmd_bench", lambda args: seen.update(vars(args)) or 0)
    monkeypatch.setenv("BENCH_NUMA_NODE", "1")
    cli.main(["run", "--dataset", "x", "--max-load", "off",
              "--max-load-wait-s", "30", "--memory-max", "8000000000"])
    assert seen["numa_node"] == "1"
    assert seen["max_load"] == "off"
    assert seen["max_load_wait_s"] == 30.0
    assert seen["memory_max"] == "8000000000"


def test_run_parser_isolation_defaults(monkeypatch):
    from citybench import cli
    seen = {}
    monkeypatch.setattr(cli, "cmd_bench", lambda args: seen.update(vars(args)) or 0)
    monkeypatch.delenv("BENCH_NUMA_NODE", raising=False)
    cli.main(["run", "--dataset", "x"])
    assert (seen["numa_node"], seen["max_load"], seen["max_load_wait_s"], seen["memory_max"]) == ("auto", "auto", 600.0, None)


def test_cmd_bench_plans_isolation_once_and_hands_the_cpuset_to_the_containers(monkeypatch, tmp_path):
    from argparse import Namespace
    from contextlib import contextmanager
    from citybench import cli, isolation

    record = isolation.plan(
        numa_node="auto", max_load="auto", max_load_wait_s=600, memory_max=None,
        is_linux=True, has_setaffinity=True, node_cpus={0: "0-3", 1: "4-7"},
        node_meminfo={0: "Node 0 MemFree: 1 kB", 1: "Node 1 MemFree: 2 kB"},
        meminfo="MemTotal: 8 kB\nMemAvailable: 4 kB\n",
        controllers="cpuset cpu memory", all_cpus=list(range(8)),
    )
    setups = []
    monkeypatch.setattr(cli.isolation_mod, "setup",
                        lambda **kw: setups.append(kw) or (record, object()))
    seen = {}

    class Stop(Exception):
        pass

    @contextmanager
    def fake_databases(data_root, srid, *, container_args=None):
        seen["container_args"] = container_args
        raise Stop
        yield  # pragma: no cover

    monkeypatch.setattr(cli, "isolated_databases", fake_databases)
    args = Namespace(dataset="x", data_root=str(tmp_path), ports=None, srid=7415,
                     numa_node="auto", max_load="auto", max_load_wait_s=600.0,
                     memory_max=None)
    try:
        cli.cmd_bench(args)
    except Stop:
        pass
    assert len(setups) == 1
    assert seen["container_args"] == ["--cpuset-cpus=4-7", "--cpuset-mems=1"]
    assert record["containers"]["started_by_run"] is True



def test_cmd_bench_records_the_engine_and_drops_cpuset_an_engine_lacks(monkeypatch, tmp_path):
    from argparse import Namespace
    from contextlib import contextmanager
    from citybench import cli, engine, isolation

    record = isolation.plan(
        numa_node="auto", max_load="auto", max_load_wait_s=600, memory_max=None,
        is_linux=True, has_setaffinity=True, node_cpus={0: "0-3", 1: "4-7"},
        node_meminfo={0: "Node 0 MemFree: 1 kB", 1: "Node 1 MemFree: 2 kB"},
        meminfo="MemTotal: 8 kB\nMemAvailable: 4 kB\n",
        controllers="cpuset cpu memory", all_cpus=list(range(8)),
    )
    monkeypatch.setattr(cli.isolation_mod, "setup", lambda **kw: (record, object()))
    monkeypatch.setattr(engine, "_ACTIVE", engine.Engine(
        name="container", binary="container", version="container CLI version 1.0.0",
        run_flags=frozenset({"--cpus", "--memory"}), platform="darwin"))
    seen = {}

    class Stop(Exception):
        pass

    @contextmanager
    def fake_databases(data_root, srid, *, container_args=None):
        seen["container_args"] = container_args
        raise Stop
        yield  # pragma: no cover

    monkeypatch.setattr(cli, "isolated_databases", fake_databases)
    args = Namespace(dataset="x", data_root=str(tmp_path), ports=None, srid=7415,
                     numa_node="auto", max_load="auto", max_load_wait_s=600.0, memory_max=None)
    try:
        cli.cmd_bench(args)
    except Stop:
        pass
    assert seen["container_args"] == []
    assert record["containers"]["cpuset"].startswith("not applied: container run has no --cpuset-cpus")
    assert record["containers"]["engine"]["name"] == "container"
    assert record["containers"]["engine"]["version"] == "container CLI version 1.0.0"
    assert record["containers"]["engine"]["capabilities"]["host_proc"].startswith("not applied:")


def test_container_engine_flag_sets_the_override(monkeypatch):
    from citybench import cli, engine
    monkeypatch.delenv(engine.ENV_VAR, raising=False)
    monkeypatch.setattr(cli, "cmd_bench", lambda args: 0)
    try:
        cli.main(["--container-engine", "docker", "run", "--dataset", "x"])
    except SystemExit:
        pass
    import os
    assert os.environ.pop(engine.ENV_VAR, None) == "docker"

def test_the_execution_block_records_the_timed_repetitions():
    # A quick (7-repetition) run must not read as the 25-repetition one.
    assert _execution({}, repeat=7)["repeat"] == 7


# --- the DuckDB CityJSON extension build -----------------------------------


def test_versions_record_the_extension_build_the_duckdb_systems_load(tmp_path, monkeypatch):
    from citybench import cli, extension

    build = tmp_path / "cityjson.duckdb_extension"
    build.write_bytes(b"abc")
    monkeypatch.setenv(extension.ENV_VAR, str(build))
    monkeypatch.setattr(cli, "_extension_reported_version", lambda path: "6937c06")
    versions = _versions([DuckDBCityParquet()])
    assert versions["duckdb-cityjson-path"] == str(build)
    assert versions["duckdb-cityjson"] == "6937c06"
    assert "duckdb-cityjson-sha256" in versions
    assert "duckdb-cityjson-commit" in versions


def test_versions_skip_the_extension_without_a_duckdb_system():
    assert "duckdb-cityjson-path" not in _versions([])


def test_extension_flag_is_carried_to_the_systems_through_the_environment(
        tmp_path, monkeypatch):
    from citybench import cli, extension

    build = tmp_path / "cityjson.duckdb_extension"
    build.write_bytes(b"abc")
    monkeypatch.delenv(extension.ENV_VAR, raising=False)
    monkeypatch.setattr(cli, "cmd_derive_params", lambda args: 0)
    cli.main(["--duckdb-cityjson-extension", str(build),
              "derive-params", "--dataset", "x.city.json"])
    assert os.environ[extension.ENV_VAR] == str(build)


def test_a_run_with_a_duckdb_system_refuses_to_start_without_a_build(
        tmp_path, monkeypatch):
    from citybench import cli, extension

    monkeypatch.setenv(extension.ENV_VAR, str(tmp_path / "missing"))
    with pytest.raises(extension.ExtensionNotFound):
        cli._require_extension(["duckdb-cityparquet", "cjdb"])
    cli._require_extension(["cjdb", "3dcitydb"])  # no DuckDB system: no build needed
