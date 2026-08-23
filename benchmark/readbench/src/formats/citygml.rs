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
//! The reader is [`cityparquet::citygml`]: quick-xml streaming that yields one
//! [`CityJSONFeature`] per top-level `cityObjectMember`, with feature-local
//! vertices quantised against the document-derived header transform.
//!
//! **Three inputs are refused rather than measured**, on one principle: a
//! benchmark row that silently reports a number for a document this reader
//! could not fully read is worse than no row at all.
//!
//! 1. **A non-2.0 CityGML document** — the reader supports 2.0 only, and
//!    [`cityparquet::source::Source`] names the version it found.
//! 2. **A non-CityGML input** — `Source` would happily read a `.city.json`
//!    handed to `--format citygml`, and the CSV would then publish another
//!    format's cost under this format's name.
//! 3. **A document containing a `cityObjectMember` whose type the reader does
//!    not map** — see [`ensure_every_member_was_mapped`]. This is the same
//!    failure mode as (1) wearing different clothes, and it is live on the real
//!    corpus: a PLATEAU `trk`, `dem`, `lsld` or `urf` tile is composed entirely
//!    of unmapped members and used to report `count = 0` with exit status 0.
//!
//! **The appearance pre-pass is skipped.** `FeatureReader::open` re-reads the
//! whole document up front to index its CityModel-level appearance; not one of
//! the seven scenarios consults appearance, so this runner opens via
//! `open_without_appearance` instead. On a real 117 MB PLATEAU tile the
//! pre-pass was ~35-45% of `count`'s elapsed time and ~20x its peak heap —
//! both published CSV columns, and both measuring this harness rather than
//! CityGML. Features stream identically apart from the appearance itself
//! (pinned in `crates/core/tests/citygml_reader_profile.rs`).
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
//! - **Every member must be of a type the reader maps, or the run fails.** The
//!   reader streams `bldg:Building` plus the 1st-level non-building types it
//!   supports (WaterBody, LandUse, CityFurniture, SolitaryVegetationObject,
//!   PlantCover, Bridge, Tunnel, GenericCityObject, CityObjectGroup, Road,
//!   Railway, TransportSquare) and skips any other member. The row therefore
//!   describes this reader's supported profile of CityGML — and rather than
//!   let that qualification hide inside a number, a document that exercises it
//!   is refused outright ([`ensure_every_member_was_mapped`]). Which modules
//!   that rules out of the corpus is recorded in
//!   `benchmark/formats/archive/2026-08-17-catalogue-corpus/catalogue_benchmark_urls.txt`
//!   (that dataset came from the retired catalogue corpus).
//!
//! None of this is silently normalised to match another format; the
//! methodology doc is responsible for disclosing it alongside the numbers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use cityparquet::citygml::FeatureReader;
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

/// An opened CityGML 2.0 document: where its bytes are, what to call it in a
/// diagnostic, and the one global quantisation transform its feature vertices
/// are encoded against (derived from the document's own `gml:Envelope`).
struct Document {
    path: PathBuf,
    /// What a diagnostic names — the local path, or the HTTP key when the
    /// bytes were fetched into a tempfile whose name means nothing to anyone.
    origin: String,
    transform: Transform,
}

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
/// CityGML 2.0 is supported)") lives in exactly one place. `Source::open` on a
/// CityGML input does nothing but sniff and parse the preamble — it never
/// reads a feature — so it is not one of the full passes this runner measures.
fn open_citygml(path: &Path, origin: &str) -> Result<Document> {
    if cityparquet::citygml::sniff_citygml(path).is_none() {
        bail!(
            "{origin} is not a CityGML document (no CityGML <CityModel> root \
             element); --format citygml must never be pointed at \
             CityJSON/CityJSONSeq, or the benchmark would report another \
             format's cost under this format's name"
        );
    }
    let source = Source::open(path)
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("opening {origin} as CityGML"))?;
    if source.format() != SourceFormat::CityGml {
        bail!(
            "{origin} sniffed as CityGML but opened as {:?}",
            source.format()
        );
    }
    Ok(Document {
        path: path.to_path_buf(),
        origin: origin.to_string(),
        transform: source.header().transform.clone(),
    })
}

