"""Chart cityparquet-rs's cross-format read-benchmark result CSVs."""

# The 13-column header `cityparquet-readbench`'s coordinator writes (see
# `crates/cityparquet-readbench/src/coordinator.rs`'s `CSV_HEADER`, which is
# the single authority on this contract — this list mirrors it, and
# `tests/test_csv_contract.py` reads that literal out of the Rust source to
# keep the two honest). A CSV whose first line doesn't *begin* with these
# columns is not a read-benchmark result (e.g. the M5 write-benchmark's
# differently-shaped CSVs under bench/results/) and is skipped.
CSV_HEADER = [
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
    "bytes_read",
    "http_requests",
]

# The formats this benchmark measures, in the canonical order
# `Format::ALL` declares (see
# `crates/cityparquet-readbench/src/format.rs`, which is the single authority
# on both the spelling and the order — `tests/test_formats.py` reads the
# `Format::as_str` match arms out of that source to keep the two honest).
# The order is the benchmark's argument, read left to right: the formats a
# city model actually ships as today, then the indexed/columnar ones, then
# the SQL-engine baseline.
#
# This list lives here, not in `plot.py` and `sizes.py`, because it used to
# live in both — with six entries in one and five in the other. A copy of a
# vocabulary is how this benchmark's CSV header contract drifted into three
# incompatible versions.
FORMAT_ORDER = [
    "citygml",
    "cityjson",
    "cityjsonseq",
    "cityjsonseq-gz",
    "flatcitybuf",
    "cityparquet",
    "cityparquet-hilbert",
    "duckdb-parquet",
]

# Per-format bar colours, keyed BY NAME and written out literally.
#
# Deriving them from a colormap by position (`zip(FORMAT_ORDER, tab10)`, as
# `sizes.py` used to) means adding one format shifts the colour of every
# format after it, silently recolouring already-published figures; `plot.py`
# was worse still, passing no colour at all and so taking matplotlib's
# default cycle by *draw order*, which changes with the format set a run
# happens to carry. Keyed by name, an addition can only ever add a colour.
#
# The five formats `sizes.py` already charted keep exactly the tab10 colours
# its positional mapping gave them, so nothing rendered before this change
# moves; the three formats added since take the tab10 slots that were left
# over, with grey for `duckdb-parquet` because it is an engine baseline
# rather than a format.
FORMAT_COLORS = {
    "citygml": "#8c564b",
    "cityjson": "#e377c2",
    "cityjsonseq": "#1f77b4",
    "cityjsonseq-gz": "#ff7f0e",
    "flatcitybuf": "#2ca02c",
    "cityparquet": "#d62728",
    "cityparquet-hilbert": "#9467bd",
    "duckdb-parquet": "#7f7f7f",
}

# A format seen in the data but not in `FORMAT_COLORS` — a tag added to the
# Rust side (or to a hand-written CSV) that this plotter has not learned yet.
# It must still be drawn (dropping measured data is worse than drawing it
# oddly), must not borrow a known format's colour, and must be obvious as
# unknown in a greyscale print, hence the hatch.
UNKNOWN_FORMAT_COLOR = "#bdbdbd"
UNKNOWN_FORMAT_HATCH = "//"


def bar_style(fmt: str) -> dict[str, str | None]:
    """Matplotlib bar kwargs (`color`, `hatch`) for one format's bars."""
    if fmt in FORMAT_COLORS:
        return {"color": FORMAT_COLORS[fmt], "hatch": None}
    return {"color": UNKNOWN_FORMAT_COLOR, "hatch": UNKNOWN_FORMAT_HATCH}


__all__ = [
    "CSV_HEADER",
    "FORMAT_COLORS",
    "FORMAT_ORDER",
    "UNKNOWN_FORMAT_COLOR",
    "UNKNOWN_FORMAT_HATCH",
    "bar_style",
]
