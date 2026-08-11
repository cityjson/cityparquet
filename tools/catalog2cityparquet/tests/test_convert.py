import json

import pytest

from catalog2cityparquet import convert
from catalog2cityparquet.discover import Item
from catalog2cityparquet.ledger import REASONS


def test_converter_errors_map_to_ledger_reasons():
    # The driver must classify failures, because "it failed" is not a finding.
    # These strings are the converter's real messages.
    assert (
        convert.classify_error("unsupported CityGML version 1.0 (only CityGML 2.0 is supported)")
        == "unsupported_citygml_version"
    )
    assert (
        convert.classify_error('CityGML srsName "EPSG:4979" resolves to geographic CRS 4979')
        == "geographic_crs"
    )
    assert (
        convert.classify_error(
            "source carries a CRS-bearing coordinate but declares no CRS a writer can resolve"
        )
        == "no_crs"
    )
    assert (
        convert.classify_error("invalid CityJSON: invalid type: integer `1`, expected a string")
        == "unsupported_cityjson_version"
    )
    assert convert.classify_error("something else entirely") == "convert_failed"


def test_every_classified_reason_is_ledger_vocabulary():
    # The ledger refuses a reason outside its closed set, so a classifier that
    # drifted from it would only be found at the end of a very long run.
    messages = [
        "unsupported CityGML version 1.0 (only CityGML 2.0 is supported)",
        'CityGML srsName "EPSG:4979" resolves to geographic CRS 4979; the reader only '
        "supports projected (metre-based) CRS",
        "source carries a CRS-bearing coordinate but declares no CRS a writer can "
        "resolve to PROJJSON",
        "invalid CityJSON: invalid type: integer `1`, expected a string at line 1 column 2860",
        "something else entirely",
        "",
    ]
    assert {convert.classify_error(message) for message in messages} <= REASONS


def test_stamp_adds_collection_and_links_without_touching_properties(tmp_path):
    pkg = tmp_path / "pkg"
    pkg.mkdir()
    original = {
        "type": "Feature",
        "stac_version": "1.1.0",
        "id": "tile-1",
        "properties": {"city3d:city_objects": 873, "proj:code": "EPSG:7415"},
        "links": [],
        "assets": {"data": {"href": "./building.parquet"}},
    }
    (pkg / "metadata.json").write_text(json.dumps(original))

    item = Item(
        collection="netherlands-3d-bag",
        item_id="tile-1",
        href="https://data.3dbag.nl/x.city.json.gz",
        media_type="application/city+json",
        source_item_url=(
            "https://storage.googleapis.com/city3d-stac/netherlands-3d-bag/items/tile-1.json"
        ),
    )
    convert.stamp(pkg, item)

    written = json.loads((pkg / "metadata.json").read_text())
    assert written["collection"] == "netherlands-3d-bag"
    rels = {link["rel"]: link["href"] for link in written["links"]}
    assert rels["collection"] == "../../collection.json"
    assert rels["parent"] == "../../collection.json"
    assert rels["root"] == "../../../catalog.json"
    assert rels["via"] == item.href
    assert rels["derived_from"] == item.source_item_url
    # Footer-derived properties are authoritative; the driver must not edit them.
    assert written["properties"] == original["properties"]


def test_stamp_is_idempotent(tmp_path):
    # A resumed run may re-stamp; links must not accumulate duplicates.
    pkg = tmp_path / "pkg"
    pkg.mkdir()
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "id": "a", "properties": {}, "links": [], "assets": {}})
    )
    item = Item("c", "a", "https://h/x", None, "https://s/i.json")
    convert.stamp(pkg, item)
    convert.stamp(pkg, item)
    written = json.loads((pkg / "metadata.json").read_text())
    assert len(written["links"]) == len({link["rel"] for link in written["links"]})


