"""Which scenarios exist, and which systems run each one."""

from __future__ import annotations

#: Read scenarios, in the order they are measured and reported. The set is
#: the read half of the author's query catalogue
#: (`notes/benchmark-queries.md`); four of the nine rows map onto a query of
#: the CJDB paper's own Q1-Q8 (see `README.md`, "Mapping to the CJDB
#: paper"), the rest are this harness's own and are captioned as such.
#:
#: `bbox-fetch`, `point-query`, `semantic-surface` and `project` are NOT
#: here: the catalogue drops containment fetches and point queries ("a
#: point query is a window query" — they measure how the query is composed,
#: not the format), single-attribute projection, and semantic-surface
#: presence. Nor is `count`: every system answers it from metadata, so it
#: measures nothing about the store.
TIER1: tuple[str, ...] = (
    "geometry-scan",
    "bbox-query",
    "attr-filter",
    "attr-range",
    "attr-stats",
    "id-lookup",
)

TIER2: tuple[str, ...] = (
    "lod-query",
    "parts-per-building",
    "parts-per-building-join",
)

#: The write tier, reported as rows below the reads under Caveat 19. These are NOT points on one
#: scale with the read scenarios or with each other: PostgreSQL rewrites
#: half a million MVCC tuples in place while CityParquet adds or drops a
#: column of an in-memory table and, on the write-back rows, re-encodes the
#: package. They run LAST, after every read row of every thread
#: configuration, because they leave dead tuples and rewritten pages behind
#: that would otherwise become part of a later read measurement.
#:
#: Order matters and is enforced here: `attr-add` creates the attribute
#: `attr-update` increments and `attr-delete` removes. `append-object`
#: (catalogue B18) comes last of all, because it is the only row that adds
#: objects rather than attributes and each system runs its own IMPORTER
#: rather than a statement — the most disruptive thing the tier does to the
#: state every earlier row was measured against.
TIER3: tuple[str, ...] = (
    "attr-add",
    "attr-update",
    "attr-delete",
    "append-object",
)

ALL: tuple[str, ...] = TIER1 + TIER2 + TIER3

READ_SCENARIOS: tuple[str, ...] = TIER1 + TIER2

SQL_SYSTEMS: tuple[str, ...] = ("duckdb-cityparquet", "cjdb", "3dcitydb")

# Scenarios measured at three window sizes rather than once.
SELECTIVITY_SCENARIOS: frozenset[str] = frozenset({"bbox-query"})

# Scenarios measured at two id probes rather than once: the id at the middle
# of the canonical CityJSONSeq stream order, plus one verified-absent id
# (`citybench.config.ID_HIT_POSITION`/`ID_MISS_TAG`). Every database indexes
# the identifier with a B-tree, so the hit's position does not change its
# cost; the miss is where Parquet's Bloom filter answers against a B-tree.
ID_PROBE_SCENARIOS: frozenset[str] = frozenset({"id-lookup"})

# What `cityparquet-readbench --child` implements. `geometry-scan` and
# `attr-range` have no counterpart in the Rust child's own `Scenario` enum
# (`benchmark/readbench/src/scenario.rs`), and the read harness is not this
# family's to extend, so the native-reader systems simply do not run them.
READBENCH_SCENARIOS: frozenset[str] = frozenset({
    "bbox-query", "attr-filter", "attr-stats", "id-lookup",
})


# A CONTROL, not a comparison: the same question as `parts-per-building`
# asked the way a normalised store must ask it, run on DuckDB alone so the
# cost of the join is visible against the natural form on the same engine
# and the same data. cjdb and 3DCityDB have only the join form, which is
# already `parts-per-building` for them.
DUCKDB_ONLY_SCENARIOS: frozenset[str] = frozenset({"parts-per-building-join"})


def systems_for(scenario: str) -> tuple[str, ...]:
    """The system tags that run ``scenario``. Raises KeyError if unknown."""
    if scenario in DUCKDB_ONLY_SCENARIOS:
        return ("duckdb-cityparquet",)
    if scenario in TIER3:
        return ("duckdb-cityparquet", "duckdb-cityparquet-writeback", "cjdb", "3dcitydb")
    if scenario not in READ_SCENARIOS:
        raise KeyError(f"unknown scenario: {scenario}")
    tags = list(SQL_SYSTEMS)
    if scenario in READBENCH_SCENARIOS:
        tags = ["cityparquet"] + tags
    return tuple(tags)

# How each scenario's `result_count` is extracted.
#
# This matters more than it looks. The cross-system count check compares
# result_count across systems, so the extraction rule must yield the same
# LOGICAL quantity everywhere. Scenarios whose SQL returns an aggregate
# (geometry-scan forces a decode via a byte sum; attr-stats returns
# min/max/sum/count) must NOT have their aggregate compared as if it were a
# count — two engines compute different sums and every row would be falsely
# flagged.
#
# Convention, enforced by every sql_* module: `attr-stats`
# (COUNT_FROM_LAST_COLUMN) returns min, max, sum, count, so its count is the
# LAST column. Every other read reports the number of rows materialised.
COUNT_FROM_LAST_COLUMN: frozenset[str] = frozenset({"attr-stats"})

# Scenarios that RETURN ROWS — ids, or whole objects — the way the CJDB
# paper's own queries do, and whose result count is therefore the number of
# rows materialised. Every one of these fetches its rows inside the timed
# window on every system, so no engine wins by handing back a lazy cursor.
COUNT_FROM_ROWCOUNT: frozenset[str] = frozenset({
    "geometry-scan", "bbox-query",
    "id-lookup", "attr-filter", "attr-range",
    "lod-query", "parts-per-building", "parts-per-building-join",
})

# The write tier reports the number of rows the statement TOUCHED, which is
# the cursor's own rowcount rather than anything in a result set — except
# `append-object`, whose count is the number of CityObjects the appended
# one-feature file carries (the Building plus its parts), a DEFINITION
# shared by all four tags because each system's importer writes a different
# number of its own rows for them.
COUNT_FROM_WRITE_ROWCOUNT: frozenset[str] = TIER3


#: The LoD `lod-query` asks for on every system.
LOD_QUERY_TARGET = "2.2"


class ScenarioUnavailable(Exception):
    """Raised when a scenario cannot run against this dataset.

    Distinct from a failure: the dataset simply lacks what the scenario
    needs (for example, no numeric attribute at all, or a plain CityJSON
    source from which no one-feature append file can be cut). The runner
    records this as a row with an explanatory note rather than dropping
    the dataset or the scenario silently.
    """


def count_mode(scenario: str) -> str:
    """'last-column', 'rowcount' or 'write-rowcount'. KeyError if unknown."""
    if scenario in COUNT_FROM_LAST_COLUMN:
        return "last-column"
    if scenario in COUNT_FROM_ROWCOUNT:
        return "rowcount"
    if scenario in COUNT_FROM_WRITE_ROWCOUNT:
        return "write-rowcount"
    raise KeyError(f"unknown scenario: {scenario}")


def require_lod_query_target(params) -> None:
    """`lod-query` on a dataset without LoD 2.2 is not applicable, never 0."""
    if LOD_QUERY_TARGET not in params.lods:
        raise ScenarioUnavailable(
            f"dataset carries no LoD {LOD_QUERY_TARGET} geometry "
            f"(LoDs: {', '.join(params.lods) or 'none'})"
        )
