//! The CityJSONSeq [`FormatRunner`]: full-parse baseline for a
//! CityJSONSeq stream (`cityjsonseq`, `.city.jsonl`). There is no index of any kind, so
//! EVERY scenario reads and JSON-parses the whole stream — that full-parse
//! cost is the honest, deliberate baseline this runner measures, not an
//! oversight.
//!
//! **Cross-format counting caveat — deliberately NOT papered over here (the
//! mirror image of [`super::cityparquet`]'s own caveat).**
//!
//! A CityJSONSeq feature line bundles one top-level CityObject (e.g. a
//! `Building`) together with all of its children (e.g. its `BuildingPart`s)
//! inline. That gives two different, equally legitimate "counting units":
//!
//! - [`Scenario::Count`] and [`Scenario::FullRead`] count top-level
//!   FEATURES (lines), because that is this format's natural unit — the
//!   delft fixture has 1115 features (one per `Building`), vs. CityParquet's
//!   own 2231 (one row per CityObject, parents AND children). This mirrors
//!   FlatCityBuf's own feature-level counting.
//! - [`Scenario::AttrFilter`], [`Scenario::AttrStats`] and
//!   [`Scenario::IdLookup`] instead iterate over CityOBJECTS — flattening
//!   every feature's `CityObjects` map (parents AND children) — so their
//!   `result_count` matches CityParquet's own object-level count EXACTLY on
//!   the same data (delft: `object_type == "BuildingPart"` -> 1116;
//!   `oorspronkelijkbouwjaar` numeric -> 1115). This is what makes these
//!   three scenarios meaningfully comparable across formats at all.
//! - [`Scenario::BBoxQuery`] is feature-level: each feature's bbox is the
//!   min/max over ALL of its (feature-local, transform-encoded) vertices,
//!   decoded via the stream header's `transform` — i.e. the union of every
//!   CityObject in that feature, not tested per-object.
//!
//! None of this is silently normalised to match another format; the
//! milestone's methodology doc is responsible for disclosing it alongside
//! the numbers.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use cityparquet::cjseq::{CityJSONFeature, CityObject, Transform};
use cityparquet::counting_store::CountingObjectStore;
use cityparquet::source::Source;
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

use super::{Answer, AttrAggregates, FormatRunner, IoStats, RunOutcome, Source as TransportSource};
use crate::scenario::{AttrPred, QueryParams, Scenario};

/// This runner's `--attr-column`/params error for a scenario missing a
/// required field — mirrors [`super::cityparquet`]'s own `require` helper.
///
/// `pub(super)` because [`super::cityjson`] reuses it verbatim: the two
/// JSON-shaped runners must never drift on what a scenario requires.
pub(super) fn require<'a, T>(opt: &'a Option<T>, flag: &str, scenario: Scenario) -> Result<&'a T> {
    opt.as_ref()
        .ok_or_else(|| anyhow!("scenario '{scenario}' requires --{flag}"))
}

/// The opened stream, behind the one `transform()`/`features()` surface the
/// scenario dispatch in [`run_scenario`] reads. Boxed: a [`Source`] is 1 KiB+,
/// dominated by its own parsed `CityJSON` header.
struct Backend(Box<Source>);

impl Backend {
    /// Opens `input`, refusing a CityGML document outright.
    ///
    /// The mirror image of [`super::citygml`]'s own sniff (`open_citygml`)
    /// and of [`super::cityjson`]'s (`Document::parse`), and it exists for
    /// the same reason: [`Source::open`] sniffs, and a `.gml` handed to it
    /// opens quite happily as CityGML — so this runner would parse XML and
    /// publish another format's cost under this format's name. That is not
    /// hypothetical: while `Format::CityJsonSeq` resolved to the `--input`
    /// itself, EVERY `.gml` dataset's `cityjsonseq` row was a CityGML
    /// measurement (~8x too slow), and nothing failed.
    ///
    fn open(input: &Path) -> Result<Self> {
        if cityparquet::citygml::sniff_citygml(input).is_some() {
            return Err(anyhow!(
                "{} is a CityGML document, not CityJSONSeq; --format cityjsonseq must never be \
                 pointed at CityGML, or the benchmark would report another format's cost under \
                 this format's name",
                input.display()
            ));
        }
        Ok(Backend(Box::new(
            Source::open(input).map_err(|e| anyhow!(e))?,
        )))
    }

