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
    )


def collect(*, dataset_name: str, source: str | None = None, ingest: dict[str, float],
            sizes: dict[str, tuple[int, int]], versions: dict[str, str],
            pg_settings: dict[str, str],
            patches: dict[str, dict[str, str]] | None = None,
            srid: dict[str, int] | None = None) -> dict[str, Any]:
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
        "ingest": {"wall_clock_s": ingest, "caveat": _INGEST_CAVEAT},
        "sizes": {
            tag: {"total_bytes": total, "no_index_bytes": no_idx}
            for tag, (total, no_idx) in sizes.items()
        },
        "patches": patches or {},
        "srid": srid or {},
        "memory_measurement": {
            "metric": "peak_rss_bytes",
            "scope": "execution process only",
            "postgresql": "query backend PID after disabling parallel workers; excludes other backend, background-worker, and idle-server processes; mapped shared pages can contribute to RSS; blank if procfs namespace mapping cannot be verified",
            "duckdb": "process executing the embedded engine; includes its idle baseline",
            "cityparquet": "fresh reader child process; includes its idle baseline",
            "sampling_interval_ms": 5,
        },
        "temporary_storage": {
            "host_tmpdir": os.environ.get("TMPDIR"),
            "run_root": os.environ.get("CITYBENCH_TEMP_DIR"),
            "citydb_tool_tmpdir": os.environ.get("CITYBENCH_CITYDB_TOOL_TMPDIR"),
            "duckdb_tmpdir": os.environ.get("CITYBENCH_DUCKDB_TMPDIR"),
            "description": "Per-run scratch directories are under host_tmpdir; PostgreSQL containers receive separate /tmp binds, and DuckDB sets temp_directory explicitly.",
        },
    }
