"""Chart cityparquet-rs's cross-format read-benchmark result CSVs."""

# The 11-column header `cityparquet-readbench`'s coordinator writes (see
# `crates/cityparquet-readbench/src/coordinator.rs`'s `CSV_HEADER`). A CSV
# whose first line doesn't match this exactly is not a read-benchmark
# result (e.g. the M5 write-benchmark's differently-shaped CSVs under
# bench/results/) and is skipped.
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
]

__all__ = ["CSV_HEADER"]
