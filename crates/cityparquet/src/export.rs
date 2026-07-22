//! Export: a CityParquet package directory back into CityJSON/CityJSONSeq.
//! Inverse of [`crate::package::convert`] at the whole-package level, built
//! on top of [`crate::decode::decode_batch`] (row -> `cjseq`-model object,
//! geometry deliberately excluded there) plus this module's own geometry
//! reconstruction (WKB -> CityJSON boundary arrays, re-quantised against the
//! dataset's own `transform`).
//!
//! `GeometryInstance` geometries are rebuilt from the `geometry_templates.parquet`
//! sidecar when the package MANIFEST lists it (M4 task 10; gating fixed under
//! the M4 Codex-review Finding 1 — the manifest, not the file's mere presence
//! on disk, is authoritative, matching how materials/textures are already
//! gated below): the header's `geometry-templates` is reconstructed from the
//! sidecar's WKB + `geometry_properties` (template vertices are RAW floats —
//! CityJSON spec §3.4 — so they are re-interned by `f64::to_bits` triple,
//! never through the dataset's quantised transform), and each object's
//! `template` reference becomes a `GeometryInstance` geometry pointing at it.
//! When the manifest does not list the sidecar (the Core profile, or a
//! Compatibility dataset with no templates at all), a `template` reference
//! cannot be resolved to anything real, so the owning object (attributes,
//! hierarchy) is still exported but its instance geometry is dropped, counted
//! in [`ExportReport::instance_geometries_dropped`]. A `template` reference
//! that names no row in a sidecar that IS present is a different situation —
//! a corrupt/hand-rolled file — and surfaces as a `Schema` error rather than
//! a silent drop. Two further corrupt-file cases the manifest gating itself
//! guards against: the manifest lists `geometry_templates.parquet` but the
//! file is missing/unreadable (an `Io` error — a truncated/tampered package,
//! never a silent all-instances-dropped outcome), and a `geometry_templates.parquet`
//! file left on disk but NOT listed in the manifest (ignored outright — the
//! manifest is the sole source of truth, exactly like an unlisted
//! `materials.parquet`/`textures.parquet` already is).
//!
//! Material/texture index maps are handled differently depending on what the
//! package actually carries: when `metadata.json`'s `sidecar_files` lists
//! `materials.parquet`/`textures.parquet` (the Compatibility profile), this
//! pass loads the dataset-global appearance definitions and, per feature,
//! slices out the subset that feature's geometries reference — reassigning
//! feature-local indices and re-interning inlined UV pairs into a
//! feature-local vertices-texture pool (see [`LocalAppearance`], the inverse
//! of [`crate::appearance::AppearanceInterner`]'s rewrite) — and attaches the
//! result as the feature's `appearance` block. When no such sidecars are
//! listed (the Core profile), the appearance DEFINITIONS the main-table maps
//! index are simply not stored anywhere in the package, so the maps are
//! dropped instead (counted in [`ExportReport::appearance_refs_dropped`]):
//! exporting them would leave dangling references — invalid CityJSON.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

use cityparquet_schema::model::{LOD_KEY, ROLE_KEY, ROLE_RESERVED};
use cityparquet_schema::{CityMetadata, CityParquetError, Result};
use cjseq::{
    Appearance, CityJSON, CityJSONFeature, Geometry, GeometryTemplates, GeometryType, Material,
    Metadata as CjMetadata, ReferenceSystem, Texture, Transform,
};

use crate::decode::{DecodedObject, decode_batch};
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use crate::sidecar::{TemplateRow, read_materials, read_templates, read_textures};
use crate::stac::properties::PackageTables;
use crate::wkb_read::{DecodedKind, wkb_to_geometry};

/// Options controlling one package -> CityJSON/CityJSONSeq export.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub package_dir: PathBuf,
    /// Output path; `.city.jsonl` writes CityJSONSeq (header line + one
    /// feature per line), `.city.json` writes a single whole CityJSON
    /// document via `cjseq::cjseq_to_cj`.
    pub output: PathBuf,
}

/// Outcome of one [`export`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportReport {
    pub feature_count: usize,
    pub object_count: usize,
    /// Objects whose `GeometryInstance` geometry was dropped because no
    /// `geometry_templates.parquet` sidecar was available to resolve it
    /// against (the Core profile, or a Compatibility dataset with no
    /// templates at all). `0` whenever the sidecar is present: every
    /// resolvable `template` reference is rebuilt into a real
    /// `GeometryInstance` geometry instead — see the module doc.
    pub instance_geometries_dropped: usize,
    /// Geometries whose material/texture index maps were dropped: the Core
    /// profile stores the maps but not the appearance definitions they index
    /// (M4 sidecar data), so exporting them would leave dangling references
    /// — invalid CityJSON, same reasoning as the GeometryInstance drop.
    pub appearance_refs_dropped: usize,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

fn io_err(msg: String) -> CityParquetError {
    CityParquetError::Io(msg)
}

/// Short file name for a table path, for user-facing messages — `tables`
/// entries are absolute paths (see `PackageTables`), and echoing the whole
/// path (e.g. a tempdir prefix in tests/benches) leaks environment detail
/// that isn't part of the package's own naming.
pub(crate) fn table_display_name(path: &std::path::Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<table>")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Seq,
    Doc,
}

fn output_format(output: &std::path::Path) -> Result<OutputFormat> {
    match output.extension().and_then(|e| e.to_str()) {
        Some("jsonl") => Ok(OutputFormat::Seq),
        Some("json") => Ok(OutputFormat::Doc),
        other => Err(err(format!(
            "unsupported export output extension {other:?}: expected .city.jsonl or .city.json"
        ))),
    }
}

/// `(scale, translate)` as fixed-size arrays, missing components defaulting
/// like [`crate::wkb_write::VertexPool`]'s identical helper.
fn transform_axes(transform: &Transform) -> ([f64; 3], [f64; 3]) {
    let take3 = |v: &[f64], d: f64| {
        [
            *v.first().unwrap_or(&d),
            *v.get(1).unwrap_or(&d),
            *v.get(2).unwrap_or(&d),
        ]
    };
    (
        take3(&transform.scale, 1.0),
        take3(&transform.translate, 0.0),
    )
}

fn quantise(c: [f64; 3], scale: [f64; 3], translate: [f64; 3]) -> [i64; 3] {
    [
        ((c[0] - translate[0]) / scale[0]).round() as i64,
        ((c[1] - translate[1]) / scale[1]).round() as i64,
        ((c[2] - translate[2]) / scale[2]).round() as i64,
    ]
}

/// Per-feature vertex pool: interns quantised coordinate triples, dedup'd.
#[derive(Default)]
struct VertexInterner {
    index: HashMap<[i64; 3], usize>,
    vertices: Vec<Vec<i64>>,
}

impl VertexInterner {
    fn intern(&mut self, c: [i64; 3]) -> usize {
        *self.index.entry(c).or_insert_with(|| {
            let idx = self.vertices.len();
            self.vertices.push(vec![c[0], c[1], c[2]]);
            idx
        })
    }

    fn finish(self) -> Vec<Vec<i64>> {
        self.vertices
    }
}

/// Header-scope vertex pool for the rebuilt `vertices-templates`: interns raw
/// `f64` XYZ triples bitwise (`f64::to_bits`) — geometry template vertices
/// are NOT subject to the dataset's quantised `transform` (CityJSON spec
/// §3.4; mirrors [`crate::wkb_write::VertexPool::raw`]'s write-side
/// counterpart), so they must never go through [`VertexInterner`]'s
/// quantise-then-intern path.
#[derive(Default)]
struct RawVertexInterner {
    index: HashMap<[u64; 3], usize>,
    vertices: Vec<Vec<f64>>,
}

impl RawVertexInterner {
    fn intern(&mut self, c: [f64; 3]) -> usize {
        let key = [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()];
        *self.index.entry(key).or_insert_with(|| {
            let idx = self.vertices.len();
            self.vertices.push(vec![c[0], c[1], c[2]]);
            idx
        })
    }

    fn finish(self) -> Vec<Vec<f64>> {
        self.vertices
    }
}

/// Requantise every local coordinate of a decoded geometry and intern it into
/// the feature's shared vertex pool, returning the local-index -> feature
/// vertex-index map (`vmap[local] = feature_index`).
fn vertex_map(
    coords: &[[f64; 3]],
    scale: [f64; 3],
    translate: [f64; 3],
    interner: &mut VertexInterner,
) -> Vec<usize> {
    coords
        .iter()
        .map(|&c| interner.intern(quantise(c, scale, translate)))
        .collect()
}

