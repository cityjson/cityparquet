"""Core value types shared by every part of the harness.

All types are frozen: a benchmark run must not be able to mutate the
parameters half way through, or two systems could silently be asked
different questions.
"""

from __future__ import annotations

import bisect
import json
from dataclasses import dataclass
from pathlib import Path

# The STAC asset role `cityparquet convert` stamps on every per-module
# OBJECT table it writes (verified against a real converted package — see
# `object_table_files`). Sidecar tables (materials/textures/
# geometry_templates) and the "data" alias entry (a convenience duplicate of
# the FIRST object table, for a single-family package) do not carry this
# role, so filtering on it is what tells object tables apart from everything
# else `assets` lists.
OBJECT_TABLE_ROLE = "cityparquet-objects"


def object_table_files(package: Path) -> list[str]:
    """The object-table Parquet filenames a CityParquet package lists.

    A single-family package (every delft-shaped dataset in this corpus)
    lists exactly one, ``building.parquet``. A by-type, multi-family
    package (``lod3_railway``, whose 121 CityObjects span 14 CityGML
    types — Railway, Bridge, Tunnel, CityFurniture, ... — none of them
    Building) lists several: ``railway.parquet``, ``bridge.parquet``,
    ``tunnel.parquet``, and so on. An EARLIER version of the DuckDB system
    hardcoded ``building.parquet`` (this is by-type layout's ONLY table
    name for a Building-only dataset like delft, which is why that bug
    slipped past every prior task's own smoke run), which fails outright
    against a package with no Building table at all — discovered running
    Task 14's heterogeneity corpus, not assumed in advance.

    Resolved from ``metadata.json``'s own ``assets``, filtered to entries
    whose ``roles`` include ``"cityparquet-objects"`` — verified against a
    real converted package to be exactly the per-module object tables,
    excluding both the "data" convenience alias (a duplicate pointer at
    the FIRST object table, carrying only the plain ``"data"`` role) and
    any materials/textures/geometry_templates sidecar assets (which carry
    their own, different roles). Sorted for a deterministic query shape
    across runs.

    Lives here rather than beside the DuckDB adapter because parameter
    derivation reads the package too: the `bbox` column the query windows
    are searched over and the attribute columns `attr-filter`/`attr-range`
    are picked from are both package facts, exactly as they are for the
    format harness (`benchmark/readbench/src/params.rs`).
    """
    manifest = json.loads((package / "metadata.json").read_text())
    hrefs = [
        asset["href"]
        for asset in manifest.get("assets", {}).values()
        if OBJECT_TABLE_ROLE in asset.get("roles", ())
    ]
    if not hrefs:
        raise ValueError(
            f"{package}/metadata.json lists no asset with role "
            f"{OBJECT_TABLE_ROLE!r}; not a valid CityParquet package"
        )
    # hrefs are relative ("./building.parquet"); normalise against the
    # package directory so the caller gets absolute, glob-free paths.
    return sorted((package / href).resolve().as_posix() for href in hrefs)


@dataclass(frozen=True)
class BBox:
    minx: float
    miny: float
    minz: float
    maxx: float
    maxy: float
    maxz: float

    def as_cli_list(self) -> list[float]:
        """The six numbers `cityparquet-readbench --bbox` expects, in order."""
        return [self.minx, self.miny, self.minz, self.maxx, self.maxy, self.maxz]


# `(target fraction of ROWS, notes tag)` — one CSV row per entry, per
# windowed scenario. These are the format harness's own targets and tags
# (`benchmark/readbench/src/params.rs::BBOX_TARGETS`), so a `bbox-1pct` row
# in either family names the same construction.
BBOX_TARGETS: tuple[tuple[float, str], ...] = (
    (0.01, "bbox-1pct"),
    (0.05, "bbox-5pct"),
    (0.25, "bbox-25pct"),
)

# How far the achieved fraction may sit from its target before the window is
# disclosed as `approx`, as a fraction OF THE TARGET — 10% of 1% is one part
# in a thousand, not one in ten. `params.rs::BBOX_TOLERANCE`.
BBOX_TOLERANCE: float = 0.1

# Bisection steps. The count of intersecting rows is a step function of the
# half-extent, so the search converges on a jump rather than a point; 60
# halvings take the bracket well below one row's width on any real extent.
# `params.rs::BBOX_SEARCH_STEPS`.
BBOX_SEARCH_STEPS: int = 60


