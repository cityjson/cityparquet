"""Drive the native Rust reader as a subprocess.

`cityparquet-readbench --child` accepts explicit query parameters, so the
native reader can be given byte-identical values to every other system
without reimplementing any measurement logic and without editing the
submodule.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

from citybench.config import Dataset, IngestResult, Measurement, Params, SizeReport
from citybench.scenarios.registry import TIER1, ScenarioUnavailable

_SELECTIVITY_TAGS = {0.01: "bbox-1pct", 0.05: "bbox-5pct", 0.25: "bbox-25pct"}


def parse_child_stdout(stdout: str) -> tuple[int, float, int, int]:
    """Parse one `--child` invocation's stdout line.

    Confirmed empirically against the built `cityparquet-readbench` binary
    (benchmark/readbench/src/main.rs, whose own
    `readbench_smoke.rs` test asserts the same shape): the child prints
    exactly ONE line of SPACE-SEPARATED fields, never JSON:

        local transport:  "<time_s> <peak_heap_bytes> <ru_maxrss_bytes> <result_count>"
        http transport:   the same, plus " <bytes> <requests>"

    This harness only ever drives local transport, so four fields are
    expected in practice; a six-field line is also accepted, with its
    trailing two (http-only) fields ignored rather than treated as an
    error — so this parser stays correct even if a future task starts
    driving the http transport too.

    Returns ``(result_count, time_s, peak_heap_bytes, peak_rss_bytes)``.

    Note the child reports its OWN elapsed time. Using it rather than
    timing the subprocess keeps process startup and Python's own overhead
    out of a number that is compared against in-process engines, which
    never pay that cost.
    """
    line = stdout.strip().splitlines()[-1]
    fields = line.split()
    if len(fields) not in (4, 6):
        raise ValueError(
            f"unexpected child stdout (want 4 or 6 fields, got {len(fields)}): {line!r}"
        )
    time_s, peak_heap, peak_rss, result_count = fields[:4]
    return int(result_count), float(time_s), int(peak_heap), int(peak_rss)


def build_child_args(scenario: str, params: Params, input_path: str,
                      selectivity: float | None = None,
                      fmt: str = "cityparquet") -> list[str]:
    """The argv for one `--child` invocation.

    The flag-per-scenario mapping below is read from
    `benchmark/readbench/src/formats/cityparquet.rs`'s own
    `Scenario` match, not guessed. In particular: `attr-filter` and
    `project` both take a `--attr-column` flag, but it must carry
    DIFFERENT `Params` fields for each. `attr-filter` needs the
    categorical column its `--attr-eq` compares against
    (`params.attr_column`). `project` also projects a categorical column —
    `sql_duckdb.sql_for`'s own `project` branch hard-codes `object_type`,
    the same value `params.attr_column` holds for every dataset — so
    `project` uses `params.attr_column` too. Only `attr-stats` reads
    `params.numeric_column`. Grouping `project` with `attr-stats` (an
    earlier draft of this function did) would silently point this
    system's `project` scenario at a different column than every SQL
    system's `project`, defeating the whole point of a byte-identical
    cross-system parameter set.
    """
    if scenario not in TIER1:
        raise ValueError(
            f"{scenario!r} is not implemented by the readbench child; "
            "tier-2 scenarios are SQL-only"
        )

    args = [
        "--child",
        "--format", fmt,
        "--scenario", scenario,
        "--input", input_path,
    ]

    if scenario == "bbox-query":
        if selectivity is None:
            raise ValueError("bbox-query requires a selectivity")
        win = params.bbox_full.window(selectivity)
        args += ["--bbox", ",".join(str(v) for v in win.as_cli_list())]
        args += ["--selectivity-tag", _SELECTIVITY_TAGS[selectivity]]
    elif scenario == "attr-filter":
        args += ["--attr-column", params.attr_column, "--attr-eq", params.attr_eq]
    elif scenario == "attr-stats":
        # Mirrors every sql_*.sql_for's own `parent_id is None` guard for
        # `hierarchy`: a dataset with no numeric attribute at all (see
        # sql_duckdb.py's equivalent guard) is a legitimate dataset
        # property, not a query bug — raised here rather than letting
        # `None` get stringified into the child process's `--attr-column`
        # argument, which would silently ask the Rust child to aggregate a
        # column literally named "None".
        if params.numeric_column is None:
            raise ScenarioUnavailable("dataset has no numeric attribute")
        args += ["--attr-column", params.numeric_column]
    elif scenario == "project":
        args += ["--attr-column", params.attr_column]
    elif scenario == "id-lookup":
        args += ["--target-id", params.target_id]

    return args


class ReadbenchSystem:
    """Deliberately NOT decorated with @register.

    The registry keys on a class attribute, but this class serves two tags
    depending on its ``hilbert`` argument, so registering it would silently
    bind only one of them. The CLI constructs both instances explicitly.
    """

    def __init__(self, *, binary: Path, hilbert: bool = False) -> None:
        self._binary = binary
        self._hilbert = hilbert
        self.tag = "cityparquet-hilbert" if hilbert else "cityparquet"
        self._package: Path | None = None

    def prepare(self) -> None:
        if not self._binary.exists():
            raise FileNotFoundError(
                f"{self._binary} not found; build it with "
                "`cargo build --release -p cityparquet-readbench` in lib/cityparquet-rs"
            )

    def ingest(self, dataset: Dataset) -> IngestResult:
        """No load step; the package was written by `cityparquet convert`."""
        self._package = dataset.hilbert_dir if self._hilbert else dataset.cityparquet_dir
        return IngestResult(wall_clock_s=0.0, notes="no load step")

    def run(self, scenario: str, params: Params, repeat: int,
            selectivity: float | None = None) -> Measurement:
        assert self._package is not None
        args = build_child_args(
            scenario, params, str(self._package), selectivity,
            # self.tag is exactly "cityparquet" / "cityparquet-hilbert" — the
            # two `--format` values `formats::resolve` accepts for this
            # runner (both dispatch to the same CityParquetRunner; passing
            # the honest one keeps this invocation self-documenting even
            # though it makes no behavioural difference today).
            fmt=self.tag,
        )

        def once() -> tuple[int, float, int, int]:
            proc = subprocess.run(
                [str(self._binary), *args], check=True, capture_output=True, text=True
            )
            return parse_child_stdout(proc.stdout)

        once()  # discarded warm-up
        samples = [once() for _ in range(repeat)]
        return Measurement(
            result_count=samples[0][0],
            # The child's OWN reported time, not our subprocess wall-clock:
            # timing the subprocess would fold in process startup and
            # Python's own overhead, which the other systems never pay.
            times_s=[s[1] for s in samples],
            server_times_s=[],
            peak_heap_bytes=max(s[2] for s in samples),
            peak_rss_bytes=max(s[3] for s in samples),
        )

    def size(self) -> SizeReport:
        assert self._package is not None
        total = sum(f.stat().st_size for f in self._package.rglob("*") if f.is_file())
        return SizeReport(size_bytes=total, size_bytes_no_index=total)

    def teardown(self) -> None:
        return None
