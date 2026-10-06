"""DuckDB reading a CityParquet package directly.

This is the fair SQL-to-SQL counterpart to the PostgreSQL systems: a SQL
engine querying our format, against SQL engines querying theirs.
"""

from __future__ import annotations

import shutil
import time
from pathlib import Path

import duckdb

import dataclasses
import multiprocessing
import resource

from citybench import memory
from citybench.parquet_sizes import package_sizes
from citybench.config import (
    Dataset, IngestResult, Measurement, Params, SizeReport, object_table_files,
)
from citybench import extension
from citybench.lifecycle import duckdb_temp_directory
from citybench.scenarios import registry, sql_duckdb
from citybench.systems import pg
from citybench.systems.base import register
from citybench.stats import peak_resident_bytes



#: The name the package schema is loaded under by `PRAGMA cityparquet_read`
#: for the write tier.
PACKAGE_SCHEMA = "pkg"


def _arrow(result):
    """Materialise a DuckDB result as an Arrow table.

    Arrow, not `fetchall()`: a Python-object fetch of a row-returning
    scenario measures the client's per-value object construction (about
    72 us/row was measured on `SELECT *`), which is not what this benchmark
    compares. `to_arrow_table` is the current spelling; `fetch_arrow_table`
    is its deprecated alias and is the only one on older clients this
    project's own floor (duckdb >= 1.3.2) allows.
    """
    fetch = getattr(result, "to_arrow_table", None) or result.fetch_arrow_table
    return fetch()