fn remap_idx(idxs: &[usize], vmap: &[usize]) -> Vec<usize> {
    idxs.iter().map(|&i| vmap[i]).collect()
}

fn remap_ring_list(rings: &[Vec<usize>], vmap: &[usize]) -> Vec<Vec<usize>> {
    rings.iter().map(|r| remap_idx(r, vmap)).collect()
}

fn remap_face_list(faces: &[Vec<Vec<usize>>], vmap: &[usize]) -> Vec<Vec<Vec<usize>>> {
    faces.iter().map(|f| remap_ring_list(f, vmap)).collect()
}

/// Partitions a flat, already-remapped face list into shells per
/// `counts` (face count per shell); falls back to a single shell holding
/// every face when `counts` is absent. Counts that do not sum to exactly
/// the face total are an error — silently mis-partitioning (or dropping
/// trailing faces) would corrupt the geometry.
pub(crate) fn partition_shells(
    faces: Vec<Vec<Vec<usize>>>,
    counts: Option<&[usize]>,
) -> Result<Vec<Vec<Vec<Vec<usize>>>>> {
    match counts {
        Some(counts) => {
            // Checked sum: `shells` can come from untrusted package
            // data, so a sum that overflows `usize` must be a clean error, not
            // a debug panic / release wraparound that then mis-partitions.
            let mut total: usize = 0;
            for &n in counts {
                total = total
                    .checked_add(n)
                    .ok_or_else(|| err("shells counts overflow usize".to_string()))?;
            }
            if total != faces.len() {
                return Err(err(format!(
                    "shells counts sum to {total} but the stored geometry has {} faces",
                    faces.len()
                )));
            }
            let mut shells = Vec::with_capacity(counts.len());
            let mut iter = faces.into_iter();
            for &n in counts {
                shells.push(iter.by_ref().take(n).collect());
            }
            Ok(shells)
        }
        None => Ok(vec![faces]),
    }
}

fn geom_shape_err(gtype: &GeometryType, kind: &DecodedKind) -> CityParquetError {
    err(format!(
        "geometry_properties type {gtype:?} does not match the decoded WKB shape {kind:?}"
    ))
}

/// The `shells` per-shell face counts from `geometry_properties` (§8), shaped
/// for a single `Solid` (a flat array).
pub(crate) fn shell_faces_flat(props: Option<&Value>) -> Result<Option<Vec<usize>>> {
    let Some(v) = props.and_then(|p| p.get("shells")) else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(v.clone())?))
}

/// The `shells` per-shell face counts from `geometry_properties` (§8), shaped
/// for `MultiSolid`/`CompositeSolid` (one per-shell face-count list per solid).
pub(crate) fn shell_faces_nested(props: Option<&Value>) -> Result<Option<Vec<Vec<usize>>>> {
    let Some(v) = props.and_then(|p| p.get("shells")) else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(v.clone())?))
}

/// Reconstructs one geometry's CityJSON `boundaries` value from its decoded
/// WKB shape, the CityJSON `type` recorded in `geometry_properties` (which
/// disambiguates MultiSurface/CompositeSurface and MultiSolid/
/// CompositeSolid — WKB alone cannot), and the per-feature vertex map.
fn reconstruct_boundaries(
    kind: &DecodedKind,
    gtype: &GeometryType,
    props: Option<&Value>,
    vmap: &[usize],
) -> Result<Value> {
    match gtype {
        GeometryType::MultiPoint => {
            let DecodedKind::MultiPoint(idxs) = kind else {
                return Err(geom_shape_err(gtype, kind));
            };
            Ok(serde_json::to_value(remap_idx(idxs, vmap))?)
        }
        GeometryType::MultiLineString => {
            let DecodedKind::MultiLineString(lines) = kind else {
                return Err(geom_shape_err(gtype, kind));
            };
            Ok(serde_json::to_value(remap_ring_list(lines, vmap))?)
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let DecodedKind::MultiPolygon(surfaces) = kind else {
                return Err(geom_shape_err(gtype, kind));
            };
            Ok(serde_json::to_value(remap_face_list(surfaces, vmap))?)
        }
        GeometryType::Solid => {
            let DecodedKind::PolyhedralSurface(faces) = kind else {
                return Err(geom_shape_err(gtype, kind));
            };
            let remapped = remap_face_list(faces, vmap);
            let counts = shell_faces_flat(props)?;
            let shells = partition_shells(remapped, counts.as_deref())?;
            Ok(serde_json::to_value(shells)?)
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let DecodedKind::GeometryCollection(members) = kind else {
                return Err(geom_shape_err(gtype, kind));
            };
            let nested_counts = shell_faces_nested(props)?;
            let mut solids = Vec::with_capacity(members.len());
            for (m, member) in members.iter().enumerate() {
                let DecodedKind::PolyhedralSurface(faces) = member else {
                    return Err(geom_shape_err(gtype, kind));
                };
                let remapped = remap_face_list(faces, vmap);
                let counts = match &nested_counts {
                    Some(c) => Some(
                        c.get(m)
                            .ok_or_else(|| {
                                err(format!(
                                    "shells lists {} solids but the stored \
                                     GeometryCollection has {} members",
                                    c.len(),
                                    members.len()
                                ))
                            })?
                            .as_slice(),
                    ),
                    None => None,
                };
                solids.push(partition_shells(remapped, counts)?);
            }
            Ok(serde_json::to_value(solids)?)
        }
        // The encoder never stores WKB for a GeometryInstance (it routes to
        // the `template` column), so a geometry_properties "type" claiming
        // one against a real WKB cell is a corrupt/hand-rolled file — an
        // error, not a crash.
        GeometryType::GeometryInstance => Err(geom_shape_err(gtype, kind)),
    }
}

/// Split a flat `face_semantics` slice into consecutive groups of `counts`
/// lengths — the inverse of the encoder's flatten. A count sum that does not
/// match the slice length is a corrupt/hand-rolled package, an error.
fn partition_face_semantics(face_semantics: &[Value], counts: &[usize]) -> Result<Vec<Value>> {
    let mut total: usize = 0;
    for &n in counts {
        total = total
            .checked_add(n)
            .ok_or_else(|| err("shells counts overflow usize".to_string()))?;
    }
    if total != face_semantics.len() {
        return Err(err(format!(
            "shells counts sum to {total} but face_semantics has {} entries",
            face_semantics.len()
        )));
    }
    let mut out = Vec::with_capacity(counts.len());
    let mut offset = 0;
    for &n in counts {
        out.push(Value::Array(face_semantics[offset..offset + n].to_vec()));
        offset += n;
    }
    Ok(out)
}

/// Rebuild a geometry's CityJSON `semantics` (`{surfaces, values}`) from the
/// flattened `geometry_properties` (§8): `surfaces` verbatim, and the nested
/// `values` re-derived from the flat `face_semantics` using `shells` (which
/// gives the per-shell face partition). The exporter emits the expanded
/// per-face form; a source that used the null shorthand round-trips up to that
/// canonicalisation (§17). `None` when the geometry carried no semantics.
fn rebuild_semantics(props: Option<&Value>, gtype: &GeometryType) -> Result<Option<Value>> {
    let Some(props) = props else {
        return Ok(None);
    };
    // No `surfaces` key ⇒ the geometry had no semantics at all.
    let Some(surfaces) = props.get("surfaces") else {
        return Ok(None);
    };
    let face_semantics: Vec<Value> = props
        .get("face_semantics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let values = match gtype {
        GeometryType::MultiPoint
        | GeometryType::MultiLineString
        | GeometryType::MultiSurface
        | GeometryType::CompositeSurface => Value::Array(face_semantics),
        GeometryType::Solid => {
            let counts = shell_faces_flat(Some(props))?;
            match counts {
                Some(counts) => Value::Array(partition_face_semantics(&face_semantics, &counts)?),
                // No `shells`: mirror `reconstruct_boundaries`, which puts every
                // face in one shell (§8). A Solid's `values` must be nested, so
                // wrap the flat list in a single shell rather than emit it flat.
                None => Value::Array(vec![Value::Array(face_semantics)]),
            }
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            // `shells` is required to split the flat list per solid; without it
            // the partition is ambiguous, so reject rather than silently drop.
            let nested = shell_faces_nested(Some(props))?.ok_or_else(|| {
                err("MultiSolid/CompositeSolid geometry_properties is missing `shells`".to_string())
            })?;
            let mut solids = Vec::with_capacity(nested.len());
            let mut offset: usize = 0;
            for shell_counts in &nested {
                let mut n: usize = 0;
                for &c in shell_counts {
                    n = n
                        .checked_add(c)
                        .ok_or_else(|| err("shells counts overflow usize".to_string()))?;
                }
                let end = offset
                    .checked_add(n)
                    .ok_or_else(|| err("shells counts overflow usize".to_string()))?;
                let slice = face_semantics.get(offset..end).ok_or_else(|| {
                    err(format!(
                        "shells describes more faces than face_semantics has ({} entries)",
                        face_semantics.len()
                    ))
                })?;
                solids.push(Value::Array(partition_face_semantics(slice, shell_counts)?));
                offset = end;
            }
            // Every face_semantics entry must be consumed — trailing entries
            // mean `shells` under-counts the faces, a corrupt package.
            if offset != face_semantics.len() {
                return Err(err(format!(
                    "shells account for {offset} faces but face_semantics has {} entries",
                    face_semantics.len()
                )));
            }
            Value::Array(solids)
        }
        GeometryType::GeometryInstance => return Ok(None),
    };
    Ok(Some(
        serde_json::json!({ "surfaces": surfaces, "values": values }),
    ))
}

