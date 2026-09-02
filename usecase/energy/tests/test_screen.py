import duckdb
import pyarrow as pa
import pytest

from energy.screen import band_for_year, load_params, screen_features


def _features(tmp_path):
    table = pa.table({
        "building_id": ["old", "new"],
        "year": [1930, 2015],
        "sv_ratio": [0.9, 0.4],
        "a_roof_flat_m2": [50.0, 50.0],
        "a_roof_pitched_m2": [50.0, 0.0],
        "a_wall_m2": [200.0, 100.0],
        "a_ground_m2": [80.0, 60.0],
    })
    path = tmp_path / "features.parquet"
    duckdb.connect().register("t", table).execute(
        f"COPY t TO '{path}' (FORMAT PARQUET)"
    )
    return str(path)


def test_default_params_load_and_cover_all_years():
    bands = load_params()
    assert len(bands) >= 4
    assert bands[-1].max_year is None
    assert band_for_year(bands, None) == bands[0]      # unknown → oldest
    assert band_for_year(bands, 1900) == bands[0]
    assert band_for_year(bands, 2030) == bands[-1]


def test_screen_computes_ht_and_ranks(tmp_path):
    bands = load_params()
    out = screen_features(_features(tmp_path), bands, hdd=2900.0,
                          year_before=None, sv_above=None, top=None)
    rows = {r["building_id"]: r for r in out.to_pylist()}
    old, new = rows["old"], rows["new"]
    expected_ht = (old["u_roof"] * 100.0 + old["u_wall"] * 200.0
                   + old["u_ground"] * 80.0)
    assert old["h_t_w_per_k"] == pytest.approx(expected_ht)
    assert old["annual_kwh"] == pytest.approx(expected_ht * 2900.0 * 24 / 1000)
    assert old["annual_kwh"] > new["annual_kwh"]
    assert old["rank"] == 1 and new["rank"] == 2


def test_screen_filters(tmp_path):
    bands = load_params()
    only_old = screen_features(_features(tmp_path), bands, hdd=2900.0,
                               year_before=1975, sv_above=None, top=None)
    assert only_old.to_pylist()[0]["building_id"] == "old"
    assert only_old.num_rows == 1
    top1 = screen_features(_features(tmp_path), bands, hdd=2900.0,
                           year_before=None, sv_above=None, top=1)
    assert top1.num_rows == 1