    fn transform(&self) -> &Transform {
        &self.0.header().transform
    }

    fn features(&self) -> Result<Box<dyn Iterator<Item = Result<CityJSONFeature>> + '_>> {
        let iter = self.0.features().map_err(|e| anyhow!(e))?;
        Ok(Box::new(iter.map(|r| r.map_err(|e| anyhow!(e)))))
    }
}

/// This runner's own attribute-predicate evaluation — the CityJSONSeq
/// analogue of `cityparquet::query::evaluate_attr_predicate`, operating on a
/// raw `serde_json::Value` cell instead of an Arrow column, since a
/// CityJSONSeq attribute has no columnar type to dispatch on. A missing or
/// JSON-`null` `value` never matches any variant.
///
/// `pub(super)` because [`super::cityjson`] reuses it: a plain CityJSON
/// document and a CityJSONSeq stream carry the very same
/// `serde_json::Value` attribute cells, so sharing this is what makes the
/// two runners' `attr-filter` answers comparable by construction rather
/// than by coincidence (see `tests/attr_consistency.rs`).
pub(super) fn matches_predicate(value: Option<&serde_json::Value>, pred: &AttrPred) -> bool {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return false;
    };
    match pred {
        AttrPred::Eq(want) => {
            if let Some(want_str) = want.as_str() {
                value.as_str() == Some(want_str)
            } else if let Some(want_num) = want.as_f64() {
                value.as_f64() == Some(want_num)
            } else {
                false
            }
        }
        AttrPred::Ge(bound) => value.as_f64().is_some_and(|v| v >= *bound),
        AttrPred::Le(bound) => value.as_f64().is_some_and(|v| v <= *bound),
        AttrPred::Range(lo, hi) => value.as_f64().is_some_and(|v| v >= *lo && v <= *hi),
    }
}

/// `column`'s value on `co`: the reserved `object_type` column reads
/// `co.thetype` (CityParquet's own reserved column, backed by the CityJSON
/// object's `"type"` field, never the `attributes` map); every other column
/// name is looked up in `co.attributes` (a JSON attribute name -> value
/// map), with a JSON-`null` entry treated the same as an absent one.
///
/// `pub(super)` for the same reason as [`matches_predicate`]: shared with
/// [`super::cityjson`] so both JSON runners resolve a column name
/// identically.
pub(super) fn column_value(co: &CityObject, column: &str) -> Option<serde_json::Value> {
    if column == "object_type" {
        return Some(serde_json::Value::String(co.thetype.clone()));
    }
    co.attributes
        .as_ref()?
        .get(column)
        .filter(|v| !v.is_null())
        .cloned()
}

/// Folds `co`'s value for `column` into `stats` when it is a JSON number —
/// integer or float alike, via `as_f64` — and leaves `stats` untouched
/// otherwise (absent, `null`, a string, a boolean, or the reserved
/// `object_type` string).
///
/// `pub(super)` for the same reason as [`column_value`]: [`super::cityjson`]
/// and [`super::citygml`] aggregate through this very function, so the three
/// parsing runners' `attr-stats` answers agree by construction.
pub(super) fn push_numeric(stats: &mut AttrAggregates, co: &CityObject, column: &str) {
    if let Some(value) = column_value(co, column).and_then(|v| v.as_f64()) {
        stats.push(value);
    }
}

