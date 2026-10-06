"""Axis order of a latitude-first dataset (Tokyo, EPSG:6697) per system.

cjdb and 3DCityDB keep the source's latitude-first order; the package's
`bbox` is longitude-first. A window applied in the wrong order must fail the
row loudly, through both the count and the identifier cross-checks."""
from citybench.config import BBox, BboxWindow
from citybench.identity import Identity, compare
from citybench.runner import cross_check
from citybench.systems import pg

PACKAGE = BBox(139.72, 35.66, 0.0, 139.79, 35.71, 200.0)   # lon-first
WINDOW = BboxWindow(tag="bbox-1pct", target=0.01, achieved=0.01,
                    window=BBox(139.75, 35.68, 0.0, 139.76, 35.69, 200.0), approx=False)
OBJECTS = {"a": (139.755, 35.685), "b": (139.751, 35.681), "c": (139.78, 35.70)}


def hits(window: BBox, coords: dict[str, tuple[float, float]]) -> set[str]:
    return {k for k, (x, y) in coords.items()
            if window.minx <= x <= window.maxx and window.miny <= y <= window.maxy}


def test_oriented_swaps_x_and_y_only_when_asked():
    w = pg.oriented(WINDOW, True).window
    assert (w.minx, w.miny, w.maxx, w.maxy) == (35.68, 139.75, 35.69, 139.76)
    assert pg.oriented(WINDOW, False) is WINDOW
    assert pg.oriented(None, True) is None


def test_a_latitude_first_store_is_detected_from_its_own_extent():
    assert pg.axis_swapped((35.666, 35.709, 139.72, 139.79), PACKAGE) is True
    assert pg.axis_swapped((139.72, 139.79, 35.666, 35.709), PACKAGE) is False
    # Projected or CRS-less data whose x and y ranges overlap: never swapped.
    assert pg.axis_swapped((0.0, 10.0, 0.0, 10.0), BBox(0, 0, 0, 10, 10, 1)) is False
    assert pg.axis_swapped(None, PACKAGE) is False


def test_a_window_in_the_wrong_axis_order_fails_both_cross_checks():
    lat_first = {k: (y, x) for k, (x, y) in OBJECTS.items()}
    duck = hits(WINDOW.window, OBJECTS)
    wrong = hits(pg.oriented(WINDOW, False).window, lat_first)
    right = hits(pg.oriented(WINDOW, True).window, lat_first)
    assert right == duck == {"a", "b"}
    _, status = cross_check({"duckdb-cityparquet": len(duck), "cjdb": len(wrong)})
    assert status == "mismatch"
    note = compare({"duckdb-cityparquet": Identity(frozenset(duck), 2),
                    "cjdb": Identity(frozenset(wrong), 0)})
    assert note is not None
    assert cross_check({"duckdb-cityparquet": len(duck), "cjdb": len(right)})[1] == "ok"
