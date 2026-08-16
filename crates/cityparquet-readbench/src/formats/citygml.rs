//! The CityGML [`FormatRunner`]: the baseline row for the format the source
//! data actually ships in.
//!
//! **What this row measures, and what it does not claim.** A CityGML document
//! carries no index of any kind — no offsets, no per-object directory, no
//! spatial or attribute tree — so EVERY scenario here is a full XML parse
//! followed by an in-memory filter. That full parse is the honest, deliberate
//! baseline this runner measures, not an oversight: the row answers "what does
//! it cost to answer this query against the format the data ships in, using
//! the same codebase as every other row?". It is **not** a claim about the
//! format's theoretical ceiling. A different parser — a streaming SAX filter
//! tuned to one query, an XML database, a pre-built external index — would
//! give different numbers; nothing here bounds what CityGML could achieve in
//! principle, only what reading it costs through this repository's own
//! reader.
//!
//! The reader is [`cityparquet::citygml`] (reached through
//! [`cityparquet::source::Source`], which is its only integration surface):
//! quick-xml streaming that yields one [`CityJSONFeature`] per top-level
//! `cityObjectMember`, with feature-local vertices quantised against the
//! document-derived header transform. **CityGML 2.0 only** — a 1.0 or 3.0
//! document is refused with a version error rather than measured, because a
//! benchmark row that silently reported 0 objects for a file it could not read
//! would be worse than no row at all. So is a non-CityGML input: `Source`
//! would happily read a `.city.json` handed to `--format citygml`, and the CSV
//! would then publish another format's cost under this format's name.
//!
//! **Cross-format counting caveat — deliberately NOT papered over here (the
//! house convention; see [`super::cityparquet`], [`super::cityjsonseq`] and
//! [`super::cityjson`] for their own).**
//!
//! This runner's grain is [`super::cityjsonseq`]'s, exactly, so the two rows
//! are directly comparable:
//!
//! - [`Scenario::Count`], [`Scenario::FullRead`] and [`Scenario::BBoxQuery`]
//!   are **member-level**: they count top-level `cityObjectMember`s, one per
//!   1st-level CityObject. A `bldg:BuildingPart` or a
//!   `bldg:BuildingInstallation` is NOT counted in its own right here — it is
//!   nested inside its parent's feature, exactly as a CityJSONSeq line bundles
//!   a Building with its parts.
//! - [`Scenario::AttrFilter`], [`Scenario::AttrStats`], [`Scenario::Project`]
//!   and [`Scenario::IdLookup`] are **CityObject-level**: they flatten every
//!   feature's `CityObjects` map, so those nested parts and installations DO
//!   count. On `railway_lod3_fragment.gml` that is 4 vs. 6 — both honest
//!   answers to different questions, and asserted rather than merely claimed
//!   in `tests/citygml_runner.rs`.
//! - Those four scenarios reuse [`super::cityjsonseq`]'s own attribute helpers
//!   verbatim, so `citygml` and the two JSON runners agree on what a column
//!   name and an `--attr-eq` predicate mean by construction rather than by
//!   coincidence.
//! - [`Scenario::FullRead`] is the SAME operation as
//!   [`super::cityjsonseq`]'s: parse every feature, then walk every geometry's
//!   `boundaries` index tree. It costs more here only because the parse itself
//!   costs more — a CityGML parse must additionally decode every `gml:pos` /
//!   `gml:posList` coordinate, resolve every `xlink:href` surface reference,
//!   and rebuild a feature-local vertex pool, none of which a JSON reader
//!   does. The boundary walk's own tally is discarded, so it is wrapped in
//!   [`std::hint::black_box`]: "full read genuinely touches every boundary"
//!   then holds by construction rather than by the optimiser's current mood.
//! - **Only members whose type the reader maps are counted.** The reader
//!   streams `bldg:Building` plus the 1st-level non-building types it
//!   supports (WaterBody, LandUse, CityFurniture, SolitaryVegetationObject,
//!   PlantCover, Bridge, Tunnel, GenericCityObject, CityObjectGroup, Road,
//!   Railway, TransportSquare); a member of any other type (e.g.
//!   `dem:ReliefFeature`) is skipped and therefore absent from every count.
//!   The row measures this reader's supported profile of CityGML, which is the
//!   same qualification the paragraph above makes about the parser generally.
//!
//! None of this is silently normalised to match another format; the
//! methodology doc is responsible for disclosing it alongside the numbers.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use cityparquet::cjseq::{CityJSONFeature, Transform};
use cityparquet::counting_store::CountingObjectStore;
use cityparquet::source::{Source, SourceFormat};
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

