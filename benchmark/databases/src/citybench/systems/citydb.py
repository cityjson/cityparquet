"""3DCityDB v5: CityJSON imported via citydb-tool into PostgreSQL+PostGIS.

The tool itself is a Java CLI, run from a pinned container image
(`docker/citydb.Dockerfile`) rather than needing a specific JRE on the
host. The container runtime is rootless podman (`just up`/`just down`
drive `podman-compose`; see `benchmark/databases/justfile`), not docker — the
`docker` binary happens to exist on this host too, but is not the
runtime this harness's containers run under.
"""

from __future__ import annotations

import subprocess
import time

from citybench.config import Dataset, IngestResult, Measurement, Params, SizeReport
from citybench.lifecycle import citydb_tool_temp_directory
from citybench.scenarios import registry, sql_citydb
from citybench.systems import pg
from citybench.systems.base import register

_IMAGE = "citybench/citydb-tool"

# citydb-tool's `import cityjson` defaults its thread pool to the host's
# `nproc` (128 on this machine — see docs/3dcitydb-v5-schema.md's "Engine
# version" -> "Resource limits" section: `nproc` reads cgroup *affinity*,
# not the 16-core bandwidth quota `compose.yml` actually enforces). Each
# thread opens a database connection; unthrottled, the import blows past
# `postgresql.conf`'s tuned `max_connections = 20` almost immediately
# ("FATAL: sorry, too many clients already" — reproduced and fixed in Task
# 5). This is a required flag on this host, not an optional performance
# tweak.
_IMPORT_THREADS = "4"


@register
class CityDbSystem:
    tag = "3dcitydb"

    def __init__(self, *, port: int = 55433, schema: str = "citydb",
                 parallel_workers: int = 0) -> None:
        self._port = port
        self._schema = schema
        self._conn = None
        self._srid: int = 0
        self._mount = ""
        self._parallel_workers = parallel_workers
        # Resolved once per ingest, live, rather than hard-coded: the
        # `objectclass`/`datatype` catalogues are schema-version facts.
        self._building_class_id: int | None = None
        self._datatype_id: int | None = None
        # Resolved ONCE per `ingest()` by `sql_citydb.resolve_cityobject_class_ids`
        # (C1 fix) — a plain `objectclass_id IN (...)` over this set replaces
        # the old correlated-subquery predicate in every scenario query.
        # `objectclass` is a fixed schema-version catalogue, not dataset-
        # derived, so this is safe to resolve once and reuse for every
        # scenario `run()` call against the same ingested schema.
        self._cityobject_class_ids: tuple[int, ...] = ()

    def _tool(self, *args: str) -> None:
        subprocess.run(
            [
                "podman", "run", "--rm", "--network", "host",
                "-v", f"{self._mount}:/work",
                "-v", f"{citydb_tool_temp_directory()}:/tmp",
                _IMAGE, *args,
                "-H", "localhost", "-P", str(self._port),
                "-d", "bench", "-u", "bench", "-p", "bench",
                "-S", self._schema,
            ],
            check=True,
        )

    def prepare(self) -> None:
        self._conn = pg.connect(self._port)
        pg.set_parallel_query(self._conn, self._parallel_workers)

    def set_parallel_workers(self, workers: int) -> None:
        """Switch the benchmark session between thread configurations
        without re-ingesting. `citybench run` measures both."""
        self._parallel_workers = workers
        if self._conn is not None:
            pg.set_parallel_query(self._conn, workers)

    def session_settings(self) -> dict[str, str]:
        assert self._conn is not None
        return pg.parallel_settings(self._conn)

    def ingest(self, dataset: Dataset) -> IngestResult:
        self._mount = str(dataset.source.parent.resolve())
        # No schema-creation step: the 3dcitydb-pg image creates the v5
        # schema when its volume is first initialised (see Task 5).
        start = time.perf_counter()
        self._tool(
            "import", "cityjson", f"/work/{dataset.source.name}",
            f"--threads={_IMPORT_THREADS}",
        )
        elapsed = time.perf_counter() - start

        assert self._conn is not None
        with self._conn.cursor() as cur:
            cur.execute(f"SELECT srid FROM {self._schema}.database_srs LIMIT 1")
            row = cur.fetchone()
            self._srid = int(row[0]) if row and row[0] else 0
            # index_ddl() is deliberately empty — see its docstring — but
            # the loop stays so the interface matches cjdb's adapter and so
            # a future genuinely-missing index (a different dataset, a
            # different scenario) is picked up automatically rather than
            # needing a second call site added here.
            for ddl in sql_citydb.index_ddl():
                cur.execute(ddl)
        pg.vacuum_analyze(self._conn, self._schema)
        # C1 fix: resolve the CityObject-granularity predicate's qualifying
        # objectclass_id set ONCE here, from the now-populated `objectclass`
        # catalogue, rather than re-evaluating the correlated recursive
        # predicate on every scenario query. See sql_citydb.py's C1 fix note
        # and resolve_cityobject_class_ids()'s own docstring.
        self._cityobject_class_ids = sql_citydb.resolve_cityobject_class_ids(self._conn)
        self._building_class_id = sql_citydb.resolve_class_id(
            self._conn, sql_citydb.BUILDING_CLASSNAME
        )
        self._datatype_id = sql_citydb.resolve_datatype_id(self._conn)
        return IngestResult(wall_clock_s=elapsed)

    def run(self, scenario: str, params: Params, repeat: int,
            window=None) -> Measurement:
        assert self._conn is not None
        mode = registry.count_mode(scenario)
        if mode == "write-rowcount":
            return self._run_write(scenario, repeat)

        sql, args = sql_citydb.sql_for(
            scenario, params, window, self._srid,
            cityobject_class_ids=self._cityobject_class_ids,
            building_class_id=self._building_class_id,
        )

        pg.time_query(self._conn, sql, args, count_mode=mode)  # discarded warm-up
        samples = [
            pg.time_query(self._conn, sql, args, count_mode=mode)
            for _ in range(repeat)
        ]
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[s[2] for s in samples],
            peak_rss_bytes=max((s[3] for s in samples if len(s) > 3 and s[3] is not None), default=None),
            peak_heap_bytes=None,
            notes="memory-scope: postgresql-backend-rss",
        )

    def _run_write(self, scenario: str, repeat: int) -> Measurement:
        """One write-tier scenario: CJDB's Q6/Q7/Q8 on v5's schema.

        No discarded warm-up, and no `EXPLAIN (ANALYZE)` re-run — see
        `pg.time_write`. `VACUUM ANALYZE` runs after the last sample so the
        bloat one scenario leaves behind is not charged to the next.
        """
        assert self._conn is not None
        assert self._building_class_id is not None and self._datatype_id is not None
        sql, args = sql_citydb.write_sql(
            scenario, self._building_class_id, self._datatype_id
        )
        reset = tuple(sql_citydb.write_reset_sql(
            scenario, self._building_class_id, self._datatype_id
        ))
        samples = [
            pg.time_write(self._conn, sql, args, reset=reset if index else ())
            for index in range(repeat)
        ]
        pg.vacuum_analyze(self._conn, self._schema)
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[],
            peak_rss_bytes=max((s[2] for s in samples if s[2] is not None), default=None),
            peak_heap_bytes=None,
            notes="memory-scope: postgresql-backend-rss write-tier: no-explain",
        )

    def size(self) -> SizeReport:
        assert self._conn is not None
        return pg.schema_size(self._conn, self._schema)

    def teardown(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None
