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
from .ledger import ENVIRONMENT, Ledger, Record, validate_collection_id

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

#: How many environment failures the summary quotes in full. The count is exact
#: however large it grows; only the listing is capped, so a run that hit the
#: same broken volume 60,000 times does not print 60,000 identical lines.
MAX_ENVIRONMENT_NOTES = 20

#: Placeholder item id for a record about a whole collection rather than an
#: item. The ledger's JSONL is read per collection, so a sentinel is clearer
#: than an empty string.
COLLECTION_LEVEL = "-"

#: Prefix for this driver's per-item working directories. Distinctive so the
#: start-of-run sweep can tell its own leftovers from anything else that shares
#: the working directory.
WORK_PREFIX = "c2cp-"

#: Name of the file by which a run claims a directory it must not share.
LOCK_NAME = ".c2cp-lock"

#: Read once: the lock records it so a lock file seen over a shared filesystem
#: is never mistaken for a local pid.
HOSTNAME = socket.gethostname()

#: What a lock protects, and the flag that gives a second run its own.
OUTPUT_DIRECTORY = "output directory"
WORKING_DIRECTORY = "working directory"
_ALTERNATIVE = {OUTPUT_DIRECTORY: "--out", WORKING_DIRECTORY: "--work-dir"}


class LockBusy(RuntimeError):
    """Another live run holds a directory this one needs."""


@dataclass(frozen=True)
class Claim:
    """A lock this run holds, and the exact bytes proving it is ours."""

    path: Path
    owner: str


def _describe(exc: BaseException) -> str:
    """Render an exception, treating an unrenderable one as news rather than an end.

    `_warn` and `_record_safely` guard the *call*, but the f-string that builds
    their argument is evaluated at the call site, outside every guard — so an
    exception whose `__str__` raises (a third-party error carrying a broken
    `args`, say) propagates out of the isolation handler and the remaining
    collections are never attempted. Every message built on an isolation path
    goes through here, so the guard covers construction as well as delivery.
    """
    try:
        return f"{type(exc).__name__}: {exc}"
    # Deliberately broad: rendering an error may not become a bigger error.
    except Exception:
        with contextlib.suppress(Exception):
            return f"{type(exc).__name__}: <the exception could not be rendered>"
        return "an exception that could not be rendered"


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


def _say(message: str) -> None:
    """Write a line of progress or report, best-effort.

    Progress lines are written from *inside* the per-collection isolation
    handler, so an unguarded failure here would be caught by that handler and
    recorded as a data failure: a run whose log volume filled would report
    every collection as `convert_failed` and exit 0. The deliverable of this
    project is a measured statement about which collections convert, and a
    fabricated one is worse than a crash.
    """
    with contextlib.suppress(Exception):
        print(message)


def _flush_stream(stream, name: str) -> None:
    """Empty one standard stream's buffer before the interpreter tries to.

    CPython's `flush_std_files()` flushes **both** `sys.stdout` and
    `sys.stderr` while finalising, and a failure on *either* sets exit status
    120 — a non-zero exit for a run that measured everything, contradicting the
    contract that non-zero means nothing was measured. `_say` and `_warn`
    cannot prevent it: with a buffered stream the writes succeed into the
    buffer and only the final flush fails.

    A failed flush leaves the data in the buffer, so the descriptor is pointed
    at the null device and the flush retried; the bytes are already lost, and
    the ledger on disk is the durable copy of everything printed here.
    """
    try:
        stream.flush()
        return
    # Deliberately broad: this is the last thing the process does.
    except Exception as exc:
        # Goes to stderr, which may be the stream that just failed — `_warn`
        # swallows that, and the stderr pass below neutralises it either way.
        _warn(
            f"  ! {name} could not be flushed ({_describe(exc)}); "
            f"the report was lost, the ledger was not"
        )
    with contextlib.suppress(Exception):
        os.dup2(os.open(os.devnull, os.O_WRONLY), stream.fileno())
    with contextlib.suppress(Exception):
        stream.flush()


def _flush_streams() -> None:
    """Neutralise both standard streams so neither can outlive the run.

    stdout first: its failure is reported on stderr, which is flushed after.
    """
    _flush_stream(sys.stdout, "stdout")
    _flush_stream(sys.stderr, "stderr")


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
    """What the run learned that the ledger has no column for, or could not keep.

    Two things. The collections that ended up without a GeoParquet index:
    `aggregate.update_collection` degrades rather than failing when a single
    unlocated Item defeats the STAC-GeoParquet encoder, and "how many
    collections got an index" is a number this project needs to be able to
    state. And the environment failures, counted here as well as ledgered
    because *the ledger is one of the things that fails*: a run whose reports
    volume filled must still be able to say so at the end rather than print a
    clean-looking table.
    """

    no_index: list[str] = field(default_factory=list)
    #: Every environment failure this process saw, ledgered or not.
    environment_seen: int = 0
    #: The first few of them, in full, for the summary.
    environment: list[str] = field(default_factory=list)
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False, compare=False)

    def note_no_index(self, cid: str) -> None:
        with self._lock:
            self.no_index.append(cid)

    def note_environment(self, note: str) -> None:
        """Remember that this machine, not the data, was what failed."""
        with self._lock:
            self.environment_seen += 1
            if len(self.environment) < MAX_ENVIRONMENT_NOTES:
                self.environment.append(note)


