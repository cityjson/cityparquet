"""Create a self-contained HTML index for paper figures."""

from __future__ import annotations

import base64
import csv
import html
import json
from pathlib import Path

from . import tables
from .paths import DEFAULT_DATA_PATH, DEFAULT_FIGURES_DIR, DEFAULT_HTML_PATH
from .tables import FORMAT_METRICS, FORMATS_DIR

# The figures after the format comparison, as (path under the figures
# directory without its extension, page title).
TAIL = (
    ("bloom", "Bloom-filter configuration"),
    ("bloom-scaling", "Bloom-filter scaling"),
    ("bloom-corpus", "Bloom filters on the corpus"),
    ("databases", "Database comparison"),
)
# The format comparison's ratio tables, as (file under `formats/`, page title).
TABLES = (
    (tables.SIZE_TABLE, "Size factors against CityGML"),
    (tables.SIZE_EXTREMES, "Best and worst dataset by CityParquet's size factor"),
    (tables.QUERY_TABLE, "Query factors against CityGML"),
)


def sections(data: dict) -> list[tuple[str, str, list[str]]]:
    """Every figure the page embeds: (relative path, title, condition lines).

    Derived from the data, never from a fixed list of datasets: the size
    figure, then a time and a memory figure per dataset — including a
    manifest corpus dataset with no results, as a missing section — then the
    rest.
    """
    conditions = data.get("meta", {}).get("conditions", {})
    result = [(f"{FORMATS_DIR}/sizes", "File size on disk", [])]
    for dataset in data.get("datasets", []):
        name = dataset.get("title") or dataset["id"]
        lines = [x for x in conditions.get("formats", []) if x.startswith(f"{dataset['id']}:")]
        for metric, _field, page_title, _axis in FORMAT_METRICS:
            result.append(
                (f"{FORMATS_DIR}/{dataset['id']}/{metric}", f"{name} — {page_title}", lines)
            )
    # A format-comparison dataset of the manifest with no results stays
    # visibly missing rather than silently absent.
    measured = {d["id"] for d in data.get("datasets", [])}
    labels = data.get("meta", {}).get("dataset_labels", {})
    for dataset_id, label in labels.items():
        if label.get("role") != "corpus" or dataset_id in measured:
            continue
        name = label.get("title") or dataset_id
        for metric, _field, page_title, _axis in FORMAT_METRICS:
            result.append((f"{FORMATS_DIR}/{dataset_id}/{metric}", f"{name} — {page_title}", []))
    for path, title in TAIL:
        result.append((path, title, conditions.get(path, [])))
    return result


def _table(path: Path) -> str:
    with path.open(newline="", encoding="utf-8") as handle:
        rows = list(csv.reader(handle))
    if not rows:
        return ""
    head = "".join(f"<th>{html.escape(c)}</th>" for c in rows[0])
    body = "".join(
        "<tr>" + "".join(f"<td>{html.escape(c)}</td>" for c in row) + "</tr>" for row in rows[1:]
    )
    return f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>"


def main(
    data_path: Path | None = None, out_path: Path | None = None, figures_dir: Path | None = None
) -> Path:
    data_path, out_path, figures_dir = (
        data_path or DEFAULT_DATA_PATH,
        out_path or DEFAULT_HTML_PATH,
        figures_dir or DEFAULT_FIGURES_DIR,
    )
    data = json.loads(data_path.read_text(encoding="utf-8"))
    completeness_path = data_path.parent / "completeness.json"
    completeness = json.loads(completeness_path.read_text()) if completeness_path.exists() else {}
    parts = []
    for name, title, conditions in sections(data):
        path = figures_dir / f"{name}.svg"
        if path.exists():
            payload = base64.b64encode(path.read_bytes()).decode("ascii")
            lines = "".join(f"<li>{html.escape(str(x))}</li>" for x in conditions)
            listing = f"<h3>Conditions</h3><ul class='conditions'>{lines}</ul>" if lines else ""
            parts.append(
                f"<section><h2>{html.escape(title)}</h2><img alt='{html.escape(title)}' src='data:image/svg+xml;base64,{payload}'>{listing}</section>"
            )
        else:
            parts.append(
                f"<section class='missing'><h2>{html.escape(title)}</h2><p>Not rendered: result family absent.</p></section>"
            )
    for name, title in TABLES:
        path = figures_dir / FORMATS_DIR / name
        if path.exists():
            parts.append(
                f"<section><h2>{html.escape(title)}</h2><p class='conditions'>{html.escape(tables.ORIENTATION)}</p>{_table(path)}</section>"
            )
    coverage = html.escape(json.dumps(completeness, indent=2))
    caveats = "".join(
        f"<li>{html.escape(str(x))}</li>" for x in data.get("meta", {}).get("caveats_read", [])
    )
    page = f"""<!doctype html><meta charset='utf-8'><title>CityParquet benchmark figures</title><style>body{{max-width:1100px;margin:2rem auto;padding:0 1rem;background:#fffff8;color:#111;font:16px Georgia,serif}}h1,h2{{font-weight:normal}}section{{margin:3rem 0}}img{{width:100%;height:auto}}.missing,pre,.conditions{{color:#666}}.conditions{{font-size:14px}}h3{{font-weight:normal;font-size:16px}}pre{{background:#f0eee6;padding:1rem}}table{{border-collapse:collapse;font-size:12px;display:block;overflow-x:auto}}th,td{{padding:2px 6px;border-bottom:1px solid #e6e3d7;text-align:left;white-space:nowrap}}</style><h1>CityParquet benchmark figures</h1><p>Self-contained paper figures. The format comparison's colours encode factors against CityGML (CityGML's value ÷ the format's; higher is better); the other families state their own baselines. Values remain printed.</p>{"".join(parts)}<section><h2>Coverage</h2><pre>{coverage}</pre></section><section><h2>Measurement caveats</h2><ul>{caveats}</ul></section>"""
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(page, encoding="utf-8")
    print(f"benchviz html -> {out_path}")
    return out_path
