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
import contextlib
import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field, replace
from pathlib import Path, PurePosixPath

import httpx

from . import aggregate, convert, discover, fetch
from .discover import Item
from .ledger import Ledger, Record, validate_collection_id

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

#: Prefix for this driver's per-item working directories. Distinctive so the
#: start-of-run sweep can tell its own leftovers from anything else that shares
#: the working directory.
WORK_PREFIX = "c2cp-"

#: Claim on a working directory, held for the length of a converting run.
LOCK_NAME = ".c2cp-lock"

#: Read once: the lock records it so a lock file seen over a shared filesystem
#: is never mistaken for a local pid.
HOSTNAME = socket.gethostname()


class WorkRootBusy(RuntimeError):
    """Another live run holds the working directory."""


def _warn(message: str) -> None:
    """Say something on stderr, treating a failed say as nothing at all.

    Every isolation handler ends by reporting what went wrong, and stderr is as
    capable of failing as the ledger beside it: a full volume raises `OSError`,
    and a run piped through `head` raises `BrokenPipeError` on the next write.
    Either would escape the handler and take down the run the handler exists to
    protect — so the diagnostic is best-effort by construction.
    """
    with contextlib.suppress(Exception):
        print(message, file=sys.stderr)


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
    #: Where per-item working directories are made. `None` means `out/_work`.
    work_dir: Path | None = None
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


def _record_safely(ledger: Ledger, rec: Record) -> None:
    """Write one record, treating a failure to write as news rather than an end.

    Every isolation handler in this module ends by recording what went wrong,
    on the same disk that may be the thing going wrong — and a rejected id or a
    full volume raising *here* would defeat the handler and take down the run
    it exists to protect. The loss is reported on stderr and the run continues:
    a missing line in the ledger is a smaller failure than a missing half of
    the catalogue.
    """
    try:
        ledger.record(rec)
    # Deliberately broad: nothing about writing a record may end the run.
    except Exception as exc:
        # The last-resort reporter, so it swallows unconditionally: `_warn`
        # already cannot raise, and anything this clause did raise would escape
        # the very handler that called it. (`suppress(Exception)` rather than a
        # bare `except: pass` only because ruff's SIM105 forbids the literal
        # form; the guarantee is the same.)
        with contextlib.suppress(Exception):
            _warn(f"  ! outcome for {rec.collection}/{rec.item_id} could not be recorded: {exc}")


def safe_item_id(item_id: str) -> str:
    """The single path component `item_id` may safely become.

    Item ids come from a published catalogue and are interpolated into an
    output path, so they get the same treatment as collection ids and asset
    filenames: reduced to a last component, which cannot climb out of the
    package directory. An id with no usable component raises, and the caller
    ledgers that one item rather than trusting it.
    """
    name = PurePosixPath(str(item_id).replace("\\", "/")).name
    if name in ("", ".", ".."):
        raise ValueError(f"unusable item id {item_id!r}: no safe path component")
    return name


def package_dir(config: Config, item: Item) -> Path:
    """Where this item's package lives — from ids that are never trusted.

    Both components are catalogue-supplied, so both are validated here rather
    than at the call sites: this is the one place where they become a path.
    """
    return (
        config.out / validate_collection_id(item.collection) / "items" / safe_item_id(item.item_id)
    )


def work_root(config: Config) -> Path:
    """The directory per-item working directories are made in."""
    return config.work_dir if config.work_dir is not None else config.out / "_work"


def _lock_holder_is_live(info: dict) -> bool:
    """Whether the process named in a lock file is still running.

    Anything unreadable counts as live. Refusing to start is recoverable — the
    message says which file to delete — whereas sweeping a live run's
    directory destroys a download in progress, and that reappears in the
    ledger as `download_failed`, indistinguishable from an origin having a bad
    day. The safe answer to "I cannot tell" is therefore "yes".
    """
    if info.get("host") != HOSTNAME:
        # A lock seen over a shared filesystem: the pid means nothing here.
        return True
    pid = info.get("pid")
    if not isinstance(pid, int) or pid <= 0:
        return True
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        # Someone else's process — running, just not ours to signal.
        return True
    return True