/// Refuses a document whose `cityObjectMember`s are not all of types the
/// reader maps.
///
/// **Why this is fatal rather than a footnote.** The reader maps
/// `bldg:Building` plus a fixed list of 1st-level non-building types and
/// silently skips the rest — right for the conversion pipeline, fatal for a
/// benchmark. A real PLATEAU `trk`, `dem`, `lsld` or `urf` tile consists
/// entirely of unmapped members, so every scenario used to return `0` with
/// exit status 0, in a fraction of the time a real read takes because nothing
/// was ever materialised. Meanwhile every OTHER format's artefact for the same
/// tile is produced by citygml-tools, which does map (say)
/// `dem:ReliefFeature` to CityJSON `TINRelief` — a type this repository treats
/// as first class — so the published CSV would read `citygml 0` beside
/// `cityjsonseq N`, with CityGML's timing flattered by the work it skipped.
///
/// The threshold is deliberately **zero members skipped**, not "zero features
/// read": the real PLATEAU `ubld` tile has two members, one mapped
/// (`grp:CityObjectGroup`) and one not (`uro:UndergroundBuilding` carrying 25
/// Rooms), and returns a perfectly plausible `1` that is simply wrong. A
/// `features > 0` check would wave it through.
///
/// This mirrors the CityGML 1.0 refusal in [`open_citygml`]: a document this
/// reader cannot fully read must fail loudly, never quietly return a number.
fn ensure_every_member_was_mapped(
    emitted: usize,
    skipped: &BTreeMap<String, usize>,
    origin: &str,
) -> Result<()> {
    let skipped_total: usize = skipped.values().sum();
    if skipped_total == 0 {
        return Ok(());
    }
    let total = emitted + skipped_total;
    let detail = skipped
        .iter()
        .map(|(name, count)| format!("{name} ×{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "{skipped_total} of {total} cityObjectMembers in {origin} are of types \
         this reader does not map ({detail}); a benchmark row must not report a \
         count that omits them — convert the tile with citygml-tools and \
         measure the CityJSON, or extend the reader's 1st-level type map"
    )
}

/// Streams every feature of `doc` through `visit`, then verifies that the
/// reader mapped every `cityObjectMember` — returning the member count.
///
/// The stream is always drained to EOF, even by a scenario that could answer
/// earlier (see [`Scenario::IdLookup`]): the skipped-member tally only becomes
/// authoritative at EOF, and an early exit would let a document whose FIRST
/// member is mapped publish a number while its later, unmapped members went
/// unnoticed. Draining is also what an unindexed format has to do to know it
/// is finished, so the cost is honest rather than added.
///
/// Opened via `open_without_appearance`: the reader's default `open` re-reads
/// the entire document up front to index its CityModel-level appearance, and
/// not one of the seven scenarios consults appearance at all. On a real 117 MB
/// PLATEAU tile that pre-pass was ~35-45% of `count`'s elapsed time and ~20x
/// its peak heap — both published CSV columns, both measuring this harness
/// rather than CityGML.
fn stream_members<F>(doc: &Document, mut visit: F) -> Result<u64>
where
    F: FnMut(CityJSONFeature) -> Result<()>,
{
    let mut reader = FeatureReader::open_without_appearance(&doc.path, &doc.transform)
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("streaming {}", doc.origin))?;
    let mut members = 0u64;
    for feature in reader.by_ref() {
        let feature = feature.map_err(|e| anyhow!(e))?;
        members += 1;
        visit(feature)?;
    }
    ensure_every_member_was_mapped(
        reader.emitted_members(),
        reader.skipped_members(),
        &doc.origin,
    )?;
    Ok(members)
}

