"""DuckDB reading a CityParquet package directly.

This is the fair SQL-to-SQL counterpart to the PostgreSQL systems: a SQL
engine querying our format, against SQL engines querying theirs.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import duckdb

from citybench.config import Dataset, IngestResult, Measurement, Params, SizeReport
from citybench.scenarios import registry, sql_duckdb
from citybench.systems import pg
from citybench.systems.base import register

# The STAC asset role `cityparquet convert` stamps on every per-module
# OBJECT table it writes (verified against a real converted package's
# metadata.json — see `object_table_files`'s own docstring). Sidecar
# tables (materials/textures/geometry_templates) and the "data" alias
# entry (a convenience duplicate of the FIRST object table, for a
# single-family package) do not carry this role, so filtering on it is
# what tells object tables apart from everything else `assets` lists.
_OBJECT_TABLE_ROLE = "cityparquet-objects"


def object_table_files(package: Path) -> list[str]:
    """The object-table Parquet filenames a CityParquet package lists.

    A single-family package (every delft-shaped dataset in this corpus)
    lists exactly one, ``building.parquet``. A by-type, multi-family
    package (``lod3_railway``, whose 121 CityObjects span 14 CityGML
    types — Railway, Bridge, Tunnel, CityFurniture, ... — none of them
    Building) lists several: ``railway.parquet``, ``bridge.parquet``,
    ``tunnel.parquet``, and so on. An EARLIER version of this system
    hardcoded ``building.parquet`` (this is by-type layout's ONLY table
    name for a Building-only dataset like delft, which is why that bug
    slipped past every prior task's own smoke run), which fails outright
    against a package with no Building table at all — discovered running
    Task 14's heterogeneity corpus, not assumed in advance.

    Resolved from ``metadata.json``'s own ``assets``, filtered to entries
    whose ``roles`` include ``"cityparquet-objects"`` — verified against a
    real converted package to be exactly the per-module object tables,
    excluding both the "data" convenience alias (a duplicate pointer at
    the FIRST object table, carrying only the plain ``"data"`` role) and
    any materials/textures/geometry_templates sidecar assets (which carry
    their own, different roles). Sorted for a deterministic query shape
    across runs.
    """
    manifest = json.loads((package / "metadata.json").read_text())
    hrefs = [
        asset["href"]
        for asset in manifest.get("assets", {}).values()
        if _OBJECT_TABLE_ROLE in asset.get("roles", ())
    ]
    if not hrefs:
        raise ValueError(
            f"{package}/metadata.json lists no asset with role "
            f"{_OBJECT_TABLE_ROLE!r}; not a valid CityParquet package"
        )
    # hrefs are relative ("./building.parquet"); normalise against the
    # package directory so the caller gets absolute, glob-free paths.
    return sorted((package / href).resolve().as_posix() for href in hrefs)


@register
class DuckDBCityParquet:
    tag = "duckdb-cityparquet"

    def __init__(self, *, threads: int = 16, memory_limit: str = "32GB") -> None:
        self._threads = threads
        self._memory_limit = memory_limit
        self._conn: duckdb.DuckDBPyConnection | None = None
        self._package: Path | None = None
        self._columns: frozenset[str] | None = None

    def prepare(self) -> None:
        self._conn = duckdb.connect()
        # Matched to the PostgreSQL containers' limits so no engine is
        # given more of the machine than another.
        self._conn.execute(f"SET threads TO {self._threads}")
        self._conn.execute(f"SET memory_limit = '{self._memory_limit}'")

    def ingest(self, dataset: Dataset) -> IngestResult:
        """No load step: DuckDB reads the package in place.

        Recorded as zero wall-clock, which is the honest figure — the
        absence of a load step is the property under discussion, not a
        measurement gap.
        """
        self._package = dataset.cityparquet_dir
        return IngestResult(wall_clock_s=0.0, notes="no load step")

    def _table(self) -> str:
        assert self._package is not None
        files = object_table_files(self._package)
        if len(files) == 1:
            return f"read_parquet('{files[0]}')"
        # Multi-family (by-type) package: every scenario in sql_duckdb.py
        # queries columns common to every object table (id, object_type,
        # bbox, ...), but a given module's own columns (e.g. a numeric
        # attribute that only Railway rows carry) are absent from every
        # OTHER module's table. `union_by_name` fills those gaps with NULL
        # per file rather than erroring on a schema mismatch — the
        # natural DuckDB mechanism for "one logical table split across
        # several same-family-ish Parquet files with a shared column
        # core", not a hand-rolled UNION ALL that would need to be kept in
        # sync with the schema by hand.
        file_list = ", ".join(f"'{f}'" for f in files)
        return f"read_parquet([{file_list}], union_by_name = true)"

    def _column_names(self) -> frozenset[str]:
        """The real column set of ``self._table()``, discovered once and cached.

        Lets `sql_duckdb.sql_for` build LoD-column SQL that matches this
        PARTICULAR package's own LoD tiers instead of assuming delft's —
        see that function's own docstring for why an assumed, hardcoded
        set raised a `BinderException` outright against Montreal.
        """
        if self._columns is None:
            assert self._conn is not None
            rows = self._conn.execute(f"DESCRIBE SELECT * FROM {self._table()}").fetchall()
            self._columns = frozenset(row[0] for row in rows)
        return self._columns

    def run(self, scenario: str, params: Params, repeat: int,
            selectivity: float | None = None) -> Measurement:
        assert self._conn is not None
        sql, args = sql_duckdb.sql_for(
            scenario, params, self._table(), selectivity,
            columns=self._column_names(),
        )

        mode = registry.count_mode(scenario)

        def once() -> tuple[int, float]:
            start = time.perf_counter()
            rows = self._conn.execute(sql, list(args)).fetchall()
            elapsed = time.perf_counter() - start
            return pg.extract_count(rows, mode), elapsed

        once()  # discarded warm-up
        samples = [once() for _ in range(repeat)]
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[],   # in-process: no client-server split to report
            peak_rss_bytes=None,
            peak_heap_bytes=None,
        )

    def size(self) -> SizeReport:
        assert self._package is not None
        total = sum(f.stat().st_size for f in self._package.rglob("*") if f.is_file())
        return SizeReport(size_bytes=total, size_bytes_no_index=total)

    def teardown(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None
