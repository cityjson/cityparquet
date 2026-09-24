"""CSVs -> bench_data.json.

Reads the benchmark result artefacts under ``benchmark/formats/`` — the CSVs a finished
``just bench`` / ``just codec-bench`` / ``just rowgroup-bench`` / ``just sizes`` run
leaves behind, never a benchmark of its own — and emits the ``bench_data.json``
data contract described in ``benchviz/DESIGN.md``.

Every path this module touches is derived from one ``Inputs.bench_dir``, so the
same code serves the in-repo default (``benchmark/formats/``) and an out-of-tree caller
that points ``--bench-dir`` at a checkout elsewhere.

stdlib only -- no third-party imports here, on purpose: this step must be
runnable from a bare Python with nothing installed.
"""

from __future__ import annotations

import csv
import json
import re
import tomllib
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
        return (
            self.bench_dir / "results"
            if (self.bench_dir / "results").exists()
            else self.bench_dir / "read_results"
        )

    # The scaling corpus: one city model cut to four cardinalities, which is
    # how the configuration axes are measured -- a codec or a row-group size
    # answers "how does this scale", not "how does this compare to Vienna".
    @property
    def scaling_codec_dir(self) -> Path:
        return self.bench_dir / "scaling_codec_results"

    @property
    def scaling_rowgroup_dir(self) -> Path:
        return self.bench_dir / "scaling_rowgroup_results"

    @property
    def scaling_bloom_dir(self) -> Path:
        return self.bench_dir / "scaling_bloom_results"

    @property
    def sizes_csv(self) -> Path:
        return self.read_dir / SIZES_CSV_NAME

    @property
    def read_benchmark_md(self) -> Path:
        return Path(__file__).resolve().parents[2] / "formats" / "READ_BENCHMARK.md"

    def label(self, path: Path) -> str:
        """A repo-qualified label for a source path, e.g.

        ``benchmark/formats/read_results``.

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
# (benchmark/readbench/src/format.rs): one tag per format family, with
# CityParquet represented by the Hilbert-ordered package — the configuration
# that would actually ship, so the comparison is not handicapped by an ordering
# choice no other format faces.
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

READ_COLUMNS = [
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    "time_s",
    "time_std_s",
    "peak_heap_bytes",
    "peak_rss_bytes",
    "repeat",
    "notes",
]
SIZES_COLUMNS = ["dataset", "format", "bytes", "mb", "ratio_vs_cityjsonseq"]

BBOX_NOTE_RE = re.compile(r"^bbox-\d+pct$")
# The positional id probes READ_BENCHMARK.md's id-lookup table defines: the id
# at 10/50/90% of the canonical order, plus one verified absent. Caveat 20 reads
# the hit rows as three samples of a distribution, so they stay three scenarios
# rather than being averaged into one.
ID_NOTE_RE = re.compile(r"^id-(?:\d+pct|miss)$")
FEATURE_NOTE_RE = re.compile(r"^feature-(?:\d+pct|miss)$")
COLD_RE = re.compile(r"\bcold\b", re.IGNORECASE)

CODEC_LEVEL_NOTE = (
    "The codec axis sweeps zstd, the codec CityParquet ships with, at levels "
    "1, 3 (the default and the 1x baseline), 9 and 19. The other codecs run "
    "at the parquet-rs defaults the writer recipe carries (gzip 6, brotli 1; "
    "crates/core/src/recipe.rs) and are drawn as reference points, not ranked "
    "against each other: no level was matched across codecs."
)
AXIS_BASELINE = "cityparquet"
AXIS_MEASURES = ("write", "full-read", "bbox-1pct", "bbox-5pct", "bbox-25pct", "id-50pct")
# The bloom axis measures the identifier lookups only; every other query is
# untouched by the filters.
BLOOM_MEASURES = ("write", "id-50pct", "id-miss", "feature-50pct", "feature-miss")
# The lookup counters a CityParquet lookup row carries, appended to the read
# CSV after `http_requests`.
LOOKUP_COLUMNS = ("row_groups_total", "bloom_pruned", "filter_bytes")
MACHINE_MD_NAME = "MACHINE.md"


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


def _dataset_csvs(directory: Path) -> list[Path]:
    """Dataset CSVs in a results directory, keyed by filename stem.

    The ``dataset`` column inside the CSVs is inconsistent (a source filename
    for most runners, a short name for the duckdb runner), so the filename
    stem is the authoritative dataset id.
    """
    return sorted(
        p
        for p in directory.glob("*.csv")
        if p.name != SIZES_CSV_NAME and not p.name.endswith(".samples.csv")
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
        raise PrepError(f"{path}: could not find a '## {heading_prefix}...' heading")
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
    indents = [len(ln) - len(ln.lstrip(" ")) for ln in lines[1:] if ln.strip()]
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
    number and a misaligned list would footnote the wrong text.
    """
    path = inputs.read_benchmark_md
    body = _extract_section(path, "Fairness caveats")
    raw = _split_numbered_items(body)
    numbers = [int(re.match(r"^(\d+)\.", item).group(1)) for item in raw]
    if numbers != list(range(1, len(raw) + 1)):
        raise PrepError(f"{path}: fairness caveats are not numbered 1..n without a gap: {numbers}")
    items = [_dedent_item(item) for item in raw]
    if not items:
        raise PrepError(f"{path}: no fairness caveats found")
    if any(not item for item in items):
        raise PrepError(f"{path}: an extracted caveat is empty")
    return items