def acquire_work_root(config: Config) -> Path:
    """Claim the working directory for this run, or raise `WorkRootBusy`.

    Two runs sharing one working directory would sweep each other's live
    downloads, and this driver is built for multi-day runs where opening a
    second shell against the same `--out` is the natural thing to do. The
    corrupted measurement that would follow is worth a startup error.
    """
    root = work_root(config)
    root.mkdir(parents=True, exist_ok=True)
    path = root / LOCK_NAME
    try:
        handle = path.open("x", encoding="utf-8")
    except FileExistsError:
        info: dict = {}
        with contextlib.suppress(OSError, ValueError):
            loaded = json.loads(path.read_text(encoding="utf-8"))
            info = loaded if isinstance(loaded, dict) else {}
        if _lock_holder_is_live(info):
            raise WorkRootBusy(
                f"another run holds the working directory {root} "
                f"(pid {info.get('pid', '?')} on {info.get('host', '?')}). "
                f"Use --work-dir for a second run, or delete {path} if no other run is active."
            ) from None
        # The holder is gone; its claim is not.
        with contextlib.suppress(OSError):
            path.unlink()
        try:
            handle = path.open("x", encoding="utf-8")
        except FileExistsError:
            raise WorkRootBusy(f"another run claimed {root} while this one was starting") from None
    with handle:
        handle.write(
            json.dumps({"pid": os.getpid(), "host": HOSTNAME, "started": time.time()}) + "\n"
        )
    return path


def release_work_root(lock: Path) -> None:
    """Drop this run's claim. Never raises: the run is over either way."""
    with contextlib.suppress(OSError):
        lock.unlink()


