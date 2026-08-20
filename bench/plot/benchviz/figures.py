"""``bench_data.json`` -> static paper figures in ``paper/assets/bench/``.

Five figures, each written as ``.svg`` (Typst primary — it cannot embed PDF)
and ``.png`` at 300 dpi:

``pareto-full-read``, ``pareto-bbox-5pct``, ``heatmap``, ``sizes``,
``compression``.

Everything plotted is a unitless ratio against the CityJSONSeq baseline for the
same (dataset, scenario); lower/left is better everywhere and the baseline sits
at 1x.  Styling follows the Tufte rules used across this project: no top/right
spines, range-framed bottom/left spines, serif titles, sans tick labels, no
gridlines, no matplotlib legends (direct labels plus a key panel instead).

The DESIGN.md honesty rules are carried *inside* the figures, so each one is
readable without its future Typst caption: the 10 ms citation floor is drawn as
a band and prefixes muted values with "~", the grain-incomparable scenarios
carry a dagger, and the duckdb-parquet startup overhead / RSS-metric / codec
level caveats are stated in the figure footers.
"""

from __future__ import annotations

import json
import math
import textwrap
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")

import matplotlib.colors as mcolors
import matplotlib.pyplot as plt
from matplotlib.axes import Axes
from matplotlib.figure import Figure
from matplotlib.patches import Rectangle

from .paths import DEFAULT_DATA_PATH, DEFAULT_FIGURES_DIR

# --------------------------------------------------------------------------
# palette + typography (light only: these go into a print manuscript)
# --------------------------------------------------------------------------

BG = "#fffff8"
INK = "#111111"
INK_2 = "#666666"
INK_3 = "#999999"
AXIS = "#cccccc"
ACCENT = "#e41a1c"
GRAY = "#666666"
FLOOR_BAND = "#e8e4d8"

FS_TITLE = 11.0
FS_SUBTITLE = 7.6
FS_PANEL = 7.4
FS_PANEL_SUB = 5.0
FS_TICK = 6.5
FS_LABEL = 7.0
FS_MARK = 5.4
FS_FOOTER = 5.0

TUFTE_RC = {
    "font.family": "serif",
    "font.serif": ["Palatino", "Palatino Linotype", "Georgia", "DejaVu Serif"],
    "font.sans-serif": ["Helvetica", "Arial", "DejaVu Sans"],
    "font.size": 8,
    "figure.facecolor": BG,
    "figure.dpi": 150,
    "axes.facecolor": BG,
    "axes.edgecolor": AXIS,
    "axes.linewidth": 0.5,
    "axes.labelcolor": INK_2,
    "axes.labelsize": FS_LABEL,
    "axes.titlesize": FS_PANEL,
    "axes.titleweight": "normal",
    "axes.spines.top": False,
    "axes.spines.right": False,
    "axes.grid": False,
    "xtick.color": INK_3,
    "ytick.color": INK_3,
    "xtick.labelsize": FS_TICK,
    "ytick.labelsize": FS_TICK,
    "xtick.direction": "in",
    "ytick.direction": "in",
    "xtick.major.size": 2.0,
    "ytick.major.size": 2.0,
    "xtick.major.width": 0.5,
    "ytick.major.width": 0.5,
    "xtick.minor.size": 0.0,
    "ytick.minor.size": 0.0,
    "lines.linewidth": 0.9,
    "legend.frameon": False,
    "savefig.facecolor": BG,
    "savefig.bbox": "tight",
    "savefig.pad_inches": 0.12,
    "svg.fonttype": "none",
}

# Marker vocabulary (DESIGN.md "Color/markers"): shape carries identity so the
# figures survive greyscale printing and colour-blind readers.
FORMAT_STYLE: dict[str, dict[str, Any]] = {
    "cityparquet": {
        "marker": "o",
        "filled": True,
        "color": ACCENT,
        "code": "CP",
        "label": "cityparquet",
    },
    "cityparquet-hilbert": {
        "marker": "o",
        "filled": False,
        "color": ACCENT,
        "code": "CPh",
        "label": "cityparquet-hilbert",
    },
    "cityjsonseq": {
        "marker": "x",
        "filled": True,
        "color": GRAY,
        "code": "SEQ",
        "label": "cityjsonseq (baseline)",
    },
    "cityjsonseq-gz": {
        "marker": "s",
        "filled": True,
        "color": GRAY,
        "code": "GZ",
        "label": "cityjsonseq-gz",
    },
    "flatcitybuf": {
        "marker": "^",
        "filled": True,
        "color": GRAY,
        "code": "FCB",
        "label": "flatcitybuf",
    },
    "duckdb-parquet": {
        "marker": "D",
        "filled": True,
        "color": GRAY,
        "code": "DDB",
        "label": "duckdb-parquet",
    },
    "citygml": {
        "marker": "*",
        "filled": True,
        "color": GRAY,
        "code": "GML",
        "label": "citygml",
    },
    "cityjson": {
        "marker": "P",
        "filled": True,
        "color": GRAY,
        "code": "CJ",
        "label": "cityjson",
    },
}

# What the views plot: the FORMAT-COMPARISON axis of
# `Format::DEFAULT_SET` (crates/cityparquet-readbench/src/format.rs) — the
# formats a city model can ship as, one tag per family, CityParquet represented
# by the Hilbert-ordered package it would actually ship as.
#
# `cityjsonseq-gz` and `duckdb-parquet` are NOT here on purpose: a compression
# variant and an SQL-engine baseline are not formats, and putting either on a
# format axis answers a different question than the one the figure asks. They
# keep their marker vocabulary for an opt-in run's footnotes.
FORMAT_AXIS = [
    "cityparquet-hilbert",
    "citygml",
    "cityjson",
    "cityjsonseq",
    "flatcitybuf",
]
# Plot order: CityParquet first (it is the subject), the baseline last so its
# cross is drawn on top of the reference lines.
FORMAT_ORDER = [
    "cityparquet-hilbert",
    "citygml",
    "cityjson",
    "flatcitybuf",
    "cityjsonseq",
]
HEATMAP_FORMATS = [
    "cityparquet-hilbert",
    "citygml",
    "cityjson",
    "cityjsonseq",
    "flatcitybuf",
]
SIZE_FORMATS = [
    "cityparquet-hilbert",
    "citygml",
    "cityjson",
    "cityjsonseq",
    "flatcitybuf",
]

# Bar fills for the per-dataset panels. The marker vocabulary in FORMAT_STYLE
# identifies a format by SHAPE, which a filled bar has no room for; these are
# the same idea in the one channel a bar does have. CityParquet keeps the
# accent, and the rest are separated by value rather than by hue so the panels
# stay readable in greyscale and to a colour-blind reader.
FORMAT_GREY = {
    "citygml": "#c6c5bb",
    "cityjson": "#9d9c95",
    "cityjsonseq": "#6b6b66",
    "flatcitybuf": "#6b6b66",
    "cityjsonseq-gz": "#b9b9ae",
    "duckdb-parquet": "#b9b9ae",
}
FORMAT_LABEL = {
    "cityparquet": "CityParquet",
    "cityparquet-hilbert": "CityParquet",
    "citygml": "CityGML",
    "cityjson": "CityJSON",
    "cityjsonseq": "CityJSONSeq",
    "flatcitybuf": "FlatCityBuf",
}

# Small-multiple geometry. One panel per dataset plus one for the key, on a
# 7.1-inch-wide sheet: four columns up to a dozen panels (what the figures were
# drawn at, so a corpus that size keeps its exact layout), five beyond that, and
# five rows at most. A 5x5 sheet holds a 24-dataset corpus with panels that read
# as a pattern rather than as values — exact numbers live in the HTML page's
# per-dataset tables, and the figures say so. Past 25 panels there is nothing
# left to read, so `main` refuses instead of drawing a grey mosaic.
GRID_MAX_ROWS = 5
GRID_MAX_COLS = 5
MAX_PANELS = GRID_MAX_ROWS * GRID_MAX_COLS - 1
MAX_COMPRESSION_PANELS = MAX_PANELS
# Tallest sheet worth printing (inches): roughly a journal page's text height.
MAX_SHEET_HEIGHT = 9.4


