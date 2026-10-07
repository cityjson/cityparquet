"""The network family: the format read benchmark over HTTP.

Results live under ``<data root>/network/<suite profile>/<network profile>/``
(``bench_suite.py``): one ``<dataset>.csv`` per dataset in the read CSV
contract, its ``.params.json`` with a ``network`` block (target, bandwidth,
latency, server and client totals), ``<dataset>.model.csv`` with the
``bytes*8/bandwidth + requests*latency`` model beside the measured median,
and the slice's Bloom pair under ``bloom/``. Network profiles are discovered
from the directories, never listed here.

A cell with no citable value stays explicit (``unavailable`` names why): it
is never drawn or written as zero. A DERIVED cell (notes tag
``derived-from=full-read``) is a whole-file format's query that the run did
not measure: the harness proved that the format's client issues one
whole-object GET, so the cell carries read all's bytes and request count and
no time. It is shown as "same transfer as read all", never as a measured
time.
"""

from __future__ import annotations

import csv
import json
from pathlib import Path

from .prep import BASELINE_FORMAT, FORMATS, _float, _int, _scenario_key, timing, unavailable_reason

NETWORK_DIR = "network"
FACTORS_TABLE = "network_factors.csv"
BLOOM_TABLE = "network_bloom.csv"
NOT_MEASURED_SECTION = "network/not-measured"
METRICS = (
    ("time_s", "Time", "s"),
    ("bytes_read", "Bytes read", "B"),
    ("http_requests", "HTTP requests", ""),
)
BLOOM_FORMATS = ("cityparquet", "cityparquet+nobloom")
DERIVED_TAG = "derived-from=full-read"
DERIVED_MARK = "†"
DERIVED_TIME_TEXT = f"= read all\ntransfer {DERIVED_MARK}"
DERIVED_NOTE = "derived: same transfer as read all (one whole-object GET, proven on this run); time not measured"
DERIVED_FOOTNOTE = (
    f"{DERIVED_MARK} derived, not measured: the format's client issues one whole-object GET "
    "(proven on this run's measured cells: 1 request, bytes = file size), so the cell repeats "
    "read all's bytes and requests; its time is that transfer plus parsing, not a measurement"
)


def _csvs(folder: Path) -> list[Path]:
    return sorted(
        p for p in folder.glob("*.csv") if not p.name.endswith(".model.csv")
    ) if folder.is_dir() else []


def _rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def _stem(dataset: str) -> str:
    return dataset.split(".", 1)[0]


def is_derived(row: dict) -> bool:
    """Whether a CSV row is a derived cell rather than a measurement."""
    return DERIVED_TAG in (t.strip() for t in str(row.get("notes", "") or "").split(";"))


def _cell(row: dict, statistic: str) -> dict:
    if is_derived(row):
        return {
            "query": _scenario_key(row),
            "format": row["format"],
            "time_s": None,
            "bytes_read": _int(row.get("bytes_read")),
            "http_requests": _int(row.get("http_requests")),
            "repeat": _int(row.get("repeat")),
            "unavailable": None,
            "derived": True,
        }
    reason = unavailable_reason(row)
    value = timing(row, statistic)["time_s"] if reason is None else None
    return {
        "query": _scenario_key(row),
        "format": row["format"],
        "time_s": value,
        "bytes_read": _int(row.get("bytes_read")) if reason is None else None,
        "http_requests": _int(row.get("http_requests")) if reason is None else None,
        "repeat": _int(row.get("repeat")),
        "unavailable": reason,
        "derived": False,
    }


