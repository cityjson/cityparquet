"""CSVs -> bench_data.json.

Reads the benchmark result artefacts under ``bench/`` — the CSVs a finished
``just bench`` / ``just compression-bench`` / ``just sizes`` run leaves behind,
never a benchmark of its own — and emits the ``bench_data.json`` data contract
described in ``benchviz/DESIGN.md``.

Every path this module touches is derived from one ``Inputs.bench_dir``, so the
same code serves the in-repo default (``bench/``) and an out-of-tree caller
that points ``--bench-dir`` at a checkout elsewhere.

stdlib only -- no third-party imports here, on purpose: this step must be
runnable from a bare Python with nothing installed.
"""

from __future__ import annotations

import csv
import json
import re
from dataclasses import dataclass
from pathlib import Path

from .paths import DEFAULT_BENCH_DIR, DEFAULT_DATA_PATH

SIZES_CSV_NAME = "sizes.csv"


@dataclass(frozen=True)
class Inputs:
    """The benchmark artefacts to read, all derived from one directory."""

    bench_dir: Path = DEFAULT_BENCH_DIR

    @property
    def read_dir(self) -> Path:
        return self.bench_dir / "read_results"

    @property
    def compression_dir(self) -> Path:
        return self.bench_dir / "compression_results"

    @property
    def ordering_dir(self) -> Path:
        return self.bench_dir / "ordering_results"

    @property
    def sizes_csv(self) -> Path:
        return self.read_dir / SIZES_CSV_NAME

    @property
    def read_benchmark_md(self) -> Path:
        return self.bench_dir / "READ_BENCHMARK.md"

    @property
    def bench_readme_md(self) -> Path:
        return self.bench_dir / "README.md"

    def label(self, path: Path) -> str:
        """A repo-qualified label for a source path, e.g.

        ``cityparquet-rs/bench/read_results``.

        The page names the artefacts it reports, so the label has to stay the
        same whether the renderer ran from inside this repository or from a
        parent workspace holding it as a submodule. Qualifying with the
        checkout's own directory name does that without knowing either.
        """
        root = self.bench_dir.parent
        try:
            return str(Path(root.name) / path.relative_to(root))
        except ValueError:  # a --bench-dir outside its own parent: bare path
            return str(path)


BASELINE_FORMAT = "cityjsonseq"
CITATION_FLOOR_S = 0.010

# The ROW-ORDERING axis, mirroring `Format::ORDERING_SET`
# (crates/cityparquet-readbench/src/format.rs). Both members are the same
# writer, reader and scenarios; the only difference is the order rows were
# written in, which is why its baseline is the source-order package and NOT
# `BASELINE_FORMAT` -- an ordering run has no CityJSONSeq row to divide by.
ORDERING_BASELINE = "cityparquet"
ORDERING_VARIANT = "cityparquet-hilbert"

KNOWN_FORMATS = (
    "citygml",
    "cityjson",
    "cityjsonseq",
    "cityjsonseq-gz",
    "cityparquet",
    "cityparquet-hilbert",
    "duckdb-parquet",
    "flatcitybuf",
)

# The FORMAT-COMPARISON axis, mirroring `Format::DEFAULT_SET`
# (crates/cityparquet-readbench/src/format.rs): one tag per format family, with
# CityParquet represented by the Hilbert-ordered package — the configuration
# that would actually ship, so the comparison is not handicapped by an ordering
# choice no other format faces. Ordering is its own question, asked by
# `Format::ORDERING_SET` over bench/ordering_results.
#
# `cityjsonseq-gz` and `duckdb-parquet` are deliberately absent: the first is a
# compression variant of a format already on the axis, the second an SQL-engine
# baseline. Neither is a format, so neither belongs on a format axis — a panel
# putting gzipped CityJSONSeq beside CityJSONSeq compares a codec, not a format.
# Rows for them still reach `bench_data.json` when a run opts in; the views omit
# them and say so.
FORMAT_AXIS = (
    "cityparquet-hilbert",
    "citygml",
    "cityjson",
    "cityjsonseq",
    "flatcitybuf",
)