def _grid(panels: int, cols_small: int = 4, rows_small: int = 3) -> tuple[int, int]:
    """(rows, cols) for ``panels`` small multiples.

    ``cols_small``/``rows_small`` are the figure's own designed shape, kept
    exactly while the corpus still fits it; a bigger one goes to five columns.
    """
    cols = cols_small if panels <= cols_small * rows_small else GRID_MAX_COLS
    rows = -(-panels // cols)  # ceil
    return rows, cols


def _sheet(
    panels: int,
    row_height: float,
    cols_small: int = 4,
    rows_small: int = 3,
    width: float = 7.1,
) -> tuple[int, int, tuple]:
    """(rows, cols, figsize) — the sheet grows with the corpus, then densifies.

    ``row_height`` is the per-row height the figure was designed at. Rows are
    added until the sheet reaches a printable page, after which the same height
    is shared by more rows: the panels shrink instead of the figure running off
    the paper.
    """
    rows, cols = _grid(panels, cols_small, rows_small)
    return rows, cols, (width, min(MAX_SHEET_HEIGHT, rows * row_height))


def _scale_panel_fonts(rows: int, cols: int) -> None:
    """Shrink the panel-level type for a denser grid than 3x4.

    The panel text sizes are module constants read as globals by every builder
    below (there is one figure set per process, so rebinding them here is the
    whole mechanism). Headline and footer sizes are left alone: they are set
    against the sheet, not the panel.
    """
    global FS_PANEL, FS_PANEL_SUB, FS_TICK, FS_LABEL, FS_MARK
    factor = min(1.0, (4 / cols) ** 0.5 * (3 / rows) ** 0.25)
    FS_PANEL, FS_PANEL_SUB = FS_PANEL * factor, FS_PANEL_SUB * factor
    FS_TICK, FS_LABEL, FS_MARK = FS_TICK * factor, FS_LABEL * factor, FS_MARK * factor
    plt.rcParams.update(
        {
            "axes.labelsize": FS_LABEL,
            "axes.titlesize": FS_PANEL,
            "xtick.labelsize": FS_TICK,
            "ytick.labelsize": FS_TICK,
        }
    )


SCENARIO_ORDER = [
    "full-read",
    "count",
    "bbox-1pct",
    "bbox-5pct",
    "bbox-25pct",
    "attr-filter",
    "attr-stats",
    "id-lookup",
    "project",
]
# Honesty rule 2: these compare feature-grain against CityObject-grain formats.
GRAIN_DAGGER = {"full-read", "count", "bbox-1pct", "bbox-5pct", "bbox-25pct"}

COMPRESSION_CODE = {
    "cityparquet": "def",
    "cityparquet+zstd": "zstd",
    "cityparquet+gzip": "gzip",
    "cityparquet+brotli": "brot",
    "cityparquet+lz4": "lz4",
    "cityparquet+snappy": "snap",
    "cityparquet+uncompressed": "none",
    "cityparquet+rg512": "rg512",
    "cityparquet+rg4096": "rg4k",
}


# --------------------------------------------------------------------------
# data loading + contract checks
# --------------------------------------------------------------------------


class DataContractError(RuntimeError):
    """``bench_data.json`` is missing or disagrees with DESIGN.md."""


def _load(data_path: Path) -> dict[str, Any]:
    if not data_path.exists():
        raise DataContractError(
            f"{data_path} not found - run `python -m benchviz prep` first."
        )
    with data_path.open(encoding="utf-8") as handle:
        data = json.load(handle)

    for key in ("meta", "datasets", "read", "sizes", "compression"):
        if key not in data:
            raise DataContractError(f"bench_data.json lacks the '{key}' key.")
    meta = data["meta"]
    for key in ("baseline", "citation_floor_s", "codec_level_note"):
        if key not in meta:
            raise DataContractError(f"bench_data.json meta lacks '{key}'.")
    if not data["datasets"]:
        raise DataContractError("bench_data.json carries no datasets.")
    required_read = {"dataset", "format", "scenario_key", "time_ratio", "rss_ratio"}
    missing = required_read - set(data["read"][0])
    if missing:
        raise DataContractError(f"read records lack fields: {sorted(missing)}")
    return data


def _present(data: dict[str, Any], candidates: Sequence[str]) -> list[str]:
    """The candidates a run actually measured on the read path, in canonical order.

    A run measures the formats it was asked for, and the corpus run of
    2026-08-17 carried three of the six. Keeping a column or a marker slot for
    the other three fills every panel with dashes; dropping them without a word
    would hide the fact. So the views plot what exists and the footer names what
    does not (`_omitted_note`).
    """
    seen = {r["format"] for r in data["read"] if r["time_ratio"] is not None}
    return [f for f in candidates if f in seen]


def _present_sizes(data: dict[str, Any], candidates: Sequence[str]) -> list[str]:
    seen = {r["format"] for r in data["sizes"] if r["frac_of_baseline"] is not None}
    return [f for f in candidates if f in seen]


def _omitted_note(candidates: Sequence[str], present: Sequence[str]) -> str:
    missing = [f for f in candidates if f not in present]
    if not missing:
        return ""
    return (
        "Not measured in this run, so absent from every panel rather than drawn "
        "empty: " + ", ".join(missing) + "."
    )


def _index_read(rows: Iterable[dict[str, Any]]) -> dict[tuple[str, str, str], dict]:
    return {(r["dataset"], r["scenario_key"], r["format"]): r for r in rows}


# --------------------------------------------------------------------------
# small styling helpers
# --------------------------------------------------------------------------


def _sans(ax: Axes) -> None:
    """Tick labels in sans-serif (rc `font.family` is serif for prose)."""
    for label in list(ax.get_xticklabels()) + list(ax.get_yticklabels()):
        label.set_fontfamily("sans-serif")


def _range_frame(ax: Axes, xs: Sequence[float], ys: Sequence[float]) -> None:
    """Tufte range-frame: spines span only the plotted data."""
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)
    if xs:
        ax.spines["bottom"].set_bounds(min(xs), max(xs))
    if ys:
        ax.spines["left"].set_bounds(min(ys), max(ys))
    ax.tick_params(direction="in", length=2.0, width=0.5)


def _panel_heading(
    ax: Axes,
    title: str,
    subtitle: str,
    title_y: float = 1.20,
    subtitle_y: float = 1.045,
) -> None:
    ax.text(
        0.0,
        title_y,
        title,
        transform=ax.transAxes,
        fontsize=FS_PANEL,
        family="serif",
        color=INK,
        va="bottom",
        ha="left",
    )
    if subtitle:
        ax.text(
            0.0,
            subtitle_y,
            subtitle,
            transform=ax.transAxes,
            fontsize=FS_PANEL_SUB,
            family="serif",
            color=INK_3,
            va="bottom",
            ha="left",
        )


def _ratio_tick(value: float) -> str:
    if value <= 0:
        return ""
    exp = math.log10(value)
    if 0.01 <= value <= 1000:
        text = f"{value:g}"
        return f"{text}×"
    return f"$10^{{{round(exp)}}}$×"


def _log_ticks(lo: float, hi: float, max_ticks: int = 4) -> list[float]:
    lo_e = math.ceil(math.log10(lo))
    hi_e = math.floor(math.log10(hi))
    exps = list(range(lo_e, hi_e + 1))
    if not exps:
        return [1.0]
    step = max(1, math.ceil(len(exps) / max_ticks))
    picked = exps[::step]
    if 0 not in picked and lo <= 1.0 <= hi:
        picked = sorted({*picked, 0})
    return [10.0**e for e in picked]


def _marker_kwargs(fmt: str, size: float = 4.0) -> dict[str, Any]:
    style = FORMAT_STYLE[fmt]
    kwargs: dict[str, Any] = {
        "marker": style["marker"],
        "markersize": size,
        "linestyle": "none",
        "color": style["color"],
        "markeredgewidth": 0.9,
    }
    if style["filled"] and style["marker"] != "x":
        kwargs["markerfacecolor"] = style["color"]
        kwargs["markeredgecolor"] = style["color"]
    else:
        kwargs["markerfacecolor"] = "none"
        kwargs["markeredgecolor"] = style["color"]
    return kwargs


def _wrap(text: str, width: int) -> list[str]:
    flat = " ".join(text.split())
    return textwrap.wrap(flat, width=width)


def _fit_wrap(fig: Figure, text: str, fontsize: float, max_in: float) -> list[str]:
    """Wrap ``text`` to at most ``max_in`` inches, measured with the renderer.

    Guessing character widths is unreliable across fonts, and an over-wide text
    artist silently inflates the ``bbox_inches="tight"`` canvas — which would
    break the A4-friendly figure width.  So measure instead of guess.
    """
    flat = " ".join(text.split())
    if not flat:
        return []
    renderer = fig.canvas.get_renderer()
    probe = fig.text(0, -5, flat, fontsize=fontsize, family="serif")
    width_in = probe.get_window_extent(renderer=renderer).width / fig.dpi
    probe.remove()
    per_char = width_in / max(1, len(flat))
    chars = max(20, int(max_in / per_char))
    return textwrap.wrap(flat, width=chars)


def _footer(fig: Figure, lines: Sequence[str], y: float = 0.008) -> None:
    max_in = fig.get_figwidth() - 0.15
    wrapped: list[str] = []
    for line in lines:
        wrapped.extend(_fit_wrap(fig, line, FS_FOOTER, max_in) or [""])
    fig.text(
        0.012,
        y,
        "\n".join(wrapped),
        fontsize=FS_FOOTER,
        family="serif",
        color=INK_2,
        va="bottom",
        ha="left",
        linespacing=1.55,
    )


def _footer_reserve(fig: Figure, lines: Sequence[str], floor: float) -> float:
    """The bottom margin the footer needs, in figure fractions.

    The footer is anchored to the bottom of the sheet and grows upward, while
    the panel grid grows downward: on a five-row sheet with a footer of derived
    notes the two met, and the last row of panels was printed through. Measuring
    the wrapped footer before laying the panels out is what keeps them apart, at
    any corpus size and any number of notes.
    """
    max_in = fig.get_figwidth() - 0.15
    count = 0
    for line in lines:
        count += len(_fit_wrap(fig, line, FS_FOOTER, max_in) or [""])
    height = count * FS_FOOTER * 1.55 / 72.0 / fig.get_figheight()
    return max(floor, 0.012 + height + 0.022)


def _headline(fig: Figure, title: str, subtitle: str) -> float:
    """Left-aligned finding-asserting title + subtitle; returns its bottom y.

    Callers use the returned figure fraction to place the panel grid, so a
    title that wraps to three lines pushes the grid down instead of colliding
    with it.
    """
    max_in = fig.get_figwidth() - 0.15
    height_pt = fig.get_figheight() * 72
    title_lines = _fit_wrap(fig, title, FS_TITLE, max_in)
    sub_lines = _fit_wrap(fig, subtitle, FS_SUBTITLE, max_in)
    fig.text(
        0.012,
        0.994,
        "\n".join(title_lines),
        fontsize=FS_TITLE,
        family="serif",
        color=INK,
        va="top",
        ha="left",
        linespacing=1.25,
    )
    drop = (len(title_lines) * FS_TITLE * 1.25 + 7) / height_pt
    sub = fig.text(
        0.012,
        0.994 - drop,
        "\n".join(sub_lines),
        fontsize=FS_SUBTITLE,
        family="serif",
        color=INK_2,
        va="top",
        ha="left",
        linespacing=1.4,
    )
    box = sub.get_window_extent(renderer=fig.canvas.get_renderer())
    return box.y0 / (fig.get_figheight() * fig.dpi)


def _blank(ax: Axes) -> Axes:
    """Strip an axes down to a bare drawing surface (used for key panels)."""
    for spine in ax.spines.values():
        spine.set_visible(False)
    ax.set_xticks([])
    ax.set_yticks([])
    ax.set_xlim(0, 1)
    ax.set_ylim(0, 1)
    return ax


def _replace_axes(fig: Figure, ax: Axes) -> Axes:
    """Detach a shared axes from its siblings so it can host the key panel."""
    pos = ax.get_position()
    ax.remove()
    return fig.add_axes(pos)


def _save(fig: Figure, name: str, out_dir: Path) -> list[Path]:
    written = []
    for suffix in (".svg", ".png"):
        path = out_dir / f"{name}{suffix}"
        fig.savefig(path, dpi=300, facecolor=BG)
        written.append(path)
    plt.close(fig)
    return written


def _no_data(ax: Axes, text: str = "no data") -> None:
    ax.text(
        0.5,
        0.5,
        text,
        transform=ax.transAxes,
        fontsize=FS_MARK,
        family="serif",
        color=INK_3,
        ha="center",
        va="center",
        style="italic",
    )


