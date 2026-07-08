//! The [`FormatRunner`] trait every per-format read-benchmark backend
//! implements, plus [`resolve`] — the `--format <name>` dispatch the
//! `--child` process (`main.rs`) uses.

pub mod cityjsonseq;
pub mod cityparquet;
pub mod flatcitybuf;

use std::path::Path;

use anyhow::{Result, bail};

use crate::scenario::{QueryParams, Scenario};

/// One format's read-benchmark backend: runs exactly one [`Scenario`]
/// against `input` (a format-specific path — a CityParquet package
/// directory or its main table file, a `.city.jsonl`/`.jsonl.gz` file, or a
/// `.fcb` file) and returns the scenario's natural result cardinality
/// (`result_count` — see the milestone plan's "Scenario & metric contract"
/// for what that means per scenario; in particular `Count`/`FullRead`
/// counts are NOT necessarily comparable across formats — each runner's own
/// doc comment discloses its counting semantics).
///
/// Implementations are expected to open `input` themselves on every call
/// (no persistent state between calls) — the `--child` protocol spawns one
/// fresh process per (format, scenario, dataset, repeat) measurement, so a
/// `FormatRunner` never needs to serve more than one [`Self::run`] call in
/// its process lifetime.
pub trait FormatRunner {
    fn run(&self, input: &Path, scenario: Scenario, params: &QueryParams) -> Result<u64>;
}

/// Resolves `--format <name>` to its [`FormatRunner`]. `cityparquet`,
/// `cityjsonseq`, `cityjsonseq-gz`, and `flatcitybuf` are implemented.
/// `duckdb-parquet` is a SQL-engine baseline driven entirely by
/// `scripts/readbench_duckdb.sh`; it is not, and never will be, a
/// `--child` format.
pub fn resolve(format: &str) -> Result<Box<dyn FormatRunner>> {
    match format {
        "cityparquet" => Ok(Box::new(cityparquet::CityParquetRunner)),
        "cityjsonseq" => Ok(Box::new(cityjsonseq::CityJsonSeqRunner::plain())),
        "cityjsonseq-gz" => Ok(Box::new(cityjsonseq::CityJsonSeqRunner::gzip())),
        "flatcitybuf" => Ok(Box::new(flatcitybuf::FlatCityBufRunner)),
        "duckdb-parquet" => bail!(
            "format 'duckdb-parquet' is a SQL-engine baseline driven by \
             scripts/readbench_duckdb.sh, not this binary's --child path"
        ),
        other => bail!(
            "unknown format '{other}'; expected one of: cityparquet, cityjsonseq, \
             cityjsonseq-gz, flatcitybuf, duckdb-parquet"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_implemented_formats_succeed_and_others_error_cleanly() {
        assert!(resolve("cityparquet").is_ok());
        assert!(resolve("cityjsonseq").is_ok());
        assert!(resolve("cityjsonseq-gz").is_ok());
        assert!(resolve("flatcitybuf").is_ok());
        assert!(resolve("duckdb-parquet").is_err());
        assert!(resolve("not-a-format").is_err());
    }
}