# Counting grain, from READ_BENCHMARK.md fairness caveat 1's own table. Only
# `count`/`full-read`/`bbox-*` split this way; the other scenarios are
# CityObject-granular in every format.
OBJECT_GRAIN_FORMATS = (
    "cityparquet",
    "cityparquet-hilbert",
    "cityjson",
    "duckdb-parquet",
)
FEATURE_GRAIN_FORMATS = (
    "citygml",
    "cityjsonseq",
    "cityjsonseq-gz",
    "flatcitybuf",
)

# Scenarios whose counting grain differs between feature-grain and
# CityObject-grain formats (READ_BENCHMARK.md fairness caveat 1).
GRAIN_INCOMPARABLE_SCENARIOS = frozenset(
    {"full-read", "count", "bbox-1pct", "bbox-5pct", "bbox-25pct"}
)

READ_COLUMNS = [
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    "time_s",
    "time_mad_s",
    "peak_heap_bytes",
    "peak_rss_bytes",
    "repeat",
    "notes",
]
SIZES_COLUMNS = ["dataset", "format", "bytes", "mb", "ratio_vs_cityjsonseq"]
COMPRESSION_COLUMNS = [
    "dataset",
    "variant",
    "object_count",
    "write_s",
    "total_bytes",
    "cityobjects_bytes",
    "sidecar_bytes",
    "full_scan_s",
    "window_query_s",
    "row_groups_total",
    "row_groups_touched",
    "roundtrip_equal",
]

BBOX_NOTE_RE = re.compile(r"^bbox-\d+pct$")
COLD_RE = re.compile(r"\bcold\b", re.IGNORECASE)

CODEC_LEVEL_NOTE = (
    "Codec levels are mismatched across the compression variants: zstd is "
    "written at level 3, gzip at level 6 and brotli at level 1. These are the "
    "parquet-rs defaults carried by the writer recipe "
    "(crates/cityparquet/src/recipe.rs), as bench/README.md states, so the "
    "codec comparison is a comparison of implementation defaults, not of codecs "
    "at equal effort. \"Smallest codec\" is therefore not a citable claim from "
    "this benchmark."
)


class PrepError(RuntimeError):
    """Raised when an input artefact does not match the expected contract."""


class ExcludedFormats:
    """Rows dropped because no view here has a vocabulary for their format.

    ``KNOWN_FORMATS`` is a *presentation* vocabulary — a colour, a marker shape
    and a caption exist for each of its members — and the corpus grows formats
    faster than the views do (a CityGML-native column arrived with the CityGML
    reader). Dropping such rows is the honest option: they cannot be drawn.
    Dropping them *silently* is not, since the page is a format comparison and
    a reader cannot tell a format that lost from one that was never plotted.
    So every drop is tallied here, lands in ``meta.excluded_formats``, and is
    stated in the page's own coverage notes.
    """

    def __init__(self) -> None:
        self._rows: dict[str, int] = {}
        self._where: dict[str, set[str]] = {}

    def record(self, fmt: str, where: str) -> None:
        self._rows[fmt] = self._rows.get(fmt, 0) + 1
        self._where.setdefault(fmt, set()).add(where)

    def as_list(self) -> list[dict]:
        return [
            {
                "format": fmt,
                "rows": self._rows[fmt],
                "where": sorted(self._where[fmt]),
            }
            for fmt in sorted(self._rows)
        ]

    def notes(self) -> list[str]:
        return [
            f"excluded {e['rows']} {'/'.join(e['where'])} row(s) of format "
            f"{e['format']!r}: no view here can draw it"
            for e in self.as_list()
        ]


# --------------------------------------------------------------------------
# small helpers
# --------------------------------------------------------------------------


def _check_columns(path: Path, got: list[str] | None, want: list[str]) -> list[str]:
    """Require ``want`` as a leading prefix; return the appended extras.

    The benchmark harness APPENDS columns as it grows — `bytes_read` and
    `http_requests` arrived on the read CSVs with the HTTP transport, and
    sizes.csv grew `baseline_format`/`ratio_vs_baseline` — and every appended
    column used to break this reader outright. What must still fail is a
    column that moved, was renamed or disappeared: then the columns this code
    reads by name no longer hold what it believes, and a chart built from them
    would be wrong rather than merely incomplete.
    """
    columns = list(got or [])
    if columns[: len(want)] != want:
        raise PrepError(
            f"{path}: unexpected columns.\n  expected (prefix): {want}\n"
            f"  found:             {columns}"
        )
    return columns[len(want) :]