def _place_labels(
    ax: Axes,
    items: Sequence[tuple[float, float, str, str]],
    fontsize: float = FS_MARK,
) -> None:
    """Direct labels with a greedy collision dodge (no legend, ever).

    Boxes are estimated from the character count; exact extents would need a
    renderer pass per candidate and this is close enough at these sizes.
    """
    placed: list[tuple[float, float, float, float]] = []
    offsets: list[tuple[float, float]] = []
    for radius in (5.5, 10.0, 15.0, 21.0, 28.0):
        # up and sideways first: downward labels tend to land on tick labels
        for dx, dy in (
            (1.0, 0.2),
            (-1.0, 0.2),
            (0.15, 1.0),
            (0.9, 0.9),
            (-0.9, 0.9),
            (1.0, -0.9),
            (-1.0, -0.9),
            (0.9, -1.1),
            (-0.9, -1.1),
            (0.15, -1.3),
        ):
            offsets.append((radius * dx, radius * dy))
    # the markers themselves are obstacles: never park a label on a data point
    pad = 4.0 * ax.figure.dpi / 72.0
    for x, y, _text, _color in items:
        try:
            px, py = ax.transData.transform((x, y))
        except (ValueError, OverflowError):
            continue
        if math.isfinite(px) and math.isfinite(py):
            placed.append((px - pad, py - pad, px + pad, py + pad))
    for x, y, text, color in items:
        try:
            px, py = ax.transData.transform((x, y))
        except (ValueError, OverflowError):
            continue
        if not (math.isfinite(px) and math.isfinite(py)):
            continue
        width = 0.58 * fontsize * len(text) * ax.figure.dpi / 72.0
        height = 1.15 * fontsize * ax.figure.dpi / 72.0
        chosen = offsets[0]
        for dx, dy in offsets:
            ox = px + dx * ax.figure.dpi / 72.0
            oy = py + dy * ax.figure.dpi / 72.0
            x0 = ox if dx >= 0 else ox - width
            box = (x0, oy, x0 + width, oy + height)
            clash = any(
                box[0] < other[2]
                and other[0] < box[2]
                and box[1] < other[3]
                and other[1] < box[3]
                for other in placed
            )
            if not clash:
                chosen = (dx, dy)
                placed.append(box)
                break
        else:
            dx, dy = offsets[0]
            ox = px + dx * ax.figure.dpi / 72.0
            oy = py + dy * ax.figure.dpi / 72.0
            x0 = ox if dx >= 0 else ox - width
            placed.append((x0, oy, x0 + width, oy + height))
        far = math.hypot(*chosen) > 12
        ax.annotate(
            text,
            xy=(x, y),
            xytext=chosen,
            textcoords="offset points",
            fontsize=fontsize,
            family="serif",
            color=color,
            ha="left" if chosen[0] >= 0 else "right",
            va="bottom",
            annotation_clip=False,
            arrowprops=(
                dict(arrowstyle="-", color=AXIS, linewidth=0.4, shrinkA=0.5, shrinkB=2.5)
                if far
                else None
            ),
        )


# --------------------------------------------------------------------------
# figure 1 + 2: speed / memory Pareto grids
# --------------------------------------------------------------------------


def _pareto_frontier(points: list[tuple[float, float]]) -> list[tuple[float, float]]:
    """Non-dominated points, minimising both axes, sorted by x ascending."""
    frontier: list[tuple[float, float]] = []
    for point in sorted(points):
        dominated = any(
            other[0] <= point[0] and other[1] <= point[1] and other != point
            for other in points
        )
        if not dominated:
            frontier.append(point)
    deduped: list[tuple[float, float]] = []
    for point in frontier:
        if deduped and point[1] >= deduped[-1][1]:
            continue
        deduped.append(point)
    return deduped


def _pareto_key_panel(ax: Axes, floor_hint: str, order: Sequence[str]) -> None:
    """The key lists the formats this sheet actually plots, in plot order.

    Listing a marker for a format the run never measured invites the reader to
    hunt for it in the panels; the footer names those instead.
    """
    _blank(ax)
    ax.text(
        0.0,
        1.20,
        "How to read",
        transform=ax.transAxes,
        fontsize=FS_PANEL,
        family="serif",
        color=INK,
        va="bottom",
    )
    ax.text(
        0.0,
        1.045,
        "below-left = faster + leaner",
        transform=ax.transAxes,
        fontsize=FS_PANEL_SUB,
        family="serif",
        color=INK_3,
        va="bottom",
    )

    labels = {"cityjsonseq": "cityjsonseq = baseline"}
    rows = [(f, labels.get(f, f)) for f in order]
    top = 0.95
    step = 0.118
    for i, (fmt, text) in enumerate(rows):
        y = top - i * step
        ax.plot([0.06], [y], **_marker_kwargs(fmt, size=4.0))
        ax.text(
            0.18,
            y,
            text,
            fontsize=4.9,
            family="serif",
            color=INK if fmt.startswith("cityparquet") else INK_2,
            va="center",
        )

    y_band = top - len(rows) * step - 0.02
    ax.add_patch(
        Rectangle(
            (0.02, y_band - 0.045),
            0.09,
            0.09,
            facecolor=FLOOR_BAND,
            edgecolor="none",
        )
    )
    ax.text(
        0.18,
        y_band,
        floor_hint,
        fontsize=4.9,
        family="serif",
        color=INK_2,
        va="center",
    )
    y_front = y_band - 0.115
    ax.step(
        [0.03, 0.07, 0.07, 0.11],
        [y_front + 0.035, y_front + 0.035, y_front - 0.02, y_front - 0.02],
        color=INK_3,
        linewidth=0.7,
    )
    ax.text(
        0.18,
        y_front,
        "Pareto frontier",
        fontsize=4.9,
        family="serif",
        color=INK_2,
        va="center",
    )


def pareto(
    data: dict[str, Any],
    scenario: str,
    name: str,
    headline: tuple[str, str],
    out_dir: Path,
) -> list[Path]:
    datasets = data["datasets"]
    index = _index_read(data["read"])
    floor_s = data["meta"]["citation_floor_s"]
    order = _present(data, FORMAT_ORDER)

    xs_all, ys_all = [], []
    for ds in datasets:
        for fmt in order:
            rec = index.get((ds["id"], scenario, fmt))
            if rec and rec["time_ratio"] and rec["rss_ratio"]:
                xs_all.append(rec["time_ratio"])
                ys_all.append(rec["rss_ratio"])
    if not xs_all:
        raise DataContractError(f"no plottable read records for scenario {scenario!r}")

    xlim = (min(xs_all) / 1.7, max(xs_all) * 1.7)
    ylim = (min(ys_all) / 1.35, max(ys_all) * 1.35)

    dagger = "†" if scenario in GRAIN_DAGGER else ""

    rows_n, cols_n, figsize = _sheet(len(datasets) + 1, 6.8 / 3)
    fig, axes = plt.subplots(rows_n, cols_n, figsize=figsize, sharex=True, sharey=True)
    head_bottom = _headline(fig, *headline)
    footer = [
        f"Baseline = cityjsonseq, at (1×, 1×) in every panel; axes share limits "
        f"across panels. Scenario: {scenario}{dagger}. Memory = peak RSS "
        "(platform units cancel in the ratio).",
        "Shaded band = the benchmark's own 10 ms citation floor, ±(10 ms ÷ that "
        "dataset's baseline time): points inside it are indistinguishable from "
        "the baseline, so their horizontal position is noise.",
    ]
    if omitted := _omitted_note(FORMAT_ORDER, order):
        footer.append(omitted)
    if note := _reader_note(order):
        footer.extend(_wrap(note, 150))
    if dagger and (note := _grain_note(data, order)):
        footer.extend(_wrap(note, 150))
    bottom = _footer_reserve(fig, footer, 0.165)
    fig.subplots_adjust(
        left=0.085,
        right=0.988,
        top=min(0.845, head_bottom - 0.055),
        bottom=bottom,
        wspace=0.24,
        hspace=0.72,
    )
    flat = axes.ravel()

    for ax, ds in zip(flat, datasets, strict=False):
        ax.set_xscale("log")
        ax.set_yscale("log")
        ax.set_xlim(*xlim)
        ax.set_ylim(*ylim)
        _panel_heading(ax, ds["id"], ds["subtitle"])

        recs = {
            fmt: index.get((ds["id"], scenario, fmt))
            for fmt in order
        }
        usable = {
            fmt: rec
            for fmt, rec in recs.items()
            if rec and rec["time_ratio"] and rec["rss_ratio"]
        }
        if not usable:
            _no_data(ax)
            for spine in ("bottom", "left"):
                ax.spines[spine].set_visible(False)
            ax.set_xticks([])
            ax.set_yticks([])
            continue

        base = index.get((ds["id"], scenario, "cityjsonseq"))
        base_t = base["time_s"] if base and base.get("time_s") else None
        if base_t:
            half = floor_s / base_t
            ax.axvspan(
                max(xlim[0], 1.0 - half),
                min(xlim[1], 1.0 + half),
                color=FLOOR_BAND,
                linewidth=0,
                zorder=0,
            )
        ax.axvline(1.0, color=AXIS, linewidth=0.4, zorder=1)
        ax.axhline(1.0, color=AXIS, linewidth=0.4, zorder=1)

        points = [(r["time_ratio"], r["rss_ratio"]) for r in usable.values()]
        frontier = _pareto_frontier(points)
        if len(frontier) > 1:
            fx = [p[0] for p in frontier]
            fy = [p[1] for p in frontier]
            ax.step(fx, fy, where="post", color=INK_3, linewidth=0.6, zorder=2)

        for fmt in order:
            rec = usable.get(fmt)
            if not rec:
                continue
            ax.plot(
                [rec["time_ratio"]],
                [rec["rss_ratio"]],
                zorder=4,
                **_marker_kwargs(fmt, size=4.2),
            )

        missing = [f for f in order if f not in usable]
        if missing:
            ax.text(
                0.99,
                0.02,
                "no data: " + ", ".join(FORMAT_STYLE[f]["code"] for f in missing),
                transform=ax.transAxes,
                fontsize=FS_FOOTER,
                family="serif",
                color=INK_3,
                ha="right",
                va="bottom",
            )

        ticks_x = _log_ticks(*xlim)
        ticks_y = _log_ticks(*ylim)
        ax.set_xticks(ticks_x)
        ax.set_xticklabels([_ratio_tick(t) for t in ticks_x])
        ax.set_yticks(ticks_y)
        ax.set_yticklabels([_ratio_tick(t) for t in ticks_y])
        ax.minorticks_off()
        _range_frame(ax, [p[0] for p in points], [p[1] for p in points])
        _sans(ax)

    # direct labels: first data panel only (they would collide everywhere else)
    first_ds = datasets[0]["id"]
    label_items = []
    for fmt in order:
        rec = index.get((first_ds, scenario, fmt))
        if rec and rec["time_ratio"] and rec["rss_ratio"]:
            label_items.append(
                (
                    rec["time_ratio"],
                    rec["rss_ratio"],
                    FORMAT_STYLE[fmt]["code"],
                    ACCENT if fmt.startswith("cityparquet") else INK_2,
                )
            )
    _place_labels(flat[0], label_items)

    # tick labels only on the bottom-most panel of each column
    n = len(datasets)
    for i, ax in enumerate(flat[:n]):
        col = i % cols_n
        below = i + cols_n
        is_bottom = below >= n
        ax.tick_params(labelbottom=is_bottom)
        ax.tick_params(labelleft=(col == 0))

    key_ax = _replace_axes(fig, flat[n])
    _pareto_key_panel(key_ax, "10 ms citation floor", order)
    for ax in flat[n + 1 :]:
        ax.set_visible(False)

    fig.text(
        0.53,
        bottom - 0.047,
        "read time ÷ CityJSONSeq read time (log)",
        fontsize=FS_LABEL,
        family="serif",
        color=INK_2,
        ha="center",
    )
    fig.text(
        0.014,
        0.51,
        "peak RSS ÷ CityJSONSeq peak RSS (log)",
        fontsize=FS_LABEL,
        family="serif",
        color=INK_2,
        rotation=90,
        va="center",
    )

    _footer(fig, footer)
    return _save(fig, name, out_dir)