@dataclass(frozen=True)
class BboxWindow:
    """One resolved bbox window, carrying its own provenance.

    A direct port of `benchmark/readbench/src/params.rs::BboxWindow`: the
    two benchmark families must mean the same thing by "the 1 % window", and
    before this port they did not (the database family scaled the extent's
    AREA from its lower-left corner and achieved 0.49 % / 6.37 % / 22.1 %
    where the format family achieved 1.00 % / 5.00 % / 25.0 % of rows —
    `notes/benchmark-fairness-review-2026-09-22.md` §4.4).
    """

    tag: str
    target: float
    achieved: float
    window: BBox
    #: `achieved` is outside `BBOX_TOLERANCE` of `target` — the target was
    #: not reachable on this data. Disclosed in `notes`, never silent.
    approx: bool

    def notes_tag(self) -> str:
        return f"{self.tag}-approx" if self.approx else self.tag


def median_of(values: list[float]) -> float:
    """The median of `values`, exactly as `params.rs::median_of` defines it.

    For an even count this is the mean of the two middle values, which is
    also what DuckDB's `median()`/`quantile_cont(x, 0.5)` computes, so the
    two families agree whichever side derives it.
    """
    if not values:
        raise ValueError("median_of needs at least one value")
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2 == 0:
        return (ordered[mid - 1] + ordered[mid]) / 2.0
    return ordered[mid]


def window_at(centre: tuple[float, float], half: float, dataset: BBox) -> BBox:
    """A window centred on `centre`, extending `half` of each of the
    dataset's OWN x/y spans, and always covering its FULL z range.

    A query window's z must never exclude a row: every SQL system's spatial
    predicate here is 2D (README Caveat 3), so a z-narrowed window would ask
    the systems different questions.
    """
    span_x = dataset.maxx - dataset.minx
    span_y = dataset.maxy - dataset.miny
    return BBox(
        minx=centre[0] - half * span_x,
        miny=centre[1] - half * span_y,
        minz=dataset.minz,
        maxx=centre[0] + half * span_x,
        maxy=centre[1] + half * span_y,
        maxz=dataset.maxz,
    )


def minimum_halves(boxes: list[tuple[float, float, float, float]],
                   centre: tuple[float, float], dataset: BBox) -> list[float]:
    """For each row box, the SMALLEST half-extent whose window touches it.

    `window_at(centre, half)` intersects a row exactly when `half` is at
    least this value, because the overlap test is monotone in `half` on
    every axis. Sorting the result once turns `params.rs`'s
    `boxes.iter().filter(intersects).count()` — which the Rust re-runs on
    every one of the 60 bisection steps — into a `bisect_right`, so the
    port stays exact while running in seconds rather than minutes on the
    1M-row package.

    The overlap test is the Rust's own closed-interval one
    (`params.rs::intersects`: reject only on `row.max < window.min` or
    `row.min > window.max`), which is also what every SQL system's
    `>=`/`<=` predicate applies.
    """
    span_x = dataset.maxx - dataset.minx
    span_y = dataset.maxy - dataset.miny
    cx, cy = centre
    halves: list[float] = []
    for minx, miny, maxx, maxy in boxes:
        # A zero span means every row shares that coordinate, so no
        # half-extent on that axis can ever exclude one: contribute nothing.
        hx = 0.0 if span_x == 0.0 else max(
            (cx - maxx) / span_x, (minx - cx) / span_x
        )
        hy = 0.0 if span_y == 0.0 else max(
            (cy - maxy) / span_y, (miny - cy) / span_y
        )
        halves.append(max(0.0, hx, hy))
    halves.sort()
    return halves


def window_for_target(boxes: list[tuple[float, float, float, float]],
                      dataset: BBox, target: float, tag: str) -> BboxWindow:
    """A window intersecting `target` (a fraction in `(0, 1]`) of `boxes`.

    Centred on the MEDIAN row centre so it lands where the data is rather
    than at a bounding-box corner, and sized by bisection on the half-extent
    — the port of `params.rs::window_for_target`, step for step, including
    its choice between the two ends of the final bracket and its `approx`
    flag. The half-extent scales with each axis's own span, so a long, thin
    tile receives a long, thin window instead of a square one that misses on
    the short axis.

    A target that cannot be reached — 1 % of a 10-row dataset is 0.1 rows —
    returns the nearest achievable window with `approx` set, never a
    silently missed target and never an empty window.
    """
    total = len(boxes)
    if total == 0:
        raise ValueError("window_for_target needs at least one row box")

    centre = (
        median_of([(b[0] + b[2]) / 2.0 for b in boxes]),
        median_of([(b[1] + b[3]) / 2.0 for b in boxes]),
    )
    return window_from_halves(
        minimum_halves(boxes, centre, dataset), centre, dataset, target, tag
    )