def _read_rows(path: Path, want: list[str]) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as fh:
        reader = csv.DictReader(fh)
        _check_columns(path, reader.fieldnames, want)
        return [dict(row) for row in reader]


def _float(value: str | None) -> float | None:
    if value is None or value.strip() == "":
        return None
    return float(value)


def _int(value: str | None) -> int | None:
    if value is None or value.strip() == "":
        return None
    return int(value)


def _ratio(value: float | None, base: float | None) -> float | None:
    if value is None or base is None or base == 0:
        return None
    return value / base


def _bool(value: str) -> bool:
    v = value.strip().lower()
    if v in ("true", "1", "yes"):
        return True
    if v in ("false", "0", "no", ""):
        return False
    raise PrepError(f"unparseable boolean {value!r}")


def _dataset_csvs(directory: Path) -> list[Path]:
    """Dataset CSVs in a results directory, keyed by filename stem.

    The ``dataset`` column inside the CSVs is inconsistent (a source filename
    for most runners, a short name for the duckdb runner), so the filename
    stem is the authoritative dataset id.
    """
    return sorted(
        p for p in directory.glob("*.csv") if p.name != SIZES_CSV_NAME
    )


# --------------------------------------------------------------------------
# verbatim caveat extraction (heading-driven, never line numbers)
# --------------------------------------------------------------------------


def _extract_section(path: Path, heading_prefix: str) -> str:
    """Return the body under the first ``## <heading_prefix>...`` heading."""
    lines = path.read_text(encoding="utf-8").splitlines()
    start = None
    for i, line in enumerate(lines):
        if line.startswith("## ") and line[3:].startswith(heading_prefix):
            start = i + 1
            break
    if start is None:
        raise PrepError(
            f"{path}: could not find a '## {heading_prefix}...' heading"
        )
    end = len(lines)
    for i in range(start, len(lines)):
        if lines[i].startswith("## "):
            end = i
            break
    return "\n".join(lines[start:end]).strip("\n")


def _split_numbered_items(body: str) -> list[str]:
    """Split a markdown ordered list into its items, verbatim."""
    item_start = re.compile(r"^(\d+)\.\s")
    items: list[list[str]] = []
    current: list[str] | None = None
    for line in body.splitlines():
        if item_start.match(line):
            if current is not None:
                items.append(current)
            current = [line]
        elif current is not None:
            current.append(line)
    if current is not None:
        items.append(current)
    return [("\n".join(item)).strip() for item in items]


def _dedent_item(text: str) -> str:
    """Drop the ``N. `` marker and the hanging indent of continuation lines."""
    lines = text.splitlines()
    head = re.sub(r"^\d+\.\s+", "", lines[0])
    indents = [
        len(ln) - len(ln.lstrip(" ")) for ln in lines[1:] if ln.strip()
    ]
    strip = min(indents) if indents else 0
    tail = [ln[strip:] if ln.strip() else "" for ln in lines[1:]]
    return "\n".join([head, *tail]).strip()


def read_caveats(inputs: Inputs) -> list[str]:
    """The fairness caveats, verbatim and in the source's own numbering.

    The count is NOT pinned — the list grew from 11 to 18 as the harness gained
    the CityGML and CityJSON readers, and a hard-coded expectation would only
    stop the page from quoting caveats that exist. What is checked instead is
    that the extraction actually produced the source's list: numbering starting
    at 1 and running without a gap, since the views deep-link to caveats by
    number (the "†" grain marker points at caveat 1, the noise floor at 8, the
    id-lookup bias at 9) and a misaligned list would footnote the wrong text.
    """
    path = inputs.read_benchmark_md
    body = _extract_section(path, "Fairness caveats")
    raw = _split_numbered_items(body)
    numbers = [int(re.match(r"^(\d+)\.", item).group(1)) for item in raw]
    if numbers != list(range(1, len(raw) + 1)):
        raise PrepError(
            f"{path}: fairness caveats are not numbered 1..n without a gap: {numbers}"
        )
    items = [_dedent_item(item) for item in raw]
    if not items:
        raise PrepError(f"{path}: no fairness caveats found")
    if any(not item for item in items):
        raise PrepError(f"{path}: an extracted caveat is empty")
    return items


