"""The one abstraction every system under test hides behind.

A system is anything that can ingest a CityJSON file, answer a
(scenario, params) pair with a count and a wall-clock, and report its own
size. Adding a further system later is one module plus one registry entry.
"""

from __future__ import annotations

from typing import Protocol, runtime_checkable

from citybench.config import Dataset, IngestResult, Measurement, Params, SizeReport


@runtime_checkable
class System(Protocol):
    tag: str

    def prepare(self) -> None:
        """Create schemas, start clients. Idempotent."""

    def ingest(self, dataset: Dataset) -> IngestResult:
        """Load ``dataset``, build indexes, ANALYZE. Returns wall-clock."""

    def run(self, scenario: str, params: Params, repeat: int,
            selectivity: float | None = None) -> Measurement:
        """Run one scenario ``repeat`` times after one discarded warm-up.

        ``selectivity`` is the window-area target for bbox-query and None
        for every other scenario. Every adapter takes this signature —
        the runner calls it uniformly.
        """

    def size(self) -> SizeReport:
        """On-disk size, with and without indexes where meaningful."""

    def teardown(self) -> None:
        """Release resources. Must not raise."""


REGISTRY: dict[str, type] = {}


def register(cls):
    """Class decorator adding a system to the registry under its tag."""
    REGISTRY[cls.tag] = cls
    return cls
