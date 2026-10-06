"""The identifier-set cross-check (`citybench.identity`)."""
from __future__ import annotations

from citybench.identity import Identity, compare, summarise


def test_row_scenarios_take_the_first_column_and_count_non_null_geometry():
    got = summarise("bbox-query", ["object_id", "geometry"],
                    [("a", "g"), ("b", None), ("c", "g")])
    assert got == Identity(frozenset({"a", "b", "c"}), 2)


def test_id_only_scenarios_return_no_geometry_count():
    assert summarise("attr-filter", ["id"], [("a",), (1,)]) == Identity(
        frozenset({"a", "1"}), None)


def test_id_lookup_reads_each_schemas_own_identifier_and_geometry_columns():
    duck = summarise("id-lookup", ["fid", "id", "geometry_lod1_3", "geometry_lod2_2",
                                   "geometry_properties_lod2_2"],
                     [(7, "NL.1", b"x", None, {"t": 1})])
    cjdb = summarise("id-lookup", ["id", "object_id", "geometry"],
                     [(3, "NL.1", '[{"lod": "1.3"}, {"lod": "2.2"}]')])
    citydb = summarise("id-lookup", ["id", "objectid", "geometries"],
                       [(9, "NL.1", ["g1", "g2", None])])
    assert duck == Identity(frozenset({"NL.1"}), 1)
    # `id` is the surrogate key in the PostgreSQL schemas; the CityJSON
    # identifier column wins.
    assert cjdb == citydb == Identity(frozenset({"NL.1"}), 2)


def test_agreeing_systems_give_no_note():
    same = Identity(frozenset({"a", "b"}), 2)
    assert compare({"x": same, "y": same, "z": same}) is None


def test_a_different_set_of_the_same_size_is_named():
    note = compare({"duckdb": Identity(frozenset({"a", "b"}), None),
                    "cjdb": Identity(frozenset({"a", "c"}), None)})
    assert note.startswith("id-mismatch: duckdb vs cjdb")
    assert "['b']" in note and "['c']" in note


def test_a_different_geometry_count_is_named():
    note = compare({"duckdb": Identity(frozenset({"a"}), 1),
                    "3dcitydb": Identity(frozenset({"a"}), 0)})
    assert "geometries duckdb=1 3dcitydb=0" in note


def test_a_tolerated_count_deviation_tolerates_the_same_set_difference():
    big = frozenset(str(i) for i in range(10_000))
    assert compare({"a": Identity(big, None),
                    "b": Identity(big | {"x"}, None)}, tolerance=0.001) is None
