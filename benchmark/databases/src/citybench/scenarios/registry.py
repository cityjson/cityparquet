"""Which scenarios exist, and which systems run each one."""

from __future__ import annotations

#: Read scenarios, in the order they are measured and reported. Eight of the
#: thirteen map onto a query of the CJDB paper's own Q1-Q8 (see
#: `README.md`, "Mapping to the CJDB paper"); the rest are this harness's
#: own and are captioned as such.
TIER1: tuple[str, ...] = (
    "geometry-scan",
    "count",
    "bbox-query",
    "bbox-fetch",
    "point-query",
    "attr-filter",
    "attr-range",
    "attr-stats",
    "id-lookup",
)

TIER2: tuple[str, ...] = (
    "lod-extract",
    "semantic-surface",
    "parts-per-building",
    "parts-per-building-join",
)

#: The write tier, reported in its own table. These are NOT points on one
#: scale with the read scenarios or with each other: PostgreSQL rewrites
#: half a million MVCC tuples in place while CityParquet adds or drops a
#: column of an in-memory table and, on the write-back rows, re-encodes the
#: package. They run LAST, after every read row of every thread
#: configuration, because they leave dead tuples and rewritten pages behind
#: that would otherwise become part of a later read measurement.
#:
#: Order matters and is enforced here: `attr-add` creates the attribute
#: `attr-update` increments and `attr-delete` removes.
TIER3: tuple[str, ...] = (
    "attr-add",
    "attr-update",
    "attr-delete",
)

ALL: tuple[str, ...] = TIER1 + TIER2 + TIER3

READ_SCENARIOS: tuple[str, ...] = TIER1 + TIER2

ALL_SYSTEMS: tuple[str, ...] = (
    "cityparquet",
    "cityparquet-hilbert",
    "duckdb-cityparquet",
    "duckdb-cityparquet-source",
    "duckdb-cityparquet-writeback",
    "cjdb",
    "3dcitydb",
)

SQL_SYSTEMS: tuple[str, ...] = ("duckdb-cityparquet", "cjdb", "3dcitydb")

# Scenarios measured at three window sizes rather than once. `point-query`
# is NOT among them: its window is degenerate (a single point), so it has
# one row, not three.
SELECTIVITY_SCENARIOS: frozenset[str] = frozenset({"bbox-query", "bbox-fetch"})

# The scenarios whose answer depends on the package's ROW ORDER, and so the
# only ones the source-order package is published alongside the Hilbert one
# for. Publishing the ordering pair on every scenario would add rows that
# differ only by noise; publishing it on none would hide that the two
# benchmark families' "CityParquet" were different artefacts under one name
# (`notes/benchmark-fairness-review-2026-09-22.md` §4.5).
ORDERING_SCENARIOS: frozenset[str] = frozenset(
    {"bbox-query", "bbox-fetch", "point-query"}
)

# What `cityparquet-readbench --child` implements. `geometry-scan`,
# `bbox-fetch`, `point-query` and `attr-range` have no counterpart in the
# Rust child's own `Scenario` enum
# (`benchmark/readbench/src/scenario.rs`), and the read harness is not this
# family's to extend, so the native-reader systems simply do not run them.
READBENCH_SCENARIOS: frozenset[str] = frozenset({
    "count", "bbox-query", "attr-filter", "attr-stats", "id-lookup",
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
    if scenario in ORDERING_SCENARIOS:
        tags.insert(1, "duckdb-cityparquet-source")
    if scenario in READBENCH_SCENARIOS:
        tags = ["cityparquet", "cityparquet-hilbert"] + tags
    return tuple(tags)

# How each scenario's `result_count` is extracted.
#
# This matters more than it looks. The cross-system count check compares
# result_count across systems, so the extraction rule must yield the same
# LOGICAL quantity everywhere. Scenarios whose SQL returns an aggregate
# (full-read forces a decode via a checksum; attr-stats returns min/max/
# sum/count) must NOT have their checksum compared as if it were a count —
# two engines hash differently and every row would be falsely flagged.
#
# Convention, enforced by every sql_* module: a scenario in
# COUNT_FROM_FIRST_COLUMN returns a single row whose FIRST column is the
# object count, with any forcing or aggregate work in later columns.
# Everything else reports the number of rows materialised.
COUNT_FROM_FIRST_COLUMN: frozenset[str] = frozenset({
    "geometry-scan", "count", "bbox-query", "attr-stats", "semantic-surface",
})

# Scenarios that RETURN ROWS — ids, or ids plus a footprint — the way the
# CJDB paper's own queries do, and whose result count is therefore the
# number of rows materialised. Every one of these fetches its rows inside
# the timed window on every system, so no engine wins by handing back a
# lazy cursor.
COUNT_FROM_ROWCOUNT: frozenset[str] = frozenset({
    "id-lookup", "bbox-fetch", "point-query", "attr-filter", "attr-range",
    "lod-extract", "parts-per-building", "parts-per-building-join",
})

# The write tier reports the number of rows the statement TOUCHED, which is
# the cursor's own rowcount rather than anything in a result set.
COUNT_FROM_WRITE_ROWCOUNT: frozenset[str] = TIER3


class ScenarioUnavailable(Exception):
    """Raised when a scenario cannot run against this dataset.

    Distinct from a failure: the dataset simply lacks what the scenario
    needs (for example, no parent/child hierarchy exists). The runner
    records this as a row with an explanatory note rather than dropping
    the dataset or the scenario silently.
    """


def count_mode(scenario: str) -> str:
    """'first-column', 'rowcount' or 'write-rowcount'. KeyError if unknown."""
    if scenario in COUNT_FROM_FIRST_COLUMN:
        return "first-column"
    if scenario in COUNT_FROM_ROWCOUNT:
        return "rowcount"
    if scenario in COUNT_FROM_WRITE_ROWCOUNT:
        return "write-rowcount"
    raise KeyError(f"unknown scenario: {scenario}")
