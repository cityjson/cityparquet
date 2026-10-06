"""The run manifest: everything a published number must be cited with.

Ingest timings live here rather than in the results CSV on purpose. The
spec scopes this benchmark to steady state; encoding a Parquet file and
populating a normalised relational schema are different operations, and
presenting them side by side as a comparison would be indefensible. They
are recorded as context, with the caveat attached to the data itself.

`patches` exists for the same reason: a reader of this manifest must never
be able to mistake a patched system's numbers for stock upstream's. cjdb is
the first (see `citybench.systems.cjdb.patch_disclosure` and
`vendor/cjdb/README.md`) — a dedicated, well-typed section rather than
folding a patch note into `versions` (which stays a flat name -> version
string mapping) keeps "what changed and why" legible on its own, even
though `_versions()` also stamps a terse marker for at-a-glance visibility.
"""

from __future__ import annotations

import os

from citybench.lifecycle import MEMORY_LIMIT

# DuckDBCityParquet's default `memory_limit` (systems/duckdb_cp.py).
DUCKDB_MEMORY_LIMIT = "32GB"
import platform
import hashlib
from typing import Any

_INGEST_CAVEAT = (
    "Ingest timings are context only and are NOT comparable across systems. "
    "Encoding a CityParquet package and populating an indexed relational "
    "schema are different operations; this benchmark is scoped to "
    "steady-state query performance."
)


def required_keys() -> tuple[str, ...]:
    return (
        "dataset", "source", "baseline", "host", "versions", "pg_settings", "ingest", "sizes",
        "patches", "srid", "memory_measurement", "temporary_storage",
        "execution", "count_check", "isolation",
    )


SIZE_DEFINITIONS = {
    "policy": (
        "Every queried predicate is indexed where the system supports it, "
        "and index sizes are reported separately."
    ),
    "total_bytes": {
        "postgresql": "pg_total_relation_size over every table of the system's schema: heap, TOAST and all indexes",
        "cityparquet": "every file of the package directory",
    },
    "no_index_bytes": {
        "postgresql": "pg_table_size over the same tables: heap and TOAST, no index",
        "cityparquet": (
            "the package without the Bloom filters and page indexes (column "
            "and offset index) inside its Parquet files; the footer, which "
            "holds the schema and the row-group min/max statistics, is kept, "
            "since a reader cannot read the file without it"
        ),
    },
}


def _setting(pg_settings: dict, key: str):
    """A setting from a flat or a per-system (``{tag: {...}}``) mapping."""
    if key in pg_settings:
        return pg_settings[key]
    return {tag: v.get(key) for tag, v in pg_settings.items() if isinstance(v, dict)} or None


def collect(*, dataset_name: str, source: str | None = None, ingest: dict[str, float],
            sizes: dict[str, tuple[int, int]], versions: dict[str, str],
            pg_settings: dict[str, str],
            patches: dict[str, dict[str, str]] | None = None,
            srid: dict[str, int] | None = None,
            execution: dict[str, Any] | None = None,
            count_check: dict[str, Any] | None = None,
            isolation: dict[str, Any] | None = None,
            index_build: dict[str, float | None] | None = None,
            size_detail: dict[str, dict[str, int]] | None = None,
            memory_read: dict[str, str] | None = None) -> dict[str, Any]:
    """``srid`` — the SRID each PostgreSQL-backed system actually landed on.

    Added for Task 14 (the heterogeneity corpus): 3DCityDB's SRID is baked
    in at schema/volume creation (`docker/compose.yml`'s `CITYDB_SRID`) and
    cannot be changed afterwards, and Montreal/Vienna/Zurich each need a
    DIFFERENT one from delft's default (7415) — getting this wrong does
    NOT error, it silently mislabels or reprojects geometry. Read back
    from each system's own adapter (`CjdbSystem._srid`/`CityDbSystem._srid`,
    themselves read from `cj_metadata`/`database_srs` immediately after
    import — the value the DATABASE actually recorded, not merely the one
    requested), so this is a verification the SRID landed, not a restated
    request.
    """
    return {
        "dataset": dataset_name,
        "source": source,
        "baseline": "3dcitydb",
        "host": {
            "platform": platform.platform(),
            "processor": platform.processor(),
            "python": platform.python_version(),
        },
        "versions": versions,
        "pg_settings": pg_settings,
        "ingest": {
            "wall_clock_s": ingest,
            # The per-dataset predicate indexes, built after the import and
            # timed apart from it; null for a system that builds none.
            "index_build_s": index_build or {},
            "caveat": _INGEST_CAVEAT,
        },
        "sizes": {
            tag: {
                "total_bytes": total,
                "no_index_bytes": no_idx,
                "index_bytes": total - no_idx,
                **(size_detail or {}).get(tag, {}),
            }
            for tag, (total, no_idx) in sizes.items()
        },
        "size_definitions": SIZE_DEFINITIONS,
        "patches": patches or {},
        "srid": srid or {},
        # Both thread configurations of this run, and what each system's
        # session actually resolved them to — asking PostgreSQL for eight
        # workers and getting fewer, because `max_worker_processes` bounds
        # the pool, is a fact about the run rather than a detail to leave
        # implicit.
        "execution": execution or {},
        # The tolerance the cross-system count check was run with, and what
        # a status of `ok-deviation` means in the CSV it produced.
        "count_check": count_check or {},
        # NUMA pinning, container cpusets, the load gate and its per-cell
        # load record: what was requested, what was applied, and why not.
        "isolation": isolation or {},
        "memory_measurement": {
            "metric": "peak_working_mem_bytes",
            "postgresql": (
                "peak over sampling instants of the summed RssAnon (/proc/<pid>/status) of "
                "the query's backend and, under the parallel configuration, its parallel "
                "worker processes (pg_stat_activity.leader_pid, read from a second "
                "connection). Excludes RssShmem (shared_buffers), RssFile and the OS page "
                "cache, other backends, background processes and the client. One reading "
                "before and one after the query are included, so a query shorter than the "
                "interval records the backend at its edges (a lower bound)"
            ),
            "duckdb": (
                "each read scenario runs in a fresh spawned process; with procfs, the peak "
                "RssAnon of that process (includes DuckDB's idle baseline); without procfs "
                "(macOS) the process's peak RSS from getrusage ru_maxrss stands in. Write "
                "scenarios stay in the long-lived process"
            ),
            "cityparquet": "allocator peak (peak_heap_bytes) of the fresh reader child process",
            "sampling_interval_ms": {
                "host_proc": 5,
                "engine_exec": "50 between readings, plus the exec round trip itself",
            },
            "read_path": {"postgresql": "not applied: no PostgreSQL query was measured",
                          **(memory_read or {})},
            "provisioned": {
                "shared_buffers": _setting(pg_settings, "shared_buffers"),
                "work_mem": _setting(pg_settings, "work_mem"),
                "container_memory_limit": MEMORY_LIMIT,
                "duckdb_memory_limit": DUCKDB_MEMORY_LIMIT,
            },
        },
        "temporary_storage": {
            "host_tmpdir": os.environ.get("TMPDIR"),
            "run_root": os.environ.get("CITYBENCH_TEMP_DIR"),
            "citydb_tool_tmpdir": os.environ.get("CITYBENCH_CITYDB_TOOL_TMPDIR"),
            "duckdb_tmpdir": os.environ.get("CITYBENCH_DUCKDB_TMPDIR"),
            "description": "Per-run scratch directories are under host_tmpdir; PostgreSQL containers receive separate /tmp binds, and DuckDB sets temp_directory explicitly.",
        },
    }
