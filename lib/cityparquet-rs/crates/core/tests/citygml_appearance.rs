//! W-M5 appearance round-trip oracles.
//!
//! CityGML → package → `.gml` → package, comparing DEREFERENCED appearance
//! (material/texture DEFINITIONS per face, not table indices — index/pool order
//! legitimately permutes on re-intern). Materials: `building_with_materials.gml`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow_array::{Array, MapArray, RecordBatch};
use cityparquet::appearance_columns::{material_cell_value, texture_cell_value};
use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert, convert_source};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::Lod;
use cityparquet::sidecar::{read_materials, read_textures};
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use cityparquet::wkb_read::{DecodedGeometry, DecodedKind};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// These committed CityGML fixtures carry no `srsName`/envelope at all.
/// Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), the FIRST convert of a raw fixture
/// injects one onto the REAL parsed header — the writer
/// (`crate::citygml::writer`) then emits `srsName` into the round-tripped
/// `.gml`'s own envelope, so a SECOND convert of that output resolves a CRS
/// natively (plain `compat`/`convert`, no injection needed).
fn compat_with_crs(input: PathBuf, out: PathBuf) {
    let raw = Source::open(&input).unwrap();
    let mut header = raw.header().clone();
    header
        .metadata
        .get_or_insert(cityparquet::cjseq::Metadata {
            geographical_extent: None,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: None,
            title: None,
        })
        .reference_system = Some(cityparquet::citygml::crs::reference_system("7415"));
    let features: Vec<_> = raw.features().unwrap().map(|f| f.unwrap()).collect();
    let src = Source::from_parts(
        header,
        features,
        raw.doc_appearance().cloned(),
        raw.format(),
    );
    convert_source(&src, &ConvertOptions::new(input, out)).unwrap();
}

