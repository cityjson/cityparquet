//! Export: a CityParquet package directory back into CityJSON/CityJSONSeq.
//! Inverse of [`crate::package::convert`] at the whole-package level, built
//! on top of [`crate::decode::decode_batch`] (row -> `cjseq`-model object,
//! geometry deliberately excluded there) plus this module's own geometry
//! reconstruction (WKB -> CityJSON boundary arrays, re-quantised against the
//! dataset's own `transform`).
//!
//! `GeometryInstance` geometries are dropped: their template definitions live
//! in the M4 compatibility-profile sidecars, which this (Core-profile-only)
//! pass does not read, so a file referencing an absent template would be
//! invalid CityJSON. The owning object (attributes, hierarchy) is still
//! exported; only its instance geometry is missing. Material/texture index
//! maps are dropped for the same reason: the appearance DEFINITIONS they
//! index also live in the M4 sidecars, so exporting the maps alone would
//! leave dangling references. Both drops are counted in [`ExportReport`].

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use arrow_array::{Array, RecordBatch, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

use cityparquet_schema::{CityParquetError, CityParquetMetadata, PackageManifest, Result};
use cjseq::{
    CityJSON, CityJSONFeature, Geometry, GeometryType, Metadata as CjMetadata, ReferenceSystem,
    Transform,
};

use crate::decode::{DecodedObject, decode_batch};
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use crate::wkb_read::DecodedKind;

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
    /// Objects whose `GeometryInstance` geometry was dropped (template
    /// definitions are M4 sidecar data this pass does not read).
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
fn partition_shells(
    faces: Vec<Vec<Vec<usize>>>,
    counts: Option<&[usize]>,
) -> Result<Vec<Vec<Vec<Vec<usize>>>>> {
    match counts {
        Some(counts) => {
            let total: usize = counts.iter().sum();
            if total != faces.len() {
                return Err(err(format!(
                    "solid_shell_faces counts sum to {total} but the stored geometry has {} faces",
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

/// `solid_shell_faces` from `geometry_properties`, shaped for a single
/// `Solid` (flat per-shell face counts).
fn shell_faces_flat(props: Option<&Value>) -> Result<Option<Vec<usize>>> {
    let Some(v) = props.and_then(|p| p.get("solid_shell_faces")) else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(v.clone())?))
}

/// `solid_shell_faces` from `geometry_properties`, shaped for
/// `MultiSolid`/`CompositeSolid` (one flat per-shell face-count list per
/// solid).
fn shell_faces_nested(props: Option<&Value>) -> Result<Option<Vec<Vec<usize>>>> {
    let Some(v) = props.and_then(|p| p.get("solid_shell_faces")) else {
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
                                    "solid_shell_faces lists {} solids but the stored \
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

/// The header `CityJSON`'s `metadata.referenceSystem`, built from the
/// dataset's `crs` KV entry (an OGC CRS URL string) when present.
fn reference_system(meta: &CityParquetMetadata) -> Result<Option<ReferenceSystem>> {
    let Some(crs) = &meta.crs else {
        return Ok(None);
    };
    let Some(url) = crs.as_str() else {
        // Only the plain OGC CRS URL string form is supported (M2/M3 scope
        // cut); a PROJJSON object CRS has no equivalent header field here.
        return Ok(None);
    };
    let rs = ReferenceSystem::from_url(url)
        .map_err(|e| err(format!("invalid referenceSystem URL {url:?}: {e}")))?;
    Ok(Some(rs))
}

/// Reconstructs the header `CityJSON` (empty `CityObjects`/`vertices`) from
/// the package's `CityParquetMetadata`.
fn build_header(meta: &CityParquetMetadata) -> Result<CityJSON> {
    let mut header = CityJSON::new();
    header.version = "2.0".to_string();
    // The transform is REQUIRED: export re-quantises every coordinate
    // against it, so a package without one cannot be exported faithfully —
    // silently substituting the identity transform would corrupt every
    // vertex.
    let transform = meta.transform.as_ref().ok_or_else(|| {
        CityParquetError::Metadata(
            "package metadata carries no 'transform'; cannot re-quantise for export".to_string(),
        )
    })?;
    header.transform = serde_json::from_value(transform.clone())?;
    if let Some(source_metadata) = &meta.source_metadata {
        // The stored source metadata already carries referenceSystem (and
        // everything else cjseq::Metadata can represent), so it supersedes
        // the referenceSystem-only fallback below.
        header.metadata = Some(serde_json::from_value(source_metadata.clone())?);
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

/// One row's `material`/`texture` JSON (the encoder's `{"<lod>": {...}}`
/// per-object map), parsed once per batch alongside `decode_batch`'s own
/// pass — `decode_batch` deliberately excludes these (see its module docs),
/// so export reads them straight off the batch.
fn row_json_object(batch: &RecordBatch, name: &str, row: usize) -> Result<Option<Value>> {
    let Some(col) = batch.column_by_name(name) else {
        return Ok(None);
    };
    if col.is_null(row) {
        return Ok(None);
    }
    let arr = col
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| err(format!("column '{name}' is not a Utf8 array")))?;
    Ok(Some(serde_json::from_str(arr.value(row))?))
}

/// Whether the object-level `{"<lod>": {...}}` appearance map (the encoder's
/// per-LoD keying of `material`/`texture`) has an entry for the geometry at
/// `lod_key`. Export only ever CHECKS for the entry — it never re-attaches
/// it: the Core profile stores these index maps but not the appearance
/// definitions they point into (M4 sidecar data), so re-attaching them would
/// leave dangling references and invalid CityJSON. M4's compatibility
/// profile, which reads the sidecars, owns the actual re-attachment.
fn has_appearance_for_lod(map: &Option<Value>, lod_key: &str) -> bool {
    map.as_ref().is_some_and(|m| m.get(lod_key).is_some())
}

/// Export the CityParquet package at `opts.package_dir` back into CityJSON
/// or CityJSONSeq at `opts.output` (format chosen by extension: `.city.jsonl`
/// -> Seq, `.city.json` -> a single whole document via `cjseq::cjseq_to_cj`).
pub fn export(opts: &ExportOptions) -> Result<ExportReport> {
    let format = output_format(&opts.output)?;

    let manifest_path = opts.package_dir.join("metadata.json");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| io_err(format!("cannot read {}: {e}", manifest_path.display())))?;
    let manifest: PackageManifest = serde_json::from_str(&manifest_text)?;
    let table_name = manifest
        .tables
        .first()
        .ok_or_else(|| err("package manifest lists no tables".to_string()))?;
    let table_path = opts.package_dir.join(table_name);

    let file = fs::File::open(&table_path)
        .map_err(|e| io_err(format!("cannot open {}: {e}", table_path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open parquet reader: {e}")))?;
    let meta = builder.cityparquet_metadata()?;
    let schema = builder.cityparquet_arrow_schema()?;
    let parquet_reader = builder
        .build()
        .map_err(|e| CityParquetError::Parquet(format!("cannot build parquet reader: {e}")))?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let header = build_header(&meta)?;
    let (scale, translate) = transform_axes(&header.transform);

    // Group decoded objects by feature_id (own id fallback), preserving
    // first-appearance order for deterministic feature emission; carry each
    // object's row-local material/texture JSON alongside it since
    // `decode_batch` deliberately excludes those columns.
    let mut groups: OrderedGroups<(DecodedObject, Option<Value>, Option<Value>)> =
        OrderedGroups::default();
    let mut object_count = 0usize;
    for batch in reader {
        let batch = batch?;
        let objects = decode_batch(&batch, &meta)?;
        for (row, obj) in objects.into_iter().enumerate() {
            let material = row_json_object(&batch, "material", row)?;
            let texture = row_json_object(&batch, "texture", row)?;
            let key = obj.feature_id.clone().unwrap_or_else(|| obj.id.clone());
            object_count += 1;
            groups.push(key, (obj, material, texture));
        }
    }

    let mut instance_geometries_dropped = 0usize;
    let mut appearance_refs_dropped = 0usize;
    let mut features: Vec<CityJSONFeature> = Vec::with_capacity(groups.items.len());
    for (feature_id, entries) in groups.into_ordered() {
        let mut feature = CityJSONFeature::new();
        feature.id = feature_id;
        let mut interner = VertexInterner::default();

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
                let semantics = props.as_ref().and_then(|p| p.get("semantics")).cloned();

                // Keyed by the CANONICAL Lod string: the encoder keyed the
                // per-object appearance map by the raw source lod string
                // (`geom.lod.clone().unwrap_or_default()`), so a
                // non-canonical source LoD (e.g. "02", which `Lod::parse`
                // normalises to "2") would not match here — a documented
                // limitation (it would only undercount the drops); both real
                // fixtures use canonical strings only.
                let lod_key = lod.map(|l| l.to_string()).unwrap_or_default();
                if has_appearance_for_lod(material, &lod_key)
                    || has_appearance_for_lod(texture, &lod_key)
                {
                    appearance_refs_dropped += 1;
                }

                geoms.push(Geometry {
                    thetype: gtype,
                    lod: lod.map(|l| l.to_string()),
                    boundaries,
                    semantics,
                    // Deliberately dropped (counted above): the definitions
                    // these maps index live in M4 sidecars; see
                    // `has_appearance_for_lod`'s docs.
                    material: None,
                    texture: None,
                    template: None,
                    transformation_matrix: None,
                });
            }
            if obj.template.is_some() {
                instance_geometries_dropped += 1;
            }
            co.geometry = if geoms.is_empty() { None } else { Some(geoms) };
            feature.add_co(obj.id.clone(), co);
        }

        feature.vertices = interner.finish();
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
    use crate::wkb_read::wkb_to_geometry;
    use crate::wkb_write::{VertexPool, geometry_to_wkb};

    fn triangle_faces(n: usize) -> Vec<Vec<Vec<usize>>> {
        (0..n).map(|i| vec![vec![i, i + 1, i + 2]]).collect()
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
    /// reconstructed with the nested `solid_shell_faces`. Vertices are used
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
            "solid_shell_counts": [2, 1],
            "solid_shell_faces": [[1, 1], [1]],
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
        // GeometryCollection of 2 members but solid_shell_faces lists only 1
        // solid: must be a Schema error, not an index-out-of-bounds panic.
        let member = DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]);
        let kind = DecodedKind::GeometryCollection(vec![member.clone(), member]);
        let props = serde_json::json!({"type": "MultiSolid", "solid_shell_faces": [[1]]});
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

    #[test]
    fn build_header_requires_a_transform() {
        let meta = CityParquetMetadata {
            cityparquet_version: "0.1.0".to_string(),
            source_format: cityparquet_schema::SourceFormat::CityJsonSeq,
            source_version: None,
            crs: None,
            transform: None,
            extensions: None,
            attribute_columns: vec![],
            reserved_columns: vec![],
            default_geometry: "geometry".to_string(),
            bbox_column: "bbox".to_string(),
            sidecar_files: vec![],
            source_metadata: None,
            appearance_defaults: None,
        };
        let err = build_header(&meta).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Metadata(_)),
            "a package without a transform cannot be re-quantised: expected Metadata error, got {err:?}"
        );
    }
}