@register
class DuckDBCityParquet:
    tag = "duckdb-cityparquet"

    def __init__(self, *, threads: int = 1, memory_limit: str = "32GB",
                 writeback: bool = False) -> None:
        """Reads the dataset's one CityParquet package, rows in Hilbert order.

        That package is the artefact the format family displays as
        "CityParquet" (`benchmark/plot/benchviz/figures.py`), so both
        benchmark families publish the same artefact under that name
        (`notes/benchmark-fairness-review-2026-09-22.md` §4.5).

        ``writeback`` selects the write tier's second row,
        `duckdb-cityparquet-writeback`: the same mutations, timed with the
        package write-back included.

        ``threads`` defaults to 1, the PRIMARY figure: it matches the format
        harness's single-threaded readers and PostgreSQL's single backend.
        The 16-thread configuration is measured and published as a disclosed
        second pass (README, "Tuning and parallelism").
        """
        self._threads = threads
        self._memory_limit = memory_limit
        self._writeback = writeback
        self.tag = "duckdb-cityparquet-writeback" if writeback else "duckdb-cityparquet"
        self._conn: duckdb.DuckDBPyConnection | None = None
        self._package: Path | None = None
        self._columns: dict[str, str] | None = None
        self._schema_loaded = False
        self._spatial = False
        self._building_rows: int | None = None

    def _fresh_config(self) -> dict:
        return {"threads": self._threads, "memory_limit": self._memory_limit,
                "writeback": self._writeback, "package": str(self._package)}

    def prepare(self) -> None:
        # Allowed to load the unsigned local extension build the write tier
        # needs (`citybench.extension`).
        self._conn = extension.connect()
        # Matched to the PostgreSQL containers' limits so no engine is
        # given more of the machine than another.
        self._conn.execute(f"SET threads TO {self._threads}")
        self._conn.execute(f"SET memory_limit = '{self._memory_limit}'")
        temp_directory = str(duckdb_temp_directory()).replace("'", "''")
        self._conn.execute(f"SET temp_directory = '{temp_directory}'")
        # The committed run left this at its default (true). It governs the
        # `geo`-FOOTER conversion path, so turning it off keeps a GeoParquet
        # footer from silently re-typing a column mid-run. MEASURED, not
        # assumed: it does NOT change `geometry_lod0_0`, which CityParquet
        # writes with Parquet's own GEOMETRY logical type and DuckDB 1.5
        # decodes to native `GEOMETRY` either way (README Caveat 18).
        self._conn.execute("SET enable_geoparquet_conversion = false")

    def set_threads(self, threads: int) -> None:
        """Switch between the two thread configurations without re-ingesting."""
        self._threads = threads
        if self._conn is not None:
            self._conn.execute(f"SET threads TO {threads}")

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

    def _column_types(self) -> dict[str, str]:
        """The real column set and TYPES of ``self._table()``, cached.

        Lets `sql_duckdb.sql_for` build LoD-column SQL that matches this
        PARTICULAR package's own LoD tiers instead of assuming delft's —
        see that function's own docstring for why an assumed, hardcoded
        set raised a `BinderException` outright against Montreal. The types
        matter too, for `geometry-scan`: a footprint column comes back as
        native `GEOMETRY` and a solid column as `BLOB`, and `octet_length`
        binds against only one of them.
        """
        if self._columns is None:
            assert self._conn is not None
            rows = self._conn.execute(f"DESCRIBE SELECT * FROM {self._table()}").fetchall()
            self._columns = {row[0]: row[1] for row in rows}
        return self._columns

    def run(self, scenario: str, params: Params, repeat: int,
            window=None, probe=None) -> Measurement:
        assert self._conn is not None
        mode = registry.count_mode(scenario)
        if mode == "write-rowcount":
            return self._run_write(scenario, params, repeat)
        if not _IN_FRESH_PROCESS:
            # A read scenario runs in a FRESH process, so its memory figure
            # is its own and not the peak a previous scenario left behind.
            return run_in_fresh_process(self._fresh_config(), scenario, params,
                                        repeat, window, probe)

        sql, args = sql_duckdb.sql_for(
            scenario, params, self._table(), window, probe=probe,
            columns=self._column_types(),
        )

        def once() -> tuple[int, float, int | None]:
            def execute() -> tuple[int, float]:
                start = time.perf_counter()
                result = self._conn.execute(sql, list(args))
                if mode == "rowcount":
                    # Materialised INSIDE the timed window, as the
                    # PostgreSQL adapters already do, so no engine wins by
                    # handing back a lazy cursor — and to Arrow, not to
                    # Python objects, so the number is the engine's rather
                    # than the client's per-value object construction
                    # (measured at about 72 us/row on `SELECT *`).
                    count = _arrow(result).num_rows
                else:
                    count = pg.extract_count(result.fetchall(), mode)
                return count, time.perf_counter() - start

            source = memory.self_source()
            if source is None:
                count, elapsed = execute()
                return count, elapsed, None
            (count, elapsed), peak = memory.sample(execute, lambda: [0], source)
            return count, elapsed, peak.anon

        once()  # discarded warm-up
        samples = [once() for _ in range(repeat)]
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[],   # in-process: no client-server split to report
            peak_working_mem_bytes=max((s[2] for s in samples if s[2] is not None), default=None),
            peak_heap_bytes=None,
            notes="memory-scope: duckdb-fresh-process fetch: arrow",
        )

    def verify_rows(self, scenario: str, params: Params, window=None,
                    probe=None) -> tuple[list[str], list[tuple]]:
        """The scenario's SQL once more, untimed (`citybench.identity`)."""
        assert self._conn is not None
        sql, args = sql_duckdb.sql_for(
            scenario, params, self._table(), window, probe=probe,
            columns=self._column_types(),
        )
        table = _arrow(self._conn.execute(sql, list(args)))
        columns = table.column_names
        values = [table.column(name).to_pylist() for name in columns]
        return columns, list(zip(*values))

    # --- the write tier ------------------------------------------------
    #
    # CityParquet has no in-place update path: a Parquet file's smallest
    # rewritable unit is a column chunk. The comparable operation is the one
    # the DuckDB CityJSON extension's package model offers — load the
    # package into DuckDB tables, mutate them, write the package back — so
    # that is what is measured, and the load is untimed setup.

    def _ensure_package(self) -> str:
        """Load the package as a DuckDB schema, once, UNTIMED.

        `PRAGMA cityparquet_read` rather than `read_parquet`: only the
        former recovers each file's Parquet footer, and without it
        `cityparquet_write` has no CRS to state and writes an explicit null
        (`lib/duckdb-cityjson/docs/FUNCTIONS.md`, "Two ways to load a
        package"). The write-back rows would then measure a package write
        that declared less than the input did.
        """
        assert self._conn is not None and self._package is not None
        if not self._schema_loaded:
            # An explicitly chosen build, never whatever a bare
            # `LOAD cityjson` finds installed (`citybench.extension`).
            extension.load(self._conn, extension.resolve())
            package = str(self._package).replace("'", "''")
            self._conn.execute(
                f"PRAGMA cityparquet_read('{package}', '{PACKAGE_SCHEMA}')"
            )
            self._schema_loaded = True
        return PACKAGE_SCHEMA

    def _footprint_area(self) -> tuple[str, str]:
        """``(expression, note)`` for a Building's plan area.

        `ST_Area` over the LoD0 footprint where DuckDB's spatial extension
        and the package's footprint column are both available; the `bbox`
        rectangle's area otherwise. The two are NOT the same quantity — a
        bounding rectangle's area is an upper bound on the footprint's — so
        which was used is stamped into the row's `notes`. CJDB's own Q6
        already mixes the two (`ST_Area(ground_geometry)` on cjdb against
        `ST_Area(envelope)` on 3DCityDB), and the README discloses both
        asymmetries in the same place.
        """
        assert self._conn is not None
        column = sql_duckdb.FOOTPRINT_COLUMN
        if column in self._column_types():
            if not self._spatial:
                try:
                    self._conn.execute("LOAD spatial")
                    self._spatial = True
                except duckdb.Error:
                    self._spatial = False
            if self._spatial:
                return f"ST_Area({column})", "area: st_area(lod0)"
        return (
            "(bbox.xmax - bbox.xmin) * (bbox.ymax - bbox.ymin)",
            "area: bbox-rectangle",
        )

    def _building_row_count(self, schema: str) -> int:
        """Buildings in the package — `attr-delete`'s DEFINED result count.

        DuckDB's `ALTER TABLE ... DROP COLUMN` reports no rowcount, so this
        row's `result_count` is a definition (the rows the other systems'
        `DELETE`/`jsonb_set_lax` touch), not a measurement. Stated here and
        in the README rather than left to look like one.
        """
        assert self._conn is not None
        if self._building_rows is None:
            self._building_rows = int(self._conn.execute(
                f"SELECT count(*) FROM {schema}.building WHERE object_type = ?",
                [sql_duckdb.BUILDING_TYPE],
            ).fetchone()[0])
        return self._building_rows

    def _scratch(self) -> Path:
        base = duckdb_temp_directory() / "cityparquet-write"
        base.mkdir(parents=True, exist_ok=True)
        return base / self.tag

    def _run_write(self, scenario: str, params: Params,
                   repeat: int) -> Measurement:
        assert self._conn is not None
        schema = self._ensure_package()
        area, area_note = self._footprint_area()
        append = params.append
        statements = sql_duckdb.write_statements(scenario, schema, area, append)
        resets = sql_duckdb.write_reset_statements(scenario, schema, area, append)
        # `append-object` adds objects rather than touching Buildings, and
        # `PRAGMA insert_cityjsonseq` reports no rowcount, so its defined
        # count is the CityObject count of the appended file — the Building
        # plus its parts — which is what every system's row reports.
        defined_count = (
            append.object_count if scenario == "append-object" and append
            else self._building_row_count(schema)
        )

        def once(reset: bool) -> tuple[int, float, int | None]:
            if reset:
                for statement, statement_args in resets:
                    self._conn.execute(statement, list(statement_args))
            target = self._scratch()
            if self._writeback and target.exists():
                shutil.rmtree(target)

            def execute() -> tuple[int, float]:
                touched: int | None = None
                start = time.perf_counter()
                for statement, statement_args in statements:
                    rows = self._conn.execute(
                        statement, list(statement_args)
                    ).fetchall()
                    if (len(rows) == 1 and len(rows[0]) == 1
                            and isinstance(rows[0][0], int)):
                        touched = int(rows[0][0])
                if self._writeback:
                    out = str(target).replace("'", "''")
                    self._conn.execute(
                        f"SELECT * FROM cityparquet_write('{schema}', '{out}/')"
                    ).fetchall()
                return (defined_count if touched is None else touched,
                        time.perf_counter() - start)

            (count, elapsed), peak = peak_resident_bytes(execute)
            return count, elapsed, peak

        # No discarded warm-up: a warm-up here would be a real mutation, and
        # the untimed reset that undoes it is exactly what the timed samples
        # already pay for.
        samples = [once(index > 0) for index in range(repeat)]
        if resets and scenario == "append-object":
            # Once more after the LAST sample, so the loaded package this
            # connection holds ends in the state every other scenario was
            # measured against rather than carrying an appended object.
            for statement, statement_args in resets:
                self._conn.execute(statement, list(statement_args))
        scope = "in-engine+package-write" if self._writeback else "in-engine"
        detail = (
            f"importer: insert_cityjsonseq objects: {defined_count}"
            if scenario == "append-object" else area_note
        )
        return Measurement(
            result_count=samples[0][0],
            times_s=[s[1] for s in samples],
            server_times_s=[],
            peak_working_mem_bytes=max((s[2] for s in samples if s[2] is not None), default=None),
            peak_heap_bytes=None,
            notes=f"memory-scope: duckdb-fresh-process write-tier: {scope} {detail}",
        )

    def size(self) -> SizeReport:
        assert self._package is not None
        # "Without indexes" drops the Bloom filters and page indexes inside
        # the files; nothing is built beside the package.
        return package_sizes(self._package)

    def build_indexes(self, params: Params) -> float | None:
        """Nothing to build: statistics and Bloom filters are in the file."""
        return None

    def teardown(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None


_IN_FRESH_PROCESS = False


def _fresh_child(config: dict, scenario: str, params: Params, repeat: int,
                 window, probe) -> Measurement:
    """The body of the fresh per-scenario process (spawned, never forked)."""
    global _IN_FRESH_PROCESS
    _IN_FRESH_PROCESS = True
    package = Path(config.pop("package"))
    system = DuckDBCityParquet(**config)
    system.prepare()
    system._package = package
    m = system.run(scenario, params, repeat, window=window, probe=probe)
    if m.peak_working_mem_bytes is None and memory.self_source() is None:
        # No procfs (macOS): the peak RSS of this fresh process, from
        # getrusage (bytes on macOS), stands in; the manifest names it.
        peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        m = dataclasses.replace(m, peak_working_mem_bytes=int(peak))
    system.teardown()
    return m


def run_in_fresh_process(config: dict, scenario: str, params: Params, repeat: int,
                         window, probe) -> Measurement:
    ctx = multiprocessing.get_context("spawn")
    with ctx.Pool(1) as pool:
        return pool.apply(_fresh_child, (config, scenario, params, repeat, window, probe))