def load(data_root: Path, suite_profile: str, statistic: str = "median") -> dict:
    """Every network result under ``data_root/network/<suite_profile>/``."""
    base = Path(data_root) / NETWORK_DIR / suite_profile
    profiles, records, bloom = [], [], []
    for folder in sorted(p for p in base.glob("*") if p.is_dir()) if base.is_dir() else []:
        csvs = _csvs(folder)
        if not csvs:
            continue
        entry = {"name": folder.name, "datasets": [], "totals_match": True}
        for path in csvs:
            dataset = path.stem
            params_path = Path(f"{path}.params.json")
            net = json.loads(params_path.read_text())["network"] if params_path.exists() else {}
            for key in ("target", "bandwidth_mbps", "latency_ms", "burst_ms", "base_url", "host", "resolved_ips"):
                if net.get(key) is not None:
                    entry.setdefault(key, net[key])
            server, clients = net.get("server", {}), net.get("clients", {})
            # A real target has no server of ours to count; only net-sim's totals can be compared.
            if server:
                entry["totals_match"] &= server.get("requests") == clients.get("requests") and server.get(
                    "body_bytes"
                ) == clients.get("bytes_read")
            model = {}
            model_path = path.with_name(f"{dataset}.model.csv")
            if model_path.exists():
                for m in _rows(model_path):
                    model[(m["format"], m["scenario"], m["notes"])] = _float(m.get("model_s"))
            entry["datasets"].append(dataset)
            for row in _rows(path):
                cell = _cell(row, statistic)
                cell.update(
                    dataset=dataset,
                    network_profile=folder.name,
                    model_time_s=model.get((row["format"], row["scenario"], row["notes"])),
                )
                records.append(cell)
            bloom_path = folder / "bloom" / path.name
            if bloom_path.exists():
                for row in _rows(bloom_path):
                    cell = _cell(row, statistic)
                    cell.update(dataset=dataset, network_profile=folder.name)
                    bloom.append(cell)
        profiles.append(entry)
    return {
        "measured": bool(records),
        "suite_profile": suite_profile,
        "profiles": profiles,
        "records": records,
        "bloom": bloom,
    }


def caption(profile: dict, statistic: str, repeat: int | None = None, derived: bool = False) -> str:
    """The profile's conditions, stated on every figure and table."""
    if profile.get("target") == "real":
        ips = ", ".join(profile.get("resolved_ips") or []) or "address not recorded"
        where = f"real object storage, one snapshot ({profile.get('base_url', 'base URL not recorded')}, {ips})"
    else:
        where = "simulated network (net-sim, local)"
    bw, lat = profile.get("bandwidth_mbps"), profile.get("latency_ms")
    if bw is not None and lat is not None:
        link = f"{bw:g} Mbps, {lat:g} ms per request"
    elif profile.get("target") == "real":
        link = "link as found, not controlled"
    else:
        link = "link not recorded"
    reps = f", {repeat} repetitions" if repeat else ""
    text = f"Network profile '{profile['name']}': {where}; {link}; time = {statistic}{reps}"
    return f"{text}. {DERIVED_FOOTNOTE}" if derived else text


def _fmt(metric: str, value) -> str:
    if value is None:
        return ""
    if metric == "time_s":
        return f"{value:.3g} s"
    if metric == "bytes_read":
        for unit, size in (("GB", 1e9), ("MB", 1e6), ("kB", 1e3)):
            if value >= size:
                return f"{value / size:.3g} {unit}"
        return f"{value} B"
    return str(value)


def factor_rows(block: dict, statistic: str) -> list[dict]:
    profiles = {p["name"]: p for p in block["profiles"]}
    base = {
        (r["dataset"], r["network_profile"], r["query"]): r
        for r in block["records"]
        if r["format"] == BASELINE_FORMAT
    }
    rows = []
    for r in block["records"]:
        p = profiles[r["network_profile"]]
        b = base.get((r["dataset"], r["network_profile"], r["query"]), {})
        row = {
            "dataset": r["dataset"],
            "network_profile": r["network_profile"],
            "target": p.get("target", ""),
            "bandwidth_mbps": p.get("bandwidth_mbps", ""),
            "latency_ms": p.get("latency_ms", ""),
            "format": r["format"],
            "query": r["query"],
            "derived": "full-read" if r.get("derived") else "",
            "statistic": statistic,
            "repeat": r["repeat"] or "",
            "time_s": r["time_s"] if r["time_s"] is not None else "",
            "bytes_read": r["bytes_read"] if r["bytes_read"] is not None else "",
            "http_requests": r["http_requests"] if r["http_requests"] is not None else "",
            "model_time_s": r["model_time_s"] if r["model_time_s"] is not None else "",
        }
        for metric, name in (("time_s", "time"), ("bytes_read", "bytes"), ("http_requests", "requests")):
            mine, theirs = r.get(metric), b.get(metric)
            row[f"{name}_factor_vs_citygml"] = round(theirs / mine, 4) if mine and theirs else ""
        row["note"] = DERIVED_NOTE if r.get("derived") else (r["unavailable"] or "")
        rows.append(row)
    return rows