/// Groups items by a string key, preserving first-appearance order.
struct OrderedGroups<T> {
    order: Vec<String>,
    index: HashMap<String, usize>,
    items: Vec<Vec<T>>,
}

impl<T> Default for OrderedGroups<T> {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            index: HashMap::new(),
            items: Vec::new(),
        }
    }
}

impl<T> OrderedGroups<T> {
    fn push(&mut self, key: String, item: T) {
        let idx = match self.index.get(&key) {
            Some(&i) => i,
            None => {
                let i = self.items.len();
                self.order.push(key.clone());
                self.items.push(Vec::new());
                self.index.insert(key, i);
                i
            }
        };
        self.items[idx].push(item);
    }

    /// Consumes the groups in first-appearance order.
    fn into_ordered(mut self) -> Vec<(String, Vec<T>)> {
        self.order
            .into_iter()
            .map(|key| {
                let idx = self.index[&key];
                (key, std::mem::take(&mut self.items[idx]))
            })
            .collect()
    }
}

/// The source CityJSON header's `metadata` object, when a writer chose to
/// keep it — folded into `city.other.source_metadata` (spec-alignment M3:
/// `source_metadata` is no longer its own footer key). **Informational
/// only**: nothing in this module's decode path may depend on this being
/// present — see [`build_header`]'s own header-metadata fallback and
/// [`synthesize_transform`], which never reads `other` at all.
pub(crate) fn source_metadata_from_other(meta: &CityMetadata) -> Option<Value> {
    meta.other.as_ref()?.get("source_metadata").cloned()
}

/// The header `CityJSON`'s `metadata.referenceSystem`, built from the dataset's
/// `crs` KV entry. As of G1 that entry is **PROJJSON**, so the OGC CRS URL is
/// rebuilt from its `id.{authority, code}` (e.g. `EPSG` + `7415` ->
/// `https://www.opengis.net/def/crs/EPSG/0/7415`). A legacy raw-URL-string
/// entry is still accepted. A CRS with no usable `id` yields no header field.
fn reference_system(meta: &CityMetadata) -> Result<Option<ReferenceSystem>> {
    let Some(crs) = &meta.crs else {
        return Ok(None);
    };
    // Legacy: a raw OGC CRS URL string.
    if let Some(url) = crs.as_str() {
        let rs = ReferenceSystem::from_url(url)
            .map_err(|e| err(format!("invalid referenceSystem URL {url:?}: {e}")))?;
        return Ok(Some(rs));
    }
    // PROJJSON: rebuild the OGC EPSG URL from the `id`.
    let id = &crs["id"];
    let authority = id["authority"].as_str();
    let code = match &id["code"] {
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    };
    match (authority, code.as_deref()) {
        (Some("EPSG"), Some(code)) => Ok(Some(crate::citygml::crs::reference_system(code))),
        // OGC:CRS84 / CRS84h are the only non-EPSG CRSs in the vendored table;
        // rebuild their registered OGC CRS URLs (versions per the OGC
        // definitions server) so a lon/lat package without source_metadata
        // still exports its referenceSystem rather than silently dropping it
        // (sol-review G1).
        (Some("OGC"), Some(code @ "CRS84")) => Ok(Some(ReferenceSystem::new(
            None,
            "OGC".to_string(),
            "1.3".to_string(),
            code.to_string(),
        ))),
        (Some("OGC"), Some(code @ "CRS84h")) => Ok(Some(ReferenceSystem::new(
            None,
            "OGC".to_string(),
            "0".to_string(),
            code.to_string(),
        ))),
        // Any other non-EPSG PROJJSON CRS has no CityJSON referenceSystem URL
        // form here.
        _ => Ok(None),
    }
}

/// Synthesises a quantisation `transform` for export, from the package's OWN
/// data rather than `city.other.transform` (spec "Informational only": a
/// reader/writer MUST NOT need `other` to decode the file). Fixed
/// millimetre-precision scale (`0.001`, the convention every real fixture in
/// this repo already uses), translated to the package's own spatial extent's
/// minimum corner — read back from the `bbox` column's Parquet row-group
/// statistics via [`crate::stac::package_bbox`], the SAME mechanism the STAC
/// derivation already uses, never `other`. A package with no geometry at all
/// (`package_bbox` returns `None`) has nothing to quantise, so `translate`
/// is the origin.
///
/// The exact scale/translate chosen need not match the SOURCE's own — the
/// comparator ([`crate::compare`]) derives its coordinate tolerance from
/// each side's own `transform.scale` (`max(scale_a, scale_b)`), so re-
/// quantising at a different, self-consistent precision never fails a
/// round-trip comparison; only genuinely lossy rounding would.
fn synthesize_transform(tables: &PackageTables) -> Result<Transform> {
    let scale = vec![0.001, 0.001, 0.001];
    let translate = match crate::stac::package_bbox(tables)? {
        Some(bbox) => vec![bbox.xmin, bbox.ymin, bbox.zmin],
        None => vec![0.0, 0.0, 0.0],
    };
    Ok(Transform { scale, translate })
}

/// Reconstructs the header `CityJSON` (empty `CityObjects`/`vertices`) from
/// the package's `CityMetadata` plus its own on-disk data (`tables`), which
/// [`synthesize_transform`] reads back rather than depending on `other`.
fn build_header(meta: &CityMetadata, tables: &PackageTables) -> Result<CityJSON> {
    let mut header = CityJSON::new();
    header.version = "2.0".to_string();
    header.transform = synthesize_transform(tables)?;
    if let Some(source_metadata) = source_metadata_from_other(meta) {
        // The stored source metadata already carries referenceSystem (and
        // everything else cjseq::Metadata can represent), so it supersedes
        // the referenceSystem-only fallback below. Informational only — see
        // `source_metadata_from_other`'s doc comment.
        header.metadata = Some(serde_json::from_value(source_metadata)?);
    } else if let Some(reference_system) = reference_system(meta)? {
        header.metadata = Some(CjMetadata {
            geographical_extent: None,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: Some(reference_system),
            title: None,
        });
    }
    header.extensions = meta.extensions.clone();
    Ok(header)
}

/// The reserved appearance columns for one theme prefix (`"material"` /
/// `"texture"`) in a batch, resolved ONCE per batch so the per-row reader need
/// not rescan the schema. Each entry pairs the column's array index with the
/// canonical LoD key it maps to — taken from the field's `cityparquet:lod`
/// metadata, or `""` for the transitional bare lod-less column.
///
/// A column qualifies only if it carries the reserved role (§5.1). This is
/// load-bearing: with per-LoD appearance columns the bare `material` /
/// `texture` names are no longer reserved, so a source ATTRIBUTE may legally
/// be named `material`, or `material_lod03` (which canonicalises to LoD 3 and
/// would otherwise collide with the real `material_lod3`). Classifying by the
/// reserved-role metadata rather than the name alone keeps such attributes out.
pub(crate) fn appearance_columns(batch: &RecordBatch, prefix: &str) -> Vec<(usize, String)> {
    batch
        .schema()
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            if field.metadata().get(ROLE_KEY).map(String::as_str) != Some(ROLE_RESERVED) {
                return None;
            }
            if field.name() == prefix {
                // Every LoD, including LoD0, is a suffixed column (spec
                // "Levels of detail"), so a bare `material`/`texture` name
                // reaching here is always the zero-analysis-geometry fallback
                // (no LoD to suffix by) — untagged, hence key `""`.
                let key = field.metadata().get(LOD_KEY).cloned().unwrap_or_default();
                return Some((index, key));
            }
            if field
                .name()
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with("_lod"))
            {
                // A reserved per-LoD appearance field always carries its
                // canonical LoD in `cityparquet:lod` (set by the schema), so
                // trust that rather than re-parsing a possibly non-canonical
                // suffix out of the column name.
                return field
                    .metadata()
                    .get(LOD_KEY)
                    .map(|lod| (index, lod.clone()));
            }
            None
        })
        .collect()
}

