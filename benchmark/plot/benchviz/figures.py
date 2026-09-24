"""Paper-oriented static figures from prepared benchmark data."""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.colors as colors
import matplotlib.pyplot as plt
from matplotlib.axes import Axes
from matplotlib.cm import ScalarMappable

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
    "cityparquet-hilbert": "#E4572E",
    "citygml": "#C2CAD0",
    "cityjson": "#A3AEB7",
    "cityjsonseq": "#83919C",
    "flatcitybuf": "#5F6E7A",
    "cityjsonseq-gz": "#D6DBDF",
    "duckdb-parquet": "#D6DBDF",
}
DATABASE_FILL = {
    "duckdb-cityparquet": "#E4572E",
    "cjdb": "#83919C",
    "3dcitydb": "#C2CAD0",
}
# The configuration axes' bars: the zstd sweep is one family in the accent hue
# at four lightness steps; the other codecs split one teal accent and a neutral
# ramp, so the sweep reads as the subject without a second rainbow. Row groups
# are one sequential teal from small groups (light) to large (dark).
CODEC_OTHER_COLOURS = ["#2A9D8F", "#5F6E7A", "#83919C", "#A3AEB7", "#C2CAD0"]
ROWGROUP_HUE = "#2A9D8F"
BLOOM_OFF_COLOUR = "#5F6E7A"  # the package without filters, against the accent default

BAD_CELL = "#efeee6"
# Cell separators. A black grid would fight the fills, which are the reading;
# a pale warm rule only tells the eye where one cell ends.
CELL_EDGE = "#e6e3d7"
# Ratio cells share one vocabulary: teal beats the baseline, the page colour is
# the baseline, the warm accent is worse. The write metrics are never cheaper
# than the streaming baseline, so they use a one-sided version of the same ramp
# rather than spending the teal half of a diverging map on values that never
# occur.
CMAP_DIVERGING = colors.LinearSegmentedColormap.from_list("cp_ratio", ["#2A9D8F", BG, ACCENT])
CMAP_COST = colors.LinearSegmentedColormap.from_list("cp_cost", [BG, "#F3B199", ACCENT])

LABELS = {
    "cityparquet-hilbert": "CityParquet",
    "cityparquet": "CityParquet (source order)",
    "cityjsonseq": "CityJSONSeq",
    "citygml": "CityGML",
    "cityjson": "CityJSON",
    "flatcitybuf": "FlatCityBuf",
    "3dcitydb": "3DCityDB",
    "cjdb": "cjdb",
    "citylake": "CityParquet / DuckDB",
    "duckdb-cityparquet": "CityParquet / DuckDB",
}
# Query keys read on the heatmap's row axis; a key without an entry falls back
# to a title-cased key.
SCENARIO_LABELS = {
    "write": "Write",
    "full-read": "Full read",
    "count": "Count",
    "project": "Project",
    "id-lookup": "Id lookup",
    "id-miss": "Id miss",
    "attr-filter": "Attr filter",
    "attr-stats": "Attr stats",
    "bbox-1pct": "Area 1%",
    "bbox-5pct": "Area 5%",
    "bbox-25pct": "Area 25%",
    "id-10pct": "Id 10%",
    "id-50pct": "Id 50%",
    "id-90pct": "Id 90%",
    "feature-50pct": "Feature 50%",
    "feature-miss": "Feature miss",
}
# Both the size and heatmap sheets read from the reference encodings to ours, so
# CityParquet is the last bar/row and the eye lands on it.
FIGURE_FORMATS = ["citygml", "cityjson", "cityjsonseq", "flatcitybuf", "cityparquet-hilbert"]
# Preferred scenario order for the heatmap rows: the whole-table read first,
# then the spatial probes narrow-to-wide, the attribute probes, then the id
# probes. A run that measured something else appends it after these.
QUERIES = [
    "full-read",
    "count",
    "bbox-1pct",
    "bbox-5pct",
    "bbox-25pct",
    "attr-filter",
    "attr-stats",
    "project",
    "id-10pct",
    "id-50pct",
    "id-90pct",
    "id-lookup",
    "id-miss",
]


