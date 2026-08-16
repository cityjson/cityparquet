//! The CityJSONSeq [`FormatRunner`]: full-parse baseline for plain
//! (`cityjsonseq`, `.city.jsonl`) and gzip-compressed (`cityjsonseq-gz`,
//! `.jsonl.gz`) CityJSONSeq streams. There is no index of any kind, so
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
//! - [`Scenario::AttrFilter`], [`Scenario::AttrStats`], [`Scenario::Project`],
//!   and [`Scenario::IdLookup`] instead iterate over CityOBJECTS — flattening
//!   every feature's `CityObjects` map (parents AND children) — so their
//!   `result_count` matches CityParquet's own object-level count EXACTLY on
//!   the same data (delft: `object_type == "BuildingPart"` -> 1116;
//!   `oorspronkelijkbouwjaar` present -> 1115). This is what makes these four
//!   scenarios meaningfully comparable across formats at all.
//! - [`Scenario::BBoxQuery`] is feature-level: each feature's bbox is the
//!   min/max over ALL of its (feature-local, transform-encoded) vertices,
//!   decoded via the stream header's `transform` — i.e. the union of every
//!   CityObject in that feature, not tested per-object.
//!
//! None of this is silently normalised to match another format; the
//! milestone's methodology doc is responsible for disclosing it alongside
//! the numbers.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use cityparquet::cjseq::{CityJSON, CityJSONFeature, CityObject, SortingStrategy, Transform};
use cityparquet::counting_store::CountingObjectStore;
use cityparquet::source::Source;
use flate2::read::GzDecoder;
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

use super::{FormatRunner, IoStats, RunOutcome, Source as TransportSource};
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

/// A gzip-compressed CityJSONSeq stream — OR a gzip-compressed
/// whole-document (single-object) CityJSON file, e.g.
/// `readbench_prepare.sh`'s `gzip -9` of a `.city.json` input that was
/// never CityJSONSeq to begin with (that script always names its gz
/// artefact `<base>.jsonl.gz` regardless of the original's own extension,
/// so the `.gz`-stripped name carries no reliable format signal). Mirrors
/// [`Source::open`]'s own sniff: a document only counts as a real Seq
/// stream when a further non-empty line actually follows the first one;
/// a single-line whole CityJSON document (no trailing newline, as e.g.
/// `lod3_railway.city.json` is) must NOT be misread as "header line, zero
/// feature lines" — that misreading previously made every scenario
/// silently return `0` for such an input (caught by the coordinator's own
/// cross-format self-consistency check, never a crash), rather than the
/// correct per-feature counts [`Source`]'s own whole-document fallback
/// produces for the plain (non-gz) `cityjsonseq` runner.
///
/// `flate2::read::GzDecoder` has no seek support, so — like [`Source`]'s
/// own plain-file handling — every [`GzSource::features`] call reopens and
/// re-decompresses `path` from the start rather than trying to rewind a
/// single decoder (except in the whole-document case, where the entire
/// content is already held in `doc` from the one sniffing pass in
/// [`GzSource::open`] — a second full decompression would be pure waste
/// for a format that cannot stream line-by-line at all).
struct GzSource {
    path: PathBuf,
    transform: Transform,
    /// `Some` only for the whole-document (non-Seq) case: the fully parsed
    /// document, pre-sorted for deterministic feature emission (matches
    /// [`Source`]'s own `CityJson`-variant field).
    doc: Option<CityJSON>,
}

impl GzSource {
    /// For a genuine CityJSONSeq stream, decompresses just far enough to
    /// read the header (first) line plus proof that a further non-empty
    /// line follows it, never the whole stream. For a whole-document
    /// CityJSON gz (no second line), the first `read_line` call already
    /// reads to EOF (there is no newline to stop at), so no extra
    /// decompression pass is needed to recover the full content — it is
    /// already sitting in `first_line`.
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut reader = BufReader::new(GzDecoder::new(file));
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .with_context(|| format!("reading gzip header line from {}", path.display()))?;
        let first_line = first_line.trim_end_matches(['\n', '\r']).to_string();

        let mut has_more = false;
        for line in reader.lines() {
            let line = line.with_context(|| format!("reading {}", path.display()))?;
            if !line.trim().is_empty() {
                has_more = true;
                break;
            }
        }

        if has_more {
            let header = CityJSON::from_str(&first_line)
                .map_err(|e| anyhow!("invalid CityJSONSeq header in {}: {e}", path.display()))?;
            Ok(Self {
                path: path.to_path_buf(),
                transform: header.transform,
                doc: None,
            })
        } else {
            let mut doc = CityJSON::from_str(&first_line)
                .map_err(|e| anyhow!("invalid CityJSON in {}: {e}", path.display()))?;
            doc.sort_cjfeatures(SortingStrategy::Lexicographical);
            let transform = doc.transform.clone();
            Ok(Self {
                path: path.to_path_buf(),
                transform,
                doc: Some(doc),
            })
        }
    }

    /// For a real Seq stream: reopens and re-decompresses `path`, skipping
    /// the header line, and streams the remaining lines as
    /// [`CityJSONFeature`]s — never collecting the whole stream into memory
    /// at once. For a whole-document gz (`self.doc` is `Some`): iterates the
    /// already-parsed, pre-sorted document's own features via
    /// [`CityJSON::get_cjfeature`] — the gz analogue of [`Source`]'s own
    /// `FeatureIter::Doc`.
    fn features(&self) -> Result<Box<dyn Iterator<Item = Result<CityJSONFeature>> + '_>> {
        if let Some(doc) = &self.doc {
            return Ok(Box::new(
                (0..).map_while(move |i| doc.get_cjfeature(i)).map(Ok),
            ));
        }

        let file =
            File::open(&self.path).with_context(|| format!("reopening {}", self.path.display()))?;
        let mut lines = BufReader::new(GzDecoder::new(file)).lines();
        lines.next(); // skip the already-parsed header line
        Ok(Box::new(lines.filter_map(move |line| match line {
            Err(e) => Some(Err(anyhow!("read error in {}: {e}", self.path.display()))),
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => Some(CityJSONFeature::from_str(&line).map_err(|e| {
                anyhow!(
                    "invalid CityJSONFeature line in {}: {e}",
                    self.path.display()
                )
            })),
        })))
    }
}

