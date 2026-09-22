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
import os

import psycopg
import psycopg.types.string

from citybench.config import SizeReport
from citybench.stats import container_init_host_pid, host_pid_for_namespace_pid, host_pid_from_podman, peak_resident_bytes


def connect(port: int, *, dbname: str = "bench", user: str = "bench",
            password: str = "bench", host: str = "localhost") -> psycopg.Connection:
    conn = psycopg.connect(
        host=host, port=port, dbname=dbname, user=user, password=password,
        autocommit=True,
    )
    register_text_passthrough(conn)
    return conn


def register_text_passthrough(conn: psycopg.Connection) -> None:
    """Hand JSON back as TEXT rather than parsed into Python objects.

    Not a tuning knob: a consistency fix, and it works AGAINST the system
    it is applied to being flattered. The DuckDB adapter materialises a
    row-returning scenario to Arrow rather than to Python objects,
    precisely so the number is the engine's and not the client's per-value
    object construction (measured at about 72 us/row on `SELECT *` —
    `duckdb_cp._arrow`). Without this, the PostgreSQL side of the same
    comparison would pay exactly that cost and more: psycopg parses every
    `jsonb` value into Python dicts and lists, and on cjdb the geometry IS
    a JSONB document with its vertices resolved inline. `lod-query` on the
    1M slice returns half a million such rows; parsed, they are tens of
    gigabytes of Python objects in the harness process, which is a client
    measurement and an out-of-memory risk rather than a database one.

    The rows are still transferred in full and still read to exhaustion
    inside the timed window — the server does all of its own work, and the
    bytes all cross the socket. Only the client-side object construction is
    skipped, on the side that would otherwise be the only one paying it.
    Disclosed as `fetch: text` in every PostgreSQL row's `notes`, beside
    the DuckDB rows' `fetch: arrow`.
    """
    for name in ("json", "jsonb"):
        conn.adapters.register_loader(name, psycopg.types.string.TextLoader)


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
               *, count_mode: str = "first-column") -> tuple[int, float, float, int | None]:
    """Run ``sql`` once, fully materialising results.

    Returns ``(result_count, wall_seconds, server_seconds, peak_backend_rss_bytes)``.
    The fourth value is sampled from the PostgreSQL backend process, never the
    Python client process. Rows are read
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
    pid = backend_pid(conn)

    def execute() -> tuple[list, float]:
        with conn.cursor() as cur:
            start = time.perf_counter()
            cur.execute(sql, args)
            rows = cur.fetchall() if cur.description is not None else []
            return rows, time.perf_counter() - start

    host_pid = host_pid_of(conn, pid)
    if host_pid is None:
        # A container PID is not safe to interpret as a host PID. Preserve an
        # unavailable measurement as blank rather than sampling another process.
        rows, wall = execute()
        peak_rss = None
    else:
        (rows, wall), peak_rss = peak_resident_bytes(execute, pid=host_pid)

    count = extract_count(rows, count_mode)

    with conn.cursor() as cur:
        cur.execute(f"EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}", args)
        payload = cur.fetchone()[0]
    server = parse_explain_execution_time(payload)

    return count, wall, server, peak_rss


def backend_pid(conn: psycopg.Connection) -> int:
    with conn.cursor() as cur:
        cur.execute("SELECT pg_backend_pid()")
        return int(cur.fetchone()[0])


def host_pid_of(conn: psycopg.Connection, pid: int) -> int | None:
    """The host PID for a backend PID inside a container, or None.

    A container PID is not safe to interpret as a host PID, so an
    unresolvable one yields None and the caller records a blank memory
    figure rather than sampling some other process.
    """
    port = getattr(getattr(conn, "info", None), "port", None)
    container_by_port = {
        int(os.environ.get("CITYBENCH_CJDB_PORT", "55432")):
            os.environ.get("CITYBENCH_CJDB_CONTAINER", "citybench-cjdb"),
        int(os.environ.get("CITYBENCH_CITYDB_PORT", "55433")):
            os.environ.get("CITYBENCH_CITYDB_CONTAINER", "citybench-citydb"),
    }
    container = container_by_port.get(port)
    host_pid = host_pid_from_podman(container, pid) if container else None
    if host_pid is None:
        host_pid = host_pid_for_namespace_pid(
            pid,
            container_init_pid=container_init_host_pid(container) if container else None,
        )
    return host_pid


def time_write(conn: psycopg.Connection, sql: str, args: tuple = (),
               *, reset: tuple[tuple[str, tuple], ...] = ()
               ) -> tuple[int, float, int | None]:
    """Run one MUTATING statement once, timed, and report rows touched.

    Deliberately NOT `time_query`: that function re-runs its statement under
    `EXPLAIN (ANALYZE, BUFFERS)` to obtain `server_time_s`, and
    `EXPLAIN ANALYZE` on an INSERT/UPDATE/DELETE **executes** it. Reusing it
    here would apply CJDB's Q6 twice (two `footprint_area` property rows per
    Building on 3DCityDB), increment Q7 by 20 rather than 10, and rewrite
    half a million cjdb tuples a second time per sample. Write rows
    therefore carry no `server_time_s`, and the README's CSV contract says
    so.

    ``reset`` runs first, UNTIMED, so every sample measures the same work
    even though the statements themselves are not idempotent.
    """
    for reset_sql, reset_args in reset:
        with conn.cursor() as cur:
            cur.execute(reset_sql, reset_args)

    pid = backend_pid(conn)

    def execute() -> tuple[int, float]:
        with conn.cursor() as cur:
            start = time.perf_counter()
            cur.execute(sql, args)
            touched = cur.rowcount
            return touched, time.perf_counter() - start

    host_pid = host_pid_of(conn, pid)
    if host_pid is None:
        (touched, wall), peak_rss = execute(), None
    else:
        (touched, wall), peak_rss = peak_resident_bytes(execute, pid=host_pid)
    return int(touched), wall, peak_rss


def set_parallel_query(conn: psycopg.Connection, workers: int) -> None:
    """Set the per-query parallel-worker budget on the benchmark session.

    ``workers = 0`` keeps execution in the one measured backend process,
    which makes the per-query RSS boundary unambiguous: without it
    PostgreSQL could add parallel workers whose resident memory is outside
    that PID. That is the PRIMARY configuration.

    A non-zero value is the disclosed second configuration. `parallel_setup_cost`
    and `min_parallel_table_scan_size` are deliberately left at their
    defaults: raising the worker cap is a resource decision, lowering the
    planner's thresholds would be tuning the query, and only the first is
    what "give PostgreSQL the CPU budget DuckDB gets" means.

    The actual number of workers a query receives is also bounded by the
    cluster-wide `max_worker_processes` pool, which the manifest records
    alongside this setting — asking for eight and getting fewer is a fact
    about the run, not a detail to leave implicit.
    """
    with conn.cursor() as cur:
        cur.execute(f"SET max_parallel_workers_per_gather = {int(workers)}")


def parallel_settings(conn: psycopg.Connection) -> dict[str, str]:
    """What the benchmark session's parallelism actually resolves to.

    Read back from the session itself rather than from the configuration
    file, because `set_parallel_query` overrides the file's value.
    """
    names = ("max_parallel_workers_per_gather", "max_parallel_workers",
             "max_worker_processes", "parallel_setup_cost",
             "min_parallel_table_scan_size")
    with conn.cursor() as cur:
        cur.execute(
            "SELECT name, current_setting(name) FROM pg_settings "
            "WHERE name = ANY(%s)", (list(names),)
        )
        return dict(cur.fetchall())


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
    if mode == "write-rowcount":
        raise ValueError(
            "write scenarios report the cursor's rowcount, not a result "
            "set; use time_write(), not extract_count()"
        )
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
