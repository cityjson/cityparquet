"""The network family: the format read benchmark over HTTP.

Results live under ``<data root>/network/<suite profile>/<network profile>/``
(``bench_suite.py``): one ``<dataset>.csv`` per dataset in the read CSV
contract, its ``.params.json`` with a ``network`` block (target, bandwidth,
latency, server and client totals), ``<dataset>.model.csv`` with the
``bytes*8/bandwidth + requests*latency`` model beside the measured median,
and the slice's Bloom pair under ``bloom/``. Network profiles are discovered
from the directories, never listed here.

A cell with no citable value stays explicit (``unavailable`` names why): it
is never drawn or written as zero.
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


def _csvs(folder: Path) -> list[Path]:
    return sorted(
        p for p in folder.glob("*.csv") if not p.name.endswith(".model.csv")
    ) if folder.is_dir() else []


def _rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def _stem(dataset: str) -> str:
    return dataset.split(".", 1)[0]


def _cell(row: dict, statistic: str) -> dict:
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
            for key in ("target", "bandwidth_mbps", "latency_ms", "burst_ms", "base_url"):
                if net.get(key) is not None:
                    entry.setdefault(key, net[key])
            server, clients = net.get("server", {}), net.get("clients", {})
            if server or clients:
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


def caption(profile: dict, statistic: str, repeat: int | None = None) -> str:
    """The profile's conditions, stated on every figure and table."""
    if profile.get("target") == "real":
        where = f"real object storage ({profile.get('base_url', 'base URL not recorded')}), one snapshot"
    else:
        where = "simulated network (net-sim, local)"
    bw, lat = profile.get("bandwidth_mbps"), profile.get("latency_ms")
    link = f"{bw:g} Mbps, {lat:g} ms per request" if bw is not None and lat is not None else "link not recorded"
    reps = f", {repeat} repetitions" if repeat else ""
    return f"Network profile '{profile['name']}': {where}; {link}; time = {statistic}{reps}"


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
        row["note"] = r["unavailable"] or ""
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


def _figure(records: list[dict], profile: dict, statistic: str, out: Path) -> list[Path]:
    import matplotlib.pyplot as plt

    formats = [f for f in FORMATS if any(r["format"] == f for r in records)]
    queries = list(dict.fromkeys(r["query"] for r in records))
    cells = {(r["format"], r["query"]): r for r in records}
    base = {r["query"]: r for r in records if r["format"] == BASELINE_FORMAT}
    repeat = max((r["repeat"] or 0) for r in records) or None
    fig, axes = plt.subplots(
        1, 3, figsize=(4.2 * 3, 0.6 + 0.32 * len(queries)), constrained_layout=True
    )
    for ax, (metric, title, _unit) in zip(axes, METRICS):
        ax.set_xlim(0, len(formats))
        ax.set_ylim(len(queries), 0)
        ax.set_xticks([i + 0.5 for i in range(len(formats))], formats, fontsize=8, rotation=20)
        ax.set_yticks([i + 0.5 for i in range(len(queries))], queries if ax is axes[0] else [], fontsize=8)
        ax.tick_params(length=0)
        for spine in ax.spines.values():
            spine.set_visible(False)
        ax.set_title(title, fontsize=10, loc="left")
        for x, fmt in enumerate(formats):
            for y, query in enumerate(queries):
                cell = cells.get((fmt, query))
                if cell is None:
                    text, colour = "not measured", "#999"
                elif cell["unavailable"]:
                    text, colour = cell["unavailable"][:18], "#b33"
                else:
                    value, ref = cell[metric], base.get(query, {}).get(metric)
                    text, colour = _fmt(metric, value), "#111"
                    if fmt != BASELINE_FORMAT and value and ref:
                        text += f" ({ref / value:.2g}x)"
                ax.text(x + 0.5, y + 0.5, text, ha="center", va="center", fontsize=7, color=colour)
    fig.suptitle(caption(profile, statistic, repeat) + "; (Nx) = CityGML / format", fontsize=8, x=0.01, ha="left")
    written = []
    for ext in ("svg", "png"):
        path = out / f"network.{ext}"
        out.mkdir(parents=True, exist_ok=True)
        fig.savefig(path, dpi=150)
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
            lines = [caption(profile, statistic)]
            if not profile.get("totals_match", True):
                lines.append("server and client request/byte totals differ")
            result.append(
                (f"{NETWORK_DIR}/{profile['name']}/{_stem(dataset)}/network", f"{_stem(dataset)} over HTTP — {profile['name']}", lines)
            )
    return result