def compression_caveats(inputs: Inputs) -> list[str]:
    """The write-side baseline's coverage caveat, verbatim — if it still exists.

    Quoted from `bench/README.md` so the page cannot drift from the methodology
    it reports. Returned empty rather than raised when the section is absent:
    the caveat is about the DuckDB baseline of the *write* benchmark, and a run
    with no compression data has no view to attach it to.
    """
    path = inputs.bench_readme_md
    try:
        body = _extract_section(path, "Baseline geometry coverage")
    except PrepError:
        return []
    return [body] if body.strip() else []


# --------------------------------------------------------------------------
# read benchmark
# --------------------------------------------------------------------------


def _scenario_key(row: dict[str, str]) -> str:
    scenario = row["scenario"]
    notes = row["notes"].strip()
    if scenario == "bbox-query":
        if not BBOX_NOTE_RE.match(notes):
            raise PrepError(
                f"bbox-query row with unrecognised notes tag {notes!r} "
                f"(dataset={row['dataset']}, format={row['format']})"
            )
        return notes
    return scenario


def load_read(
    inputs: Inputs, excluded: ExcludedFormats
) -> tuple[list[dict], list[str]]:
    """Return (read records, anomaly notes)."""
    anomalies: list[str] = []
    cold_rows = 0
    per_dataset: dict[str, list[dict[str, str]]] = {}

    for path in _dataset_csvs(inputs.read_dir):
        dataset = path.stem
        rows = _read_rows(path, READ_COLUMNS)
        kept = []
        for row in rows:
            if COLD_RE.search(row["notes"]):
                cold_rows += 1
                continue
            if row["format"] not in KNOWN_FORMATS:
                excluded.record(row["format"], "read")
                continue
            kept.append(row)
        per_dataset[dataset] = kept

    if cold_rows:
        anomalies.append(
            f"excluded {cold_rows} cold-tagged read row(s) (warm-only policy)"
        )

    records: list[dict] = []
    for dataset, rows in per_dataset.items():
        groups: dict[str, dict[str, dict[str, str]]] = {}
        for row in rows:
            key = _scenario_key(row)
            bucket = groups.setdefault(key, {})
            if row["format"] in bucket:
                raise PrepError(
                    f"{dataset}: duplicate row for "
                    f"({key}, {row['format']}) -- cannot pick a baseline"
                )
            bucket[row["format"]] = row

        for key in sorted(groups):
            bucket = groups[key]
            base = bucket.get(BASELINE_FORMAT)
            base_time = _float(base["time_s"]) if base else None
            base_heap = _float(base["peak_heap_bytes"]) if base else None
            base_rss = _float(base["peak_rss_bytes"]) if base else None

            for fmt in KNOWN_FORMATS:
                row = bucket.get(fmt)
                if row is None:
                    continue
                time_s = _float(row["time_s"])
                heap_b = _int(row["peak_heap_bytes"])
                rss_b = _int(row["peak_rss_bytes"])

                if base is None or time_s is None or base_time is None:
                    below_floor = None
                elif fmt == BASELINE_FORMAT:
                    below_floor = None  # the baseline is the reference itself
                else:
                    below_floor = abs(time_s - base_time) < CITATION_FLOOR_S

                records.append(
                    {
                        "dataset": dataset,
                        "format": fmt,
                        "scenario_key": key,
                        "grain_comparable": key
                        not in GRAIN_INCOMPARABLE_SCENARIOS,
                        "time_s": time_s,
                        "time_mad_s": _float(row["time_mad_s"]),
                        # The reference's own seconds, so a view can turn a
                        # ratio back into wall-clock without re-deriving it.
                        "base_time_s": base_time,
                        "heap_b": heap_b,
                        "rss_b": rss_b,
                        "result_count": _int(row["result_count"]),
                        "time_ratio": _ratio(time_s, base_time),
                        "heap_ratio": _ratio(
                            float(heap_b) if heap_b is not None else None,
                            base_heap,
                        ),
                        "rss_ratio": _ratio(
                            float(rss_b) if rss_b is not None else None,
                            base_rss,
                        ),
                        "below_floor": below_floor,
                    }
                )
    return records, anomalies


