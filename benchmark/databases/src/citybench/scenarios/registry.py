"""Which scenarios exist, and which systems run each one."""

from __future__ import annotations

TIER1: tuple[str, ...] = (
    "full-read",
    "count",
    "bbox-query",
    "attr-filter",
    "attr-stats",
    "id-lookup",
    "project",
)

TIER2: tuple[str, ...] = (
    "lod-extract",
    "semantic-surface",
    "hierarchy",
)

ALL: tuple[str, ...] = TIER1 + TIER2

ALL_SYSTEMS: tuple[str, ...] = (
    "cityparquet",
    "cityparquet-hilbert",
    "duckdb-cityparquet",
    "cjdb",
    "3dcitydb",
)

SQL_SYSTEMS: tuple[str, ...] = ("duckdb-cityparquet", "cjdb", "3dcitydb")

# Scenarios measured at three window sizes rather than once.
SELECTIVITY_SCENARIOS: frozenset[str] = frozenset({"bbox-query"})

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
    "full-read", "count", "bbox-query", "attr-filter", "attr-stats",
    "project", "lod-extract", "semantic-surface", "hierarchy",
})

# id-lookup materialises the matching row itself, so its result count is
# the row count — 1 on a hit, 0 on a miss.
COUNT_FROM_ROWCOUNT: frozenset[str] = frozenset({"id-lookup"})


class ScenarioUnavailable(Exception):
    """Raised when a scenario cannot run against this dataset.

    Distinct from a failure: the dataset simply lacks what the scenario
    needs (for example, no parent/child hierarchy exists). The runner
    records this as a row with an explanatory note rather than dropping
    the dataset or the scenario silently.
    """


def count_mode(scenario: str) -> str:
    """Either 'first-column' or 'rowcount'. Raises KeyError if unknown."""
    if scenario in COUNT_FROM_FIRST_COLUMN:
        return "first-column"
    if scenario in COUNT_FROM_ROWCOUNT:
        return "rowcount"
    raise KeyError(f"unknown scenario: {scenario}")


def systems_for(scenario: str) -> tuple[str, ...]:
    """The system tags that run ``scenario``. Raises KeyError if unknown."""
    if scenario in TIER1:
        return ALL_SYSTEMS
    if scenario in TIER2:
        return SQL_SYSTEMS
    raise KeyError(f"unknown scenario: {scenario}")