# --------------------------------------------------------------------------
# read benchmark
# --------------------------------------------------------------------------


def _primary_tag(notes: str) -> str:
    """A note's own tag, with any ``;``-separated disclosures dropped.

    A disclosure qualifies a measurement without changing WHICH measurement it
    is — ``no-attr-index`` on the FlatCityBuf rows, ``id-substituted`` when a
    probe had to be replaced by its nearest verified neighbour. Keying on it
    would put that format in a group of its own, with no baseline row to be a
    ratio against, and quietly drop it from the comparison.
    """
    return notes.split(";", 1)[0].strip()


def _scenario_key(row: dict[str, str]) -> str:
    scenario = row["scenario"]
    notes = row["notes"].strip()
    if scenario == "bbox-query":
        if not BBOX_NOTE_RE.match(notes.split(";", 1)[0]):
            raise PrepError(
                f"bbox-query row with unrecognised notes tag {notes!r} "
                f"(dataset={row['dataset']}, format={row['format']})"
            )
        return notes.split(";", 1)[0]
    if scenario == "id-lookup":
        # Two generations of the runner are committed at once: `read_results/`
        # carries the positional probes, which are one scenario each, while the
        # scaling directories still hold the single `id=<identifier>` probe of
        # the run that produced them. An unrecognised tag is that older shape,
        # not an error — those artefacts are not re-measured to suit a renderer.
        tag = _primary_tag(notes)
        if ID_NOTE_RE.match(tag):
            return tag
    if scenario == "feature-lookup":
        tag = _primary_tag(notes)
        if FEATURE_NOTE_RE.match(tag):
            return tag
    return scenario


def load_read(inputs: Inputs, excluded: ExcludedFormats) -> tuple[list[dict], list[str]]:
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
        anomalies.append(f"excluded {cold_rows} cold-tagged read row(s) (warm-only policy)")

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

                records.append(
                    {
                        "dataset": dataset,
                        "format": fmt,
                        "scenario_key": key,
                        "time_s": time_s,
                        "time_std_s": _float(row["time_std_s"]),
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
                        "notes": row["notes"],
                        "status": row.get("status", ""),
                    }
                )
    return records, anomalies


# --------------------------------------------------------------------------
# scaling corpus
# --------------------------------------------------------------------------


def _measure_key(row: dict[str, str]) -> str:
    return "write" if row["scenario"] == "write" else _scenario_key(row)


MANIFEST_PATH = Path(__file__).resolve().parents[2] / "manifest.toml"
SCALING_ROLES = frozenset({"scaling", "largest-scaling"})


def _manifest_stem(source: str) -> str:
    return source.removesuffix(".city.jsonl").removesuffix(".city.json")