/// Flatten a material `values` tree's leaves (a non-negative integer, or `null`)
/// in DFS (face-walk) order.
fn flatten(v: &Value, out: &mut Vec<Option<usize>>) {
    match v {
        Value::Array(items) => items.iter().for_each(|it| flatten(it, out)),
        Value::Number(n) => out.push(Some(n.as_u64().unwrap() as usize)),
        _ => out.push(None),
    }
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// One ring canonicalised to a rotation-invariant vertex sequence (mm-quantised,
/// rotated to start at its minimum vertex) so a face can be a stable key
/// independent of the round-trip's face reordering.
fn canonical_ring(ring: &[usize], coords: &[[f64; 3]]) -> String {
    let t: Vec<(i64, i64, i64)> = ring
        .iter()
        .map(|&i| (mm(coords[i][0]), mm(coords[i][1]), mm(coords[i][2])))
        .collect();
    let rotated = match (0..t.len()).min_by_key(|&i| t[i]) {
        Some(m) => {
            let mut r = t[m..].to_vec();
            r.extend_from_slice(&t[..m]);
            r
        }
        None => t,
    };
    format!("{rotated:?}")
}

/// A geometry's faces in walk order, each a `Vec` of its canonical ring strings
/// (exterior first). The appearance `values` trees flatten in this same order, so
/// zipping pairs each face/ring with its geometry — letting the oracle key by
/// geometry and stay order-independent (MultiSurface faces reorder via boundedBy).
fn geometry_faces(dec: &DecodedGeometry) -> Vec<Vec<String>> {
    fn walk(kind: &DecodedKind, coords: &[[f64; 3]], out: &mut Vec<Vec<String>>) {
        match kind {
            DecodedKind::PolyhedralSurface(fs) | DecodedKind::MultiPolygon(fs) => {
                for f in fs {
                    out.push(f.iter().map(|r| canonical_ring(r, coords)).collect());
                }
            }
            DecodedKind::GeometryCollection(ms) => ms.iter().for_each(|m| walk(m, coords, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&dec.kind, &dec.coords, &mut out);
    out
}

/// Flatten a texture `values` tree to `[face][ring-leaf]` in walk order (each face
/// an array of its ring leaves), collapsing the shell/solid nesting.
fn texture_faces(v: &Value, out: &mut Vec<Vec<Value>>) {
    if let Value::Array(items) = v {
        let is_face = items.first().is_some_and(|f| {
            matches!(f, Value::Array(a) if matches!(a.first(), Some(Value::Number(_)) | Some(Value::Null) | None))
        });
        if is_face {
            out.push(items.clone());
        } else {
            items.iter().for_each(|it| texture_faces(it, out));
        }
    }
}

/// Per `(object id, lod, theme)`: the per-face material DEFINITION (canonical
/// JSON string), `None` where the face has no material — dereferenced through the
/// package's `materials.parquet`, so the comparison is index-permutation
/// independent.
type FaceMaterials = BTreeMap<(String, String, String), Vec<(String, Option<String>)>>;

/// Rebuild the `{"<lod>": {"<theme>": …}}` per-object appearance map from the
/// per-LoD `material_lod*` / `texture_lod*` MAP columns (§11.1, G20), so these
/// face-level assertions keep their LoD-keyed view. Each cell is read through
/// the shared column readers and flattened to its CityJSON-shaped JSON.
/// `None` when the row has no appearance for `prefix`.
fn lod_keyed_appearance(batch: &RecordBatch, prefix: &str, row: usize) -> Option<Value> {
    let mut map = serde_json::Map::new();
    for (i, field) in batch.schema().fields().iter().enumerate() {
        let Some(suffix) = field
            .name()
            .strip_prefix(prefix)
            .and_then(|r| r.strip_prefix('_'))
        else {
            continue;
        };
        let Some(lod) = Lod::from_column_suffix(suffix) else {
            continue;
        };
        let Some(col) = batch.column(i).as_any().downcast_ref::<MapArray>() else {
            continue;
        };
        let cell = match prefix {
            "material" => material_cell_value(col, row).unwrap(),
            _ => texture_cell_value(col, row).unwrap(),
        };
        let Some(cell) = cell else { continue };
        map.insert(lod.to_string(), cell);
    }
    (!map.is_empty()).then_some(Value::Object(map))
}

fn face_materials(pkg: &Path) -> FaceMaterials {
    let tables = PackageTables::open(pkg).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(File::open(&tables.tables[0]).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap();
    let table = defs_only(read_materials(&pkg.join("materials.parquet")).unwrap());

    let mut out = BTreeMap::new();
    for path in &tables.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            let objs = decode_batch(&batch, &meta).unwrap();
            for (row, obj) in objs.iter().enumerate() {
                let Some(mat) = lod_keyed_appearance(&batch, "material", row) else {
                    continue;
                };
                // Per-LoD geometry faces (canonical), to key materials by face.
                let faces_by_lod: BTreeMap<String, Vec<Vec<String>>> = obj
                    .geometries
                    .iter()
                    .map(|(lod, dec, _)| {
                        (
                            lod.as_ref().map(|l| l.to_string()).unwrap_or_default(),
                            geometry_faces(dec),
                        )
                    })
                    .collect();
                for (lod, themes) in mat.as_object().unwrap() {
                    let faces = faces_by_lod.get(lod).cloned().unwrap_or_default();
                    for (theme, inner) in themes.as_object().unwrap() {
                        let mut leaves = Vec::new();
                        if let Some(values) = inner.get("values") {
                            flatten(values, &mut leaves);
                        } else if let Some(v) = inner.get("value") {
                            // Scalar (whole-geometry) → expand to every face.
                            let g = v.as_u64().unwrap() as usize;
                            leaves = vec![Some(g); faces.len()];
                        }
                        // Pair each face's material def with its (canonical) face,
                        // then sort — order-independent (faces reorder on round-trip).
                        let mut pairs: Vec<(String, Option<String>)> = leaves
                            .iter()
                            .enumerate()
                            .map(|(fi, o)| {
                                let key = faces.get(fi).map(|f| f.join("|")).unwrap_or_default();
                                let def = o.map(|g| serde_json::to_string(&table[g]).unwrap());
                                (key, def)
                            })
                            .collect();
                        pairs.sort();
                        out.insert((obj.id.clone(), lod.clone(), theme.clone()), pairs);
                    }
                }
            }
        }
    }
    out
}

/// The package's materials table as a canonical-JSON set (order-independent).
fn material_set(pkg: &Path) -> BTreeSet<String> {
    read_materials(&pkg.join("materials.parquet"))
        .unwrap()
        .iter()
        .map(|(_, m)| serde_json::to_string(m).unwrap())
        .collect()
}

fn texture_set(pkg: &Path) -> BTreeSet<String> {
    read_textures(&pkg.join("textures.parquet"))
        .unwrap()
        .iter()
        .map(|(_, t)| serde_json::to_string(t).unwrap())
        .collect()
}

/// The definitions alone: the sidecar readers now return each definition
/// paired with its `id`, but these comparisons are about payloads.
fn defs_only(rows: Vec<(i64, Value)>) -> Vec<Value> {
    rows.into_iter().map(|(_, def)| def).collect()
}

/// Dereference a texture `values` tree: replace each ring leaf's texture INDEX
/// (position 0) with the texture DEFINITION (canonical JSON string), keeping the
/// inlined UV pairs — so the comparison is index-permutation independent.
fn deref_texture(node: &Value, table: &[Value]) -> Value {
    match node {
        Value::Array(items) => match items.first() {
            Some(Value::Number(n)) => {
                let tid = n.as_u64().unwrap() as usize;
                let mut out = vec![Value::String(serde_json::to_string(&table[tid]).unwrap())];
                out.extend(items[1..].iter().cloned());
                Value::Array(out)
            }
            Some(Value::Null) => node.clone(),
            _ => Value::Array(items.iter().map(|i| deref_texture(i, table)).collect()),
        },
        _ => node.clone(),
    }
}

type FaceTextures = BTreeMap<(String, String, String), Vec<(String, usize, String)>>;

/// Per `(object id, lod, theme)`: a SORTED list of `(canonical face, ring index,
/// dereferenced ring leaf)` — the texture def + inlined UVs of each textured ring,
/// keyed by its face geometry so the comparison is order- and index-independent.
fn face_textures(pkg: &Path) -> FaceTextures {
    let tables = PackageTables::open(pkg).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(File::open(&tables.tables[0]).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap();
    let table = defs_only(read_textures(&pkg.join("textures.parquet")).unwrap());

    let mut out = BTreeMap::new();
    for path in &tables.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            let objs = decode_batch(&batch, &meta).unwrap();
            for (row, obj) in objs.iter().enumerate() {
                let Some(tex) = lod_keyed_appearance(&batch, "texture", row) else {
                    continue;
                };
                let faces_by_lod: BTreeMap<String, Vec<Vec<String>>> = obj
                    .geometries
                    .iter()
                    .map(|(lod, dec, _)| {
                        (
                            lod.as_ref().map(|l| l.to_string()).unwrap_or_default(),
                            geometry_faces(dec),
                        )
                    })
                    .collect();
                for (lod, themes) in tex.as_object().unwrap() {
                    let faces = faces_by_lod.get(lod).cloned().unwrap_or_default();
                    for (theme, inner) in themes.as_object().unwrap() {
                        let mut flat_faces = Vec::new();
                        texture_faces(inner.get("values").unwrap(), &mut flat_faces);
                        let mut entries: Vec<(String, usize, String)> = Vec::new();
                        for (fi, rings) in flat_faces.iter().enumerate() {
                            let key = faces.get(fi).map(|f| f.join("|")).unwrap_or_default();
                            for (ri, ring) in rings.iter().enumerate() {
                                // Skip an untextured ring ([null]); record textured ones.
                                if matches!(
                                    ring.as_array().and_then(|a| a.first()),
                                    Some(Value::Number(_))
                                ) {
                                    let d = serde_json::to_string(&deref_texture(ring, &table))
                                        .unwrap();
                                    entries.push((key.clone(), ri, d));
                                }
                            }
                        }
                        entries.sort();
                        out.insert((obj.id.clone(), lod.clone(), theme.clone()), entries);
                    }
                }
            }
        }
    }
    out
}

#[test]
fn materials_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    compat_with_crs(data_fixture("building_with_materials.gml"), pkg.clone());
    let report = write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: out_gml.clone(),
    })
    .unwrap();
    // red (2 faces) + green (1 face) emitted; the unused "blue" was dropped at
    // encode (unreferenced feature-local materials never reach the package).
    assert_eq!(report.materials_written, 2);
    assert_eq!(report.material_geometries_dropped, 0);
    assert_eq!(report.appearance_skipped_core_profile, 0);

    let opts2 = ConvertOptions::new(out_gml.clone(), pkg2.clone());
    convert(&opts2).unwrap();

    let before = face_materials(&pkg);
    let after = face_materials(&pkg2);

    // Sanity: BM's lod2 "visual" theme has 2 red faces, 1 green, 1 untargeted.
    let visual = before
        .get(&("BM".to_string(), "2.0".to_string(), "visual".to_string()))
        .expect("BM lod2 visual materials");
    assert_eq!(visual.len(), 4, "four faces");
    let defs: Vec<&Option<String>> = visual.iter().map(|(_, d)| d).collect();
    let has = |n: &str| {
        defs.iter()
            .filter(|d| d.as_ref().is_some_and(|s| s.contains(n)))
            .count()
    };
    assert_eq!(has("\"red\""), 2, "{visual:?}");
    assert_eq!(has("\"green\""), 1, "{visual:?}");
    assert_eq!(defs.iter().filter(|d| d.is_none()).count(), 1, "{visual:?}");

    assert_eq!(before, after, "per-face material definitions survive");
    assert_eq!(
        material_set(&pkg),
        material_set(&pkg2),
        "materials table equal as a canonical-JSON set"
    );
    assert_eq!(material_set(&pkg).len(), 2, "only red + green in the table");
}

