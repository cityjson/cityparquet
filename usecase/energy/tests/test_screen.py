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
    # Assert band selection independently first.
    band_1930 = band_for_year(bands, 1930)
    assert band_1930.name == "pre1946"
    assert (band_1930.u_roof, band_1930.u_wall, band_1930.u_ground) == (2.0, 2.0, 1.7)
    # Compute expected H_T from hardcoded band values, not from row.
    expected_ht = 2.0 * 100.0 + 2.0 * 200.0 + 1.7 * 80.0
    assert expected_ht == pytest.approx(736.0)
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


def test_year_none_kept_when_year_before_active(tmp_path):
    """year=None row is kept with year_before filter (unknown→oldest)."""
    bands = load_params()
    table = pa.table({
        "building_id": ["unknown", "recent"],
        "year": [None, 2015],
        "sv_ratio": [0.5, 0.4],
        "a_roof_flat_m2": [50.0, 50.0],
        "a_roof_pitched_m2": [50.0, 0.0],
        "a_wall_m2": [200.0, 100.0],
        "a_ground_m2": [80.0, 60.0],
    })
    path = tmp_path / "features.parquet"
    duckdb.connect().register("t", table).execute(
        f"COPY t TO '{path}' (FORMAT PARQUET)"
    )
    out = screen_features(str(path), bands, hdd=2900.0,
                          year_before=1975, sv_above=None, top=None)
    building_ids = [r["building_id"] for r in out.to_pylist()]
    assert "unknown" in building_ids  # year=None should be kept
    assert "recent" not in building_ids  # year=2015 >= year_before=1975
    # Verify unknown row gets the oldest band.
    unknown_row = [r for r in out.to_pylist() if r["building_id"] == "unknown"][0]
    assert unknown_row["u_roof"] == bands[0].u_roof


def test_sv_ratio_none_dropped_when_sv_above_active(tmp_path):
    """sv_ratio=None row is dropped when sv_above filter is active."""
    bands = load_params()
    table = pa.table({
        "building_id": ["unknown_sv", "high_sv"],
        "year": [1950, 1950],
        "sv_ratio": [None, 0.8],
        "a_roof_flat_m2": [50.0, 50.0],
        "a_roof_pitched_m2": [50.0, 0.0],
        "a_wall_m2": [200.0, 100.0],
        "a_ground_m2": [80.0, 60.0],
    })
    path = tmp_path / "features.parquet"
    duckdb.connect().register("t", table).execute(
        f"COPY t TO '{path}' (FORMAT PARQUET)"
    )
    out = screen_features(str(path), bands, hdd=2900.0,
                          year_before=None, sv_above=0.5, top=None)
    building_ids = [r["building_id"] for r in out.to_pylist()]
    assert "unknown_sv" not in building_ids  # sv_ratio=None should be dropped
    assert "high_sv" in building_ids  # sv_ratio=0.8 > 0.5


def test_empty_filter_result_preserves_schema(tmp_path):
    """Zero-match filter yields 0 rows but all output columns."""
    bands = load_params()
    out = screen_features(_features(tmp_path), bands, hdd=2900.0,
                          year_before=1900, sv_above=None, top=None)
    assert out.num_rows == 0
    # Check all expected columns are present.
    column_names = out.schema.names
    assert "building_id" in column_names
    assert "u_roof" in column_names
    assert "u_wall" in column_names
    assert "u_ground" in column_names
    assert "h_t_w_per_k" in column_names
    assert "annual_kwh" in column_names
    assert "rank" in column_names
