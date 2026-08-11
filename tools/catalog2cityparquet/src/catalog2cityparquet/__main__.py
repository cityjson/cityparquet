"""Drive the whole conversion: catalogue in, CityParquet mirror out.

Failure isolation is the design centre. An item that fails is recorded and the
next one starts; a collection that fails is recorded and the next one starts.
The process exits non-zero only when the catalogue root itself is unreachable,
which is the one case where nothing at all could be attempted — a run with
failures is a successful run that *measured* failures, and the measurement is
the deliverable.

Everything here is orchestration: which bytes to fetch, in what order, and how
to record what happened. No CityJSON, CityGML or Parquet is interpreted in this
process — the Rust `cityparquet` binary owns every format decision, and
`city3dstac` owns every aggregation decision.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path

import httpx

from . import aggregate, convert, discover, fetch
from .discover import Item
from .ledger import Ledger, Record

BASE_URL = "https://storage.googleapis.com/city3d-stac"
BUCKET_API = "https://storage.googleapis.com/storage/v1/b/city3d-stac/o"

#: Concurrent downloads/conversions by default. This is a politeness bound, not
#: a capacity one: the whole catalogue is served from a handful of origins, some
#: of them small national portals, and hammering one of them would be both rude
#: and self-defeating (a throttled origin fails items that would otherwise
#: convert, corrupting the very measurement the run exists to produce).
DEFAULT_JOBS = 8

#: How long a catalogue/collection metadata request may take. Item payloads get
#: `Config.download_timeout` instead; these are small JSON documents.
METADATA_TIMEOUT = 120.0

#: Longest exception text kept on a ledger record — one record is one line.
MAX_ERROR_CHARS = 2000

#: Placeholder item id for a record about a whole collection rather than an
#: item. The ledger's JSONL is read per collection, so a sentinel is clearer
#: than an empty string.
COLLECTION_LEVEL = "-"


@dataclass
class Config:
    """Everything a run needs that is not derived from the catalogue itself."""

    out: Path
    binary: Path
    tool: Path
    jobs: int = DEFAULT_JOBS
    skip_existing: bool = True
    keep_downloads: bool = False
    limit_per_collection: int | None = None
    crs_by_collection: dict[str, str] = field(default_factory=dict)
    base_url: str = BASE_URL
    bucket_api: str = BUCKET_API
    download_timeout: float = 1800.0
    convert_timeout: float = 3600.0


@dataclass
class RunState:
    """What the run learned that the ledger has no column for.

    Currently just the collections that ended up without a GeoParquet index:
    `aggregate.update_collection` degrades rather than failing when a single
    unlocated Item defeats the STAC-GeoParquet encoder, and "how many
    collections got an index" is a number this project needs to be able to
    state.
    """

    no_index: list[str] = field(default_factory=list)
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False, compare=False)

    def note_no_index(self, cid: str) -> None:
        with self._lock:
            self.no_index.append(cid)


@dataclass
class ItemStats:
    """What one item cost, filled in as it goes.

    Passed in rather than returned so the numbers survive an exception: the
    bytes an item moved before failing to convert are exactly as interesting as
    the bytes a successful one moved.
    """

    downloaded: int = 0


def package_dir(config: Config, item: Item) -> Path:
    return config.out / item.collection / "items" / item.item_id


def already_converted(config: Config, item: Item) -> bool:
    """Whether a previous run finished this item.

    A partially written package is safe to re-attempt: the Rust writer commits
    atomically and renames `metadata.json` last, so a half-finished conversion
    never leaves a parseable Item behind. Anything unparseable — or absent
    beside a directory full of Parquet — therefore means "not finished".
    """
    path = package_dir(config, item) / "metadata.json"
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    return isinstance(doc, dict) and doc.get("type") == "Feature" and "stac_version" in doc


def process_item(
    item: Item, *, config: Config, client: httpx.Client, stats: ItemStats | None = None
) -> int:
    """Download, normalise, convert and stamp one item. Returns object count.

    The working directory lives under `--out` rather than in the system
    temporary directory: payloads reach gigabytes, and `/tmp` is a small tmpfs
    on many machines. It is removed per item unless `--keep-downloads`,
    *including* when the item failed — the failure is already in the ledger,
    and hundreds of abandoned downloads would fill the volume long before the
    run ended.
    """
    work_root = config.out / "_work"
    work_root.mkdir(parents=True, exist_ok=True)
    workdir = Path(tempfile.mkdtemp(prefix="c2cp-", dir=work_root))
    try:
        # The saved name matters: `normalise` decides convertibility from the
        # suffix, and several origins hide the real filename in a query
        # parameter, so a fixed name would discard whole collections.
        source = workdir / fetch.local_name(item.href)
        downloaded = fetch.download(item.href, source, client, timeout=config.download_timeout)
        if stats is not None:
            stats.downloaded = downloaded
        inputs = fetch.normalise(source, workdir / "extract")
        if not inputs:
            raise convert.ConvertError("unsupported_archive", "no convertible file in the asset")
        out_dir = package_dir(config, item)
        count = convert.run_convert(
            config.binary,
            inputs,
            out_dir,
            config.crs_by_collection.get(item.collection),
            timeout=config.convert_timeout,
        )
        convert.stamp(out_dir, item)
        return count
    finally:
        if not config.keep_downloads:
            shutil.rmtree(workdir, ignore_errors=True)


def convert_items(items: list[Item], *, ledger: Ledger, config: Config) -> None:
    """Convert every item, isolating each failure to its own record."""
    client = httpx.Client(timeout=config.download_timeout, follow_redirects=True)

    def handle(item: Item) -> None:
        if fetch.is_duplicate_bundle(item):
            # Skipped before the download, which is the whole point: these are
            # hundreds of gigabytes of data we convert from its tiles instead.
            ledger.record(
                Record(item.collection, item.item_id, "skipped", reason="duplicate_bundle")
            )
            return
        if config.skip_existing and already_converted(config, item):
            # No record at all: this is not an outcome of *this* run, and
            # counting it would make a resumed run look like a fresh success.
            return
        started = time.monotonic()
        stats = ItemStats()
        try:
            process_item(item, config=config, client=client, stats=stats)
        except convert.ConvertError as exc:
            # Already classified against the ledger's closed vocabulary.
            _fail(ledger, item, exc.reason, exc.detail, started, stats)
        except (httpx.HTTPError, OSError) as exc:
            # Transport and filesystem problems are the origin's fault, not the
            # converter's; keeping them a separate reason stops upstream
            # flakiness from inflating the converter's failure count.
            _fail(ledger, item, "download_failed", str(exc), started, stats)
        # Deliberately broad: one item must never stop the run.
        except Exception as exc:
            _fail(ledger, item, "convert_failed", f"{type(exc).__name__}: {exc}", started, stats)
        else:
            ledger.record(
                Record(
                    item.collection,
                    item.item_id,
                    "converted",
                    bytes=stats.downloaded,
                    seconds=time.monotonic() - started,
                )
            )

    try:
        if config.jobs <= 1:
            for item in items:
                handle(item)
        else:
            with ThreadPoolExecutor(max_workers=config.jobs) as pool:
                # Consumed eagerly so the pool is drained inside the try block.
                list(pool.map(handle, items))
    finally:
        client.close()


def _fail(
    ledger: Ledger, item: Item, reason: str, detail: str, started: float, stats: ItemStats
) -> None:
    ledger.record(
        Record(
            item.collection,
            item.item_id,
            "failed",
            reason=reason,
            error=detail[:MAX_ERROR_CHARS],
            bytes=stats.downloaded,
            seconds=time.monotonic() - started,
        )
    )


def convert_collection(
    cid: str, *, ledger: Ledger, config: Config, client: httpx.Client, state: RunState
) -> None:
    """Convert one collection and aggregate the packages into a collection.json.

    Aggregation runs over the output *directory*, not over this run's
    successes, so a resumed run rebuilds the collection from everything ever
    converted.
    """
    collection = discover.fetch_collection(config.base_url, cid, client)
    items, note = discover.enumerate_items(
        config.base_url, config.bucket_api, cid, collection, client
    )
    if note:
        # A published index disagreeing with the object listing is a fact about
        # the catalogue worth recording, not just a log line.
        print(f"  ! {cid}: {note}", file=sys.stderr)
        ledger.record(
            Record(cid, COLLECTION_LEVEL, "skipped", reason="stale_item_index", error=note)
        )
    if not items:
        # 20 of the 53 collections publish only a collection.json.
        ledger.record(Record(cid, COLLECTION_LEVEL, "skipped", reason="empty_collection"))
        print(f"==> {cid}: no items")
        return
    if config.limit_per_collection:
        items = items[: config.limit_per_collection]
    print(f"==> {cid}: {len(items)} item(s)")
    convert_items(items, ledger=ledger, config=config)

    config_path = aggregate.write_config(
        aggregate.collection_config(collection), config.out / "_configs" / f"{cid}.yaml"
    )
    indexed = aggregate.update_collection(
        config.tool,
        config.out / cid / "items",
        config_path,
        config.out / cid / "collection.json",
    )
    if not indexed:
        state.note_no_index(cid)


def run_collections(
    cids: list[str], *, ledger: Ledger, config: Config, state: RunState | None = None
) -> RunState:
    """Attempt every collection; a failure is recorded, never fatal."""
    state = RunState() if state is None else state
    client = httpx.Client(timeout=METADATA_TIMEOUT, follow_redirects=True)
    try:
        for cid in cids:
            try:
                convert_collection(cid, ledger=ledger, config=config, client=client, state=state)
            # Deliberately broad: one collection must never stop the run.
            except Exception as exc:
                print(f"  ! {cid} failed: {exc}", file=sys.stderr)
                ledger.record(
                    Record(
                        cid,
                        COLLECTION_LEVEL,
                        "failed",
                        reason="convert_failed",
                        error=f"{type(exc).__name__}: {exc}"[:MAX_ERROR_CHARS],
                    )
                )
    finally:
        client.close()
    return state


def aggregate_all(config: Config, state: RunState) -> None:
    """Link every collection written so far into one catalogue.

    Never raises: the per-collection outputs are already on disk and complete
    in themselves, so a failure here degrades the mirror's root rather than the
    run. It is reported on stderr instead.
    """
    catalog: dict = {}
    client = httpx.Client(timeout=METADATA_TIMEOUT, follow_redirects=True)
    try:
        response = client.get(f"{config.base_url}/catalog.json")
        response.raise_for_status()
        catalog = response.json()
    # Deliberately broad: the root's identity is nice to have, not required.
    except Exception as exc:
        print(f"  ! catalogue root metadata unavailable ({exc}); using defaults", file=sys.stderr)
    finally:
        client.close()

    collections = sorted(config.out.glob("*/collection.json"))
    if not collections:
        print("  ! no collection.json written; skipping catalogue aggregation", file=sys.stderr)
        return
    try:
        config_path = aggregate.write_config(
            aggregate.catalog_config(catalog), config.out / "_configs" / "catalog.yaml"
        )
        aggregate.update_catalog(config.tool, collections, config.out, config_path)
    # Deliberately broad: the collections stand on their own without a root.
    except Exception as exc:
        print(f"  ! catalogue aggregation failed: {exc}", file=sys.stderr)


def print_summary(ledger: Ledger, state: RunState) -> None:
    """Print what the run measured: outcomes, reasons, and degraded indexes."""
    summary = ledger.write_summary()
    print("\n--- summary ---")
    print(f"{'collection':<32} {'converted':>9} {'failed':>7} {'skipped':>8}")
    totals = {"converted": 0, "failed": 0, "skipped": 0}
    for collection in ledger.collections():
        counts = ledger.counts(collection)
        for status in totals:
            totals[status] += counts.get(status, 0)
        print(
            f"{collection:<32} {counts.get('converted', 0):>9} "
            f"{counts.get('failed', 0):>7} {counts.get('skipped', 0):>8}"
        )
    print(f"{'TOTAL':<32} {totals['converted']:>9} {totals['failed']:>7} {totals['skipped']:>8}")

    histogram = ledger.histogram()
    print("\nreasons:")
    if histogram:
        for reason, count in sorted(histogram.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {count:>7}  {reason}")
    else:
        print("  (none)")

    print(f"\ncollections without a GeoParquet index: {len(state.no_index)}")
    for cid in state.no_index:
        print(f"  - {cid}")
    print(f"\nledger: {summary.parent}")


def resolve_collections(config: Config) -> list[str]:
    """The collection ids to attempt, read from the catalogue root.

    Raises whatever the transport raises: this is the one failure that means
    nothing at all can be attempted.
    """
    client = httpx.Client(timeout=METADATA_TIMEOUT, follow_redirects=True)
    try:
        cids, note = discover.collection_ids(config.base_url, client)
    finally:
        client.close()
    if note:
        # A child link we could not turn into a collection id is a collection
        # silently missing from the run; it must not pass unmentioned.
        print(f"  ! {note}", file=sys.stderr)
    return cids


def run(
    config: Config, collections: list[str] | None = None, *, aggregate_only: bool = False
) -> int:
    """Execute a whole run. Returns the process exit code."""
    config.out.mkdir(parents=True, exist_ok=True)
    ledger = Ledger(config.out / "_reports")
    state = RunState()

    if not aggregate_only:
        cids = list(collections) if collections else None
        if cids is None:
            try:
                cids = resolve_collections(config)
            # Deliberately broad: whatever the failure, nothing could be attempted.
            except Exception as exc:
                print(f"catalogue root unreachable: {exc}", file=sys.stderr)
                return 1
        run_collections(cids, ledger=ledger, config=config, state=state)

    aggregate_all(config, state)
    print_summary(ledger, state)
    return 0


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="catalog2cityparquet",
        description="Convert a published City3D STAC catalogue into CityParquet packages.",
    )
    parser.add_argument("--out", type=Path, default=Path("out/cityparquet-catalog"))
    parser.add_argument("--binary", type=Path, default=Path("target/release/cityparquet"))
    parser.add_argument(
        "--tool",
        type=Path,
        default=Path("vendor/city3d-stac-tool/target/release/city3dstac"),
    )
    parser.add_argument(
        "--collection",
        action="append",
        dest="collections",
        help="convert only this collection; repeatable",
    )
    parser.add_argument("--limit-per-collection", type=int)
    parser.add_argument(
        "--jobs",
        type=int,
        default=DEFAULT_JOBS,
        help=f"concurrent items (default {DEFAULT_JOBS}; a politeness bound on the origins)",
    )
    parser.add_argument("--keep-downloads", action="store_true")
    parser.add_argument(
        "--no-skip-existing",
        action="store_true",
        help="re-convert items that already have a valid metadata.json",
    )
    parser.add_argument(
        "--crs",
        action="append",
        default=[],
        metavar="COLLECTION=EPSG:xxxx",
        help="fallback CRS for a collection whose sources declare none; repeatable",
    )
    parser.add_argument("--base-url", default=BASE_URL)
    parser.add_argument("--bucket-api", default=BUCKET_API)
    parser.add_argument(
        "--aggregate-only",
        action="store_true",
        help="rebuild collections and catalogue from what is already converted",
    )
    return parser.parse_args(argv)


def config_from_args(args: argparse.Namespace) -> Config:
    """Turn parsed arguments into a `Config`, rejecting a malformed `--crs`.

    Rejected here rather than at first use: a typo that only surfaced after
    forty collections had been converted would waste the whole run.
    """
    crs_by_collection: dict[str, str] = {}
    for pair in args.crs:
        collection, separator, code = pair.partition("=")
        if not separator or not collection or not code:
            raise SystemExit(f"--crs expects COLLECTION=EPSG:xxxx, got {pair!r}")
        crs_by_collection[collection] = code
    return Config(
        out=args.out,
        binary=args.binary,
        tool=args.tool,
        jobs=args.jobs,
        skip_existing=not args.no_skip_existing,
        keep_downloads=args.keep_downloads,
        limit_per_collection=args.limit_per_collection,
        crs_by_collection=crs_by_collection,
        base_url=args.base_url,
        bucket_api=args.bucket_api,
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    return run(
        config_from_args(args),
        args.collections,
        aggregate_only=args.aggregate_only,
    )


if __name__ == "__main__":
    raise SystemExit(main())