# --------------------------------------------------------------------------
# figure 3: read speed-up heatmap grid
# --------------------------------------------------------------------------


def _speedup_label(value: float | None, below_floor: bool) -> str:
    if value is None:
        return "–"
    prefix = "≈" if below_floor else ""
    if value >= 1000:
        body = f"{value / 1000:.0f}k"
    elif value >= 10:
        body = f"{value:.0f}"
    elif value >= 1 or value >= 0.1:
        body = f"{value:.1f}"
    elif value >= 0.01:
        body = f"{value:.2f}"
    else:
        return prefix + "<0.01×"
    return f"{prefix}{body}×"


def heatmap(
    data: dict[str, Any], headline: tuple[str, str], out_dir: Path
) -> list[Path]:
    datasets = data["datasets"]
    index = _index_read(data["read"])
    fmts = _present(data, HEATMAP_FORMATS)
    cmap = plt.get_cmap("PRGn")
    vmax = 8.0  # log2 units: colour saturates at 1/256x and 256x
    norm = mcolors.Normalize(-vmax, vmax)

    rows_n, cols_n, figsize = _sheet(
        len(datasets) + 1, 9.1 / 4, cols_small=3, rows_small=4
    )
    fig, axes = plt.subplots(rows_n, cols_n, figsize=figsize)
    head_bottom = _headline(fig, *headline)
    footer = [
        "Every cell is a ratio against the cityjsonseq baseline row for the same "
        "dataset and scenario; the baseline column (SEQ) is 1× by construction. "
        "Colour saturates beyond 1/256× and 256× — read the printed value, "
        "never the colour alone.",
        *_wrap(
            _grain_note(data, fmts)
            + " attr-filter, attr-stats, id-lookup and project are"
            " CityObject-granular in every format and are the comparable rows.",
            150,
        ),
        *_wrap(_reader_note(fmts), 150),
        "id-lookup samples a table-order-first identifier, which favours scanning "
        "formats. An empty cell is a scenario the run did not measure for that "
        "dataset, never a zero.",
        *_wrap(_missing_note(data), 150),
        *_wrap(_omitted_note(HEATMAP_FORMATS, fmts), 150),
    ]
    fig.subplots_adjust(
        left=0.115,
        right=0.99,
        top=min(0.895, head_bottom - 0.075),
        bottom=_footer_reserve(fig, footer, 0.088),
        wspace=0.20,
        hspace=0.46,
    )
    flat = axes.ravel()
    n = len(datasets)

    for i, (ax, ds) in enumerate(zip(flat, datasets, strict=False)):
        col = i % cols_n
        ax.set_xlim(0, len(fmts))
        ax.set_ylim(len(SCENARIO_ORDER), 0)
        ax.set_xticks([])
        ax.set_yticks([])
        for spine in ax.spines.values():
            spine.set_visible(False)
        _panel_heading(
            ax, ds["id"], ds["subtitle"], title_y=1.30, subtitle_y=1.145
        )

        for c, fmt in enumerate(fmts):
            ax.text(
                c + 0.5,
                -0.12,
                FORMAT_STYLE[fmt]["code"],
                fontsize=FS_FOOTER,
                family="sans-serif",
                color=ACCENT if fmt.startswith("cityparquet") else INK_2,
                ha="center",
                va="bottom",
            )
        for r, scenario in enumerate(SCENARIO_ORDER):
            if col == 0:
                mark = "†" if scenario in GRAIN_DAGGER else ""
                ax.text(
                    -0.16,
                    r + 0.5,
                    f"{scenario}{mark}",
                    fontsize=FS_FOOTER,
                    family="serif",
                    color=INK_2,
                    ha="right",
                    va="center",
                )
            for c, fmt in enumerate(fmts):
                rec = index.get((ds["id"], scenario, fmt))
                ratio = rec["time_ratio"] if rec else None
                speedup = 1.0 / ratio if ratio else None
                below = bool(rec and rec.get("below_floor"))
                if speedup is None:
                    face = "#f1f1e8"
                    text_color = INK_3
                    alpha = 1.0
                else:
                    face = cmap(norm(math.log2(speedup)))
                    alpha = 0.42 if below else 1.0
                    strength = abs(math.log2(speedup))
                    text_color = (
                        "#ffffff" if (strength > 4.6 and not below) else "#1a1a1a"
                    )
                ax.add_patch(
                    Rectangle(
                        (c, r),
                        1,
                        1,
                        facecolor=face,
                        alpha=alpha,
                        edgecolor=BG,
                        linewidth=0.6,
                    )
                )
                ax.text(
                    c + 0.5,
                    r + 0.5,
                    _speedup_label(speedup, below),
                    fontsize=4.6,
                    family="sans-serif",
                    color=text_color,
                    ha="center",
                    va="center",
                )

    # key cell: colour ramp + code expansion + symbol glossary
    key_ax = flat[n]
    _blank(key_ax)
    key_ax.text(
        0.0,
        1.30,
        "How to read",
        transform=key_ax.transAxes,
        fontsize=FS_PANEL,
        family="serif",
        color=INK,
        va="bottom",
    )
    key_ax.text(
        0.0,
        1.145,
        "cell = CityJSONSeq time ÷ this format's time",
        transform=key_ax.transAxes,
        fontsize=FS_PANEL_SUB,
        family="serif",
        color=INK_3,
        va="bottom",
    )
    ramp_y, ramp_h = 0.80, 0.075
    steps = 96
    for s in range(steps):
        v = -vmax + 2 * vmax * (s + 0.5) / steps
        key_ax.add_patch(
            Rectangle(
                (s / steps, ramp_y),
                1.0 / steps + 0.002,
                ramp_h,
                facecolor=cmap(norm(v)),
                edgecolor="none",
            )
        )
    for frac, text in [
        (0.0, "1/256×"),
        (0.25, "1/16×"),
        (0.5, "1×"),
        (0.75, "16×"),
        (1.0, "256×"),
    ]:
        key_ax.text(
            frac,
            ramp_y - 0.03,
            text,
            fontsize=4.6,
            family="sans-serif",
            color=INK_2,
            ha="center" if 0 < frac < 1 else ("left" if frac == 0 else "right"),
            va="top",
        )
    key_ax.text(
        0.0,
        ramp_y + ramp_h + 0.025,
        "slower than CityJSONSeq",
        fontsize=4.6,
        family="serif",
        color=INK_2,
        va="bottom",
        ha="left",
    )
    key_ax.text(
        1.0,
        ramp_y + ramp_h + 0.025,
        "faster",
        fontsize=4.6,
        family="serif",
        color=INK_2,
        va="bottom",
        ha="right",
    )

    lines = [("columns, left to right:", INK_2)]
    lines += [
        (f"   {FORMAT_STYLE[f]['code']} = {FORMAT_STYLE[f]['label']}", INK_2)
        for f in fmts
    ]
    lines += [
        ("≈  within the 10 ms citation floor (muted)", INK_2),
        ("–  scenario not run for this dataset", INK_2),
        ("†  grain-incomparable scenario", INK_2),
    ]
    for j, (text, color) in enumerate(lines):
        key_ax.text(
            0.0,
            0.66 - j * 0.062,
            text,
            fontsize=4.7,
            family="serif",
            color=color,
            va="center",
        )
    for ax in flat[n + 1 :]:
        ax.set_visible(False)

    _footer(fig, footer)
    return _save(fig, "heatmap", out_dir)


# --------------------------------------------------------------------------
# figure 4: on-disk size grid
# --------------------------------------------------------------------------


