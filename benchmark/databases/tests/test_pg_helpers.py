import pytest

from citybench.systems import pg
from citybench.systems.pg import dump_indexes, extract_count, parse_explain_execution_time


def test_parses_execution_time_from_explain_json():
    plan = [{"Plan": {"Node Type": "Seq Scan"}, "Execution Time": 12.345, "Planning Time": 0.5}]
    assert parse_explain_execution_time(plan) == pytest.approx(0.012345)


def test_missing_execution_time_raises():
    with pytest.raises(ValueError):
        parse_explain_execution_time([{"Plan": {}}])


def test_empty_plan_raises():
    with pytest.raises(ValueError):
        parse_explain_execution_time([])


def test_first_column_mode_returns_first_column_of_multi_column_row():
    # (count, checksum) — the real shape for full-read and attr-stats: a
    # single-column row would pass even under the buggy shape-inference
    # this function exists to replace.
    rows = [(42, "deadbeef")]
    assert extract_count(rows, "first-column") == 42


def test_first_column_mode_on_empty_result_is_zero():
    assert extract_count([], "first-column") == 0


def test_rowcount_mode_returns_number_of_rows():
    rows = [(1, "a"), (2, "b"), (3, "c")]
    assert extract_count(rows, "rowcount") == 3


def test_rowcount_mode_on_empty_result_is_zero():
    assert extract_count([], "rowcount") == 0


def test_invalid_mode_raises():
    with pytest.raises(ValueError):
        extract_count([(1,)], "checksum")


# --- dump_indexes -----------------------------------------------------
#
# I7 (final whole-branch review): results/<dataset>.indexes.sql must
# record the FULL, live pg_indexes definition set for a schema, not just
# what this harness's own index_ddl() added on top of it. Fake connection,
# not a live database — mirrors test_citydb_adapter.py's _FakeCursor
# style.


class _FakeIndexCursor:
    def __init__(self, rows):
        self._rows = rows
        self.executed: list[tuple[str, tuple]] = []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql, args=()):
        self.executed.append((sql, args))

    def fetchall(self):
        return self._rows


class _FakeIndexConnection:
    def __init__(self, rows):
        self.cur = _FakeIndexCursor(rows)

    def cursor(self):
        return self.cur


def test_dump_indexes_returns_every_indexdef_for_the_schema():
    rows = [
        ("CREATE UNIQUE INDEX feature_pk ON citydb.feature USING btree (id)",),
        ("CREATE INDEX feature_objectclass_inx ON citydb.feature USING btree (objectclass_id)",),
    ]
    conn = _FakeIndexConnection(rows)

    result = dump_indexes(conn, "citydb")

    assert result == [
        "CREATE UNIQUE INDEX feature_pk ON citydb.feature USING btree (id)",
        "CREATE INDEX feature_objectclass_inx ON citydb.feature USING btree (objectclass_id)",
    ]


def test_dump_indexes_queries_pg_indexes_scoped_to_the_given_schema():
    conn = _FakeIndexConnection([])

    dump_indexes(conn, "cjdb")

    sql, args = conn.cur.executed[0]
    assert "pg_indexes" in sql
    assert args == ("cjdb",)


def test_dump_indexes_returns_an_empty_list_for_a_schema_with_no_indexes():
    conn = _FakeIndexConnection([])
    assert dump_indexes(conn, "empty_schema") == []


def test_the_benchmark_connection_hands_json_back_as_text():
    """Keeps the client's per-value object construction off the PostgreSQL
    side of the comparison, as `_arrow` already keeps it off DuckDB's.
    Without it, cjdb's `lod-query` would parse half a million
    inline-vertex geometry documents into Python dicts inside the timed
    window — a client measurement, and an out-of-memory risk at 1M."""
    import psycopg
    import psycopg.types.string

    class FakeAdapters:
        def __init__(self):
            self.registered = {}

        def register_loader(self, name, loader):
            self.registered[name] = loader

    class FakeConn:
        def __init__(self):
            self.adapters = FakeAdapters()

    conn = FakeConn()
    pg.register_text_passthrough(conn)
    assert set(conn.adapters.registered) == {"json", "jsonb"}
    assert all(loader is psycopg.types.string.TextLoader
               for loader in conn.adapters.registered.values())


def test_parses_execution_time_from_a_text_explain_payload():
    """The benchmark connections load json/jsonb as TEXT (register_text_passthrough),
    so the EXPLAIN payload is a string there; every PostgreSQL read row errored
    with ValueError in the 2026-09-22 smoke before this was handled."""
    from citybench.systems.pg import parse_explain_execution_time
    payload = '[{"Plan": {"Node Type": "Seq Scan"}, "Execution Time": 12.5}]'
    assert parse_explain_execution_time(payload) == 0.0125
