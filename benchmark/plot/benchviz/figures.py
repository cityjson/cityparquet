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

from .paths import DEFAULT_DATA_PATH, DEFAULT_FIGURES_DIR

BG, INK, MUTED, ACCENT = "#fffff8", "#111111", "#666666", "#c53b35"
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
FORMATS = ["cityparquet-hilbert", "citygml", "cityjson", "flatcitybuf", "cityjsonseq"]
QUERIES = ["full-read", "bbox-1pct", "bbox-5pct", "bbox-25pct", "id-50pct", "id-lookup"]


def _load(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def _label(value: str) -> str:
    return LABELS.get(value, value.replace("-", " ").title())


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


def _heat(
    ax: Axes,
    cells: list[list[tuple[float | None, str]]],
    rows: list[str],
    columns: list[str],
    title: str,
) -> None:
    cmap = plt.get_cmap("RdYlGn_r").with_extremes(bad="#e5e2d9")
    vals = [[math.log2(v) if v and v > 0 else math.nan for v, _ in row] for row in cells]
    ax.imshow(vals, cmap=cmap, norm=colors.TwoSlopeNorm(vmin=-3, vcenter=0, vmax=3), aspect="auto")
    ax.set_title(title, fontsize=9, loc="left")
    ax.set_xticks(
        range(len(columns)), [_label(c) for c in columns], rotation=45, ha="right", fontsize=6
    )
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
    formats = list(FORMATS)
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
        bars = ax.bar(
            range(len(formats)),
            [float(v) / 1024**2 if v is not None else math.nan for v in values],
            color=[ACCENT if f == "cityparquet-hilbert" else "#777777" for f in formats],
        )
        ax.set_title(_title(dataset), fontsize=8)
        ax.set_ylabel("MiB", fontsize=7)
        ax.set_xticks(
            range(len(formats)), [_label(f) for f in formats], rotation=35, ha="right", fontsize=6
        )
        for x, (bar, value) in enumerate(zip(bars, values, strict=False)):
            if value is None:
                ax.text(x, 0, "missing", ha="center", va="bottom", fontsize=5)
            else:
                ratio = _ratio(value, base)
                ax.text(
                    x,
                    bar.get_height(),
                    f"{_mib(value)}\n{ratio:.2g}×" if ratio else _mib(value),
                    ha="center",
                    va="bottom",
                    fontsize=5,
                )
    for ax in list(axes.flat)[len(datasets) :]:
        ax.axis("off")
    fig.suptitle("File size on disk — multiples of CityJSONSeq", x=0.01, ha="left", fontsize=12)
    return _save(fig, "sizes", out)


def format_heatmap(data: dict[str, Any], out: Path) -> list[Path]:
    """Stack the wide read matrices so every query retains a legible cell."""
    datasets = data.get("datasets", [])
    if not datasets or not data.get("read"):
        return _missing("heatmap", out)
    formats = list(FORMATS)
    records = data.get("read", [])
    queries = sorted(
        {
            r.get("scenario_key")
            for r in records
            if r.get("scenario_key") and r.get("scenario_key") != "write"
        }
    )
    queries = queries or ["read"]
    # A complete query matrix is deliberately a tall standalone sheet. Its
    # width is fixed; adding datasets increases height, never shrinks labels.
    fig = plt.figure(figsize=(8.5, 4.15 * len(datasets)), layout="constrained")
    subfigures = fig.subfigures(nrows=len(datasets), ncols=1, squeeze=False)
    for i, dataset in enumerate(datasets):
        subfig = subfigures[i, 0]
        subfig.suptitle(_title(dataset), fontsize=11, x=0.01, ha="left")
        grid = subfig.add_gridspec(3, 2, height_ratios=[1, 1, 1])
        axes = [
            subfig.add_subplot(grid[0, 0]),
            subfig.add_subplot(grid[0, 1]),
            subfig.add_subplot(grid[1, :]),
            subfig.add_subplot(grid[2, :]),
        ]
        index = {
            (r.get("format"), r.get("scenario_key")): r
            for r in records
            if r.get("dataset") == dataset.get("id")
        }
        for column, (field, scenario, title) in enumerate(
            (
                ("time_s", "write", "Write time (s)"),
                ("rss_b", "write", "Write peak RSS (MiB)"),
                ("time_s", None, "Read time (s)"),
                ("rss_b", None, "Read peak RSS (MiB)"),
            )
        ):
            ax = axes[column]
            columns = [scenario] if scenario else queries
            matrix = []
            for fmt in formats:
                row = []
                for query in columns:
                    value = index.get((fmt, query), {})
                    base = index.get(("cityjsonseq", query), {})

                    def valid(record: dict) -> bool:
                        notes = str(record.get("notes", ""))
                        return record.get("status", "ok") in (None, "", "ok") and not (
                            notes.startswith(("error", "skipped")) or "mismatch" in notes
                        )

                    measured = value.get(field) if valid(value) else None
                    baseline = base.get(field) if valid(base) else None
                    if measured is None:
                        label = "—"
                    else:
                        number = float(measured) / (1024**2 if field == "rss_b" else 1)
                        label = f"{number:.2g}"
                    row.append((_ratio(measured, baseline), label))
                matrix.append(row)
            _heat(ax, matrix, formats, columns, title)
            for text in ax.texts:
                text.set_fontsize(7)
            ax.tick_params(axis="y", labelsize=7)
            ax.tick_params(axis="x", labelsize=6.5)
            if scenario or column == 2:
                ax.set_xticks([])
            if column == 1:
                ax.set_yticks([])
        colourbar = subfig.colorbar(axes[-1].images[0], ax=axes, shrink=0.7, pad=0.02)
        colourbar.set_ticks([-3, -2, -1, 0, 1, 2, 3])
        colourbar.set_ticklabels(["≤0.125×", "0.25×", "0.5×", "1×", "2×", "4×", "≥8×"])
        colourbar.ax.tick_params(labelsize=6)
        colourbar.set_label("Ratio to CityJSONSeq; lower is better", fontsize=7)
    fig.suptitle("Format comparison", fontsize=13, x=0.01, ha="left")
    return _save(fig, "heatmap", out)


def _axis(data: dict[str, Any], key: str) -> tuple[list[dict], list[dict], list[str]]:
    axis = data.get("scaling", {}).get(key, {})
    return axis.get("records", []), axis.get("sizes", []), axis.get("variants", [])


def _axis_queries(records: list[dict]) -> list[str]:
    wanted = ["full-read", "bbox-1pct", "bbox-5pct", "bbox-25pct", "id-50pct", "id-lookup"]
    present = {r.get("measure") for r in records}
    return [q for q in wanted if q in present]


def _axis_main(data: dict[str, Any], key: str, out: Path) -> list[Path]:
    records, sizes, variants = _axis(data, key)
    if not records:
        return _missing(key, out)
    largest = max(
        {r["dataset"] for r in records},
        key=lambda d: max(r.get("objects") or 0 for r in records if r["dataset"] == d),
    )
    selected = [r for r in records if r.get("dataset") == largest]
    queries = _axis_queries(selected)
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
            range(len(variants)), [v if v is not None else float("nan") for v in vals], color=ACCENT
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
                ax.text(x, v, f"{actual:.3g}", ha="center", va="bottom", fontsize=5)
    image = None
    lower = grid[1, :].subgridspec(1, 2)
    for col, (field, title) in enumerate(
        (("time_s", "read time (s)"), ("rss_b", "read peak RSS (MiB)"))
    ):
        ax = fig.add_subplot(lower[col])
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
                text = (
                    (
                        f"{float(actual) / (1024**2):.3g}"
                        if field == "rss_b"
                        else f"{float(actual):.3g}"
                    )
                    if actual is not None
                    else "—"
                )
                row.append((_ratio(actual, b.get(field) if b else None), text))
            cells.append(row)
        short_variants = [
            v.replace("cityparquet+", "").replace("cityparquet", "default") for v in variants
        ]
        _heat(ax, cells, short_variants, queries, title)
        if col == 1:
            ax.set_yticks([])
        image = ax.images[0]
    cbar = fig.colorbar(image, cax=fig.add_axes([0.90, 0.25, 0.02, 0.5]))
    cbar.set_ticks([-3, -2, -1, 0, 1, 2, 3])
    cbar.set_ticklabels(["≤0.125×", "0.25×", "0.5×", "1×", "2×", "4×", "≥8×"])
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
    queries = _axis_queries(records)
    counts = sorted({r["objects"] for r in records if r.get("objects") is not None})
    colours = {variant: plt.get_cmap("tab10")(i % 10) for i, variant in enumerate(variants)}
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
            by = {
                r.get("objects"): r
                for r in source
                if r.get("variant") == variant and (measure is None or r.get("measure") == measure)
            }
            divisor = 1024**2 if field in ("bytes", "rss_b") else 1
            values = [
                float(by[count][field]) / divisor
                if count in by and by[count].get(field) is not None
                else float("nan")
                for count in counts
            ]
            ax.plot(
                counts,
                values,
                color=colours[variant],
                marker=markers[vi % len(markers)],
                markersize=3,
                linewidth=0.9,
                label=variant.replace("cityparquet+", "").replace("cityparquet", "default"),
            )
            if field == "time_s":
                spreads = [
                    float(by[count].get("time_mad_s") or 0) if count in by else 0
                    for count in counts
                ]
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


def _missing(name: str, out: Path) -> list[Path]:
    fig, ax = plt.subplots(figsize=(6, 2.2))
    ax.axis("off")
    ax.text(0.5, 0.58, name, ha="center", va="center", fontsize=12)
    ax.text(
        0.5,
        0.35,
        "Not rendered: this result family is absent from this run.",
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
        color=[ACCENT if s == baseline else "#777777" for s in systems],
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
    for ax, field, title, formatter in (
        (time_ax, "time_s", "Mean query time", _seconds),
        (rss_ax, "peak_rss_bytes", "Peak execution-process RSS", _mib),
    ):
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
                    row.append((ratio, f"{formatter(metric)}\n{ratio:.2g}×"))
                else:
                    row.append((None, (value or {}).get("status") or "missing"))
            cells.append(row)
        _heat(ax, cells, systems, queries, title)
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
            "font.family": "DejaVu Sans",
            "figure.facecolor": BG,
            "axes.facecolor": BG,
            "savefig.facecolor": BG,
            "axes.spines.top": False,
            "axes.spines.right": False,
        }
    )
    written = sizes(data, out) + format_heatmap(data, out)
    for key in ("codec", "rowgroup"):
        written += _axis_main(data, key, out) + _axis_scaling(data, key, out)
    written += databases(data, out)
    print(f"benchviz figures -> {out}")
    for path in written:
        print(f"  {path.name}")
    return out
