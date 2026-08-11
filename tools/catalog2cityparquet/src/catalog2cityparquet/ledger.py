"""Per-item outcome records for a catalogue conversion run.

The run is a conformance measurement as much as a conversion: a failure is
data, not an abort. Every item lands here with a reason drawn from a closed
vocabulary, so the end-of-run histogram is comparable between runs.
"""

from __future__ import annotations

import csv
import json
import re
import threading
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Literal

Status = Literal["converted", "failed", "skipped"]

#: Closed set. A reason outside it is a programming error, not a new category —
#: silently admitting typos would make the histogram meaningless.
REASONS = frozenset(
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


#: Collection ids are slugs such as ``japan-plateau-3d``. They come from a
#: published catalogue, not from us, and are interpolated into a ledger
#: filename, so the pattern is deliberately narrow: no separators, no dots, and
#: therefore no way to write outside the reports directory.
COLLECTION_ID_PATTERN = re.compile(r"[A-Za-z0-9]+(?:[._-]?[A-Za-z0-9]+)*")


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
    """

    collection: str
    item_id: str
    status: Status
    reason: str | None = None
    error: str | None = None
    bytes: int = 0
    seconds: float = 0.0


@dataclass
class Ledger:
    """Append-only JSONL per collection, plus a rolled-up summary.

    Items are converted through a thread pool, so :meth:`record` is guarded by
    a lock: one record is one intact line, and the tallies stay consistent.
    """

    reports_dir: Path
    _counts: dict[str, Counter] = field(default_factory=lambda: defaultdict(Counter))
    _reasons: Counter = field(default_factory=Counter)
    _lock: threading.Lock = field(default_factory=threading.Lock, repr=False, compare=False)

    def __post_init__(self) -> None:
        self.reports_dir.mkdir(parents=True, exist_ok=True)

    def record(self, rec: Record) -> None:
        """Append one outcome, rejecting any reason outside the closed set.

        The collection id is validated here too, because this is where it
        becomes a path.
        """
        if rec.reason is not None and rec.reason not in REASONS:
            raise ValueError(f"unknown reason {rec.reason!r}; expected one of {sorted(REASONS)}")
        validate_collection_id(rec.collection)
        line = json.dumps(asdict(rec), ensure_ascii=False) + "\n"
        path = self.reports_dir / f"{rec.collection}.jsonl"
        with self._lock:
            with path.open("a", encoding="utf-8") as fh:
                fh.write(line)
            self._counts[rec.collection][rec.status] += 1
            if rec.reason is not None:
                self._reasons[rec.reason] += 1

    def counts(self, collection: str) -> dict[str, int]:
        """Status tallies for one collection, e.g. ``{"converted": 3}``."""
        with self._lock:
            return dict(self._counts.get(collection, Counter()))

    def histogram(self) -> dict[str, int]:
        """Reason tallies across every collection seen so far."""
        with self._lock:
            return dict(self._reasons)

    def write_summary(self) -> Path:
        """Overwrite ``summary.csv`` with the current per-collection roll-up."""
        with self._lock:
            snapshot = {collection: Counter(c) for collection, c in self._counts.items()}
        path = self.reports_dir / "summary.csv"
        with path.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.writer(fh)
            writer.writerow(["collection", "converted", "failed", "skipped"])
            for collection in sorted(snapshot):
                c = snapshot[collection]
                writer.writerow([collection, c["converted"], c["failed"], c["skipped"]])
        return path
