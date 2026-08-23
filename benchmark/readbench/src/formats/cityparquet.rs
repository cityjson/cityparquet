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
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use object_store::ObjectStore;
use object_store::ObjectStoreExt;
use object_store::http::{HttpBuilder, HttpStore};
use object_store::path::Path as ObjectPath;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::async_reader::ParquetObjectReader;

use cityparquet::counting_store::CountingObjectStore;
use cityparquet::query::{self, AttrPredicate};
use cityparquet::query_async;
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::stac::properties::{PackageTables, table_names_from_manifest_bytes};
use cityparquet_schema::CityMetadata;

use super::{FormatRunner, IoStats, RunOutcome, Source};
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
/// required field — every scenario branch in [`ScenarioPlan::resolve`]
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

/// One scenario's fully-resolved parameters — the single place the
/// `--bbox`/`--attr-column`/`--target-id` requirements are checked and the
/// CLI predicate is converted, shared by the local (sync) and HTTP (async)
/// dispatch arms so the two transports can never drift on what a scenario
/// requires (review P3).
enum ScenarioPlan<'a> {
    Count,
    FullRead,
    BBoxQuery([f64; 6]),
    AttrFilter {
        column: &'a str,
        pred: AttrPredicate,
    },
    AttrStats {
        column: &'a str,
    },
    IdLookup {
        id: &'a str,
    },
    Project {
        column: &'a str,
    },
}

impl<'a> ScenarioPlan<'a> {
    fn resolve(scenario: Scenario, params: &'a QueryParams) -> Result<Self> {
        Ok(match scenario {
            Scenario::Count => Self::Count,
            Scenario::FullRead => Self::FullRead,
            Scenario::BBoxQuery => Self::BBoxQuery(*require(&params.bbox, "bbox", scenario)?),
            Scenario::AttrFilter => Self::AttrFilter {
                column: require(&params.attr_column, "attr-column", scenario)?.as_str(),
                pred: to_query_predicate(require(
                    &params.attr_pred,
                    "attr-eq/--attr-ge/--attr-le",
                    scenario,
                )?),
            },
            Scenario::AttrStats => Self::AttrStats {
                column: require(&params.attr_column, "attr-column", scenario)?.as_str(),
            },
            Scenario::IdLookup => Self::IdLookup {
                id: require(&params.target_id, "target-id", scenario)?.as_str(),
            },
            Scenario::Project => Self::Project {
                column: require(&params.attr_column, "attr-column", scenario)?.as_str(),
            },
        })
    }
}

/// Opens `table` once, just far enough to read its embedded CityParquet
/// key-value metadata — the `meta` argument `query::full_read`/
/// `query::id_lookup` need to decode geometry/attributes.
fn open_metadata(table: &Path) -> Result<CityMetadata> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet metadata from {}", table.display()))?;
    Ok(builder.cityparquet_metadata()?)
}

/// Resolves `base_url`/`key`'s single main table over HTTP: range-fetches
/// `<key>/metadata.json` (the same STAC Item the local [`locate_main_table`]
/// reads via [`PackageTables::open`]), rejects a multi-table manifest
/// (mirrors the local runner's own single-family restriction), and returns
/// a ready-to-query `(CountingObjectStore-wrapped store, table object path)`
/// pair.
async fn resolve_http_main_table(
    base_url: &str,
    key: &str,
) -> Result<(Arc<CountingObjectStore<HttpStore>>, ObjectPath)> {
    // `with_allow_http(true)` is required for a plain `http://` target (the
    // in-test Range server this crate's own tests point at); it does not
    // disable or otherwise affect `https://` targets (real S3/R2 buckets),
    // so it is set unconditionally rather than sniffed from `base_url`.
    let store = HttpBuilder::new()
        .with_url(base_url)
        .with_client_options(object_store::ClientOptions::new().with_allow_http(true))
        .build()?;
    let counting = Arc::new(CountingObjectStore::new(store));

    let manifest_path = ObjectPath::from(format!("{key}/metadata.json"));
    let manifest_bytes = counting.get(&manifest_path).await?.bytes().await?;
    let tables = table_names_from_manifest_bytes(&manifest_bytes)?;
    let [only] = tables.as_slice() else {
        bail!(
            "package at '{key}' has {} tables; the read-benchmark only supports \
             single-table (single-family) packages over HTTP",
            tables.len()
        );
    };
    let table_path = ObjectPath::from(format!("{key}/{only}"));
    Ok((counting, table_path))
}

