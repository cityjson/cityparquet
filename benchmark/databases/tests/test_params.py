import json
from pathlib import Path

import duckdb
import pytest

from citybench.config import BBox, window_for_target
from citybench.params import (
    ATTR_RANGE_QUANTILE, derive, from_json, resolve_attr_filter,
    resolve_attr_range, source_facts, to_json,
)

FIXTURE = Path(__file__).parent / "fixtures" / "tiny.city.jsonl"
FIXTURE_TRANSFORM = Path(__file__).parent / "fixtures" / "tiny_transform.city.jsonl"


#: Attribute names the fixture builder types as VARCHAR rather than DOUBLE.
_STRING_ATTRS = frozenset({
    "roofType", "b3_dak_type", "class", "klumMaterialClass", "BIN", "function",
})


def build_package(directory: Path, rows: list[dict], attributes: list[str],
                  name: str = "building") -> Path:
    """A minimal but REAL CityParquet package: a Parquet object table whose
    `city` footer declares its CityJSON attributes, plus the `metadata.json`
    STAC Item whose asset roles `config.object_table_files` reads.

    Derivation reads the package, not a mock, so these tests exercise the
    same `parquet_kv_metadata`/`bbox`-struct path a real run takes.
    """
    directory.mkdir(parents=True, exist_ok=True)
    conn = duckdb.connect()
    columns = ["id", "object_type", "bbox", *attributes]
    values = []
    for row in rows:
        bbox = row["bbox"]
        literals = [
            f"'{row['id']}'",
            f"'{row['object_type']}'",
            ("{{'xmin': {0}::DOUBLE, 'ymin': {1}::DOUBLE, 'zmin': {2}::DOUBLE, "
             "'xmax': {3}::DOUBLE, 'ymax': {4}::DOUBLE, 'zmax': {5}::DOUBLE}}"
             ).format(*bbox),
        ]
        for attribute in attributes:
            value = row.get(attribute)
            if value is None:
                literals.append("NULL::DOUBLE" if attribute not in _STRING_ATTRS
                                else "NULL::VARCHAR")
            elif isinstance(value, str):
                escaped = value.replace("'", "''")
                literals.append(f"'{escaped}'")
            else:
                literals.append(f"{float(value)!r}::DOUBLE")
        values.append("(" + ", ".join(literals) + ")")
    conn.execute(
        f"CREATE TABLE t ({', '.join(columns)}) AS "
        f"SELECT * FROM (VALUES {', '.join(values)}) v({', '.join(columns)})"
    )
    footer = json.dumps({"attributes": attributes, "version": "1.0.0"}).replace("'", "''")
    target = directory / f"{name}.parquet"
    conn.execute(
        f"COPY t TO '{target}' (FORMAT PARQUET, KV_METADATA {{city: '{footer}'}})"
    )
    conn.close()
    (directory / "metadata.json").write_text(
        json.dumps({
            "type": "Feature",
            "assets": {
                name: {"href": f"./{name}.parquet",
                       "roles": ["data", "cityparquet-objects"]},
            },
        })
    )
    return directory


def line_package(directory: Path, count: int = 200, **kwargs) -> Path:
    """`count` evenly spaced boxes along the x axis.

    A window centred on the median row centre grows symmetrically, so it
    admits rows two at a time; with 200 rows the 1 %, 5 % and 25 % targets
    (2, 10 and 50 rows) are therefore all exactly reachable and none of the
    three windows is `approx`. A square grid is NOT a good fixture for
    this: there the reachable counts are 4, 16, 36 … and 25 % is not among
    them, which tests the `approx` path rather than the exact one.
    """
    rows = [
        {
            "id": f"o{i}",
            "object_type": "Building" if i % 2 == 0 else "BuildingPart",
            "bbox": (float(i), 0.0, 0.0, float(i) + 0.5, 0.5, 1.0),
            "roofType": "flat" if i % 4 else "slanted",
            "height": float(i),
        }
        for i in range(count)
    ]
    return build_package(directory, rows, ["roofType", "height"], **kwargs)


def test_source_facts_count_every_city_object_not_just_features():
    # 2 features, but 3 CityObjects (b1, b1-0, b2)
    total, target_id, numeric = source_facts(FIXTURE)
    assert total == 3
    assert target_id in {"b1", "b1-0", "b2"}
    assert numeric == "h_dak_max"


