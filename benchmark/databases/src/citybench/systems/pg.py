"""Helpers shared by both PostgreSQL-backed systems.

Timing is captured two ways on purpose. The wall-clock covers the full
end-to-end cost a user pays, including result transfer over the socket.
EXPLAIN (ANALYZE) captures the server's own execution time, excluding
transfer. Publishing both means the client-server tax is visible and
attributable rather than silently folded into the headline number.

I3 (final whole-branch review) corrected a framing error in how the two
were compared: `server_time_s` is not a clean "engine-only" baseline to
subtract `time_s` against. `EXPLAIN (ANALYZE, BUFFERS)` itself adds real
instrumentation overhead (per-node timing/buffer-counting instrumentation,
plus `track_io_timing = on`'s own clock calls) on top of the query's own
execution — so `server_time_s` is an UPPER BOUND on the engine's true,
uninstrumented execution time, not a lower one. Seventeen committed rows
have `server_time_s > time_s` (the instrumented re-run outweighing the
plain, uninstrumented `time_s` measurement it is compared against) —
impossible if `server_time_s` were a clean subset of `time_s` the way a
naive "client-server tax = time_s - server_time_s" framing assumes. This
count is re-derived from the currently committed `results/*.csv` files,
not a number to copy forward by hand — it moved from 18 to 17 between
when this note was first written and a later Zurich re-run landing on
this branch, which is exactly the failure mode this note itself warns
about; re-count rather than trust either figure if this branch changes
again. See `time_query`'s own note below and README Caveat 4 for the
corrected framing: the two numbers are still both worth publishing, just
not subtracted from one another.
"""

from __future__ import annotations

import time

import psycopg

from citybench.config import SizeReport


def connect(port: int, *, dbname: str = "bench", user: str = "bench",
            password: str = "bench", host: str = "localhost") -> psycopg.Connection:
    return psycopg.connect(
        host=host, port=port, dbname=dbname, user=user, password=password,
        autocommit=True,
    )


def parse_explain_execution_time(plan: list) -> float:
    """Seconds, from an ``EXPLAIN (ANALYZE, FORMAT JSON)`` payload.

    PostgreSQL reports 'Execution Time' in milliseconds.
    """
    if not plan:
        raise ValueError("empty EXPLAIN payload")
    root = plan[0]
    if "Execution Time" not in root:
        raise ValueError("EXPLAIN payload has no 'Execution Time'; was ANALYZE used?")
    return float(root["Execution Time"]) / 1000.0


def time_query(conn: psycopg.Connection, sql: str, args: tuple = (),
               *, count_mode: str = "first-column") -> tuple[int, float, float]:
    """Run ``sql`` once, fully materialising results.

    Returns ``(result_count, wall_seconds, server_seconds)``. Rows are read
    to exhaustion so no system can win by returning a lazy cursor.

    ``server_seconds`` comes from a SECOND, untimed re-run under
    ``EXPLAIN (ANALYZE, BUFFERS)`` below, not from the first, plainly-timed
    run above — that instrumentation itself adds real overhead (per-node
    timing/buffer counters, ``track_io_timing``'s own clock calls), so
    ``server_seconds`` is an UPPER BOUND on the engine's true execution
    time, not a clean, lower "server-only" component of ``wall_seconds``.
    Do not subtract the two to compute a "client-server tax" — see
    README Caveat 4 and this module's own docstring (I3, final
    whole-branch review).
    """
    with conn.cursor() as cur:
        start = time.perf_counter()
        cur.execute(sql, args)
        rows = cur.fetchall() if cur.description is not None else []
        wall = time.perf_counter() - start

    count = extract_count(rows, count_mode)

    with conn.cursor() as cur:
        cur.execute(f"EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}", args)
        payload = cur.fetchone()[0]
    server = parse_explain_execution_time(payload)

    return count, wall, server


def extract_count(rows: list, mode: str) -> int:
    """A scenario's result count, per the registry's declared mode.

    'first-column' takes the first column of the single returned row —
    every such scenario's SQL is written to put the object count there.
    'rowcount' counts materialised rows.

    Inferring this from the result shape instead would silently compare
    two engines' checksums on full-read and flag every row as a mismatch.
    """
    if mode == "rowcount":
        return len(rows)
    if mode != "first-column":
        raise ValueError(f"unknown count mode: {mode!r}")
    if not rows:
        return 0
    return int(rows[0][0])


def dump_indexes(conn: psycopg.Connection, schema: str) -> list[str]:
    """Every index actually defined in ``schema``, as PostgreSQL itself
    states it (``pg_indexes.indexdef`` — a complete, runnable
    ``CREATE [UNIQUE] INDEX ... ON ...`` statement per row), not just the
    handful this harness's own ``index_ddl()`` functions may have added.

    I7 (final whole-branch review): ``results/<dataset>.indexes.sql`` was
    supposed to be "the exact DDL each system ran" (this project's own
    spec), but only ever recorded what THIS harness's ``index_ddl()``
    added on top of a system's own defaults — for 3DCityDB that is always
    an empty list (see ``sql_citydb.index_ddl()``'s docstring: every index
    it needs already exists), so the artefact carried effectively one
    real line and could not support an index-parity audit at all. This
    function is the fix's data source: called against BOTH `cjdb` and
    `3dcitydb`'s live connections at run time, it makes the artefact
    actually show the complete index set each system is running its
    scenario queries against — self-built defaults included, not just
    this harness's additions.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = %s "
            "ORDER BY tablename, indexname",
            (schema,),
        )
        return [row[0] for row in cur.fetchall()]


def schema_size(conn: psycopg.Connection, schema: str) -> SizeReport:
    """Total and index-free byte size of every table in ``schema``.

    Both figures are reported because the comparison against a file format
    flips depending on whether indexes are counted, and picking one would
    be choosing the flattering number.
    """
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT
              coalesce(sum(pg_total_relation_size(c.oid)), 0)::bigint,
              coalesce(sum(pg_table_size(c.oid)), 0)::bigint
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = %s AND c.relkind = 'r'
            """,
            (schema,),
        )
        total, table_only = cur.fetchone()
    return SizeReport(size_bytes=int(total), size_bytes_no_index=int(table_only))


def vacuum_analyze(conn: psycopg.Connection, schema: str) -> None:
    """Refresh planner statistics. Never skip this before measuring."""
    with conn.cursor() as cur:
        cur.execute(
            "SELECT tablename FROM pg_tables WHERE schemaname = %s", (schema,)
        )
        tables = [r[0] for r in cur.fetchall()]
        for table in tables:
            cur.execute(f'VACUUM ANALYZE {schema}."{table}"')