def sizes(
    data: dict[str, Any], headline: tuple[str, str], out_dir: Path
) -> list[Path]:
    datasets = data["datasets"]
    by_dataset: dict[str, dict[str, float]] = {}
    for row in data["sizes"]:
        frac = row.get("frac_of_baseline")
        if frac is None:
            continue
        by_dataset.setdefault(row["dataset"], {})[row["format"]] = frac
    if not by_dataset:
        raise DataContractError("bench_data.json carries no usable size rows.")

    size_fmts = _present_sizes(data, SIZE_FORMATS)
    fracs = [v for row in by_dataset.values() for v in row.values()]
    # A LOG axis with the bars growing OUT OF the 1x baseline, not out of zero.
    # The axis spans CityParquet at ~0.3x and CityGML at up to ~25x of the same
    # bytes: on a linear 0-to-max axis the whole CityParquet series — the subject
    # of the figure — collapses into a sliver against the panel edge, and the
    # tick labels of a 0/0.5/1 scale overprint each other. Anchoring at 1x also
    # matches what a ratio bar means: distance from the baseline, left or right.
    xlo = min(min(fracs) / 1.6, 0.5)
    xhi = max(max(fracs) * 1.6, 2.0)

    rows_n, cols_n, figsize = _sheet(len(datasets) + 1, 6.0 / 3)
    fig, axes = plt.subplots(rows_n, cols_n, figsize=figsize, sharex=True)
    head_bottom = _headline(fig, *headline)
    footer = [
        "Baseline = the uncompressed CityJSONSeq artefact for the same dataset "
        "(1×); bars grow out of that reference line — left is smaller on disk, "
        "right is larger. Shared logarithmic x scale across all panels.",
        "duckdb-parquet is absent: it reads the CityParquet artefact rather than "
        "writing one of its own, so it has no size to report. Sizes are a pure "
        "artefact property — no timing, so the 10 ms citation floor does not "
        "apply here.",
        *_wrap(_omitted_note(SIZE_FORMATS, size_fmts), 150),
    ]
    bottom = _footer_reserve(fig, footer, 0.155)
    fig.subplots_adjust(
        left=0.075,
        right=0.99,
        top=min(0.845, head_bottom - 0.055),
        bottom=bottom,
        wspace=0.36,
        hspace=0.80,
    )
    flat = axes.ravel()
    n = len(datasets)

    for ax, ds in zip(flat, datasets, strict=False):
        _panel_heading(ax, ds["id"], ds["subtitle"])
        rows = by_dataset.get(ds["id"], {})
        entries = sorted(
            ((f, rows[f]) for f in size_fmts if f in rows), key=lambda kv: kv[1]
        )
        ax.set_xscale("log")
        ax.set_xlim(xlo, xhi)
        if not entries:
            _no_data(ax)
            for spine in ax.spines.values():
                spine.set_visible(False)
            ax.set_xticks([])
            ax.set_yticks([])
            continue

        ys = list(range(len(entries)))[::-1]
        for y, (fmt, frac) in zip(ys, entries, strict=True):
            if fmt == "cityparquet":
                color, alpha = ACCENT, 1.0
            elif fmt == "cityparquet-hilbert":
                color, alpha = ACCENT, 0.5
            else:
                color, alpha = "#b9b9ae", 1.0
            left, width = (frac, 1.0 - frac) if frac < 1.0 else (1.0, frac - 1.0)
            ax.barh(
                [y],
                [width],
                left=[left],
                height=0.62,
                color=color,
                alpha=alpha,
                linewidth=0,
            )
            smaller = frac < 1.0
            ax.text(
                frac / 1.12 if smaller else frac * 1.12,
                y,
                f"{frac:.2f}×",
                fontsize=FS_MARK * 0.9,
                family="sans-serif",
                color=INK_2,
                va="center",
                ha="right" if smaller else "left",
            )
        ax.axvline(1.0, color=AXIS, linewidth=0.5, zorder=0)
        ax.set_yticks(ys)
        ax.set_yticklabels([FORMAT_STYLE[f]["code"] for f in (e[0] for e in entries)])
        ax.set_ylim(-0.8, len(entries) - 0.2)
        ticks = _log_ticks(xlo, xhi)
        ax.set_xticks(ticks)
        ax.set_xticklabels([_ratio_tick(t) for t in ticks])
        ax.minorticks_off()
        _range_frame(ax, [xlo, xhi], [])
        # the bar labels are the y axis here: no left spine, no y tick marks
        ax.spines["left"].set_visible(False)
        ax.tick_params(axis="y", length=0)
        for label, (fmt, _frac) in zip(ax.get_yticklabels(), entries, strict=True):
            label.set_fontsize(FS_MARK * 0.9)
            label.set_color(ACCENT if fmt.startswith("cityparquet") else INK_2)
        _sans(ax)

    for i, ax in enumerate(flat[:n]):
        ax.tick_params(labelbottom=(i + cols_n) >= n)

    key_ax = _replace_axes(fig, flat[n])
    _blank(key_ax)
    key_ax.text(
        0.0,
        1.20,
        "How to read",
        transform=key_ax.transAxes,
        fontsize=FS_PANEL,
        family="serif",
        color=INK,
        va="bottom",
    )
    key_ax.text(
        0.0,
        1.045,
        "bar = on-disk bytes ÷ CityJSONSeq bytes",
        transform=key_ax.transAxes,
        fontsize=FS_PANEL_SUB,
        family="serif",
        color=INK_3,
        va="bottom",
    )
    key_rows = [
        (f, f"{FORMAT_STYLE[f]['code']} = {FORMAT_STYLE[f]['label']}")
        for f in size_fmts
    ]
    for j, (fmt, text) in enumerate(key_rows):
        y = 0.88 - j * 0.15
        if fmt == "cityparquet":
            color, alpha = ACCENT, 1.0
        elif fmt == "cityparquet-hilbert":
            color, alpha = ACCENT, 0.5
        else:
            color, alpha = "#b9b9ae", 1.0
        key_ax.add_patch(
            Rectangle((0.0, y - 0.035), 0.10, 0.07, facecolor=color, alpha=alpha)
        )
        key_ax.text(
            0.14,
            y,
            text,
            fontsize=4.9,
            family="serif",
            color=INK if fmt.startswith("cityparquet") else INK_2,
            va="center",
        )
    key_ax.text(
        0.0,
        0.10,
        "bars sorted smallest first;\nthe 1× rule is CityJSONSeq",
        fontsize=4.7,
        family="serif",
        color=INK_2,
        va="center",
        linespacing=1.5,
    )
    for ax in flat[n + 1 :]:
        ax.set_visible(False)

    fig.text(
        0.53,
        bottom - 0.05,
        "on-disk size ÷ CityJSONSeq size",
        fontsize=FS_LABEL,
        family="serif",
        color=INK_2,
        ha="center",
    )
    _footer(fig, footer)
    return _save(fig, "sizes", out_dir)


# --------------------------------------------------------------------------
# figure 5: compression variant grid (deliberately plainer / de-emphasised)
# --------------------------------------------------------------------------


def compression(
    data: dict[str, Any], headline: tuple[str, str], out_dir: Path
) -> list[Path]:
    order = [d["id"] for d in data["datasets"]]
    rows_by_ds: dict[str, list[dict[str, Any]]] = {}
    for row in data["compression"]:
        rows_by_ds.setdefault(row["dataset"], []).append(row)
    populated = [d for d in order if rows_by_ds.get(d)]
    if not populated:
        raise DataContractError("bench_data.json carries no compression rows.")

    failed = {
        gap["dataset"]
        for gap in data.get("compression_gaps", [])
        if "roundtrip" in gap.get("issue", "")
    }
    for ds, rows in rows_by_ds.items():
        if all(r.get("roundtrip") is False for r in rows):
            failed.add(ds)

    xs = [r["write_ratio"] for r in data["compression"] if r.get("write_ratio")]
    ys = [r["size_ratio"] for r in data["compression"] if r.get("size_ratio")]
    xlim = (min(xs) * 0.82, max(xs) * 1.22)
    ylim = (min(ys) * 0.88, max(ys) * 1.5)

    rows_n, cols_n, figsize = _sheet(
        len(populated) + 1, 6.3 / 3, cols_small=3, rows_small=3
    )
    fig, axes = plt.subplots(rows_n, cols_n, figsize=figsize, sharex=True, sharey=True)
    head_bottom = _headline(fig, *headline)
    footer = _wrap(data["meta"]["codec_level_note"], 150)
    gaps = data.get("compression_gaps", [])
    if gaps:
        footer += _wrap(
            "Flagged in this run: "
            + "; ".join(f"{g['dataset']} — {g['issue']}" for g in gaps)
            + ". A failed round-trip is drawn grey and badged; a dataset with no "
            "rows has no panel. Both are named here rather than dropped silently.",
            150,
        )
    bottom = _footer_reserve(fig, footer, 0.165)
    fig.subplots_adjust(
        left=0.085,
        right=0.99,
        top=min(0.855, head_bottom - 0.055),
        bottom=bottom,
        wspace=0.20,
        hspace=0.55,
    )
    flat = axes.ravel()
    n = len(populated)

    for ax, ds_id in zip(flat, populated, strict=False):
        rows = rows_by_ds[ds_id]
        grayed = ds_id in failed
        ax.set_yscale("log")
        ax.set_xlim(*xlim)
        ax.set_ylim(*ylim)
        _panel_heading(ax, ds_id, "")
        ax.axvline(1.0, color=AXIS, linewidth=0.4, zorder=0)
        ax.axhline(1.0, color=AXIS, linewidth=0.4, zorder=0)

        base_color = "#c8c8bf" if grayed else GRAY
        label_color = "#c8c8bf" if grayed else INK_2
        items = []
        for row in rows:
            wx, sy = row.get("write_ratio"), row.get("size_ratio")
            if not wx or not sy:
                continue
            kind = row.get("kind")
            code = COMPRESSION_CODE.get(row["variant"], row["variant"].split("+")[-1])
            if kind == "default":
                ax.plot(
                    [wx],
                    [sy],
                    marker="x",
                    markersize=4.4,
                    markeredgewidth=0.9,
                    linestyle="none",
                    color=base_color,
                    zorder=3,
                )
            elif kind == "rowgroup":
                ax.plot(
                    [wx],
                    [sy],
                    marker="o",
                    markersize=3.8,
                    markerfacecolor="none",
                    markeredgecolor=base_color,
                    markeredgewidth=0.8,
                    linestyle="none",
                    zorder=3,
                )
            else:
                ax.plot(
                    [wx],
                    [sy],
                    marker="o",
                    markersize=3.4,
                    markerfacecolor=base_color,
                    markeredgecolor=base_color,
                    linestyle="none",
                    zorder=3,
                )
            items.append((wx, sy, code, label_color))
        _place_labels(ax, items, fontsize=4.5)

        if grayed:
            ax.text(
                0.5,
                0.5,
                "roundtrip FAILED\n— not citable",
                transform=ax.transAxes,
                fontsize=5.4,
                family="serif",
                color="#8c8c84",
                ha="center",
                va="center",
            )

        ax.set_xticks([0.5, 1.0, 1.5, 2.0])
        ax.set_xticklabels(["0.5×", "1×", "1.5×", "2×"])
        yticks = [1, 2, 4, 8]
        ax.set_yticks(yticks)
        ax.set_yticklabels([f"{t}×" for t in yticks])
        ax.minorticks_off()
        _range_frame(ax, [it[0] for it in items], [it[1] for it in items])
        _sans(ax)

    for i, ax in enumerate(flat[:n]):
        ax.tick_params(labelbottom=(i + cols_n) >= n, labelleft=(i % cols_n == 0))

    key_ax = _replace_axes(fig, flat[n]) if n < len(flat) else None
    if key_ax is not None:
        _blank(key_ax)
        key_ax.text(
            0.0,
            1.20,
            "How to read",
            transform=key_ax.transAxes,
            fontsize=FS_PANEL,
            family="serif",
            color=INK,
            va="bottom",
        )
        key_ax.text(
            0.0,
            1.045,
            "vs each dataset's default CityParquet write",
            transform=key_ax.transAxes,
            fontsize=FS_PANEL_SUB,
            family="serif",
            color=INK_3,
            va="bottom",
        )
        entries = [
            ("x", True, "def = default recipe, at (1×, 1×)"),
            ("o", True, "codec: gzip, brot(li), lz4, snap(py), none"),
            ("o", False, "row group: rg512, rg4k (not a codec)"),
        ]
        for j, (marker, filled, text) in enumerate(entries):
            y = 0.88 - j * 0.16
            key_ax.plot(
                [0.05],
                [y],
                marker=marker,
                markersize=3.8,
                markerfacecolor=GRAY if filled else "none",
                markeredgecolor=GRAY,
                markeredgewidth=0.8,
                linestyle="none",
            )
            key_ax.text(
                0.14, y, text, fontsize=4.7, family="serif", color=INK_2, va="center"
            )
        gap_lines = [
            f"{gap['dataset']}: {gap['issue']}"
            for gap in data.get("compression_gaps", [])
        ]
        for j, text in enumerate(gap_lines):
            for k, part in enumerate(_wrap(text, 46)):
                key_ax.text(
                    0.0,
                    0.36 - (j * 2 + k) * 0.085,
                    part,
                    fontsize=4.5,
                    family="serif",
                    color=INK_3,
                    va="center",
                )
        for ax in flat[n + 1 :]:
            ax.set_visible(False)

    fig.text(
        0.53,
        bottom - 0.053,
        "write time ÷ default write time",
        fontsize=FS_LABEL,
        family="serif",
        color=INK_2,
        ha="center",
    )
    fig.text(
        0.014,
        0.53,
        "total bytes ÷ default bytes (log)",
        fontsize=FS_LABEL,
        family="serif",
        color=INK_2,
        rotation=90,
        va="center",
    )

    _footer(fig, footer)
    return _save(fig, "compression", out_dir)


