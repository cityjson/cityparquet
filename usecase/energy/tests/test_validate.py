import json

import pyarrow as pa

from energy.validate import validate, write_report


def _table():
    return pa.table({
        "building_id": ["a", "b"],
        "volume_m3": [100.0, 210.0],
        "a_roof_flat_m2": [10.0, 0.0],
        "a_roof_pitched_m2": [0.0, 50.0],
        "a_ground_m2": [40.0, 60.0],
        "a_wall_m2": [80.0, 120.0],
        "b3_volume_lod22": [100.0, 200.0],
        "b3_opp_dak_plat": [10.0, 0.0],
        "b3_opp_dak_schuin": [0.0, 50.0],
        "b3_opp_grond": [40.0, 60.0],
        "b3_opp_buitenmuur": [50.0, 70.0],
        "b3_opp_scheidingsmuur": [30.0, 50.0],
    })


def test_validate_metrics():
    report = validate(_table())
    assert set(report) == {"volume", "roof_flat", "roof_pitched", "ground", "wall"}
    vol = report["volume"]
    assert vol["n"] == 2
    assert abs(vol["mae"] - 5.0) < 1e-9                 # (0 + 10) / 2
    assert abs(vol["median_rel_err_pct"] - 2.5) < 1e-9  # median of 0%, 5%
    assert vol["worst"][0]["building_id"] == "b"
    assert report["wall"]["mae"] == 0.0                 # 80 = 50+30, 120 = 70+50


def test_write_report(tmp_path):
    path = tmp_path / "report.json"
    write_report(validate(_table()), str(path))
    loaded = json.loads(path.read_text())
    assert loaded["volume"]["n"] == 2


def test_zero_reference_mismatch_surfaces_in_worst():
    """Zero-reference mismatches should rank first in worst[], with rel_err_pct=null,
    excluded from median, but included in mae. Genuine 0-vs-0 matches don't count as mismatches."""
    table = pa.table({
        "building_id": ["z", "a", "b"],
        "a_roof_flat_m2": [12.5, 10.0, 0.0],
        "b3_opp_dak_plat": [0.0, 10.0, 0.0],
    })
    report = validate(table)
    roof_flat = report["roof_flat"]

    # z is a zero-reference mismatch: computed 12.5 vs reference 0
    # a is a normal match: computed 10 vs reference 10 (0% error)
    # b is a genuine 0-vs-0 match (not a mismatch)

    assert roof_flat["n"] == 3
    assert roof_flat["n_zero_reference_mismatches"] == 1

    # worst[0] should be the zero-reference mismatch (ranks first)
    assert roof_flat["worst"][0]["building_id"] == "z"
    assert roof_flat["worst"][0]["rel_err_pct"] is None
    assert roof_flat["worst"][0]["computed"] == 12.5
    assert roof_flat["worst"][0]["reference"] == 0.0

    # Median should be computed only from finite rels (a and b)
    # a: 0%, b: 0% -> median = 0%
    assert roof_flat["median_rel_err_pct"] == 0.0

    # mae should include the zero-reference mismatch: (12.5 + 0 + 0) / 3
    assert abs(roof_flat["mae"] - (12.5 / 3)) < 1e-9