@dataclass
class ItemStats:
    """What one item cost, filled in as it goes.

    Passed in rather than returned so the numbers survive an exception: the
    bytes an item moved before failing to convert are exactly as interesting as
    the bytes a successful one moved.
    """

    downloaded: int = 0


def _record_safely(ledger: Ledger, rec: Record, state: RunState | None = None) -> None:
    """Write one record, treating a failure to write as news rather than an end.

    Every isolation handler in this module ends by recording what went wrong,
    on the same disk that may be the thing going wrong — and a rejected id or a
    full volume raising *here* would defeat the handler and take down the run
    it exists to protect. The loss is reported on stderr and counted against
    `state` (the ledger cannot count what it could not write), and the run
    continues: a missing line in the ledger is a smaller failure than a missing
    half of the catalogue.
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
            if state is not None:
                state.note_environment(f"{rec.collection}: an outcome could not be recorded")
            _warn(
                f"  ! outcome for {rec.collection}/{rec.item_id} "
                f"could not be recorded: {_describe(exc)}"
            )


def _environment_failure(
    ledger: Ledger,
    collection: str,
    item_id: str,
    detail: str,
    state: RunState | None = None,
) -> None:
    """Record that *this machine* failed here — never that the data is unconvertible.

    The whole point of the run is a measured statement about which collections
    convert. A full volume, an unwritable `_configs`, a `city3dstac` that was
    never built: none of them is evidence about a dataset, and recording them
    as `convert_failed` publishes fabricated conformance data with the real
    packages sitting on disk beside it.

    `state` is notified first and `_record_safely` is deliberately called
    *without* it, so one environment failure is counted once even when the
    ledger is itself the thing that failed.
    """
    if state is not None:
        state.note_environment(f"{collection}/{item_id}: {detail}")
    _warn(f"  ! {collection}: environment failure (not a conversion failure): {detail}")
    _record_safely(
        ledger,
        Record(
            collection,
            item_id,
            ENVIRONMENT,
            reason=ENVIRONMENT,
            error=detail[:MAX_ERROR_CHARS],
        ),
    )


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


def acquire_lock(directory: Path, purpose: str) -> Claim:
    """Claim `directory` for this run, or raise `LockBusy`.

    Both of a run's shared resources are claimed, not just one. The working
    directory matters because two runs would sweep each other's live downloads;
    the output directory matters more, because two runs write two ledger lines
    per collection while `summary.csv` — rewritten wholesale by whichever
    finishes last — reports one. Locking only the working directory would let
    an operator follow the advice in this very message and produce exactly the
    corrupted measurement the lock exists to prevent.
    """
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / LOCK_NAME
    owner = json.dumps({"pid": os.getpid(), "host": HOSTNAME, "started": time.time()}) + "\n"
    try:
        handle = path.open("x", encoding="utf-8")
    except FileExistsError:
        info: dict = {}
        with contextlib.suppress(OSError, ValueError):
            loaded = json.loads(path.read_text(encoding="utf-8"))
            info = loaded if isinstance(loaded, dict) else {}
        if _lock_holder_is_live(info):
            raise LockBusy(
                f"another run holds the {purpose} {directory} "
                f"(pid {info.get('pid', '?')} on {info.get('host', '?')}). "
                f"Wait for it to finish, use a different {_ALTERNATIVE[purpose]}, "
                f"or delete {path} if no other run is active."
            ) from None
        # The holder is gone; its claim is not.
        with contextlib.suppress(OSError):
            path.unlink()
        try:
            handle = path.open("x", encoding="utf-8")
        except FileExistsError:
            raise LockBusy(f"another run claimed {directory} while this one was starting") from None
    with handle:
        handle.write(owner)
    return Claim(path=path, owner=owner)


def release_lock(claim: Claim) -> None:
    """Drop this run's claim, and only this run's.

    An operator who follows the busy message and deletes a lock they believe is
    stale may already have started a replacement run. Unlinking whatever file
    happens to be there would then have this run's exit revoke that run's live
    claim — so the contents are checked first.

    Never raises. It runs from an `ExitStack`'s unwinding in `run`'s `finally`,
    so anything it raised would escape `main` as a traceback and a non-zero
    exit *after* a fully measured run — the one thing the exit-code contract
    forbids. `ValueError` is caught beside `OSError` because a lock file
    holding invalid UTF-8 raises `UnicodeDecodeError`, which is neither an
    `OSError` nor rare on a volume that filled mid-write.
    """
    try:
        if claim.path.read_text(encoding="utf-8") != claim.owner:
            _warn(f"  ! not releasing {claim.path}: it is another run's claim now")
            return
    except (OSError, ValueError):
        # Unreadable or unparseable: not provably ours, so not ours to drop.
        return
    with contextlib.suppress(OSError):
        claim.path.unlink()


@contextlib.contextmanager
def locked(directory: Path, purpose: str):
    """Hold `directory`'s lock for the duration of the block."""
    claim = acquire_lock(directory, purpose)
    try:
        yield claim
    finally:
        release_lock(claim)


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
    items: list[Item],
    *,
    ledger: Ledger,
    config: Config,
    collection: str | None = None,
    state: RunState | None = None,
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
                    state,
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
            _fail(ledger, item, exc.reason, exc.detail, started, stats, state)
        except (httpx.HTTPError, OSError) as exc:
            # Transport and filesystem problems are the origin's fault, not the
            # converter's; keeping them a separate reason stops upstream
            # flakiness from inflating the converter's failure count.
            _fail(ledger, item, "download_failed", _describe(exc), started, stats, state)
        # Deliberately broad: one item must never stop the run.
        except Exception as exc:
            _fail(ledger, item, "convert_failed", _describe(exc), started, stats, state)
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
                state,
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
                _warn(
                    f"  ! thread pool failed ({_describe(exc)}); finishing this collection serially"
                )
                for numbered in enumerate(items):
                    handle_once(numbered)
    finally:
        client.close()


