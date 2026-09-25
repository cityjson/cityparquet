//! The [`FormatRunner`] trait every per-format read-benchmark backend
//! implements, plus [`resolve`] — the `--format <name>` dispatch the
//! `--child` process (`main.rs`) uses.

pub mod citygml;
pub mod cityjson;
pub mod cityjsonseq;
pub mod cityparquet;
pub mod flatcitybuf;

use std::path::PathBuf;

use anyhow::{Result, bail};
use cityparquet_readbench::format::Format;

use crate::scenario::{QueryParams, Scenario};

/// Where a format's artefact lives: a local filesystem path (today's only
/// transport) or an HTTP location (`base_url` + the artefact's own relative
/// `key`, e.g. `"delft.parquet"`, `"delft.parquet/building.parquet"` for a
/// sub-file within a CityParquet package directory, `"delft.fcb"`,
/// `"delft.city.jsonl"`).
#[derive(Debug, Clone)]
pub enum Source {
    Local(PathBuf),
    Http { base_url: String, key: String },
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
/// cardinality, plus [`IoStats`] when `source` was [`Source::Http`], plus
/// [`LookupCounters`] for a CityParquet identifier lookup.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub result_count: u64,
    pub io: Option<IoStats>,
    pub lookup: Option<LookupCounters>,
    /// The four aggregates of an [`Scenario::AttrStats`] run; `None` for
    /// every other scenario.
    pub attr_stats: Option<AttrAggregates>,
}

/// `(min, max, sum, count)` of one numeric attribute over every CityObject
/// carrying a numeric value for it — the four aggregates
/// [`Scenario::AttrStats`] computes in EVERY format, so that no format's row
/// is timing a cheaper question than another's.
///
/// One accumulation rule for every runner that walks values itself: each
/// value is taken as `f64` (`serde_json::Value::as_f64`, so integers and
/// floats alike) and folded with `f64::min`/`f64::max`/`+=`. A NaN never
/// occurs in the fixtures; if one did, `min`/`max` would ignore it and `sum`
/// would become NaN — never a panic. `count` is the scenario's
/// `result_count`. With `count == 0`, `min`/`max` stay at `+inf`/`-inf`.
///
/// CityParquet fills this from `cityparquet::query::attr_stats` instead (min
/// and max from column-chunk statistics, sum and count from a projected
/// scan): that is the format's own mechanism, not this accumulator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttrAggregates {
    pub min: f64,
    pub max: f64,
    pub sum: f64,
    pub count: u64,
}

impl AttrAggregates {
    /// The aggregates of no values at all.
    pub const EMPTY: Self = Self {
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        sum: 0.0,
        count: 0,
    };

    /// Folds one numeric value in.
    pub fn push(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.sum += value;
        self.count += 1;
    }
}

/// What a scenario body answers before its transport adds [`IoStats`]: the
/// `result_count`, plus the aggregates when the scenario was
/// [`Scenario::AttrStats`].
pub(crate) struct Answer {
    pub result_count: u64,
    pub attr_stats: Option<AttrAggregates>,
}

impl From<u64> for Answer {
    fn from(result_count: u64) -> Self {
        Self {
            result_count,
            attr_stats: None,
        }
    }
}

impl From<AttrAggregates> for Answer {
    fn from(stats: AttrAggregates) -> Self {
        Self {
            result_count: stats.count,
            attr_stats: Some(stats),
        }
    }
}

/// A child reports its [`AttrAggregates`] on stderr, after its timed stdout
/// line, as `<marker> <min> <max> <sum> <count>`, so the aggregates can be
/// checked across formats without changing the timed stdout protocol or the
/// results CSV.
pub const ATTR_STATS_MARKER: &str = "cityparquet-readbench: attr-stats";

/// What the bloom filters did for one CityParquet identifier lookup —
/// `cityparquet::query::LookupStats` as the child reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookupCounters {
    pub row_groups_total: u64,
    pub bloom_pruned: u64,
    /// Bitset bytes of every filter examined (`LookupStats::filter_bytes`).
    pub filter_bytes: u64,
}

/// A child reports its [`LookupCounters`] on stderr, after its timed stdout
/// line, as `<marker> <row_groups_total> <bloom_pruned> <filter_bytes>`, so
/// the timed stdout protocol keeps its shape.
pub const LOOKUP_STATS_MARKER: &str = "cityparquet-readbench: lookup-stats";

