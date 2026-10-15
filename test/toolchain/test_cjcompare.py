"""The comparator's own tests: every check must fail on a real dataset with
one deliberate change in it, in the category that change belongs to, and pass
on the dataset unchanged.

    uv run --with pytest --with flatcitybuf==0.5.2 pytest test/toolchain/test_cjcompare.py

The datasets are the corpus's (corpus.toml), fetched by `run.py fetch`; the
tests skip when they are absent.
"""

from __future__ import annotations

import copy
import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

import cjcompare

CORPUS = Path(os.environ.get("TOOLCHAIN_CORPUS", Path(__file__).resolve().parents[2] / "test/runs/toolchain/corpus"))
RAILWAY = CORPUS / "LoD3_Railway.city.json"
DELFT_SEQ = CORPUS / "delft.city.jsonl"
DELFT_FCB = CORPUS / "delft.fcb"
ADDRESS_SEQ = CORPUS / "address_location_lod.city.jsonl"


def need(p: Path) -> Path:
    if not p.exists():
        pytest.skip(f"{p} not fetched (run.py fetch)")
    return p


@pytest.fixture(scope="module")
def railway() -> dict:
    with open(need(RAILWAY), encoding="utf-8") as f:
        return json.load(f)


def write(tmp_path: Path, doc: dict, name: str = "out.city.json") -> str:
    p = tmp_path / name
    p.write_text(json.dumps(doc), encoding="utf-8")
    return str(p)


def status(rep: dict, cat: str) -> str:
    return rep["categories"][cat]["status"]


def first(doc: dict, pred) -> tuple[str, dict]:
    for oid, co in doc["CityObjects"].items():
        if pred(co):
            return oid, co
    raise AssertionError("no such object in the fixture")


def surfaces_of(g: dict) -> list:
    if g["type"] in ("MultiSurface", "CompositeSurface"):
        return g["boundaries"]
    if g["type"] == "Solid":
        return [s for sh in g["boundaries"] for s in sh]
    return [s for so in g["boundaries"] for sh in so for s in sh]


def test_unchanged_dataset_passes(railway, tmp_path):
    rep = cjcompare.compare(str(RAILWAY), write(tmp_path, railway))
    assert rep["ok"], rep["examples"]
    assert status(rep, "materials") == "pass"
    assert status(rep, "textures") == "pass"
    assert status(rep, "templates") == "pass"


def test_seq_against_fcb_of_the_same_data():
    rep = cjcompare.compare(str(need(DELFT_SEQ)), str(need(DELFT_FCB)))
    assert rep["objects_source"] == rep["objects_output"] == 2231
    assert status(rep, "objects") == "pass"


def _delft_with(tmp_path: Path, change) -> str:
    """delft.city.jsonl with one object's `tijdstipregistratie` changed."""
    lines = need(DELFT_SEQ).read_text(encoding="utf-8").splitlines()
    for i, line in enumerate(lines[1:], 1):
        f = json.loads(line)
        for co in f["CityObjects"].values():
            if "tijdstipregistratie" in (co.get("attributes") or {}):
                co["attributes"]["tijdstipregistratie"] = change(co["attributes"]["tijdstipregistratie"])
                lines[i] = json.dumps(f)
                p = tmp_path / "out.city.jsonl"
                p.write_text("\n".join(lines) + "\n", encoding="utf-8")
                return str(p)
    raise AssertionError("no tijdstipregistratie in delft")


def test_same_instant_respelled_is_admitted(tmp_path):
    # A UTC-normalised TIMESTAMP column gives the instant back in another spelling.
    rep = cjcompare.compare(str(DELFT_SEQ), _delft_with(tmp_path, lambda v: datetime.fromisoformat(v).astimezone(timezone(timedelta(hours=1))).isoformat(timespec="milliseconds")))
    assert status(rep, "attributes") == "admitted"


def test_other_instant_fails_attributes(tmp_path):
    rep = cjcompare.compare(str(DELFT_SEQ), _delft_with(tmp_path, lambda v: v.replace("Z", "+01:00")))
    assert status(rep, "attributes") == "fail"


def _address_with(tmp_path: Path, change) -> str:
    lines = need(ADDRESS_SEQ).read_text(encoding="utf-8").splitlines()
    f = json.loads(lines[1])
    co = next(c for c in f["CityObjects"].values() if c.get("address"))
    change(co["address"][0])
    lines[1] = json.dumps(f)
    p = tmp_path / "out.city.jsonl"
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return str(p)


def test_changed_street_fails_address(tmp_path):
    rep = cjcompare.compare(str(ADDRESS_SEQ), _address_with(tmp_path, lambda a: a.update(thoroughfareName="Elsewhere")))
    assert status(rep, "address") == "fail"