def _fail(
    ledger: Ledger,
    item: Item,
    reason: str,
    detail: str,
    started: float,
    stats: ItemStats,
    state: RunState | None = None,
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
        state,
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
            ledger,
            Record(cid, COLLECTION_LEVEL, "skipped", reason="stale_item_index", error=note),
            state,
        )
    if not items:
        # 20 of the 53 collections publish only a collection.json.
        _record_safely(
            ledger, Record(cid, COLLECTION_LEVEL, "skipped", reason="empty_collection"), state
        )
        _say(f"==> {cid}: no items")
        return
    if config.limit_per_collection is not None:
        items = items[: config.limit_per_collection]
    _say(f"==> {cid}: {len(items)} item(s)")
    convert_items(items, ledger=ledger, config=config, collection=cid, state=state)

    try:
        config_path = aggregate.write_config(
            aggregate.collection_config(collection), config.out / "_configs" / f"{cid}.yaml"
        )
        indexed = aggregate.update_collection(
            config.tool,
            config.out / cid / "items",
            config_path,
            config.out / cid / "collection.json",
        )
    except OSError as exc:
        # Writing the config and starting `city3dstac` are this machine's
        # business: an unwritable `_configs`, a full volume, a tool that was
        # never built. The items above have already converted and their
        # packages are on disk — publishing the collection as a conversion
        # failure on the strength of this would be a fabricated measurement.
        _environment_failure(ledger, cid, COLLECTION_LEVEL, f"aggregation: {_describe(exc)}", state)
    else:
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
            except OSError as exc:
                # Nothing local is evidence about the data. Everything remote
                # arrives as an `httpx` error, and every filesystem failure
                # inside the item loop is already handled there, so an
                # `OSError` reaching here is this machine: a full disk, a
                # read-only mirror, a missing `city3dstac`.
                _environment_failure(ledger, cid, COLLECTION_LEVEL, _describe(exc), state)
            # Deliberately broad: one collection must never stop the run.
            except Exception as exc:
                detail = _describe(exc)
                _warn(f"  ! {cid} failed: {detail}")
                _record_safely(
                    ledger,
                    Record(
                        cid,
                        COLLECTION_LEVEL,
                        "failed",
                        reason="convert_failed",
                        error=detail[:MAX_ERROR_CHARS],
                    ),
                    state,
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
        _warn(f"  ! catalogue root metadata unavailable ({_describe(exc)}); using defaults")
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
        detail = _describe(exc)
        _warn(f"  ! catalogue aggregation failed: {detail}")
        # Not a ledger record — there is no collection this belongs to — but
        # still an incomplete run, and the summary must say so rather than
        # imply the mirror has a usable root.
        state.note_environment(f"catalogue aggregation: {detail}")


def print_summary(ledger: Ledger, state: RunState) -> None:
    """Print what the run measured: outcomes, reasons, and degraded indexes.

    Environment failures are kept visibly apart from conversion outcomes
    throughout — their own column, out of the reasons histogram, and a block of
    their own at the end. A reader who cannot tell the two apart would read a
    run that hit a full disk as a catalogue half of which does not convert.

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
        _warn(f"  ! summary.csv could not be written: {_describe(exc)}")
    _say("\n--- summary ---")
    _say(f"{'collection':<32} {'converted':>9} {'failed':>7} {'skipped':>8} {'environment':>11}")
    totals = {"converted": 0, "failed": 0, "skipped": 0, ENVIRONMENT: 0}
    for collection in ledger.collections():
        counts = ledger.counts(collection)
        for status in totals:
            totals[status] += counts.get(status, 0)
        _say(
            f"{collection:<32} {counts.get('converted', 0):>9} "
            f"{counts.get('failed', 0):>7} {counts.get('skipped', 0):>8} "
            f"{counts.get(ENVIRONMENT, 0):>11}"
        )
    _say(
        f"{'TOTAL':<32} {totals['converted']:>9} {totals['failed']:>7} "
        f"{totals['skipped']:>8} {totals[ENVIRONMENT]:>11}"
    )

    histogram = ledger.histogram()
    _say("\nreasons (what the data did):")
    if histogram:
        for reason, count in sorted(histogram.items(), key=lambda kv: (-kv[1], kv[0])):
            _say(f"  {count:>7}  {reason}")
    else:
        _say("  (none)")

    # The in-process count is authoritative: it includes the failures the
    # ledger could not be told about, which are exactly the ones a broken
    # ledger would otherwise hide.
    environment_seen = max(state.environment_seen, totals[ENVIRONMENT])
    if environment_seen:
        message = (
            f"\n!! {environment_seen} environment failure(s): this machine, not the data.\n"
            "   They are excluded from the reasons above and say nothing about\n"
            "   whether these collections convert; this run is incomplete."
        )
        _say(message)
        # Also on stderr, so a run whose stdout is piped away still says it.
        _warn(message)
        for note in state.environment:
            _say(f"  - {note}")
        if environment_seen > len(state.environment):
            _say(f"  ... and {environment_seen - len(state.environment)} more")

    _say(f"\ncollections without a GeoParquet index: {len(state.no_index)}")
    for cid in state.no_index:
        _say(f"  - {cid}")
    if summary is not None:
        _say(f"\nledger: {summary.parent}")


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
    prepared, another run holds a directory this one needs, or the catalogue
    root is unreachable. Once items start being attempted the answer is 0,
    however many of them failed — and however badly the report itself fared.
    """
    try:
        config.out.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        # Nothing can be recorded, so there is nothing to run. Reported like the
        # other startup errors rather than as a bare traceback.
        _warn(f"cannot prepare the output directory {config.out}: {exc}")
        return 1
    state = RunState()

    with contextlib.ExitStack() as stack:
        # The output directory is claimed even for `--aggregate-only`, which
        # would otherwise index a package another run is halfway through
        # writing. Claimed before the ledger is opened, so a refused run adds
        # nothing to a directory that is not its own.
        try:
            stack.enter_context(locked(config.out, OUTPUT_DIRECTORY))
        except (LockBusy, OSError) as exc:
            _warn(f"cannot start: {exc}")
            return 1
        try:
            ledger = Ledger(config.out / "_reports")
        except OSError as exc:
            _warn(f"cannot prepare the output directory {config.out}: {exc}")
            return 1

        if not aggregate_only:
            cids = list(collections) if collections else None
            if cids is None:
                try:
                    cids = resolve_collections(config)
                # Deliberately broad: whatever the failure, nothing could be attempted.
                except Exception as exc:
                    _warn(f"catalogue root unreachable: {_describe(exc)}")
                    return 1
            # Claimed after the root is known to be reachable, so a run that
            # never starts leaves no empty working directory behind. Skipped
            # when the working directory *is* the output directory, which this
            # run already holds.
            try:
                if work_root(config).resolve() != config.out.resolve():
                    stack.enter_context(locked(work_root(config), WORKING_DIRECTORY))
            except (LockBusy, OSError) as exc:
                _warn(f"cannot start: {exc}")
                return 1
            sweep_work_root(config)
            run_collections(cids, ledger=ledger, config=config, state=state)

        aggregate_all(config, state)
        try:
            print_summary(ledger, state)
        # Deliberately broad: the run is over, and a report that cannot be
        # printed must not turn a completed measurement into a traceback. The
        # ledger on disk is the durable copy.
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
    """The process entry point: a run, and streams that cannot outlive it."""
    args = parse_args(argv)
    try:
        return run(
            config_from_args(args),
            args.collections,
            aggregate_only=args.aggregate_only,
        )
    finally:
        # Every return path, so the exit code is only ever this function's.
        _flush_streams()


if __name__ == "__main__":
    raise SystemExit(main())