use super::cityjsonseq::{
    column_value, count_boundary_leaves, feature_bbox, intersects, matches_predicate, require,
};
use super::{FormatRunner, IoStats, RunOutcome, Source as TransportSource};
use crate::scenario::{QueryParams, Scenario};

/// Opens `path` as a CityGML 2.0 document, refusing anything else.
///
/// The sniff is deliberately done here as well as inside [`Source::open`]: on
/// its own, `Source::open` falls back to the CityJSON/CityJSONSeq branches for
/// a non-XML input and would read a `.city.json` quite happily — which would
/// publish another format's cost in this format's CSV row. It reads only far
/// enough to see the root element, so the extra pass is a handful of XML
/// events, not a second parse.
///
/// A CityGML document of an UNSUPPORTED version is left to `Source::open` to
/// reject, so its version message ("unsupported CityGML version 1.0 (only
/// CityGML 2.0 is supported)") lives in exactly one place.
fn open_citygml(path: &Path) -> Result<Source> {
    if cityparquet::citygml::sniff_citygml(path).is_none() {
        bail!(
            "{} is not a CityGML document (no CityGML <CityModel> root element); \
             --format citygml must never be pointed at CityJSON/CityJSONSeq, or \
             the benchmark would report another format's cost under this \
             format's name",
            path.display()
        );
    }
    let source = Source::open(path)
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("opening {} as CityGML", path.display()))?;
    if source.format() != SourceFormat::CityGml {
        bail!(
            "{} sniffed as CityGML but opened as {:?}",
            path.display(),
            source.format()
        );
    }
    Ok(source)
}

/// Every feature in `source`, as this runner's error type.
fn features(source: &Source) -> Result<impl Iterator<Item = Result<CityJSONFeature>> + '_> {
    Ok(source
        .features()
        .map_err(|e| anyhow!(e))?
        .map(|r| r.map_err(|e| anyhow!(e))))
}

