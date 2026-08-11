"""The published collection.json is the metadata seed for the mirror.

Most tests here are pure unit tests over dicts or shell out to a *fake*
`city3dstac` written into `tmp_path`. The last four drive the real vendored
binary — they are what established how it treats an unlocated Item — and skip
when it has not been built. Nothing here touches the network.
"""

import json
import stat
from pathlib import Path

import pytest
import yaml

from catalog2cityparquet import aggregate

#: The vendored tool, built by `cargo build --release --manifest-path
#: vendor/city3d-stac-tool/Cargo.toml`. The tests that use it are skipped when
#: it is absent; none of them touch the network.
CITY3DSTAC = (
    Path(__file__).resolve().parents[3] / "vendor/city3d-stac-tool/target/release/city3dstac"
)


def test_config_is_derived_from_the_published_collection():
    # No registry dependency: the collection.json fetched during traversal is
    # the metadata seed, so ids always match.
    published = {
        "id": "rotterdam-3d",
        "title": "Rotterdam 3D City Model",
        "description": "3D LoD2 city model of Rotterdam.",
        "license": "other",
        "keywords": ["3d city model", "buildings"],
        "providers": [{"name": "Municipality of Rotterdam", "roles": ["producer"]}],
        "links": [
            {"rel": "source", "href": "https://data.rotterdam.nl/", "type": "text/html"},
            {"rel": "self", "href": "./collection.json"},
            {"rel": "item", "href": "./items/x.json"},
        ],
        "extent": {"spatial": {"bbox": [[0, 0, 0, 1, 1, 1]]}},
    }
    config = aggregate.collection_config(published)

    assert config["id"] == "rotterdam-3d"
    assert config["title"] == "Rotterdam 3D City Model"
    assert config["license"] == "other"
    assert config["keywords"] == ["3d city model", "buildings"]
    assert config["providers"][0]["name"] == "Municipality of Rotterdam"
    # Structural links belong to the generated tree, not the config: carrying
    # them over would point the mirror at the source catalogue's items.
    rels = {link["rel"] for link in config["links"]}
    assert rels == {"source"}
    # Extent is recomputed by the tool from the generated items.
    assert "extent" not in config


def test_config_round_trips_through_yaml(tmp_path):
    config = aggregate.collection_config({"id": "x", "description": "d", "license": "CC-BY-4.0"})
    path = aggregate.write_config(config, tmp_path / "x.yaml")
    assert yaml.safe_load(path.read_text())["license"] == "CC-BY-4.0"


def test_missing_optional_fields_are_omitted_not_nulled():
    config = aggregate.collection_config({"id": "x", "description": "d"})
    assert "keywords" not in config
    assert "providers" not in config
    assert config["id"] == "x"


def test_only_fields_the_tool_accepts_are_emitted():
    # `CollectionConfigFile` is the contract. A field outside it is at best
    # dropped and at worst rejects the whole config, so nothing else may leak
    # out of a third-party collection.json.
    published = {
        "id": "x",
        "description": "d",
        "type": "Collection",
        "stac_version": "1.1.0",
        "stac_extensions": ["https://example.com/schema.json"],
        "summaries": {"city3d:lods": ["2.2"]},
        "assets": {"portal": {"href": "https://example.com/"}},
        "item_assets": {"data": {"type": "application/json"}},
    }
    assert set(aggregate.collection_config(published)) <= aggregate.TOOL_CONFIG_FIELDS


def test_links_the_tool_cannot_parse_are_dropped():
    # `rel` and `href` are required by the tool's LinkConfig; a link missing
    # either would make the whole config unparseable and lose the collection.
    published = {
        "id": "x",
        "description": "d",
        "links": [
            {"rel": "about", "href": "https://example.com/about"},
            {"rel": "license"},
            {"href": "https://example.com/orphan"},
        ],
    }
    assert aggregate.collection_config(published)["links"] == [
        {"rel": "about", "href": "https://example.com/about"}
    ]