/// Rebuild an object row's appearance into an in-memory `{"<lod>": <theme
/// map>}` object from the pre-resolved appearance `columns` (see
/// [`appearance_columns`]), keyed by each column's canonical LoD. This is the
/// inverse of the encoder's per-LoD column layout: the downstream restore loop
/// looks appearance up by a geometry's canonical LoD (`lod.to_string()`), and
/// because both sides now derive that key from the same §9 column suffix, the
/// raw-vs-canonical key mismatch the single-column layout had to detect (the
/// former `appearance_lod_misses`) cannot arise. Returns `None` when the row
/// carries no appearance at all.
pub(crate) fn read_lod_keyed_appearance(
    batch: &RecordBatch,
    columns: &[(usize, String)],
    row: usize,
) -> Result<Option<Value>> {
    let mut map = serde_json::Map::new();
    for (index, lod_key) in columns {
        let col = batch.column(*index);
        if col.is_null(row) {
            continue;
        }
        let arr = col.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
            err(format!(
                "appearance column at index {index} is not a Utf8 array"
            ))
        })?;
        map.insert(lod_key.clone(), serde_json::from_str(arr.value(row))?);
    }
    Ok((!map.is_empty()).then_some(Value::Object(map)))
}

/// Whether the rebuilt `{"<lod>": {...}}` appearance map (see
/// [`read_lod_keyed_appearance`]) has an entry for the geometry at `lod_key`.
/// Used only on the Core-profile path (no `materials.parquet`/
/// `textures.parquet` sidecars listed in the manifest): the appearance
/// DEFINITIONS these maps index are not stored anywhere in that kind of
/// package, so export only ever CHECKS for the entry (to count the drop) and
/// never re-attaches it — re-attaching would leave dangling references and
/// invalid CityJSON. When the sidecars ARE listed, [`LocalAppearance`] owns
/// the actual re-attachment instead and this function is not consulted.
fn has_appearance_for_lod(map: &Option<Value>, lod_key: &str) -> bool {
    map.as_ref().is_some_and(|m| m.get(lod_key).is_some())
}

/// A *ring* is the innermost texture-map array. Pre-rewrite/pre-localisation
/// it is `[t, uv0, uv1, ...]` (all integers); post-rewrite (the main-table
/// form this module reads) it is `[t, [u, v], [u, v], ...]` — UV indices
/// inlined as coordinate pairs by [`crate::appearance::AppearanceInterner`].
/// Either way, a ring is recognised the same way: an array whose first
/// element is a number (the texture index) or `null` (no texture) — the
/// element shape of anything AFTER position 0 does not matter for
/// recognition, only for how [`LocalAppearance`] subsequently reads it. This
/// deliberately differs from `AppearanceInterner`'s own `is_texture_ring`,
/// which additionally requires every element to be non-array — a check that
/// would incorrectly reject the inlined `[u, v]` pairs this module's rings
/// contain.
fn is_localised_texture_ring(items: &[Value]) -> bool {
    !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null)
}

/// Per-feature slicer: collects the global appearance definitions a feature's
/// geometries reference, assigns feature-local indices (first-use order),
/// and re-interns each inlined `[u, v]` pair into a feature-local
/// vertices-texture pool (dedupe by `f64::to_bits` pair). The inverse of
/// [`crate::appearance::AppearanceInterner`]'s rewrite: that struct turns
/// feature-local indices into dataset-global ones (with UVs inlined) on the
/// way in; this one turns dataset-global ids (with UVs still inlined) back
/// into a fresh, self-contained feature-local slice on the way out.
struct LocalAppearance<'a> {
    global_materials: &'a [Value],
    global_textures: &'a [Value],
    local_materials: Vec<Value>,
    material_ids: HashMap<usize, usize>,
    local_textures: Vec<Value>,
    texture_ids: HashMap<usize, usize>,
    local_uvs: Vec<Vec<f64>>,
    uv_ids: HashMap<(u64, u64), usize>,
}

impl<'a> LocalAppearance<'a> {
    fn new(global_materials: &'a [Value], global_textures: &'a [Value]) -> Self {
        Self {
            global_materials,
            global_textures,
            local_materials: Vec::new(),
            material_ids: HashMap::new(),
            local_textures: Vec::new(),
            texture_ids: HashMap::new(),
            local_uvs: Vec::new(),
            uv_ids: HashMap::new(),
        }
    }

    /// The feature-local material index for dataset-global id `global_id`
    /// (assigning one, first-use order, if this is the first reference).
    /// `Schema` error naming the id and the loaded-definitions count when out
    /// of range.
    fn local_material_id(&mut self, global_id: usize) -> Result<usize> {
        if let Some(&id) = self.material_ids.get(&global_id) {
            return Ok(id);
        }
        let def = self.global_materials.get(global_id).ok_or_else(|| {
            err(format!(
                "material global id {global_id} out of range (loaded {} definitions)",
                self.global_materials.len()
            ))
        })?;
        let id = self.local_materials.len();
        self.local_materials.push(def.clone());
        self.material_ids.insert(global_id, id);
        Ok(id)
    }

    /// The feature-local texture index for dataset-global id `global_id` (see
    /// [`Self::local_material_id`]).
    fn local_texture_id(&mut self, global_id: usize) -> Result<usize> {
        if let Some(&id) = self.texture_ids.get(&global_id) {
            return Ok(id);
        }
        let def = self.global_textures.get(global_id).ok_or_else(|| {
            err(format!(
                "texture global id {global_id} out of range (loaded {} definitions)",
                self.global_textures.len()
            ))
        })?;
        let id = self.local_textures.len();
        self.local_textures.push(def.clone());
        self.texture_ids.insert(global_id, id);
        Ok(id)
    }

    /// The feature-local UV vertex-pool index for one inlined `[u, v]` pair,
    /// deduped by the pair's `f64::to_bits` representation.
    fn local_uv_id(&mut self, uv: [f64; 2]) -> usize {
        let key = (uv[0].to_bits(), uv[1].to_bits());
        if let Some(&id) = self.uv_ids.get(&key) {
            return id;
        }
        let id = self.local_uvs.len();
        self.local_uvs.push(vec![uv[0], uv[1]]);
        self.uv_ids.insert(key, id);
        id
    }

    /// Localise one geometry's `material` member (the per-LoD map entry —
    /// `{"<theme>": {"values": <nested global-ids|null>} | {"value":
    /// <global-id>}}`, dataset-global indices) into the same shape with
    /// feature-local indices.
    fn localise_material_map(&mut self, map: &Value) -> Result<Value> {
        let obj = map.as_object().ok_or_else(|| {
            err("material map must be a JSON object of theme -> {value|values}".to_string())
        })?;
        let mut out = serde_json::Map::with_capacity(obj.len());
        for (theme, inner) in obj {
            let inner_obj = inner
                .as_object()
                .ok_or_else(|| err(format!("material theme '{theme}' must be an object")))?;
            let mut new_inner = serde_json::Map::with_capacity(inner_obj.len());
            if let Some(v) = inner_obj.get("value") {
                new_inner.insert("value".to_string(), self.localise_material_index(v, theme)?);
            }
            if let Some(v) = inner_obj.get("values") {
                new_inner.insert("values".to_string(), self.localise_material_tree(v, theme)?);
            }
            out.insert(theme.clone(), Value::Object(new_inner));
        }
        Ok(Value::Object(out))
    }

    fn localise_material_tree(&mut self, v: &Value, theme: &str) -> Result<Value> {
        match v {
            Value::Array(items) => Ok(Value::Array(
                items
                    .iter()
                    .map(|x| self.localise_material_tree(x, theme))
                    .collect::<Result<Vec<_>>>()?,
            )),
            _ => self.localise_material_index(v, theme),
        }
    }

    fn localise_material_index(&mut self, v: &Value, theme: &str) -> Result<Value> {
        match v {
            Value::Null => Ok(Value::Null),
            Value::Number(n) => {
                let gid = n.as_u64().ok_or_else(|| {
                    err(format!(
                        "material index in theme '{theme}' is not a non-negative integer: {n}"
                    ))
                })? as usize;
                Ok(Value::from(self.local_material_id(gid)?))
            }
            other => Err(err(format!(
                "material index in theme '{theme}' must be an integer or null, got {other}"
            ))),
        }
    }