# --------------------------------------------------------------------------
# entry point
# --------------------------------------------------------------------------


# --------------------------------------------------------------------------
# headline sentences, computed from the data they describe
# --------------------------------------------------------------------------
#
# Every number and every comparative word below is derived from the run being
# plotted. The previous edition typed them by hand, which held exactly until the
# next benchmark run: the figures then asserted one corpus's findings over
# another corpus's marks, with nothing in the code to notice.


def _primary_cityparquet(data: dict[str, Any]) -> str:
    """The CityParquet series the sentences are about.

    The Hilbert-ordered package where a run carries it: that is the
    configuration `Format::DEFAULT_SET` puts on the format axis, so that the
    format comparison is not handicapped by an ordering choice no other format
    faces. A run that measured only the source-ordered package (an
    ordering-comparison run) falls back to it.
    """
    present = {r["format"] for r in data["read"] if r["time_ratio"] is not None}
    if "cityparquet-hilbert" in present:
        return "cityparquet-hilbert"
    return "cityparquet"


def _stats(data: dict[str, Any], scenario: str, fmt: str) -> dict[str, Any] | None:
    rows = [
        r
        for r in data["read"]
        if r["scenario_key"] == scenario and r["format"] == fmt
    ]
    times = [r["time_ratio"] for r in rows if r["time_ratio"]]
    rss = [r["rss_ratio"] for r in rows if r["rss_ratio"]]
    if not times:
        return None
    return {
        "n": len(times),
        "time": _median(times),
        "rss": _median(rss) if rss else None,
        "faster": sum(1 for t in times if t < 1.0),
        "leaner": sum(1 for r in rss if r < 1.0),
        "n_rss": len(rss),
        "floored": sum(1 for r in rows if r.get("below_floor")),
    }


def _median(values: Sequence[float]) -> float:
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def _ratio_word(ratio: float) -> str:
    """Faster / at parity / slower, with the same 10 % band in both directions."""
    if ratio < 0.9:
        return "faster than"
    if ratio > 1.1:
        return "slower than"
    return "at parity with"


def _times(ratio: float) -> str:
    if ratio >= 10:
        return f"{ratio:.0f}×"
    if ratio >= 1:
        return f"{ratio:.2f}×"
    if ratio >= 0.01:
        return f"{ratio:.3f}×"
    return f"{ratio:.4f}×"


def _size_median(data: dict[str, Any], fmt: str) -> tuple[float, float, float] | None:
    fracs = [
        r["frac_of_baseline"]
        for r in data["sizes"]
        if r["format"] == fmt and r["frac_of_baseline"]
    ]
    if not fracs:
        return None
    return _median(fracs), min(fracs), max(fracs)


def _corpus_phrase(data: dict[str, Any]) -> str:
    return f"{len(data['datasets'])} corpus dataset" + (
        "s" if len(data["datasets"]) != 1 else ""
    )


def _density_note(data: dict[str, Any]) -> str:
    """Said on the sheet when the panels have shrunk past reading individual marks."""
    rows, cols = _grid(len(data["datasets"]) + 1)
    if cols <= 4 and rows <= 3:
        return ""
    return (
        f"At {len(data['datasets'])} datasets the panels are meant to be read as a "
        "pattern; exact per-dataset values are in the HTML summary page's tables."
    )


def _missing_note(data: dict[str, Any]) -> str:
    """Which (scenario, dataset) pairs the run never measured — a source fact."""
    covered: dict[str, set[str]] = {}
    for r in data["read"]:
        covered.setdefault(r["scenario_key"], set()).add(r["dataset"])
    ids = [d["id"] for d in data["datasets"]]
    missing = {
        sc: [i for i in ids if i not in covered.get(sc, set())] for sc in SCENARIO_ORDER
    }
    parts = [
        f"{sc} for {len(names)} of {len(ids)} datasets"
        for sc, names in missing.items()
        if names
    ]
    if not parts:
        return "Every scenario was measured for every dataset in this corpus."
    return "Not measured in this run: " + "; ".join(parts) + "."


def _grain_note(data: dict[str, Any], present: Sequence[str]) -> str:
    """Which of the plotted formats count features and which count CityObjects.

    Straight from READ_BENCHMARK.md fairness caveat 1's own table (carried in
    `meta`), because the split is per format and changes with the axis: naming
    the two sides by hand is how a footnote came to cite `cityjsonseq-gz`, a
    format the views no longer plot.
    """
    meta = data["meta"]
    feature = [f for f in present if f in meta.get("feature_grain_formats", [])]
    objects = [f for f in present if f in meta.get("object_grain_formats", [])]
    if not feature or not objects:
        return ""
    return (
        "† grain-incomparable in this scenario: "
        + ", ".join(feature)
        + " count top-level features (children inline) while "
        + ", ".join(objects)
        + " count one row per CityObject, so the two sides do not do identical work."
    )


def _reader_note(present: Sequence[str]) -> str:
    """The citygml row's own caveat, said on any sheet that plots it."""
    if "citygml" not in present:
        return ""
    return (
        "The citygml row measures THIS repository's CityGML reader, not CityGML's "
        "ceiling: the constant factor is ours, the linear term is the format's — a "
        "published .gml carries no index, so any reader must traverse the document "
        "(caveat 12)."
    )


def _pareto_headline(
    data: dict[str, Any], scenario: str, label: str
) -> tuple[str, str]:
    cp = _primary_cityparquet(data)
    st = _stats(data, scenario, cp)
    if st is None:
        return (
            f"{label}: no CityParquet measurements in this run",
            f"Reading {label.lower()} across {_corpus_phrase(data)}.",
        )
    others = [
        (f, s)
        for f in FORMAT_ORDER
        if f not in (cp, "cityjsonseq") and (s := _stats(data, scenario, f))
    ]
    best = min(others, key=lambda kv: kv[1]["time"], default=None)
    title = (
        f"{label}: {FORMAT_STYLE[cp]['label']} is {_ratio_word(st['time'])} "
        f"CityJSONSeq at a median {_times(st['time'])} of its time"
    )
    if st["rss"]:
        title += f", using {_times(st['rss'])} its peak RSS"
    subtitle = (
        f"{_corpus_phrase(data)}, each panel one dataset against its own CityJSONSeq "
        f"baseline at (1×, 1×). Faster in {st['faster']} of {st['n']} datasets"
    )
    if st["n_rss"]:
        subtitle += f" and leaner in {st['leaner']} of {st['n_rss']}"
    if best is not None:
        subtitle += (
            f". The frontier's lower-left corner is held by {best[0]} "
            f"(median {_times(best[1]['time'])})"
        )
    if st["floored"]:
        subtitle += (
            f". {st['floored']} of this format's time deltas fall inside the 10 ms "
            "noise floor and are drawn hollow"
        )
    note = _density_note(data)
    return title + ".", subtitle + "." + (f" {note}" if note else "")


def _heatmap_headline(data: dict[str, Any]) -> tuple[str, str]:
    cp = _primary_cityparquet(data)
    selective = [
        s["time"]
        for sc in SCENARIO_ORDER
        if sc != "full-read" and (s := _stats(data, sc, cp))
    ]
    full = _stats(data, "full-read", cp)
    formats = len({r["format"] for r in data["read"]})
    scenarios = len({r["scenario_key"] for r in data["read"]})
    if selective and full:
        title = (
            "Where columnar layout pays: the selective scenarios run at a median "
            f"{_times(_median(selective))} of the baseline's time, full "
            f"materialisation at {_times(full['time'])}"
        )
    else:
        title = "Read speed-up over the CityJSONSeq baseline, by scenario and format"
    subtitle = (
        f"Speed-up (1 ÷ time ratio) for {scenarios} scenarios × {formats} formats × "
        f"{_corpus_phrase(data)}. Green beats the baseline, purple loses to it; the "
        "printed value is the datum and colour is only a second reading."
    )
    note = _density_note(data)
    return title + ".", subtitle + (f" {note}" if note else "")


def _sizes_headline(data: dict[str, Any]) -> tuple[str, str]:
    primary = _primary_cityparquet(data)
    cp = _size_median(data, primary) or _size_median(data, "cityparquet")
    if cp is None:
        return (
            "On-disk footprint against the CityJSONSeq baseline",
            f"{_corpus_phrase(data)}; no CityParquet artefact was sized in this run.",
        )
    median, lo, hi = cp
    title = (
        f"CityParquet stores a city model in a median {_times(median)} of the "
        f"CityJSONSeq bytes (range {_times(lo)}–{_times(hi)})"
    )
    parts = [
        f"Fraction of the uncompressed CityJSONSeq artefact, {_corpus_phrase(data)}, "
        "sorted per panel, shorter is smaller."
    ]
    others = [
        (f, _size_median(data, f))
        for f in SIZE_FORMATS
        if f not in (primary, "cityjsonseq")
    ]
    stated = [f"{f} at a median {_times(m[0])}" for f, m in others if m]
    if stated:
        parts.append("The other formats on the axis: " + "; ".join(stated) + ".")
    note = _density_note(data)
    return title + ".", " ".join(parts) + (f" {note}" if note else "")


def _compression_headline(data: dict[str, Any]) -> tuple[str, str]:
    rows = data["compression"]
    sizes = [r["size_ratio"] for r in rows if r.get("size_ratio")]
    writes = [r["write_ratio"] for r in rows if r.get("write_ratio")]
    datasets = len({r["dataset"] for r in rows})
    title = (
        "Compression variants move size far more than write time — and none of it "
        "is a citable codec ranking"
    )
    if sizes and writes:
        title = (
            f"Compression variants move size across {_times(min(sizes))}–"
            f"{_times(max(sizes))} of the default write's bytes for "
            f"{_times(min(writes))}–{_times(max(writes))} of its time — and none of "
            "it is a citable codec ranking"
        )
    subtitle = (
        f"{datasets} dataset(s) measured, each variant against that dataset's own "
        "default CityParquet write at (1×, 1×). Codecs are filled markers, row-group "
        "variants open ones. The codec levels are not matched, so this figure is "
        "exploratory."
    )
    return title + ".", subtitle


# --------------------------------------------------------------------------
# figure 6: per-dataset format comparison (time beside memory)
# --------------------------------------------------------------------------