def scaling_series_ids(path: Path = MANIFEST_PATH) -> frozenset[str]:
    """Dataset ids of the nested 3DBAG slices, from the suite manifest.

    Only these form a scaling curve: the slices are prefixes of one source, so
    their differences are differences of size. A corpus dataset is a different
    city model, and joining it to the curve would read a data difference as a
    scale effect.
    """
    if not path.exists():
        return frozenset()
    manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    return frozenset(
        _manifest_stem(entry.get("source", ""))
        for entry in manifest.get("datasets", {}).values()
        if entry.get("role") in SCALING_ROLES
    )


def load_scaling_axis(
    directory: Path, baseline: str = AXIS_BASELINE, measures: tuple[str, ...] = AXIS_MEASURES
) -> dict:
    """One configuration axis (codec, row group or bloom) from a `--variants` run.

    Every record and size row carries `series`: `scaling` for a nested 3DBAG
    slice (`scaling_series_ids`), `corpus` for any other input, so the
    renderer draws curves from the slices alone.

    Every ratio is variant over default, so values below 1x use less time,
    memory or disk. The
    variant order is the CSVs' own first-seen order, because the recipe's
    list is the figure's order and sorting would lose it. Absolute seconds
    and bytes stay: the trend strip plots them.
    """
    series_ids = scaling_series_ids()
    records: list[dict] = []
    sizes: list[dict] = []
    gaps: list[dict] = []
    variants: list[str] = []
    objects_by: dict[str, int | None] = {}

    for path in _dataset_csvs(directory):
        name = path.stem
        rows = _read_rows(path, READ_COLUMNS)
        if not rows:
            gaps.append({"dataset": name, "issue": "CSV present but header-only"})
            continue
        by_measure: dict[str, dict[str, dict[str, str]]] = {}
        for row in rows:
            if COLD_RE.search(row["notes"]):
                continue
            by_measure.setdefault(_measure_key(row), {})[row["format"]] = row
            if row["format"] not in variants:
                variants.append(row["format"])
        if not any(baseline in bucket for bucket in by_measure.values()):
            gaps.append({"dataset": name, "issue": f"no '{baseline}' baseline rows"})
            continue
        write_base = by_measure.get("write", {}).get(baseline)
        write_valid = write_base is not None and write_base.get("status", "").strip().lower() in {"", "ok"}
        objects_by[name] = (
            _int(write_base["result_count"])
            if write_valid and _int(write_base["result_count"]) is not None
            else next(
                (
                    _int(r["result_count"])
                    for bucket in by_measure.values()
                    for r in bucket.values()
                    if r.get("status", "").strip().lower() in {"", "ok"}
                    and _int(r["result_count"]) is not None
                ),
                None,
            )
        )
        present = {v for bucket in by_measure.values() for v in bucket}
        for key in measures:
            bucket = by_measure.get(key)
            if not bucket:
                continue
            base = bucket.get(baseline)
            base_status = (base.get("status", "") if base else "missing").strip()
            base_valid = base is not None and base_status.lower() in {"", "ok"}
            base_t = _float(base["time_s"]) if base_valid else None
            # The dispersion travels with the time it belongs to: a headline
            # that names one variant the fastest has to be able to check the
            # lead against the two runs' own spread, not against a fixed floor.
            base_mad = _float(base["time_std_s"]) if base_valid else None
            base_rss = _int(base["peak_rss_bytes"]) if base_valid else None
            if base is not None and not base_valid:
                gaps.append({"dataset": name, "issue": f"{baseline} {key} status={base_status}"})
            for variant in variants:
                row = bucket.get(variant)
                if row is None:
                    if variant in present:
                        gaps.append({"dataset": name, "issue": f"{variant} has no {key} row"})
                    continue
                status = row.get("status", "").strip()
                valid = status.lower() in {"", "ok"}
                if not valid:
                    gaps.append({"dataset": name, "issue": f"{variant} {key} status={status}"})
                # Failed, skipped and mismatched probes remain visible to the
                # renderer as labelled empty cells; they never become ratios.
                t = _float(row["time_s"]) if valid else None
                mad = _float(row["time_std_s"]) if valid else None
                rss = _int(row["peak_rss_bytes"]) if valid else None
                records.append(
                    {
                        "dataset": name,
                        "series": "scaling" if name in series_ids else "corpus",
                        "objects": objects_by[name],
                        "variant": variant,
                        "kind": "default" if variant == baseline else "variant",
                        "measure": key,
                        "time_s": t,
                        "time_std_s": mad,
                        "rss_b": rss,
                        "base_time_s": base_t,
                        "base_time_std_s": base_mad,
                        "base_rss_b": base_rss,
                        "time_ratio": _ratio(t, base_t),
                        "rss_ratio": _ratio(
                            float(rss) if rss is not None else None,
                            float(base_rss) if base_rss is not None else None,
                        ),
                        "status": status,
                        "notes": row.get("notes", ""),
                        **{column: _int(row.get(column)) for column in LOOKUP_COLUMNS},
                    }
                )

    sizes_csv = directory / SIZES_CSV_NAME
    if sizes_csv.exists():
        by_ds: dict[str, dict[str, dict[str, str]]] = {}
        for row in _read_rows(sizes_csv, SIZES_COLUMNS):
            by_ds.setdefault(row["dataset"], {})[row["format"]] = row
        measured = {(r["dataset"], r["variant"]) for r in records}
        for ds, fmts in by_ds.items():
            base_b = _int(fmts[baseline]["bytes"]) if baseline in fmts else None
            for fmt in variants:
                row = fmts.get(fmt)
                if row is None:
                    continue
                b = _int(row["bytes"])
                if (ds, fmt) not in measured:
                    gaps.append(
                        {"dataset": ds, "issue": f"sizes.csv row for {fmt} without a measurement"}
                    )
                sizes.append(
                    {
                        "dataset": ds,
                        "series": "scaling" if ds in series_ids else "corpus",
                        "objects": objects_by.get(ds),
                        "variant": fmt,
                        "bytes": b,
                        "mb": _float(row["mb"]),
                        "ratio_vs_cityjsonseq": _float(row["ratio_vs_cityjsonseq"]),
                        "size_ratio": _ratio(
                            float(base_b) if base_b is not None else None,
                            float(b) if b is not None else None,
                        ),
                    }
                )

    gaps.sort(key=lambda g: (g["dataset"], g["issue"]))
    return {"records": records, "sizes": sizes, "gaps": gaps, "variants": variants}


