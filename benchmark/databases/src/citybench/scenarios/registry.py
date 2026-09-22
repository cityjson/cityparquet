"""Which scenarios exist, and which systems run each one."""

from __future__ import annotations

#: Read scenarios, in the order they are measured and reported. The set is
#: the read half of the author's query catalogue
#: (`notes/benchmark-queries.md`); five of the ten rows map onto a query of
#: the CJDB paper's own Q1-Q8 (see `README.md`, "Mapping to the CJDB
#: paper"), the rest are this harness's own and are captioned as such.
#:
#: `bbox-fetch`, `point-query`, `semantic-surface` and `project` are NOT
#: here: the catalogue drops containment fetches and point queries ("a
#: point query is a window query" — they measure how the query is composed,
#: not the format), single-attribute projection, and semantic-surface
#: presence.
TIER1: tuple[str, ...] = (
    "geometry-scan",
    "count",
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

#: The write tier, reported in its own table. These are NOT points on one
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

# Scenarios measured at three window sizes rather than once.
SELECTIVITY_SCENARIOS: frozenset[str] = frozenset({"bbox-query"})

# Scenarios measured at four id probes rather than once — the format
# family's own construction (`benchmark/readbench/src/params.rs`,
# `ID_DECILES`/`ID_MISS_TAG`): the ids at 10 %, 50 % and 90 % of the
# canonical CityJSONSeq stream order, plus one verified-absent id. A single
# target would make the published time a function of where that one id
# happened to sit in the stream.
ID_PROBE_SCENARIOS: frozenset[str] = frozenset({"id-lookup"})

# The scenarios whose answer depends on the package's ROW ORDER, and so the
# only ones the source-order package is published alongside the Hilbert one
# for. `bbox-query` is now the only one — `bbox-fetch` and `point-query`
# left with the catalogue review — which leaves
# `duckdb-cityparquet-source` a one-scenario system. It is KEPT at that
# size: it is the ordering-dependence control, and without it the two
# benchmark families' "CityParquet" would again be different artefacts
# under one name (`notes/benchmark-fairness-review-2026-09-22.md` §4.5).
ORDERING_SCENARIOS: frozenset[str] = frozenset({"bbox-query"})

# What `cityparquet-readbench --child` implements. `geometry-scan` and
# `attr-range` have no counterpart in the Rust child's own `Scenario` enum
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
# (geometry-scan forces a decode via a byte sum; attr-stats returns
# min/max/sum/count) must NOT have their aggregate compared as if it were a
# count — two engines compute different sums and every row would be falsely
# flagged.
#
# Convention, enforced by every sql_* module: a scenario in
# COUNT_FROM_FIRST_COLUMN returns a single row whose FIRST column is the
# object count, with any forcing or aggregate work in later columns.
# Everything else reports the number of rows materialised.
COUNT_FROM_FIRST_COLUMN: frozenset[str] = frozenset({
    "geometry-scan", "count", "bbox-query", "attr-stats",
})

# Scenarios that RETURN ROWS — ids, or whole objects — the way the CJDB
# paper's own queries do, and whose result count is therefore the number of
# rows materialised. Every one of these fetches its rows inside the timed
# window on every system, so no engine wins by handing back a lazy cursor.
COUNT_FROM_ROWCOUNT: frozenset[str] = frozenset({
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


class ScenarioUnavailable(Exception):
    """Raised when a scenario cannot run against this dataset.

    Distinct from a failure: the dataset simply lacks what the scenario
    needs (for example, no numeric attribute at all, or a plain CityJSON
    source from which no one-feature append file can be cut). The runner
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