# Scenarios drawn per dataset panel. The 1 % and 25 % windows are omitted
# deliberately: across this corpus they land within the citation floor of the
# 5 % window on almost every dataset, so drawing all three spends two rows per
# panel restating one result. The HTML page keeps all three.
PANEL_SCENARIOS = [
    "full-read",
    "count",
    "bbox-5pct",
    "attr-filter",
    "attr-stats",
    "project",
    "id-lookup",
]
SCENARIO_LABEL = {
    "full-read": "full read",
    "count": "count",
    "bbox-1pct": "spatial 1%",
    "bbox-5pct": "spatial 5%",
    "bbox-25pct": "spatial 25%",
    "attr-filter": "attr filter",
    "attr-stats": "attr stats",
    "project": "projection",
    "id-lookup": "id lookup",
}
# Panels on the per-dataset sheet. Four is what the sheet's two-axis panels fit
# at a readable size; the selection below is derived, never named.
FORMAT_PANELS = 4
# A dataset with fewer CityObjects than this cannot exercise a selective query:
# every filter matches all of it or none of it, so its ratios carry no signal
# (READ_BENCHMARK.md makes the same point about its one-object tile). Such
# datasets are excluded from the panel PICK, never from the data.
DEGENERATE_OBJECTS = 100


def _panel_pick(datasets: Sequence[dict[str, Any]], count: int) -> list[dict]:
    """`count` datasets spread across the corpus by CityObject count.

    `datasets` arrives sorted by object count descending. Taking evenly spaced
    positions out of it gives the largest, the smallest non-degenerate, and an
    even spread between — a selection that follows whatever corpus was measured
    instead of naming datasets this package must not know about.
    """
    pool = [d for d in datasets if (d.get("objects") or 0) >= DEGENERATE_OBJECTS]
    if not pool:
        pool = list(datasets)
    if len(pool) <= count:
        return pool
    step = (len(pool) - 1) / (count - 1)
    return [pool[round(i * step)] for i in range(count)]


def _axis_label(ax: Axes, text: str) -> None:
    ax.set_xlabel(text, fontsize=FS_LABEL, family="serif", color=INK_2)


def _ratio_bar(ax: Axes, y: float, value: float, lo: float, hi: float, **kw) -> None:
    """One bar from the 1x rule to `value` on a log axis, clamped to the panel.

    The anchor is 1x rather than zero because the quantity is a ratio: on this
    corpus a single scenario spans five orders of magnitude between formats, so
    a zero-anchored linear bar renders every format but the fastest as an
    invisible sliver. Anchored at the reference, a bar's length is its distance
    from the baseline and its direction is better-or-worse.
    """
    x = min(max(value, lo), hi)
    left, width = (1.0, x - 1.0) if x >= 1.0 else (x, 1.0 - x)
    ax.barh([y], [max(width, 1e-9)], left=[left], height=0.78, **kw)


def _format_panel_axis(ax: Axes, lo: float, hi: float, rows: int) -> None:
    ax.set_xscale("log")
    ax.set_xlim(lo, hi)
    ax.set_ylim(-0.8, rows - 0.2)
    ax.invert_yaxis()
    ax.axvline(1.0, color=GRAY, lw=0.7, zorder=1)
    for side in ("top", "right", "left"):
        ax.spines[side].set_visible(False)
    ax.set_yticks([])
    ticks = _log_ticks(lo, hi, 5)
    ax.set_xticks(ticks)
    ax.set_xticklabels([_ratio_tick(t) for t in ticks])
    # A narrow log span leaves matplotlib drawing MINOR ticks, which it labels
    # with its own "6 x 10^-1" formatter rather than this package's ratio one.
    ax.minorticks_off()
    _sans(ax)


def formats(
    data: dict[str, Any], headline: tuple[str, str], out_dir: Path
) -> list[Path]:
    """Read time and peak memory per dataset, per scenario, per format."""
    index = _index_read(data["read"])
    fmts = [f for f in FORMAT_AXIS if f != "cityjsonseq"]
    picked = _panel_pick(data["datasets"], FORMAT_PANELS)
    if not picked:
        raise DataContractError("bench_data.json carries no datasets to draw.")

    times, mems = [], []
    for ds in picked:
        for scenario in PANEL_SCENARIOS:
            for fmt in fmts:
                rec = index.get((ds["id"], scenario, fmt))
                if not rec:
                    continue
                if rec.get("time_ratio"):
                    times.append(1.0 / rec["time_ratio"])
                if rec.get("rss_ratio"):
                    mems.append(1.0 / rec["rss_ratio"])
    if not times:
        raise DataContractError("bench_data.json carries no usable read ratios.")
    tlo, thi = min(min(times) / 2, 0.5), max(max(times) * 2, 2.0)
    mlo, mhi = min(min(mems) / 2, 0.5), max(max(mems) * 2, 2.0)

    # One dataset per ROW, two axes wide. A two-up grid fits the sheet, but the
    # scenario labels and the baseline's own seconds both live left of the time
    # axis, and only a first-column panel has a figure margin to put them in;
    # the inner column overprinted its neighbour. One row per dataset reserves
    # that gutter once, for every panel.
    rows_n = len(picked)
    fig = plt.figure(figsize=(7.1, min(MAX_SHEET_HEIGHT, 1.92 * rows_n + 1.9)))
    axes = fig.subplots(rows_n, 2, squeeze=False)
    head_bottom = _headline(fig, *headline)
    drawn, total = len(picked), len(data["datasets"])
    footer = [
        "Bars grow out of the 1x rule -- the CityJSONSeq artefact for the same "
        "dataset and the same scenario -- on a logarithmic axis, so a bar's "
        "length is orders of magnitude and its direction is better or worse. "
        "Right is faster, and leaner, than the baseline.",
        "A faded read-time bar is a difference smaller than the "
        f"{data['meta']['citation_floor_s'] * 1000:.0f} ms citation floor: "
        "position without signal. The grey figure beside each scenario is the "
        "baseline's own absolute time, so a ratio can be read back into seconds. "
        "CityJSONSeq is the reference and is not drawn.",
        f"{drawn} of {total} measured datasets are drawn, spread across the "
        "corpus by CityObject count; the HTML summary page carries every dataset "
        "and every scenario, including the 1 % and 25 % windows omitted here.",
    ]
    bottom = _footer_reserve(fig, footer, 0.10) + 0.055  # + the key strip
    fig.subplots_adjust(
        left=0.175, right=0.985, top=min(0.88, head_bottom - 0.055),
        bottom=bottom, wspace=0.16, hspace=0.90,
    )

    for i, ds in enumerate(picked):
        ax_t, ax_m = axes[i][0], axes[i][1]
        _panel_heading(ax_t, ds["id"], ds["subtitle"])
        step = len(fmts) + 1.4
        rows_total = len(PANEL_SCENARIOS) * step
        for si, scenario in enumerate(PANEL_SCENARIOS):
            base = si * step
            present = [
                (fmt, index.get((ds["id"], scenario, fmt))) for fmt in fmts
            ]
            label = SCENARIO_LABEL.get(scenario, scenario)
            if scenario in GRAIN_DAGGER:
                label += "\u2020"
            ax_t.text(
                -0.235, base + (len(fmts) - 1) / 2, label,
                transform=ax_t.get_yaxis_transform(), fontsize=FS_LABEL,
                family="serif", color=INK, ha="right", va="center",
            )
            got = [r for _, r in present if r]
            if not got:
                ax_t.text(
                    0.02, base + (len(fmts) - 1) / 2, "not measured in this run",
                    transform=ax_t.get_yaxis_transform(), fontsize=FS_MARK,
                    family="serif", color=INK_3, ha="left", va="center",
                    style="italic",
                )
                continue
            base_s = got[0].get("base_time_s")
            if base_s:
                ax_t.text(
                    -0.022, base + (len(fmts) - 1) / 2,
                    f"{base_s * 1000:,.0f} ms" if base_s < 1 else f"{base_s:,.1f} s",
                    transform=ax_t.get_yaxis_transform(), fontsize=FS_MARK,
                    family="serif", color=INK_3, ha="right", va="center",
                )
            for fi, (fmt, rec) in enumerate(present):
                y = base + fi
                if not rec:
                    continue
                color = ACCENT if fmt.startswith("cityparquet") else FORMAT_GREY[fmt]
                if rec.get("time_ratio"):
                    _ratio_bar(
                        ax_t, y, 1.0 / rec["time_ratio"], tlo, thi,
                        color=color, alpha=0.34 if rec.get("below_floor") else 1.0,
                        linewidth=0,
                    )
                if rec.get("rss_ratio"):
                    _ratio_bar(
                        ax_m, y, 1.0 / rec["rss_ratio"], mlo, mhi,
                        color=color, linewidth=0,
                    )
        _format_panel_axis(ax_t, tlo, thi, rows_total)
        _format_panel_axis(ax_m, mlo, mhi, rows_total)
        _axis_label(ax_t, "read time (x faster)")
        _axis_label(ax_m, "peak memory (x leaner)")

    for ax in list(axes.ravel())[len(picked) * 2 :]:
        ax.set_visible(False)


    _format_key(fig, fmts, bottom)
    _footer(fig, footer)
    return _save(fig, "formats", out_dir)


def _format_key(fig: Figure, fmts: Sequence[str], bottom: float) -> None:
    """One swatch strip for the whole sheet.

    The per-panel mark repeats 28 times, so the package's usual direct labels
    would print the same four words 28 times over. One strip, and the bar order
    is the same in every group.
    """
    x = 0.012
    y = bottom - 0.044
    for fmt in fmts:
        color = ACCENT if fmt.startswith("cityparquet") else FORMAT_GREY[fmt]
        fig.add_artist(
            Rectangle(
                (x, y), 0.016, 0.006, transform=fig.transFigure,
                facecolor=color, edgecolor="none", zorder=5,
            )
        )
        label = FORMAT_LABEL.get(fmt, fmt)
        fig.text(
            x + 0.020, y, label, fontsize=FS_LABEL, family="serif",
            color=ACCENT if fmt.startswith("cityparquet") else INK, va="bottom",
        )
        x += 0.022 + 0.0088 * len(label)
    fig.text(
        x + 0.006, y, "-- same order in every group", fontsize=FS_MARK,
        family="serif", color=INK_2, va="bottom", style="italic",
    )


# --------------------------------------------------------------------------
# figure 7: configuration axes (row ordering)
# --------------------------------------------------------------------------


def _ordering_by_dataset(data: dict[str, Any]) -> dict[str, list[dict]]:
    by: dict[str, list[dict]] = {}
    for row in data.get("ordering", []):
        by.setdefault(row["dataset"], []).append(row)
    return by


def _ordering_pick(by_dataset: dict[str, list[dict]]) -> list[tuple[str, list[dict]]]:
    """The two datasets that bracket what this axis can show.

    One is where ordering is most measurable (the most scenarios clearing the
    citation floor, ties broken by the largest single difference), the other
    where it is least (the smallest largest-difference). Both are derived: the
    figure's whole point is that the answer depends on the dataset, so naming a
    pair here would be asserting the conclusion rather than reading it.
    """
    # A dataset too small to exercise a selective query cannot show a
    # configuration effect either, and would make "ordering is unmeasurable
    # here" look like a finding about ordering rather than about the input.
    pool = {
        ds: rows
        for ds, rows in by_dataset.items()
        if max((r.get("objects") or 0) for r in rows) >= DEGENERATE_OBJECTS
    } or by_dataset
    if not pool:
        return []
    def cleared(rows: Sequence[dict]) -> int:
        return sum(1 for r in rows if not r["below_floor"])
    def widest(rows: Sequence[dict]) -> float:
        return max((r["delta_s"] for r in rows), default=0.0)
    ranked = sorted(
        pool.items(), key=lambda kv: (cleared(kv[1]), widest(kv[1])), reverse=True
    )
    if len(ranked) == 1:
        return ranked
    return [ranked[0], ranked[-1]]


