"""DuckDB reading a CityParquet package directly.

This is the fair SQL-to-SQL counterpart to the PostgreSQL systems: a SQL
engine querying our format, against SQL engines querying theirs.
"""

from __future__ import annotations

import time
from pathlib import Path

import duckdb

from citybench.config import (
    Dataset, IngestResult, Measurement, Params, SizeReport, object_table_files,
)
from citybench.lifecycle import duckdb_temp_directory
from citybench.scenarios import registry, sql_duckdb
from citybench.systems import pg
from citybench.systems.base import register
from citybench.stats import peak_resident_bytes



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
        temp_directory = str(duckdb_temp_directory()).replace("'", "''")
        self._conn.execute(f"SET temp_directory = '{temp_directory}'")

    def ingest(self, dataset: Dataset) -> IngestResult:
        """No load step: DuckDB reads the package in place.

        Recorded as zero wall-clock, which is the honest figure — the
        absence of a load step is the property under discussion, not a
        measurement gap.
        """
        package = dataset.cityparquet_dir
        files = object_table_files(package)
        missing = [path for path in files if not Path(path).is_file()]
        if missing:
            raise FileNotFoundError(
                f"CityParquet object table assets missing from {package}: "
                + ", ".join(missing)
            )
        self._package = package
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

        def once() -> tuple[int, float, int | None]:
            def execute() -> tuple[int, float]:
                start = time.perf_counter()
                rows = self._conn.execute(sql, list(args)).fetchall()
                return pg.extract_count(rows, mode), time.perf_counter() - start

            (count, elapsed), peak = peak_resident_bytes(execute)
            return count, elapsed, peak

        once()  # discarded warm-up
        samples = [once() for _ in range(repeat)]
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[],   # in-process: no client-server split to report
            peak_rss_bytes=max((s[2] for s in samples if s[2] is not None), default=None),
            peak_heap_bytes=None,
            notes="memory-scope: duckdb-process-rss",
        )

    def size(self) -> SizeReport:
        assert self._package is not None
        total = sum(f.stat().st_size for f in self._package.rglob("*") if f.is_file())
        return SizeReport(size_bytes=total, size_bytes_no_index=total)

    def teardown(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None
