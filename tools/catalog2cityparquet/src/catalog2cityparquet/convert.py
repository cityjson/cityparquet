"""Invoke the converter and finish the STAC Item it emits.

`cityparquet convert` already writes `metadata.json` as a STAC Item derived
from the Parquet footer. The driver adds only what a single package cannot
know: which collection it belongs to, and where it came from. Footer-derived
properties (`city3d:*`, `proj:*`, `cityparquet:*`) are never edited — the spec
makes the footer authoritative wherever Item and footer disagree, and the Item
is built from that footer by construction. A package whose CRS cannot be
reprojected to WGS84 therefore carries no `geometry` and no `bbox`, and none is
invented here.

Classifying failures is the point of the run: roughly half the catalogue's
collections cannot be converted, and the deliverable is a measured statement of
which and why. "It failed" measures nothing, so `classify_error` maps the
converter's real stderr onto the ledger's closed reason vocabulary.

Nothing here interprets CityJSON, CityGML or Parquet — the Rust binary owns
every format decision.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from .discover import Item
from .ledger import CONFORMANCE_REASONS

#: Longest converter stderr kept on a `ConvertError`. Some failures run to
#: thousands of lines, and every one of them ends up on a single ledger line.
MAX_DETAIL_CHARS = 2000

#: Substring (lower-cased) → ledger reason, tried in order. The order is load
#: bearing: a CityGML version failure no longer says "invalid CityJSON", but
#: the version test stays ahead of the CityJSON test so the classifier survives
#: the message drifting back.
_CLASSIFIERS: tuple[tuple[str, str], ...] = (
    ("unsupported citygml version", "unsupported_citygml_version"),
    ("geographic crs", "geographic_crs"),
    ("declares no crs", "no_crs"),
    ("invalid cityjson", "unsupported_cityjson_version"),
)

#: What an unrecognised failure is called. Also the timeout's reason: a hung
#: converter is a conversion failure like any other.
_FALLBACK_REASON = "convert_failed"

#: Link relations this module owns. Re-stamping replaces exactly these, so a
#: resumed run cannot accumulate duplicates — duplicated links would corrupt
#: the collection aggregated from these Items.
_OWNED_RELS = frozenset({"collection", "parent", "root", "via", "derived_from"})

_returnable = {reason for _, reason in _CLASSIFIERS} | {_FALLBACK_REASON}
_unknown_reasons = _returnable - CONFORMANCE_REASONS
if _unknown_reasons:
    # Checked at import rather than per call: the ledger rejects a reason
    # outside its closed set, and discovering the drift mid-run would waste it.
    # Checked against the *conformance* subset, not the whole vocabulary: this
    # classifier reads the converter's own stderr, so everything it returns is
    # a statement about the data. It must never be able to return the
    # environment reason, which would smuggle a host failure into the histogram.
    raise RuntimeError(
        f"classify_error can return reasons the ledger rejects: {sorted(_unknown_reasons)}"
    )


class ConvertError(RuntimeError):
    """A conversion that failed, carrying the reason the ledger will record."""

    def __init__(self, reason: str, detail: str) -> None:
        super().__init__(detail)
        self.reason = reason
        self.detail = detail


def classify_error(stderr: str) -> str:
    """Map converter stderr to a ledger reason.

    Classification is the point of the run: an unclassified pile of failures
    measures nothing. The return value is always a member of `ledger.REASONS`.
    """
    text = stderr.lower()
    for needle, reason in _CLASSIFIERS:
        if needle in text:
            return reason
    return _FALLBACK_REASON


def run_convert(
    binary: Path, inputs: list[Path], out_dir: Path, crs: str | None, timeout: float
) -> int:
    """Convert `inputs` into `out_dir`; return the city-object count.

    Several inputs are passed at once because the converter merges them, which
    is what a multi-tile archive needs. `crs` is the operator-supplied fallback
    the converter honours only for a source that declares none.

    Raises `ConvertError` on a non-zero exit and on timeout. No retry happens
    here — retries belong to the orchestrator, which alone knows the budget.
    """
    cmd = [str(binary), "convert", *[str(p) for p in inputs], "-o", str(out_dir), "--overwrite"]
    if crs:
        cmd += ["--crs", crs]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise ConvertError(_FALLBACK_REASON, f"timed out after {timeout}s") from exc
    if proc.returncode != 0:
        raise ConvertError(classify_error(proc.stderr), proc.stderr.strip()[:MAX_DETAIL_CHARS])
    # The report's first token is the object count. A conversion that succeeded
    # while printing nothing countable is still a success, so it reports 0
    # rather than raising.
    fields = proc.stdout.split()
    return int(fields[0]) if fields and fields[0].isdigit() else 0


def stamp(pkg_dir: Path, item: Item) -> None:
    """Add collection membership and provenance to the emitted Item.

    Idempotent: a resumed run may re-stamp a package, and duplicated links
    would corrupt the aggregated collection. A missing or unparseable
    `metadata.json` raises rather than being skipped — a package without a
    valid Item is a broken package, and silence would hide it.
    """
    path = pkg_dir / "metadata.json"
    doc = json.loads(path.read_text(encoding="utf-8"))
    doc["collection"] = item.collection

    links = [link for link in doc.get("links", []) if link.get("rel") not in _OWNED_RELS]
    links.append({"rel": "collection", "href": "../../collection.json", "type": "application/json"})
    links.append({"rel": "parent", "href": "../../collection.json", "type": "application/json"})
    links.append({"rel": "root", "href": "../../../catalog.json", "type": "application/json"})
    if item.href:
        # Where the source bytes came from.
        links.append({"rel": "via", "href": item.href})
    if item.source_item_url:
        # The published Item this package was derived from.
        links.append(
            {"rel": "derived_from", "href": item.source_item_url, "type": "application/json"}
        )
    doc["links"] = links

    path.write_text(json.dumps(doc, indent=2, ensure_ascii=False), encoding="utf-8")