fn compat(input: PathBuf, out: PathBuf) {
    let opts = ConvertOptions::new(input, out);
    convert(&opts).unwrap();
}

fn workspace_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// The real `lod3_railway.city.json` fixture (a CityJSON file, unlike the
/// committed `.gml` fixtures [`compat_with_crs`] handles) carries no
/// `referenceSystem` either. Writes a small on-disk COPY with a CRS injected
/// via JSON mutation of the real fixture — never hand-written CityJSON.
fn railway_fixture_with_crs() -> (tempfile::TempDir, PathBuf) {
    let mut doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(workspace_fixture("lod3_railway.city.json")).unwrap(),
    )
    .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_with_crs.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    (dir, path)
}

#[test]
fn full_appearance_round_trip() {
    // building_with_appearance.gml: materials (red/green) + a texture on p0_r0.
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    compat_with_crs(data_fixture("building_with_appearance.gml"), pkg.clone());
    let report = write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: out_gml.clone(),
    })
    .unwrap();
    assert_eq!(report.materials_written, 2);
    assert_eq!(report.textures_written, 1);
    assert_eq!(report.texture_geometries_dropped, 0);
    compat(out_gml.clone(), pkg2.clone());

    // Sanity: BA's lod2 "visual" texture dereferences to the wall.jpg JPG def.
    let bt = face_textures(&pkg);
    let visual = bt
        .get(&("BA".to_string(), "2.0".to_string(), "visual".to_string()))
        .expect("BA lod2 visual texture");
    assert_eq!(visual.len(), 1, "one textured ring");
    assert!(visual[0].2.contains("wall.jpg"), "{visual:?}");
    assert!(
        visual[0].2.contains("\\\"type\\\":\\\"JPG\\\""),
        "{visual:?}"
    );

    assert_eq!(
        face_materials(&pkg),
        face_materials(&pkg2),
        "per-face material defs survive"
    );
    assert_eq!(
        face_textures(&pkg),
        face_textures(&pkg2),
        "per-ring texture defs + UVs survive"
    );
    assert_eq!(material_set(&pkg), material_set(&pkg2));
    assert_eq!(texture_set(&pkg), texture_set(&pkg2));
}