def _write(path: Path, rows: list[dict]) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as handle:
        if rows:
            writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
    return path


def _factor_text(factor: float) -> str:
    return f"{factor:.2g}×" if factor < 10 else f"{factor:.0f}×"


def _factor_colour(factor: float | None) -> str:
    """The format figures' factor vocabulary: teal better than CityGML, the
    page colour at 1x, the warm accent worse."""
    import math

    from matplotlib import colors

    from . import figures  # imports this module, so not at the top

    if factor is None:
        return figures.BG
    if math.isclose(factor, 1.0):
        return figures.BG
    limit = figures.FACTOR_COLOUR_LIMIT
    norm = colors.TwoSlopeNorm(vmin=-limit, vcenter=0, vmax=limit)
    return colors.to_hex(figures.CMAP_FACTOR(norm(max(-limit, min(limit, math.log2(factor))))))


def figure_grid(records: list[dict]) -> dict:
    """What the figure prints: formats and queries (measured order first, with
    the format figures' display labels) and, per (metric, format, query), the
    cell's text, kind, factor against CityGML (None where none is citable)
    and fill colour."""
    from . import figures  # imports this module, so not at the top

    formats = [f for f in FORMATS if any(r["format"] == f for r in records)]
    queries = list(dict.fromkeys([r["query"] for r in records if not r.get("derived")] + [r["query"] for r in records]))
    by = {(r["format"], r["query"]): r for r in records}
    base = {r["query"]: r for r in records if r["format"] == BASELINE_FORMAT}
    cells = {}
    for metric, _title, _unit in METRICS:
        for fmt in formats:
            for query in queries:
                cell, factor = by.get((fmt, query)), None
                if cell is None:
                    text, kind = "not measured", "missing"
                elif cell["unavailable"]:
                    text, kind = cell["unavailable"][:18], "unavailable"
                elif cell.get("derived") and metric == "time_s":
                    text, kind = DERIVED_TIME_TEXT, "derived"
                else:
                    value, ref = cell[metric], base.get(query, {}).get(metric)
                    text, kind = _fmt(metric, value), "derived" if cell.get("derived") else "measured"
                    if kind == "derived":
                        text += f" {DERIVED_MARK}"
                    if metric != "http_requests" and value and ref:
                        factor = ref / value
                        if fmt != BASELINE_FORMAT:
                            text += "\n" + _factor_text(factor)
                fill = figures.BAD_CELL if kind in ("missing", "unavailable") else _factor_colour(factor)
                cells[(metric, fmt, query)] = {"text": text, "kind": kind, "factor": factor, "colour": fill}
    return {
        "formats": formats,
        "queries": queries,
        "format_labels": [figures._label(f) for f in formats],
        "query_labels": [figures._label(q) for q in queries],
        "cells": cells,
    }