# --------------------------------------------------------------------------
# ordering
# --------------------------------------------------------------------------


def load_ordering(inputs: Inputs) -> list[dict]:
    """One record per (dataset, scenario) of the row-ordering run.

    A corpus with no ordering run at all is normal, not an error: it is a
    separate pass with its own recipe (`just ordering-bench`), and its corpus
    need not match the read benchmark's -- it routinely covers datasets the
    read benchmark never measured. Records therefore carry the dataset shape
    they need (object count, and the baseline's own absolute time) rather than
    expecting a `datasets` entry to exist for them.
    """
    records: list[dict] = []

    for path in _dataset_csvs(inputs.ordering_dir):
        dataset = path.stem
        rows = _read_rows(path, READ_COLUMNS)
        groups: dict[str, dict[str, dict[str, str]]] = {}
        for row in rows:
            if COLD_RE.search(row["notes"]):
                continue
            if row["format"] not in (ORDERING_BASELINE, ORDERING_VARIANT):
                continue
            bucket = groups.setdefault(_scenario_key(row), {})
            if row["format"] in bucket:
                raise PrepError(
                    f"{dataset}: duplicate ordering row for "
                    f"({_scenario_key(row)}, {row['format']})"
                )
            bucket[row["format"]] = row

        objects = None
        for bucket in groups.values():
            variant = bucket.get(ORDERING_VARIANT) or bucket.get(ORDERING_BASELINE)
            if variant is not None and variant["scenario"] == "full-read":
                objects = _int(variant["result_count"])

        for key in sorted(groups):
            bucket = groups[key]
            base, variant = bucket.get(ORDERING_BASELINE), bucket.get(ORDERING_VARIANT)
            if base is None or variant is None:
                continue  # a half-measured scenario answers no question
            base_t, variant_t = _float(base["time_s"]), _float(variant["time_s"])
            base_m = _float(base["peak_rss_bytes"])
            variant_m = _float(variant["peak_rss_bytes"])
            if base_t is None or variant_t is None:
                continue
            records.append(
                {
                    "dataset": dataset,
                    "scenario_key": key,
                    "objects": objects,
                    "base_time_s": base_t,
                    "variant_time_s": variant_t,
                    "base_rss_b": _int(base["peak_rss_bytes"]),
                    "variant_rss_b": _int(variant["peak_rss_bytes"]),
                    # Speed-up of the Hilbert package over the source-order one:
                    # >1 means ordering paid, <1 means it cost.
                    "time_ratio": _ratio(base_t, variant_t),
                    "rss_ratio": _ratio(base_m, variant_m),
                    "delta_s": abs(base_t - variant_t),
                    "below_floor": abs(base_t - variant_t) < CITATION_FLOOR_S,
                }
            )
    return records


# --------------------------------------------------------------------------
# sizes
# --------------------------------------------------------------------------


def load_sizes(
    inputs: Inputs, excluded: ExcludedFormats
) -> tuple[list[dict], dict[str, float]]:
    """Return (size records, {dataset: baseline MB})."""
    sizes_csv = inputs.sizes_csv
    rows = _read_rows(sizes_csv, SIZES_COLUMNS)
    by_dataset: dict[str, list[dict[str, str]]] = {}
    for row in rows:
        by_dataset.setdefault(row["dataset"], []).append(row)

    records: list[dict] = []
    raw_mb: dict[str, float] = {}
    for dataset in sorted(by_dataset):
        group = by_dataset[dataset]
        base = next(
            (r for r in group if r["format"] == BASELINE_FORMAT), None
        )
        base_bytes = _float(base["bytes"]) if base else None
        if base is not None:
            raw_mb[dataset] = float(base["mb"])
        for row in group:
            if row["format"] not in KNOWN_FORMATS:
                excluded.record(row["format"], "sizes")
                continue
            records.append(
                {
                    "dataset": dataset,
                    "format": row["format"],
                    "bytes": _int(row["bytes"]),
                    "frac_of_baseline": _ratio(_float(row["bytes"]), base_bytes),
                }
            )
    return records, raw_mb


