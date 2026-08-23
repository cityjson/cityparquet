"""Core value types shared by every part of the harness.

All types are frozen: a benchmark run must not be able to mutate the
parameters half way through, or two systems could silently be asked
different questions.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class BBox:
    minx: float
    miny: float
    minz: float
    maxx: float
    maxy: float
    maxz: float

    def window(self, area_fraction: float) -> BBox:
        """A sub-window covering ``area_fraction`` of this bbox's x/y area.

        Anchored at the lower-left corner, matching the construction the
        existing cityparquet-rs harness uses, so selectivity tags stay
        comparable across the two harnesses. The z range is never narrowed:
        the window is 2D, so every object is in range vertically.
        """
        side = math.sqrt(area_fraction)
        return BBox(
            minx=self.minx,
            miny=self.miny,
            minz=self.minz,
            maxx=self.minx + (self.maxx - self.minx) * side,
            maxy=self.miny + (self.maxy - self.miny) * side,
            maxz=self.maxz,
        )

    def as_cli_list(self) -> list[float]:
        """The six numbers `cityparquet-readbench --bbox` expects, in order."""
        return [self.minx, self.miny, self.minz, self.maxx, self.maxy, self.maxz]


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
class Params:
    """Query parameters derived once and shared by every system verbatim."""

    bbox_full: BBox
    attr_column: str        # categorical attribute for attr-filter/project
    attr_eq: str            # the equality value for attr-filter
    numeric_column: str | None  # numeric attribute for attr-stats; None if the dataset has none
    target_id: str          # for id-lookup
    parent_id: str | None   # for hierarchy; None if the dataset has no parent-child pair
    total_city_objects: int  # selectivity denominator


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
