import dataclasses

import pytest

from citybench.config import BBox, Dataset, Measurement, Params


def test_bbox_window_fraction_anchors_lower_left():
    full = BBox(minx=0.0, miny=0.0, minz=0.0, maxx=100.0, maxy=100.0, maxz=10.0)
    win = full.window(0.25)
    assert win.minx == 0.0
    assert win.miny == 0.0
    # 25% of AREA -> 50% of each side
    assert win.maxx == pytest.approx(50.0)
    assert win.maxy == pytest.approx(50.0)
    # z is never narrowed: the window is 2D by construction
    assert win.minz == 0.0
    assert win.maxz == 10.0


def test_bbox_as_cli_list_is_six_numbers_in_order():
    b = BBox(minx=1.0, miny=2.0, minz=3.0, maxx=4.0, maxy=5.0, maxz=6.0)
    assert b.as_cli_list() == [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]


def test_params_and_measurement_are_frozen():
    assert dataclasses.is_dataclass(Params)
    p = Params(
        bbox_full=BBox(0, 0, 0, 1, 1, 1),
        attr_column="object_type",
        attr_eq="Building",
        numeric_column="h_dak_max",
        target_id="abc",
        parent_id="def",
        total_city_objects=10,
    )
    with pytest.raises(dataclasses.FrozenInstanceError):
        p.attr_column = "other"

    m = Measurement(result_count=1, times_s=[0.1], server_times_s=[], peak_rss_bytes=None)
    with pytest.raises(dataclasses.FrozenInstanceError):
        m.result_count = 2


def test_dataset_name_derives_from_path_stripping_city_suffixes():
    assert Dataset.name_from_path("/x/delft.city.jsonl") == "delft"
    assert Dataset.name_from_path("/x/Montreal.city.json") == "Montreal"
    assert Dataset.name_from_path("/x/plain.jsonl") == "plain"
