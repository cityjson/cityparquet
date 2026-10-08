"""Paper-oriented static figures from prepared benchmark data."""

from __future__ import annotations

import json
import math
import re
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.colors as colors
import matplotlib.pyplot as plt
from matplotlib.axes import Axes
from matplotlib.cm import ScalarMappable

from . import network, prep, tables, units
from .paths import DEFAULT_DATA_PATH, DEFAULT_FIGURES_DIR

BG = "#fffff8"
INK = "#111111"
MUTED = "#666666"
ACCENT = "#E4572E"

# A modern, low-chroma palette for the bar sheets: one warm accent carries
# "ours" (CityParquet) while the comparison formats run a single neutral
# lightness ramp. Lightness, not a rainbow of hues, separates the others, so the
# panel stays quiet and the subject stays unmistakable; the axis labels and the
# printed values are the second identity channel.
FORMAT_FILL = {
    "cityparquet": "#E4572E",
    "citygml": "#C2CAD0",
    "cityjson": "#A3AEB7",
    "cityjsonseq": "#83919C",
    "flatcitybuf": "#5F6E7A",
}
DATABASE_FILL = {
    "duckdb-cityparquet": "#E4572E",
    "cityparquet": "#E4572E",
    "cjdb": "#83919C",
    "3dcitydb": "#C2CAD0",
}
BLOOM_OFF_COLOUR = "#5F6E7A"  # the package without filters, against the accent default

BAD_CELL = "#efeee6"
# Cell separators. A black grid would fight the fills, which are the reading;
# a pale warm rule only tells the eye where one cell ends.
CELL_EDGE = "#e6e3d7"
# Ratio cells share one vocabulary: teal beats the baseline, the page colour is
# the baseline, the warm accent is worse. A metric that can never beat its
# baseline (the database write tier) uses a one-sided version of the same ramp
# rather than spending the teal half of a diverging map on values that never
# occur.
CMAP_DIVERGING = colors.LinearSegmentedColormap.from_list("cp_ratio", ["#2A9D8F", BG, ACCENT])
# The format comparison's FACTORS run the other way — CityGML's value over the
# format's, larger is better — so the same vocabulary is mirrored: teal for a
# factor above 1 (faster or leaner than CityGML), the accent below it.
CMAP_FACTOR = colors.LinearSegmentedColormap.from_list("cp_factor", [ACCENT, BG, "#2A9D8F"])
CMAP_COST = colors.LinearSegmentedColormap.from_list("cp_cost", [BG, "#F3B199", ACCENT])

LABELS = {
    "cityparquet": "CityParquet",
    "cityjsonseq": "CityJSONSeq",
    "citygml": "CityGML",
    "cityjson": "CityJSON",
    "flatcitybuf": "FlatCityBuf",
    "3dcitydb": "3DCityDB",
    "cjdb": "cjdb",
    "duckdb-cityparquet": "CityParquet (DuckDB)",
    "duckdb-cityparquet-writeback": "CityParquet (DuckDB, + package write-back)",
}
# The database figures put the native reader beside the DuckDB tags, where the
# format family's "CityParquet" would be ambiguous; column headers wrap.
DATABASE_LABELS = {
    "duckdb-cityparquet": "CityParquet\n(DuckDB)",
    "duckdb-cityparquet-writeback": "CityParquet\n(DuckDB, + package\nwrite-back)",
    "cityparquet": "CityParquet\n(native reader)",
    "cjdb": "cjdb",
    "3dcitydb": "3DCityDB",
}
# Query keys as the catalogue's plain-language questions
# (notes/benchmark-queries.md); a key without an entry falls back to a
# title-cased key.
SCENARIO_LABELS = {
    "full-read": "Read all",
    "geometry-scan": "Read all (geometry)",
    "count": "Count",
    "id-lookup": "Id lookup",
    "id-miss": "Id lookup (miss)",
    "attr-filter": "Attribute filter",
    "attr-range": "Attribute range",
    "attr-stats": "Attribute stats",
    "bbox-1pct": "Spatial 1 %",
    "bbox-5pct": "Spatial 5 %",
    "bbox-25pct": "Spatial 25 %",
    "id-10pct": "Id lookup (hit 10 %)",
    "id-50pct": "Id lookup (hit 50 %)",
    "id-90pct": "Id lookup (hit 90 %)",
    "feature-50pct": "Feature lookup (hit 50 %)",
    "feature-miss": "Feature lookup (miss)",
    "parts-per-building": "Parts per building",
    "parts-per-building-join": "Parts per building (join)",
    "attr-add": "Add attribute",
    "attr-update": "Update attribute",
    "attr-delete": "Delete attribute",
    "append-object": "Append one building",
}


