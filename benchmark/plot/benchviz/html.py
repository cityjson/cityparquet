"""Create a self-contained HTML index for paper figures."""

from __future__ import annotations

import base64
import html
import json
from pathlib import Path

from .paths import DEFAULT_DATA_PATH, DEFAULT_FIGURES_DIR, DEFAULT_HTML_PATH

ORDER = ("sizes", "heatmap", "codec", "codec-scaling", "rowgroup", "rowgroup-scaling", "databases")
TITLES = {
    "sizes": "File size on disk",
    "heatmap": "Format comparison",
    "codec": "Compression configuration",
    "codec-scaling": "Compression scaling",
    "rowgroup": "Row-group configuration",
    "rowgroup-scaling": "Row-group scaling",
    "databases": "Database comparison",
}


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
    sections = []
    for name in ORDER:
        path = figures_dir / f"{name}.svg"
        if path.exists():
            payload = base64.b64encode(path.read_bytes()).decode("ascii")
            sections.append(
                f"<section><h2>{TITLES[name]}</h2><img alt='{html.escape(TITLES[name])}' src='data:image/svg+xml;base64,{payload}'></section>"
            )
        else:
            sections.append(
                f"<section class='missing'><h2>{TITLES[name]}</h2><p>Not rendered: result family absent.</p></section>"
            )
    coverage = html.escape(json.dumps(completeness, indent=2))
    caveats = "".join(
        f"<li>{html.escape(str(x))}</li>" for x in data.get("meta", {}).get("caveats_read", [])
    )
    page = f"""<!doctype html><meta charset='utf-8'><title>CityParquet benchmark figures</title><style>body{{max-width:1100px;margin:2rem auto;padding:0 1rem;background:#fffff8;color:#111;font:16px Georgia,serif}}h1,h2{{font-weight:normal}}section{{margin:3rem 0}}img{{width:100%;height:auto}}.missing,pre{{color:#666}}pre{{background:#f0eee6;padding:1rem}}</style><h1>CityParquet benchmark figures</h1><p>Self-contained paper figures. Colours encode ratios to the stated baseline; values remain printed.</p>{"".join(sections)}<section><h2>Coverage</h2><pre>{coverage}</pre></section><section><h2>Measurement caveats</h2><ul>{caveats}</ul></section>"""
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(page, encoding="utf-8")
    print(f"benchviz html -> {out_path}")
    return out_path