    /// Localise one geometry's `texture` member (the per-LoD map entry —
    /// `{"<theme>": {"values": <nested rings>}}`, where each innermost ring is
    /// `[global_t, [u, v], [u, v], ...]` with UVs already inlined by the
    /// encoder) into `[local_t, uv_idx0, uv_idx1, ...]` index form: the
    /// texture id becomes feature-local and every inlined pair is re-interned
    /// into the feature-local UV pool.
    fn localise_texture_map(&mut self, map: &Value) -> Result<Value> {
        let obj = map.as_object().ok_or_else(|| {
            err("texture map must be a JSON object of theme -> {values}".to_string())
        })?;
        let mut out = serde_json::Map::with_capacity(obj.len());
        for (theme, inner) in obj {
            let inner_obj = inner
                .as_object()
                .ok_or_else(|| err(format!("texture theme '{theme}' must be an object")))?;
            let values = inner_obj
                .get("values")
                .ok_or_else(|| err(format!("texture theme '{theme}' is missing 'values'")))?;
            let localised = self.localise_texture_tree(values, theme)?;
            let mut new_inner = serde_json::Map::with_capacity(1);
            new_inner.insert("values".to_string(), localised);
            out.insert(theme.clone(), Value::Object(new_inner));
        }
        Ok(Value::Object(out))
    }

    fn localise_texture_tree(&mut self, v: &Value, theme: &str) -> Result<Value> {
        match v {
            Value::Array(items) => {
                if is_localised_texture_ring(items) {
                    self.localise_texture_ring(items, theme)
                } else {
                    Ok(Value::Array(
                        items
                            .iter()
                            .map(|x| self.localise_texture_tree(x, theme))
                            .collect::<Result<Vec<_>>>()?,
                    ))
                }
            }
            other => Err(err(format!(
                "unexpected non-array node in texture theme '{theme}': {other}"
            ))),
        }
    }

    fn localise_texture_ring(&mut self, items: &[Value], theme: &str) -> Result<Value> {
        if items.len() == 1 && items[0].is_null() {
            return Ok(Value::Array(vec![Value::Null]));
        }
        let mut out = Vec::with_capacity(items.len());
        out.push(match &items[0] {
            Value::Null => Value::Null,
            Value::Number(n) => {
                let gid = n.as_u64().ok_or_else(|| {
                    err(format!(
                        "texture index in theme '{theme}' is not a non-negative integer: {n}"
                    ))
                })? as usize;
                Value::from(self.local_texture_id(gid)?)
            }
            other => {
                return Err(err(format!(
                    "texture index in theme '{theme}' must be an integer or null, got {other}"
                )));
            }
        });
        for uv in &items[1..] {
            let pair = uv.as_array().ok_or_else(|| {
                err(format!(
                    "inlined UV in theme '{theme}' must be a [u, v] pair, got {uv}"
                ))
            })?;
            if pair.len() != 2 {
                return Err(err(format!(
                    "inlined UV in theme '{theme}' must have exactly 2 coordinates, got {}",
                    pair.len()
                )));
            }
            let u = pair[0].as_f64().ok_or_else(|| {
                err(format!(
                    "inlined UV u-coordinate in theme '{theme}' is not a number: {}",
                    pair[0]
                ))
            })?;
            let v = pair[1].as_f64().ok_or_else(|| {
                err(format!(
                    "inlined UV v-coordinate in theme '{theme}' is not a number: {}",
                    pair[1]
                ))
            })?;
            out.push(Value::from(self.local_uv_id([u, v])));
        }
        Ok(Value::Array(out))
    }

