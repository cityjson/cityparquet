import json
from pathlib import Path

import pytest

from citybench.params import derive, from_json, to_json

FIXTURE = Path(__file__).parent / "fixtures" / "tiny.city.jsonl"
FIXTURE_TRANSFORM = Path(__file__).parent / "fixtures" / "tiny_transform.city.jsonl"


def test_derives_bbox_over_all_features():
    p = derive(FIXTURE)
    assert p.bbox_full.minx == 0.0
    assert p.bbox_full.miny == 0.0
    assert p.bbox_full.maxx == 30.0
    assert p.bbox_full.maxy == 30.0
    assert p.bbox_full.maxz == 5.0


def test_counts_every_city_object_not_just_features():
    # 2 features, but 3 CityObjects (b1, b1-0, b2)
    p = derive(FIXTURE)
    assert p.total_city_objects == 3


def test_picks_most_common_object_type_for_attr_filter():
    p = derive(FIXTURE)
    assert p.attr_column == "object_type"
    assert p.attr_eq == "Building"


def test_picks_a_numeric_attribute_present_on_most_objects():
    p = derive(FIXTURE)
    assert p.numeric_column == "h_dak_max"


def test_target_id_and_parent_id_are_real_objects():
    p = derive(FIXTURE)
    assert p.target_id in {"b1", "b1-0", "b2"}
    # parent_id must be an object that actually has children
    assert p.parent_id == "b1"


def test_derivation_is_deterministic():
    assert to_json(derive(FIXTURE)) == to_json(derive(FIXTURE))


def test_json_roundtrip():
    p = derive(FIXTURE)
    assert from_json(to_json(p)) == p


def test_json_is_human_readable_and_sorted():
    text = to_json(derive(FIXTURE))
    parsed = json.loads(text)
    assert parsed["attr_eq"] == "Building"
    assert list(parsed.keys()) == sorted(parsed.keys())


def test_dequantises_cityjsonseq_vertices_with_non_identity_transform():
    """Regression guard for the header-transform-before-vertices ordering.

    ``tiny.city.jsonl`` uses scale [1,1,1] / translate [0,0,0], so a bug
    that swapped the transform-capture and vertex-processing order (e.g.
    applying the identity default, or a stale transform, to a feature
    line's vertices) would still pass every assertion above — quantised
    and world coordinates coincide there by construction. This fixture
    uses a non-identity scale AND translate on the header line, so the
    same ordering bug would produce a visibly wrong bbox here.
    """
    p = derive(FIXTURE_TRANSFORM)
    assert p.bbox_full.minx == 100.0
    assert p.bbox_full.miny == 200.0
    assert p.bbox_full.minz == 10.0
    assert p.bbox_full.maxx == 110.0
    assert p.bbox_full.maxy == 210.0
    assert p.bbox_full.maxz == 20.0


def _write_no_parent_fixture(tmp_path) -> Path:
    """A minimal CityJSONSeq source where no CityObject has children.

    Used by the two tests below: a dataset with no parent-child pair is a
    legitimate input (e.g. a railway or terrain corpus), not an error, so
    ``derive`` must not raise for it — ``parent_id`` should simply be
    ``None``.
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


def test_parent_id_is_none_when_no_object_has_children(tmp_path):
    """No CityObject has children: there is no real parent-child pair to
    query, so ``parent_id`` is None rather than an id that isn't actually
    anyone's parent. This must not raise — bbox, attr-filter, attr-stats
    and id-lookup are all still meaningful for a hierarchy-less dataset,
    and only the one scenario that reads ``parent_id`` is affected.
    """
    p = derive(_write_no_parent_fixture(tmp_path))
    assert p.parent_id is None


def test_json_roundtrip_preserves_none_parent_id(tmp_path):
    """JSON null handling for an absent parent is easy to get subtly
    wrong (e.g. via a falsy-default lookup), so this is asserted
    explicitly rather than trusted to the general roundtrip test, which
    only ever exercises a dataset that does have a parent.
    """
    p = derive(_write_no_parent_fixture(tmp_path))
    text = to_json(p)
    assert json.loads(text)["parent_id"] is None
    assert from_json(text) == p
    assert from_json(text).parent_id is None


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
    """Mirrors ``test_parent_id_is_none_when_no_object_has_children``: an
    absent numeric attribute is a legitimate dataset property, not an
    input ``derive`` cannot proceed without. This must not raise — bbox,
    attr-filter, id-lookup and hierarchy are all still meaningful without
    a numeric column, and only the one scenario that reads it
    (``attr-stats``) is affected downstream (recorded as a ``skipped:``
    row, via each ``sql_*.sql_for``'s own ``ScenarioUnavailable`` guard).

    An EARLIER version of ``derive`` raised ``ValueError`` here, which
    blocked deriving params for the WHOLE dataset — not just attr-stats —
    discovered against Task 14's own heterogeneity corpus (Montreal,
    lod3_railway), which is why this test exists.
    """
    p = derive(_write_no_numeric_attribute_fixture(tmp_path))
    assert p.numeric_column is None
    # The rest of derivation must still have succeeded.
    assert p.total_city_objects == 2
    assert p.attr_eq == "Building"


def test_json_roundtrip_preserves_none_numeric_column(tmp_path):
    p = derive(_write_no_numeric_attribute_fixture(tmp_path))
    text = to_json(p)
    assert json.loads(text)["numeric_column"] is None
    assert from_json(text) == p
    assert from_json(text).numeric_column is None