/// The scenario dispatch shared by the local and HTTP branches of
/// [`FormatRunner::run`]: everything below the open (which only differs in
/// WHERE the bytes come from) is transport-independent.
///
/// Every scenario re-streams the document from the start — there is nothing
/// else a format with no index can do, and pretending otherwise is exactly
/// what this row exists to disprove.
fn run_scenario(source: &Source, scenario: Scenario, params: &QueryParams) -> Result<u64> {
    match scenario {
        Scenario::Count => {
            let mut members = 0u64;
            for feature in features(source)? {
                feature?;
                members += 1;
            }
            Ok(members)
        }
        Scenario::FullRead => {
            let mut members = 0u64;
            let mut boundary_work = 0u64;
            for feature in features(source)? {
                let feature = feature?;
                members += 1;
                for co in feature.city_objects.values() {
                    if let Some(geoms) = &co.geometry {
                        for geom in geoms {
                            boundary_work += count_boundary_leaves(&geom.boundaries);
                        }
                    }
                }
            }
            // `boundary_work` is computed purely to force the full geometry
            // traversal; the returned metric stays member-level per this
            // module's own counting-grain ruling. `black_box` rather than
            // `let _ =`, so the walk cannot be optimised away as dead code.
            std::hint::black_box(boundary_work);
            Ok(members)
        }
        Scenario::BBoxQuery => {
            let query_bbox = *require(&params.bbox, "bbox", scenario)?;
            let transform: Transform = source.header().transform.clone();
            let mut matched = 0u64;
            for feature in features(source)? {
                let feature = feature?;
                // A member carrying no vertices at all (e.g. a
                // `grp:CityObjectGroup`, or an object whose only geometry is an
                // implicit representation the reader does not expand) has no
                // bbox and is honestly excluded rather than counted as
                // intersecting everything.
                if let Some((min, max)) = feature_bbox(&feature, &transform)
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
            let mut matched = 0u64;
            for feature in features(source)? {
                let feature = feature?;
                for co in feature.city_objects.values() {
                    if matches_predicate(column_value(co, column).as_ref(), pred) {
                        matched += 1;
                    }
                }
            }
            Ok(matched)
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut count = 0u64;
            for feature in features(source)? {
                let feature = feature?;
                for co in feature.city_objects.values() {
                    if column_value(co, column).and_then(|v| v.as_f64()).is_some() {
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
        // No index means no early exit worth having: the whole document must
        // be parsed to know the id is absent, and the scan stops early only in
        // the lucky case of an early hit — which is precisely the cost this
        // scenario exists to expose.
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            for feature in features(source)? {
                let feature = feature?;
                if feature.city_objects.contains_key(id) {
                    return Ok(1);
                }
            }
            Ok(0)
        }
        Scenario::Project => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut count = 0u64;
            for feature in features(source)? {
                let feature = feature?;
                for co in feature.city_objects.values() {
                    if column_value(co, column).is_some() {
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
    }
}

/// The HTTP-transport body of [`CityGmlRunner::run`]: a single whole-object
/// GET (via a [`CountingObjectStore`]-wrapped `object_store::http::HttpStore`
/// — exactly 1 request, the whole file's bytes, by construction) written to a
/// [`tempfile::NamedTempFile`], then handed to the same unchanged
/// [`open_citygml`]/[`run_scenario`] path the local transport uses.
///
/// A range request would buy nothing here: with no index there is no way to
/// know which byte range holds the answer, and the reader re-streams the
/// document from the start for every scenario anyway. The reported
/// [`IoStats`] therefore say "the whole file, once", which is the honest
/// answer rather than one that flatters the format with a partial read it
/// cannot actually perform.
async fn run_http(
    base_url: &str,
    key: &str,
    scenario: Scenario,
    params: &QueryParams,
) -> Result<RunOutcome> {
    // `with_allow_http(true)` is required for a plain `http://` target (the
    // in-test server this crate's own tests point at); it does not disable or
    // otherwise affect `https://` targets (real S3/R2 buckets).
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

    // The reader is path-based (it re-opens the document for its appearance
    // pre-pass and again for the feature stream), so the fetched bytes land in
    // a tempfile rather than being parsed from memory. The sniff and parse are
    // content-based, so the tempfile's lack of a `.gml` name is irrelevant.
    let tmp =
        tempfile::NamedTempFile::new().context("creating a tempfile for the whole-object GET")?;
    std::fs::write(tmp.path(), &bytes)
        .with_context(|| format!("writing {} bytes to {}", bytes.len(), tmp.path().display()))?;
    let stats = counting.tally();

    let source = open_citygml(tmp.path())?;
    let result_count = run_scenario(&source, scenario, params)?;
    Ok(RunOutcome {
        result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
    })
}

/// The CityGML backend: re-parses the whole document on every
/// [`FormatRunner::run`] call (the `--child` protocol spawns one fresh process
/// per measurement, so there is nothing to cache between calls) and answers
/// each scenario from the streamed features.
pub struct CityGmlRunner;

impl FormatRunner for CityGmlRunner {
    fn run(
        &self,
        source: &TransportSource,
        scenario: Scenario,
        params: &QueryParams,
    ) -> Result<RunOutcome> {
        let (base_url, key) = match source {
            TransportSource::Local(path) => {
                let source = open_citygml(path)?;
                let result_count = run_scenario(&source, scenario, params)?;
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
