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
//!   FlatCityBuf's own feature-level counting. Read all reads every field
//!   of every CityObject natively ([`super::cjvisit::visit_object`]) and
//!   reports the object-level comparable totals.
//! - [`Scenario::AttrFilter`], [`Scenario::AttrStats`] and
//!   [`Scenario::IdLookup`] instead iterate over CityOBJECTS — flattening
//!   every feature's `CityObjects` map (parents AND children) — so their
//!   `result_count` matches CityParquet's own object-level count EXACTLY on
//!   the same data (delft: `object_type == "BuildingPart"` -> 1116;
//!   `oorspronkelijkbouwjaar` numeric -> 1115). The attribute filter returns
//!   the matching identifiers; the identifier lookup reads every field of
//!   the object it finds and stops at that feature.
//! - [`Scenario::BBoxQuery`] is CityObject-level too: an object's box is the
//!   min/max over every vertex of its `children` subtree (the feature
//!   carries the whole subtree), decoded via the stream header's
//!   `transform`. It returns the matching objects' identifiers and walks
//!   each one's highest-LoD geometry in place
//!   ([`super::cjvisit::window_objects`]).
//!
//! None of this is silently normalised to match another format; the
//! milestone's methodology doc is responsible for disclosing it alongside
//! the numbers.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, anyhow, bail};
use cityparquet::cjseq::{CityJSON, CityJSONFeature, CityObject, Transform};
use cityparquet::counting_store::CountingObjectStore;
use cityparquet::source::Source;
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

use super::cjvisit::{Vertices, visit_object, window_objects};
use super::returned::{ComparableTotals, IdDigest, ReturnedGeometry};
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
            let transform = backend.transform().clone();
            let mut feature_count = 0u64;
            let mut totals = ComparableTotals::default();
            for feature in backend.features()? {
                let feature = feature?;
                feature_count += 1;
                let verts = Vertices {
                    vertices: &feature.vertices,
                    transform: &transform,
                };
                for co in feature.city_objects.values() {
                    visit_object(co, verts, &mut totals)?;
                }
            }
            Ok(Answer::reading(feature_count, totals))
        }
        Scenario::BBoxQuery => {
            let query_bbox = *require(&params.bbox, "bbox", scenario)?;
            let transform = backend.transform().clone();
            let mut ids = IdDigest::default();
            let mut geometry = ReturnedGeometry::default();
            for feature in backend.features()? {
                let feature = feature?;
                let verts = Vertices {
                    vertices: &feature.vertices,
                    transform: &transform,
                };
                window_objects(
                    &feature.city_objects,
                    verts,
                    &query_bbox,
                    &mut ids,
                    &mut geometry,
                )?;
            }
            Ok(Answer::returning(ids, Some(geometry)))
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            let mut ids = IdDigest::default();
            for feature in backend.features()? {
                let feature = feature?;
                for (id, co) in &feature.city_objects {
                    if matches_predicate(column_value(co, column).as_ref(), pred) {
                        ids.push(id);
                    }
                }
            }
            Ok(Answer::returning(ids, None))
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
            let transform = backend.transform().clone();
            let mut totals = ComparableTotals::default();
            for feature in backend.features()? {
                let feature = feature?;
                if let Some(co) = feature.city_objects.get(id) {
                    let verts = Vertices {
                        vertices: &feature.vertices,
                        transform: &transform,
                    };
                    visit_object(co, verts, &mut totals)?;
                    return Ok(Answer::reading(1, totals));
                }
            }
            Ok(Answer::reading(0, totals))
        }
        Scenario::FeatureLookup | Scenario::AttrLookup => {
            bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY)
        }
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
        .with_client_options(cityparquet_readbench::http_client::client_options())
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

        if scenario == Scenario::IdLookup {
            return id_lookup_streamed(base_url, key, params);
        }
        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, scenario, params))
    }
}

/// [`Scenario::IdLookup`] over HTTP: the GET's body is parsed as it arrives
/// and the transfer is abandoned at the hit (see [`super::body_stream`]), so
/// `bytes_read` is the bytes received up to the hit — the whole file only on a
/// miss. The found object is read in full, as the local arm reads it: the
/// same line parser as [`Source`]'s CityJSONSeq iterator, the same
/// [`visit_object`].
fn id_lookup_streamed(base_url: &str, key: &str, params: &QueryParams) -> Result<RunOutcome> {
    let id = require(&params.target_id, "target-id", Scenario::IdLookup)?;
    let mut body = BufReader::new(super::body_stream::BodyStream::get(base_url, key)?);
    let received = body.get_ref().received();
    let mut line = String::new();
    body.read_line(&mut line)
        .with_context(|| format!("reading the header line of {key}"))?;
    if line.trim_start().starts_with('<') {
        bail!(
            "{key} is a CityGML document, not CityJSONSeq; --format cityjsonseq must never be \
             pointed at CityGML"
        );
    }
    let header = CityJSON::from_str(line.trim_end())
        .map_err(|e| anyhow!("invalid CityJSONSeq header in {key}: {e}"))?;
    let transform = header.transform;
    let mut totals = ComparableTotals::default();
    let mut found = 0;
    loop {
        line.clear();
        if body
            .read_line(&mut line)
            .with_context(|| format!("reading {key}"))?
            == 0
        {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let feature = CityJSONFeature::from_str(line.trim_end())
            .map_err(|e| anyhow!("invalid CityJSONFeature line in {key}: {e}"))?;
        if let Some(co) = feature.city_objects.get(id) {
            let verts = Vertices {
                vertices: &feature.vertices,
                transform: &transform,
            };
            visit_object(co, verts, &mut totals)?;
            found = 1;
            break;
        }
    }
    // Dropping the body abandons the rest of the transfer; only then is the
    // received count final.
    drop(body);
    let answer = Answer::reading(found, totals);
    Ok(RunOutcome {
        result_count: answer.result_count,
        io: Some(IoStats {
            bytes: received.load(Ordering::Relaxed),
            requests: 1,
        }),
        lookup: None,
        attr_stats: answer.attr_stats,
        returned: answer.returned,
    })
}