#[test]
fn lod3_railway_building_appearance_round_trip() {
    // Real 3DCityDB export as the independent-authoring anchor for the appearance
    // round-trip on non-synthetic data. The CityGML writer is Building-only (W-M1
    // scope), so of railway's 121 objects across 14 types it emits only the 3
    // `Building`s (lod3 MultiSurface: one material+texture, two texture-only) and
    // drops the rest — so the round-trip is verified for exactly those Buildings.
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    compat(railway_path, pkg.clone());
    write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: out_gml.clone(),
    })
    .unwrap();
    compat(out_gml.clone(), pkg2.clone());

    let before_mat = face_materials(&pkg);
    let after_mat = face_materials(&pkg2);
    let before_tex = face_textures(&pkg);
    let after_tex = face_textures(&pkg2);

    // The writer-emitted objects (⊆ originals) carry real appearance and must
    // round-trip it bit-exactly, keyed by face geometry.
    assert!(
        !after_tex.is_empty(),
        "railway Buildings' textures survive the round-trip"
    );
    for (k, v) in &after_mat {
        assert_eq!(before_mat.get(k), Some(v), "material {k:?}");
    }
    for (k, v) in &after_tex {
        assert_eq!(before_tex.get(k), Some(v), "texture {k:?}");
    }
}
