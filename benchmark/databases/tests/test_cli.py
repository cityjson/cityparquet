"""Unit tests for the CLI's own glue logic.

Everything here is pure or fake-driven — no subprocess, no Docker, no
network. `test_citydb_integration.py`/`test_cjdb_integration.py` (marked
`integration`) exercise the real systems end to end; this file exercises
only what the CLI itself decides, independent of any backend.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from citybench.cli import (
    ROOT,
    _build_systems,
    _dataset,
    _format_ddl,
    _indexes_sql,
    _patches,
    _pg_settings,
    _run_all_scenarios,
    _srids,
    _versions,
)
from citybench.config import BBox, Measurement, Params
from citybench.scenarios.registry import SQL_SYSTEMS, TIER1, TIER2

PARAMS = Params(
    bbox_full=BBox(0.0, 0.0, 0.0, 100.0, 100.0, 10.0),
    attr_column="object_type",
    attr_eq="Building",
    numeric_column="h",
    target_id="a",
    parent_id="b",
    total_city_objects=100,
)


# --- _dataset -----------------------------------------------------------


def test_dataset_derives_name_and_both_package_dirs_under_root_data():
    d = _dataset(Path("/somewhere/delft.city.jsonl"))
    assert d.name == "delft"
    assert d.source == Path("/somewhere/delft.city.jsonl")
    assert d.cityparquet_dir == ROOT / "data" / "cityparquet" / "delft"
    assert d.hilbert_dir == ROOT / "data" / "cityparquet-hilbert" / "delft"


def test_dataset_strips_city_jsonl_suffix_not_just_the_extension():
    d = _dataset(Path("/x/Montreal.city.jsonl"))
    assert d.name == "Montreal"
    assert d.cityparquet_dir == ROOT / "data" / "cityparquet" / "Montreal"


# --- _build_systems -------------------------------------------------------


def test_build_systems_returns_one_instance_per_tag_in_order():
    systems = _build_systems(["cjdb", "3dcitydb"])
    assert [s.tag for s in systems] == ["cjdb", "3dcitydb"]


def test_build_systems_constructs_both_readbench_variants_with_distinct_tags():
    # ReadbenchSystem is deliberately not @register-decorated (one class,
    # two tags) — this is the one call site responsible for giving each
    # tag its own instance with the right `hilbert` flag.
    systems = _build_systems(["cityparquet", "cityparquet-hilbert"])
    by_tag = {s.tag: s for s in systems}
    assert set(by_tag) == {"cityparquet", "cityparquet-hilbert"}
    assert by_tag["cityparquet"]._hilbert is False
    assert by_tag["cityparquet-hilbert"]._hilbert is True


def test_build_systems_all_five_default_tags_are_known():
    tags = ["cityparquet", "cityparquet-hilbert", "duckdb-cityparquet", "cjdb", "3dcitydb"]
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
# The integration bug this task exists to catch: a single run_matrix call
# across every scenario and every system would hand the Rust child a TIER2
# scenario name (`hierarchy`, `lod-extract`, `semantic-surface`) it does
# not implement. `TierAwareFakeSystem` reproduces exactly the failure mode
# `ReadbenchSystem.run` has for real (see `build_child_args`'s
# `test_tier2_scenarios_are_rejected`) so this test would catch a
# regression back to the single-call shape.


class TierAwareFakeSystem:
    def __init__(self, tag: str, count: int = 1):
        self.tag = tag
        self._count = count

    def run(self, scenario, params, repeat, selectivity=None):
        if scenario in TIER2 and self.tag not in SQL_SYSTEMS:
            raise ValueError(f"{self.tag} cannot run tier2 scenario {scenario!r}")
        return Measurement(
            result_count=self._count,
            times_s=[0.01] * repeat,
            server_times_s=[],
            peak_rss_bytes=None,
        )


def test_run_all_scenarios_never_asks_a_non_sql_system_for_a_tier2_scenario():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    # Must not raise: if this called run_matrix once across ALL scenarios
    # and all systems, TierAwareFakeSystem("cityparquet") would raise on
    # the first TIER2 scenario it was asked to run.
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={})
    assert not any("error:" in r["notes"] for r in rows)


def test_run_all_scenarios_runs_tier2_only_against_sql_systems():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={})
    tier2_formats = {r["format"] for r in rows if r["scenario"] in TIER2}
    assert tier2_formats == {"cjdb"}


def test_run_all_scenarios_runs_tier1_against_every_system():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={})
    tier1_formats = {r["format"] for r in rows if r["scenario"] in TIER1}
    assert tier1_formats == {"cityparquet", "cjdb"}


def test_run_all_scenarios_row_count_matches_tier1_all_plus_tier2_sql_only():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={})
    # TIER1 has 7 scenarios, one of which (bbox-query) expands into 3 rows
    # per system; TIER2 has 3 scenarios, SQL-systems-only.
    expected_tier1_scenario_rows = (len(TIER1) - 1 + 3) * len(systems)
    expected_tier2_scenario_rows = len(TIER2) * 1  # only "cjdb" is a SQL system here
    assert len(rows) == expected_tier1_scenario_rows + expected_tier2_scenario_rows


def test_run_all_scenarios_with_no_sql_systems_produces_no_tier2_rows_or_errors():
    systems = [TierAwareFakeSystem("cityparquet"), TierAwareFakeSystem("cityparquet-hilbert")]
    rows = _run_all_scenarios(systems, PARAMS, "delft", repeat=1, sizes={})
    assert not any(r["scenario"] in TIER2 for r in rows)
    assert not any("error:" in r["notes"] for r in rows)


def test_run_all_scenarios_stamps_sizes_through_to_both_tiers():
    systems = [TierAwareFakeSystem("cjdb")]
    rows = _run_all_scenarios(
        systems, PARAMS, "delft", repeat=1, sizes={"cjdb": (900, 700)},
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


def test_versions_omits_cjdb_when_cjdb_is_not_in_the_run():
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
    # cityparquet/cityparquet-hilbert/duckdb-cityparquet carry no `_srid`
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


def test_pg_settings_queries_max_parallel_workers_per_gather(monkeypatch):
    # I4 (final whole-branch review): the per-query-binding setting, not
    # just the cluster-wide pool it draws from, must be captured so a
    # published manifest can be cited against it.
    from citybench.systems import pg as pg_module

    fake_conn = _FakeSettingsConnection([])
    monkeypatch.setattr(pg_module, "connect", lambda port: fake_conn)

    _pg_settings()

    # _pg_settings() queries once per port (cjdb, 3dcitydb); both calls
    # share the fake connection here, so both entries are checked.
    assert fake_conn._cur.executed_args, "execute() was never called"
    for (queried_names,) in fake_conn._cur.executed_args:
        assert "max_parallel_workers_per_gather" in queried_names


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
