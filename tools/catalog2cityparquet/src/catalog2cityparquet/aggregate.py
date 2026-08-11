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

The mirror's layout — one directory per package, each holding a `metadata.json`
— needs a `city3dstac` that derives item hrefs relative to the collection
(`vendor/city3d-stac-tool`, `feat/items-dir`). An older binary links every item
as `./metadata.json`.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import yaml

from .ledger import HOST_FAILURE_MARKERS, HostFailure, is_host_failure

#: Fields `city3dstac`'s `CollectionConfigFile` accepts. It carries no
#: `deny_unknown_fields`, so anything else is silently DROPPED rather than
#: rejected — quiet metadata loss, which is why the emitter is tested against
#: this set rather than trusted to fail loudly.
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

#: How long a single `city3dstac` call may take. Aggregating the catalogue's
#: largest collection — tens of thousands of Items — is the longest tool call
#: in a run, and a hang there would stall it with nothing recorded.
DEFAULT_TIMEOUT = 3600.0

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
#: `items-geoparquet` asset, leaving a zero-byte items.parquet behind. Matched
#: narrowly: the recovery deletes items.parquet, which must not happen for some
#: other failure that merely mentions the sidecar.
_GEOPARQUET_FAILED = "geoparquet encode error"

#: The sidecar the tool writes beside the collection it is given.
_INDEX_NAME = "items.parquet"

#: Re-exported for the callers that read them as `aggregate.*`. Their home is
#: `ledger.py`, beside the conformance/environment vocabulary they decide
#: between, because the `cityparquet` converter needs exactly the same reading
#: of exactly the same kernel wording — and two copies of the list would drift.
__all__ = [
    "HOST_FAILURE_MARKERS",
    "HostFailure",
    "catalog_config",
    "collection_config",
    "update_catalog",
    "update_collection",
    "write_config",
]


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
    tool: Path,
    items_dir: Path,
    config: Path,
    out: Path,
    geoparquet: bool = True,
    timeout: float = DEFAULT_TIMEOUT,
) -> bool:
    """Aggregate every Item under `items_dir` into `out`.

    Returns whether the GeoParquet index was written. A caller that ignores it
    is unaffected, but a run that degraded N collections can now say N — the
    point of the exercise being a *measured* statement about the catalogue.

    A GeoParquet encode failure is retried without the sidecar: an unlocated
    Item is honest output, and the collection matters far more than the
    optional items.parquet. Every other failure raises `RuntimeError` carrying
    the tool's stderr — including the one collection that genuinely cannot be
    aggregated, whose Items are *all* unlocated so no spatial extent exists.
    The orchestrator catches it and ledgers the collection as failed; nothing
    is printed here. A failure that is the *host's* raises `HostFailure`, which
    the orchestrator ledgers as an environment failure instead.
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
    if geoparquet:
        tolerated = _run(
            [*cmd, "--geoparquet"], "update-collection", timeout, tolerate=_GEOPARQUET_FAILED
        )
        if tolerated is None:
            return True
    _run(cmd, "update-collection", timeout)
    # Removed only after the collection that no longer advertises it has been
    # written: an index nothing points at is untidy, one pointed at but absent
    # is broken. Covers both the zero-byte file a failed encode leaves behind
    # and a good index left by an earlier run.
    (out.parent / _INDEX_NAME).unlink(missing_ok=True)
    return False


def update_catalog(
    tool: Path,
    collection_jsons: list[Path],
    out_dir: Path,
    config: Path,
    timeout: float = DEFAULT_TIMEOUT,
) -> None:
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
    _run(cmd, "update-catalog", timeout)


def _run(cmd: list[str], what: str, timeout: float, tolerate: str | None = None) -> str | None:
    """Run `cmd`, raising on failure. Returns the stderr of a tolerated failure.

    `tolerate` is a lower-cased substring of the one failure the caller intends
    to recover from; anything else still raises, as does a timeout.

    A failure whose stderr names a host failure raises `HostFailure` instead of
    a plain `RuntimeError`: the subprocess having no disk left is no more a
    fact about the data than this process having none, and only the tool's
    stderr can say which of the two happened.
    """
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"{what} failed: timed out after {timeout}s") from exc
    if proc.returncode == 0:
        return None
    detail = proc.stderr.strip()[:MAX_DETAIL_CHARS]
    if is_host_failure(detail):
        # Ahead of `tolerate`, so a sidecar the kernel refused to write is
        # never mistaken for one the encoder declined to build: tolerating it
        # would have `update_collection` return True for an index that does not
        # exist, and the run would report a degraded collection as complete.
        raise HostFailure(f"{what} failed: {detail}")
    if tolerate and tolerate in detail.lower():
        return detail
    if _NO_EXTENT in detail.lower():
        detail += (
            " — every Item in this collection is unlocated (no bbox), because"
            " the package CRS could not be reprojected to WGS84; the tool"
            " cannot compute a spatial extent from them"
        )
    raise RuntimeError(f"{what} failed: {detail}")
