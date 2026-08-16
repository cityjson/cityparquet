//! The plain-CityJSON [`FormatRunner`]: full-parse baseline for a
//! whole-document CityJSON file (`cityjson`, `.city.json`) — one JSON object
//! holding a `CityObjects` map, one document-level `vertices` array shared by
//! every object, and the `transform` those integer vertices decode through.
//!
//! This is deliberately NOT an alias for [`super::cityjsonseq`]. That runner
//! is line-oriented (`BufReader::lines`, one self-contained feature per
//! line, feature-local vertices); a plain CityJSON document must be parsed in
//! ONE piece before any object can be read, and its geometry resolves against
//! the shared document-level vertex array. The parse shape genuinely differs,
//! so the measurement does too — which is the whole point of measuring this
//! format separately.
//!
//! There is no index of any kind, so EVERY scenario parses the whole
//! document. That full-parse cost is the honest, deliberate baseline this
//! runner measures, not an oversight — it is exactly what a consumer of a
//! published `.city.json` pays to answer even the cheapest question.
//!
//! **Cross-format counting caveat — deliberately NOT papered over here (the
//! house convention; see [`super::cityparquet`] and [`super::cityjsonseq`]
//! for their own).**
//!
//! A CityJSON document's natural unit is a **`CityObjects` map entry**, and
//! that map is flat: a `Building` and its `BuildingPart`s /
//! `BuildingInstallation`s are sibling entries linked only by
//! `parents`/`children`. This runner therefore counts SECOND-LEVEL objects in
//! their own right:
//!
//! - [`Scenario::Count`] and [`Scenario::FullRead`] return the size of the
//!   `CityObjects` map — the `lod3_railway` fixture has 121 (of which only 38
//!   are top-level), where the [`super::cityjsonseq`] runner reading the very
//!   same file reports its 38 top-level FEATURES. Both are honest answers to
//!   different questions. This runner's grain matches
//!   [`super::cityparquet`]'s own one-row-per-CityObject grain.
//! - [`Scenario::AttrFilter`], [`Scenario::AttrStats`],
//!   [`Scenario::Project`] and [`Scenario::IdLookup`] are CityObject-level
//!   too, and reuse [`super::cityjsonseq`]'s own attribute helpers verbatim,
//!   so the two JSON runners agree exactly on the same document by
//!   construction rather than by coincidence.
//! - [`Scenario::BBoxQuery`] is CityObject-level: each object's bbox is the
//!   min/max over every vertex its own geometries reference, resolved through
//!   the document `transform`. An object carrying no geometry at all (e.g.
//!   the railway fixture's lone `CityObjectGroup`) has no bbox and matches no
//!   window — it is excluded rather than counted as intersecting everything.
//!   A `GeometryInstance` contributes its anchor point (the one vertex index
//!   its `boundaries` hold), not the bounds of the template it instantiates;
//!   the same simplification [`super::cityjsonseq`]'s feature bbox makes.
//! - [`Scenario::AttrStats`] aggregates NUMERIC values only, so a
//!   string-typed column (the railway fixture's numeric-LOOKING `function`
//!   codes, e.g. `"1070"`) counts 0 — identical to
//!   [`super::cityjsonseq`]'s own behaviour on the same data, and the reason
//!   `attr-stats` and `project` can legitimately disagree.
//!
//! None of this is silently normalised to match another format; the
//! methodology doc is responsible for disclosing it alongside the numbers.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use cityparquet::cjseq::{CityJSON, CityObject, Transform};
use cityparquet::counting_store::CountingObjectStore;
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

use super::cityjsonseq::{column_value, intersects, matches_predicate, require};
use super::{FormatRunner, IoStats, RunOutcome, Source as TransportSource};
use crate::scenario::{QueryParams, Scenario};

/// A parsed whole CityJSON document, with its `transform` validated once so
/// the per-vertex hot path below can index `scale`/`translate` without
/// re-checking their length on every coordinate.
struct Document {
    doc: CityJSON,
}