def read_machine(directory: Path) -> str | None:
    """The run's MACHINE.md, verbatim, or None when the directory has none."""
    path = directory / MACHINE_MD_NAME
    if not path.is_file():
        return None
    return path.read_text(encoding="utf-8")


# --------------------------------------------------------------------------
# sizes
# --------------------------------------------------------------------------


def load_sizes(inputs: Inputs, excluded: ExcludedFormats) -> tuple[list[dict], dict[str, float]]:
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
        base = next((r for r in group if r["format"] == BASELINE_FORMAT), None)
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
# datasets
# --------------------------------------------------------------------------


def _format_mb(mb: float) -> str:
    if mb < 10:
        return f"{mb:.1f}"
    return f"{round(mb):,}"


def build_datasets(read_records: list[dict], raw_mb: dict[str, float]) -> list[dict]:
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
    for dataset in sorted(set(counts) | set(raw_mb) | {r["dataset"] for r in read_records}):
        entry = counts.get(dataset, {})
        objects = entry.get("objects")
        if objects is None:
            objects = entry.get("objects_hilbert")
        features = entry.get("features")
        mb = raw_mb.get(dataset)
        datasets.append(
            {
                "id": dataset,
                "objects": objects,
                "features": features,
                "raw_mb": mb,
                "subtitle": " · ".join(
                    part
                    for part in (
                        f"{objects:,} CityObjects" if objects is not None else "",
                        f"{_format_mb(mb)} MB CityJSONSeq" if mb is not None else "",
                    )
                    if part
                ),
            }
        )
    # Objects descending; ties broken by id for a stable, reproducible order.
    datasets.sort(key=lambda d: (-(d["objects"] or 0), d["id"]))
    return datasets


# --------------------------------------------------------------------------
# entry point
# --------------------------------------------------------------------------