/// Opens `table_path` (over `store`) just far enough to read its embedded
/// CityParquet key-value metadata — the async, HTTP-sourced mirror of
/// [`open_metadata`].
async fn open_metadata_http(
    store: Arc<dyn ObjectStore>,
    table_path: &ObjectPath,
) -> Result<CityMetadata> {
    let reader = ParquetObjectReader::new(store, table_path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader).await?;
    Ok(CityParquetReaderBuilder::cityparquet_metadata(&builder)?)
}

/// The HTTP-transport body of [`CityParquetRunner::run`]: resolves the
/// package's main table, dispatches `scenario` onto the matching
/// `cityparquet::query_async::*_async` primitive (the exact async mirror of
/// the local branch's own `cityparquet::query::*` call), and reports the
/// `CountingObjectStore`'s tally as [`IoStats`].
async fn run_http(
    base_url: &str,
    key: &str,
    scenario: Scenario,
    params: &QueryParams,
) -> Result<RunOutcome> {
    let (store, table_path) = resolve_http_main_table(base_url, key).await?;
    let dyn_store = || Arc::clone(&store) as Arc<dyn ObjectStore>;

    let plan = ScenarioPlan::resolve(scenario, params)?;
    let result_count = match plan {
        ScenarioPlan::Count => query_async::count_async(dyn_store(), &table_path).await?,
        ScenarioPlan::FullRead => {
            let meta = open_metadata_http(dyn_store(), &table_path).await?;
            query_async::full_read_async(dyn_store(), &table_path, &meta)
                .await?
                .feature_count
        }
        ScenarioPlan::BBoxQuery(bbox) => {
            query_async::bbox_query_async(dyn_store(), &table_path, bbox)
                .await?
                .ids
                .len() as u64
        }
        ScenarioPlan::AttrFilter { column, pred } => {
            query_async::attr_filter_async(dyn_store(), &table_path, column, &pred).await?
        }
        ScenarioPlan::AttrStats { column } => {
            query_async::attr_stats_async(dyn_store(), &table_path, column)
                .await?
                .count
        }
        ScenarioPlan::IdLookup { id } => {
            let meta = open_metadata_http(dyn_store(), &table_path).await?;
            query_async::id_lookup_async(dyn_store(), &table_path, &meta, id)
                .await?
                .is_some() as u64
        }
        ScenarioPlan::Project { column } => {
            query_async::project_column_async(dyn_store(), &table_path, column).await?
        }
    };

    let stats = store.tally();
    Ok(RunOutcome {
        result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
    })
}

/// The CityParquet backend: every scenario locates `input`'s main table
/// (see [`locate_main_table`]) and calls straight into
/// `cityparquet::query`, so this file adds no query logic of its own — it
/// is purely the [`Scenario`] -> primitive dispatch, plus package/CLI
/// plumbing.
pub struct CityParquetRunner;

impl FormatRunner for CityParquetRunner {
    fn run(&self, source: &Source, scenario: Scenario, params: &QueryParams) -> Result<RunOutcome> {
        let (base_url, key) = match source {
            Source::Local(path) => {
                let table = locate_main_table(path)?;
                let plan = ScenarioPlan::resolve(scenario, params)?;
                let result_count = match plan {
                    ScenarioPlan::Count => query::count(&table)?,
                    ScenarioPlan::FullRead => {
                        let meta = open_metadata(&table)?;
                        query::full_read(&table, &meta)?.feature_count
                    }
                    ScenarioPlan::BBoxQuery(bbox) => {
                        query::bbox_query(&table, bbox)?.ids.len() as u64
                    }
                    ScenarioPlan::AttrFilter { column, pred } => {
                        query::attr_filter(&table, column, &pred)?
                    }
                    ScenarioPlan::AttrStats { column } => query::attr_stats(&table, column)?.count,
                    ScenarioPlan::IdLookup { id } => {
                        let meta = open_metadata(&table)?;
                        query::id_lookup(&table, &meta, id)?.is_some() as u64
                    }
                    ScenarioPlan::Project { column } => query::project_column(&table, column)?,
                };
                return Ok(RunOutcome {
                    result_count,
                    io: None,
                });
            }
            Source::Http { base_url, key } => (base_url, key),
        };

        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, scenario, params))
    }
}
