"""Command-line entry point behind the justfile."""

from __future__ import annotations

import argparse
import csv
import json
import sys
import os
from contextlib import nullcontext
from citybench.lifecycle import isolated_databases
from pathlib import Path

from citybench import manifest, params as params_mod
from citybench.config import Dataset
from citybench.report import write_csv
from citybench.runner import run_matrix
from citybench.scenarios import sql_citydb, sql_cjdb
from citybench.runner import DEFAULT_COUNT_TOLERANCE
from citybench.scenarios.registry import READ_SCENARIOS, TIER3
from citybench.systems import pg
from citybench.systems.cjdb import CjdbSystem
from citybench.systems.citydb import CityDbSystem
from citybench.systems.duckdb_cp import DuckDBCityParquet
from citybench.systems.readbench import ReadbenchSystem

ROOT = Path(__file__).resolve().parents[2]
# ROOT is benchmark/databases/; the read harness this shells out to is its own
# Cargo workspace next door, built by
# `cargo build --release --manifest-path benchmark/readbench/Cargo.toml`.
BENCHMARK_DIR = ROOT.parent
READBENCH_BIN = BENCHMARK_DIR / "readbench" / "target" / "release" / "cityparquet-readbench"


def _dataset(source: Path, prepared_dir: Path | None = None) -> Dataset:
    name = Dataset.name_from_path(source)
    prepared = prepared_dir or BENCHMARK_DIR / "formats" / "data" / "readbench"
    return Dataset(name=name, source=source, cityparquet_dir=prepared / f"{name}.parquet", hilbert_dir=prepared / f"{name}-hilbert.parquet")


#: The two named execution conditions, both measured and both reported.
#:
#: `single` is the PRIMARY figure: DuckDB on one thread, PostgreSQL in one
#: backend. It is the condition under which the two engines are asked for
#: the same amount of CPU, and it matches the format harness's
#: single-threaded readers. The committed run's DuckDB ran on 16 threads
#: against a PostgreSQL with parallel query disabled, which concentrated a
#: 5-8x advantage on exactly the headline rows
#: (`notes/benchmark-fairness-review-2026-09-22.md` §4.3).
#:
#: `parallel` gives both engines a parallel budget. PostgreSQL's planner
#: thresholds (`parallel_setup_cost`, `min_parallel_table_scan_size`) stay
#: at their defaults: raising the worker cap is a resource decision;
#: lowering those would be tuning the query.
THREAD_CONFIGURATIONS: tuple[tuple[str, int, int], ...] = (
    ("single", 1, 0),
    ("parallel", 16, 8),
)


def _build_systems(tags: list[str], *, ports: dict[str, int] | None = None) -> list:
    ports = ports or {}
    available = {
        "cjdb": lambda: CjdbSystem(port=ports.get("cjdb", 55432)),
        "3dcitydb": lambda: CityDbSystem(port=ports.get("3dcitydb", 55433)),
        # The Hilbert package, which is what the format family's figures
        # call "CityParquet".
        "duckdb-cityparquet": lambda: DuckDBCityParquet(package="hilbert"),
        # The source-order package, for the ordering-sensitive scenarios
        # only (`registry.ORDERING_SCENARIOS`).
        "duckdb-cityparquet-source": lambda: DuckDBCityParquet(package="source"),
        # The write tier's second row: the same mutations, timed with the
        # package write-back included.
        "duckdb-cityparquet-writeback": lambda: DuckDBCityParquet(
            package="hilbert", writeback=True
        ),
        "cityparquet": lambda: ReadbenchSystem(binary=READBENCH_BIN),
        "cityparquet-hilbert": lambda: ReadbenchSystem(
            binary=READBENCH_BIN, hilbert=True
        ),
    }
    unknown = set(tags) - set(available)
    if unknown:
        raise SystemExit(f"unknown system tags: {sorted(unknown)}")
    return [available[tag]() for tag in tags]


def _apply_threads(systems: list, threads: int, workers: int) -> None:
    for system in systems:
        if hasattr(system, "set_threads"):
            system.set_threads(threads)
        elif hasattr(system, "set_parallel_workers"):
            system.set_parallel_workers(workers)


