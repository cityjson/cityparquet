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

__all__ = ["CSV_HEADER"]