def test_dropped_unkept_address_member_is_admitted(tmp_path):
    # A source member outside the struct's fields is not kept
    # (02-object-table-schema, "Addresses"): its absence is admitted.
    source = _address_with(tmp_path, lambda a: a.update(Country="Finland"))
    rep = cjcompare.compare(source, str(ADDRESS_SEQ))
    assert status(rep, "address") == "admitted"


def test_moved_vertex_fails_coordinates(railway, tmp_path):
    d = copy.deepcopy(railway)
    d["vertices"][0][2] += 1000  # one metre at scale 0.001
    rep = cjcompare.compare(str(RAILWAY), write(tmp_path, d))
    assert status(rep, "coordinates") == "fail"


def test_shift_within_one_quantum_passes(railway, tmp_path):
    d = copy.deepcopy(railway)
    d["transform"]["translate"][0] += d["transform"]["scale"][0] * 0.5
    rep = cjcompare.compare(str(RAILWAY), write(tmp_path, d))
    assert status(rep, "coordinates") == "pass"


def test_rotated_ring_passes_and_reversed_ring_fails(railway, tmp_path):
    oid, co = first(railway, lambda c: any(g["type"] == "MultiSurface" and "texture" not in g for g in c.get("geometry", [])))
    gi = next(i for i, g in enumerate(co["geometry"]) if g["type"] == "MultiSurface" and "texture" not in g)
    fi = next(i for i, s in enumerate(co["geometry"][gi]["boundaries"]) if len(set(s[0])) >= 4)

    d = copy.deepcopy(railway)
    ring = d["CityObjects"][oid]["geometry"][gi]["boundaries"][fi][0]
    ring[:] = ring[1:] + ring[:1]
    assert cjcompare.compare(str(RAILWAY), write(tmp_path, d))["ok"]

    d = copy.deepcopy(railway)
    d["CityObjects"][oid]["geometry"][gi]["boundaries"][fi][0].reverse()
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "coordinates") == "fail"


def test_changed_semantic_surface_fails_semantics(railway, tmp_path):
    oid, _ = first(railway, lambda c: any("semantics" in g for g in c.get("geometry", [])))
    d = copy.deepcopy(railway)
    g = next(g for g in d["CityObjects"][oid]["geometry"] if "semantics" in g)
    g["semantics"]["surfaces"][0]["type"] = "ClosureSurface"
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "semantics") == "fail"


def test_swapped_material_fails_materials(railway, tmp_path):
    oid, _ = first(railway, lambda c: any("material" in g for g in c.get("geometry", [])))
    d = copy.deepcopy(railway)
    g = next(g for g in d["CityObjects"][oid]["geometry"] if "material" in g)
    theme = next(iter(g["material"]))
    m = g["material"][theme]
    n = len(d["appearance"]["materials"])
    if "value" in m:
        m["value"] = (m["value"] + 1) % n
    else:
        m["values"][0] = (m["values"][0] + 1) % n if m["values"][0] is not None else 0
    # A different palette entry with the same content would be no change.
    assert d["appearance"]["materials"][0] != d["appearance"]["materials"][1 % n]
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "materials") == "fail"


def test_reordered_material_palette_passes(railway, tmp_path):
    d = copy.deepcopy(railway)
    n = len(d["appearance"]["materials"])
    d["appearance"]["materials"].reverse()
    geoms = [g for co in d["CityObjects"].values() for g in co.get("geometry", [])]
    geoms += d.get("geometry-templates", {}).get("templates", [])
    for g in geoms:
        for m in (g.get("material") or {}).values():
            if "value" in m and m["value"] is not None:
                m["value"] = n - 1 - m["value"]
            if "values" in m:
                m["values"] = json.loads(json.dumps(m["values"]), parse_int=lambda s: n - 1 - int(s))
    assert cjcompare.compare(str(RAILWAY), write(tmp_path, d))["ok"]


def test_moved_uv_fails_textures(railway, tmp_path):
    d = copy.deepcopy(railway)
    d["appearance"]["vertices-texture"][0][0] += 0.25
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "textures") == "fail"


def test_changed_image_fails_textures(railway, tmp_path):
    d = copy.deepcopy(railway)
    d["appearance"]["textures"][0]["image"] = "elsewhere.jpg"
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "textures") == "fail"


def test_dropped_palettes_fail_appearance(railway, tmp_path):
    # The faces keep their indices but the document defines nothing for them.
    d = copy.deepcopy(railway)
    del d["appearance"]["textures"], d["appearance"]["materials"]
    report = cjcompare.compare(str(RAILWAY), write(tmp_path, d))
    assert status(report, "textures") == "fail"
    assert status(report, "materials") == "fail"


def test_changed_template_matrix_fails_templates(railway, tmp_path):
    oid, _ = first(railway, lambda c: any(g["type"] == "GeometryInstance" for g in c.get("geometry", [])))
    d = copy.deepcopy(railway)
    g = next(g for g in d["CityObjects"][oid]["geometry"] if g["type"] == "GeometryInstance")
    g["transformationMatrix"][0] *= 2
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "templates") == "fail"