def test_window_is_centred_on_the_median_row_and_hits_its_row_target(tmp_path):
    package = line_package(tmp_path / "line.parquet")
    conn = duckdb.connect()
    table = f"read_parquet('{package / 'building.parquet'}')"
    from citybench.params import resolve_windows

    dataset, centre, windows, rows = resolve_windows(table, conn)
    conn.close()

    assert rows == 200
    # The rows span [0, 199.5]; the median row centre is the middle of the
    # DATA, not the lower-left corner the retired construction anchored at.
    assert centre[0] == pytest.approx(99.75, abs=0.5)
    assert dataset.minx == 0.0 and dataset.maxx == pytest.approx(199.5)

    achieved = {w.tag: w.achieved for w in windows}
    assert achieved["bbox-1pct"] == pytest.approx(0.01)
    assert achieved["bbox-5pct"] == pytest.approx(0.05)
    assert achieved["bbox-25pct"] == pytest.approx(0.25)
    assert not any(w.approx for w in windows)

    # The achieved fraction must be what the SQL predicate the scenarios run
    # will actually return, not an estimate of it.
    conn = duckdb.connect()
    for window in windows:
        w = window.window
        matched = conn.execute(
            f"SELECT count(*) FROM {table} WHERE bbox.xmax >= ? AND bbox.xmin <= ? "
            f"AND bbox.ymax >= ? AND bbox.ymin <= ?",
            [w.minx, w.maxx, w.miny, w.maxy],
        ).fetchone()[0]
        assert matched == round(window.achieved * rows)
    conn.close()


def test_window_reports_approx_when_the_target_is_unreachable():
    """1 % of a 10-row dataset is 0.1 rows: the nearest achievable window
    holds one row, i.e. 10 %, which is outside the ±10 %-of-target band —
    disclosed as `approx`, never silently missed."""
    boxes = [(float(i), 0.0, float(i) + 0.1, 0.1) for i in range(10)]
    dataset = BBox(minx=0.0, miny=0.0, minz=0.0, maxx=9.1, maxy=0.1, maxz=1.0)
    window = window_for_target(boxes, dataset, 0.01, "bbox-1pct")
    assert window.approx
    assert window.achieved > 0.0            # never an empty window
    assert window.notes_tag() == "bbox-1pct-approx"

    reachable = window_for_target(boxes, dataset, 0.5, "bbox-50pct")
    assert not reachable.approx
    assert reachable.achieved == pytest.approx(0.5, abs=0.05)
    assert reachable.notes_tag() == "bbox-50pct"


def test_attr_filter_uses_the_hand_picked_column_for_a_known_dataset(tmp_path):
    """`3dbag_*` is hand-picked as `b3_dak_type = 'slanted'` in both
    families (`params.rs::HAND_PICKED`), so the two ask the same question."""
    rows = [
        {"id": f"o{i}", "object_type": "BuildingPart",
         "bbox": (float(i), 0.0, 0.0, float(i) + 1, 1.0, 1.0),
         "b3_dak_type": "slanted" if i < 30 else "horizontal",
         "b3_h_dak_max": float(i)}
        for i in range(100)
    ]
    package = build_package(tmp_path / "3dbag_n100.parquet", rows,
                            ["b3_dak_type", "b3_h_dak_max"])
    conn = duckdb.connect()
    table = f"read_parquet('{package / 'building.parquet'}')"
    types = {r[0]: r[1] for r in conn.execute(f"DESCRIBE SELECT * FROM {table}").fetchall()}
    spec = resolve_attr_filter(
        "3dbag_n100", table, ["b3_dak_type", "b3_h_dak_max"], types, conn
    )
    conn.close()
    assert spec is not None
    assert spec.hand_picked
    assert spec.column == "b3_dak_type"
    assert spec.op == "eq" and spec.eq_value == "slanted"
    assert spec.matched == 30
    assert spec.notes_tag() == "attr=b3_dak_type=slanted"


def test_attr_filter_falls_back_to_the_derived_rule_for_an_unknown_dataset(tmp_path):
    package = line_package(tmp_path / "unknown.parquet")
    conn = duckdb.connect()
    table = f"read_parquet('{package / 'building.parquet'}')"
    types = {r[0]: r[1] for r in conn.execute(f"DESCRIBE SELECT * FROM {table}").fetchall()}
    spec = resolve_attr_filter("unknown", table, ["roofType", "height"], types, conn)
    conn.close()
    assert spec is not None
    assert not spec.hand_picked
    # `roofType` is the only string attribute with 2..1000 distinct values.
    assert spec.column == "roofType"
    assert spec.op == "eq"