def _figure(records: list[dict], profile: dict, statistic: str, out: Path) -> list[Path]:
    import textwrap

    import matplotlib.pyplot as plt
    from matplotlib.patches import Rectangle

    from . import figures  # imports this module, so not at the top

    grid = figure_grid(records)
    formats, queries = grid["formats"], grid["queries"]
    repeat = max((r["repeat"] or 0) for r in records) or None
    derived = any(r.get("derived") for r in records)
    note = (
        caption(profile, statistic, repeat, derived=derived)
        + ". Cell text: absolute value over its factor against CityGML (CityGML's value ÷ the format's; "
        "higher is better); colour: teal better than CityGML, page colour at 1×, warm accent worse; "
        "request counts uncoloured."
    )
    # The paper's text width, as the format heatmaps.
    fig, axes = plt.subplots(
        1, 3, figsize=(7.2, 1.7 + 0.36 * len(queries)), layout="constrained", facecolor=figures.BG
    )
    ink = {"missing": figures.MUTED, "unavailable": "#b33", "derived": figures.MUTED, "measured": figures.INK}
    for ax, (metric, title, _unit) in zip(axes, METRICS):
        ax.set_facecolor(figures.BG)
        ax.set_xlim(0, len(formats))
        ax.set_ylim(len(queries), 0)
        ax.set_xticks(
            [i + 0.5 for i in range(len(formats))], grid["format_labels"], fontsize=6, rotation=35, ha="right"
        )
        ax.set_yticks(
            [i + 0.5 for i in range(len(queries))], grid["query_labels"] if ax is axes[0] else [], fontsize=6
        )
        ax.tick_params(length=0)
        for spine in ax.spines.values():
            spine.set_visible(False)
        ax.set_title(title, fontsize=8, loc="left")
        for x, fmt in enumerate(formats):
            for y, query in enumerate(queries):
                cell = grid["cells"][(metric, fmt, query)]
                ax.add_patch(Rectangle((x, y), 1, 1, facecolor=cell["colour"], edgecolor=figures.CELL_EDGE, lw=0.5))
                ax.text(x + 0.5, y + 0.5, cell["text"], ha="center", va="center", fontsize=5, color=ink[cell["kind"]])
    fig.suptitle(textwrap.fill(note, 170), fontsize=5, x=0.01, ha="left", color=figures.INK)
    written = []
    for ext in ("svg", "png"):
        path = out / f"network.{ext}"
        out.mkdir(parents=True, exist_ok=True)
        fig.savefig(path, dpi=200, facecolor=figures.BG)
        written.append(path)
    plt.close(fig)
    return written


def render(data: dict, out: Path) -> list[Path]:
    """The figures and tables under ``out/network/``; nothing when unmeasured."""
    block = data.get("network") or {}
    if not block.get("measured"):
        return []
    statistic = data.get("statistic", "median")
    written = [_write(out / NETWORK_DIR / FACTORS_TABLE, factor_rows(block, statistic))]
    for profile in block["profiles"]:
        for dataset in profile["datasets"]:
            records = [
                r for r in block["records"] if r["network_profile"] == profile["name"] and r["dataset"] == dataset
            ]
            written += _figure(records, profile, statistic, out / NETWORK_DIR / profile["name"] / _stem(dataset))
        bloom = [r for r in block["bloom"] if r["network_profile"] == profile["name"]]
        if bloom:
            rows = [
                {
                    "dataset": r["dataset"],
                    "lookup": r["query"],
                    "configuration": "with filters" if r["format"] == "cityparquet" else "without filters",
                    "statistic": statistic,
                    "time_s": "" if r["time_s"] is None else r["time_s"],
                    "bytes_read": "" if r["bytes_read"] is None else r["bytes_read"],
                    "http_requests": "" if r["http_requests"] is None else r["http_requests"],
                    "note": r["unavailable"] or "",
                    "conditions": caption(profile, statistic, r["repeat"]),
                }
                for r in bloom
            ]
            written.append(_write(out / NETWORK_DIR / profile["name"] / BLOOM_TABLE, rows))
    return written


def sections(data: dict) -> list[tuple[str, str, list[str]]]:
    """The page's network sections: one per profile and dataset, or one
    stating the family was not measured."""
    block = data.get("network") or {}
    if not block.get("measured"):
        return [(NOT_MEASURED_SECTION, "Format reads over HTTP (network family): not measured", [])]
    statistic = data.get("statistic", "median")
    result = []
    for profile in block["profiles"]:
        for dataset in profile["datasets"]:
            derived = any(
                r.get("derived")
                for r in block["records"]
                if r["network_profile"] == profile["name"] and r["dataset"] == dataset
            )
            lines = [caption(profile, statistic, derived=derived)]
            if not profile.get("totals_match", True):
                lines.append("server and client request/byte totals differ")
            result.append(
                (f"{NETWORK_DIR}/{profile['name']}/{_stem(dataset)}/network", f"{_stem(dataset)} over HTTP — {profile['name']}", lines)
            )
    return result
