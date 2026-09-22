"""Shared construction of the `Params` every test needs.

`Params` carries the resolved query windows, the per-dataset `attr-filter`
predicate and `attr-range`'s threshold, all of which a real run derives from
a CityParquet package (`citybench.params.derive`). A unit test of one SQL
builder does not want a package on disk, so this builds the same value
directly, with the same shape, in one place: eight test modules previously
repeated the literal and every field change touched all eight.

`window_for_target` is used rather than a hand-written rectangle so the
fixture's windows carry a real `achieved` fraction and `approx` flag.
"""

from __future__ import annotations

from citybench.config import (
    BBOX_TARGETS, AttrFilter, AttrRange, BBox, Params, window_for_target,
)

#: 200 boxes evenly spaced along the fixture extent, so the three windows
#: are real searched windows rather than invented rectangles — and so every
#: target is exactly reachable, which keeps the fixture's window tags free
#: of the `-approx` suffix an unreachable target correctly adds. (A square
#: grid would be a poor choice here: a centred window admits its rows 4, 16,
#: 36 … at a time, so 25 % is not among the reachable fractions.)
_BOXES = [
    (float(i) * 0.5, 50.0, float(i) * 0.5, 50.0) for i in range(200)
]
_EXTENT = BBox(minx=0.0, miny=0.0, minz=0.0, maxx=100.0, maxy=100.0, maxz=10.0)


def make_params(**overrides) -> Params:
    """The canonical test `Params`, with any field overridden by keyword."""
    defaults = dict(
        bbox_full=_EXTENT,
        windows=tuple(
            window_for_target(_BOXES, _EXTENT, target, tag)
            for target, tag in BBOX_TARGETS
        ),
        point_xy=(45.5, 45.5),
        attr_filter=AttrFilter(
            column="b3_dak_type", op="eq", eq_value="slanted", ge_bound=None,
            matched=25, share=0.25, hand_picked=True,
        ),
        attr_range=AttrRange(
            column="b3_h_dak_max", quantile=0.8, threshold=20.0, matched=20,
        ),
        numeric_column="h_dak_max",
        target_id="obj-1",
        total_city_objects=100,
        window_rows=100,
    )
    defaults.update(overrides)
    return Params(**defaults)


def ge_attr_filter(column: str = "TerrainHeight", bound: float = 2.45) -> AttrFilter:
    """The numeric-bound form of the `attr-filter` predicate (Rotterdam's)."""
    return AttrFilter(
        column=column, op="ge", eq_value=None, ge_bound=bound,
        matched=25, share=0.25, hand_picked=True,
    )
