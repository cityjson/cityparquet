//! The [`FormatRunner`] trait every per-format read-benchmark backend
//! implements, plus [`resolve`] — the `--format <name>` dispatch the
//! `--child` process (`main.rs`) uses.

pub mod cityjsonseq;
pub mod cityparquet;
pub mod flatcitybuf;

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::scenario::{QueryParams, Scenario};

/// Where a format's artefact lives: a local filesystem path (today's only
/// transport) or an HTTP location (`base_url` + the artefact's own relative
/// `key`, e.g. `"delft.parquet"`, `"delft.parquet/building.parquet"` for a
/// sub-file within a CityParquet package directory, `"delft.fcb"`,
/// `"delft.city.jsonl"`).
#[derive(Debug, Clone)]
pub enum Source {
    Local(PathBuf),
    // `base_url`/`key` are read starting with the CityParquet HTTP runner
    // (next task); `#[allow(dead_code)]` is temporary until every format's
    // `Source::Http` arm is implemented.
    #[allow(dead_code)]
    Http {
        base_url: String,
        key: String,
    },
}

/// Bytes transferred and HTTP request count for one measurement — `None`
/// for [`Source::Local`] (no meaningful "HTTP request" concept, and this
/// keeps every existing local CSV row's shape unchanged; see
/// `coordinator::write_row`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoStats {
    pub bytes: u64,
    pub requests: u64,
}

/// A [`FormatRunner::run`] call's result: the scenario's natural result
/// cardinality, plus [`IoStats`] when `source` was [`Source::Http`].
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub result_count: u64,
    pub io: Option<IoStats>,
}

/// One format's read-benchmark backend: runs exactly one [`Scenario`]
/// against `source` (a format-specific location — a CityParquet package
/// directory or its main table file, a `.city.jsonl`/`.jsonl.gz` file, or a
/// `.fcb` file, either local or over HTTP) and returns the scenario's
/// natural result cardinality (`result_count` — see the milestone plan's
/// "Scenario & metric contract" for what that means per scenario; in
/// particular `Count`/`FullRead` counts are NOT necessarily comparable
/// across formats — each runner's own doc comment discloses its counting
/// semantics).
///
/// Implementations are expected to open `source` themselves on every call
/// (no persistent state between calls) — the `--child` protocol spawns one
/// fresh process per (format, scenario, dataset, repeat) measurement, so a
/// `FormatRunner` never needs to serve more than one [`Self::run`] call in
/// its process lifetime.
pub trait FormatRunner {
    fn run(&self, source: &Source, scenario: Scenario, params: &QueryParams) -> Result<RunOutcome>;
}

/// Resolves `--format <name>` to its [`FormatRunner`]. `cityparquet`,
/// `cityjsonseq`, `cityjsonseq-gz`, and `flatcitybuf` are implemented.
/// `cityparquet-hilbert` is an ALIAS for the same [`cityparquet::CityParquetRunner`]:
/// the Hilbert-ordered package is still a plain CityParquet package on
/// disk (same reader, same query primitives) — the only difference from
/// `cityparquet` is WHICH artefact path the coordinator resolves
/// `--input` to (`coordinator::resolve_format_artefact` already points
/// `cityparquet-hilbert` at the `<name>-hilbert.parquet` package), so no
/// separate runner type is needed here. `duckdb-parquet` is a SQL-engine
/// baseline driven entirely by `scripts/readbench_duckdb.sh`; it is not,
/// and never will be, a `--child` format.
pub fn resolve(format: &str) -> Result<Box<dyn FormatRunner>> {
    match format {
        "cityparquet" | "cityparquet-hilbert" => Ok(Box::new(cityparquet::CityParquetRunner)),
        "cityjsonseq" => Ok(Box::new(cityjsonseq::CityJsonSeqRunner::plain())),
        "cityjsonseq-gz" => Ok(Box::new(cityjsonseq::CityJsonSeqRunner::gzip())),
        "flatcitybuf" => Ok(Box::new(flatcitybuf::FlatCityBufRunner)),
        "duckdb-parquet" => bail!(
            "format 'duckdb-parquet' is a SQL-engine baseline driven by \
             scripts/readbench_duckdb.sh, not this binary's --child path"
        ),
        other => bail!(
            "unknown format '{other}'; expected one of: cityparquet, cityparquet-hilbert, \
             cityjsonseq, cityjsonseq-gz, flatcitybuf, duckdb-parquet"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_implemented_formats_succeed_and_others_error_cleanly() {
        assert!(resolve("cityparquet").is_ok());
        assert!(resolve("cityparquet-hilbert").is_ok());
        assert!(resolve("cityjsonseq").is_ok());
        assert!(resolve("cityjsonseq-gz").is_ok());
        assert!(resolve("flatcitybuf").is_ok());
        assert!(resolve("duckdb-parquet").is_err());
        assert!(resolve("not-a-format").is_err());
    }
}
