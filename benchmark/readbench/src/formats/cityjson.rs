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
//! - [`Scenario::AttrFilter`], [`Scenario::AttrStats`] and
//!   [`Scenario::IdLookup`] are CityObject-level
//!   too, and reuse [`super::cityjsonseq`]'s own attribute helpers verbatim,
//!   so the two JSON runners agree exactly on the same document by
//!   construction rather than by coincidence.
//! - [`Scenario::BBoxQuery`] is CityObject-level: each object's bbox is the
//!   min/max over every vertex referenced by a geometry in its SUBTREE — its
//!   own geometries and every descendant's, reached through `children` —
//!   resolved through the document `transform`. A `Building` with no
//!   geometry of its own therefore matches a window its `BuildingPart`s
//!   intersect, as its CityParquet row's `bbox` does (the specification's
//!   "Spatial metadata"). An object with no geometry anywhere in its subtree
//!   has no bbox and matches no window.
//!   A `GeometryInstance` contributes its anchor point (the one vertex index
//!   its `boundaries` hold), not the bounds of the template it instantiates.
//!   The window returns the matching objects' identifiers and walks each
//!   one's highest-LoD geometry in place. The same definition is shared with
//!   `cityjsonseq` and `citygml` ([`super::cjvisit::window_objects`]).
//! - [`Scenario::FullRead`] and [`Scenario::IdLookup`] read every field of
//!   the objects they reach natively ([`super::cjvisit::visit_object`]),
//!   exactly as `cityjsonseq` and `citygml` do: every boundary leaf resolved
//!   through the vertex array and `transform` into a real coordinate, every
//!   semantic reference and surface object, every attribute value. The
//!   difference left between this runner and `cityjsonseq` is where the
//!   vertices live: one document-level array here, a feature-local one per
//!   Seq line there.
//! - [`Scenario::AttrStats`] aggregates NUMERIC values only, so a
//!   string-typed column (the railway fixture's numeric-LOOKING `function`
//!   codes, e.g. `"1070"`) counts 0 — identical to
//!   [`super::cityjsonseq`]'s own behaviour on the same data. A column that
//!   is present is therefore not necessarily a column `attr-stats` counts.
//!
//! None of this is silently normalised to match another format; the
//! methodology doc is responsible for disclosing it alongside the numbers.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use cityparquet::cjseq::{CityJSON, CityObject};
use cityparquet::counting_store::CountingObjectStore;
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

use super::cityjsonseq::{column_value, matches_predicate, push_numeric, require};
use super::cjvisit::{Vertices, visit_object, window_objects};
use super::returned::{ComparableTotals, IdDigest, ReturnedGeometry};
use super::{Answer, AttrAggregates, FormatRunner, IoStats, RunOutcome, Source as TransportSource};
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

/// The scenario dispatch shared by the local and HTTP branches of
/// [`FormatRunner::run`]: everything below the parse (which only differs in
/// WHERE the bytes come from) is transport-independent.
fn run_scenario(document: &Document, scenario: Scenario, params: &QueryParams) -> Result<Answer> {
    let doc = &document.doc;
    let verts = Vertices {
        vertices: &doc.vertices,
        transform: &doc.transform,
    };
    match scenario {
        // The `CityObjects` map is already fully materialised by the parse
        // this format cannot avoid, so `count` IS its size — there is no
        // cheaper path to pretend otherwise.
        Scenario::Count => Ok((doc.city_objects.len() as u64).into()),
        Scenario::FullRead => {
            let mut totals = ComparableTotals::default();
            for co in document.objects() {
                visit_object(co, verts, &mut totals)?;
            }
            Ok(Answer::reading(totals.objects, totals))
        }
        Scenario::BBoxQuery => {
            let query_bbox = *require(&params.bbox, "bbox", scenario)?;
            let mut ids = IdDigest::default();
            let mut geometry = ReturnedGeometry::default();
            window_objects(
                &doc.city_objects,
                verts,
                &query_bbox,
                &mut ids,
                &mut geometry,
            )?;
            Ok(Answer::returning(ids, Some(geometry)))
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            let mut ids = IdDigest::default();
            for (id, co) in &doc.city_objects {
                if matches_predicate(column_value(co, column).as_ref(), pred) {
                    ids.push(id);
                }
            }
            Ok(Answer::returning(ids, None))
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut stats = AttrAggregates::EMPTY;
            for co in document.objects() {
                push_numeric(&mut stats, co, column);
            }
            // Pinned like `FullRead`'s leaf resolution: the aggregates are
            // the answer, so the arithmetic must not be reduced to a count.
            Ok(std::hint::black_box(stats).into())
        }
        // The map is a `HashMap`, but a lookup still costs the whole parse
        // that built it — which is precisely the cost this scenario is meant
        // to expose for an unindexed format.
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            let mut totals = ComparableTotals::default();
            if let Some(co) = doc.city_objects.get(id) {
                visit_object(co, verts, &mut totals)?;
            }
            Ok(Answer::reading(totals.objects, totals))
        }
        Scenario::FeatureLookup | Scenario::AttrLookup => {
            bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY)
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
    let stats = counting.tally();

    let text = std::str::from_utf8(&bytes).with_context(|| format!("{key} is not valid UTF-8"))?;
    let document = Document::parse(text, key)?;
    let answer = run_scenario(&document, scenario, params)?;
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
                let answer = run_scenario(&document, scenario, params)?;
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