def _require_read_results(inputs: Inputs) -> None:
    """Fail with an actionable message when the read results are absent.

    The read results are the spine of the summary page: every other view is
    keyed by the datasets found there, so an empty `read_dir` produces not an
    empty page but a `FileNotFoundError` on `sizes.csv` from three frames
    down. That is a legitimate state rather than a broken checkout — the
    corpus was replaced on 2026-08-23 and the previous run's CSVs were
    archived rather than left where this would chart them as current — so it
    deserves a sentence naming the recipe that fixes it, not a traceback.
    """
    if inputs.sizes_csv.exists() and _dataset_csvs(inputs.read_dir):
        return
    raise PrepError(
        f"no read-benchmark results in {inputs.read_dir}.\n"
        "  The summary page is built from them, so there is nothing to plot "
        "yet.\n"
        "  Produce them with:\n"
        "      just fetch-tools                 # once, needs java 17+\n"
        "      just fetch-data                  # the corpus (network, 423 MB)\n"
        "      just bench benchmark/formats/data/benchmark  # the format comparison\n"
        "  The previous corpus's results were archived on 2026-08-23 under\n"
        "  benchmark/formats/archive/2026-08-17-catalogue-corpus/ — see its README."
    )


def load_databases(inputs: Inputs) -> dict:
    """Load one selected database result set, preserving unavailable cells."""

    def safe_float(value: str | None) -> float | None:
        try:
            return _float(value)
        except ValueError:
            return None

    def safe_int(value: str | None) -> int | None:
        try:
            return _int(value)
        except ValueError:
            return None

    smoke = inputs.bench_dir.name == "smoke"
    data_root = inputs.bench_dir.parent.parent if smoke else inputs.bench_dir.parent
    directory = data_root / "databases" / ("smoke" if smoke else "results")
    if not directory.exists():
        return {"baseline": "3dcitydb", "records": [], "sizes": []}
    candidates = sorted(
        p for p in directory.glob("*.csv") if not p.name.endswith((".sizes.csv", ".samples.csv"))
    )
    groups: list[tuple[int, Path, list[dict[str, str]]]] = []
    for path in candidates:
        rows = list(csv.DictReader(path.open(encoding="utf-8", newline="")))
        params = path.with_suffix(".params.json")
        objects = 0
        if params.exists():
            objects = int(json.loads(params.read_text()).get("total_city_objects", 0))
        groups.append((objects, path, rows))
    if not groups:
        return {"baseline": "3dcitydb", "records": [], "sizes": []}
    objects, path, rows = max(groups, key=lambda item: (item[0], item[1].name))
    records, sizes = [], []
    for row in rows:
        note = row.get("notes", "")
        scenario = row.get("scenario", "")
        match = re.search(r"\bbbox-(\d+pct)\b", note)
        query = f"bbox-{match.group(1)}" if match else scenario
        records.append(
            {
                "dataset": path.stem,
                "objects": objects,
                "format": row.get("format"),
                "scenario": query,
                "time_s": safe_float(row.get("time_s")),
                "peak_rss_bytes": safe_int(row.get("peak_rss_bytes")),
                "size_bytes": safe_int(row.get("size_bytes")),
                "status": row.get("status", ""),
                "notes": note,
            }
        )
    by_system: dict[str, dict] = {}
    for row in records:
        if row["format"] and row["format"] not in by_system:
            by_system[row["format"]] = row
    for system, row in by_system.items():
        sizes.append({"format": system, "size_bytes": row["size_bytes"]})
    return {
        "baseline": "3dcitydb",
        "records": records,
        "sizes": sizes,
        "dataset": path.stem,
        "objects": objects,
    }


def apply_manifest_titles(inputs: Inputs, datasets: list[dict]) -> None:
    path = Path(__file__).resolve().parents[2] / "manifest.toml"
    if not path.exists():
        return
    manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    entries = manifest.get("datasets", {})
    for dataset in datasets:
        entry = next(
            (
                item
                for item in entries.values()
                if item.get("source", "").removesuffix(".city.jsonl").removesuffix(".city.json")
                == dataset["id"]
            ),
            {},
        )
        if entry.get("title"):
            dataset["title"] = entry["title"]


