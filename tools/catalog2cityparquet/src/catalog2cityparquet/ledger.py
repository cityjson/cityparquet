"""Per-item outcome records for a catalogue conversion run.

The run is a conformance measurement as much as a conversion: a failure is
data, not an abort. Every item lands here with a reason drawn from a closed
vocabulary, so the end-of-run histogram is comparable between runs.

The vocabulary makes one distinction above all others: **"this dataset could
not be converted" is not "this machine could not complete the run"**. Every
conformance reason is a statement about the data and belongs in the histogram
the paper quotes; :data:`ENVIRONMENT` is a statement about the host — a full
disk, an unwritable directory, a missing tool, a broken stream — and belongs
nowhere near it. Without that distinction every local failure has to be
recorded as a conversion failure, which is how a run with a full log volume
comes to publish half a catalogue as unconvertible.
"""

from __future__ import annotations

import csv
import json
import os
import re
import threading
import time
import uuid
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field, replace
from pathlib import Path
from typing import Literal

#: The one outcome that is not an outcome of the *data*: the run hit a local
#: failure here and this record says nothing about whether the dataset is
#: convertible. It is both a status and a reason, and the two always travel
#: together (see :meth:`Ledger.record`) — the status keeps it out of the
#: `failed` column, the reason keeps it out of the histogram.
ENVIRONMENT = "environment"

Status = Literal["converted", "failed", "skipped", "environment"]

#: What the *data* did. These, and only these, make the conformance histogram,
#: which is the number this project publishes.
CONFORMANCE_REASONS = frozenset(
    {
        "download_failed",
        "unsupported_archive",
        "unsupported_citygml_version",
        "unsupported_cityjson_version",
        "no_crs",
        "geographic_crs",
        "convert_failed",
        "empty_collection",
        "duplicate_bundle",
        "stale_item_index",
    }
)

#: Closed set. A reason outside it is a programming error, not a new category —
#: silently admitting typos would make the histogram meaningless.
REASONS = CONFORMANCE_REASONS | {ENVIRONMENT}

#: Substrings (lower-cased) of a subprocess failure that is *this machine's*,
#: not the data's. Every tool this driver runs — the `cityparquet` converter
#: and the `city3dstac` aggregator alike — exits non-zero either way, so its
#: stderr is the only thing that tells the two apart. The list lives here, with
#: the vocabulary it decides between, because two copies of it would drift and
#: a drifted copy silently republishes a full disk as unconvertible data.
#: Deliberately short, matching the kernel's own wording as it reaches a Rust
#: `std::io::Error`, so it does not depend on either tool's phrasing.
HOST_FAILURE_MARKERS = (
    "no space left",
    "read-only file system",
    "disk quota exceeded",
    "too many open files",
)


class HostFailure(RuntimeError):
    """A tool ran and failed because this *machine* could not do the work.

    A `RuntimeError` like any other tool failure — every existing caller keeps
    working — but a distinct type, so the orchestrator can route it to the
    environment path instead of the conformance histogram. What it says is
    "nothing was learned about this dataset", never "this dataset does not
    convert".
    """


def is_host_failure(stderr: str) -> bool:
    """Whether a tool's stderr names a failure of the host rather than the data."""
    text = stderr.lower()
    return any(marker in text for marker in HOST_FAILURE_MARKERS)


#: Collection ids are slugs such as ``japan-plateau-3d``. They come from a
#: published catalogue, not from us, and are interpolated into a ledger
#: filename, so the pattern is deliberately narrow: no separators, no dots, and
#: therefore no way to write outside the reports directory.
COLLECTION_ID_PATTERN = re.compile(r"[A-Za-z0-9]+(?:[._-]?[A-Za-z0-9]+)*")


def new_run_id() -> str:
    """An identifier for one process's records in the cumulative ledger.

    Readable rather than opaque — the time and pid are what an operator has to
    hand when matching a run to a log — with a random tail so two runs started
    in the same second on different hosts cannot collide.
    """
    return f"{int(time.time())}-{os.getpid()}-{uuid.uuid4().hex[:8]}"


