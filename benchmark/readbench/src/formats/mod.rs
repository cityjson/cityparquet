//! The [`FormatRunner`] trait every per-format read-benchmark backend
//! implements, plus [`resolve`] — the `--format <name>` dispatch the
//! `--child` process (`main.rs`) uses.

pub mod citygml;
pub mod cityjson;
pub mod cityjsonseq;
pub mod cityparquet;
mod cjvisit;
pub mod flatcitybuf;
pub mod returned;

use std::path::PathBuf;

use anyhow::Result;
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
    /// What the run returned — identifier digest, comparable totals,
    /// returned geometry — reported after the timed line (see
    /// [`returned`]).
    pub returned: returned::Returned,
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
    pub returned: returned::Returned,
}

impl Answer {
    /// An answer carrying the returned identifiers (and, for the spatial
    /// window, the returned geometry); `result_count` is the identifier count.
    pub(crate) fn returning(
        ids: returned::IdDigest,
        geometry: Option<returned::ReturnedGeometry>,
    ) -> Self {
        Self {
            result_count: ids.count,
            attr_stats: None,
            returned: returned::Returned {
                ids: Some(ids),
                totals: None,
                geometry,
            },
        }
    }

    /// An answer carrying the comparable totals of what was read, with
    /// `result_count` as given.
    pub(crate) fn reading(result_count: u64, totals: returned::ComparableTotals) -> Self {
        Self {
            result_count,
            attr_stats: None,
            returned: returned::Returned {
                ids: None,
                totals: Some(totals),
                geometry: None,
            },
        }
    }
}

impl From<u64> for Answer {
    fn from(result_count: u64) -> Self {
        Self {
            result_count,
            attr_stats: None,
            returned: returned::Returned::default(),
        }
    }
}

impl From<AttrAggregates> for Answer {
    fn from(stats: AttrAggregates) -> Self {
        Self {
            result_count: stats.count,
            attr_stats: Some(stats),
            returned: returned::Returned::default(),
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
    /// Row groups the statistics (min/max) pruned after the bloom filters
    /// (`LookupStats::stats_pruned`).
    pub stats_pruned: u64,
}

/// A child reports its [`LookupCounters`] on stderr, after its timed stdout
/// line, as `<marker> <row_groups_total> <bloom_pruned> <filter_bytes>
/// <stats_pruned>`, so
/// the timed stdout protocol keeps its shape.
pub const LOOKUP_STATS_MARKER: &str = "cityparquet-readbench: lookup-stats";

/// Every non-CityParquet runner's answer to [`Scenario::FeatureLookup`].
pub const FEATURE_LOOKUP_CITYPARQUET_ONLY: &str =
    "scenario 'feature-lookup' is measured for CityParquet only";

/// One format's read-benchmark backend: runs exactly one [`Scenario`]
/// against `source` (a format-specific location — a CityParquet package
/// directory or its main table file, a `.city.jsonl` file, or a
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
/// [`Format::CityJson`] is NOT an alias for [`Format::CityJsonSeq`]: a plain
/// whole-document `.city.json` parses as one JSON object with a shared
/// document-level `vertices` array, where a Seq stream is line-oriented with
/// feature-local vertices — different parse shape, different counting grain,
/// so its own runner (see [`cityjson`]'s module doc).
///
/// [`Format::CityGml`] has its own runner (see [`citygml`]'s module doc): the
/// format the source data ships in, read with this repository's own CityGML
/// 2.0 reader, with no index and therefore a full parse per scenario.
pub fn resolve(format: Format) -> Result<Box<dyn FormatRunner>> {
    match format {
        Format::CityParquet => Ok(Box::new(cityparquet::CityParquetRunner)),
        Format::CityJson => Ok(Box::new(cityjson::CityJsonRunner)),
        Format::CityJsonSeq => Ok(Box::new(cityjsonseq::CityJsonSeqRunner)),
        Format::FlatCityBuf => Ok(Box::new(flatcitybuf::FlatCityBufRunner)),
        Format::CityGml => Ok(Box::new(citygml::CityGmlRunner)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every format in [`Format::ALL`] resolves to a runner.
    #[test]
    fn every_format_resolves_to_a_runner() {
        for format in Format::ALL {
            assert!(
                resolve(format).is_ok(),
                "{format} should resolve to a runner"
            );
        }
    }
}
