"""Renders a run directory's results.jsonl as report.md (run.py calls it).

Statuses: pass; admitted (passes, with a loss the specification allows);
noted (passes, with a difference no rule covers, listed for a reader to judge);
known (fails, and known.toml explains every failure in the step); fail; skip.
"""

from __future__ import annotations

import json
from collections import Counter, defaultdict
from pathlib import Path

MARK = {"pass": "✓", "admitted": "✓·", "noted": "✓ⁿ", "known": "K", "fail": "✗", "skip": "–"}
ROUND_TRIPS = [
    "round trip rs",
    "round trip dk",
    "cross dk package -> rs export",
    "cross rs package -> dk export",
    "rewritten package round trip",
    "packages agree (rs export of each)",
]
CATEGORIES = ["objects", "hierarchy", "attributes", "address", "geometry", "coordinates", "semantics", "materials", "textures", "templates"]
CAT_MARK = {"pass": "✓", "admitted": "✓·", "noted": "✓ⁿ", "fail": "✗", "n/a": ""}


def render(out: Path) -> str:
    rows = [json.loads(l) for l in (out / "results.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]
    env = json.loads((out / "environment.json").read_text())
    corpus = json.loads((out / "corpus.json").read_text()) if (out / "corpus.json").exists() else []
    known = {}
    kp = Path(__file__).with_name("known.toml")
    if kp.exists():
        import tomllib

        known = {k["id"]: k for k in tomllib.loads(kp.read_text()).get("issue", [])}

    by_ds: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by_ds[r["dataset"]].append(r)
    totals = Counter(r["status"] for r in rows)
    failing = [r for r in rows if r["status"] == "fail"]

    L: list[str] = []
    L.append("# CityParquet tool-chain test report")
    L.append("")
    verdict = "**No unexplained failure.**" if not failing else f"**{len(failing)} unexplained failure(s)** in {len({r['dataset'] for r in failing})} dataset(s)."
    L.append(
        f"{verdict} {totals['pass']} steps passed, {totals['admitted']} passed with a loss the specification admits, "
        f"{totals['noted']} passed with a difference listed under Notes, "
        f"{totals['known']} failed on a known issue, {totals['skip']} were skipped."
    )
    L.append("")
    L.append(f"Profile `{env.get('profile')}`, run {env.get('date')}.")
    L.append("")

    L.append("## What was tested")
    L.append("")
    L.append("| Tool | Version |")
    L.append("| --- | --- |")
    dirty = " (with uncommitted changes)" if env.get("cityparquet_dirty") else ""
    L.append(f"| cityparquet-rs | {env.get('cityparquet_version', '?')}; monorepo `{env.get('cityparquet_branch')}` at `{env.get('cityparquet_commit')}`{dirty} |")
    exts = dict(env.get("extensions", []))
    L.append(f"| DuckDB | {env.get('duckdb', '?')} |")
    L.append(f"| duckdb-cityjson (`cityjson`) | `{exts.get('cityjson', 'not loaded')}` |")
    L.append(f"| duckdb-3d (`three_d`) | `{exts.get('three_d', 'not loaded')}` |")
    L.append(f"| citygml-tools (CityGML comparisons) | {env.get('citygml_tools', 'not found')} |")
    L.append(f"| Machine | {env.get('machine')} |")
    if env.get("missing"):
        L.append("")
        L.append("Missing tools, whose steps were skipped: " + ", ".join(f"`{k}` ({v})" for k, v in env["missing"].items()) + ".")
    L.append("")

    L.append("## Corpus")
    L.append("")
    L.append("| Dataset | Format | Kind | Size | What it exercises |")
    L.append("| --- | --- | --- | ---: | --- |")
    for d in corpus:
        pending = " **(pending)**" if d.get("status", "active") != "active" else ""
        L.append(f"| `{d['id']}`{pending} | {d['format']} | {d['kind']} | {d['bytes'] / 1e6:.1f} MB | {d['exercises']} |")
    L.append("")

    L.append("## Round trips")
    L.append("")
    L.append(
        "Each output is compared with the source by `cjcompare.py`, which shares no code with the writers. "
        "`✓` equal, `✓·` equal up to a loss the specification lists, `✓ⁿ` equal after a canonicalisation the "
        "table does not list (see Notes), `✗` different, blank: nothing of that kind in the source."
    )
    L.append("")
    L.append("| Dataset | Path | Objects | " + " | ".join(CATEGORIES[1:]) + " | Max dev. |")
    L.append("| --- | --- | ---: | " + " | ".join("---" for _ in CATEGORIES[1:]) + " | ---: |")
    for ds, rs in by_ds.items():
        for r in rs:
            if "categories" not in r:
                continue
            cats = r["categories"]
            objs = r.get("objects", ["", ""])
            obj_cell = f"{objs[0]}" if objs[0] == objs[1] else f"{objs[0]} → {objs[1]}"
            obj_cell = f"{CAT_MARK.get(cats.get('objects'), '')} {obj_cell}"
            dev = r.get("max_deviation")
            L.append(
                f"| `{ds}` | {r['step']}{' (K)' if r['status'] == 'known' else ''} | {obj_cell} | "
                + " | ".join(CAT_MARK.get(cats.get(c, "n/a"), "") for c in CATEGORIES[1:])
                + f" | {dev:.2g} |"
            )
    L.append("")

    L.append("## Every step")
    L.append("")
    steps: list[str] = []
    for rs in by_ds.values():
        for r in rs:
            if r["step"] not in steps:
                steps.append(r["step"])
    L.append("| Step | " + " | ".join(f"`{d}`" for d in by_ds) + " |")
    L.append("| --- | " + " | ".join("---" for _ in by_ds) + " |")
    for s in steps:
        cells = []
        for ds, rs in by_ds.items():
            hit = [r for r in rs if r["step"] == s]
            cells.append(MARK.get(hit[0]["status"], hit[0]["status"]) if hit else "")
        L.append(f"| {s} | " + " | ".join(cells) + " |")
    L.append("")
    L.append("`✓` pass, `✓·` pass with an admitted loss, `✓ⁿ` pass with a noted difference, `K` known issue, `✗` fail, `–` skipped.")
    L.append("")

    L.append("## Failures")
    L.append("")
    if not failing:
        L.append("None.")
    for r in failing:
        L.append(f"### `{r['dataset']}`: {r['step']}")
        L.append("")
        L.append("```text")
        L.append(r["detail"].strip()[:3000] or "(no detail)")
        L.append("```")
        L.append("")
    L.append("")

    L.append("## Known issues")
    L.append("")
    hit = defaultdict(list)
    for r in rows:
        if r["status"] == "known":
            for k in r["known"].split(","):
                hit[k].append(f"`{r['dataset']}` {r['step']}")
    if not hit:
        L.append("None hit.")
    for k, where in hit.items():
        info = known.get(k, {})
        L.append(f"- **{k}** — {info.get('summary', '')} ({info.get('reference', '')}). Hit by: {', '.join(where)}.")
    L.append("")

    L.append("## Notes")
    L.append("")
    notes = Counter()
    admitted = Counter()
    for r in rows:
        for k, ex in (r.get("examples") or {}).items():
            if k.startswith("noted:"):
                notes[k[6:]] += 1
            if k.startswith("admitted:"):
                admitted[k[9:]] += 1
    L.append(
        "Admitted losses (01-dataset-package.mdx, round-trip table) appeared in these categories: "
        + (", ".join(f"{c} ({n} comparisons)" for c, n in admitted.items()) or "none")
        + ". Unlisted canonicalisations (`noted`): "
        + (", ".join(f"{c} ({n} comparisons)" for c, n in notes.items()) or "none")
        + ". The first examples of each are in `results.jsonl`."
    )
    L.append("")
    noted = [r for r in rows if r["status"] == "noted"]
    if noted:
        L.append("Noted steps:")
        L.append("")
        for r in noted:
            L.append(f"- `{r['dataset']}` {r['step']}: " + "; ".join(r["detail"].splitlines())[:600])
        L.append("")
    skipped = [r for r in rows if r["status"] == "skip"]
    if skipped:
        L.append("Skipped:")
        L.append("")
        for r in skipped:
            L.append(f"- `{r['dataset']}` {r['step']}: {r['detail']}")
        L.append("")
    return "\n".join(L) + "\n"