def _load(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def lod_query_label(params: dict) -> str:
    """The `lod-query` row label: the highest LoD the database run carried.

    `citybench` writes the dataset's LoDs (`lods`, read from the package's
    `geometry_lod<d>_<d>` column names: "0", "1.3", "2.2") into every params
    sidecar. `lod-query` targets `LOD_QUERY_TARGET` in
    `citybench/scenarios/registry.py`, which the sidecar does not record; on
    the database family's dataset that target is the highest LoD it carries.
    A sidecar without `lods`, or with none, is not one the harness wrote: it
    is refused.
    """
    lods = params.get("lods")
    if not lods:
        raise prep.PrepError("database params carry no `lods`: cannot label the lod-query row")
    return f"LoD {max(lods, key=float)} rows"


def _label(value: str) -> str:
    attr = prep.ATTR_NOTE_RE.match(value)
    if attr and value not in LABELS and value not in SCENARIO_LABELS:
        probe = "miss" if value.endswith("-miss") else "hit 50 %"
        return f"{attr.group('column')} lookup ({probe})"
    return LABELS.get(value) or SCENARIO_LABELS.get(value) or value.replace("-", " ").title()


def _seconds(value: Any) -> str:
    if value is None:
        return "—"
    value = float(value)
    return f"{value * 1000:.1f} ms" if value < 1 else f"{value:.2f} s"


def _statistic(data: dict[str, Any]) -> str:
    """The timing statistic `prep.build` recorded for these figures."""
    return data.get("statistic", "median")


def _statistic_note(data: dict[str, Any]) -> str:
    """The caption line naming the plotted timing statistic (`prep.statistic_note`)."""
    return data.get("meta", {}).get("statistic_note") or prep.statistic_note(_statistic(data), [])


def _ratio(value: Any, base: Any) -> float | None:
    return float(value) / float(base) if value is not None and base not in (None, 0) else None


def _format_fill(fmt: str) -> str:
    return FORMAT_FILL.get(fmt, MUTED)


def _axis_palette(variants: list[str]) -> dict[str, str]:
    return {v: BLOOM_OFF_COLOUR for v in variants if v != "cityparquet"}


def _save(fig: plt.Figure, name: str, out: Path) -> list[Path]:
    out.mkdir(parents=True, exist_ok=True)
    files = [out / f"{name}.svg", out / f"{name}.png"]
    # No timestamp, and element ids salted with a constant (`main` sets
    # `svg.hashsalt`): the same data renders to the same bytes, so a
    # re-render changes a committed figure only when the figure changed.
    fig.savefig(files[0], bbox_inches="tight", metadata={"Date": None})
    fig.savefig(files[1], dpi=300, bbox_inches="tight")
    plt.close(fig)
    return files


def _title(dataset: dict[str, Any]) -> str:
    """A dataset's figure title: its name and its size.

    A slice is titled by the size it was cut to (`nominal_objects`); its exact
    CityObject count can exceed that, because a feature is indivisible, and
    `slice_note` states it beside the figure. A corpus dataset is titled by
    its CityObject count.
    """
    name = dataset.get("title") or dataset.get("id")
    nominal = dataset.get("nominal_objects")
    if nominal:
        return f"{name} — {int(nominal):,}-object slice"
    count = dataset.get("objects") or dataset.get("object_count")
    return f"{name} — {int(count):,} CityObjects" if count else str(name)


def slice_note(dataset: dict[str, Any]) -> str:
    """The exact CityObject count of a slice whose title names its nominal size."""
    nominal, count = dataset.get("nominal_objects"), dataset.get("objects")
    if not nominal or not count:
        return ""
    return (
        f"The slice holds {int(count):,} CityObjects: it is cut at whole features, "
        f"so it can exceed its nominal {int(nominal):,}."
    )


def _nice_vmax(value: float) -> float:
    """The smallest comfortable log2 bound at or above `value`."""
    value = max(1.0, value)
    for step in (1, 2, 3, 4, 6, 8, 12, 16, 24, 32):
        if value <= step:
            return float(step)
    return float(2 ** math.ceil(math.log2(value)))


def _cell_bound(cells: list[list[tuple[float | None, str]]], scale: str) -> float:
    """A comfortable colour bound for a set of ratio cells."""
    exps = [
        abs(math.log2(ratio))
        for row in cells
        for ratio, _text in row
        if ratio and ratio > 0 and (scale == "diverging" or ratio >= 1)
    ]
    return _nice_vmax(max(exps) if exps else 1.0)


def _ratio_from_log2(exp: float) -> str:
    """A log2 ratio as a compact multiplier for a colourbar tick."""
    ratio = 2.0**exp
    if ratio >= 1e3 or ratio < 1e-2:
        return f"{ratio:.0e}×"
    if ratio >= 1:
        return f"{ratio:g}×"
    return f"{ratio:.3g}×"


def _ratio_short(value: float | None) -> str:
    """A ratio as it goes inside a heatmap cell."""
    if value is None:
        return "—"
    if value >= 1e3 or value < 1e-2:
        return f"{value:.0e}×"
    if value >= 10:
        return f"{value:.0f}×"
    return f"{value:.2g}×"


def _compact(value: float) -> str:
    """An absolute value short enough to sit over a narrow bar."""
    value = float(value)
    if abs(value) >= 1e6:
        return f"{value / 1e6:.2f}M"
    if abs(value) >= 1e3:
        return f"{value / 1e3:.2f}k"
    return f"{value:.3g}"


def _heat_ticks(vmax: float, scale: str) -> list[float]:
    # Three ticks, always: a colourbar this narrow cannot carry more without
    # the labels colliding, and the printed cells carry the precision.
    if scale == "cost":
        return [0.0, vmax / 2, vmax]
    return [-vmax, 0.0, vmax]


def _heat_colors(scale: str, vmax: float) -> tuple[colors.Colormap, colors.Normalize]:
    if scale == "factor":
        return CMAP_FACTOR.with_extremes(bad=BAD_CELL), colors.TwoSlopeNorm(
            vmin=-vmax, vcenter=0, vmax=vmax
        )
    if scale == "cost":
        return CMAP_COST.with_extremes(bad=BAD_CELL), colors.Normalize(vmin=0, vmax=vmax)
    return CMAP_DIVERGING.with_extremes(bad=BAD_CELL), colors.TwoSlopeNorm(
        vmin=-vmax, vcenter=0, vmax=vmax
    )


def _heat(
    ax: Axes,
    cells: list[list[tuple[float | None, str]]],
    rows: list[str],
    columns: list[str],
    title: str,
    *,
    vmax: float = 3.0,
    scale: str = "diverging",
    x_rotation: int = 45,
    row_labels: dict[str, str] | None = None,
) -> None:
    cmap, norm = _heat_colors(scale, vmax)
    vals = [[math.log2(v) if v and v > 0 else math.nan for v, _ in row] for row in cells]
    ax.imshow(vals, cmap=cmap, norm=norm, aspect="auto")
    for x in range(1, len(columns)):
        ax.axvline(x - 0.5, color=CELL_EDGE, linewidth=0.6, zorder=2)
    for y in range(1, len(rows)):
        ax.axhline(y - 0.5, color=CELL_EDGE, linewidth=0.6, zorder=2)
    ax.set_title(title, fontsize=9, loc="left")
    if x_rotation:
        ax.set_xticks(
            range(len(columns)),
            [_label(c) for c in columns],
            rotation=x_rotation,
            ha="right",
            fontsize=6,
        )
    else:
        ax.set_xticks(range(len(columns)), [_label(c) for c in columns], fontsize=6)
    ax.set_yticks(
        range(len(rows)), [(row_labels or {}).get(r) or _label(r) for r in rows], fontsize=6
    )
    for y, row in enumerate(cells):
        for x, (_, text) in enumerate(row):
            ax.text(x, y, text, ha="center", va="center", fontsize=5, color=INK)
    for spine in ax.spines.values():
        spine.set_visible(False)


FORMATS_DIR = tables.FORMATS_DIR
FORMAT_METRICS = tables.FORMAT_METRICS
# log2 of the factor at which a cell's colour saturates: 1024x.
FACTOR_COLOUR_LIMIT = 10.0
FACTOR_KEY = (
    "Cell text: absolute value over its factor against CityGML (CityGML's value ÷ the "
    "format's; higher is better). × n/a: the CityGML cell is missing, skipped or failed."
)


def _factor_text(value: float | None) -> str:
    return "× n/a" if value is None else f"{_ratio_short(value)}"


def sizes(data: dict[str, Any], out: Path) -> list[Path]:
    """File size per dataset, with each format's factor against CityGML."""
    datasets = data.get("datasets", [])
    folder = out / FORMATS_DIR
    if not data.get("sizes"):
        return _missing("sizes", folder)
    formats = list(prep.FORMATS)
    nrows = max(1, math.ceil(len(datasets) / 3))
    fig, axes = plt.subplots(
        nrows, 3, figsize=(8.5, 2.8 * nrows), squeeze=False, constrained_layout=True
    )
    for ax, dataset in zip(axes.flat, datasets, strict=False):
        by = {
            r["format"]: r for r in data.get("sizes", []) if r.get("dataset") == dataset.get("id")
        }
        values = [by.get(f, {}).get("bytes") for f in formats]
        unit, divisor = units.size_unit(values)
        bars = ax.bar(
            range(len(formats)),
            [float(v) / divisor if v is not None else math.nan for v in values],
            color=[_format_fill(f) for f in formats],
        )
        ax.set_title(_title(dataset), fontsize=8)
        # Headroom for the two-line label over the tallest bar.
        top = max((float(v) / divisor for v in values if v is not None), default=1.0)
        ax.set_ylim(0, top * 1.22)
        ax.set_ylabel(unit, fontsize=7)
        ax.set_xticks(
            range(len(formats)), [_label(f) for f in formats], rotation=35, ha="right", fontsize=6
        )
        ax.tick_params(axis="y", labelsize=5.5)
        for x, (bar, fmt, value) in enumerate(zip(bars, formats, values, strict=False)):
            if value is None:
                ax.text(x, 0, "missing", ha="center", va="bottom", fontsize=5)
                continue
            factor = by[fmt].get("factor")
            ax.text(
                x,
                bar.get_height(),
                f"{units.format_bytes(value)}\n{_factor_text(factor)}",
                ha="center",
                va="bottom",
                fontsize=5,
            )
    for ax in list(axes.flat)[len(datasets) :]:
        ax.axis("off")
    fig.suptitle(
        "File size on disk — factor against CityGML (CityGML bytes ÷ format bytes; "
        "higher is smaller)",
        x=0.01,
        ha="left",
        fontsize=11,
    )
    return _save(fig, "sizes", folder)


def _unavailable_label(record: dict | None) -> str:
    """The text of a format cell with no citable value."""
    if record is None:
        return "—"
    reason = str(record.get("unavailable") or "")
    if reason.startswith("skipped"):
        return "skipped"
    if reason.startswith(("error", "status=")):
        return "failed"
    if "mismatch" in reason:
        return "mismatch"
    return "—"


def format_cells(
    data: dict[str, Any], dataset: dict[str, Any], field: str
) -> tuple[list[str], list[list[tuple[float | None, str]]]]:
    """One dataset's (queries, cells) for one metric: rows are queries, columns
    the formats in display order, each cell (factor, text).

    The text is the absolute value over its factor against CityGML; a cell
    without a CityGML value keeps its own value and says `n/a` where the
    factor would be, and a cell that was not measured, skipped or failed
    carries that word instead of a number. A factor is never zero and never against another format.
    """
    records = [r for r in data.get("read", []) if r.get("dataset") == dataset.get("id")]
    queries = tables.dataset_queries(records)
    index = {(r.get("format"), r.get("scenario_key")): r for r in records}
    factor_field = "time_factor" if field == "time_s" else "rss_factor"
    cells = []
    for query in queries:
        row = []
        for fmt in prep.FORMATS:
            record = index.get((fmt, query))
            value = (record or {}).get(field)
            if value is None:
                row.append((None, _unavailable_label(record)))
                continue
            number = float(value) / (units.MB if field == "rss_b" else 1)
            factor = record.get(factor_field)
            row.append((factor, f"{number:.3g}\n{_factor_text(factor)}"))
        cells.append(row)
    return queries, cells


def format_figures(data: dict[str, Any], out: Path) -> list[Path]:
    """`formats/<dataset>/time` and `formats/<dataset>/rss`, one figure each.

    Each metric's colour bound is shared across the datasets, so a colour
    means the same factor in every figure.
    """
    datasets = data.get("datasets", [])
    if not datasets or not data.get("read"):
        return _missing("formats", out / FORMATS_DIR)
    blocks = {
        (dataset["id"], field): format_cells(data, dataset, field)
        for dataset in datasets
        for _name, field, _page, _axis in FORMAT_METRICS
    }
    # Factors span six orders of magnitude (a count read from metadata against
    # a full CityGML parse), so the colour saturates at FACTOR_COLOUR_LIMIT
    # either way; the printed factor carries the precision.
    bounds = {
        field: min(
            FACTOR_COLOUR_LIMIT,
            _cell_bound(
                [row for (_d, f), (_q, cells) in blocks.items() if f == field for row in cells],
                "diverging",
            ),
        )
        for _name, field, _page, _axis in FORMAT_METRICS
    }
    written: list[Path] = []
    formats = list(prep.FORMATS)
    for dataset in datasets:
        note = slice_note(dataset)
        for name, field, _page, axis_title in FORMAT_METRICS:
            queries, cells = blocks[(dataset["id"], field)]
            fig = plt.figure(figsize=(7.2, 0.42 * max(1, len(queries)) + 2.4), layout="constrained")
            grid = fig.add_gridspec(2, 1, height_ratios=[max(1, len(queries)), 0.35])
            ax = fig.add_subplot(grid[0])
            _heat(
                ax,
                cells,
                queries,
                formats,
                axis_title,
                vmax=bounds[field],
                scale="factor",
                x_rotation=0,
            )
            for text in ax.texts:
                text.set_fontsize(6)
            ax.tick_params(axis="y", labelsize=7)
            ax.tick_params(axis="x", labelsize=6.5)
            cax = fig.add_subplot(grid[1])
            cmap, norm = _heat_colors("factor", bounds[field])
            bar = fig.colorbar(
                ScalarMappable(norm=norm, cmap=cmap),
                cax=cax,
                orientation="horizontal",
                extend="both" if bounds[field] >= FACTOR_COLOUR_LIMIT else "neither",
            )
            ticks = _heat_ticks(bounds[field], "factor")
            bar.set_ticks(ticks)
            bar.set_ticklabels([_ratio_from_log2(t) for t in ticks])
            bar.ax.tick_params(labelsize=6, length=0)
            bar.outline.set_visible(False)
            caption = FACTOR_KEY + (f" {note}" if note else "")
            if field == "time_s":
                caption += f" {_statistic_note(data)}"
            bar.set_label(caption, fontsize=6, wrap=True)
            fig.suptitle(_title(dataset), fontsize=11, x=0.01, ha="left")
            written += _save(fig, name, out / FORMATS_DIR / dataset["id"])
    return written


def _axis(data: dict[str, Any], key: str) -> tuple[list[dict], list[dict], list[str]]:
    axis = data.get(key, {})
    return axis.get("records", []), axis.get("sizes", []), axis.get("variants", [])


def _axis_queries(records: list[dict]) -> list[str]:
    wanted = [
        "full-read",
        "bbox-1pct",
        "bbox-5pct",
        "bbox-25pct",
        "id-50pct",
        "id-miss",
        "feature-50pct",
        "feature-miss",
        "id-lookup",
    ]
    present = {r.get("measure") for r in records}
    return [q for q in wanted if q in present]


def _axis_main(data: dict[str, Any], key: str, out: Path) -> list[Path]:
    records, sizes, variants = _axis(data, key)
    if not records:
        return _missing(key, out)
    # The headline dataset is the manifest's slice: the suite's largest input
    # and the one dataset every family measures. A run that did not measure it
    # gets a placeholder rather than a corpus city standing in for it.
    largest = data.get("meta", {}).get("slice_dataset")
    selected = [r for r in records if r.get("dataset") == largest]
    if not selected:
        return _missing(
            key,
            out,
            f"Not rendered: the slice dataset ({largest or 'unnamed'}) was not measured; "
            f"the {key} axis measures the slice alone.",
        )
    # The axis's own list of queries, so a lookup this run did not measure
    # keeps its column and says so instead of vanishing from the figure.
    queries = list(data.get(key, {}).get("measures") or []) or _axis_queries(selected)
    answered = {r.get("measure") for r in selected}
    unmeasured = [q for q in queries if q not in answered]
    palette = _axis_palette(variants)
    # One row: the package size beside the read heatmaps it explains and the
    # row groups each lookup pruned.
    fig = plt.figure(figsize=(15, 4.8))
    grid = fig.add_gridspec(
        1,
        4,
        width_ratios=[0.45, 1, 1, 1],
        left=0.05,
        right=0.9,
        bottom=0.36,
        top=0.84,
        wspace=0.3,
    )
    for col, (title, source, _ratio_field, value, measure) in enumerate(
        (
            (
                "size (MB)",
                [r for r in sizes if r.get("dataset") == largest],
                "size_ratio",
                "bytes",
                None,
            ),
        )
    ):
        ax = fig.add_subplot(grid[0, col])
        by = {r.get("variant"): r for r in source if measure is None or r.get("measure") == measure}
        divisor = units.MB if value in ("bytes", "rss_b") else 1
        vals = [
            (float(by[v][value]) / divisor) if v in by and by[v].get(value) is not None else None
            for v in variants
        ]
        ax.bar(
            range(len(variants)),
            [v if v is not None else float("nan") for v in vals],
            color=[palette.get(v, MUTED) for v in variants],
        )
        baseline_value = vals[variants.index("cityparquet")] if "cityparquet" in variants else None
        if baseline_value is not None:
            ax.axhline(baseline_value, color=MUTED, linewidth=0.5, linestyle=":")
        ax.set_title(title, fontsize=8)
        ax.set_xticks(
            range(len(variants)),
            [v.replace("cityparquet+", "").replace("cityparquet", "default") for v in variants],
            rotation=40,
            ha="right",
            fontsize=5,
        )
        for x, v, row in zip(
            range(len(variants)), vals, [by.get(v) for v in variants], strict=False
        ):
            if row and v is not None:
                actual = float(row[value])
                if value in ("bytes", "rss_b"):
                    actual /= units.MB
                ax.text(x, v, _compact(actual), ha="center", va="bottom", fontsize=4.5)
    short_variants = [
        v.replace("cityparquet+", "").replace("cityparquet", "default") for v in variants
    ]
    heat_axes = [fig.add_subplot(grid[0, 1]), fig.add_subplot(grid[0, 2])]
    cell_blocks = []
    for field in ("time_s", "rss_b"):
        cells = []
        for variant in variants:
            row = []
            for query in queries:
                rows = [
                    r for r in selected if r.get("variant") == variant and r.get("measure") == query
                ]
                base = [
                    r
                    for r in selected
                    if r.get("variant") == "cityparquet" and r.get("measure") == query
                ]
                r = rows[0] if rows else None
                b = base[0] if base else None
                actual = r.get(field) if r else None
                ratio = _ratio(actual, b.get(field) if b else None)
                if query in unmeasured:
                    text = "not\nmeasured"
                elif actual is None:
                    text = "—"
                else:
                    # Milliseconds: a lookup takes well under a second, and
                    # six decimal places of seconds overrun the cell.
                    number = float(actual) / units.MB if field == "rss_b" else actual * 1000
                    text = f"{number:.3g}\n{_ratio_short(ratio)}" if ratio else f"{number:.3g}"
                row.append((ratio, text))
            cells.append(row)
        cell_blocks.append(cells)
    # Both rows are read ratios against the default package, so one diverging
    # bound can serve them; the cells carry the precision either way.
    bound = _cell_bound([row for block in cell_blocks for row in block], "diverging")
    for col, (ax, cells, title) in enumerate(
        zip(heat_axes, cell_blocks, ("read time (ms)", "read peak RSS (MB)"), strict=True)
    ):
        _heat(ax, cells, short_variants, queries, title, vmax=bound, scale="diverging")
        for text in ax.texts:
            text.set_fontsize(5.5)
        ax.tick_params(axis="y", labelsize=6)
        ax.tick_params(axis="x", labelsize=6)
        if col == 1:
            ax.set_yticks([])
    # The pruning counts: row groups the Bloom filters ruled out, then those
    # the min/max statistics ruled out, of the table's row groups. Text only —
    # a count is not a ratio to the default, so the cells carry no colour.
    prune_ax = fig.add_subplot(grid[0, 3])
    prune_cells = []
    for variant in variants:
        row = []
        for query in queries:
            r = next(
                (s for s in selected if s.get("variant") == variant and s.get("measure") == query),
                None,
            )
            total = r.get("row_groups_total") if r else None
            if query in unmeasured:
                text = "not\nmeasured"
            elif total is None:
                text = "—"
            else:
                text = f"{r.get('bloom_pruned') or 0} + {r.get('stats_pruned') or 0}\nof {total}"
            row.append((None, text))
        prune_cells.append(row)
    _heat(
        prune_ax,
        prune_cells,
        short_variants,
        queries,
        "row groups pruned (bloom + stats of total)",
        vmax=bound,
        scale="diverging",
    )
    for text in prune_ax.texts:
        text.set_fontsize(5.5)
    prune_ax.tick_params(axis="x", labelsize=6)
    prune_ax.set_yticks([])
    cbar = fig.colorbar(heat_axes[-1].images[0], cax=fig.add_axes([0.925, 0.36, 0.012, 0.48]))
    ticks = _heat_ticks(bound, "diverging")
    cbar.set_ticks(ticks)
    cbar.set_ticklabels([_ratio_from_log2(t) for t in ticks])
    cbar.ax.tick_params(labelsize=6, length=0)
    cbar.outline.set_visible(False)
    cbar.set_label("Ratio to default; lower is better", fontsize=7)
    headline = {
        "id": largest,
        "objects": max(r.get("objects") or 0 for r in selected) or None,
        **data.get("meta", {}).get("dataset_labels", {}).get(largest, {}),
    }
    fig.suptitle(f"{key.capitalize()} filters — {_title(headline)}", x=0.01, ha="left", fontsize=11)
    unmeasured_note = (
        f"Not measured in this run: {', '.join(_label(q) for q in unmeasured)}."
        if unmeasured
        else ""
    )
    note = " ".join(filter(None, (slice_note(headline), _statistic_note(data), unmeasured_note)))
    fig.text(0.01, 0.02, note, fontsize=6, color=MUTED, ha="left", wrap=True)
    return _save(fig, key, out)


def _remove_retired(out: Path) -> list[Path]:
    """Remove the retired `bloom-corpus` figure from a re-used directory.

    The bloom axis measures the 3DBAG slice alone (a filter rules out whole
    row groups, and every corpus dataset but Zurich is a single group), so no
    corpus figure is drawn; one an earlier run left behind must not reach the
    summary page as if this run had measured it.
    """
    for suffix in ("svg", "png"):
        (out / f"bloom-corpus.{suffix}").unlink(missing_ok=True)
    return []


def _missing(
    name: str,
    out: Path,
    reason: str = "Not rendered: this result family is absent from this run.",
) -> list[Path]:
    fig, ax = plt.subplots(figsize=(6, 2.2))
    ax.axis("off")
    ax.text(0.5, 0.58, name, ha="center", va="center", fontsize=12)
    ax.text(
        0.5,
        0.35,
        reason,
        ha="center",
        va="center",
        color=MUTED,
        fontsize=8,
    )
    return _save(fig, name.lower().replace(" ", "-"), out)


# Column order of the database figure: CityParquet first, the baseline last.
# The write-back tag is not a column: its write cells are stacked inside the
# `duckdb-cityparquet` cell (`DATABASE_STACKED`).
DATABASE_READ_SYSTEMS = (
    "duckdb-cityparquet",
    "cityparquet",
    "cjdb",
    "3dcitydb",
)
DATABASE_STACKED = {"duckdb-cityparquet": "duckdb-cityparquet-writeback"}
# The blank band between the read rows and the write tier.
DB_WRITE_SEPARATOR = ""
THREAD_TITLES = {
    "single": "threads=single (primary)",
    "parallel": "threads=parallel (disclosed second pass)",
    None: "threads unspecified",
}


def _db_label(system: str) -> str:
    return DATABASE_LABELS.get(system) or _label(system)


def _db_status_text(record: dict | None) -> str:
    """What a cell says when it holds no citable number."""
    if record is None:
        return "n/a"
    return record.get("status") or "missing"


def _notes_axis(ax: Axes, lines: list[str]) -> None:
    ax.axis("off")
    ax.text(
        0.0,
        1.0,
        "\n".join(lines),
        ha="left",
        va="top",
        fontsize=6,
        color=MUTED,
        transform=ax.transAxes,
        wrap=True,
    )


# A database cell: the colour ratio, the printed text, whether it is a stacked
# CityParquet write cell, and that cell's write-back ratio (its lower half).
DbCell = tuple[float | None, str, bool, float | None]


def database_blocks(data: dict[str, Any]) -> dict[str, Any]:
    """The database heatmap cells, reads then the write tier, per configuration.

    Rows are the read scenarios, a blank separator, then the write tier; the
    write tier runs once under `threads=single`, so any other configuration
    shows `n/a` there. Ratios are to the baseline within ONE configuration.
    Each `duckdb-cityparquet` write cell stacks the in-engine value over the
    `duckdb-cityparquet-writeback` value, each with its own ratio.
    """
    db = data.get("databases") or {}
    records = db.get("records", [])
    reads = [r for r in records if r.get("tier", "read") == "read"]
    writes = [r for r in records if r.get("tier") == "write"]
    baseline = db.get("baseline") or prep.DB_BASELINE
    stacked = set(DATABASE_STACKED.values())
    present = {r.get("format") for r in reads}
    systems = [s for s in DATABASE_READ_SYSTEMS if s in present]
    systems += sorted(s for s in present - set(systems) if s)
    for record in writes:
        system = record.get("format")
        if system and system not in stacked and system not in systems:
            systems.append(system)
    found = {r.get("scenario") for r in reads if r.get("scenario")}
    queries = [q for q in prep.DB_READ_SCENARIOS if q in found]
    queries += sorted(found - set(queries))
    found = {r.get("scenario") for r in writes if r.get("scenario")}
    write_rows = [q for q in prep.DB_WRITE_SCENARIOS if q in found]
    write_rows += sorted(found - set(write_rows))
    rows = queries + ([DB_WRITE_SEPARATOR, *write_rows] if write_rows else [])
    configs = [t for t in prep.DB_THREADS if any(r.get("threads") == t for r in reads)]
    if any(r.get("threads") not in prep.DB_THREADS for r in reads):
        configs.append(None)
    index = {(r.get("format"), r.get("scenario"), r.get("threads")): r for r in records}
    footnotes: dict[tuple[str, str | None, str], int] = {}
    skips: dict[tuple[str, str], int] = {}

    def value(system: str, query: str, config: str | None, field: str, formatter, sep: str):
        """(ratio, text) for one system's value; `sep` joins value and ratio."""
        record = index.get((system, query, config))
        base = index.get((baseline, query, config))
        citable = record is not None and record.get("status") in prep.DB_CITABLE
        metric = record.get(field) if citable else None
        if metric is None:
            if citable and query in write_rows and field == "peak_memory_bytes":
                return (None, "not sampled")
            if citable:
                return (None, "—")
            text = _db_status_text(record)
            if record is not None and record.get("status") == "skipped":
                reason = _db_skip_reason(record.get("notes", ""))
                number = skips.setdefault((query, reason), len(skips) + 1)
                text += f" †{number}"
            return (None, text)
        base_metric = (
            base.get(field) if base is not None and base.get("status") in prep.DB_CITABLE else None
        )
        ratio = _ratio(metric, base_metric)
        text = formatter(metric) + (f"{sep}{_ratio_short(ratio)}" if ratio else "")
        if record.get("status") == "ok-deviation":
            key = (query, config, record.get("deviation") or record.get("notes", ""))
            number = footnotes.setdefault(key, len(footnotes) + 1)
            text += f" *{number}"
        return (ratio, text)

    def cell(system: str, query: str, config: str | None, field: str, formatter) -> DbCell:
        if query == DB_WRITE_SEPARATOR:
            return (None, "", False, None)
        writeback = DATABASE_STACKED.get(system)
        if query in write_rows and config == "single" and writeback:
            top_ratio, top = value(system, query, config, field, formatter, " · ")
            low_ratio, low = value(writeback, query, config, field, formatter, " · ")
            return (top_ratio, f"{top}\n+wb {low}", True, low_ratio)
        ratio, text = value(system, query, config, field, formatter, "\n")
        return (ratio, text, False, None)

    blocks = {
        (field, config): [
            [cell(system, query, config, field, formatter) for system in systems] for query in rows
        ]
        for field, _title, formatter in DB_HEAT_SPECS
        for config in configs
    }
    return {
        "baseline": baseline,
        "systems": systems,
        "rows": rows,
        "write_rows": write_rows,
        "configs": configs,
        "blocks": blocks,
        "footnotes": footnotes,
        "skips": skips,
    }


DB_HEAT_SPECS = (
    ("time_s", "{statistic} query time", _seconds),
    ("peak_memory_bytes", "{memory}", units.format_bytes),
)


def _db_skip_reason(notes: str) -> str:
    """The `skipped: <reason>` text of a row's notes, without the thread tag."""
    for tag in notes.split(";"):
        if (at := tag.find("skipped:")) >= 0:
            return DB_THREADS_TAG_RE.sub("", tag[at + len("skipped:") :]).strip()
    return "no reason recorded"


DB_THREADS_TAG_RE = re.compile(r"\s*\bthreads=\w+")


def _db_memory_title(db: dict) -> str:
    """The memory panel's title, named by the metric the run actually wrote."""
    metric = db.get("memory_metric") or "peak_rss_bytes"
    return prep.DB_MEMORY_LABELS.get(metric, metric)


def _db_bytes(count: float | None) -> str:
    """A storage size; kB below a megabyte, so a small index never reads as 0.0 MB."""
    if count is not None and 0 < count < units.MB:
        return f"{count / 1_000:.0f} kB" if count >= 1_000 else f"{count:.0f} B"
    return units.format_bytes(count)


def _db_storage(ax: Axes, sizes: list[dict], systems: list[str], baseline: str) -> None:
    """Storage per system: with and without indexes when the manifest splits them.

    With an `index_bytes` breakdown each bar stacks the data without indexes
    (solid) under the indexes (hatched), annotated with both; without one the
    bar is the total including indexes, as the CSV's `size_bytes` states.
    """
    by_format = {r.get("format"): r for r in sizes}
    size_systems = [s for s in systems if s in by_format] or systems
    split = any(r.get("index_bytes") is not None for r in sizes)
    totals = [(by_format.get(s) or {}).get("size_bytes") for s in size_systems]
    base_total = (by_format.get(baseline) or {}).get("size_bytes")
    fills = [DATABASE_FILL.get(s, MUTED) for s in size_systems]
    xs = range(len(size_systems))
    if split:
        data_part = [(by_format.get(s) or {}).get("no_index_bytes") for s in size_systems]
        index_part = [(by_format.get(s) or {}).get("index_bytes") for s in size_systems]
        base_data = (by_format.get(baseline) or {}).get("no_index_bytes")
        mb = [float(v) / units.MB if v is not None else math.nan for v in data_part]
        ax.bar(xs, mb, color=fills, label="without indexes")
        ax.bar(
            xs,
            [float(v) / units.MB if v is not None else math.nan for v in index_part],
            bottom=[0.0 if math.isnan(v) else v for v in mb],
            color=fills,
            alpha=0.45,
            hatch="////",
            edgecolor=INK,
            linewidth=0,
            label="indexes",
        )
        ax.set_title(
            "Storage with and without indexes (solid: data without indexes; hatched: indexes)",
            loc="left",
            fontsize=9,
        )
        for x, (total, no_index, index) in enumerate(
            zip(totals, data_part, index_part, strict=True)
        ):
            if total is None:
                ax.text(x, 0, "missing", ha="center", va="bottom", fontsize=6)
                continue
            ratio = _ratio(total, base_total)
            no_ratio = _ratio(no_index, base_data) if no_index is not None else None
            lines = [
                "total " + _db_bytes(total) + (f" · {ratio:.2g}×" if ratio else ""),
                "no index "
                + (_db_bytes(no_index) if no_index is not None else "n/a")
                + (f" · {no_ratio:.2g}×" if no_ratio else ""),
                "indexes " + (_db_bytes(index) if index is not None else "n/a"),
            ]
            ax.text(x, total / units.MB, "\n".join(lines), ha="center", va="bottom", fontsize=5.5)
        ax.margins(y=0.45)
    else:
        bars = ax.bar(
            xs, [float(v) / units.MB if v is not None else math.nan for v in totals], color=fills
        )
        ax.set_title("Storage including indexes", loc="left", fontsize=9)
        for x, (bar, value) in enumerate(zip(bars, totals, strict=False)):
            if value is None:
                ax.text(x, 0, "missing", ha="center", va="bottom", fontsize=6)
            else:
                ratio = _ratio(value, base_total)
                detail = _db_bytes(value) + (f" · {ratio:.2g}×" if ratio else "")
                ax.text(x, bar.get_height(), detail, ha="center", va="bottom", fontsize=6)
    ax.set_ylabel("MB", fontsize=7)
    ax.set_xticks(xs, [_db_label(s).replace("\n", " ") for s in size_systems], fontsize=6.5)
    ax.tick_params(axis="y", labelsize=6)


def databases(data: dict[str, Any], out: Path) -> list[Path]:
    """The database comparison: storage, then time and memory per thread configuration.

    Ratios are to 3DCityDB within ONE thread configuration; `single` is the
    left, primary column and `parallel` the disclosed second pass beside it.
    The write tier sits below the reads under a rule, in the `single` panels
    only, and carries Caveat 19 as a footnote.
    """
    db = data.get("databases") or {}
    if not [r for r in db.get("records", []) if r.get("tier", "read") == "read"]:
        return _missing("databases", out)
    layout = database_blocks(data)
    systems, rows, configs = layout["systems"], layout["rows"], layout["configs"]
    blocks, footnotes = layout["blocks"], layout["footnotes"]
    sizes = db.get("sizes", [])
    baseline = layout["baseline"]
    bounds = {
        field: _cell_bound(
            [
                [(ratio, "") for c in row for ratio in (c[0], c[3])]
                for config in configs
                for row in blocks[(field, config)]
            ],
            "diverging",
        )
        for field, _title, _formatter in DB_HEAT_SPECS
    }

    notes = [
        _statistic_note(data),
        "Ratios to 3DCityDB within one thread configuration only; never read a "
        "threads=single cell against a threads=parallel one.",
        "Uncoloured: no citable baseline or no citable value. n/a: the system does not "
        "run this scenario. error / skipped / mismatch / id-mismatch: the row's status, "
        "printed in the cell; not citable, never zero. *n ok-deviation: citable, see the "
        "note; †n skipped: the reason below.",
    ]
    conditions = data.get("meta", {}).get("conditions", {}).get("databases", [])
    notes += [
        line
        for line in conditions
        if line.startswith(("Spatial windows", "Memory (old evidence)", "Storage no_index_bytes"))
    ]
    if layout["write_rows"]:
        notes.append(
            "CityParquet (DuckDB) write cells are stacked: the upper line and upper half "
            "are in-engine (the table inside DuckDB), the lower line (+wb) and lower half "
            "add the package write-back (duckdb-cityparquet-writeback); each carries its own "
            "ratio. The write tier runs once, under threads=single, so its "
            "threads=parallel cells are n/a."
        )
        notes += [line for line in conditions if line.startswith(prep.DB_WRITE_NOTE_PREFIXES)]
    row_labels = {"lod-query": lod_query_label(db["params"])} if "lod-query" in rows else {}
    for (query, config, deviation), number in sorted(footnotes.items(), key=lambda kv: kv[1]):
        notes.append(f"*{number} ok-deviation, {row_labels.get(query) or _label(query)}, threads={config}: {deviation}")
    for (query, reason), number in sorted(layout["skips"].items(), key=lambda kv: kv[1]):
        notes.append(f"†{number} skipped, {row_labels.get(query) or _label(query)}: {reason}")

    n_cols = len(configs)
    width = max(8.5, 1.1 * len(systems) * n_cols + 3.0)
    row_height = 0.3 * len(rows) + 0.9
    notes_height = 0.2 + 0.13 * sum(1 + len(line) // 170 for line in notes)
    fig = plt.figure(
        figsize=(width, 1.9 + 2 * row_height + notes_height + 0.2), layout="constrained"
    )
    grid = fig.add_gridspec(
        4, n_cols, height_ratios=(1.9, row_height, row_height, notes_height), wspace=0.05
    )
    storage = fig.add_subplot(grid[0, :])
    _db_storage(storage, sizes, systems, baseline)

    separator = rows.index(DB_WRITE_SEPARATOR) if DB_WRITE_SEPARATOR in rows else None
    statistic, memory_title = _statistic(data).capitalize(), _db_memory_title(db)
    for mi, (field, title, _formatter) in enumerate(DB_HEAT_SPECS):
        axes = []
        cmap, norm = _heat_colors("diverging", bounds[field])
        for ci, config in enumerate(configs):
            ax = fig.add_subplot(grid[1 + mi, ci])
            axes.append(ax)
            block = blocks[(field, config)]
            _heat(
                ax,
                [[(c[0], c[1]) for c in row] for row in block],
                rows,
                systems,
                f"{title.format(statistic=statistic, memory=memory_title)}\n"
                f"{THREAD_TITLES.get(config, config)}",
                vmax=bounds[field],
                scale="diverging",
                x_rotation=0,
                row_labels=row_labels,
            )
            ax.set_xticks(range(len(systems)), [_db_label(s) for s in systems], fontsize=5.5)
            for text in ax.texts:
                text.set_fontsize(5.5)
            # A stacked write cell: the lower half takes the write-back ratio's colour.
            for y, row in enumerate(block):
                for x, (_ratio_top, _text, stacked, low) in enumerate(row):
                    if not stacked:
                        continue
                    colour = cmap(norm(math.log2(low))) if low and low > 0 else BAD_CELL
                    ax.add_patch(
                        plt.Rectangle((x - 0.5, y), 1, 0.5, color=colour, linewidth=0, zorder=1)
                    )
                    ax.plot([x - 0.5, x + 0.5], [y, y], color=CELL_EDGE, lw=0.4, zorder=2)
            if separator is not None:
                ax.add_patch(
                    plt.Rectangle(
                        (-0.5, separator - 0.5), len(systems), 1, color=BG, linewidth=0, zorder=2.5
                    )
                )
                ax.axhline(separator - 0.5, color=INK, linewidth=0.9, zorder=3)
                ax.yaxis.get_major_ticks()[separator].tick1line.set_visible(False)
                if ci == 0:
                    ax.text(
                        -0.45,
                        separator + 0.1,
                        "write tier",
                        ha="left",
                        va="center",
                        fontsize=6,
                        style="italic",
                        color=MUTED,
                        zorder=4,
                    )
            ax.tick_params(axis="y", labelsize=6.5)
            if ci:
                ax.set_yticks([])
        cbar = fig.colorbar(
            axes[-1].images[0], ax=axes, orientation="vertical", fraction=0.02, pad=0.01
        )
        ticks = _heat_ticks(bounds[field], "diverging")
        cbar.set_ticks(ticks)
        cbar.set_ticklabels([_ratio_from_log2(t) for t in ticks])
        cbar.ax.tick_params(labelsize=6, length=0)
        cbar.outline.set_visible(False)
        cbar.set_label("Ratio to 3DCityDB; lower is better", fontsize=6)

    _notes_axis(fig.add_subplot(grid[3, :]), notes)
    title = f"Database comparison — {db.get('dataset', '')}"
    if db.get("objects"):
        title += f", {int(db['objects']):,} objects"
    fig.suptitle(title, fontsize=12, x=0.01, ha="left")
    return _save(fig, "databases", out)


def main(data_path: Path | None = None, out_dir: Path | None = None) -> Path:
    data = _load(data_path or DEFAULT_DATA_PATH)
    out = out_dir or DEFAULT_FIGURES_DIR
    plt.rcParams.update(
        {
            "font.family": "sans-serif",
            "font.sans-serif": [
                "Avenir Next",
                "Helvetica Neue",
                "Helvetica",
                "Arial",
                "DejaVu Sans",
            ],
            "figure.facecolor": BG,
            "axes.facecolor": BG,
            "savefig.facecolor": BG,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "svg.hashsalt": "benchviz",
        }
    )
    written = sizes(data, out) + format_figures(data, out) + tables.write_tables(data, out)
    written += _axis_main(data, "bloom", out) + _remove_retired(out)
    written += databases(data, out)
    written += network.render(data, out)
    print(f"benchviz figures -> {out}")
    for path in written:
        print(f"  {path.name}")
    return out