/// The scenario dispatch shared by the local and HTTP branches of
/// [`FormatRunner::run`]: everything below the open (which only differs in
/// WHERE the bytes come from) is transport-independent.
///
/// Every scenario re-streams the document from the start — there is nothing
/// else a format with no index can do, and pretending otherwise is exactly
/// what this row exists to disprove.
fn run_scenario(doc: &Document, scenario: Scenario, params: &QueryParams) -> Result<u64> {
    match scenario {
        Scenario::Count => stream_members(doc, |_| Ok(())),
        Scenario::FullRead => {
            let mut boundary_work = 0u64;
            let members = stream_members(doc, |feature| {
                for co in feature.city_objects.values() {
                    if let Some(geoms) = &co.geometry {
                        for geom in geoms {
                            boundary_work += count_boundary_leaves(&geom.boundaries);
                        }
                    }
                }
                Ok(())
            })?;
            // `boundary_work` is computed purely to force the full geometry
            // traversal; the returned metric stays member-level per this
            // module's own counting-grain ruling. `black_box` rather than
            // `let _ =`, so the walk cannot be optimised away as dead code.
            std::hint::black_box(boundary_work);
            Ok(members)
        }
        Scenario::BBoxQuery => {
            let query_bbox = *require(&params.bbox, "bbox", scenario)?;
            let mut matched = 0u64;
            stream_members(doc, |feature| {
                // A member carrying no vertices at all (e.g. a
                // `grp:CityObjectGroup`, or an object whose only geometry is an
                // implicit representation the reader does not expand) has no
                // bbox and is honestly excluded rather than counted as
                // intersecting everything.
                if let Some((min, max)) = feature_bbox(&feature, &doc.transform)
                    && intersects(min, max, &query_bbox)
                {
                    matched += 1;
                }
                Ok(())
            })?;
            Ok(matched)
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            let mut matched = 0u64;
            stream_members(doc, |feature| {
                for co in feature.city_objects.values() {
                    if matches_predicate(column_value(co, column).as_ref(), pred) {
                        matched += 1;
                    }
                }
                Ok(())
            })?;
            Ok(matched)
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut count = 0u64;
            stream_members(doc, |feature| {
                for co in feature.city_objects.values() {
                    if column_value(co, column).and_then(|v| v.as_f64()).is_some() {
                        count += 1;
                    }
                }
                Ok(())
            })?;
            Ok(count)
        }
        // Deliberately no early exit: see [`stream_members`]. With no index the
        // whole document must be parsed to know an id is absent anyway, and
        // stopping at a lucky early hit would both flatter the timing and let
        // the skipped-member guard miss what came after.
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            let mut found = false;
            stream_members(doc, |feature| {
                found |= feature.city_objects.contains_key(id);
                Ok(())
            })?;
            Ok(found as u64)
        }
        Scenario::Project => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut count = 0u64;
            stream_members(doc, |feature| {
                for co in feature.city_objects.values() {
                    if column_value(co, column).is_some() {
                        count += 1;
                    }
                }
                Ok(())
            })?;
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

    // The reader is path-based (quick-xml streams from the file), so the
    // fetched bytes land in a tempfile rather than being parsed from memory.
    // The sniff and parse are content-based, so the tempfile's lack of a
    // `.gml` name is irrelevant — but its name is meaningless in a diagnostic,
    // hence `key` as the reported origin.
    let tmp =
        tempfile::NamedTempFile::new().context("creating a tempfile for the whole-object GET")?;
    std::fs::write(tmp.path(), &bytes)
        .with_context(|| format!("writing {} bytes to {}", bytes.len(), tmp.path().display()))?;
    let stats = counting.tally();

    let doc = open_citygml(tmp.path(), key)?;
    let result_count = run_scenario(&doc, scenario, params)?;
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
                let doc = open_citygml(path, &path.display().to_string())?;
                let result_count = run_scenario(&doc, scenario, params)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(entries: &[(&str, usize)]) -> BTreeMap<String, usize> {
        entries
            .iter()
            .map(|(name, count)| ((*name).to_string(), *count))
            .collect()
    }

    /// The threshold is "zero members skipped", NOT "zero features read".
    ///
    /// This is the real PLATEAU `ubld` tile's tally: two `cityObjectMember`s,
    /// one mapped (`grp:CityObjectGroup`) and one not
    /// (`uro:UndergroundBuilding`, carrying 25 Rooms). It returns a perfectly
    /// plausible `1`, and a `features > 0` guard would publish it. The tile
    /// itself is not committed as a fixture because its single unmapped member
    /// is 22 MB of one object; the tally it produces is the whole of what the
    /// guard sees, so it is pinned directly.
    #[test]
    fn a_partially_mapped_document_is_refused_even_though_it_yielded_features() {
        let skipped = tally(&[("uro:UndergroundBuilding", 1)]);
        let err = ensure_every_member_was_mapped(1, &skipped, "53394611_ubld_6697_op.gml")
            .expect_err("one skipped member must be fatal even beside one mapped member");
        let msg = err.to_string();
        assert!(msg.contains("1 of 2"), "got: {msg}");
        assert!(msg.contains("uro:UndergroundBuilding ×1"), "got: {msg}");
        assert!(msg.contains("does not map"), "got: {msg}");
    }

    /// Several unmapped types in one document are all named, in a
    /// deterministic order, with their counts — an operator has to be able to
    /// see what the tile actually holds.
    #[test]
    fn the_refusal_names_every_unmapped_type_with_its_count() {
        let skipped = tally(&[("dem:ReliefFeature", 3), ("tran:Track", 11)]);
        let msg = ensure_every_member_was_mapped(0, &skipped, "tile.gml")
            .expect_err("all members skipped")
            .to_string();
        assert!(msg.contains("14 of 14"), "got: {msg}");
        // `BTreeMap` ordering, so the diagnostic is byte-stable across runs.
        assert!(
            msg.contains("dem:ReliefFeature ×3, tran:Track ×11"),
            "got: {msg}"
        );
    }

    /// The other side: a document whose members were all mapped must pass, or
    /// every valid input would be refused.
    #[test]
    fn a_fully_mapped_document_passes() {
        assert!(ensure_every_member_was_mapped(4, &BTreeMap::new(), "ok.gml").is_ok());
        // A document with genuinely no members at all is not this guard's
        // business — it has nothing to omit.
        assert!(ensure_every_member_was_mapped(0, &BTreeMap::new(), "empty.gml").is_ok());
    }
}
