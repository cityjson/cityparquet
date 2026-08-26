import pytest

from citybench.scenarios.registry import (
    ALL,
    COUNT_FROM_FIRST_COLUMN,
    COUNT_FROM_ROWCOUNT,
    TIER1,
    TIER2,
    count_mode,
    systems_for,
)


def test_ten_scenarios_in_total():
    assert len(ALL) == 10
    assert set(ALL) == set(TIER1) | set(TIER2)


def test_tier1_is_the_seven_inherited_scenarios():
    assert TIER1 == (
        "full-read", "count", "bbox-query", "attr-filter",
        "attr-stats", "id-lookup", "project",
    )


def test_tier2_is_the_three_semantic_additions():
    assert TIER2 == ("lod-extract", "semantic-surface", "hierarchy")


def test_tier1_runs_on_all_five_systems():
    assert systems_for("count") == (
        "cityparquet", "cityparquet-hilbert", "duckdb-cityparquet", "cjdb", "3dcitydb",
    )


def test_tier2_runs_on_sql_systems_only():
    # The Rust child implements only the inherited seven; rather than edit
    # the submodule, tier 2 is SQL-only and the gap is disclosed.
    assert systems_for("hierarchy") == ("duckdb-cityparquet", "cjdb", "3dcitydb")


def test_unknown_scenario_raises():
    with pytest.raises(KeyError):
        systems_for("nonsense")


@pytest.mark.parametrize("scenario", sorted(COUNT_FROM_FIRST_COLUMN))
def test_count_mode_is_first_column_for_first_column_scenarios(scenario):
    assert count_mode(scenario) == "first-column"


@pytest.mark.parametrize("scenario", sorted(COUNT_FROM_ROWCOUNT))
def test_count_mode_is_rowcount_for_rowcount_scenarios(scenario):
    assert count_mode(scenario) == "rowcount"


def test_count_mode_unknown_scenario_raises():
    with pytest.raises(KeyError):
        count_mode("nonsense")


def test_count_mode_sets_partition_all_scenarios_exactly():
    # Every scenario in ALL must be in exactly one of the two sets: none
    # missing (count_mode would raise for a real scenario), none in both
    # (extract_count would be told two contradictory things about the same
    # scenario). This is the invariant a newly added scenario could break
    # silently if only added to ALL and forgotten here.
    assert COUNT_FROM_FIRST_COLUMN | COUNT_FROM_ROWCOUNT == set(ALL)
    assert COUNT_FROM_FIRST_COLUMN & COUNT_FROM_ROWCOUNT == set()