def configuration(
    data: dict[str, Any], headline: tuple[str, str], out_dir: Path
) -> list[Path]:
    """Row ordering: the Hilbert package against the source-order one."""
    picked = _ordering_pick(_ordering_by_dataset(data))
    if not picked:
        raise DataContractError("bench_data.json carries no ordering rows.")

    ratios = [
        r["time_ratio"] for _, rows in picked for r in rows if r.get("time_ratio")
    ]
    mems = [
        r["rss_ratio"] for _, rows in picked for r in rows if r.get("rss_ratio")
    ]
    lo, hi = min(min(ratios) / 1.8, 0.5), max(max(ratios) * 1.8, 2.0)
    mlo, mhi = min(min(mems) / 1.8, 0.5), max(max(mems) * 1.8, 2.0)
    order = [s for s in SCENARIO_ORDER]

    fig = plt.figure(figsize=(7.1, 4.5))
    axes = fig.subplots(2, 2, squeeze=False).ravel()
    head_bottom = _headline(fig, *headline)
    floor_ms = data["meta"]["citation_floor_s"] * 1000
    footer = [
        f"Baseline (1x) is the same package written in source order; a bar right "
        f"of the rule is a scenario Hilbert ordering made faster. A faded bar is "
        f"a difference smaller than the {floor_ms:.0f} ms citation floor, and the "
        "grey figure beside each scenario is the source-order package's own "
        "absolute time.",
        "Two datasets are drawn, chosen from the ordering run itself: the one "
        "where the most scenarios clear the floor and the one where the largest "
        "difference is smallest. Everything else about the two packages -- "
        "writer, reader, codec, row-group size -- is identical.",
        "Row ordering is the only configuration axis this run measures. The "
        "codec and row-group axes come from `just compression-bench`, which "
        "reports write time, size and row-group counts but no peak RSS and only "
        "two query types; partitioning granularity has no recipe at all.",
    ]
    bottom = _footer_reserve(fig, footer, 0.14) + 0.045
    fig.subplots_adjust(
        left=0.175, right=0.985, top=min(0.84, head_bottom - 0.075),
        bottom=bottom, wspace=0.16, hspace=0.85,
    )

    for i, (dataset, rows) in enumerate(picked):
        ax_t, ax_m = axes[i * 2], axes[i * 2 + 1]

        by_scenario = {r["scenario_key"]: r for r in rows}
        objects = next((r["objects"] for r in rows if r.get("objects")), None)
        cleared = sum(1 for r in rows if not r["below_floor"])
        subtitle = (
            f"{objects:,} CityObjects · {cleared} of {len(rows)} scenarios clear "
            f"the {floor_ms:.0f} ms floor"
            if objects
            else f"{cleared} of {len(rows)} scenarios clear the floor"
        )
        _panel_heading(ax_t, dataset, subtitle)
        for si, scenario in enumerate(order):
            rec = by_scenario.get(scenario)
            ax_t.text(
                -0.235, si, SCENARIO_LABEL.get(scenario, scenario),
                transform=ax_t.get_yaxis_transform(), fontsize=FS_LABEL,
                family="serif", color=INK, ha="right", va="center",
            )
            if not rec:
                continue
            base_s = rec["base_time_s"]
            ax_t.text(
                -0.022, si,
                f"{base_s * 1000:,.0f} ms" if base_s < 1 else f"{base_s:,.2f} s",
                transform=ax_t.get_yaxis_transform(), fontsize=FS_MARK,
                family="serif", color=INK_3, ha="right", va="center",
            )
            faded = rec["below_floor"]
            if rec.get("time_ratio"):
                _ratio_bar(
                    ax_t, si, rec["time_ratio"], lo, hi, color=ACCENT,
                    alpha=0.26 if faded else 1.0, linewidth=0,
                )
            if rec.get("rss_ratio"):
                _ratio_bar(
                    ax_m, si, rec["rss_ratio"], mlo, mhi, color=GRAY,
                    alpha=0.26 if faded else 1.0, linewidth=0,
                )
        _format_panel_axis(ax_t, lo, hi, len(order))
        _format_panel_axis(ax_m, mlo, mhi, len(order))
        _axis_label(ax_t, "read time (x faster than source order)")
        _axis_label(ax_m, "peak memory (x leaner)")

    _footer(fig, footer)
    return _save(fig, "configuration", out_dir)


def _formats_headline(data: dict[str, Any]) -> tuple[str, str]:
    """Assert what THIS run's per-dataset panels show, computed from them."""
    index = _index_read(data["read"])
    picked = _panel_pick(data["datasets"], FORMAT_PANELS)
    primary = _primary_cityparquet(data)

    def speedups(scenario: str, fmt: str) -> list[float]:
        out = []
        for ds in picked:
            rec = index.get((ds["id"], scenario, fmt))
            if rec and rec.get("time_ratio"):
                out.append(1.0 / rec["time_ratio"])
        return out

    full = speedups("full-read", primary)
    selective = [
        v
        for scenario in ("attr-filter", "attr-stats", "project")
        for v in speedups(scenario, primary)
    ]
    window_ours = speedups("bbox-5pct", primary)
    window_fcb = speedups("bbox-5pct", "flatcitybuf")
    lost = sum(
        1
        for a, b in zip(window_ours, window_fcb, strict=False)
        if b > a
    )

    if selective:
        lead = (
            f"CityParquet answers the selective queries at "
            f"{_times(min(selective))}–{_times(max(selective))} the CityJSONSeq "
            "baseline"
        )
    else:
        lead = "CityParquet is drawn against the CityJSONSeq baseline"
    if full:
        lead += (
            f", and materialises every CityObject at "
            f"{_times(min(full))}–{_times(max(full))} of it"
        )
    if lost and window_fcb:
        lead += (
            f"; FlatCityBuf's index takes the spatial window on {lost} of "
            f"{len(window_fcb)} panels"
        )
    subtitle = (
        f"{len(picked)} datasets, {len(PANEL_SCENARIOS)} query types and "
        f"{len([f for f in FORMAT_AXIS if f != 'cityjsonseq'])} formats, each "
        "against the CityJSONSeq artefact for the same dataset and scenario. "
        "Read time left, peak memory right; both logarithmic, both anchored at 1×."
    )
    return lead + ".", subtitle


def _configuration_headline(data: dict[str, Any]) -> tuple[str, str]:
    """The ordering axis's own verdict, counted rather than asserted."""
    picked = _ordering_pick(_ordering_by_dataset(data))
    floor_ms = data["meta"]["citation_floor_s"] * 1000
    if not picked:
        return (
            "No row-ordering run in this corpus.",
            "`just ordering-bench` produces it.",
        )
    best_rows = picked[0][1]
    best_cleared = [r for r in best_rows if not r["below_floor"]]
    worst_rows = picked[-1][1]
    worst_cleared = [r for r in worst_rows if not r["below_floor"]]
    gain = max((r["time_ratio"] or 0) for r in best_cleared) if best_cleared else 0

    if best_cleared and not worst_cleared:
        title = (
            f"Whether row ordering is measurable at all is a property of the "
            f"dataset: {len(best_cleared)} of {len(best_rows)} scenarios clear "
            f"the {floor_ms:.0f} ms floor on one panel and "
            f"{len(worst_cleared)} of {len(worst_rows)} on the other"
        )
    else:
        title = (
            f"Row ordering clears the {floor_ms:.0f} ms floor on "
            f"{len(best_cleared)} of {len(best_rows)} scenarios at best and "
            f"{len(worst_cleared)} of {len(worst_rows)} at worst"
        )
    if gain > 1:
        title += f", where it pays it reaches {_times(gain)}"
    subtitle = (
        "Hilbert-ordered package against the same package written in source "
        "order — same writer, same reader, same scenarios. Read time left, peak "
        "memory right; both logarithmic, both anchored at the source-order 1×."
    )
    return title + ".", subtitle


def _check_capacity(data: dict[str, Any]) -> None:
    datasets = len(data["datasets"])
    compression = len({r["dataset"] for r in data["compression"]})
    if datasets > MAX_PANELS or compression > MAX_COMPRESSION_PANELS:
        raise SystemExit(
            "benchviz figures: this run does not fit the figures' panel grid "
            f"({datasets} datasets and {compression} compression datasets "
            f"against room for {MAX_PANELS} and {MAX_COMPRESSION_PANELS}).\n"
            "  The HTML summary page has no such limit and covers all of them; "
            "only the static print figures are pinned.\n"
            "  Re-fitting them means deciding a layout for that many panels "
            "and revising the finding sentences each figure asserts, which are "
            "written for the corpus they were drawn from."
        )


def main(data_path: Path | None = None, out_dir: Path | None = None) -> Path:
    data_path = data_path or DEFAULT_DATA_PATH
    out_dir = out_dir or DEFAULT_FIGURES_DIR
    data = _load(data_path)
    _check_capacity(data)
    out_dir.mkdir(parents=True, exist_ok=True)
    plt.rcParams.update(TUFTE_RC)
    _scale_panel_fonts(*_grid(len(data["datasets"]) + 1))

    written: list[Path] = []
    written += pareto(
        data,
        "full-read",
        "pareto-full-read",
        _pareto_headline(data, "full-read", "Materialising every CityObject"),
        out_dir,
    )
    written += pareto(
        data,
        "bbox-5pct",
        "pareto-bbox-5pct",
        _pareto_headline(data, "bbox-5pct", "A 5 % bounding-box window"),
        out_dir,
    )
    written += formats(data, _formats_headline(data), out_dir)
    if data.get("ordering"):
        written += configuration(data, _configuration_headline(data), out_dir)
    else:
        print(
            "  configuration figure skipped: this corpus has no ordering run "
            "(bench/ordering_results is empty) — `just ordering-bench` "
            "produces it"
        )
    written += heatmap(data, _heatmap_headline(data), out_dir)
    written += sizes(data, _sizes_headline(data), out_dir)
    if data["compression"]:
        written += compression(data, _compression_headline(data), out_dir)
    else:
        print(
            "  compression figure skipped: this corpus has no compression run "
            "(bench/compression_results is empty) — `just compression-bench` "
            "produces it"
        )

    print(f"benchviz figures -> {out_dir}")
    for path in written:
        print(f"  {path.name}  {path.stat().st_size:,} B")
    return out_dir


if __name__ == "__main__":  # pragma: no cover
    main()
