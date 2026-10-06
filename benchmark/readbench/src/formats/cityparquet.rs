//! The CityParquet [`FormatRunner`]: maps each [`Scenario`] onto the
//! matching `cityparquet::query::*` primitive (or its `query_async` mirror
//! over HTTP), following the return rule in [`super::returned`]:
//!
//! - read all: `full_read_visit` visits every field of every row in place;
//! - spatial window: `bbox_query_geometry` returns the matching identifiers
//!   and walks each one's highest-LoD WKB in place;
//! - attribute filter: `attr_filter_ids` returns the matching identifiers
//!   (bloom, then min/max statistics, prune the row groups);
//! - identifier and feature lookup: `id_lookup_visit`/`feature_lookup_visit`
//!   visit every field of the matching rows (bloom, then statistics, prune);
//! - `count` and `attr-stats`: the format's own count and aggregates.
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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use object_store::ObjectStore;
use object_store::ObjectStoreExt;
use object_store::http::{HttpBuilder, HttpStore};
use object_store::path::Path as ObjectPath;

use cityparquet::counting_store::CountingObjectStore;
use cityparquet::query::{self, AttrPredicate, BBoxGeometryResult, LookupStats};
use cityparquet::query_async;
use cityparquet::stac::properties::{PackageTables, table_names_from_manifest_bytes};
use cityparquet::visit::VisitTotals;

use super::returned::{ComparableTotals, IdDigest, Returned, ReturnedGeometry};
use super::{AttrAggregates, FormatRunner, IoStats, LookupCounters, RunOutcome, Source};
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

fn counters(stats: LookupStats) -> LookupCounters {
    LookupCounters {
        row_groups_total: stats.row_groups_total as u64,
        bloom_pruned: stats.bloom_pruned as u64,
        filter_bytes: stats.filter_bytes,
        stats_pruned: stats.stats_pruned as u64,
    }
}

/// One scenario's answer before the transport adds [`IoStats`]: the
/// `result_count`, the lookup counters, the aggregates and what was
/// returned.
#[derive(Default)]
struct Reply {
    result_count: u64,
    lookup: Option<LookupCounters>,
    attr_stats: Option<AttrAggregates>,
    returned: Returned,
}

/// Read all and the lookups: every field of every read row was visited by
/// the library; the comparable totals are what is returned.
fn visited(totals: &VisitTotals, stats: Option<LookupStats>) -> Reply {
    Reply {
        result_count: totals.objects,
        lookup: stats.map(counters),
        returned: Returned {
            totals: Some(ComparableTotals::from(totals)),
            ..Returned::default()
        },
        ..Reply::default()
    }
}

/// The spatial window: the matching objects' identifiers and the highest-LoD
/// geometry the library walked in place for each.
fn window(result: &BBoxGeometryResult) -> Reply {
    Reply {
        result_count: result.ids.len() as u64,
        returned: Returned {
            ids: Some(IdDigest::of(result.ids.iter().map(String::as_str))),
            geometry: Some(ReturnedGeometry {
                geometries: result.geometries,
                extent: result.extent,
            }),
            ..Returned::default()
        },
        ..Reply::default()
    }
}

/// The attribute filter: the matching objects' identifiers.
fn matched(ids: &[String]) -> Reply {
    Reply {
        result_count: ids.len() as u64,
        returned: Returned {
            ids: Some(IdDigest::of(ids.iter().map(String::as_str))),
            ..Returned::default()
        },
        ..Reply::default()
    }
}

/// The aggregates of an `attr-stats` run.
fn stats_reply(stats: AttrAggregates) -> Reply {
    Reply {
        result_count: stats.count,
        attr_stats: Some(stats),
        ..Reply::default()
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
    FeatureLookup {
        feature_id: &'a str,
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
            Scenario::FeatureLookup => Self::FeatureLookup {
                feature_id: require(&params.target_feature_id, "target-feature-id", scenario)?
                    .as_str(),
            },
        })
    }
}

/// The library's [`query::AttrStats`] as the runner reports it. The
/// aggregation itself stays the format's own mechanism — min and max from
/// column-chunk statistics, sum and count from a one-column projected scan —
/// and is not re-derived here.
fn aggregates(stats: query::AttrStats) -> AttrAggregates {
    AttrAggregates {
        min: stats.min,
        max: stats.max,
        sum: stats.sum,
        count: stats.count,
    }
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
    let reply = match plan {
        ScenarioPlan::Count => Reply {
            result_count: query_async::count_async(dyn_store(), &table_path).await?,
            ..Reply::default()
        },
        ScenarioPlan::FullRead => visited(
            &query_async::full_read_visit_async(dyn_store(), &table_path).await?,
            None,
        ),
        ScenarioPlan::BBoxQuery(bbox) => {
            window(&query_async::bbox_query_geometry_async(dyn_store(), &table_path, bbox).await?)
        }
        ScenarioPlan::AttrFilter { column, pred } => {
            let (ids, _) =
                query_async::attr_filter_ids_async(dyn_store(), &table_path, column, &pred).await?;
            matched(&ids)
        }
        ScenarioPlan::AttrStats { column } => stats_reply(aggregates(
            query_async::attr_stats_async(dyn_store(), &table_path, column).await?,
        )),
        ScenarioPlan::IdLookup { id } => {
            let (totals, stats) =
                query_async::id_lookup_visit_async(dyn_store(), &table_path, id).await?;
            visited(&totals, Some(stats))
        }
        ScenarioPlan::FeatureLookup { feature_id } => {
            let (totals, stats) =
                query_async::feature_lookup_visit_async(dyn_store(), &table_path, feature_id)
                    .await?;
            visited(&totals, Some(stats))
        }
    };

    let stats = store.tally();
    Ok(RunOutcome {
        result_count: reply.result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
        lookup: reply.lookup,
        attr_stats: reply.attr_stats,
        returned: reply.returned,
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
                let reply = match plan {
                    ScenarioPlan::Count => Reply {
                        result_count: query::count(&table)?,
                        ..Reply::default()
                    },
                    ScenarioPlan::FullRead => visited(&query::full_read_visit(&table)?, None),
                    ScenarioPlan::BBoxQuery(bbox) => {
                        window(&query::bbox_query_geometry(&table, bbox)?)
                    }
                    ScenarioPlan::AttrFilter { column, pred } => {
                        matched(&query::attr_filter_ids(&table, column, &pred)?.0)
                    }
                    ScenarioPlan::AttrStats { column } => {
                        stats_reply(aggregates(query::attr_stats(&table, column)?))
                    }
                    ScenarioPlan::IdLookup { id } => {
                        let (totals, stats) = query::id_lookup_visit(&table, id)?;
                        visited(&totals, Some(stats))
                    }
                    ScenarioPlan::FeatureLookup { feature_id } => {
                        let (totals, stats) = query::feature_lookup_visit(&table, feature_id)?;
                        visited(&totals, Some(stats))
                    }
                };
                return Ok(RunOutcome {
                    result_count: reply.result_count,
                    io: None,
                    lookup: reply.lookup,
                    attr_stats: reply.attr_stats,
                    returned: reply.returned,
                });
            }
            Source::Http { base_url, key } => (base_url, key),
        };

        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, scenario, params))
    }
}