/// Unifies the plain ([`Source`]) and gzip ([`GzSource`]) backends behind
/// one `transform()`/`features()` surface, so the scenario dispatch in
/// [`CityJsonSeqRunner::run`] never needs an `if self.gzip` branch of its
/// own.
enum Backend {
    /// Both variants are boxed: `Source` and `GzSource` are each 1 KiB+,
    /// dominated by their own parsed `CityJSON` header (or, for a
    /// whole-document gz input, `GzSource`'s own `doc: Option<CityJSON>`) —
    /// without boxing, clippy's `large_enum_variant` flags every `Backend`
    /// value paying the larger variant's size regardless of which one is
    /// actually held.
    Plain(Box<Source>),
    Gz(Box<GzSource>),
}

impl Backend {
    fn open(input: &Path, gzip: bool) -> Result<Self> {
        if gzip {
            Ok(Backend::Gz(Box::new(GzSource::open(input)?)))
        } else {
            Ok(Backend::Plain(Box::new(
                Source::open(input).map_err(|e| anyhow!(e))?,
            )))
        }
    }

    fn transform(&self) -> &Transform {
        match self {
            Backend::Plain(source) => &source.header().transform,
            Backend::Gz(gz) => &gz.transform,
        }
    }

    fn features(&self) -> Result<Box<dyn Iterator<Item = Result<CityJSONFeature>> + '_>> {
        match self {
            Backend::Plain(source) => {
                let iter = source.features().map_err(|e| anyhow!(e))?;
                Ok(Box::new(iter.map(|r| r.map_err(|e| anyhow!(e)))))
            }
            Backend::Gz(gz) => gz.features(),
        }
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

/// Total leaf (numeric) values in `value`'s nested-array tree — a
/// geometry-type-agnostic stand-in for "how much boundary work would
/// decoding this geometry take", used only to force [`Scenario::FullRead`]
/// to actually traverse every geometry's `boundaries`, not merely to have
/// deserialized them into a [`serde_json::Value`] tree. The result itself is
/// discarded — [`Scenario::FullRead`]'s returned metric stays feature-level,
/// per this module's own counting-unit ruling above.
fn count_boundary_leaves(value: &serde_json::Value) -> u64 {
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
fn feature_bbox(feature: &CityJSONFeature, transform: &Transform) -> Option<([f64; 3], [f64; 3])> {
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

/// The CityJSONSeq backend, handling both plain (`cityjsonseq`) and
/// gzip-compressed (`cityjsonseq-gz`) streams depending on `gzip`.
pub struct CityJsonSeqRunner {
    gzip: bool,
}

impl CityJsonSeqRunner {
    /// The plain (`cityjsonseq`, `.city.jsonl`) backend.
    pub fn plain() -> Self {
        Self { gzip: false }
    }

    /// The gzip-compressed (`cityjsonseq-gz`, `.jsonl.gz`) backend.
    pub fn gzip() -> Self {
        Self { gzip: true }
    }
}

/// The scenario-dispatch body shared by both the local and (future) HTTP
/// branches of [`FormatRunner::run`]: everything below `Backend::open`
/// (which itself only differs in WHERE the bytes come from) is
/// transport-independent.
fn run_scenario(backend: &Backend, scenario: Scenario, params: &QueryParams) -> Result<u64> {
    match scenario {
        Scenario::Count => {
            let mut feature_count = 0u64;
            for feature in backend.features()? {
                feature?;
                feature_count += 1;
            }
            Ok(feature_count)
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
            let _ = boundary_work;
            Ok(feature_count)
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
            Ok(matched)
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
            Ok(matched)
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let mut count = 0u64;
            for feature in backend.features()? {
                let feature = feature?;
                for co in feature.city_objects.values() {
                    if column_value(co, column).and_then(|v| v.as_f64()).is_some() {
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            for feature in backend.features()? {
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
            for feature in backend.features()? {
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
    gzip: bool,
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

    let backend = Backend::open(tmp.path(), gzip)?;
    let result_count = run_scenario(&backend, scenario, params)?;
    Ok(RunOutcome {
        result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
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
                let backend = Backend::open(path, self.gzip)?;
                let result_count = run_scenario(&backend, scenario, params)?;
                return Ok(RunOutcome {
                    result_count,
                    io: None,
                });
            }
            TransportSource::Http { base_url, key } => (base_url, key),
        };

        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, self.gzip, scenario, params))
    }
}