def test_attr_filter_never_picks_a_structural_column(tmp_path):
    """`object_type` — the column this scenario used to be driven with —
    is not a member of the package's declared `attributes`, so it can never
    be chosen, whatever its distribution looks like."""
    package = line_package(tmp_path / "structural.parquet")
    conn = duckdb.connect()
    table = f"read_parquet('{package / 'building.parquet'}')"
    types = {r[0]: r[1] for r in conn.execute(f"DESCRIBE SELECT * FROM {table}").fetchall()}
    spec = resolve_attr_filter("unknown", table, ["roofType", "height"], types, conn)
    conn.close()
    assert spec is not None and spec.column != "object_type"


def test_attr_range_thresholds_the_preferred_column_at_its_own_quantile(tmp_path):
    rows = [
        {"id": f"o{i}", "object_type": "Building",
         "bbox": (float(i), 0.0, 0.0, float(i) + 1, 1.0, 1.0),
         "b3_dak_type": "slanted", "b3_h_dak_max": float(i)}
        for i in range(100)
    ]
    package = build_package(tmp_path / "3dbag_q.parquet", rows,
                            ["b3_dak_type", "b3_h_dak_max"])
    conn = duckdb.connect()
    table = f"read_parquet('{package / 'building.parquet'}')"
    types = {r[0]: r[1] for r in conn.execute(f"DESCRIBE SELECT * FROM {table}").fetchall()}
    spec = resolve_attr_range(
        table, ["b3_dak_type", "b3_h_dak_max"], types, "b3_h_dak_max", conn
    )
    conn.close()
    assert spec is not None
    assert spec.column == "b3_h_dak_max"
    assert spec.quantile == ATTR_RANGE_QUANTILE
    # 0.8 quantile of 0..99 is 79.2; `> 79.2` selects 20 of 100 rows.
    assert spec.threshold == pytest.approx(79.2)
    assert spec.matched == 20


def test_attr_range_is_none_without_a_numeric_attribute(tmp_path):
    rows = [
        {"id": "o1", "object_type": "Building", "bbox": (0.0, 0.0, 0.0, 1.0, 1.0, 1.0),
         "function": "railway"},
    ]
    package = build_package(tmp_path / "nonumeric.parquet", rows, ["function"])
    conn = duckdb.connect()
    table = f"read_parquet('{package / 'building.parquet'}')"
    types = {r[0]: r[1] for r in conn.execute(f"DESCRIBE SELECT * FROM {table}").fetchall()}
    assert resolve_attr_range(table, ["function"], types, None, conn) is None
    conn.close()


def test_derivation_is_deterministic_and_roundtrips(tmp_path):
    package = line_package(tmp_path / "3dbag_rt.parquet")
    first = derive(FIXTURE, package)
    second = derive(FIXTURE, package)
    assert to_json(first) == to_json(second)
    assert from_json(to_json(first)) == first

    parsed = json.loads(to_json(first))
    assert list(parsed.keys()) == sorted(parsed.keys())
    assert [w["tag"] for w in parsed["windows"]] == [
        "bbox-1pct", "bbox-5pct", "bbox-25pct",
    ]
    assert parsed["point_xy"] == list(first.point_xy)


def test_duckdb_median_agrees_with_the_ported_definition():
    """`resolve_windows` takes the window centre from DuckDB's `median()`
    while `config.median_of` defines it for the pure-Python path. The two
    must agree for both parities, or the two families' windows drift."""
    from citybench.config import median_of

    conn = duckdb.connect()
    for values in ([1.0, 2.0, 3.0], [1.0, 2.0, 3.0, 4.0], [5.0]):
        literal = ", ".join(repr(v) for v in values)
        duck = conn.execute(
            f"SELECT median(x)::DOUBLE FROM (SELECT unnest([{literal}])::DOUBLE x)"
        ).fetchone()[0]
        assert duck == pytest.approx(median_of(values))
    conn.close()


def test_extent_comes_from_the_package_not_the_source_vertices(tmp_path):
    """The extent the windows are built in is the union of the package's
    per-row `bbox` values — the format harness's own `scan_row_bboxes`
    dataset — not the source's dequantised vertex extent. `tiny.city.jsonl`
    spans [0, 30]; the package below spans [0, 9.5], and the derived
    `bbox_full` must be the latter.
    """
    package = line_package(tmp_path / "extent.parquet")
    p = derive(FIXTURE, package)
    assert p.bbox_full.minx == 0.0
    assert p.bbox_full.maxx == pytest.approx(199.5)
    # The source still supplies the selectivity denominator and the probe.
    assert p.total_city_objects == 3
    assert p.window_rows == 200