    /// Consumes `self` into a `cjseq::Appearance` carrying exactly the
    /// materials/textures/UVs this feature's geometries referenced, plus
    /// `defaults`' `default-theme-material`/`default-theme-texture` (dataset-
    /// wide, so every feature gets the same ones when present). `None` when
    /// the feature referenced nothing at all AND `defaults` is `None` — a
    /// feature with no appearance of its own shouldn't grow an empty block.
    ///
    /// `local_uvs` MUST be part of this emptiness check, not just
    /// `local_materials`/`local_textures`: a legal `[null, uv0, uv1, ...]`
    /// texture ring (null texture index, real UV pairs — `localise_texture_ring`
    /// still walks and interns those UVs even though it never touches
    /// `local_textures` for a `null` index) can populate `local_uvs` while
    /// leaving `local_materials`/`local_textures` both empty. Without this
    /// check, such a feature would drop its whole `appearance` block here
    /// while its geometry still emits `vertices-texture` indices into
    /// that now-nonexistent block — dangling references, invalid CityJSON.
    fn into_appearance(self, defaults: Option<&Value>) -> Option<Appearance> {
        if self.local_materials.is_empty()
            && self.local_textures.is_empty()
            && self.local_uvs.is_empty()
            && defaults.is_none()
        {
            return None;
        }
        let default_theme_material = defaults
            .and_then(|d| d.get("default-theme-material"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let default_theme_texture = defaults
            .and_then(|d| d.get("default-theme-texture"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        Some(Appearance {
            materials: (!self.local_materials.is_empty()).then_some(self.local_materials),
            textures: (!self.local_textures.is_empty()).then_some(self.local_textures),
            vertices_texture: (!self.local_uvs.is_empty()).then_some(self.local_uvs),
            default_theme_texture,
            default_theme_material,
        })
    }
}

/// The rebuilt header `geometry-templates` plus the header-scope appearance
/// its templates' `material`/`texture` were localised into, and the join
/// table from a sidecar row's `id` (the string the main-table `template`
/// column's `id` references) to that template's position in
/// `templates`/the exported `GeometryInstance.template` index.
struct RebuiltTemplates {
    templates: Vec<Geometry>,
    vertices_templates: Value,
    appearance: Option<Appearance>,
    id_to_pos: HashMap<String, usize>,
}

/// Rebuilds the header's `geometry-templates` from `geometry_templates.parquet`
/// rows: each row's WKB decodes into a boundary tree via the SAME
/// [`reconstruct_boundaries`] the main geometry path uses, its vertices are
/// re-interned into a template-scope, RAW-float pool (never quantised — see
/// [`RawVertexInterner`]), and its `material`/`texture` (dataset-global ids,
/// same convention as the main table) are localised into ONE header-scope
/// [`LocalAppearance`] shared across every template — mirroring how a single
/// feature's geometries share one [`LocalAppearance`]. `rows` is assumed
/// non-empty (callers skip this entirely when no sidecar was loaded).
fn rebuild_templates(
    rows: &[TemplateRow],
    global_materials: &[Value],
    global_textures: &[Value],
) -> Result<RebuiltTemplates> {
    let mut interner = RawVertexInterner::default();
    let mut local_appearance = LocalAppearance::new(global_materials, global_textures);
    let mut templates = Vec::with_capacity(rows.len());
    let mut id_to_pos = HashMap::with_capacity(rows.len());

    for (pos, row) in rows.iter().enumerate() {
        id_to_pos.insert(row.id.clone(), pos);

        let decoded = wkb_to_geometry(&row.wkb)?;
        let vmap: Vec<usize> = decoded.coords.iter().map(|&c| interner.intern(c)).collect();

        let props = row.geometry_properties.as_ref();
        let gtype: GeometryType = props
            .and_then(|p| p.get("type"))
            .ok_or_else(|| {
                err(format!(
                    "geometry template {pos}: geometry_properties missing 'type'"
                ))
            })
            .and_then(|v| serde_json::from_value(v.clone()).map_err(CityParquetError::from))?;
        // Templates fold "lod" into geometry_properties (the main table
        // instead encodes it in the geometry COLUMN NAME) — see
        // `crate::package::build_template_rows`'s doc comment.
        let lod = props
            .and_then(|p| p.get("lod"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let boundaries = reconstruct_boundaries(&decoded.kind, &gtype, props, &vmap)?;
        let semantics = rebuild_semantics(props, &gtype)?;

        let material = row
            .material
            .as_ref()
            .map(|m| local_appearance.localise_material_map(m))
            .transpose()
            .map_err(|e| {
                err(format!(
                    "geometry template {pos}: cannot restore material appearance: {e}"
                ))
            })?
            .map(serde_json::from_value)
            .transpose()?;
        let texture = row
            .texture
            .as_ref()
            .map(|t| local_appearance.localise_texture_map(t))
            .transpose()
            .map_err(|e| {
                err(format!(
                    "geometry template {pos}: cannot restore texture appearance: {e}"
                ))
            })?
            .map(serde_json::from_value)
            .transpose()?;

        templates.push(Geometry {
            thetype: gtype,
            lod,
            boundaries,
            semantics,
            material,
            texture,
            template: None,
            transformation_matrix: None,
        });
    }

    let vertices_templates = serde_json::to_value(interner.finish())?;
    // No dataset-wide default-theme-material/texture at header/template
    // scope: those already get attached per-feature (see
    // `LocalAppearance::into_appearance`'s call site below), and a geometry
    // template itself never carries a default theme of its own.
    let appearance = local_appearance.into_appearance(None);

    Ok(RebuiltTemplates {
        templates,
        vertices_templates,
        appearance,
        id_to_pos,
    })
}

/// Export the CityParquet package at `opts.package_dir` back into CityJSON
/// or CityJSONSeq at `opts.output` (format chosen by extension: `.city.jsonl`
/// -> Seq, `.city.json` -> a single whole document via `cjseq::cjseq_to_cj`).
pub fn export(opts: &ExportOptions) -> Result<ExportReport> {
    let format = output_format(&opts.output)?;

    // `PackageTables::open` reads the manifest's `tables`/`sidecar_files`
    // lists and already rejects an empty or duplicate-naming manifest — see
    // its doc comment — so this is the SOLE place `export` learns the
    // package's file inventory from.
    let tables = PackageTables::open(&opts.package_dir)?;

    // The FIRST table supplies the package-level facts that ARE genuinely
    // dataset-wide — `transform`/`crs`/`extensions`/`source_metadata`/
    // `appearance_defaults` are written identically into every table's
    // footer by `crate::package`'s by-module writer (see
    // `crate::scan::ScanResult::metadata`, called once per conversion), so
    // any one table's copy is authoritative for header-building below.
    // What is NOT dataset-wide any more (spec "object-table-schema": "a
    // table carries exactly the LoD columns its data needs") is the
    // rendered Arrow schema and `attribute_columns`/`default_geometry` — a
    // module's own table only carries the geometry/appearance columns its
    // own rows need, so EVERY table (this one included) is read against its
    // OWN `cityparquet_metadata()`/`cityparquet_arrow_schema()` in the loop
    // below, never a shared one. `tables.tables` entries are already
    // absolute paths.
    let first_table_path = &tables.tables[0];
    let first_file = fs::File::open(first_table_path)
        .map_err(|e| io_err(format!("cannot open {}: {e}", first_table_path.display())))?;
    let first_builder = ParquetRecordBatchReaderBuilder::try_new(first_file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open parquet reader: {e}")))?;
    let meta = first_builder.cityparquet_metadata()?;
    let first_schema = first_builder.cityparquet_arrow_schema()?;
    let first_parquet_reader = first_builder
        .build()
        .map_err(|e| CityParquetError::Parquet(format!("cannot build parquet reader: {e}")))?;
    // Wrapped in `Option` so the objects-decode loop below can `.take()` it
    // for the `idx == 0` table without re-opening the file it already holds
    // open — the first object table's already-open reader is reused for
    // `idx == 0`, so it never pays for a second `File::open`/
    // `ParquetRecordBatchReaderBuilder`. Paired with `meta.clone()`: this
    // table's own metadata, exactly like every other table's own reader is
    // paired with its own metadata below.
    let mut first_reader = Some((
        CityParquetRecordBatchReader::new(first_parquet_reader, Arc::clone(&first_schema)),
        meta.clone(),
    ));

    let mut header = build_header(&meta, &tables)?;
    let (scale, translate) = transform_axes(&header.transform);

    // Whether this package carries the appearance-DEFINITION sidecars: when
    // it does (the Compatibility profile), the per-feature loop below
    // restores `material`/`texture` via `LocalAppearance` instead of
    // dropping the index maps (see the module doc). Loaded once, up front —
    // `read_materials`/`read_textures` already read as empty when the
    // corresponding file is absent, matching a package that only wrote one
    // of the two sidecars.
    let restore_appearance = tables
        .sidecar_files
        .iter()
        .any(|f| f == "materials.parquet" || f == "textures.parquet");
    let global_materials = if restore_appearance {
        read_materials(&opts.package_dir.join("materials.parquet"))?
    } else {
        Vec::new()
    };
    let global_textures = if restore_appearance {
        read_textures(&opts.package_dir.join("textures.parquet"))?
    } else {
        Vec::new()
    };

    // Whether this package carries the geometry-templates sidecar: gated by
    // the MANIFEST, not the file's mere presence on disk (M4 Codex-review
    // Finding 1) — same reasoning as `restore_appearance` above. When the
    // manifest doesn't list it (a Core-profile package, or a Compatibility
    // one with no templates at all), a `template` reference cannot be
    // resolved to anything real, so the per-object loop below falls back to
    // the counted-drop path instead (see the module doc); `template_rows`
    // stays empty and an unlisted-but-present file on disk is simply never
    // read. When the manifest DOES list it, the file must actually be there —
    // a manifest promise the package can't keep is a corrupt/truncated
    // package, not a silent 0-templates fallback.
    let templates_path = opts.package_dir.join("geometry_templates.parquet");
    let templates_listed = tables
        .sidecar_files
        .iter()
        .any(|f| f == "geometry_templates.parquet");
    let template_rows = if templates_listed {
        if !templates_path.exists() {
            return Err(io_err(format!(
                "package manifest lists 'geometry_templates.parquet' but {} does not exist",
                templates_path.display()
            )));
        }
        read_templates(&templates_path)?
    } else {
        Vec::new()
    };
    let template_id_to_pos: HashMap<String, usize> = if template_rows.is_empty() {
        HashMap::new()
    } else {
        let rebuilt = rebuild_templates(&template_rows, &global_materials, &global_textures)?;
        header.geometry_templates = Some(GeometryTemplates {
            templates: rebuilt.templates,
            vertices_templates: rebuilt.vertices_templates,
        });
        header.appearance = rebuilt.appearance;
        rebuilt.id_to_pos
    };

    // Group decoded objects by feature_id (own id fallback), preserving
    // first-appearance order for deterministic feature emission; carry each
    // object's row-local material/texture JSON alongside it since
    // `decode_batch` deliberately excludes those columns.
    let mut groups: OrderedGroups<(DecodedObject, Option<Value>, Option<Value>)> =
        OrderedGroups::default();
    let mut object_count = 0usize;
    // Iterate EVERY table the manifest lists (M5 task 5), not just the
    // first: decode order follows manifest order, and the feature grouping
    // above already tolerates one feature's objects arriving split across
    // batches (or, here, across whole tables) — see `OrderedGroups`.
    for (idx, table_path) in tables.tables.iter().enumerate() {
        let (reader, table_meta) = if idx == 0 {
            first_reader
                .take()
                .expect("the first table's reader is only ever consumed once")
        } else {
            let file = fs::File::open(table_path)
                .map_err(|e| io_err(format!("cannot open {}: {e}", table_path.display())))?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
                CityParquetError::Parquet(format!("cannot open parquet reader: {e}"))
            })?;
            // Every table must agree on `city.version` — a genuine
            // internal inconsistency (different writer versions cannot
            // safely coexist in one package) rather than the module-scoped
            // schema variation this fix otherwise makes legal.
            let table_meta = builder.cityparquet_metadata()?;
            if table_meta.version != meta.version {
                return Err(err(format!(
                    "table '{}' has city.version {:?}, expected {:?} \
                     (matching table '{}')",
                    table_display_name(table_path),
                    table_meta.version,
                    meta.version,
                    table_display_name(first_table_path)
                )));
            }
            // Each table is read and decoded against its OWN rendered Arrow
            // schema and its OWN `CityParquetMetadata` (never the first
            // table's, and never cross-checked against it) — a module's own
            // table legitimately carries only the geometry/appearance
            // columns its own rows need (spec "object-table-schema": "a
            // table carries exactly the LoD columns its data needs"), so two
            // tables in the SAME valid package can have genuinely different
            // schemas. `decode_batch` already derives geometry columns from
            // each batch's own schema; `table_meta.attribute_columns` is what
            // makes attribute decoding self-contained per table too.
            let table_schema = builder.cityparquet_arrow_schema()?;
            let parquet_reader = builder.build().map_err(|e| {
                CityParquetError::Parquet(format!("cannot build parquet reader: {e}"))
            })?;
            (
                CityParquetRecordBatchReader::new(parquet_reader, Arc::clone(&table_schema)),
                table_meta,
            )
        };
        for batch in reader {
            let batch = batch?;
            let material_cols = appearance_columns(&batch, "material");
            let texture_cols = appearance_columns(&batch, "texture");
            let objects = decode_batch(&batch, &table_meta)?;
            for (row, obj) in objects.into_iter().enumerate() {
                let material = read_lod_keyed_appearance(&batch, &material_cols, row)?;
                let texture = read_lod_keyed_appearance(&batch, &texture_cols, row)?;
                let key = obj.feature_id.clone().unwrap_or_else(|| obj.id.clone());
                object_count += 1;
                groups.push(key, (obj, material, texture));
            }
        }
    }

    let mut instance_geometries_dropped = 0usize;
    let mut appearance_refs_dropped = 0usize;
    let mut features: Vec<CityJSONFeature> = Vec::with_capacity(groups.items.len());
    for (feature_id, entries) in groups.into_ordered() {
        let mut feature = CityJSONFeature::new();
        feature.id = feature_id;
        let mut interner = VertexInterner::default();
        let mut local_appearance =
            restore_appearance.then(|| LocalAppearance::new(&global_materials, &global_textures));

        for (obj, material, texture) in &entries {
            let mut co = obj.object.clone();
            let mut geoms = Vec::with_capacity(obj.geometries.len());
            for (lod, decoded, props) in &obj.geometries {
                let gtype: GeometryType = props
                    .as_ref()
                    .and_then(|p| p.get("type"))
                    .ok_or_else(|| {
                        err(format!(
                            "object {}: geometry_properties missing 'type'",
                            obj.id
                        ))
                    })
                    .and_then(|v| {
                        serde_json::from_value(v.clone()).map_err(CityParquetError::from)
                    })?;

                let vmap = vertex_map(&decoded.coords, scale, translate, &mut interner);
                let boundaries =
                    reconstruct_boundaries(&decoded.kind, &gtype, props.as_ref(), &vmap)?;
                let semantics = rebuild_semantics(props.as_ref(), &gtype)?;

                // Look appearance up by this geometry's CANONICAL LoD. Both
                // this key and the map's keys (see `read_lod_keyed_appearance`)
                // derive from the same §9 column suffix, so the lookup is exact
                // — the raw-vs-canonical mismatch the single-column layout had
                // to detect (G20) is gone with the per-LoD columns.
                let lod_key = lod.map(|l| l.to_string()).unwrap_or_default();
                let mut geom_material: Option<HashMap<String, Material>> = None;
                let mut geom_texture: Option<HashMap<String, Texture>> = None;
                match local_appearance.as_mut() {
                    Some(local) => {
                        let material_hit = material.as_ref().and_then(|m| m.get(lod_key.as_str()));
                        let texture_hit = texture.as_ref().and_then(|t| t.get(lod_key.as_str()));
                        if let Some(m) = material_hit {
                            let localised = local.localise_material_map(m).map_err(|e| {
                                err(format!(
                                    "object {}: cannot restore material appearance: {e}",
                                    obj.id
                                ))
                            })?;
                            geom_material = Some(serde_json::from_value(localised)?);
                        }
                        if let Some(t) = texture_hit {
                            let localised = local.localise_texture_map(t).map_err(|e| {
                                err(format!(
                                    "object {}: cannot restore texture appearance: {e}",
                                    obj.id
                                ))
                            })?;
                            geom_texture = Some(serde_json::from_value(localised)?);
                        }
                    }
                    None => {
                        // Core profile (no appearance-definition sidecars):
                        // the index maps must be dropped, not re-attached —
                        // see `has_appearance_for_lod`'s docs.
                        if has_appearance_for_lod(material, &lod_key)
                            || has_appearance_for_lod(texture, &lod_key)
                        {
                            appearance_refs_dropped += 1;
                        }
                    }
                }

                geoms.push(Geometry {
                    thetype: gtype,
                    lod: lod.map(|l| l.to_string()),
                    boundaries,
                    semantics,
                    material: geom_material,
                    texture: geom_texture,
                    template: None,
                    transformation_matrix: None,
                });
            }
            if let Some(tpl) = &obj.template {
                if template_rows.is_empty() {
                    // No templates sidecar: the reference cannot be resolved
                    // to anything real (see the module doc) — count the drop,
                    // don't fabricate a geometry.
                    instance_geometries_dropped += 1;
                } else {
                    let pos = template_id_to_pos.get(&tpl.id).copied().ok_or_else(|| {
                        err(format!(
                            "object {}: template id {:?} does not name a row in geometry_templates.parquet",
                            obj.id, tpl.id
                        ))
                    })?;
                    let point_idx = interner.intern(quantise(tpl.point, scale, translate));
                    geoms.push(Geometry {
                        thetype: GeometryType::GeometryInstance,
                        lod: None,
                        boundaries: serde_json::to_value(vec![point_idx])?,
                        semantics: None,
                        material: None,
                        texture: None,
                        template: Some(pos),
                        transformation_matrix: tpl.transformation_matrix.clone(),
                    });
                }
            }
            co.geometry = if geoms.is_empty() { None } else { Some(geoms) };
            feature.add_co(obj.id.clone(), co);
        }

        feature.vertices = interner.finish();
        feature.appearance = match local_appearance {
            Some(local) => local.into_appearance(meta.appearance_defaults.as_ref()),
            // No appearance-definition sidecars restored for this package
            // (the Core profile, or a Compatibility one with no
            // materials/textures at all): there is no per-feature slicer to
            // consult, but dataset-wide defaults are still real KV metadata
            // that must not be lost just because there is nothing else to
            // attach alongside them (M5 debt item 2, rule half (b)). A throw-
            // away, never-fed `LocalAppearance` reuses `into_appearance`'s
            // own emptiness/defaults rule rather than duplicating it: with
            // both global slices empty it can only ever produce a
            // defaults-only block (or `None` when there are no defaults).
            None => meta.appearance_defaults.as_ref().and_then(|defaults| {
                LocalAppearance::new(&[], &[]).into_appearance(Some(defaults))
            }),
        };
        features.push(feature);
    }

    let feature_count = features.len();
    write_output(format, &opts.output, header, features)?;

    Ok(ExportReport {
        feature_count,
        object_count,
        instance_geometries_dropped,
        appearance_refs_dropped,
    })
}

fn write_output(
    format: OutputFormat,
    output: &std::path::Path,
    header: CityJSON,
    features: Vec<CityJSONFeature>,
) -> Result<()> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| io_err(format!("cannot create {}: {e}", parent.display())))?;
    }
    let mut file = fs::File::create(output)
        .map_err(|e| io_err(format!("cannot create {}: {e}", output.display())))?;
    match format {
        OutputFormat::Seq => {
            writeln!(file, "{}", serde_json::to_string(&header)?)
                .map_err(|e| io_err(format!("write error: {e}")))?;
            for feature in &features {
                writeln!(file, "{}", serde_json::to_string(feature)?)
                    .map_err(|e| io_err(format!("write error: {e}")))?;
            }
        }
        OutputFormat::Doc => {
            let doc = cjseq::cjseq_to_cj(header, features);
            write!(file, "{}", serde_json::to_string(&doc)?)
                .map_err(|e| io_err(format!("write error: {e}")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wkb_write::{VertexPool, geometry_to_wkb};

    fn triangle_faces(n: usize) -> Vec<Vec<Vec<usize>>> {
        (0..n).map(|i| vec![vec![i, i + 1, i + 2]]).collect()
    }

    fn bare_material_schema(lod_tag: Option<&str>) -> RecordBatch {
        use arrow_array::ArrayRef;
        use arrow_schema::{DataType, Field, Schema};
        use std::sync::Arc;
        let mut meta = std::collections::HashMap::new();
        meta.insert(ROLE_KEY.to_string(), ROLE_RESERVED.to_string());
        if let Some(l) = lod_tag {
            meta.insert(LOD_KEY.to_string(), l.to_string());
        }
        let field = Field::new("material", DataType::Utf8, true).with_metadata(meta);
        let schema = Arc::new(Schema::new(vec![field]));
        let col: ArrayRef = Arc::new(StringArray::from(vec![None::<&str>]));
        RecordBatch::try_new(schema, vec![col]).unwrap()
    }

    #[test]
    fn appearance_columns_maps_bare_lod0_material_to_key_zero() {
        // The LoD0 footprint's bare `material` column is tagged lod "0", so it
        // must pair with the geometry's recovered LoD "0" — not the "" fallback,
        // which would silently drop LoD0 appearance on export.
        let batch = bare_material_schema(Some("0"));
        assert_eq!(
            appearance_columns(&batch, "material"),
            vec![(0, "0".to_string())]
        );
    }

    #[test]
    fn appearance_columns_maps_untagged_bare_material_to_empty_key() {
        // The zero-analysis-geometry fallback bare column carries no lod tag.
        let batch = bare_material_schema(None);
        assert_eq!(
            appearance_columns(&batch, "material"),
            vec![(0, String::new())]
        );
    }

    #[test]
    fn partition_shells_rejects_a_count_face_mismatch() {
        // counts [2, 1] describe 3 faces; handing them 4 faces must be a
        // Schema error naming both numbers, never a silent mis-partition
        // that drops the 4th face.
        let err = partition_shells(triangle_faces(4), Some(&[2, 1])).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected Schema error, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains('3') && msg.contains('4'),
            "error must name the mismatched counts, got: {msg}"
        );
        // The matching case still partitions.
        let shells = partition_shells(triangle_faces(3), Some(&[2, 1])).unwrap();
        assert_eq!(shells.len(), 2);
        assert_eq!((shells[0].len(), shells[1].len()), (2, 1));
    }

    /// A hand-built MultiSolid (no fixture carries one): 2 solids — the
    /// first with 2 single-face shells, the second with 1 — written through
    /// the real WKB writer, read back through the real WKB reader, and
    /// reconstructed with the nested `shells`. Vertices are used
    /// in ascending first-appearance order so both the WKB reader's interner
    /// and export's `VertexInterner` assign identity indices, making the
    /// reconstructed boundary tree comparable to the source verbatim.
    #[test]
    fn multisolid_reconstruction_round_trips_through_wkb() {
        let vertices: Vec<Vec<i64>> = vec![
            vec![0, 0, 0],
            vec![1000, 0, 0],
            vec![1000, 1000, 0],
            vec![0, 1000, 0],
            vec![0, 0, 1000],
        ];
        let transform = Transform {
            scale: vec![1.0; 3],
            translate: vec![0.0; 3],
        };
        let pool = VertexPool::new(&vertices, &transform);
        let boundaries = serde_json::json!([[[[[0, 1, 2, 3]]], [[[0, 1, 4]]]], [[[[1, 2, 4]]]]]);
        let geom = Geometry {
            thetype: GeometryType::MultiSolid,
            lod: Some("2".into()),
            boundaries: boundaries.clone(),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let bytes = geometry_to_wkb(&geom, &pool).unwrap().unwrap().bytes;
        let decoded = wkb_to_geometry(&bytes).unwrap();

        let mut interner = VertexInterner::default();
        let vmap = vertex_map(&decoded.coords, [1.0; 3], [0.0; 3], &mut interner);
        let props = serde_json::json!({
            "type": "MultiSolid",
            "shells": [[1, 1], [1]],
        });
        let rebuilt = reconstruct_boundaries(
            &decoded.kind,
            &GeometryType::MultiSolid,
            Some(&props),
            &vmap,
        )
        .unwrap();
        assert_eq!(
            rebuilt, boundaries,
            "MultiSolid boundary tree must round-trip verbatim"
        );
        assert_eq!(
            interner.finish(),
            vertices,
            "identity re-quantisation must reproduce the source vertex pool"
        );
    }

    #[test]
    fn multisolid_with_too_few_nested_counts_is_an_error_not_a_panic() {
        // GeometryCollection of 2 members but shells lists only 1
        // solid: must be a Schema error, not an index-out-of-bounds panic.
        let member = DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]);
        let kind = DecodedKind::GeometryCollection(vec![member.clone(), member]);
        let props = serde_json::json!({"type": "MultiSolid", "shells": [[1]]});
        let err =
            reconstruct_boundaries(&kind, &GeometryType::MultiSolid, Some(&props), &[0, 1, 2])
                .unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected Schema error, got {err:?}"
        );
    }

    #[test]
    fn geometry_instance_type_in_properties_is_an_error_not_a_panic() {
        // A hand-rolled/corrupt file could label any WKB cell
        // "GeometryInstance" in geometry_properties; the encoder never
        // stores WKB for instances, so this shape mismatch must surface as
        // an error, not a crash.
        let err = reconstruct_boundaries(
            &DecodedKind::MultiPoint(vec![0]),
            &GeometryType::GeometryInstance,
            None,
            &[0],
        )
        .unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected Schema error, got {err:?}"
        );
    }

    /// spec-alignment M3: `transform` is no longer read from `city.other` at
    /// all — `build_header` always SYNTHESISES its own quantisation
    /// transform (see `synthesize_transform`), so a package missing it
    /// (or missing `other` altogether) is no longer an error. Real-fixture
    /// coverage that `build_header`'s output round-trips lives in
    /// `tests/export_real_data.rs`; see `crate::stac::mod::package_bbox` for
    /// the on-disk mechanism `synthesize_transform` reads.
    #[test]
    fn source_metadata_from_other_is_none_when_other_is_absent() {
        let meta = CityMetadata::new();
        assert!(source_metadata_from_other(&meta).is_none());
    }

    #[test]
    fn reference_system_rebuilds_the_ogc_crs84_url() {
        // sol-review G1: metadata `crs` is now PROJJSON. A package whose CRS is
        // OGC:CRS84 (a lon/lat dataset) with no source_metadata must still
        // export a `referenceSystem`, not silently drop it.
        let meta = CityMetadata {
            crs: Some(serde_json::json!({
                "type": "GeographicCRS",
                "name": "WGS 84 (CRS84)",
                "id": { "authority": "OGC", "code": "CRS84" }
            })),
            ..CityMetadata::new()
        };
        let rs = reference_system(&meta)
            .expect("resolution must not error")
            .expect("OGC:CRS84 must yield a referenceSystem");
        assert_eq!(rs.to_url(), "https://www.opengis.net/def/crs/OGC/1.3/CRS84");
    }

    /// M4 final-review Fix 4: a legal `[null, [u, v], ...]` texture ring —
    /// null texture index, real inlined UV pairs — must keep `into_appearance`
    /// from dropping the whole `appearance` block, even though it never
    /// touches `local_materials`/`local_textures` (only `local_uvs`, via
    /// `localise_texture_ring`'s UV-interning loop, which runs regardless of
    /// whether the ring's own texture index is `null`). Before this fix the
    /// emptiness check ignored `local_uvs` entirely, so this exact case
    /// returned `None` here while the geometry it came from still emitted
    /// `vertices-texture` index references — dangling references into a
    /// nonexistent `appearance` block, invalid CityJSON.
    #[test]
    fn into_appearance_is_not_none_when_only_local_uvs_are_populated() {
        let mut local = LocalAppearance::new(&[], &[]);
        let map = serde_json::json!({
            "visual": {"values": [[null, [0.1, 0.2], [0.3, 0.4]]]}
        });
        let localised = local.localise_texture_map(&map).unwrap();
        // Precondition: the localise pass really did populate local_uvs
        // while leaving local_materials/local_textures empty.
        assert!(local.local_materials.is_empty());
        assert!(local.local_textures.is_empty());
        assert_eq!(local.local_uvs.len(), 2, "both UV pairs must be interned");
        // The localised ring itself keeps its null texture index and now
        // references the interned UV pool by position.
        assert_eq!(
            localised["visual"]["values"][0],
            serde_json::json!([null, 0, 1])
        );

        let appearance = local
            .into_appearance(None)
            .expect("local_uvs alone must be enough to keep the appearance block");
        assert_eq!(
            appearance.vertices_texture,
            Some(vec![vec![0.1, 0.2], vec![0.3, 0.4]])
        );
        assert!(appearance.materials.is_none());
        assert!(appearance.textures.is_none());
    }

    /// M5 debt item 2, rule half (a): an EMPTY slicer (a feature/template
    /// that referenced no material/texture/UV at all) with dataset-wide
    /// `defaults` present must still return `Some`, carrying ONLY the
    /// default-theme members — `into_appearance`'s `Some iff referenced OR
    /// defaults exist` rule.
    #[test]
    fn into_appearance_with_an_empty_slicer_and_defaults_is_some_with_only_default_members() {
        let local = LocalAppearance::new(&[], &[]);
        let defaults = serde_json::json!({
            "default-theme-material": "theme-a",
            "default-theme-texture": "theme-b",
        });
        let appearance = local
            .into_appearance(Some(&defaults))
            .expect("defaults alone must be enough to keep the appearance block");
        assert!(appearance.materials.is_none());
        assert!(appearance.textures.is_none());
        assert!(appearance.vertices_texture.is_none());
        assert_eq!(
            appearance.default_theme_material.as_deref(),
            Some("theme-a")
        );
        assert_eq!(appearance.default_theme_texture.as_deref(), Some("theme-b"));
    }

    /// M5 debt item 2, rule half (b): an empty slicer with NO defaults must
    /// return `None` — a feature/template that referenced nothing must not
    /// grow an appearance block out of nowhere.
    #[test]
    fn into_appearance_with_an_empty_slicer_and_no_defaults_is_none() {
        let local = LocalAppearance::new(&[], &[]);
        assert!(local.into_appearance(None).is_none());
    }
}
