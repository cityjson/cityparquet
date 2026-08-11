"""Turn converted packages into a collection, and collections into a catalogue.

The published `collection.json` is the metadata seed: it is already fetched
during traversal and its id always matches. It is translated into the shape
`city3dstac`'s `--config` already accepts, so no tool change is needed for
metadata — only `--items-dir`.

Extent and summaries are deliberately NOT carried over: the tool recomputes
them from the generated items, so they describe the CityParquet mirror rather
than the source catalogue.

Aggregation is over the *directory*, not over the current run's successes. A
resumed run therefore rebuilds the collection from everything ever converted,
which is what makes resumption correct.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import yaml

#: Fields `city3dstac`'s `CollectionConfigFile` accepts. Emitting anything else
#: is at best ignored and at worst rejects the whole config, so this is the
#: contract the emitter is tested against.
TOOL_CONFIG_FIELDS = frozenset(
    {
        "id",
        "title",
        "description",
        "license",
        "keywords",
        "providers",
        "extent",
        "summaries",
        "links",
        "assets",
        "inputs",
    }
)

#: Links that describe the generated tree rather than the dataset. Carrying
#: these over would point the mirror's collection at the source catalogue.
STRUCTURAL_RELS = frozenset({"self", "root", "parent", "item", "child", "collection"})

#: Longest tool stderr kept on the raised error. A failure here is ledgered on
#: a single line, and some tool errors run long.
MAX_DETAIL_CHARS = 2000

#: Carried verbatim from the published collection. `extent` and `summaries` are
#: absent on purpose (recomputed from the generated items), and so are `assets`
#: — a source portal or download asset describes the origin, not the mirror.
_CARRIED = ("id", "title", "description", "license", "keywords", "providers")

#: The tool's guard when no item carried a bbox. `cityparquet` omits
#: `geometry`/`bbox` whenever the package CRS cannot be reprojected to WGS84,
#: so a collection whose items are *all* unlocated has no spatial extent to
#: aggregate and cannot be built at all.
_NO_EXTENT = "spatial extent bbox is required"

#: The STAC-GeoParquet encoder refuses a null geometry, so a *single* unlocated
#: Item is enough to fail `--geoparquet` for the whole collection — and the
#: failure comes after collection.json has already been written advertising an
#: `items-geoparquet` asset, leaving a zero-byte items.parquet behind.
_GEOPARQUET_FAILED = "geoparquet"


def collection_config(collection_json: dict) -> dict:
    """Translate a published collection into the tool's `--config` shape.

    Missing fields are omitted rather than emitted as nulls — a config full of
    `title: null` says nothing the absence did not — and no licence is invented
    for a source that declares none: the tool applies its own default.
    """
    config = {key: collection_json[key] for key in _CARRIED if collection_json.get(key) is not None}
    links = [
        link
        for link in collection_json.get("links", [])
        # `rel` and `href` are both required by the tool's `LinkConfig`. A
        # third-party collection missing either would make the config
        # unparseable, losing the whole collection over one malformed link.
        if link.get("rel") not in STRUCTURAL_RELS and link.get("rel") and link.get("href")
    ]
    if links:
        config["links"] = links
    return config


def catalog_config(catalog_json: dict) -> dict:
    """Translate the published catalogue root into `update-catalog`'s config.

    Only identity is carried: the child links are the generated tree's, and the
    tool writes them from the collections it is given.
    """
    return {
        key: catalog_json[key]
        for key in ("id", "title", "description")
        if catalog_json.get(key) is not None
    }


def write_config(config: dict, dest: Path) -> Path:
    """Write `config` as YAML and return the path written."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(yaml.safe_dump(config, sort_keys=False, allow_unicode=True), encoding="utf-8")
    return dest


def update_collection(
    tool: Path, items_dir: Path, config: Path, out: Path, geoparquet: bool = True
) -> None:
    """Aggregate every Item under `items_dir` into `out`.

    A GeoParquet failure is retried without the sidecar: an unlocated Item is
    honest output, and the collection matters far more than the optional
    items.parquet. Every other failure raises `RuntimeError` carrying the
    tool's stderr — including the one collection that genuinely cannot be
    aggregated, whose Items are *all* unlocated so no spatial extent exists.
    The orchestrator catches it and ledgers the collection as failed; nothing
    is printed here.
    """
    cmd = [
        str(tool),
        "update-collection",
        "--items-dir",
        str(items_dir),
        "--config",
        str(config),
        "-o",
        str(out),
    ]
    if not geoparquet:
        _run(cmd, "update-collection")
        return
    tolerated = _run([*cmd, "--geoparquet"], "update-collection", tolerate=_GEOPARQUET_FAILED)
    if tolerated is None:
        return
    # Written by the attempt that then failed to encode; nothing references it
    # once the retry rewrites collection.json without the asset.
    (out.parent / "items.parquet").unlink(missing_ok=True)
    _run(cmd, "update-collection")


def update_catalog(tool: Path, collection_jsons: list[Path], out_dir: Path, config: Path) -> None:
    """Link the given collections into a catalogue written under `out_dir`."""
    cmd = [
        str(tool),
        "update-catalog",
        *[str(p) for p in collection_jsons],
        "-o",
        str(out_dir),
        "--config",
        str(config),
    ]
    _run(cmd, "update-catalog")


def _run(cmd: list[str], what: str, tolerate: str | None = None) -> str | None:
    """Run `cmd`, raising on failure. Returns the stderr of a tolerated failure.

    `tolerate` is a lower-cased substring of the one failure the caller intends
    to recover from; anything else still raises.
    """
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode == 0:
        return None
    detail = proc.stderr.strip()[:MAX_DETAIL_CHARS]
    if tolerate and tolerate in detail.lower():
        return detail
    if _NO_EXTENT in detail.lower():
        detail += (
            " — every Item in this collection is unlocated (no bbox), because"
            " the package CRS could not be reprojected to WGS84; the tool"
            " cannot compute a spatial extent from them"
        )
    raise RuntimeError(f"{what} failed: {detail}")
