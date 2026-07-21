//! The CityParquet [`FormatRunner`]: maps each [`Scenario`] onto the
//! matching `cityparquet::query::*` primitive.
//!
//! **Cross-format counting caveat — deliberately NOT papered over here.**
//! CityParquet's `Count`/`FullRead` count ONE ROW PER CITYOBJECT: both
//! parents AND children get their own row (e.g. the delft fixture has 2231
//! rows; the 60-object rural benchmark tile has 60). FlatCityBuf and
//! CityJSONSeq instead count top-level FEATURES only (the same rural tile
//! has 30 top-level features, excluding their children as separate counted
//! units) — a genuine semantic difference in what "count" means per format,
//! not a bug to reconcile inside this runner. This runner always reports
//! CityParquet's own natural object-row count; the coordinator/methodology
//! doc (later tasks) are responsible for disclosing the difference
//! alongside the numbers, never silently normalising one format's count to
//! match another's.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use cityparquet::query::{self, AttrPredicate};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::stac::properties::PackageTables;
use cityparquet_schema::CityParquetMetadata;

use super::FormatRunner;
use crate::scenario::{AttrPred, QueryParams, Scenario};

/// Locates the main CityObject table inside a CityParquet package:
///
/// - If `input` is itself a file, it IS the table.
/// - If `input` is a directory without a `metadata.json` manifest, this is
///   not a package this crate produced — an error rather than a guess.
/// - If `input` is a directory WITH a manifest listing exactly one table,
///   uses that one (every by-type package that came from a single-family
///   dataset — e.g. delft, all Building/BuildingPart — lists exactly one).
///   A manifest listing more than one table (a multi-family by-type
///   package, e.g. the 10-family `lod3_railway` fixture) is rejected: this
///   runner only ever queries a single Parquet file, so a package split
///   across several family tables has no single file that holds the whole
///   dataset — out of scope here rather than silently reading only one
///   family's rows.
fn locate_main_table(input: &Path) -> Result<PathBuf> {
    if input.is_file() {
        return Ok(input.to_path_buf());
    }
    if !input.is_dir() {
        bail!(
            "input path '{}' is neither a file nor a directory",
            input.display()
        );
    }

    let manifest_path = input.join("metadata.json");
    if !manifest_path.exists() {
        bail!(
            "no metadata.json manifest at {}; not a CityParquet package",
            input.display()
        );
    }

    // `PackageTables::open` is the sole reader of `metadata.json` here; it
    // already rejects an empty or duplicate-naming manifest. The
    // "exactly one object table" requirement below is this runner's own —
    // it only ever queries a single Parquet file (see this fn's doc
    // comment).
    let tables = PackageTables::open(input)
        .with_context(|| format!("reading {}", manifest_path.display()))?;

    match tables.tables.as_slice() {
        [only] => Ok(only.clone()),
        many => {
            let names: Vec<&str> = many
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect();
            bail!(
                "package at {} has {} tables ({names:?}); the read-benchmark only \
                 supports single-table (single-family) packages, not multi-table \
                 by-type packages",
                input.display(),
                many.len(),
            )
        }
    }
}

/// This runner's `--attr-column`/params error for a scenario missing a
/// required field — every scenario branch in [`CityParquetRunner::run`]
/// that reads an `Option` field routes its `None` case through here so the
/// child process exits with a clear message instead of a panic.
fn require<'a, T>(opt: &'a Option<T>, flag: &str, scenario: Scenario) -> Result<&'a T> {
    opt.as_ref()
        .ok_or_else(|| anyhow::anyhow!("scenario '{scenario}' requires --{flag}"))
}

/// Maps a CLI-level [`AttrPred`] onto `cityparquet::query`'s own
/// `AttrPredicate` — the one place that conversion happens, so
/// `scenario.rs` never needs to depend on the `cityparquet` crate.
fn to_query_predicate(pred: &AttrPred) -> AttrPredicate {
    match pred {
        AttrPred::Eq(v) => AttrPredicate::Eq(v.clone()),
        AttrPred::Ge(bound) => AttrPredicate::Ge(*bound),
        AttrPred::Le(bound) => AttrPredicate::Le(*bound),
        AttrPred::Range(lo, hi) => AttrPredicate::Range(*lo, *hi),
    }
}

/// Opens `table` once, just far enough to read its embedded CityParquet
/// key-value metadata — the `meta` argument `query::full_read`/
/// `query::id_lookup` need to decode geometry/attributes.
fn open_metadata(table: &Path) -> Result<CityParquetMetadata> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet metadata from {}", table.display()))?;
    Ok(builder.cityparquet_metadata()?)
}

/// The CityParquet backend: every scenario locates `input`'s main table
/// (see [`locate_main_table`]) and calls straight into
/// `cityparquet::query`, so this file adds no query logic of its own — it
/// is purely the [`Scenario`] -> primitive dispatch, plus package/CLI
/// plumbing.
pub struct CityParquetRunner;

impl FormatRunner for CityParquetRunner {
    fn run(&self, input: &Path, scenario: Scenario, params: &QueryParams) -> Result<u64> {
        let table = locate_main_table(input)?;
        match scenario {
            Scenario::Count => Ok(query::count(&table)?),
            Scenario::FullRead => {
                let meta = open_metadata(&table)?;
                Ok(query::full_read(&table, &meta)?.feature_count)
            }
            Scenario::BBoxQuery => {
                let bbox = *require(&params.bbox, "bbox", scenario)?;
                Ok(query::bbox_query(&table, bbox)?.ids.len() as u64)
            }
            Scenario::AttrFilter => {
                let column = require(&params.attr_column, "attr-column", scenario)?;
                let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
                Ok(query::attr_filter(
                    &table,
                    column,
                    &to_query_predicate(pred),
                )?)
            }
            Scenario::AttrStats => {
                let column = require(&params.attr_column, "attr-column", scenario)?;
                Ok(query::attr_stats(&table, column)?.count)
            }
            Scenario::IdLookup => {
                let id = require(&params.target_id, "target-id", scenario)?;
                let meta = open_metadata(&table)?;
                Ok(query::id_lookup(&table, &meta, id)?.is_some() as u64)
            }
            Scenario::Project => {
                let column = require(&params.attr_column, "attr-column", scenario)?;
                Ok(query::project_column(&table, column)?)
            }
        }
    }
}