/// Total leaf (numeric) values in `value`'s nested-array tree — a
/// geometry-type-agnostic stand-in for "how much boundary work would
/// decoding this geometry take", used only to force [`Scenario::FullRead`]
/// to actually traverse every geometry's `boundaries`, not merely to have
/// deserialized them into a [`serde_json::Value`] tree. The result itself is
/// discarded — [`Scenario::FullRead`]'s returned metric stays feature-level,
/// per this module's own counting-unit ruling above.
///
/// `pub(super)` because [`super::citygml`] reuses it: that runner's
/// [`Scenario::FullRead`] is deliberately the SAME operation as this one's, so
/// the two rows stay comparable — sharing the traversal is what makes that
/// true by construction rather than by two copies happening to agree.
pub(super) fn count_boundary_leaves(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::Array(items) => items.iter().map(count_boundary_leaves).sum(),
        serde_json::Value::Number(_) => 1,
        _ => 0,
    }
}

/// A feature's overall bbox: the min/max over every one of its
/// (feature-local, integer, transform-encoded) vertices, decoded via the
/// stream header's `transform` exactly as CityJSON's own
/// `scale`/`translate` convention specifies. `None` if the feature carries
/// no vertices at all (never true for a real geometry-bearing feature, but
/// guards against a division-by-nothing rather than panicking).
///
/// `pub(super)` because [`super::citygml`] reuses it: a CityGML feature is
/// built with feature-local, transform-quantised vertices exactly like a
/// CityJSONSeq one, so both runners' `bbox-query` windows must mean the same
/// thing.
pub(super) fn feature_bbox(
    feature: &CityJSONFeature,
    transform: &Transform,
) -> Option<([f64; 3], [f64; 3])> {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut any = false;
    for vertex in &feature.vertices {
        any = true;
        for axis in 0..3 {
            let real = vertex[axis] as f64 * transform.scale[axis] + transform.translate[axis];
            min[axis] = min[axis].min(real);
            max[axis] = max[axis].max(real);
        }
    }
    any.then_some((min, max))
}

/// Axis-aligned 3D interval-overlap test, identical in spirit to
/// `cityparquet::reader::box_intersects_query` (that function is
/// `pub(crate)` inside the `cityparquet` crate, so this runner keeps its own
/// copy rather than depending on a private item). Shared with
/// [`super::cityjson`] so the two JSON runners' `bbox-query` windows mean
/// exactly the same thing.
pub(super) fn intersects(min: [f64; 3], max: [f64; 3], query: &[f64; 6]) -> bool {
    for axis in 0..3 {
        if max[axis] < query[axis] || min[axis] > query[axis + 3] {
            return false;
        }
    }
    true
}

/// The CityJSONSeq runner (`cityjsonseq`, `.city.jsonl`).
pub struct CityJsonSeqRunner;

