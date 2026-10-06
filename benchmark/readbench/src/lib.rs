//! The read benchmark's shared vocabulary.
//!
//! The benchmark itself is a binary (`src/main.rs`): a coordinator that
//! drives a whole (format x scenario) matrix, and a `--child` worker it
//! spawns once per measurement. This library holds the parts that the
//! binary, the coordinator, and the integration tests must all spell
//! identically: [`format`], the set of formats measured, [`naming`], the
//! input-extension convention every artefact path is derived through, and
//! [`params`], the query parameters every measurement is driven with.

pub mod bloom_columns;
pub mod format;
pub mod isolation;
pub mod lod;
pub mod naming;
pub mod params;
pub mod sampling;
pub mod seq_order;
pub mod slice;
pub mod stats;
pub mod variant_package;

/// The STAC `datetime` of every benchmark package: the day the benchmark
/// corpus was fixed. The prepare script passes the same value to
/// `cityparquet convert --datetime` (`CORPUS_DATETIME` there), so a package
/// is the same bytes whichever of the two builds it.
pub const CORPUS_DATETIME: &str = "2026-10-05T00:00:00Z";