def validate_collection_id(cid: str) -> str:
    """Return `cid` unchanged, or raise if it is not a conservative slug."""
    if not isinstance(cid, str) or not COLLECTION_ID_PATTERN.fullmatch(cid):
        raise ValueError(
            f"invalid collection id {cid!r}: expected a slug such as 'japan-plateau-3d'"
        )
    return cid


@dataclass(frozen=True)
class Record:
    """The outcome of one catalogue item.

    ``bytes`` deliberately shadows the builtin inside this namespace: it is the
    field name the ledger's readers and the JSONL schema are written against.

    ``run_id`` and ``timestamp`` are stamped by :meth:`Ledger.record`, so no
    caller has to remember them. They exist because the JSONL accumulates
    across runs and ``--skip-existing`` skips only *successes*: a previously
    failed item is re-attempted and appends a SECOND record. Without a way to
    tell the attempts apart, any roll-up of the cumulative file double-counts
    that item — see :func:`roll_up`, which is the roll-up this project
    publishes from.
    """

    collection: str
    item_id: str
    status: Status
    reason: str | None = None
    error: str | None = None
    bytes: int = 0
    seconds: float = 0.0
    run_id: str = ""
    timestamp: float = 0.0


@dataclass
class Ledger:
    """Append-only JSONL per collection, plus a rolled-up summary.

    Items are converted through a thread pool, so :meth:`record` is guarded by
    a lock: one record is one intact line, and the tallies stay consistent.
    """

    reports_dir: Path
    #: Identifies this process's records in the cumulative JSONL. Generated per
    #: `Ledger`, so a resumed run's second record for an item is
    #: distinguishable from the first.
    run_id: str = field(default_factory=lambda: new_run_id())
    _counts: dict[str, Counter] = field(default_factory=lambda: defaultdict(Counter))
    _reasons: Counter = field(default_factory=Counter)
    _discovered: Counter = field(default_factory=Counter)
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False, compare=False)

    def __post_init__(self) -> None:
        self.reports_dir.mkdir(parents=True, exist_ok=True)

    def note_discovered(self, collection: str, count: int) -> None:
        """Record how many items enumeration found for `collection`.

        Reported beside the outcomes because they are not the same number: a
        collection whose enumeration was truncated — by a limit, or by item
        documents the origin would not serve — would otherwise look complete.
        """
        with self._lock:
            self._discovered[collection] = count

    def record(self, rec: Record) -> None:
        """Append one outcome, rejecting any reason outside the closed set.

        The collection id is validated here too, because this is where it
        becomes a path.
        """
        if rec.reason is not None and rec.reason not in REASONS:
            raise ValueError(f"unknown reason {rec.reason!r}; expected one of {sorted(REASONS)}")
        if (rec.reason == ENVIRONMENT) != (rec.status == ENVIRONMENT):
            # Two columns, one concept. Letting them drift would put an
            # environment failure back in the `failed` column, which is the
            # misreport the distinction exists to prevent.
            raise ValueError(
                f"the {ENVIRONMENT!r} status and reason travel together: "
                f"got status {rec.status!r} with reason {rec.reason!r}"
            )
        validate_collection_id(rec.collection)
        # Stamped here rather than at 20 call sites: a record with no run
        # identity is one the cumulative roll-up cannot place.
        rec = replace(rec, run_id=self.run_id, timestamp=time.time())
        line = json.dumps(asdict(rec), ensure_ascii=False) + "\n"
        path = self.reports_dir / f"{rec.collection}.jsonl"
        with self._lock:
            with path.open("a", encoding="utf-8") as fh:
                fh.write(line)
            self._counts[rec.collection][rec.status] += 1
            if rec.reason is not None and rec.reason != ENVIRONMENT:
                # Environment failures are excluded here rather than filtered
                # by every reader: the histogram is the published artefact, and
                # it must be clean by construction.
                self._reasons[rec.reason] += 1

    def collections(self) -> list[str]:
        """Every collection with at least one record, in id order."""
        with self._lock:
            return sorted(self._counts)

    def counts(self, collection: str) -> dict[str, int]:
        """Status tallies for one collection, e.g. ``{"converted": 3}``."""
        with self._lock:
            return dict(self._counts.get(collection, Counter()))

    def histogram(self) -> dict[str, int]:
        """Conformance reason tallies across every collection seen so far.

        This is the conformance histogram: what the *data* did. Environment
        failures never appear in it, whatever the run went through.
        """
        with self._lock:
            return dict(self._reasons)

    def environment_failures(self) -> dict[str, int]:
        """Per-collection tally of the times this *machine* was what failed.

        Reported separately so a run that hit local trouble cannot be mistaken
        for one that measured unconvertible data.
        """
        with self._lock:
            return {
                collection: counter[ENVIRONMENT]
                for collection, counter in self._counts.items()
                if counter[ENVIRONMENT]
            }

    def discovered(self, collection: str) -> int:
        """How many items enumeration found for `collection` (0 if unrecorded)."""
        with self._lock:
            return self._discovered.get(collection, 0)

    def write_summary(self) -> Path:
        """Overwrite ``summary.csv`` with the current per-collection roll-up."""
        with self._lock:
            snapshot = {collection: Counter(c) for collection, c in self._counts.items()}
            discovered = Counter(self._discovered)
        path = self.reports_dir / "summary.csv"
        with path.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.writer(fh)
            writer.writerow(
                ["collection", "discovered", "converted", "failed", "skipped", ENVIRONMENT]
            )
            for collection in sorted(snapshot):
                c = snapshot[collection]
                writer.writerow(
                    [
                        collection,
                        discovered.get(collection, 0),
                        c["converted"],
                        c["failed"],
                        c["skipped"],
                        c[ENVIRONMENT],
                    ]
                )
        return path