def _run_all_scenarios(systems: list, params, dataset_name: str, repeat: int,
                        sizes: dict[str, tuple[int, int]],
                        tolerance: float) -> list[dict[str, str]]:
    """Every read scenario under BOTH thread configurations, then the writes.

    Order is load-bearing, not incidental:

    - every read row of both configurations comes first, because CJDB's
      Q6-Q8 leave dead tuples on cjdb and rewritten pages on 3DCityDB that
      a later read pass would measure as if they were the steady state;
    - the write tier then runs ONCE, under the `single` configuration, and
      its rows say so. Measuring mutations under both conditions would mean
      restoring both databases between them, which is a longer and less
      informative experiment than the one the paper needs.

    `run_matrix` selects which systems answer each scenario from
    `registry.systems_for`, so the native Rust child is never handed a
    scenario name it does not implement (which it would raise `ValueError`
    for, indistinguishable from a real failure) and the two
    package-ordering tags appear only on the ordering-sensitive rows.
    """
    rows: list[dict[str, str]] = []
    for name, threads, workers in THREAD_CONFIGURATIONS:
        _apply_threads(systems, threads, workers)
        rows += run_matrix(
            systems, params, dataset_name, repeat=repeat,
            scenarios=READ_SCENARIOS, sizes=sizes, tolerance=tolerance,
            run_note=f"threads={name}",
        )

    primary = THREAD_CONFIGURATIONS[0]
    _apply_threads(systems, primary[1], primary[2])
    rows += run_matrix(
        systems, params, dataset_name, repeat=repeat, scenarios=TIER3,
        sizes=sizes, tolerance=tolerance, run_note=f"threads={primary[0]}",
    )
    return rows


def _format_ddl(statements: list[str]) -> str:
    """Render a list of DDL statements as one `;`-terminated block.

    Returns an empty string for an empty list rather than a dangling `;` —
    3DCityDB's `index_ddl()` legitimately returns `[]` (see its
    docstring), and that must render as nothing, not as a stray semicolon
    that misleadingly suggests a statement was dropped.
    """
    if not statements:
        return ""
    return ";\n".join(statements) + ";\n"


def _indexes_sql(systems: list) -> str:
    """The full ``results/<dataset>.indexes.sql`` artefact text.

    I7 (final whole-branch review): this used to write ONLY what each
    system's own ``index_ddl()`` function added on top of its defaults —
    for `3dcitydb` that is always an empty list (see
    `sql_citydb.index_ddl()`'s docstring: every index the queries need
    already exists), so the committed artefact carried effectively one
    real line (`cjdb`'s single added index) and could not support the
    index-parity audit this project's own spec promises ("the exact DDL
    each system ran"). Now also dumps `pg_indexes` LIVE for both `cjdb`'s
    and `3dcitydb`'s schemas, via each system's own still-open connection
    (called before `teardown()` in `cmd_bench`) — the FULL index set each
    system is actually running its scenario queries against, self-built
    defaults included, not just this harness's own additions.
    """
    sections = [
        "-- cjdb: the one index genuinely missing from cjdb's own defaults\n"
        "-- (added by this harness's ingest() -- see sql_cjdb.index_ddl()'s\n"
        "-- docstring for why every other index is already cjdb's own):\n"
        + _format_ddl(sql_cjdb.index_ddl()),
        "-- 3dcitydb: this harness adds nothing -- citydb-tool's own import\n"
        "-- already creates every index the scenario queries need (see\n"
        "-- sql_citydb.index_ddl()'s docstring). An empty list here is the\n"
        "-- correct, verified answer, not an omission.\n"
        + _format_ddl(sql_citydb.index_ddl()),
    ]

    by_tag = {system.tag: system for system in systems}
    for tag, label in (("cjdb", "cjdb"), ("3dcitydb", "3dcitydb")):
        system = by_tag.get(tag)
        header = f"\n-- {label}: FULL live `pg_indexes` dump (I7) --\n"
        conn = getattr(system, "_conn", None) if system is not None else None
        if conn is None:
            sections.append(
                f"{header}-- {label} was not part of this run; no live "
                "connection to dump from.\n"
            )
            continue
        schema = system._schema
        indexes = pg.dump_indexes(conn, schema)
        if not indexes:
            sections.append(
                f"{header}-- pg_indexes reports NO indexes for schema "
                f"{schema!r} -- unexpected; investigate before citing.\n"
            )
        else:
            sections.append(header + _format_ddl(indexes))

    return "".join(sections)