impl Document {
    /// Parses `text` as one whole CityJSON document. Unlike
    /// [`super::cityjsonseq`]'s [`cityparquet::source::Source`], this never
    /// sniffs for a Seq stream: the `cityjson` format is only ever pointed at
    /// an artefact that IS a single document (see `Format::artefact`), and
    /// silently accepting a Seq stream here would measure a different format
    /// under this format's name.
    fn parse(text: &str, origin: &str) -> Result<Self> {
        let doc =
            CityJSON::from_str(text).map_err(|e| anyhow!("invalid CityJSON in {origin}: {e}"))?;
        if doc.transform.scale.len() < 3 || doc.transform.translate.len() < 3 {
            bail!(
                "CityJSON in {origin} has a malformed transform (scale/translate must each have \
                 3 components, got {}/{})",
                doc.transform.scale.len(),
                doc.transform.translate.len()
            );
        }
        Ok(Self { doc })
    }

    fn open(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text, &path.display().to_string())
    }

    fn objects(&self) -> impl Iterator<Item = &CityObject> {
        self.doc.city_objects.values()
    }
}

/// A running min/max over decoded (real-world) coordinates, plus whether any
/// vertex was seen at all — `any == false` means "this object has no bbox",
/// which is a real state (a geometry-less `CityObjectGroup`), not an error.
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
    any: bool,
}

impl Bounds {
    fn new() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
            any: false,
        }
    }

    fn finish(self) -> Option<([f64; 3], [f64; 3])> {
        self.any.then_some((self.min, self.max))
    }
}

/// Walks one geometry's `boundaries` tree, resolving EVERY leaf (a
/// document-level vertex index) through `vertices` + `transform` into a
/// real-world coordinate and folding it into `acc`.
///
/// This is the runner's geometry decode: [`Scenario::BBoxQuery`] uses the
/// resulting bounds, and [`Scenario::FullRead`] runs the identical traversal
/// purely for the work it forces (so "full read" really does touch every
/// coordinate rather than merely deserialising the boundary arrays into a
/// [`serde_json::Value`] tree).
///
/// Fallible on malformed input rather than silently skipping it: a boundary
/// leaf that is not a valid, in-range vertex index means the document is
/// broken, and a benchmark that quietly measured a shorter walk over broken
/// data would report a number nobody could trust.
fn accumulate_boundaries(
    value: &serde_json::Value,
    vertices: &[Vec<i64>],
    transform: &Transform,
    acc: &mut Bounds,
) -> Result<()> {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                accumulate_boundaries(item, vertices, transform, acc)?;
            }
            Ok(())
        }
        serde_json::Value::Number(number) => {
            let index = number.as_u64().ok_or_else(|| {
                anyhow!("geometry boundary index '{number}' is not a whole number")
            })? as usize;
            let vertex = vertices.get(index).ok_or_else(|| {
                anyhow!(
                    "geometry boundary references vertex {index}, but the document has only {}",
                    vertices.len()
                )
            })?;
            if vertex.len() < 3 {
                bail!("vertex {index} has {} components, expected 3", vertex.len());
            }
            for (axis, coord) in vertex.iter().take(3).enumerate() {
                let real = *coord as f64 * transform.scale[axis] + transform.translate[axis];
                acc.min[axis] = acc.min[axis].min(real);
                acc.max[axis] = acc.max[axis].max(real);
            }
            acc.any = true;
            Ok(())
        }
        other => bail!("unexpected value in geometry boundaries: {other}"),
    }
}

/// One CityObject's bbox over the DOCUMENT-level vertex array — `None` when
/// the object carries no geometry (or no boundaries) at all.
fn object_bounds(
    co: &CityObject,
    vertices: &[Vec<i64>],
    transform: &Transform,
) -> Result<Option<([f64; 3], [f64; 3])>> {
    let mut acc = Bounds::new();
    if let Some(geometries) = &co.geometry {
        for geometry in geometries {
            accumulate_boundaries(&geometry.boundaries, vertices, transform, &mut acc)?;
        }
    }
    Ok(acc.finish())
}