def window_from_halves(halves: list[float], centre: tuple[float, float],
                       dataset: BBox, target: float, tag: str) -> BboxWindow:
    """`window_for_target`'s search, over a pre-sorted `minimum_halves` list.

    Split out so a caller that derived the halves in SQL (see
    `params.derive`, which must not pull a million bbox structs through the
    Python client) runs exactly the same bisection as one that derived them
    in Python.
    """
    total = len(halves)
    if total == 0:
        raise ValueError("window_from_halves needs at least one row box")

    def count_at(half: float) -> int:
        return bisect.bisect_right(halves, half)

    wanted = target * total

    # `hi` must select everything: half = 1.0 spans the full extent either
    # side of the centre, which covers the dataset whatever the centre is.
    lo, hi = 0.0, 1.0
    for _ in range(BBOX_SEARCH_STEPS):
        mid = (lo + hi) / 2.0
        if count_at(mid) < wanted:
            lo = mid
        else:
            hi = mid

    # `hi` is the smallest searched half-extent reaching the target; `lo` the
    # largest falling short. Whichever lands closer to the target wins, but
    # never an empty window — a zero-row window is the defect this construction
    # replaces.
    best, best_count = hi, count_at(hi)
    lo_count = count_at(lo)
    if lo_count > 0 and abs(lo_count / total - target) < abs(best_count / total - target):
        best, best_count = lo, lo_count

    achieved = best_count / total
    return BboxWindow(
        tag=tag,
        target=target,
        achieved=achieved,
        window=window_at(centre, best, dataset),
        approx=abs(achieved - target) > BBOX_TOLERANCE * target,
    )


@dataclass(frozen=True)
class Dataset:
    name: str
    source: Path          # the .city.json / .city.jsonl input
    cityparquet_dir: Path  # the converted CityParquet package
    hilbert_dir: Path      # the Hilbert-ordered CityParquet package

    @staticmethod
    def name_from_path(path: str | Path) -> str:
        """Dataset name = basename minus CityJSON extensions."""
        base = Path(path).name
        for suffix in (".city.jsonl", ".city.json", ".jsonl", ".json"):
            if base.endswith(suffix):
                return base[: -len(suffix)]
        return base


@dataclass(frozen=True)
class AttrFilter:
    """The `attr-filter` predicate, and what it matched when it was derived.

    A port of `params.rs::AttrFilterSpec`, so both families run the same
    predicate on the same dataset. The column is always a member of the
    CityJSON `attributes` map — never a reserved structural column such as
    `object_type`, which the database family used to filter on and which
    FlatCityBuf's attribute index can never cover.
    """

    column: str
    op: str                   # "eq" or "ge"
    eq_value: str | None      # set when op == "eq"
    ge_bound: float | None    # set when op == "ge"
    matched: int
    share: float
    hand_picked: bool

    def notes_tag(self) -> str:
        if self.op == "eq":
            return f"attr={self.column}={self.eq_value}"
        return f"attr={self.column}>={self.ge_bound}"


@dataclass(frozen=True)
class AttrRange:
    """The `attr-range` (CJDB Q1) predicate: `column > threshold`.

    CJDB's own Q1 is `b3_h_dak_max > 20`, which selects 20.5 % of the 3DBAG
    rows. Rather than hard-coding 20 for every corpus, the threshold is the
    column's own `quantile` (0.8), which reproduces roughly that selectivity
    on any dataset. Both the column and the threshold are recorded in the
    params sidecar so a reader can check what was actually asked.
    """

    column: str
    quantile: float
    threshold: float
    matched: int


@dataclass(frozen=True)
class Params:
    """Query parameters derived once and shared by every system verbatim."""

    bbox_full: BBox
    #: The three row-fraction windows, in `BBOX_TARGETS` order.
    windows: tuple[BboxWindow, ...]
    #: The median row-centre the windows are built around; `point-query`'s
    #: degenerate window (CJDB Q3) is this point.
    point_xy: tuple[float, float]
    attr_filter: AttrFilter | None   # None when no attribute supports a predicate
    attr_range: AttrRange | None     # None if the dataset has no numeric attribute
    numeric_column: str | None  # numeric attribute for attr-stats; None if the dataset has none
    target_id: str          # for id-lookup
    total_city_objects: int  # selectivity denominator
    #: Rows carrying a non-NULL `bbox` in the CityParquet package — the
    #: denominator every window's `achieved` fraction is a fraction of.
    window_rows: int

    def window(self, tag: str) -> BboxWindow:
        for candidate in self.windows:
            if candidate.tag == tag:
                return candidate
        raise KeyError(f"no window tagged {tag!r}")


@dataclass(frozen=True)
class Measurement:
    """One system's repeated samples of one scenario.

    ``result_count`` is None only for a row that failed or timed out, where
    there is no answer to report.
    """

    result_count: int | None
    times_s: list[float]
    server_times_s: list[float]      # empty for in-process systems
    peak_rss_bytes: int | None
    peak_heap_bytes: int | None = None
    notes: str = ""


@dataclass(frozen=True)
class IngestResult:
    wall_clock_s: float
    notes: str = ""


@dataclass(frozen=True)
class SizeReport:
    size_bytes: int
    size_bytes_no_index: int | None = None
