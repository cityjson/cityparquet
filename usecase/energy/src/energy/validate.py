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
        finite_rel_entries = []  # Entries with defined rel_err_pct
        zero_ref_mismatches = []  # Zero-reference mismatches (rel_err_pct = None)
        all_entries = []  # All entries for mae calculation

        for r in rows:
            refs = [r.get(c) for c in ref_cols]
            if any(v is None for v in refs) or r.get(computed_col) is None:
                continue
            reference = sum(refs)
            computed = r[computed_col]
            err = abs(computed - reference)

            # Store for mae calculation
            all_entries.append((r["building_id"], computed, reference, err, None))

            if reference == 0.0:
                # Zero reference case
                if abs(computed) < 1e-9:
                    # Genuine 0-vs-0 match: rel = 0.0
                    rel = 0.0
                    finite_rel_entries.append((r["building_id"], computed, reference, err, rel))
                else:
                    # Zero-reference mismatch: rel = None (undefined)
                    zero_ref_mismatches.append((r["building_id"], computed, reference, err, None))
            else:
                # Normal case: reference != 0
                rel = err / reference * 100.0
                finite_rel_entries.append((r["building_id"], computed, reference, err, rel))

        # Sort finite_rel_entries by rel_err_pct descending for worst[]
        finite_rel_entries.sort(key=lambda e: e[4], reverse=True)

        # Sort zero_ref_mismatches by absolute error descending
        zero_ref_mismatches.sort(key=lambda e: e[3], reverse=True)

        # Compute median from finite rels only
        finite_rels = sorted(e[4] for e in finite_rel_entries)
        n_finite = len(finite_rels)
        if n_finite > 0:
            if n_finite % 2 == 1:
                median_rel = finite_rels[n_finite // 2]
            else:
                median_rel = (finite_rels[n_finite // 2 - 1] + finite_rels[n_finite // 2]) / 2
        else:
            median_rel = None

        # Compute mae from all entries
        n_total = len(all_entries)
        mae = sum(e[3] for e in all_entries) / n_total if n_total else None

        # Build worst[] with zero-ref mismatches first, then finite entries
        worst_entries = []
        for b, c, ref, err, rel_err in zero_ref_mismatches[:5]:
            worst_entries.append({"building_id": b, "computed": c, "reference": ref, "rel_err_pct": None})

        # Add finite entries if space in worst (up to 5 total)
        remaining_slots = 5 - len(worst_entries)
        for b, c, ref, err, rel_err in finite_rel_entries[:remaining_slots]:
            worst_entries.append({"building_id": b, "computed": c, "reference": ref, "rel_err_pct": rel_err})

        report[name] = {
            "n": n_total,
            "mae": mae,
            "median_rel_err_pct": median_rel if n_finite else None,
            "n_zero_reference_mismatches": len(zero_ref_mismatches),
            "worst": worst_entries,
        }
    return report


def write_report(report: dict, path: str) -> None:
    with open(path, "w") as fh:
        json.dump(report, fh, indent=2)