/// The scenario-dispatch body shared by both the local and (future) HTTP
/// branches of [`FormatRunner::run`]: everything below `Backend::open`
/// (which itself only differs in WHERE the bytes come from) is
/// transport-independent.
fn run_scenario(backend: &Backend, scenario: Scenario, params: &QueryParams) -> Result<Answer> {
    match scenario {
        Scenario::Count => {
            let mut feature_count = 0u64;
            for feature in backend.features()? {
                feature?;
                feature_count += 1;
            }
            Ok(feature_count.into())
        }
        Scenario::FullRead => {
            let mut feature_count = 0u64;
            let mut boundary_work = 0u64;
            for feature in backend.features()? {
                let feature = feature?;
                feature_count += 1;
                for co in feature.city_objects.values() {
                    if let Some(geoms) = &co.geometry {
                        for geom in geoms {
                            boundary_work += count_boundary_leaves(&geom.boundaries);
                        }
                    }
                }
            }
            // `boundary_work` is computed purely to force full geometry
            // traversal (the "full read" cost); the returned metric
            // stays feature-level, per this module's own doc comment.
            // `black_box` rather than `let _ =`, so the traversal cannot be
            // optimised away as dead code — the same guarantee
            // [`super::cityjson`]'s own `FullRead` needs for its coordinate
            // resolution.
            std::hint::black_box(boundary_work);
            Ok(feature_count.into())
        }
        Scenario::BBoxQuery => {
            let query_bbox = *require(&params.bbox, "bbox", scenario)?;
            let transform = backend.transform().clone();
            let mut matched = 0u64;
            for feature in backend.features()? {
                let feature = feature?;
                if let Some((min, max)) = feature_bbox(&feature, &transform)
                    && intersects(min, max, &query_bbox)
                {
                    matched += 1;
                }
            }
            Ok(matched.into())
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            let mut matched = 0u64;
            for feature in backend.features()? {
                let feature = feature?;
                for co in feature.city_objects.values() {
                    if matches_predicate(column_value(co, column).as_ref(), pred) {
                        matched += 1;
                    }
                }
            }
            Ok(matched.into())
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut stats = AttrAggregates::EMPTY;
            for feature in backend.features()? {
                let feature = feature?;
                for co in feature.city_objects.values() {
                    push_numeric(&mut stats, co, column);
                }
            }
            // `black_box`, as `FullRead` pins its traversal: the aggregates
            // are the scenario's answer, so the arithmetic must not be
            // optimised down to the count.
            Ok(std::hint::black_box(stats).into())
        }
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            for feature in backend.features()? {
                let feature = feature?;
                if feature.city_objects.contains_key(id) {
                    return Ok(1u64.into());
                }
            }
            Ok(0u64.into())
        }
        Scenario::FeatureLookup => bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY),
    }
}

/// The HTTP-transport body of [`CityJsonSeqRunner::run`]: a single
/// whole-object GET (via a [`CountingObjectStore`]-wrapped
/// `object_store::http::HttpStore` — exactly 1 request, the whole file's
/// bytes, by construction) written to a [`tempfile::NamedTempFile`], then
/// handed to the existing, unchanged [`Backend::open`]/[`run_scenario`]
/// local parsing — no in-memory CityJSONSeq parser duplicated for this
/// transport.
async fn run_http(
    base_url: &str,
    key: &str,
    scenario: Scenario,
    params: &QueryParams,
) -> Result<RunOutcome> {
    // `with_allow_http(true)` is required for a plain `http://` target (the
    // in-test Range server this crate's own tests point at); it does not
    // disable or otherwise affect `https://` targets (real S3/R2 buckets).
    let store = HttpBuilder::new()
        .with_url(base_url)
        .with_client_options(object_store::ClientOptions::new().with_allow_http(true))
        .build()?;
    let counting = CountingObjectStore::new(store);

    let obj_path = ObjectPath::from(key);
    let bytes = counting
        .get(&obj_path)
        .await
        .with_context(|| format!("GET {key}"))?
        .bytes()
        .await
        .with_context(|| format!("reading body of {key}"))?;

    let tmp =
        tempfile::NamedTempFile::new().context("creating a tempfile for the whole-object GET")?;
    std::fs::write(tmp.path(), &bytes)
        .with_context(|| format!("writing {} bytes to {}", bytes.len(), tmp.path().display()))?;
    let stats = counting.tally();

    let backend = Backend::open(tmp.path())?;
    let answer = run_scenario(&backend, scenario, params)?;
    Ok(RunOutcome {
        result_count: answer.result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
        lookup: None,
        attr_stats: answer.attr_stats,
        returned: answer.returned,
    })
}

impl FormatRunner for CityJsonSeqRunner {
    fn run(
        &self,
        source: &TransportSource,
        scenario: Scenario,
        params: &QueryParams,
    ) -> Result<RunOutcome> {
        let (base_url, key) = match source {
            TransportSource::Local(path) => {
                let backend = Backend::open(path)?;
                let answer = run_scenario(&backend, scenario, params)?;
                return Ok(RunOutcome {
                    result_count: answer.result_count,
                    io: None,
                    lookup: None,
                    attr_stats: answer.attr_stats,
                    returned: answer.returned,
                });
            }
            TransportSource::Http { base_url, key } => (base_url, key),
        };

        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, scenario, params))
    }
}
