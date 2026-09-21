"""Publishing: lay converted packages out as the tree the public bucket serves.

The layout is `<out>/<collection>/<slug>/` — or `<out>/<collection>/` itself for
a collection of one package — with every Item's id, title and structural links
rewritten to match, and the payload hard-linked rather than copied.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from catalog2cityparquet import publish


def _package(root: Path, name: str, extra: dict | None = None) -> Path:
    pkg = root / name
    pkg.mkdir(parents=True)
    (pkg / "building.parquet").write_bytes(b"PAR1")
    doc = {
        "type": "Feature",
        "stac_version": "1.1.0",
        "id": name,
        "properties": {"city3d:lods": ["2"]},
        "assets": {"data": {"href": "./building.parquet"}},
        "links": [
            {"rel": "collection", "href": "../../collection.json"},
            {"rel": "root", "href": "../../../catalog.json"},
            {"rel": "via", "href": "https://example.invalid/a.zip"},
        ],
        **(extra or {}),
    }
    (pkg / "metadata.json").write_text(json.dumps(doc), encoding="utf-8")
    return pkg


def _spec(src: Path, **collection) -> publish.CollectionSpec:
    return publish.CollectionSpec(
        **{"name": "plateau", "source": "jp", "packages": str(src / "*"), **collection}
    )


def test_packages_are_laid_out_under_their_slug(tmp_path):
    src = tmp_path / "src"
    _package(src, "13101_chiyoda-ku_pref_2025_citygml_1_op")
    _package(src, "13229_nishitokyo-shi_pref_2025_citygml_1_op")
    spec = _spec(src, slug=r"^\d+_(?P<slug>[^_]+)_")

    written = publish.lay_out(spec, tmp_path / "out")

    assert sorted(p.name for p in written) == ["chiyoda-ku", "nishitokyo-shi"]
    doc = json.loads((tmp_path / "out/plateau/chiyoda-ku/metadata.json").read_text())
    assert doc["id"] == "chiyoda-ku"
    assert doc["collection"] == "plateau"
    assert doc["properties"]["title"] == "Chiyoda-ku"
    # Footer-derived properties are carried untouched.
    assert doc["properties"]["city3d:lods"] == ["2"]
    rels = {link["rel"]: link["href"] for link in doc["links"]}
    assert rels["collection"] == "../collection.json"
    assert rels["parent"] == "../collection.json"
    assert rels["root"] == "../../catalog.json"
    # Provenance survives.
    assert rels["via"] == "https://example.invalid/a.zip"


def test_the_payload_is_hard_linked_not_copied(tmp_path):
    src = tmp_path / "src"
    pkg = _package(src, "a_x_")
    publish.lay_out(_spec(src, slug=r"^a_(?P<slug>x)"), tmp_path / "out")
    dest = tmp_path / "out/plateau/x/building.parquet"
    assert os.stat(dest).st_ino == os.stat(pkg / "building.parquet").st_ino
    # The Item is rewritten, so it must be a new file, not a link the rewrite
    # would edit in place under the source package.
    src_doc = json.loads((pkg / "metadata.json").read_text())
    assert src_doc["id"] == "a_x_"


def test_a_single_package_collection_is_flat(tmp_path):
    src = tmp_path / "3dbag"
    _package(tmp_path, "3dbag")
    spec = publish.CollectionSpec(
        name="3dbag", source="netherlands-3d-bag", packages=str(src), title="3DBAG"
    )
    publish.lay_out(spec, tmp_path / "out")

    doc = json.loads((tmp_path / "out/3dbag/metadata.json").read_text())
    assert doc["id"] == "3dbag"
    assert doc["properties"]["title"] == "3DBAG"
    rels = {link["rel"]: link["href"] for link in doc["links"]}
    assert rels["collection"] == "./collection.json"
    assert rels["root"] == "../catalog.json"


def test_two_packages_with_one_slug_are_refused(tmp_path):
    src = tmp_path / "src"
    _package(src, "1_a_x")
    _package(src, "2_a_y")
    with pytest.raises(ValueError, match="slug"):
        publish.lay_out(_spec(src, slug=r"^\d+_(?P<slug>a)"), tmp_path / "out")


def test_a_package_the_slug_does_not_match_is_refused(tmp_path):
    src = tmp_path / "src"
    _package(src, "nomatch")
    with pytest.raises(ValueError, match="nomatch"):
        publish.lay_out(_spec(src, slug=r"^\d+_(?P<slug>a)"), tmp_path / "out")


def test_a_directory_without_an_item_is_not_a_package(tmp_path):
    # A half-written conversion leaves Parquet and no Item; publishing it would
    # advertise a package that does not exist.
    src = tmp_path / "src"
    _package(src, "1_ok_")
    (src / "1_broken_").mkdir()
    (src / "1_broken_" / "building.parquet").write_bytes(b"PAR1")
    written = publish.lay_out(_spec(src, slug=r"^\d+_(?P<slug>[a-z]+)_"), tmp_path / "out")
    assert [p.name for p in written] == ["ok"]


def test_republishing_replaces_the_previous_tree(tmp_path):
    src = tmp_path / "src"
    _package(src, "1_a_")
    spec = _spec(src, slug=r"^\d+_(?P<slug>a)_")
    publish.lay_out(spec, tmp_path / "out")
    (tmp_path / "out/plateau/stale").mkdir()
    publish.lay_out(spec, tmp_path / "out")
    assert not (tmp_path / "out/plateau/stale").exists()


def test_the_spec_file_is_read(tmp_path):
    spec_file = tmp_path / "s.yaml"
    spec_file.write_text(
        "catalog:\n  id: showcase\n  title: Datasets\n"
        "collections:\n"
        "  - name: plateau\n    source: japan-plateau-3d\n    packages: a/*\n"
        "    slug: '^(?P<slug>x)'\n",
        encoding="utf-8",
    )
    spec = publish.load_spec(spec_file)
    assert spec.catalog == {"id": "showcase", "title": "Datasets"}
    assert spec.collections[0].name == "plateau"
    assert spec.collections[0].slug == "^(?P<slug>x)"


def test_a_relative_package_glob_resolves_against_the_data_root(tmp_path):
    _package(tmp_path / "data/src", "1_a_")
    spec = publish.CollectionSpec(name="p", source="s", packages="src/*", slug=r"^\d+_(?P<slug>a)_")
    written = publish.lay_out(spec, tmp_path / "out", data_root=tmp_path / "data")
    assert [p.name for p in written] == ["a"]


def test_each_collection_carries_its_sources_identity_under_its_own_id(tmp_path, monkeypatch):
    calls = []
    monkeypatch.setattr(
        publish.discover,
        "fetch_collection",
        lambda base, cid, client: {"id": cid, "title": f"T {cid}", "license": "CC-BY-4.0"},
    )
    monkeypatch.setattr(
        publish.aggregate,
        "update_collection",
        lambda tool, items_dir, config, out, geoparquet: calls.append(
            ("collection", items_dir.name, __import__("yaml").safe_load(config.read_text()))
        ),
    )
    monkeypatch.setattr(
        publish.aggregate,
        "update_catalog",
        lambda tool, collections, out_dir, config: calls.append(
            ("catalog", [p.parent.name for p in collections])
        ),
    )
    spec = publish.Spec(
        catalog={"id": "showcase"},
        collections=[
            publish.CollectionSpec(
                name="plateau", source="japan-plateau-3d", packages="x", overrides={"title": "P"}
            ),
            publish.CollectionSpec(name="3dbag", source="netherlands-3d-bag", packages="y"),
        ],
    )
    publish.aggregate_tree(spec, tmp_path / "out", tool=Path("t"), base_url="b", client=None)

    assert calls[0] == (
        "collection",
        "plateau",
        {"id": "plateau", "title": "P", "license": "CC-BY-4.0"},
    )
    assert calls[1][2]["id"] == "3dbag" and calls[1][2]["title"] == "T netherlands-3d-bag"
    assert calls[2] == ("catalog", ["plateau", "3dbag"])