/// The scenario dispatch shared by the local and HTTP branches of
/// [`FormatRunner::run`]: everything below the parse (which only differs in
/// WHERE the bytes come from) is transport-independent.
fn run_scenario(document: &Document, scenario: Scenario, params: &QueryParams) -> Result<u64> {
    let doc = &document.doc;
    match scenario {
        // The `CityObjects` map is already fully materialised by the parse
        // this format cannot avoid, so `count` IS its size — there is no
        // cheaper path to pretend otherwise.
        Scenario::Count => Ok(doc.city_objects.len() as u64),
        Scenario::FullRead => {
            let mut object_count = 0u64;
            for co in document.objects() {
                object_count += 1;
                // Result discarded: the traversal itself is the measured
                // work (see `accumulate_boundaries`), and the returned
                // metric stays object-level per this module's own doc
                // comment.
                object_bounds(co, &doc.vertices, &doc.transform)?;
            }
            Ok(object_count)
        }
        Scenario::BBoxQuery => {
            let query_bbox = *require(&params.bbox, "bbox", scenario)?;
            let mut matched = 0u64;
            for co in document.objects() {
                if let Some((min, max)) = object_bounds(co, &doc.vertices, &doc.transform)?
                    && intersects(min, max, &query_bbox)
                {
                    matched += 1;
                }
            }
            Ok(matched)
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            Ok(document
                .objects()
                .filter(|co| matches_predicate(column_value(co, column).as_ref(), pred))
                .count() as u64)
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            Ok(document
                .objects()
                .filter(|co| column_value(co, column).and_then(|v| v.as_f64()).is_some())
                .count() as u64)
        }
        // The map is a `HashMap`, but a lookup still costs the whole parse
        // that built it — which is precisely the cost this scenario is meant
        // to expose for an unindexed format.
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            Ok(doc.city_objects.contains_key(id) as u64)
        }
        Scenario::Project => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            Ok(document
                .objects()
                .filter(|co| column_value(co, column).is_some())
                .count() as u64)
        }
    }
}

/// The HTTP-transport body of [`CityJsonRunner::run`]: a single whole-object
/// GET (via a [`CountingObjectStore`]-wrapped `object_store::http::HttpStore`
/// — exactly 1 request, the whole file's bytes, by construction), parsed
/// straight from memory.
///
/// A plain CityJSON document carries NO index and cannot be parsed in
/// pieces, so a range request would buy nothing: whatever the scenario, the
/// whole document must arrive before any question can be answered. The
/// reported [`IoStats`] say exactly that, rather than flattering the format
/// with a partial read it cannot actually perform.
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
    let stats = counting.tally();

    let text = std::str::from_utf8(&bytes).with_context(|| format!("{key} is not valid UTF-8"))?;
    let document = Document::parse(text, key)?;
    let result_count = run_scenario(&document, scenario, params)?;
    Ok(RunOutcome {
        result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
    })
}

/// The plain-CityJSON backend: parses the whole document on every
/// [`FormatRunner::run`] call (the `--child` protocol spawns one fresh
/// process per measurement, so there is nothing to cache between calls) and
/// answers each scenario from the parsed `CityObjects` map.
pub struct CityJsonRunner;

impl FormatRunner for CityJsonRunner {
    fn run(
        &self,
        source: &TransportSource,
        scenario: Scenario,
        params: &QueryParams,
    ) -> Result<RunOutcome> {
        let (base_url, key) = match source {
            TransportSource::Local(path) => {
                let document = Document::open(path)?;
                let result_count = run_scenario(&document, scenario, params)?;
                return Ok(RunOutcome {
                    result_count,
                    io: None,
                });
            }
            TransportSource::Http { base_url, key } => (base_url, key),
        };

        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, scenario, params))
    }
}