@dataclass(frozen=True)
class Rollup:
    """What the cumulative ledger says, once every item is counted exactly once."""

    #: Items counted, i.e. distinct `(collection, item_id)` pairs.
    items: int
    #: Status tallies across every collection.
    statuses: Counter
    #: The conformance histogram — the number this project publishes.
    reasons: Counter
    #: Times this machine, not the data, was what failed.
    environment: int
    #: Per-collection status tallies.
    per_collection: dict[str, Counter]
    #: Lines that could not be parsed — a run that died mid-write leaves one.
    #: Counted rather than skipped: silently dropping them would shrink the
    #: denominator exactly as the bugs this roll-up exists to survive did.
    unreadable: int


def roll_up(reports_dir: Path) -> Rollup:
    """Reduce the cumulative JSONL to one outcome per item, then tally it.

    The JSONL is append-only and accumulates across runs, and a resumed run
    re-attempts a previously failed item — so the same item legitimately
    appears twice with two different outcomes. **The last record wins**: the
    file is append-only, so the last line for an item is that item's current
    state. Anything else double-counts, which is why this exists as reviewed
    code rather than as an instruction to roll the files up by hand.
    """
    latest: dict[tuple[str, str], dict] = {}
    unreadable = 0
    for path in sorted(Path(reports_dir).glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except ValueError:
                unreadable += 1
                continue
            if not isinstance(rec, dict) or "collection" not in rec or "item_id" not in rec:
                unreadable += 1
                continue
            latest[(str(rec["collection"]), str(rec["item_id"]))] = rec

    statuses: Counter = Counter()
    reasons: Counter = Counter()
    per_collection: dict[str, Counter] = defaultdict(Counter)
    environment = 0
    for (collection, _item), rec in latest.items():
        status = str(rec.get("status", ""))
        statuses[status] += 1
        per_collection[collection][status] += 1
        reason = rec.get("reason")
        if reason == ENVIRONMENT:
            environment += 1
        elif reason is not None:
            # The same exclusion `Ledger.record` makes: the histogram is the
            # published artefact and must be clean by construction.
            reasons[str(reason)] += 1
    return Rollup(
        items=len(latest),
        statuses=statuses,
        reasons=reasons,
        environment=environment,
        per_collection=dict(per_collection),
        unreadable=unreadable,
    )
