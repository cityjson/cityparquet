"""`cityjson_merge.py` on a real CityJSON 2.0 document.

The input is `lod3_railway.city.json`, the real fixture `just fixtures`
fetches into `lib/cityparquet-rs/tests/fixtures/`; nothing here is hand-written
CityJSON. The second document of a merge is that same document under other
ids and a shifted `transform.translate`, with its integer vertices shifted
back, so every one of its coordinates is the original's.
"""

import copy
import json
import sys
import unittest
from decimal import Decimal
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1]))
import cityjson_merge  # noqa: E402

FIXTURE = Path(__file__).parents[3] / "lib/cityparquet-rs/tests/fixtures/lod3_railway.city.json"


def load() -> dict:
    if not FIXTURE.exists():
        raise AssertionError(f"missing fixture {FIXTURE}; run `just fixtures` in lib/cityparquet-rs")
    return json.loads(FIXTURE.read_text(encoding="utf-8"))


def coordinates(doc: dict, ids) -> dict:
    """Each object's geometry, its boundaries resolved to real coordinates."""
    scale = [Decimal(repr(s)) for s in doc["transform"]["scale"]]
    translate = [Decimal(repr(t)) for t in doc["transform"]["translate"]]

    def resolve(value):
        if isinstance(value, list):
            return [resolve(v) for v in value]
        vertex = doc["vertices"][value]
        return tuple(vertex[a] * scale[a] + translate[a] for a in range(3))

    return {
        key: [resolve(g["boundaries"]) for g in doc["CityObjects"][key].get("geometry", [])]
        for key in ids
    }


def shifted_copy(doc: dict, prefix: str, steps) -> dict:
    """`doc` under prefixed ids, its translate moved by `steps` scale units
    per axis and its integers moved back by the same amount."""
    other = copy.deepcopy(doc)
    other["CityObjects"] = {prefix + k: v for k, v in other["CityObjects"].items()}
    for axis in range(3):
        scale = Decimal(repr(other["transform"]["scale"][axis]))
        translate = Decimal(repr(other["transform"]["translate"][axis]))
        other["transform"]["translate"][axis] = float(translate + steps[axis] * scale)
    for v in other["vertices"]:
        for axis in range(3):
            v[axis] -= steps[axis]
    return other


class MergeTests(unittest.TestCase):
    def test_merging_one_document_keeps_every_coordinate(self):
        doc = load()
        merged = cityjson_merge.merge([copy.deepcopy(doc)])
        self.assertEqual(merged["CityObjects"].keys(), doc["CityObjects"].keys())
        self.assertEqual(coordinates(merged, doc["CityObjects"]), coordinates(doc, doc["CityObjects"]))

    def test_two_documents_with_different_translates_keep_their_coordinates(self):
        doc = load()
        other = shifted_copy(doc, "copy-", [1000, -250, 40])
        merged = cityjson_merge.merge([copy.deepcopy(doc), copy.deepcopy(other)])
        self.assertEqual(len(merged["CityObjects"]), 2 * len(doc["CityObjects"]))
        self.assertEqual(len(merged["vertices"]), 2 * len(doc["vertices"]))
        original = coordinates(doc, doc["CityObjects"])
        copied = coordinates(merged, ["copy-" + k for k in doc["CityObjects"]])
        self.assertEqual(list(copied.values()), list(original.values()))

    def test_a_duplicate_id_across_inputs_is_refused(self):
        doc = load()
        with self.assertRaises(SystemExit):
            cityjson_merge.merge([copy.deepcopy(doc), copy.deepcopy(doc)])

    def test_z_is_requantised_half_up_within_half_a_step(self):
        doc = load()
        merged = cityjson_merge.merge([copy.deepcopy(doc)])
        before = [v[2] for v in merged["vertices"]]
        old = Decimal(repr(merged["transform"]["scale"][2]))
        target = str(old * 10)
        cityjson_merge.requantise_z(merged, target)
        self.assertEqual(merged["transform"]["scale"][2], float(target))
        for z, quantised in zip(before, (v[2] for v in merged["vertices"]), strict=True):
            self.assertLessEqual(abs(quantised * 10 - z), 5)


if __name__ == "__main__":
    unittest.main()
