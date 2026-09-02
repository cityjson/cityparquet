"""Self-contained validation against 3DBAG's own b3_* reference columns."""
from __future__ import annotations

import json

import pyarrow as pa

_PAIRS = {
    "volume": ("volume_m3", ("b3_volume_lod22",)),
    "roof_flat": ("a_roof_flat_m2", ("b3_opp_dak_plat",)),
    "roof_pitched": ("a_roof_pitched_m2", ("b3_opp_dak_schuin",)),
    "ground": ("a_ground_m2", ("b3_opp_grond",)),
    "wall": ("a_wall_m2", ("b3_opp_buitenmuur", "b3_opp_scheidingsmuur")),
}


def validate(table: pa.Table) -> dict:
    rows = table.to_pylist()
    report: dict = {}
    for name, (computed_col, ref_cols) in _PAIRS.items():
        entries = []
        for r in rows:
            refs = [r.get(c) for c in ref_cols]
            if any(v is None for v in refs) or r.get(computed_col) is None:
                continue
            reference = sum(refs)
            computed = r[computed_col]
            err = abs(computed - reference)
            rel = err / reference * 100.0 if reference else 0.0
            entries.append((r["building_id"], computed, reference, err, rel))
        entries.sort(key=lambda e: e[4], reverse=True)
        rels = sorted(e[4] for e in entries)
        n = len(entries)
        if n % 2 == 1:
            median_rel = rels[n // 2]
        else:
            median_rel = (rels[n // 2 - 1] + rels[n // 2]) / 2 if n else None
        report[name] = {
            "n": n,
            "mae": sum(e[3] for e in entries) / n if n else None,
            "median_rel_err_pct": median_rel if n else None,
            "worst": [
                {"building_id": b, "computed": c, "reference": ref, "rel_err_pct": rel}
                for b, c, ref, _, rel in entries[:5]
            ],
        }
    return report


def write_report(report: dict, path: str) -> None:
    with open(path, "w") as fh:
        json.dump(report, fh, indent=2)