def test_catalog_config_carries_only_identity():
    catalog = {
        "id": "city3d",
        "title": "City3D",
        "description": "Catalogue of 3D city models.",
        "type": "Catalog",
        "links": [{"rel": "child", "href": "./a/collection.json"}],
    }
    assert aggregate.catalog_config(catalog) == {
        "id": "city3d",
        "title": "City3D",
        "description": "Catalogue of 3D city models.",
    }


def _fake_tool(tmp_path, body: str):
    """Write an executable stand-in for `city3dstac` and return its path."""
    path = tmp_path / "fake-city3dstac"
    path.write_text("#!/usr/bin/env python3\nimport sys\n" + body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def test_update_collection_passes_items_dir_and_geoparquet(tmp_path):
    argv_log = tmp_path / "argv.txt"
    tool = _fake_tool(tmp_path, f"open({str(argv_log)!r}, 'w').write('\\n'.join(sys.argv[1:]))\n")
    aggregate.update_collection(
        tool, tmp_path / "items", tmp_path / "c.yaml", tmp_path / "collection.json"
    )
    argv = argv_log.read_text().splitlines()

    assert argv[0] == "update-collection"
    assert argv[argv.index("--items-dir") + 1] == str(tmp_path / "items")
    assert argv[argv.index("--config") + 1] == str(tmp_path / "c.yaml")
    assert argv[argv.index("-o") + 1] == str(tmp_path / "collection.json")
    assert "--geoparquet" in argv


def test_update_collection_omits_geoparquet_when_disabled(tmp_path):
    argv_log = tmp_path / "argv.txt"
    tool = _fake_tool(tmp_path, f"open({str(argv_log)!r}, 'w').write('\\n'.join(sys.argv[1:]))\n")
    aggregate.update_collection(
        tool,
        tmp_path / "items",
        tmp_path / "c.yaml",
        tmp_path / "collection.json",
        geoparquet=False,
    )
    assert "--geoparquet" not in argv_log.read_text().splitlines()


def test_update_collection_raises_with_the_captured_stderr(tmp_path):
    tool = _fake_tool(tmp_path, "sys.stderr.write('Error: boom\\n')\nsys.exit(1)\n")
    with pytest.raises(RuntimeError, match="boom"):
        aggregate.update_collection(
            tool, tmp_path / "items", tmp_path / "c.yaml", tmp_path / "collection.json"
        )


def test_an_all_unlocated_collection_fails_with_an_explanation(tmp_path):
    # `cityparquet` omits geometry/bbox when the package CRS cannot be
    # reprojected to WGS84, and the tool refuses to build a collection with no
    # spatial extent. Task 10 ledgers this, so the message must say why rather
    # than leaking a bare tool error.
    tool = _fake_tool(
        tmp_path,
        "sys.stderr.write('Error: STAC generation error: "
        "Spatial extent bbox is required\\n')\nsys.exit(1)\n",
    )
    with pytest.raises(RuntimeError, match="unlocated"):
        aggregate.update_collection(
            tool, tmp_path / "items", tmp_path / "c.yaml", tmp_path / "collection.json"
        )


#: What the real tool does when an Item carries `geometry: null` and
#: `--geoparquet` is on: collection.json is written (advertising an
#: `items-geoparquet` asset), a zero-byte items.parquet is left behind, and the
#: process exits 1. Verified against the built binary.
_GEOPARQUET_CHOKES = """
out = sys.argv[sys.argv.index('-o') + 1]
open(out, 'w').write('{"id": "demo"}')
if '--geoparquet' in sys.argv:
    import os
    open(os.path.join(os.path.dirname(out), 'items.parquet'), 'w').close()
    sys.stderr.write('Error: STAC generation error: GeoParquet encode error: '
                     'Encountered a non-object type for GeoJSON: `null`\\n')
    sys.exit(1)
"""


def _counting_tool(tmp_path, body: str):
    """A fake tool that appends one line per invocation to `calls.txt`."""
    log = tmp_path / "calls.txt"
    return _fake_tool(
        tmp_path,
        f"open({str(log)!r}, 'a').write(' '.join(sys.argv[1:]) + '\\n')\n" + body,
    ), log


def test_a_geoparquet_failure_falls_back_to_a_plain_collection(tmp_path):
    # CityParquet emits unlocated Items on purpose, and the GeoParquet encoder
    # cannot encode a null geometry. Losing the whole collection over the
    # optional sidecar would be the wrong trade: the collection is the
    # deliverable, items.parquet is a convenience.
    tool, log = _counting_tool(tmp_path, _GEOPARQUET_CHOKES)
    out = tmp_path / "out" / "collection.json"
    out.parent.mkdir()

    aggregate.update_collection(tool, tmp_path / "items", tmp_path / "c.yaml", out)

    calls = log.read_text().splitlines()
    assert len(calls) == 2, "the first attempt asks for GeoParquet, the retry does not"
    assert "--geoparquet" in calls[0]
    assert "--geoparquet" not in calls[1]
    # The zero-byte sidecar the failed attempt left behind would be advertised
    # by nothing and readable by no one.
    assert not (out.parent / "items.parquet").exists()


def test_the_fallback_does_not_hide_an_unrelated_failure(tmp_path):
    tool, log = _counting_tool(tmp_path, "sys.stderr.write('Error: boom\\n')\nsys.exit(1)\n")
    with pytest.raises(RuntimeError, match="boom"):
        aggregate.update_collection(
            tool, tmp_path / "items", tmp_path / "c.yaml", tmp_path / "collection.json"
        )
    assert len(log.read_text().splitlines()) == 1, "only a GeoParquet failure is retried"


def test_update_catalog_passes_every_collection(tmp_path):
    argv_log = tmp_path / "argv.txt"
    tool = _fake_tool(tmp_path, f"open({str(argv_log)!r}, 'w').write('\\n'.join(sys.argv[1:]))\n")
    collections = [tmp_path / "a" / "collection.json", tmp_path / "b" / "collection.json"]
    aggregate.update_catalog(tool, collections, tmp_path, tmp_path / "catalog.yaml")
    argv = argv_log.read_text().splitlines()

    assert argv[0] == "update-catalog"
    assert [str(p) for p in collections] == argv[1:3]
    assert argv[argv.index("-o") + 1] == str(tmp_path)
    assert argv[argv.index("--config") + 1] == str(tmp_path / "catalog.yaml")


def test_update_catalog_raises_with_the_captured_stderr(tmp_path):
    tool = _fake_tool(tmp_path, "sys.stderr.write('Error: nope\\n')\nsys.exit(1)\n")
    with pytest.raises(RuntimeError, match="nope"):
        aggregate.update_catalog(
            tool, [tmp_path / "collection.json"], tmp_path, tmp_path / "c.yaml"
        )


def _write_item(path: Path, item_id: str, located: bool) -> None:
    """A minimal Item in the shape `cityparquet convert` emits.

    An unlocated Item — no `geometry`, no `bbox` — is what the converter writes
    when the package CRS cannot be reprojected to WGS84.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    item = {
        "type": "Feature",
        "stac_version": "1.1.0",
        "id": item_id,
        "geometry": None,
        "bbox": None,
        "properties": {"datetime": "2026-01-01T00:00:00Z", "city3d:lods": ["2.2"]},
        "links": [],
        "assets": {"data": {"href": "./building.parquet", "roles": ["data"]}},
    }
    if located:
        item["bbox"] = [4.3, 52.0, 0.0, 4.4, 52.1, 20.0]
        item["geometry"] = {
            "type": "Polygon",
            "coordinates": [[[4.3, 52.0], [4.4, 52.0], [4.4, 52.1], [4.3, 52.1], [4.3, 52.0]]],
        }
    path.write_text(json.dumps(item), encoding="utf-8")


@pytest.mark.skipif(not CITY3DSTAC.is_file(), reason="city3dstac not built")
def test_real_tool_aggregates_a_partly_unlocated_collection(tmp_path):
    _write_item(tmp_path / "items" / "a" / "metadata.json", "a", located=False)
    _write_item(tmp_path / "items" / "b" / "metadata.json", "b", located=True)
    config = aggregate.write_config(
        aggregate.collection_config({"id": "demo", "description": "d"}), tmp_path / "c.yaml"
    )
    out = tmp_path / "collection.json"

    aggregate.update_collection(CITY3DSTAC, tmp_path / "items", config, out)

    collection = json.loads(out.read_text())
    assert collection["id"] == "demo"
    assert collection["extent"]["spatial"]["bbox"] == [[4.3, 52.0, 0.0, 4.4, 52.1, 20.0]]
    # The GeoParquet sidecar is dropped rather than dangling: the encoder
    # cannot write the unlocated Item's null geometry.
    assert "items-geoparquet" not in collection.get("assets", {})
    assert not (tmp_path / "items.parquet").exists()


@pytest.mark.skipif(not CITY3DSTAC.is_file(), reason="city3dstac not built")
def test_real_tool_refuses_an_entirely_unlocated_collection(tmp_path):
    # No item carries a bbox, so there is no spatial extent to aggregate and
    # the tool refuses to build the collection. Task 10 ledgers this as failed.
    _write_item(tmp_path / "items" / "a" / "metadata.json", "a", located=False)
    config = aggregate.write_config(
        aggregate.collection_config({"id": "demo", "description": "d"}), tmp_path / "c.yaml"
    )
    with pytest.raises(RuntimeError, match="unlocated"):
        aggregate.update_collection(
            CITY3DSTAC, tmp_path / "items", config, tmp_path / "collection.json"
        )


@pytest.mark.skipif(not CITY3DSTAC.is_file(), reason="city3dstac not built")
def test_real_tool_writes_geoparquet_when_every_item_is_located(tmp_path):
    _write_item(tmp_path / "items" / "b" / "metadata.json", "b", located=True)
    config = aggregate.write_config(
        aggregate.collection_config({"id": "demo", "description": "d"}), tmp_path / "c.yaml"
    )
    out = tmp_path / "collection.json"

    aggregate.update_collection(CITY3DSTAC, tmp_path / "items", config, out)

    assert "items-geoparquet" in json.loads(out.read_text())["assets"]
    assert (tmp_path / "items.parquet").stat().st_size > 0


@pytest.mark.skipif(not CITY3DSTAC.is_file(), reason="city3dstac not built")
def test_real_tool_accepts_a_config_derived_from_a_published_collection(tmp_path):
    # The emitter's contract with the tool: every field it emits is understood,
    # and none of the dropped ones were needed.
    _write_item(tmp_path / "items" / "b" / "metadata.json", "b", located=True)
    published = {
        "type": "Collection",
        "stac_version": "1.1.0",
        "id": "rotterdam-3d",
        "title": "Rotterdam 3D City Model",
        "description": "3D LoD2 city model of Rotterdam.",
        "license": "other",
        "keywords": ["3d city model"],
        "providers": [{"name": "Municipality of Rotterdam", "roles": ["producer"]}],
        "links": [
            {"rel": "about", "href": "https://data.rotterdam.nl/", "type": "text/html"},
            {"rel": "self", "href": "https://example.com/rotterdam-3d/collection.json"},
        ],
        "extent": {"spatial": {"bbox": [[0, 0, 1, 1]]}, "temporal": {"interval": [[None, None]]}},
        "summaries": {"city3d:lods": ["1.2"]},
    }
    config = aggregate.write_config(aggregate.collection_config(published), tmp_path / "c.yaml")
    out = tmp_path / "collection.json"

    aggregate.update_collection(CITY3DSTAC, tmp_path / "items", config, out)

    collection = json.loads(out.read_text())
    assert collection["id"] == "rotterdam-3d"
    assert collection["title"] == "Rotterdam 3D City Model"
    assert collection["license"] == "other"
    assert collection["keywords"] == ["3d city model"]
    assert collection["providers"][0]["name"] == "Municipality of Rotterdam"
    assert {"rel": "about", "href": "https://data.rotterdam.nl/", "type": "text/html"} in [
        {k: v for k, v in link.items() if k in {"rel", "href", "type"}}
        for link in collection["links"]
    ]
    rels = {link["rel"] for link in collection["links"]}
    assert "self" in rels and "item" in rels, "structural links are the generated tree's"
    # Recomputed from the generated items, not carried over from the source.
    assert collection["extent"]["spatial"]["bbox"] == [[4.3, 52.0, 0.0, 4.4, 52.1, 20.0]]
    assert collection["summaries"]["city3d:lods"] == ["2.2"]