def test_stamp_keeps_absent_geometry_absent(tmp_path):
    # A package whose CRS cannot be reprojected to WGS84 carries no geometry.
    # That is honest, and the driver has nothing better to put there.
    pkg = tmp_path / "pkg"
    pkg.mkdir()
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "id": "a", "geometry": None, "bbox": None, "properties": {}})
    )
    convert.stamp(pkg, Item("c", "a", "https://h/x", None, None))
    written = json.loads((pkg / "metadata.json").read_text())
    assert written["geometry"] is None
    assert written["bbox"] is None
    assert "derived_from" not in {link["rel"] for link in written["links"]}


def test_stamp_raises_when_the_item_is_missing(tmp_path):
    # A package without a valid Item is broken; skipping it silently would let
    # the run report a success that Task 9 cannot aggregate.
    pkg = tmp_path / "pkg"
    pkg.mkdir()
    with pytest.raises(FileNotFoundError):
        convert.stamp(pkg, Item("c", "a", "https://h/x", None, None))


def test_run_convert_raises_a_classified_error(tmp_path):
    fake = tmp_path / "fake-cityparquet"
    fake.write_text(
        "#!/bin/sh\n"
        "echo 'error: schema error: unsupported CityGML version 1.0 "
        "(only CityGML 2.0 is supported)' >&2\n"
        "exit 1\n"
    )
    fake.chmod(0o755)

    with pytest.raises(convert.ConvertError) as excinfo:
        convert.run_convert(fake, [tmp_path / "in.gml"], tmp_path / "out", None, timeout=30)
    assert excinfo.value.reason == "unsupported_citygml_version"


def test_run_convert_returns_the_object_count(tmp_path):
    fake = tmp_path / "fake-cityparquet"
    fake.write_text("#!/bin/sh\necho '2231 2 0 0 0 0 0 0 0'\nexit 0\n")
    fake.chmod(0o755)

    count = convert.run_convert(fake, [tmp_path / "in.json"], tmp_path / "out", None, timeout=30)
    assert count == 2231


def test_run_convert_reports_zero_for_unparseable_stdout(tmp_path):
    # A success that printed nothing countable is still a success.
    fake = tmp_path / "fake-cityparquet"
    fake.write_text("#!/bin/sh\nexit 0\n")
    fake.chmod(0o755)

    assert (
        convert.run_convert(fake, [tmp_path / "in.json"], tmp_path / "out", None, timeout=30) == 0
    )


def test_run_convert_passes_the_operator_supplied_crs(tmp_path):
    # The CRS is per collection and the converter only honours it for a source
    # that declares none, so it must reach the command line unchanged.
    fake = tmp_path / "fake-cityparquet"
    fake.write_text('#!/bin/sh\necho "$@" > "$0.args"\necho 1\n')
    fake.chmod(0o755)

    convert.run_convert(fake, [tmp_path / "in.gml"], tmp_path / "out", "EPSG:25832", timeout=30)
    args = (tmp_path / "fake-cityparquet.args").read_text().split()
    assert args[0] == "convert"
    assert "--overwrite" in args
    assert args[-2:] == ["--crs", "EPSG:25832"]


def test_run_convert_raises_on_timeout(tmp_path):
    # A hung converter is a failure like any other: it must be classified and
    # ledgered, not left to block the pool for ever.
    fake = tmp_path / "fake-cityparquet"
    fake.write_text("#!/bin/sh\nsleep 5\n")
    fake.chmod(0o755)

    with pytest.raises(convert.ConvertError) as excinfo:
        convert.run_convert(fake, [tmp_path / "in.json"], tmp_path / "out", None, timeout=0.2)
    assert excinfo.value.reason == "convert_failed"
    assert "timed out" in excinfo.value.detail


def test_convert_error_detail_is_bounded(tmp_path):
    # Some converter failures are verbose; the ledger holds one line per item.
    fake = tmp_path / "fake-cityparquet"
    fake.write_text("#!/bin/sh\nawk 'BEGIN{while(i++<9000)printf \"x\"}' >&2\nexit 1\n")
    fake.chmod(0o755)

    with pytest.raises(convert.ConvertError) as excinfo:
        convert.run_convert(fake, [tmp_path / "in.json"], tmp_path / "out", None, timeout=30)
    assert len(excinfo.value.detail) <= convert.MAX_DETAIL_CHARS