def test_changed_template_vertex_fails_templates(railway, tmp_path):
    d = copy.deepcopy(railway)
    d["geometry-templates"]["vertices-templates"][0][2] += 1.0
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "templates") == "fail"


def test_changed_attribute_fails(railway, tmp_path):
    oid, co = first(railway, lambda c: any(isinstance(v, str) for v in (c.get("attributes") or {}).values()))
    key = next(k for k, v in co["attributes"].items() if isinstance(v, str))
    d = copy.deepcopy(railway)
    d["CityObjects"][oid]["attributes"][key] += " (changed)"
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "attributes") == "fail"


def test_type_change_is_admitted_only_for_a_mixed_attribute(railway, tmp_path):
    ids = [oid for oid, c in railway["CityObjects"].items() if c.get("attributes")][:2]
    d = copy.deepcopy(railway)
    d["CityObjects"][ids[0]]["attributes"]["count"] = 3
    d["CityObjects"][ids[1]]["attributes"]["count"] = 3
    consistent = write(tmp_path, d, "consistent.city.json")
    d["CityObjects"][ids[1]]["attributes"]["count"] = 2.5
    mixed = write(tmp_path, d, "mixed.city.json")
    d["CityObjects"][ids[0]]["attributes"]["count"] = 3.0
    promoted = write(tmp_path, d, "promoted.city.json")
    rep = cjcompare.compare(mixed, promoted)
    assert rep["ok"] and status(rep, "attributes") == "admitted"
    d["CityObjects"][ids[1]]["attributes"]["count"] = 3
    assert status(cjcompare.compare(consistent, write(tmp_path, d)), "attributes") == "fail"
    d["CityObjects"][ids[0]]["attributes"]["count"] = "3"
    assert status(cjcompare.compare(consistent, write(tmp_path, d)), "attributes") == "fail"


def test_unmapped_member_may_come_back_as_an_attribute(railway, tmp_path):
    oid, _ = first(railway, lambda c: True)
    d = copy.deepcopy(railway)
    d["CityObjects"][oid]["myMember"] = {"a": 1}
    src = write(tmp_path, d, "src.city.json")
    del d["CityObjects"][oid]["myMember"]
    d["CityObjects"][oid].setdefault("attributes", {})["myMember"] = {"a": 1}
    assert cjcompare.compare(src, write(tmp_path, d))["ok"]


def test_dropped_unreferenced_semantic_surface_fails_semantics(railway, tmp_path):
    oid, _ = first(railway, lambda c: any("semantics" in g for g in c.get("geometry", [])))
    d = copy.deepcopy(railway)
    g = next(g for g in d["CityObjects"][oid]["geometry"] if "semantics" in g)
    g["semantics"]["surfaces"].append({"type": "WallSurface"})
    src = write(tmp_path, d, "src.city.json")
    assert status(cjcompare.compare(src, str(RAILWAY)), "semantics") == "fail"


def test_missing_object_fails_objects(railway, tmp_path):
    d = copy.deepcopy(railway)
    oid, _ = first(d, lambda c: not c.get("children") and not c.get("parents"))
    del d["CityObjects"][oid]
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "objects") == "fail"


def test_changed_hierarchy_fails_hierarchy(railway, tmp_path):
    d = copy.deepcopy(railway)
    _, co = first(d, lambda c: c.get("children"))
    co["children"] = co["children"][1:]
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "hierarchy") == "fail"


def test_second_geometry_at_one_lod_is_admitted(railway, tmp_path):
    d = copy.deepcopy(railway)
    _, co = first(d, lambda c: any(g["type"] == "MultiSurface" for g in c.get("geometry", [])))
    co["geometry"].append(copy.deepcopy(next(g for g in co["geometry"] if g["type"] == "MultiSurface")))
    src = write(tmp_path, d, "src.city.json")
    rep = cjcompare.compare(src, str(RAILWAY))
    assert rep["ok"] and status(rep, "geometry") == "admitted"
    # The other way round, the output holding two is a failure.
    assert status(cjcompare.compare(str(RAILWAY), src), "geometry") == "fail"


def test_dropped_face_fails_geometry(railway, tmp_path):
    d = copy.deepcopy(railway)
    _, co = first(d, lambda c: any(g["type"] == "MultiSurface" and len(g["boundaries"]) > 2 for g in c.get("geometry", [])))
    g = next(g for g in co["geometry"] if g["type"] == "MultiSurface" and len(g["boundaries"]) > 2)
    g["boundaries"].pop()
    # The face's semantics and appearance go with it, so only the face is missing.
    if "semantics" in g:
        g["semantics"]["values"].pop()
    for m in (g.get("material") or {}).values():
        if "values" in m:
            m["values"].pop()
    for t in (g.get("texture") or {}).values():
        t["values"].pop()
    assert status(cjcompare.compare(str(RAILWAY), write(tmp_path, d)), "geometry") == "fail"