# --------------------------------------------------------------------------
# compression
# --------------------------------------------------------------------------


def _compression_kind(variant: str) -> str:
    if variant == "cityparquet":
        return "default"
    if variant in ("cityparquet+rg512", "cityparquet+rg4096"):
        return "rowgroup"
    return "codec"


def load_compression(inputs: Inputs) -> tuple[list[dict], list[dict]]:
    """Compression records and the gaps worth stating.

    A corpus with no compression run at all is normal, not an error: the
    compression benchmark is a separate, much slower pass over the same inputs.
    Both renderers say so where the view would have been.
    """
    records: list[dict] = []
    gaps: list[dict] = []

    for path in _dataset_csvs(inputs.compression_dir):
        dataset = path.stem
        rows = _read_rows(path, COMPRESSION_COLUMNS)
        if not rows:
            gaps.append(
                {"dataset": dataset, "issue": "CSV present but header-only"}
            )
            continue

        base = next((r for r in rows if r["variant"] == "cityparquet"), None)
        base_write = _float(base["write_s"]) if base else None
        base_bytes = _float(base["total_bytes"]) if base else None

        roundtrips = [_bool(r["roundtrip_equal"]) for r in rows]
        if not any(roundtrips):
            gaps.append(
                {
                    "dataset": dataset,
                    "issue": "all roundtrip_equal=false (undocumented)",
                }
            )

        for row in rows:
            records.append(
                {
                    "dataset": dataset,
                    "variant": row["variant"],
                    "kind": _compression_kind(row["variant"]),
                    "write_s": _float(row["write_s"]),
                    "total_bytes": _int(row["total_bytes"]),
                    "full_scan_s": _float(row["full_scan_s"]),
                    "window_query_s": _float(row["window_query_s"]),
                    "write_ratio": _ratio(_float(row["write_s"]), base_write),
                    "size_ratio": _ratio(
                        _float(row["total_bytes"]), base_bytes
                    ),
                    "roundtrip": _bool(row["roundtrip_equal"]),
                }
            )

    gaps.sort(key=lambda g: g["dataset"])
    return records, gaps


# --------------------------------------------------------------------------
# datasets
# --------------------------------------------------------------------------


def _format_mb(mb: float) -> str:
    if mb < 10:
        return f"{mb:.1f}"
    return f"{round(mb):,}"


def build_datasets(
    read_records: list[dict], raw_mb: dict[str, float]
) -> list[dict]:
    counts: dict[str, dict[str, int | None]] = {}
    for rec in read_records:
        if rec["scenario_key"] != "full-read":
            continue
        entry = counts.setdefault(rec["dataset"], {})
        # Either CityParquet variant answers "how many CityObjects?": the count
        # is a property of the dataset, and Hilbert ordering changes the row
        # order, not the rows. A run that measured only one of the two (the
        # 2026-08-17 corpus run measured only the Hilbert package) still gets a
        # subtitle. Plain `cityparquet` wins where both were measured, so a
        # run carrying both reads exactly as it did before.
        if rec["format"] == "cityparquet":
            entry["objects"] = rec["result_count"]
        elif rec["format"] == "cityparquet-hilbert":
            entry.setdefault("objects_hilbert", rec["result_count"])
        elif rec["format"] == BASELINE_FORMAT:
            entry["features"] = rec["result_count"]

    datasets = []
    for dataset in sorted(counts):
        entry = counts[dataset]
        objects = entry.get("objects")
        if objects is None:
            objects = entry.get("objects_hilbert")
        features = entry.get("features")
        mb = raw_mb.get(dataset)
        if objects is None or mb is None:
            raise PrepError(
                f"{dataset}: missing full-read CityObject count (no cityparquet "
                f"or cityparquet-hilbert row) or sizes.csv baseline row "
                f"(objects={objects}, raw_mb={mb})"
            )
        datasets.append(
            {
                "id": dataset,
                "objects": objects,
                "features": features,
                "raw_mb": mb,
                "subtitle": (
                    f"{objects:,} CityObjects · {_format_mb(mb)} MB CityJSONSeq"
                ),
            }
        )
    # Objects descending; ties broken by id for a stable, reproducible order.
    datasets.sort(key=lambda d: (-d["objects"], d["id"]))
    return datasets