def _write_no_parent_fixture(tmp_path) -> Path:
    """A minimal CityJSONSeq source with a single childless CityObject.

    A dataset with no parent-child pair is a legitimate input (a railway or
    terrain corpus), not an error: `parts-per-building` simply reports one
    row per Building and none of them has a child.
    """
    fixture = tmp_path / "no_parent.city.jsonl"
    fixture.write_text(
        json.dumps(
            {
                "type": "CityJSON",
                "version": "2.0",
                "transform": {
                    "scale": [1.0, 1.0, 1.0],
                    "translate": [0.0, 0.0, 0.0],
                },
                "CityObjects": {},
                "vertices": [],
            }
        )
        + "\n"
        + json.dumps(
            {
                "type": "CityJSONFeature",
                "id": "b1",
                "CityObjects": {
                    "b1": {
                        "type": "Building",
                        "attributes": {"h_dak_max": 5.0},
                        "geometry": [],
                    }
                },
                "vertices": [[0, 0, 0]],
            }
        )
        + "\n"
    )
    return fixture


def test_a_childless_dataset_still_derives(tmp_path):
    """No CityObject has children. Derivation must still succeed — the
    retired `hierarchy` scenario was the only one that needed a parent id,
    and `parts-per-building` (CJDB Q4) deliberately keeps childless
    Buildings rather than needing a parent to exist at all."""
    package = line_package(tmp_path / "childless.parquet")
    p = derive(_write_no_parent_fixture(tmp_path), package)
    assert p.total_city_objects == 1
    assert p.windows and p.target_id == "b1"


def _write_no_numeric_attribute_fixture(tmp_path) -> Path:
    """A minimal CityJSONSeq source where no CityObject carries a numeric
    attribute at all — some carry no ``attributes`` object whatsoever, one
    carries only a categorical (string) attribute. A real property of the
    heterogeneity corpus (Task 14): Montreal's 294 Buildings carry no
    attributes at all, and lod3_railway's 121 CityObjects carry only
    categorical attributes ("function"/"class"/"species"), never a
    numeric one.
    """
    fixture = tmp_path / "no_numeric.city.jsonl"
    fixture.write_text(
        json.dumps(
            {
                "type": "CityJSON",
                "version": "2.0",
                "transform": {
                    "scale": [1.0, 1.0, 1.0],
                    "translate": [0.0, 0.0, 0.0],
                },
                "CityObjects": {},
                "vertices": [],
            }
        )
        + "\n"
        + json.dumps(
            {
                "type": "CityJSONFeature",
                "id": "b1",
                "CityObjects": {"b1": {"type": "Building", "geometry": []}},
                "vertices": [[0, 0, 0]],
            }
        )
        + "\n"
        + json.dumps(
            {
                "type": "CityJSONFeature",
                "id": "b2",
                "CityObjects": {
                    "b2": {
                        "type": "Building",
                        "attributes": {"function": "railway"},
                        "geometry": [],
                    }
                },
                "vertices": [[10, 10, 0]],
            }
        )
        + "\n"
    )
    return fixture


def test_numeric_column_is_none_when_no_object_has_a_numeric_attribute(tmp_path):
    """An absent numeric attribute is a legitimate dataset property, not an
    input ``derive`` cannot proceed without. This must not raise — the
    windows, `attr-filter` and `id-lookup` are all still meaningful without
    a numeric column, and only the scenarios that read it (``attr-stats``,
    and ``attr-range`` when the package has no numeric attribute either)
    are affected downstream, recorded as ``skipped:`` rows via each
    ``sql_*.sql_for``'s own ``ScenarioUnavailable`` guard.

    An EARLIER version of ``derive`` raised ``ValueError`` here, which
    blocked deriving params for the WHOLE dataset — discovered against Task
    14's own heterogeneity corpus (Montreal, lod3_railway).
    """
    package = line_package(tmp_path / "nonumeric_pkg.parquet")
    p = derive(_write_no_numeric_attribute_fixture(tmp_path), package)
    assert p.numeric_column is None
    # The rest of derivation must still have succeeded.
    assert p.total_city_objects == 2
    assert p.attr_filter is not None


def test_json_roundtrip_preserves_none_numeric_column(tmp_path):
    package = line_package(tmp_path / "nonumeric_pkg2.parquet")
    p = derive(_write_no_numeric_attribute_fixture(tmp_path), package)
    text = to_json(p)
    assert json.loads(text)["numeric_column"] is None
    assert from_json(text) == p
    assert from_json(text).numeric_column is None