def cmd_prep(args) -> int:
    """Build pinned local database tools; data preparation remains suite-owned."""
    import subprocess
    if getattr(args, "data_root", None):
        root = Path(args.data_root).resolve()
        allowed = (BENCHMARK_DIR / "runs").resolve()
        if root != allowed and allowed not in root.parents:
            raise ValueError(f"data root must be below {allowed}")
        if getattr(args, "prepared_dir", None):
            Path(args.prepared_dir).mkdir(parents=True, exist_ok=True)
    subprocess.run(["just", "build-citydb"], cwd=ROOT, check=True)
    subprocess.run(["just", "patch-cjdb"], cwd=ROOT, check=True)
    return 0


def cmd_derive_params(args) -> int:
    source = Path(args.dataset)
    dataset = _dataset(
        source, Path(args.prepared_dir) if getattr(args, "prepared_dir", None) else None
    )
    p = params_mod.derive(source, dataset.cityparquet_dir)
    out = ROOT / "params" / f"{Dataset.name_from_path(source)}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(params_mod.to_json(p))
    print(f"wrote {out}")
    return 0


def cmd_bench(args) -> int:
    # The public runner creates fresh, UUID-scoped databases when a data root
    # is supplied. Recursive entry carries only discovered ports.
    if getattr(args, "data_root", None) and not getattr(args, "ports", None):
        with isolated_databases(Path(args.data_root), args.srid) as context:
            args.ports = context["ports"]
            os.environ["CITYBENCH_CJDB_CONTAINER"] = context["containers"]["cjdb"]
            os.environ["CITYBENCH_CITYDB_CONTAINER"] = context["containers"]["3dcitydb"]
            os.environ["CITYBENCH_CJDB_PORT"] = str(context["ports"]["cjdb"])
            os.environ["CITYBENCH_CITYDB_PORT"] = str(context["ports"]["3dcitydb"])
            return cmd_bench(args)
    source = Path(args.dataset)
    dataset = _dataset(source, Path(args.prepared_dir) if getattr(args, "prepared_dir", None) else None)
    tags = args.systems.split(",") if args.systems else [
        "duckdb-cityparquet", "duckdb-cityparquet-source",
        "duckdb-cityparquet-writeback", "cjdb", "3dcitydb",
    ]
    systems = _build_systems(tags, ports=getattr(args, "ports", None))

    # Never reuse name-keyed parameters: scaling inputs can be regenerated at
    # the same path. Derive from this run's source and persist beside output.
    #
    # The SOURCE-ORDER package supplies the package-derived parameters even
    # though `duckdb-cityparquet` reads the Hilbert one: the two hold the
    # same rows in a different order, so the windows and attribute picks are
    # identical either way, and naming one makes the derivation
    # deterministic regardless of which systems this run includes.
    p = params_mod.derive(source, dataset.cityparquet_dir)

    ingest_times: dict[str, float] = {}
    sizes: dict[str, tuple[int, int]] = {}
    for system in systems:
        system.prepare()
        result = system.ingest(dataset)
        ingest_times[system.tag] = result.wall_clock_s
        report = system.size()
        sizes[system.tag] = (
            report.size_bytes,
            report.size_bytes_no_index or report.size_bytes,
        )

    tolerance = getattr(args, "count_tolerance", DEFAULT_COUNT_TOLERANCE)
    rows = _run_all_scenarios(systems, p, dataset.name, args.repeat, sizes, tolerance)

    results_dir = Path(args.output_dir) if args.output_dir else BENCHMARK_DIR / "runs" / "databases" / "results"
    results_dir.mkdir(parents=True, exist_ok=True)
    (results_dir / f"{dataset.name}.params.json").write_text(params_mod.to_json(p))
    write_csv(results_dir / f"{dataset.name}.csv", rows)

    pg_settings = _pg_settings(getattr(args, "ports", None))

    (results_dir / f"{dataset.name}.manifest.json").write_text(
        json.dumps(
            manifest.collect(
                dataset_name=dataset.name,
                source=__import__("hashlib").sha256(source.read_bytes()).hexdigest(),
                ingest=ingest_times,
                sizes=sizes,
                versions=_versions(systems),
                pg_settings=pg_settings,
                patches=_patches(systems),
                srid=_srids(systems),
                execution=_execution(systems),
                count_check={
                    "relative_spread_tolerance": tolerance,
                    "statuses": {
                        "ok": "every answering system returned the same count",
                        "ok-deviation": (
                            "the counts differ, but the relative spread "
                            "(max - min) / max is within the tolerance; the "
                            "`count-mismatch: ...` detail stays in `notes` "
                            "and the run does not fail"
                        ),
                        "mismatch": (
                            "the spread exceeds the tolerance; the run exits "
                            "non-zero and the row is not citable"
                        ),
                    },
                },
            ),
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )

    (results_dir / f"{dataset.name}.indexes.sql").write_text(
        _indexes_sql(systems)
    )

    for system in systems:
        system.teardown()

    deviations = [r for r in rows if r["status"] == "ok-deviation"]
    if deviations:
        print(
            f"NOTE: {len(deviations)} row(s) carry an explained count "
            f"deviation within the {tolerance:.4%} tolerance; see `notes`",
            file=sys.stderr,
        )
    mismatches = [r for r in rows if r["status"] == "mismatch"]
    if mismatches:
        print(
            f"WARNING: {len(mismatches)} row(s) carry a count mismatch "
            "beyond the tolerance",
            file=sys.stderr,
        )
    print(f"wrote {results_dir / f'{dataset.name}.csv'} ({len(rows)} rows)")
    if any(row["status"] in {"error", "mismatch"} for row in rows):
        return 1
    return 0


def cmd_smoke(args) -> int:
    """Full pipeline on the small fixture; fails on any count mismatch."""
    ns = argparse.Namespace(
        dataset=args.dataset or str(ROOT / "data" / "delft.city.jsonl"),
        repeat=2,
        systems=None,
        output_dir=getattr(args, "output_dir", None),
        prepared_dir=getattr(args, "prepared_dir", None),
        ports=None,
        data_root=getattr(args, "data_root", None),
        srid=getattr(args, "srid", 7415),
        count_tolerance=getattr(args, "count_tolerance", DEFAULT_COUNT_TOLERANCE),
    )
    bench_status = cmd_bench(ns)
    output_dir = Path(ns.output_dir) if ns.output_dir else BENCHMARK_DIR / "runs" / "databases" / "results"
    csv_path = output_dir / f"{Dataset.name_from_path(ns.dataset)}.csv"
    if bench_status:
        return bench_status
    # The STATUS column, not the `notes` text: an `ok-deviation` row keeps
    # its full `count-mismatch: ...` decomposition in `notes` on purpose,
    # so a substring search there would now fail every run that has one.
    with csv_path.open(newline="") as handle:
        rows = list(csv.DictReader(handle))
    if any(row["status"] == "mismatch" for row in rows):
        print(
            "SMOKE FAILED: systems disagree on a result count by more than "
            "the tolerance. At least one is answering a different question; "
            "its timing is meaningless until reconciled.",
            file=sys.stderr,
        )
        return 1
    if any(row["status"] == "error" for row in rows):
        print("SMOKE FAILED: a system errored; see notes column.", file=sys.stderr)
        return 1
    deviations = sum(1 for row in rows if row["status"] == "ok-deviation")
    print(f"smoke OK ({len(rows)} rows, {deviations} explained deviation(s))")
    return 0


def _execution(systems: list) -> dict:
    """The two thread configurations, and what each session resolved to.

    The PostgreSQL block is read back from the benchmark session itself
    (`pg.parallel_settings`), because `max_worker_processes` bounds how many
    workers a query can actually get regardless of what
    `max_parallel_workers_per_gather` asks for — recorded rather than
    assumed.
    """
    resolved = {}
    for system in systems:
        if hasattr(system, "session_settings"):
            try:
                resolved[system.tag] = system.session_settings()
            except Exception as exc:               # a closed session is a result
                resolved[system.tag] = {"error": str(exc)}
    return {
        "configurations": [
            {"name": name, "duckdb_threads": threads,
             "postgresql_max_parallel_workers_per_gather": workers,
             "primary": index == 0}
            for index, (name, threads, workers) in enumerate(THREAD_CONFIGURATIONS)
        ],
        "read_tier": "every read scenario is measured under BOTH configurations; CSV `notes` carry threads=<name>",
        "write_tier": (
            f"measured once, under the {THREAD_CONFIGURATIONS[0][0]} "
            "configuration, after every read row"
        ),
        "postgresql_session_resolved": resolved,
        "postgresql_planner_thresholds": (
            "parallel_setup_cost and min_parallel_table_scan_size are left at "
            "their defaults under both configurations"
        ),
    }


def _versions(systems: list) -> dict[str, str]:
    import duckdb

    versions = {"duckdb": duckdb.__version__}
    # The write tier runs through the DuckDB CityJSON extension's package
    # model, so which build of that extension answered is part of the
    # result, not an environment detail.
    try:
        conn = duckdb.connect()
        conn.execute("LOAD cityjson")
        row = conn.execute(
            "SELECT extension_version, install_mode FROM duckdb_extensions() "
            "WHERE extension_name = 'cityjson'"
        ).fetchone()
        conn.close()
        if row is not None:
            versions["duckdb-cityjson"] = f"{row[0]} ({row[1]})"
    except Exception as exc:
        versions["duckdb-cityjson"] = f"unavailable: {exc}"
    if any(s.tag == "cjdb" for s in systems):
        from citybench.systems.cjdb import CJDB_UPSTREAM_VERSION

        # Terse, at-a-glance marker; `_patches()` below carries the full
        # "what changed and why" — see manifest.py's own module docstring
        # for why the two are deliberately not merged into one.
        versions["cjdb"] = f"{CJDB_UPSTREAM_VERSION}+ground-surfaces-tie-patch"
    return versions


def _patches(systems: list) -> dict[str, dict[str, str]]:
    """Which systems in this run were patched from stock, and how.

    A reader of the manifest must never be able to mistake a patched
    system's numbers for stock upstream's — see
    `citybench.systems.cjdb.patch_disclosure` and
    `vendor/cjdb/README.md`.
    """
    patches: dict[str, dict[str, str]] = {}
    if any(s.tag == "cjdb" for s in systems):
        from citybench.systems.cjdb import patch_disclosure

        patches["cjdb"] = patch_disclosure()
    return patches


def _srids(systems: list) -> dict[str, int]:
    """The SRID each PostgreSQL-backed system actually landed on.

    Only `cjdb`/`3dcitydb` carry a `_srid` attribute (set from a live
    `SELECT ... FROM cj_metadata`/`database_srs` read-back inside
    `ingest()`, not merely echoing what was requested) — the two
    CityParquet-reading systems and duckdb-cityparquet have no SRID
    concept at all, so they are simply absent from this dict rather than
    stamped with a meaningless placeholder.
    """
    return {
        system.tag: system._srid
        for system in systems
        if hasattr(system, "_srid")
    }


def _pg_settings(ports: dict[str, int] | None = None) -> dict[str, str]:
    """Human-readable values for the manifest's ``pg_settings`` block.

    M1 (final whole-branch review): this used to concatenate
    ``pg_settings.setting`` (the raw stored integer) directly with
    ``pg_settings.unit`` (the GUC's OWN internal unit string, e.g.
    ``"8kB"`` for ``shared_buffers`` -- meaning "multiply the raw integer
    by 8kB to get the real value", not "append the literal text after the
    number"). For a ``GUC_UNIT_BLOCKS`` setting like ``shared_buffers``
    (raw ``1048576``, unit ``"8kB"``) that produced the string
    ``"1048576" + "8kB" = "10485768kB"`` -- which READS as roughly 10.5GB
    but the true configured value is ``1048576 * 8kB = 8388608kB = 8GB``
    exactly. ``current_setting(name)`` asks PostgreSQL itself for the same
    pretty-printed value ``SHOW`` would report (``"8GB"``, ``"256MB"``,
    ``"16"``, ...) -- correct by construction, for every GUC unit kind, not
    just the block-unit ones this bug happened to be caught on.
    """
    from citybench.systems import pg

    settings = {}
    ports = ports or {"cjdb": 55432, "3dcitydb": 55433}
    for tag in ("cjdb", "3dcitydb"):
        port = ports[tag]
        try:
            conn = pg.connect(port)
            with conn.cursor() as cur:
                cur.execute(
                    "SELECT name, current_setting(name) FROM pg_settings "
                    "WHERE name = ANY(%s)",
                    (["shared_buffers", "work_mem", "effective_cache_size",
                      "random_page_cost", "max_parallel_workers",
                      # I4 (final whole-branch review): this is the setting
                      # that actually binds PER QUERY (leader + this many
                      # workers) -- max_parallel_workers above is only the
                      # cluster-wide pool it draws from. Omitting it let a
                      # published manifest look "tuned identically" while
                      # the per-query CPU budget silently differed from
                      # DuckDB's own (see README "Tuning parity").
                      "max_parallel_workers_per_gather"],),
                )
                settings[tag] = dict(cur.fetchall())
            conn.close()
        except Exception as exc:
            settings[tag] = {"error": str(exc)}
    return settings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="citybench")
    sub = parser.add_subparsers(dest="command", required=True)

    p_prep = sub.add_parser("prep")
    p_prep.add_argument("--data-root", default=None)
    p_prep.add_argument("--prepared-dir", default=None)
    p_prep.add_argument("--srid", type=int, default=None)
    p_prep.set_defaults(func=cmd_prep)

    p_derive = sub.add_parser("derive-params")
    p_derive.add_argument("--dataset", required=True)
    p_derive.add_argument("--prepared-dir", default=None,
                          help="directory holding <dataset>.parquet; the "
                               "windows and attribute picks are derived from "
                               "that package, as the format harness derives "
                               "its own")
    p_derive.set_defaults(func=cmd_derive_params)

    p_bench = sub.add_parser("run")
    p_bench.add_argument("--dataset", required=True)
    p_bench.add_argument("--repeat", type=int, default=7)
    p_bench.add_argument("--systems", default=None,
                         help="comma-separated tags; default is DuckDB over CityParquet, cjdb, and 3DCityDB")
    p_bench.add_argument("--prepared-dir", default=None)
    p_bench.add_argument("--data-root", default=None)
    p_bench.add_argument("--srid", type=int, default=7415)
    p_bench.add_argument("--count-tolerance", type=float,
                         default=DEFAULT_COUNT_TOLERANCE,
                         help="relative spread (max-min)/max below which a "
                              "cross-system count disagreement is published "
                              "as status=ok-deviation with its decomposition "
                              "in `notes`, instead of failing the run "
                              f"(default {DEFAULT_COUNT_TOLERANCE})")
    p_bench.add_argument("--output-dir", default=None,
                         help="directory for CSV, manifest, and index artefacts")
    p_bench.set_defaults(func=cmd_bench)

    p_smoke = sub.add_parser("smoke")
    p_smoke.add_argument("--dataset", default=None,
                         help="dataset for a smoke run; defaults to the tiny fixture")
    p_smoke.add_argument("--output-dir", default=None)
    p_smoke.add_argument("--prepared-dir", default=None)
    p_smoke.add_argument("--data-root", default=None)
    p_smoke.add_argument("--srid", type=int, default=7415)
    p_smoke.add_argument("--count-tolerance", type=float,
                         default=DEFAULT_COUNT_TOLERANCE)
    p_smoke.set_defaults(func=cmd_smoke)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