def sweep_work_root(config: Config) -> None:
    """Delete working directories an earlier run abandoned.

    A run killed mid-item leaves its partial download behind, and by default
    that sits inside the deliverable mirror. Swept at the start rather than at
    the end, because the run that made the mess is by definition not around to
    clean it up. `--keep-downloads` is honoured: an operator who asked to keep
    them means across runs too.

    Only ever called while this run holds the working directory's lock, so what
    it deletes is certainly abandoned and not another run's work in progress.
    """
    if config.keep_downloads:
        return
    for path in sorted(work_root(config).glob(f"{WORK_PREFIX}*")):
        if path.is_dir():
            shutil.rmtree(path, ignore_errors=True)


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

    The working directory lives under `--work-dir` (by default `<out>/_work`)
    rather than in the system temporary directory: payloads reach gigabytes,
    and `/tmp` is a small tmpfs on many machines. It is removed per item unless
    `--keep-downloads`, *including* when the item failed — the failure is
    already in the ledger, and hundreds of abandoned downloads would fill the
    volume long before the run ended.
    """
    root = work_root(config)
    root.mkdir(parents=True, exist_ok=True)
    workdir = Path(tempfile.mkdtemp(prefix=WORK_PREFIX, dir=root))
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


def convert_items(
    items: list[Item], *, ledger: Ledger, config: Config, collection: str | None = None
) -> None:
    """Convert every item, isolating each failure to its own record.

    `collection` is the id being converted, and it overrides whatever the
    catalogue put in `Item.collection`. That field is not authoritative:
    `discover.items_from_parquet` reads it straight out of a published
    `items.parquet` and falls back to `""` on a null cell, and one such cell
    would otherwise send the packages to `out//items/` where aggregation (which
    reads `out/<cid>/items`) cannot see them, defeat the duplicate-bundle
    filter, and have the ledger reject the record — losing the whole collection
    over one null. The id we asked for is the id we record against.
    """
    client = httpx.Client(timeout=config.download_timeout, follow_redirects=True)

    def handle(item: Item) -> None:
        if collection is not None:
            item = replace(item, collection=collection)
        started = time.monotonic()
        stats = ItemStats()
        try:
            if fetch.is_duplicate_bundle(item):
                # Skipped before the download, which is the whole point: these
                # are hundreds of gigabytes of data we convert from its tiles.
                _record_safely(
                    ledger,
                    Record(item.collection, item.item_id, "skipped", reason="duplicate_bundle"),
                )
                return
            if config.skip_existing and already_converted(config, item):
                # No record at all: this is not an outcome of *this* run, and
                # counting it would make a resumed run look like a fresh
                # success.
                return
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
            _record_safely(
                ledger,
                Record(
                    item.collection,
                    item.item_id,
                    "converted",
                    bytes=stats.downloaded,
                    seconds=time.monotonic() - started,
                ),
            )

    # Which items have been taken up, so the serial fallback below can finish
    # the collection without redoing — and double-recording — anything.
    claimed: set[int] = set()
    claim_lock = threading.Lock()

    def handle_once(numbered: tuple[int, Item]) -> None:
        index, item = numbered
        with claim_lock:
            if index in claimed:
                return
            claimed.add(index)
        handle(item)

    try:
        if config.jobs <= 1:
            for numbered in enumerate(items):
                handle_once(numbered)
        else:
            try:
                with ThreadPoolExecutor(max_workers=config.jobs) as pool:
                    # Consumed eagerly so the pool is drained inside this block.
                    list(pool.map(handle_once, list(enumerate(items))))
            # Deliberately broad: a pool that cannot run (a host out of threads
            # raises RuntimeError from submit) must be contained exactly as an
            # item failure is. The remainder is finished serially rather than
            # dropped, because silently converting half a collection and
            # aggregating it as whole is the failure this driver exists to
            # prevent.
            except Exception as exc:
                _warn(f"  ! thread pool failed ({exc}); finishing this collection serially")
                for numbered in enumerate(items):
                    handle_once(numbered)
    finally:
        client.close()


def _fail(
    ledger: Ledger, item: Item, reason: str, detail: str, started: float, stats: ItemStats
) -> None:
    _record_safely(
        ledger,
        Record(
            item.collection,
            item.item_id,
            "failed",
            reason=reason,
            error=detail[:MAX_ERROR_CHARS],
            bytes=stats.downloaded,
            seconds=time.monotonic() - started,
        ),
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
        _warn(f"  ! {cid}: {note}")
        _record_safely(
            ledger, Record(cid, COLLECTION_LEVEL, "skipped", reason="stale_item_index", error=note)
        )
    if not items:
        # 20 of the 53 collections publish only a collection.json.
        _record_safely(ledger, Record(cid, COLLECTION_LEVEL, "skipped", reason="empty_collection"))
        print(f"==> {cid}: no items")
        return
    if config.limit_per_collection is not None:
        items = items[: config.limit_per_collection]
    print(f"==> {cid}: {len(items)} item(s)")
    convert_items(items, ledger=ledger, config=config, collection=cid)

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
                _warn(f"  ! {cid} failed: {exc}")
                _record_safely(
                    ledger,
                    Record(
                        cid,
                        COLLECTION_LEVEL,
                        "failed",
                        reason="convert_failed",
                        error=f"{type(exc).__name__}: {exc}"[:MAX_ERROR_CHARS],
                    ),
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
        _warn(f"  ! catalogue root metadata unavailable ({exc}); using defaults")
    finally:
        client.close()

    collections = sorted(config.out.glob("*/collection.json"))
    if not collections:
        _warn("  ! no collection.json written; skipping catalogue aggregation")
        return
    try:
        config_path = aggregate.write_config(
            aggregate.catalog_config(catalog), config.out / "_configs" / "catalog.yaml"
        )
        aggregate.update_catalog(config.tool, collections, config.out, config_path)
    # Deliberately broad: the collections stand on their own without a root.
    except Exception as exc:
        _warn(f"  ! catalogue aggregation failed: {exc}")


def print_summary(ledger: Ledger, state: RunState) -> None:
    """Print what the run measured: outcomes, reasons, and degraded indexes.

    The tallies are printed whether or not `summary.csv` could be written: by
    the time this runs the conversion is over, and turning a completed
    measurement into a traceback because a roll-up file could not be saved
    would throw away the run's whole point. The stdout report is the primary
    artefact; the CSV is a convenience.
    """
    summary: Path | None = None
    try:
        summary = ledger.write_summary()
    # Deliberately broad: a lost roll-up must not cost the report.
    except Exception as exc:
        _warn(f"  ! summary.csv could not be written: {exc}")
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
    if summary is not None:
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
        _warn(f"  ! {note}")
    return cids


def run(
    config: Config, collections: list[str] | None = None, *, aggregate_only: bool = False
) -> int:
    """Execute a whole run. Returns the process exit code.

    Non-zero means nothing was measured: the output directory could not be
    prepared, another run holds the working directory, or the catalogue root is
    unreachable. Once items start being attempted the answer is 0, however many
    of them failed.
    """
    try:
        config.out.mkdir(parents=True, exist_ok=True)
        ledger = Ledger(config.out / "_reports")
    except OSError as exc:
        # Nothing can be recorded, so there is nothing to run. Reported like the
        # other startup errors rather than as a bare traceback.
        _warn(f"cannot prepare the output directory {config.out}: {exc}")
        return 1
    state = RunState()

    if not aggregate_only:
        try:
            work_lock = acquire_work_root(config)
        except (WorkRootBusy, OSError) as exc:
            _warn(f"cannot start: {exc}")
            return 1
        try:
            cids = list(collections) if collections else None
            if cids is None:
                try:
                    cids = resolve_collections(config)
                # Deliberately broad: whatever the failure, nothing could be attempted.
                except Exception as exc:
                    _warn(f"catalogue root unreachable: {exc}")
                    return 1
            sweep_work_root(config)
            run_collections(cids, ledger=ledger, config=config, state=state)
        finally:
            release_work_root(work_lock)

    aggregate_all(config, state)
    try:
        print_summary(ledger, state)
    # Deliberately broad: the run is over, and a report that cannot be printed
    # (a closed stdout, say) must not turn a completed measurement into a
    # traceback. The ledger on disk is the durable copy.
    except Exception as exc:
        _warn(f"  ! the summary could not be printed: {exc}")
    return 0


def _positive_int(text: str) -> int:
    """An argparse type for counts, so `0` cannot quietly mean "everything".

    The old truthiness test read `--limit-per-collection 0` as "no limit" and a
    negative value as "silently drop the tail"; both are typos, and both would
    have produced a plausible-looking run that measured the wrong thing.
    """
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError(f"expected a count of 1 or more, got {value}")
    return value


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
    parser.add_argument("--limit-per-collection", type=_positive_int)
    parser.add_argument(
        "--jobs",
        type=_positive_int,
        default=DEFAULT_JOBS,
        help=f"concurrent items (default {DEFAULT_JOBS}; a politeness bound on the origins)",
    )
    parser.add_argument("--keep-downloads", action="store_true")
    parser.add_argument(
        "--work-dir",
        type=Path,
        help="where downloads are unpacked (default <out>/_work); needs room for the "
        "largest single payload, and must not be shared with a concurrent run",
    )
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
    """Turn parsed arguments into a validated `Config`.

    Both checks below happen here rather than at first use: a typo that only
    surfaced after forty collections had been converted would waste the whole
    run, and a collection id reaching the ledger unchecked used to abort it
    outright.
    """
    crs_by_collection: dict[str, str] = {}
    for pair in args.crs:
        collection, separator, code = pair.partition("=")
        if not separator or not collection or not code:
            raise SystemExit(f"--crs expects COLLECTION=EPSG:xxxx, got {pair!r}")
        crs_by_collection[collection] = code
    for cid in args.collections or []:
        try:
            validate_collection_id(cid)
        except ValueError as exc:
            raise SystemExit(f"--collection: {exc}") from exc
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
        work_dir=args.work_dir,
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