def build(inputs: Inputs | None = None) -> tuple[dict, list[str]]:
    inputs = inputs or Inputs()
    excluded = ExcludedFormats()
    has_read = bool(_dataset_csvs(inputs.read_dir))
    has_sizes = inputs.sizes_csv.exists()
    read_records, anomalies = load_read(inputs, excluded) if has_read else ([], [])
    size_records, raw_mb = load_sizes(inputs, excluded) if has_sizes else ([], {})
    for row in size_records:
        raw_mb.setdefault(row["dataset"], None)
    datasets = build_datasets(read_records, raw_mb)
    apply_manifest_titles(inputs, datasets)
    database_data = load_databases(inputs)
    scaling = {
        "codec": load_scaling_axis(inputs.scaling_codec_dir),
        "rowgroup": load_scaling_axis(inputs.scaling_rowgroup_dir),
        "bloom": load_scaling_axis(inputs.scaling_bloom_dir, measures=BLOOM_MEASURES),
    }

    order = {d["id"]: i for i, d in enumerate(datasets)}
    read_records.sort(
        key=lambda r: (
            order.get(r["dataset"], len(order)),
            r["scenario_key"],
            r["format"],
        )
    )
    size_records.sort(key=lambda r: (order.get(r["dataset"], len(order)), r["format"]))

    data = {
        "meta": {
            "baseline": BASELINE_FORMAT,
            "sources": {
                "read": inputs.label(inputs.read_dir),
                "sizes": inputs.label(inputs.sizes_csv),
                "codec": inputs.label(inputs.scaling_codec_dir),
                "rowgroup": inputs.label(inputs.scaling_rowgroup_dir),
                "bloom": inputs.label(inputs.scaling_bloom_dir),
            },
            "caveats_read": read_caveats(inputs),
            "codec_level_note": CODEC_LEVEL_NOTE,
            "axis_baseline": AXIS_BASELINE,
            "format_axis": list(FORMAT_AXIS),
            "object_grain_formats": list(OBJECT_GRAIN_FORMATS),
            "feature_grain_formats": list(FEATURE_GRAIN_FORMATS),
            "excluded_formats": excluded.as_list(),
            "machine": {
                "codec": read_machine(inputs.scaling_codec_dir),
                "rowgroup": read_machine(inputs.scaling_rowgroup_dir),
                "bloom": read_machine(inputs.scaling_bloom_dir),
            },
        },
        "datasets": datasets,
        "read": read_records,
        "sizes": size_records,
        "scaling": scaling,
        "databases": database_data,
    }
    return data, anomalies + excluded.notes()


def main(inputs: Inputs | None = None, out_path: Path | None = None) -> Path:
    out = out_path or DEFAULT_DATA_PATH
    data, anomalies = build(inputs)
    # allow_nan=False: a NaN/inf would silently produce invalid JSON.
    text = json.dumps(data, indent=2, ensure_ascii=False, allow_nan=False)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text + "\n", encoding="utf-8")
    completeness = {
        "formats": bool(data["read"] or data["sizes"]),
        "codec": bool(data["scaling"]["codec"]["records"]),
        "rowgroup": bool(data["scaling"]["rowgroup"]["records"]),
        "bloom": bool(data["scaling"]["bloom"]["records"]),
        "databases": bool(data["databases"]["records"] or data["databases"]["sizes"]),
    }
    (out.parent / "completeness.json").write_text(
        json.dumps(completeness, indent=2) + "\n", encoding="utf-8"
    )

    print(f"wrote {out} ({out.stat().st_size / 1024:.1f} KB)")
    print(
        f"  {len(data['datasets'])} datasets, {len(data['read'])} read records, "
        f"{len(data['sizes'])} size records, "
        f"{len(data['scaling']['codec']['records'])} codec records, "
        f"{len(data['scaling']['rowgroup']['records'])} row-group records, "
        f"{len(data['scaling']['bloom']['records'])} bloom records"
    )
    for note in anomalies:
        print(f"  anomaly: {note}")
    if not anomalies:
        print("  anomaly: none (no cold rows, no unknown formats)")
    return out


if __name__ == "__main__":  # pragma: no cover
    main()