# --------------------------------------------------------------------------
# entry point
# --------------------------------------------------------------------------


def build(inputs: Inputs | None = None) -> tuple[dict, list[str]]:
    inputs = inputs or Inputs()
    excluded = ExcludedFormats()
    read_records, anomalies = load_read(inputs, excluded)
    size_records, raw_mb = load_sizes(inputs, excluded)
    datasets = build_datasets(read_records, raw_mb)
    compression_records, compression_gaps = load_compression(inputs)
    ordering_records = load_ordering(inputs)

    order = {d["id"]: i for i, d in enumerate(datasets)}
    read_records.sort(
        key=lambda r: (
            order.get(r["dataset"], len(order)),
            r["scenario_key"],
            r["format"],
        )
    )
    size_records.sort(
        key=lambda r: (order.get(r["dataset"], len(order)), r["format"])
    )
    compression_records.sort(
        key=lambda r: (order.get(r["dataset"], len(order)), r["variant"])
    )
    # The ordering corpus is not a subset of `datasets`, so datasets it alone
    # measured sort after the shared ones rather than being dropped.
    ordering_records.sort(
        key=lambda r: (
            order.get(r["dataset"], len(order)),
            r["dataset"],
            r["scenario_key"],
        )
    )

    data = {
        "meta": {
            "baseline": BASELINE_FORMAT,
            "sources": {
                "read": inputs.label(inputs.read_dir),
                "sizes": inputs.label(inputs.sizes_csv),
                "compression": inputs.label(inputs.compression_dir),
                "ordering": inputs.label(inputs.ordering_dir),
            },
            "caveats_read": read_caveats(inputs),
            "caveats_compression": compression_caveats(inputs),
            "codec_level_note": CODEC_LEVEL_NOTE,
            "citation_floor_s": CITATION_FLOOR_S,
            "ordering_baseline": ORDERING_BASELINE,
            "ordering_variant": ORDERING_VARIANT,
            "format_axis": list(FORMAT_AXIS),
            "object_grain_formats": list(OBJECT_GRAIN_FORMATS),
            "feature_grain_formats": list(FEATURE_GRAIN_FORMATS),
            "excluded_formats": excluded.as_list(),
        },
        "datasets": datasets,
        "read": read_records,
        "sizes": size_records,
        "compression": compression_records,
        "compression_gaps": compression_gaps,
        "ordering": ordering_records,
    }
    return data, anomalies + excluded.notes()


def main(inputs: Inputs | None = None, out_path: Path | None = None) -> Path:
    out = out_path or DEFAULT_DATA_PATH
    data, anomalies = build(inputs)
    # allow_nan=False: a NaN/inf would silently produce invalid JSON.
    text = json.dumps(data, indent=2, ensure_ascii=False, allow_nan=False)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text + "\n", encoding="utf-8")

    print(f"wrote {out} ({out.stat().st_size / 1024:.1f} KB)")
    print(
        f"  {len(data['datasets'])} datasets, {len(data['read'])} read records, "
        f"{len(data['sizes'])} size records, "
        f"{len(data['compression'])} compression records, "
        f"{len(data['compression_gaps'])} compression gaps, "
        f"{len(data['ordering'])} ordering records "
        f"({len({r['dataset'] for r in data['ordering']})} datasets)"
    )
    for note in anomalies:
        print(f"  anomaly: {note}")
    if not anomalies:
        print("  anomaly: none (no cold rows, no unknown formats)")
    return out


if __name__ == "__main__":  # pragma: no cover
    main()
