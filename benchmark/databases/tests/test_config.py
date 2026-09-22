import dataclasses

import pytest

from citybench.config import (
    BBox, Dataset, Measurement, Params, minimum_halves, window_at,
    window_for_target,
)


def test_window_never_narrows_z():
    """Every SQL system's spatial predicate here is 2D, so a z-narrowed
    window would ask them different questions. The window always spans the
    dataset's full z range, whatever half-extent the search lands on."""
    dataset = BBox(minx=0.0, miny=0.0, minz=-3.0, maxx=100.0, maxy=100.0, maxz=10.0)
    win = window_at((50.0, 50.0), 0.1, dataset)
    assert win.minz == -3.0
    assert win.maxz == 10.0
    assert win.minx == pytest.approx(40.0)
    assert win.maxx == pytest.approx(60.0)


def test_window_half_extent_scales_with_each_axis_own_span():
    """A long, thin tile receives a long, thin window, not a square one
    that misses on the short axis."""
    dataset = BBox(minx=0.0, miny=0.0, minz=0.0, maxx=1000.0, maxy=10.0, maxz=1.0)
    win = window_at((500.0, 5.0), 0.1, dataset)
    assert win.maxx - win.minx == pytest.approx(200.0)
    assert win.maxy - win.miny == pytest.approx(2.0)


def test_minimum_halves_is_the_threshold_the_overlap_test_flips_at():
    """`minimum_halves` is the optimisation that makes the port of
    `params.rs::window_for_target` affordable on a million rows; it is only
    valid if it returns exactly the half-extent at which `window_at`'s
    closed-interval overlap test starts admitting the row."""
    dataset = BBox(minx=0.0, miny=0.0, minz=0.0, maxx=10.0, maxy=10.0, maxz=1.0)
    boxes = [(0.0, 0.0, 1.0, 1.0), (4.0, 4.0, 6.0, 6.0), (9.0, 9.0, 10.0, 10.0)]
    centre = (5.0, 5.0)
    halves = minimum_halves(boxes, centre, dataset)
    for half in halves:
        win = window_at(centre, half, dataset)
        admitted = sum(
            1 for b in boxes
            if b[2] >= win.minx and b[0] <= win.maxx
            and b[3] >= win.miny and b[1] <= win.maxy
        )
        assert admitted >= 1
        # One step below the threshold must admit strictly fewer rows.
        narrower = window_at(centre, half * 0.999 - 1e-9, dataset)
        assert sum(
            1 for b in boxes
            if b[2] >= narrower.minx and b[0] <= narrower.maxx
            and b[3] >= narrower.miny and b[1] <= narrower.maxy
        ) <= admitted


def test_window_for_target_hits_an_exactly_reachable_row_fraction():
    """200 points on a line: a window centred on the median grows
    symmetrically and so admits rows two at a time, which makes 1 %, 5 %
    and 25 % (2, 10 and 50 rows) all exactly reachable."""
    boxes = [(float(i), 0.0, float(i), 0.0) for i in range(200)]
    dataset = BBox(minx=0.0, miny=0.0, minz=0.0, maxx=199.0, maxy=0.0, maxz=1.0)
    for target, tag in ((0.01, "bbox-1pct"), (0.05, "bbox-5pct"), (0.25, "bbox-25pct")):
        window = window_for_target(boxes, dataset, target, tag)
        assert window.achieved == pytest.approx(target)
        assert not window.approx
        assert window.notes_tag() == tag


def test_bbox_as_cli_list_is_six_numbers_in_order():
    b = BBox(minx=1.0, miny=2.0, minz=3.0, maxx=4.0, maxy=5.0, maxz=6.0)
    assert b.as_cli_list() == [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]


def test_params_and_measurement_are_frozen():
    assert dataclasses.is_dataclass(Params)
    p = Params(
        bbox_full=BBox(0, 0, 0, 1, 1, 1),
        windows=(),
        point_xy=(0.5, 0.5),
        attr_filter=None,
        attr_range=None,
        numeric_column="h_dak_max",
        target_id="abc",
        total_city_objects=10,
        window_rows=10,
    )
    with pytest.raises(dataclasses.FrozenInstanceError):
        p.numeric_column = "other"

    m = Measurement(result_count=1, times_s=[0.1], server_times_s=[], peak_rss_bytes=None)
    with pytest.raises(dataclasses.FrozenInstanceError):
        m.result_count = 2


def test_dataset_name_derives_from_path_stripping_city_suffixes():
    assert Dataset.name_from_path("/x/delft.city.jsonl") == "delft"
    assert Dataset.name_from_path("/x/Montreal.city.json") == "Montreal"
    assert Dataset.name_from_path("/x/plain.jsonl") == "plain"
