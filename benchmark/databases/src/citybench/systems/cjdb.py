"""cjdb: CityJSONL imported into PostgreSQL+PostGIS.

Drives a PATCHED cjdb, not stock cjdb==2.2.0 from PyPI — see
`vendor/cjdb/README.md` for the full rationale. In short: stock cjdb's
`get_ground_surfaces()` (`cjdb/modules/geometric.py`) accumulates candidate
footprint faces into a dict keyed by their own mean Z height, so two
non-vertical faces sharing a mean Z silently overwrite one another and
`ground_geometry` ends up missing part of the object's true footprint — an
architecture-independent import defect, not something the CityParquet-vs-
cjdb comparison this benchmark exists to make is about. Benchmarking a
crippled importer would attack a strawman rather than cjdb's actual
row-oriented/JSONB architecture. `scripts/patch_cjdb.sh` (`just
patch-cjdb`) builds the patched source this module drives; every ingest
disclosed via `manifest.py`/`results/<dataset>.manifest.json` records that
the patch was applied.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
import time
from pathlib import Path

from citybench.config import (
    AppendSpec, Dataset, IngestResult, Measurement, Params, SizeReport,
)
from citybench.scenarios import registry, sql_cjdb
from citybench.scenarios.registry import ScenarioUnavailable
from citybench.systems import pg
from citybench.systems.base import register

# Matches docker/compose.yml's cjdb-db service exactly (see this harness's
# docker-compose file): password is supplied via PGPASSWORD, never a CLI
# flag — `cjdb import` has no positional filepath argument either, only
# `-f`/`--filepath`.
_HOST = "localhost"
_USER = "bench"
_DATABASE = "bench"
_PASSWORD = "bench"

# benchmark/databases/src/citybench/systems/cjdb.py -> parents[3] == benchmark/databases/
_BENCH_ROOT = Path(__file__).resolve().parents[3]
_PATCH_FILE = _BENCH_ROOT / "vendor" / "cjdb" / "ground-surfaces-tie.patch"
_POINTER_FILE = _BENCH_ROOT / ".cjdb-patched" / "current-path"
CJDB_UPSTREAM_VERSION = "2.2.0"


def patched_cjdb_source() -> Path:
    """The local, patched cjdb source `--with` points at.

    Built by `scripts/patch_cjdb.sh` (`just patch-cjdb`), which must be run
    once before any `CjdbSystem.ingest()` call. Deliberately NOT built
    automatically here — the build step downloads from PyPI and is slow
    enough on a cold cache that doing it silently, mid-benchmark, would be
    a surprise rather than a courtesy (the same reasoning
    `ReadbenchSystem.prepare()` already applies to its own missing-binary
    check).
    """
    if not _POINTER_FILE.exists():
        raise FileNotFoundError(
            f"{_POINTER_FILE} not found; build the patched cjdb with "
            "`just patch-cjdb` (or `./scripts/patch_cjdb.sh`) first — see "
            "vendor/cjdb/README.md for why cjdb is patched at all."
        )
    path = Path(_POINTER_FILE.read_text().strip())
    if not path.is_dir():
        raise FileNotFoundError(
            f"{_POINTER_FILE} points at {path}, which does not exist; "
            "re-run `just patch-cjdb`."
        )
    # The build directory's own name embeds the patch file's hash at build
    # time (see patch_cjdb.sh) precisely so this check is possible: if
    # ground-surfaces-tie.patch has been edited since the last build, the
    # current hash will not match the directory name, and continuing would
    # silently run a stale patch rather than the one actually committed —
    # `uv run --with <path>`'s own build cache does not reliably notice an
    # in-place source change at a fixed path (confirmed while building this
    # mechanism), which is exactly why the path is content-addressed at all.
    current_hash = _sha256(_PATCH_FILE)[:12]
    if current_hash not in path.name:
        raise RuntimeError(
            f"{path} was built from an OLDER version of {_PATCH_FILE} "
            f"(current patch hash: {current_hash}); re-run `just "
            "patch-cjdb` to rebuild against the current patch."
        )
    return path


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def patch_disclosure() -> dict[str, str]:
    """What the run manifest stamps for cjdb — see `manifest.py`.

    A reader of `results/<dataset>.manifest.json` must never be able to
    mistake these numbers for stock cjdb 2.2.0's; this is the one place
    that fact is assembled, so `cli.py` and any other caller stay in sync.
    """
    source = patched_cjdb_source()
    return {
        "upstream_version": CJDB_UPSTREAM_VERSION,
        "patched": "true",
        "patch_file": "vendor/cjdb/ground-surfaces-tie.patch",
        "patch_sha256": _sha256(_PATCH_FILE),
        "patch_summary": (
            "get_ground_surfaces() retains tied-Z footprint faces; the "
            "CityJSONSeq importer also streams input and uses 5,000-row "
            "INSERT batches. Object batches share cjdb's original object "
            "transaction; relationship batches remain deferred until all "
            "objects are present. See vendor/cjdb/README.md."
        ),
        "built_from": str(source),
    }


def _append(append: AppendSpec | None) -> AppendSpec:
    if append is None:
        raise ScenarioUnavailable(
            "no one-feature append file was derived for this dataset"
        )
    return append


@register
class CjdbSystem:
    tag = "cjdb"

    def __init__(self, *, port: int = 55432, schema: str = "cjdb",
                 parallel_workers: int = 0) -> None:
        self._port = port
        self._schema = schema
        self._conn = None
        self._srid: int = 0
        self._parallel_workers = parallel_workers

    def prepare(self) -> None:
        # Fail fast, before touching the database, if the patched cjdb
        # this class always drives (never stock cjdb — see the module
        # docstring) has not been built yet.
        patched_cjdb_source()
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

    def _import(self, path: str, *, overwrite: bool) -> None:
        """One `cjdb import` of one file — the ingest path, reused verbatim
        by `append-object` on the derived one-feature file.

        `--overwrite` on the full ingest, none on the append: cjdb keys its
        already-imported check on the SOURCE FILE NAME
        (`cjdb/modules/importer.py`), so importing a differently-named file
        into a populated schema simply adds its objects under a new
        `cj_metadata` row. Without `--overwrite` and WITH a name it has
        seen before it prompts on stdin instead, which in a benchmark is a
        hang rather than a question — `stdin` is closed so that case fails
        loudly instead, and `append-object`'s untimed reset removes the
        `cj_metadata` row that would cause it.
        """
        cjdb_source = patched_cjdb_source()
        subprocess.run(
            [
                "uv", "run", "--with", str(cjdb_source), "cjdb", "import",
                "-H", _HOST, "-p", str(self._port),
                "-U", _USER, "-d", _DATABASE, "-s", self._schema,
                *(("--overwrite",) if overwrite else ()),
                "-f", path,
            ],
            check=True,
            stdin=subprocess.DEVNULL,
            env={**os.environ, "PGPASSWORD": _PASSWORD},
        )

    def ingest(self, dataset: Dataset) -> IngestResult:
        start = time.perf_counter()
        self._import(str(dataset.source), overwrite=True)
        elapsed = time.perf_counter() - start

        assert self._conn is not None
        with self._conn.cursor() as cur:
            cur.execute(f"SELECT srid FROM {self._schema}.cj_metadata LIMIT 1")
            row = cur.fetchone()
            self._srid = int(row[0]) if row and row[0] else 0
            for ddl in sql_cjdb.index_ddl():
                cur.execute(ddl)
        pg.vacuum_analyze(self._conn, self._schema)
        return IngestResult(wall_clock_s=elapsed)

    def run(self, scenario: str, params: Params, repeat: int,
            window=None, probe=None) -> Measurement:
        assert self._conn is not None
        mode = registry.count_mode(scenario)
        if mode == "write-rowcount":
            if scenario == "append-object":
                return self._run_append(params, repeat)
            return self._run_write(scenario, repeat)

        sql, args = sql_cjdb.sql_for(
            scenario, params, window, self._srid, probe=probe
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
        """One write-tier scenario: CJDB's own Q6/Q7/Q8.

        No discarded warm-up, unlike every read scenario: a warm-up here
        would be a real mutation, and the reset that undoes it is exactly
        what the timed samples already pay for. `VACUUM ANALYZE` runs after
        the last sample, never between the reset and the timed statement, so
        no sample's number includes it — but it does run, so the dead tuples
        `attr-add` leaves behind are not silently charged to `attr-update`.
        """
        assert self._conn is not None
        sql, args = sql_cjdb.write_sql(scenario)
        reset = tuple(sql_cjdb.write_reset_sql(scenario))
        samples = [
            pg.time_write(self._conn, sql, args, reset=reset if index else ())
            for index in range(repeat)
        ]
        pg.vacuum_analyze(self._conn, self._schema)
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            # Write rows carry no server_time_s: obtaining it would mean a
            # second EXPLAIN ANALYZE execution of the mutation itself.
            server_times_s=[],
            peak_rss_bytes=max((s[2] for s in samples if s[2] is not None), default=None),
            peak_heap_bytes=None,
            notes="memory-scope: postgresql-backend-rss write-tier: no-explain",
        )

    def _watermarks(self) -> dict[str, int]:
        assert self._conn is not None
        marks: dict[str, int] = {}
        with self._conn.cursor() as cur:
            for table, sql in sql_cjdb.append_watermark_sql():
                cur.execute(sql)
                marks[table] = int(cur.fetchone()[0])
        return marks

    def _reset_append(self, watermarks: dict[str, int]) -> None:
        assert self._conn is not None
        with self._conn.cursor() as cur:
            for sql, args in sql_cjdb.append_reset_sql(watermarks):
                cur.execute(sql, args)

    def _run_append(self, params: Params, repeat: int) -> Measurement:
        """Catalogue B18, through cjdb's OWN importer on the one-feature file.

        Timed as an external process, so the number includes what a cjdb
        user genuinely pays to add an object: `uv`'s resolution of the
        patched source, the interpreter start, the connection, the
        footprint derivation and the inserts. The first two are launcher
        cost rather than database work, so a non-mutating `cjdb --version`
        runs first, UNTIMED, to take the cold resolve out of sample 1 — a
        warm-up of the launcher, not of the mutation, which stays
        un-warmed-up like every other write row.

        Between samples the appended rows are deleted untimed, by
        watermark, and once more after the last sample so the schema ends
        as it began.
        """
        append = _append(params.append)
        self._warm_launcher()
        watermarks = self._watermarks()
        added = 0
        samples: list[tuple[float, int | None]] = []
        for index in range(repeat):
            if index:
                self._reset_append(watermarks)
            start = time.perf_counter()
            self._import(append.path, overwrite=False)
            samples.append((time.perf_counter() - start, None))
            if index == 0:
                added = self._rows_added(watermarks)
        self._reset_append(watermarks)
        pg.vacuum_analyze(self._conn, self._schema)
        return Measurement(
            result_count=append.object_count,
            times_s=[s[0] for s in samples],
            server_times_s=[],
            # The importer is a separate process this harness starts and
            # waits on; it is not the PostgreSQL backend the other rows
            # sample, and sampling the backend would report only the part of
            # the work that reached it.
            peak_rss_bytes=None,
            peak_heap_bytes=None,
            notes=("write-tier: external-importer importer: cjdb-import "
                   f"objects: {append.object_count} "
                   f"city-object-rows-added: {added} "
                   "memory-scope: not-sampled"),
        )

    def _rows_added(self, watermarks: dict[str, int]) -> int:
        """`city_object` rows the import wrote — the importer's own grain.

        cjdb stores one row per CityObject, so this should equal the file's
        CityObject count; it is measured rather than assumed, because "what
        each importer actually writes" is the disclosure this row exists
        for.
        """
        assert self._conn is not None
        with self._conn.cursor() as cur:
            cur.execute(
                f"SELECT count(*) FROM {self._schema}.city_object WHERE id > %s",
                (watermarks["city_object"],),
            )
            return int(cur.fetchone()[0])

    def _warm_launcher(self) -> None:
        """Resolve the patched cjdb source once, UNTIMED and non-mutating."""
        subprocess.run(
            ["uv", "run", "--with", str(patched_cjdb_source()), "cjdb",
             "--help"],
            check=False, stdin=subprocess.DEVNULL,
            capture_output=True,
        )

    def size(self) -> SizeReport:
        assert self._conn is not None
        return pg.schema_size(self._conn, self._schema)

    def teardown(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None