def _load(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def _label(value: str) -> str:
    return LABELS.get(value) or SCENARIO_LABELS.get(value) or value.replace("-", " ").title()


def _mib(value: Any) -> str:
    if value is None:
        return "—"
    return f"{float(value) / 1024**2:.1f} MiB"


def _seconds(value: Any) -> str:
    if value is None:
        return "—"
    value = float(value)
    return f"{value * 1000:.1f} ms" if value < 1 else f"{value:.2f} s"


def _ratio(value: Any, base: Any) -> float | None:
    return float(value) / float(base) if value is not None and base not in (None, 0) else None


def _size_text(value: Any) -> str:
    """A byte count in the unit a reader expects: GB once it is a GB."""
    if value is None:
        return "—"
    value = float(value)
    if value >= 1024**3:
        return f"{value / 1024**3:.2f} GB"
    return f"{value / 1024**2:.1f} MB"


def _size_unit(values: list[Any]) -> str:
    """The unit a whole panel is drawn in: GB when its largest bar needs one."""
    known = [float(v) for v in values if v is not None]
    return "GB" if known and max(known) >= 1024**3 else "MB"


def _format_fill(fmt: str) -> str:
    return FORMAT_FILL.get(fmt, MUTED)


def _mix(colour: str, white: float) -> str:
    r, g, b = colors.to_rgb(colour)
    return colors.to_hex((r + (1 - r) * white, g + (1 - g) * white, b + (1 - b) * white))


def _rowgroup_size(variant: str) -> int:
    suffix = variant.removeprefix("cityparquet+")
    if suffix.startswith("rg") and suffix[2:].isdigit():
        return int(suffix[2:])
    return 0


def _axis_palette(key: str, variants: list[str]) -> dict[str, str]:
    palette: dict[str, str] = {}
    if key == "codec":
        zstd = [v for v in variants if v.startswith("cityparquet+zstd")]
        others = [v for v in variants if v not in zstd and v != "cityparquet"]
        for i, v in enumerate(zstd):
            palette[v] = _mix(ACCENT, 0.55 * (1 - i / max(len(zstd) - 1, 1)))
        for i, v in enumerate(others):
            palette[v] = CODEC_OTHER_COLOURS[i % len(CODEC_OTHER_COLOURS)]
    elif key == "rowgroup":
        ordered = sorted((v for v in variants if v != "cityparquet"), key=_rowgroup_size)
        for i, v in enumerate(ordered):
            palette[v] = _mix(ROWGROUP_HUE, 0.6 * (1 - i / max(len(ordered) - 1, 1)))
    else:
        for v in variants:
            if v != "cityparquet":
                palette[v] = BLOOM_OFF_COLOUR
    return palette


def _save(fig: plt.Figure, name: str, out: Path) -> list[Path]:
    out.mkdir(parents=True, exist_ok=True)
    files = [out / f"{name}.svg", out / f"{name}.png"]
    fig.savefig(files[0], bbox_inches="tight")
    fig.savefig(files[1], dpi=300, bbox_inches="tight")
    plt.close(fig)
    return files


def _title(dataset: dict[str, Any]) -> str:
    count = dataset.get("objects") or dataset.get("object_count")
    name = dataset.get("title") or dataset.get("id")
    suffix = f" — {int(count):,} objects" if count else ""
    return f"{name}{suffix}"


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
    ax.set_yticks(range(len(rows)), [_label(r) for r in rows], fontsize=6)
    for y, row in enumerate(cells):
        for x, (_, text) in enumerate(row):
            ax.text(x, y, text, ha="center", va="center", fontsize=5, color=INK)
    for spine in ax.spines.values():
        spine.set_visible(False)


def sizes(data: dict[str, Any], out: Path) -> list[Path]:
    datasets = data.get("datasets", [])
    if not data.get("sizes"):
        return _missing("sizes", out)
    formats = list(FIGURE_FORMATS)
    nrows = max(1, math.ceil(len(datasets) / 3))
    fig, axes = plt.subplots(
        nrows, 3, figsize=(8.5, 2.8 * nrows), squeeze=False, constrained_layout=True
    )
    for ax, dataset in zip(axes.flat, datasets, strict=False):
        by = {
            r["format"]: r for r in data.get("sizes", []) if r.get("dataset") == dataset.get("id")
        }
        values = [by.get(f, {}).get("bytes") for f in formats]
        base = by.get("cityjsonseq", {}).get("bytes")
        unit = _size_unit(values)
        divisor = 1024**3 if unit == "GB" else 1024**2
        bars = ax.bar(
            range(len(formats)),
            [float(v) / divisor if v is not None else math.nan for v in values],
            color=[_format_fill(f) for f in formats],
        )
        ax.set_title(_title(dataset), fontsize=8)
        ax.set_ylabel(unit, fontsize=7)
        ax.set_xticks(
            range(len(formats)), [_label(f) for f in formats], rotation=35, ha="right", fontsize=6
        )
        ax.tick_params(axis="y", labelsize=5.5)
        for x, (bar, value) in enumerate(zip(bars, values, strict=False)):
            if value is None:
                ax.text(x, 0, "missing", ha="center", va="bottom", fontsize=5)
            else:
                ratio = _ratio(value, base)
                ax.text(
                    x,
                    bar.get_height(),
                    f"{_size_text(value)}\n{ratio:.2g}×" if ratio else _size_text(value),
                    ha="center",
                    va="bottom",
                    fontsize=5,
                )
    for ax in list(axes.flat)[len(datasets) :]:
        ax.axis("off")
    fig.suptitle("File size on disk — multiples of CityJSONSeq", x=0.01, ha="left", fontsize=12)
    return _save(fig, "sizes", out)


def format_heatmap(data: dict[str, Any], out: Path) -> list[Path]:
    """One ratio matrix per metric and dataset, each on its own colour scale."""
    datasets = data.get("datasets", [])
    if not datasets or not data.get("read"):
        return _missing("heatmap", out)
    formats = list(FIGURE_FORMATS)
    records = data.get("read", [])
    present = {
        r.get("scenario_key")
        for r in records
        if r.get("scenario_key") and r.get("scenario_key") != "write"
    }
    queries = [q for q in QUERIES if q in present]
    queries += sorted(present - set(QUERIES))
    queries = queries or ["read"]
    metrics = (
        ("time_s", "write", "Write time (s)", "cost"),
        ("rss_b", "write", "Write peak RSS (MiB)", "cost"),
        ("time_s", None, "Read time (s)", "diverging"),
        ("rss_b", None, "Read peak RSS (MiB)", "diverging"),
    )

    def valid(record: dict) -> bool:
        notes = str(record.get("notes", ""))
        return record.get("status", "ok") in (None, "", "ok") and not (
            notes.startswith(("error", "skipped")) or "mismatch" in notes
        )

    def matrix_for(dataset: dict[str, Any], field: str, scenario: str | None) -> list:
        # Rows are queries, columns are formats: the format axis is short and
        # fixed, so it reads across the top, and the query labels get the tall
        # axis where they fit without turning.
        row_labels = [scenario] if scenario else queries
        index = {
            (r.get("format"), r.get("scenario_key")): r
            for r in records
            if r.get("dataset") == dataset.get("id")
        }
        rows = []
        for query in row_labels:
            row = []
            for fmt in formats:
                value, base = index.get((fmt, query), {}), index.get(("cityjsonseq", query), {})
                measured = value.get(field) if valid(value) else None
                baseline = base.get(field) if valid(base) else None
                ratio = _ratio(measured, baseline)
                if measured is None:
                    label = "—"
                else:
                    number = float(measured) / (1024**2 if field == "rss_b" else 1)
                    # The absolute value and its multiplier, stacked: the colour
                    # gives the pattern, the numbers give the reading.
                    label = f"{number:.3g}\n{_ratio_short(ratio)}" if ratio else f"{number:.3g}"
                row.append((ratio, label))
            rows.append(row)
        return rows

    # A single ratio scale cannot serve four metrics: a write time on a million
    # objects is tens of thousands of times the streaming baseline and would
    # saturate any bound a read metric sets, painting a whole column one colour.
    # Each metric therefore gets its own bound, shared across the datasets.
    metric_matrices = [
        [matrix_for(dataset, field, scenario) for field, scenario, _t, _s in metrics]
        for dataset in datasets
    ]
    bounds = [
        _cell_bound([row for matrices in metric_matrices for row in matrices[mi]], scale)
        for mi, (_field, _scenario, _name, scale) in enumerate(metrics)
    ]

    # A complete query matrix is deliberately a tall standalone sheet. Its
    # width is fixed; adding datasets increases height, never shrinks labels.
    n = len(datasets)
    row_units = max(1, len(queries))
    # Width and height are sized so a two-line cell keeps a margin inside its
    # border: the numbers never touch the rule, at any column count.
    fig = plt.figure(figsize=(10.0, 4.5 * n + 1.3), layout="constrained")
    subfigures = fig.subfigures(
        nrows=n + 1, ncols=1, squeeze=False, height_ratios=[*([4] * n), 1.2]
    )
    for i, dataset in enumerate(datasets):
        subfig = subfigures[i, 0]
        subfig.suptitle(_title(dataset), fontsize=11, x=0.01, ha="left")
        # Write above idle write, read beside read: the two read metrics share
        # the query rows, so they read across; the write metrics share the same
        # format columns and sit directly above.
        grid = subfig.add_gridspec(2, 2, height_ratios=[1.2, row_units], wspace=0.2)
        axes = [
            subfig.add_subplot(grid[0, 0]),
            subfig.add_subplot(grid[0, 1]),
            subfig.add_subplot(grid[1, 0]),
            subfig.add_subplot(grid[1, 1]),
        ]
        for mi, (_field, scenario, title, scale) in enumerate(metrics):
            ax = axes[mi]
            row_labels = [scenario] if scenario else queries
            _heat(
                ax,
                metric_matrices[i][mi],
                row_labels,
                formats,
                title,
                vmax=bounds[mi],
                scale=scale,
                x_rotation=0,
            )
            for text in ax.texts:
                text.set_fontsize(5.8)
            ax.tick_params(axis="y", labelsize=7)
            ax.tick_params(axis="x", labelsize=6.5)
            # Format labels live under the read row only; the write row shares
            # its columns. Query labels live left of the read-time panel only.
            if mi in (0, 1):
                ax.set_xticks([])
            if mi in (1, 3):
                ax.set_yticks([])
    key = subfigures[n, 0]
    key.suptitle(
        "Cell text: absolute value over ×ratio to CityJSONSeq; lower is better",
        fontsize=8,
        x=0.01,
        ha="left",
    )
    key_grid = key.add_gridspec(1, len(metrics), wspace=0.55)
    for mi, (_field, _scenario, title, scale) in enumerate(metrics):
        cax = key.add_subplot(key_grid[0, mi])
        cmap, norm = _heat_colors(scale, bounds[mi])
        bar = key.colorbar(ScalarMappable(norm=norm, cmap=cmap), cax=cax, orientation="horizontal")
        ticks = _heat_ticks(bounds[mi], scale)
        bar.set_ticks(ticks)
        bar.set_ticklabels([_ratio_from_log2(t) for t in ticks])
        bar.ax.tick_params(labelsize=6, length=0)
        bar.outline.set_visible(False)
        bar.set_label(title, fontsize=7)
    fig.suptitle("Format comparison", fontsize=13, x=0.01, ha="left")
    return _save(fig, "heatmap", out)


def _axis(data: dict[str, Any], key: str) -> tuple[list[dict], list[dict], list[str]]:
    axis = data.get("scaling", {}).get(key, {})
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


def _scaling_points(source: list[dict], variant: str, measure: str | None) -> list[dict]:
    """One variant's curve: the nested 3DBAG slices only, one point per dataset.

    Keyed by dataset, never by object count, so two inputs of equal count both
    stay; ordered by count (then id) for the x axis. Corpus rows are never
    points on a curve — a different city model is not a larger slice.
    """
    points = [
        r
        for r in source
        if r.get("series") == "scaling"
        and r.get("variant") == variant
        and (measure is None or r.get("measure") == measure)
        and r.get("objects") is not None
    ]
    return sorted(points, key=lambda r: (r["objects"], r["dataset"]))


def _corpus_datasets(records: list[dict]) -> list[str]:
    """The corpus datasets an axis measured, smallest first."""
    objects: dict[str, int] = {}
    for r in records:
        if r.get("series") == "corpus":
            objects[r["dataset"]] = max(objects.get(r["dataset"], 0), r.get("objects") or 0)
    return sorted(objects, key=lambda d: (objects[d], d))


def _axis_main(data: dict[str, Any], key: str, out: Path) -> list[Path]:
    records, sizes, variants = _axis(data, key)
    if not records:
        return _missing(key, out)
    # The headline dataset is the largest SLICE when the axis has any: the
    # scaling figure's right-hand end, not a corpus model of another city.
    slices = [r for r in records if r.get("series") == "scaling"]
    largest = max(
        {r["dataset"] for r in (slices or records)},
        key=lambda d: max(r.get("objects") or 0 for r in records if r["dataset"] == d),
    )
    selected = [r for r in records if r.get("dataset") == largest]
    queries = _axis_queries(selected)
    palette = _axis_palette(key, variants)
    fig = plt.figure(figsize=(10, 7))
    grid = fig.add_gridspec(
        2,
        3,
        height_ratios=[1, 1.2],
        left=0.12,
        right=0.86,
        bottom=0.14,
        top=0.88,
        hspace=0.65,
        wspace=0.3,
    )
    for col, (title, source, _ratio_field, value, measure) in enumerate(
        (
            (
                "size (MiB)",
                [r for r in sizes if r.get("dataset") == largest],
                "size_ratio",
                "bytes",
                None,
            ),
            ("write time (s)", selected, "time_ratio", "time_s", "write"),
            ("write peak RSS (MiB)", selected, "rss_ratio", "rss_b", "write"),
        )
    ):
        ax = fig.add_subplot(grid[0, col])
        by = {r.get("variant"): r for r in source if measure is None or r.get("measure") == measure}
        divisor = 1024**2 if value in ("bytes", "rss_b") else 1
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
                    actual /= 1024**2
                ax.text(x, v, _compact(actual), ha="center", va="bottom", fontsize=4.5)
    short_variants = [
        v.replace("cityparquet+", "").replace("cityparquet", "default") for v in variants
    ]
    lower = grid[1, :].subgridspec(1, 2)
    heat_axes = [fig.add_subplot(lower[0]), fig.add_subplot(lower[1])]
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
                if actual is None:
                    text = "—"
                else:
                    number = float(actual) / (1024**2 if field == "rss_b" else 1)
                    text = f"{number:.3g}\n{_ratio_short(ratio)}" if ratio else f"{number:.3g}"
                row.append((ratio, text))
            cells.append(row)
        cell_blocks.append(cells)
    # Both rows are read ratios against the default write, so one diverging
    # bound can serve them; the cells carry the precision either way.
    bound = _cell_bound([row for block in cell_blocks for row in block], "diverging")
    for col, (ax, cells, title) in enumerate(
        zip(heat_axes, cell_blocks, ("read time (s)", "read peak RSS (MiB)"), strict=True)
    ):
        _heat(ax, cells, short_variants, queries, title, vmax=bound, scale="diverging")
        for text in ax.texts:
            text.set_fontsize(5.5)
        ax.tick_params(axis="y", labelsize=6)
        ax.tick_params(axis="x", labelsize=6)
        if col == 1:
            ax.set_yticks([])
    cbar = fig.colorbar(heat_axes[-1].images[0], cax=fig.add_axes([0.90, 0.25, 0.02, 0.5]))
    ticks = _heat_ticks(bound, "diverging")
    cbar.set_ticks(ticks)
    cbar.set_ticklabels([_ratio_from_log2(t) for t in ticks])
    cbar.ax.tick_params(labelsize=6, length=0)
    cbar.outline.set_visible(False)
    cbar.set_label("Ratio to default; lower is better", fontsize=7)
    fig.suptitle(
        f"{key.replace('rowgroup', 'row group')} — "
        f"{max(r.get('objects') or 0 for r in selected):,} objects",
        x=0.01,
        ha="left",
        fontsize=11,
    )
    return _save(fig, key, out)


def _axis_scaling(data: dict[str, Any], key: str, out: Path) -> list[Path]:
    records, sizes, variants = _axis(data, key)
    if not records:
        return _missing(f"{key}-scaling", out)
    if not any(r.get("series") == "scaling" for r in records):
        return _missing(
            f"{key}-scaling",
            out,
            "Not rendered: no 3DBAG scaling slice was measured; corpus datasets are drawn apart.",
        )
    queries = _axis_queries(records)
    palette = _axis_palette(key, variants)
    colours = {variant: palette.get(variant, MUTED) for variant in variants}
    markers = ["o", "s", "^", "D", "v", "P", "X", "<", ">"]
    fig = plt.figure(figsize=(max(8.5, 2.0 * len(queries)), 7.2), layout="constrained")
    outer = fig.add_gridspec(3, 1)
    top = outer[0].subgridspec(1, 3)
    panels = [
        (fig.add_subplot(top[i]), title, source, field, measure)
        for i, (title, source, field, measure) in enumerate(
            (
                ("File size (MiB)", sizes, "bytes", None),
                ("Write time (s)", records, "time_s", "write"),
                ("Write peak RSS (MiB)", records, "rss_b", "write"),
            )
        )
    ]
    for row, (field, title) in enumerate(
        (("time_s", "Read time (s)"), ("rss_b", "Read peak RSS (MiB)")), 1
    ):
        subgrid = outer[row].subgridspec(1, max(1, len(queries)))
        for i, query in enumerate(queries):
            panels.append(
                (fig.add_subplot(subgrid[i]), f"{title}\n{_label(query)}", records, field, query)
            )
    for ax, title, source, field, measure in panels:
        for vi, variant in enumerate(variants):
            points = _scaling_points(source, variant, measure)
            counts = [r["objects"] for r in points]
            divisor = 1024**2 if field in ("bytes", "rss_b") else 1
            values = [
                float(r[field]) / divisor if r.get(field) is not None else float("nan")
                for r in points
            ]
            ax.plot(
                counts,
                values,
                color=colours[variant],
                marker=markers[vi % len(markers)],
                markersize=3,
                linewidth=1.6 if variant == "cityparquet" else 0.9,
                label=variant.replace("cityparquet+", "").replace("cityparquet", "default"),
            )
            if field == "time_s":
                spreads = [float(r.get("time_std_s") or 0) for r in points]
                ax.fill_between(
                    counts,
                    [v - spread for v, spread in zip(values, spreads, strict=True)],
                    [v + spread for v, spread in zip(values, spreads, strict=True)],
                    color=colours[variant],
                    alpha=0.08,
                )
        ax.set_title(title, fontsize=8)
        ax.set_xlabel("CityObjects", fontsize=7)
        ax.set_xscale("log")
        ax.tick_params(labelsize=6)
    handles, labels = panels[0][0].get_legend_handles_labels()
    fig.legend(
        handles,
        labels,
        loc="outside lower center",
        ncol=min(5, len(variants)),
        fontsize=7,
        frameon=False,
    )
    fig.suptitle(f"{key.replace('rowgroup', 'row group').capitalize()} scaling", fontsize=12)
    return _save(fig, f"{key}-scaling", out)


def _axis_corpus(data: dict[str, Any], key: str, out: Path) -> list[Path]:
    """The corpus datasets of an axis, per dataset and apart from the curve.

    Grouped bars, one group per corpus dataset and one bar per variant, for
    the same metrics as the scaling figure. Nothing is written for an axis
    that measured no corpus dataset (codec and row group run slices only), and
    any `{key}-corpus` figure already in `out` is removed.
    """
    records, sizes, variants = _axis(data, key)
    datasets = _corpus_datasets(records)
    if not datasets:
        # Remove a corpus figure an earlier run left in a re-used directory,
        # so the summary page cannot embed it as if this run had measured it.
        for suffix in ("svg", "png"):
            (out / f"{key}-corpus.{suffix}").unlink(missing_ok=True)
        return []
    corpus = [r for r in records if r.get("series") == "corpus"]
    corpus_sizes = [r for r in sizes if r.get("series") == "corpus"]
    queries = _axis_queries(corpus)
    palette = _axis_palette(key, variants)
    metrics = [
        ("File size (MiB)", corpus_sizes, "bytes", None),
        ("Write time (s)", corpus, "time_s", "write"),
        ("Write peak RSS (MiB)", corpus, "rss_b", "write"),
    ] + [(f"Read time (s)\n{_label(q)}", corpus, "time_s", q) for q in queries]
    columns = 3
    rows = -(-len(metrics) // columns)
    fig, axes = plt.subplots(
        rows, columns, figsize=(10, 2.6 * rows), layout="constrained", squeeze=False
    )
    width = 0.8 / max(1, len(variants))
    for ax, (title, source, field, measure) in zip(axes.flat, metrics, strict=False):
        divisor = 1024**2 if field in ("bytes", "rss_b") else 1
        for vi, variant in enumerate(variants):
            by = {
                r["dataset"]: r
                for r in source
                if r.get("variant") == variant and (measure is None or r.get("measure") == measure)
            }
            values = [
                float(by[d][field]) / divisor
                if d in by and by[d].get(field) is not None
                else float("nan")
                for d in datasets
            ]
            ax.bar(
                [i + (vi - (len(variants) - 1) / 2) * width for i in range(len(datasets))],
                values,
                width=width,
                color=palette.get(variant, MUTED),
                label=variant.replace("cityparquet+", "").replace("cityparquet", "default"),
            )
        ax.set_title(title, fontsize=8)
        ax.set_xticks(range(len(datasets)), datasets, rotation=30, ha="right", fontsize=6)
        ax.tick_params(axis="y", labelsize=6)
    for ax in list(axes.flat)[len(metrics) :]:
        ax.axis("off")
    handles, labels = axes.flat[0].get_legend_handles_labels()
    fig.legend(
        handles,
        labels,
        loc="outside lower center",
        ncol=min(5, len(variants)),
        fontsize=7,
        frameon=False,
    )
    fig.suptitle(
        f"{key.replace('rowgroup', 'row group').capitalize()} — corpus datasets",
        fontsize=12,
    )
    return _save(fig, f"{key}-corpus", out)


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


def databases(data: dict[str, Any], out: Path) -> list[Path]:
    db = data.get("databases") or {}
    records, sizes = db.get("records", []), db.get("sizes", [])
    if not records:
        return _missing("databases", out)
    baseline = "3dcitydb"
    systems = [
        s
        for s in ("duckdb-cityparquet", "cjdb", "3dcitydb")
        if any(r.get("format") == s for r in records)
    ]
    queries = sorted({r.get("scenario") for r in records if r.get("scenario")})
    index = {(r.get("format"), r.get("scenario")): r for r in records}
    fig = plt.figure(figsize=(8.5, 7.0), constrained_layout=True)
    grid = fig.add_gridspec(3, 1, height_ratios=(1.0, 1.45, 1.45))
    storage = fig.add_subplot(grid[0])
    time_ax = fig.add_subplot(grid[1])
    rss_ax = fig.add_subplot(grid[2])
    by_size = {r.get("format"): r.get("size_bytes") for r in sizes}
    base_size = by_size.get(baseline)
    values = [by_size.get(system) for system in systems]
    bars = storage.bar(
        range(len(systems)),
        [float(v) / 1024**2 if v is not None else math.nan for v in values],
        color=[DATABASE_FILL.get(s, MUTED) for s in systems],
    )
    storage.set_title("Storage including indexes", loc="left", fontsize=9)
    storage.set_ylabel("MiB", fontsize=7)
    storage.set_xticks(range(len(systems)), [_label(s) for s in systems], fontsize=7)
    storage.tick_params(axis="y", labelsize=6)
    for x, (bar, value) in enumerate(zip(bars, values, strict=False)):
        if value is None:
            storage.text(x, 0, "missing", ha="center", va="bottom", fontsize=6)
        else:
            ratio = _ratio(value, base_size)
            detail = _mib(value) + (f" · {ratio:.2g}×" if ratio else "")
            storage.text(x, bar.get_height(), detail, ha="center", va="bottom", fontsize=6)
    heat_specs = (
        (time_ax, "time_s", "Mean query time", _seconds),
        (rss_ax, "peak_rss_bytes", "Peak execution-process RSS", _mib),
    )
    cell_blocks = []
    for _ax, field, _title, formatter in heat_specs:
        cells = []
        for system in systems:
            row = []
            for query in queries:
                value, base = index.get((system, query)), index.get((baseline, query))
                valid = (
                    value and base and value.get("status") == "ok" and base.get("status") == "ok"
                )
                metric = value.get(field) if value else None
                base_metric = base.get(field) if base else None
                if valid and metric is not None and base_metric not in (None, 0):
                    ratio = _ratio(metric, base_metric)
                    row.append((ratio, f"{formatter(metric)}\n{_ratio_short(ratio)}"))
                else:
                    row.append((None, (value or {}).get("status") or "missing"))
            cells.append(row)
        cell_blocks.append(cells)
    bound = _cell_bound([row for block in cell_blocks for row in block], "diverging")
    for (ax, _field, title, _formatter), cells in zip(heat_specs, cell_blocks, strict=True):
        _heat(ax, cells, systems, queries, title, vmax=bound, scale="diverging")
        for text in ax.texts:
            text.set_fontsize(6)
        ax.tick_params(axis="y", labelsize=6)
        ax.tick_params(axis="x", labelsize=6)
    cbar = fig.colorbar(
        rss_ax.images[0],
        ax=[time_ax, rss_ax],
        orientation="vertical",
        fraction=0.035,
        pad=0.02,
        aspect=28,
    )
    ticks = _heat_ticks(bound, "diverging")
    cbar.set_ticks(ticks)
    cbar.set_ticklabels([_ratio_from_log2(t) for t in ticks])
    cbar.ax.tick_params(labelsize=6, length=0)
    cbar.outline.set_visible(False)
    cbar.set_label("Ratio to 3DCityDB; lower is better", fontsize=7)
    fig.text(
        0.01,
        0.01,
        "3DCityDB baseline. Cells with missing, failed, or invalid-baseline "
        "measurements are not coloured.",
        fontsize=6,
        color=MUTED,
    )
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
        }
    )
    written = sizes(data, out) + format_heatmap(data, out)
    for key in ("codec", "rowgroup", "bloom"):
        written += _axis_main(data, key, out) + _axis_scaling(data, key, out)
        written += _axis_corpus(data, key, out)
    written += databases(data, out)
    print(f"benchviz figures -> {out}")
    for path in written:
        print(f"  {path.name}")
    return out
