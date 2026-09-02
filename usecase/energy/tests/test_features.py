import duckdb
import pytest

from energy.errors import MissingLoD
from energy.features import available_lods, build_features, lod_to_suffix
from .conftest import requires_extensions


def test_lod_to_suffix():
    assert lod_to_suffix("2.2") == "lod2_2"
    assert lod_to_suffix("1.3") == "lod1_3"
    with pytest.raises(MissingLoD):
        lod_to_suffix("22")


def test_available_lods(fixture_path):
    con = duckdb.connect()
    assert available_lods(con, str(fixture_path)) == ["0.0", "1.2", "1.3", "2.2"]


def test_missing_lod_lists_available(fixture_path):
    con = duckdb.connect()
    with pytest.raises(MissingLoD, match="2.2"):
        build_features(con, str(fixture_path), "9.9")


@requires_extensions
def test_build_features_volumes_match_3dbag(fixture_path):
    from energy.db import connect

    table = build_features(connect(), str(fixture_path), "2.2")
    assert table.num_rows == 150
    rows = table.to_pylist()
    rel_errs = sorted(
        abs(r["volume_m3"] - r["b3_volume_lod22"]) / r["b3_volume_lod22"]
        for r in rows if r["b3_volume_lod22"]
    )
    median = rel_errs[len(rel_errs) // 2]
    assert median < 0.01, f"median volume error {median:.4f}"
    assert all(r["height_m"] > 0 for r in rows)
    assert all(r["sv_ratio"] and r["sv_ratio"] > 0 for r in rows)
