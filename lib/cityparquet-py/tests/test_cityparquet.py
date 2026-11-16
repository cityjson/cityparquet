"""The bindings over the library's committed test data: convert, validate and
export in turn, offline, as the `cityparquet` CLI does them."""

from pathlib import Path

import cityparquet
import pytest

DATA = Path(__file__).parents[2] / "cityparquet-rs" / "crates" / "core" / "tests" / "data"
FIXTURE = DATA / "address_location.city.jsonl"


def test_convert_validate_export_round_trips_every_feature(tmp_path):
    package, back = tmp_path / "package", tmp_path / "back.city.jsonl"
    cityparquet.convert([FIXTURE], package)
    assert cityparquet.validate(package) == []
    cityparquet.export(package, back)
    # A header line, then one line a feature, in the source and the export alike.
    assert len(back.read_text().splitlines()) == len(FIXTURE.read_text().splitlines())


def test_a_missing_input_raises_cityparquet_error(tmp_path):
    with pytest.raises(cityparquet.CityParquetError):
        cityparquet.convert([tmp_path / "missing.city.jsonl"], tmp_path / "package")