/// Every non-CityParquet runner's answer to [`Scenario::FeatureLookup`].
pub const FEATURE_LOOKUP_CITYPARQUET_ONLY: &str =
    "scenario 'feature-lookup' is measured for CityParquet only";

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

/// Resolves a [`Format`] to its [`FormatRunner`]. This match is exhaustive,
/// so adding a [`Format`] variant is a compiler error here rather than a
/// silently-missing backend; an unknown NAME never reaches this function at
/// all, because `Format`'s own [`FromStr`](std::str::FromStr) rejects it at
/// CLI-parse time.
///
/// [`Format::CityParquetHilbert`] is an ALIAS for the same
/// [`cityparquet::CityParquetRunner`]: the Hilbert-ordered package is still
/// a plain CityParquet package on disk (same reader, same query
/// primitives) — the only difference from [`Format::CityParquet`] is WHICH
/// artefact path the coordinator resolves `--input` to (see
/// [`Format::artefact`]), so no separate runner type is needed here.
///
/// [`Format::CityJson`] is NOT an alias for [`Format::CityJsonSeq`]: a plain
/// whole-document `.city.json` parses as one JSON object with a shared
/// document-level `vertices` array, where a Seq stream is line-oriented with
/// feature-local vertices — different parse shape, different counting grain,
/// so its own runner (see [`cityjson`]'s module doc).
///
/// [`Format::CityGml`] has its own runner (see [`citygml`]'s module doc): the
/// format the source data ships in, read with this repository's own CityGML
/// 2.0 reader, with no index and therefore a full parse per scenario.
/// [`Format::DuckDbParquet`] is a SQL-engine baseline driven entirely by
/// `benchmark/scripts/readbench_duckdb.sh`; it is not, and never will be, a `--child`
/// format — the `bail!` arm below is the single production statement of that
/// fact, and [`Format::artefact`]'s `NotCoordinated` is its counterpart on
/// the coordinator side.
pub fn resolve(format: Format) -> Result<Box<dyn FormatRunner>> {
    match format {
        Format::CityParquet | Format::CityParquetHilbert => {
            Ok(Box::new(cityparquet::CityParquetRunner))
        }
        Format::CityJson => Ok(Box::new(cityjson::CityJsonRunner)),
        Format::CityJsonSeq => Ok(Box::new(cityjsonseq::CityJsonSeqRunner::plain())),
        Format::CityJsonSeqGz => Ok(Box::new(cityjsonseq::CityJsonSeqRunner::gzip())),
        Format::FlatCityBuf => Ok(Box::new(flatcitybuf::FlatCityBufRunner)),
        Format::CityGml => Ok(Box::new(citygml::CityGmlRunner)),
        Format::DuckDbParquet => bail!(
            "format 'duckdb-parquet' is a SQL-engine baseline driven by \
             benchmark/scripts/readbench_duckdb.sh, not this binary's --child path"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enumerates [`Format::ALL`] rather than a hand-written list, so a new
    /// variant cannot slip past this test unclassified.
    #[test]
    fn resolve_implemented_formats_succeed_and_others_error_cleanly() {
        for format in Format::ALL {
            let resolved = resolve(format);
            match format {
                // Implemented today.
                Format::CityGml
                | Format::CityParquet
                | Format::CityParquetHilbert
                | Format::CityJson
                | Format::CityJsonSeq
                | Format::CityJsonSeqGz
                | Format::FlatCityBuf => {
                    assert!(resolved.is_ok(), "{format} should resolve to a runner");
                }
                // Never a `--child` format at all.
                Format::DuckDbParquet => {
                    assert!(resolved.is_err(), "{format} is not a --child format");
                }
            }
        }
    }
}

#[cfg(test)]
mod attr_aggregates_tests {
    use super::AttrAggregates;

    #[test]
    fn integers_and_floats_fold_into_one_f64_accumulation() {
        let mut stats = AttrAggregates::EMPTY;
        for v in [
            serde_json::json!(3),
            serde_json::json!(-1.5),
            serde_json::json!(10),
        ] {
            stats.push(v.as_f64().unwrap());
        }
        assert_eq!(
            stats,
            AttrAggregates {
                min: -1.5,
                max: 10.0,
                sum: 11.5,
                count: 3
            }
        );
    }

    #[test]
    fn a_nan_does_not_panic() {
        let mut stats = AttrAggregates::EMPTY;
        stats.push(1.0);
        stats.push(f64::NAN);
        assert_eq!((stats.min, stats.max, stats.count), (1.0, 1.0, 2));
        assert!(stats.sum.is_nan());
    }
}
