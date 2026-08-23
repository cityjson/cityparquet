"""Command-line entry point behind the justfile."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from citybench import manifest, params as params_mod
from citybench.config import Dataset
from citybench.report import write_csv
from citybench.runner import run_matrix
from citybench.scenarios import sql_citydb, sql_cjdb
from citybench.scenarios.registry import SQL_SYSTEMS, TIER1, TIER2
from citybench.systems import pg
from citybench.systems.cjdb import CjdbSystem
from citybench.systems.citydb import CityDbSystem
from citybench.systems.duckdb_cp import DuckDBCityParquet
from citybench.systems.readbench import ReadbenchSystem

ROOT = Path(__file__).resolve().parents[2]
# ROOT is benchmark/databases/; the read harness this shells out to is a crate
# in the other half of the monorepo, built by
# `cargo build --release -p cityparquet-readbench` in lib/cityparquet-rs.
MONO_ROOT = ROOT.parents[1]
READBENCH_BIN = (
    MONO_ROOT / "lib" / "cityparquet-rs" / "target" / "release" / "cityparquet-readbench"
)


def _dataset(source: Path) -> Dataset:
    name = Dataset.name_from_path(source)
    return Dataset(
        name=name,
        source=source,
        cityparquet_dir=ROOT / "data" / "cityparquet" / name,
        hilbert_dir=ROOT / "data" / "cityparquet-hilbert" / name,
    )


def _build_systems(tags: list[str]) -> list:
    available = {
        "cjdb": lambda: CjdbSystem(),
        "3dcitydb": lambda: CityDbSystem(),
        "duckdb-cityparquet": lambda: DuckDBCityParquet(),
        "cityparquet": lambda: ReadbenchSystem(binary=READBENCH_BIN),
        "cityparquet-hilbert": lambda: ReadbenchSystem(
            binary=READBENCH_BIN, hilbert=True
        ),
    }
    unknown = set(tags) - set(available)
    if unknown:
        raise SystemExit(f"unknown system tags: {sorted(unknown)}")
    return [available[tag]() for tag in tags]


def _run_all_scenarios(systems: list, params, dataset_name: str, repeat: int,
                        sizes: dict[str, tuple[int, int]]) -> list[dict[str, str]]:
    """Run TIER1 on every requested system, TIER2 on the SQL-only subset.

    A single `run_matrix(systems, ..., scenarios=ALL)` call would hand the
    Rust child (`cityparquet`/`cityparquet-hilbert`) a TIER2 scenario name
    it does not implement — `ReadbenchSystem.run` raises `ValueError` for
    those (see `systems/readbench.py`'s `build_child_args`), which
    `run_matrix` cannot tell apart from a genuine failure and records as
    `error: ValueError`. Since `just smoke` fails on ANY `error:` note,
    that single-call shape would make the smoke target fail forever, not
    just on a real regression. `registry.systems_for` already documents
    the intended split (TIER1 -> every system, TIER2 -> `SQL_SYSTEMS`
    only); this is that split, applied at the one call site that drives
    the whole matrix.
    """
    tier1_rows = run_matrix(
        systems, params, dataset_name, repeat=repeat, scenarios=TIER1, sizes=sizes,
    )
    sql_systems = [s for s in systems if s.tag in SQL_SYSTEMS]
    tier2_rows = run_matrix(
        sql_systems, params, dataset_name, repeat=repeat, scenarios=TIER2, sizes=sizes,
    )
    return tier1_rows + tier2_rows


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


def cmd_derive_params(args) -> int:
    source = Path(args.dataset)
    p = params_mod.derive(source)
    out = ROOT / "params" / f"{Dataset.name_from_path(source)}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(params_mod.to_json(p))
    print(f"wrote {out}")
    return 0


def cmd_bench(args) -> int:
    source = Path(args.dataset)
    dataset = _dataset(source)
    tags = args.systems.split(",") if args.systems else [
        "cityparquet", "cityparquet-hilbert", "duckdb-cityparquet",
        "cjdb", "3dcitydb",
    ]
    systems = _build_systems(tags)

    params_file = ROOT / "params" / f"{dataset.name}.json"
    if params_file.exists():
        p = params_mod.from_json(params_file.read_text())
    else:
        p = params_mod.derive(source)
        params_file.parent.mkdir(parents=True, exist_ok=True)
        params_file.write_text(params_mod.to_json(p))

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

    rows = _run_all_scenarios(systems, p, dataset.name, args.repeat, sizes)

    results_dir = ROOT / "results"
    write_csv(results_dir / f"{dataset.name}.csv", rows)

    (results_dir / f"{dataset.name}.manifest.json").write_text(
        json.dumps(
            manifest.collect(
                dataset_name=dataset.name,
                ingest=ingest_times,
                sizes=sizes,
                versions=_versions(systems),
                pg_settings=_pg_settings(),
                patches=_patches(systems),
                srid=_srids(systems),
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

    mismatches = [r for r in rows if "count-mismatch" in r["notes"]]
    if mismatches:
        print(
            f"WARNING: {len(mismatches)} row(s) carry a count mismatch",
            file=sys.stderr,
        )
    print(f"wrote {results_dir / f'{dataset.name}.csv'} ({len(rows)} rows)")
    return 0


def cmd_smoke(args) -> int:
    """Full pipeline on the small fixture; fails on any count mismatch."""
    ns = argparse.Namespace(
        dataset=str(ROOT / "data" / "delft.city.jsonl"),
        repeat=2,
        systems=None,
    )
    cmd_bench(ns)
    csv_path = ROOT / "results" / "delft.csv"
    text = csv_path.read_text()
    if "count-mismatch" in text:
        print(
            "SMOKE FAILED: systems disagree on a result count. "
            "At least one is answering a different question; its timing "
            "is meaningless until reconciled.",
            file=sys.stderr,
        )
        return 1
    if "error:" in text:
        print("SMOKE FAILED: a system errored; see notes column.", file=sys.stderr)
        return 1
    print("smoke OK")
    return 0


def _versions(systems: list) -> dict[str, str]:
    import duckdb

    versions = {"duckdb": duckdb.__version__}
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


def _pg_settings() -> dict[str, str]:
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
    for port, tag in ((55432, "cjdb"), (55433, "3dcitydb")):
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

    p_derive = sub.add_parser("derive-params")
    p_derive.add_argument("--dataset", required=True)
    p_derive.set_defaults(func=cmd_derive_params)

    p_bench = sub.add_parser("bench")
    p_bench.add_argument("--dataset", required=True)
    p_bench.add_argument("--repeat", type=int, default=7)
    p_bench.add_argument("--systems", default=None,
                         help="comma-separated tags; default is all five")
    p_bench.set_defaults(func=cmd_bench)

    p_smoke = sub.add_parser("smoke")
    p_smoke.set_defaults(func=cmd_smoke)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
