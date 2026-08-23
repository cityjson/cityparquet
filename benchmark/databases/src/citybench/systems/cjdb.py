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

from citybench.config import Dataset, IngestResult, Measurement, Params, SizeReport
from citybench.scenarios import registry, sql_cjdb
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
        "patch_summary": (
            "get_ground_surfaces() no longer drops non-vertical footprint "
            "faces that share a mean Z height with another face (was a "
            "dict keyed by mean Z; now a list, so ties are retained). "
            "See vendor/cjdb/README.md."
        ),
        "built_from": str(source),
    }


@register
class CjdbSystem:
    tag = "cjdb"

    def __init__(self, *, port: int = 55432, schema: str = "cjdb") -> None:
        self._port = port
        self._schema = schema
        self._conn = None
        self._srid: int = 0

    def prepare(self) -> None:
        # Fail fast, before touching the database, if the patched cjdb
        # this class always drives (never stock cjdb — see the module
        # docstring) has not been built yet.
        patched_cjdb_source()
        self._conn = pg.connect(self._port)

    def ingest(self, dataset: Dataset) -> IngestResult:
        cjdb_source = patched_cjdb_source()
        start = time.perf_counter()
        subprocess.run(
            [
                "uv", "run", "--with", str(cjdb_source), "cjdb", "import",
                "-H", _HOST, "-p", str(self._port),
                "-U", _USER, "-d", _DATABASE, "-s", self._schema,
                "--overwrite", "-f", str(dataset.source),
            ],
            check=True,
            env={**os.environ, "PGPASSWORD": _PASSWORD},
        )
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
            selectivity: float | None = None) -> Measurement:
        assert self._conn is not None
        sql, args = sql_cjdb.sql_for(scenario, params, selectivity, self._srid)
        mode = registry.count_mode(scenario)

        pg.time_query(self._conn, sql, args, count_mode=mode)  # discarded warm-up
        samples = [
            pg.time_query(self._conn, sql, args, count_mode=mode)
            for _ in range(repeat)
        ]
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[s[2] for s in samples],
            peak_rss_bytes=None,
            peak_heap_bytes=None,
        )

    def size(self) -> SizeReport:
        assert self._conn is not None
        return pg.schema_size(self._conn, self._schema)

    def teardown(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None
