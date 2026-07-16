//! W-M5 appearance round-trip oracles.
//!
//! CityGML → package → `.gml` → package, comparing DEREFERENCED appearance
//! (material/texture DEFINITIONS per face, not table indices — index/pool order
//! legitimately permutes on re-intern). Materials: `building_with_materials.gml`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use arrow_array::{Array, StringArray};
use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::PackageManifest;
use cityparquet::schema::Profile;
use cityparquet::sidecar::read_materials;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
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

/// Per `(object id, lod, theme)`: the per-face material DEFINITION (canonical
/// JSON string), `None` where the face has no material — dereferenced through the
/// package's `materials.parquet`, so the comparison is index-permutation
/// independent.
fn face_materials(pkg: &Path) -> BTreeMap<(String, String, String), Vec<Option<String>>> {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();
    let table = read_materials(&pkg.join("materials.parquet")).unwrap();

    let mut out = BTreeMap::new();
    for name in &manifest.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(pkg.join(name)).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            let material_col = batch
                .column_by_name("material")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let objs = decode_batch(&batch, &meta).unwrap();
            for (row, obj) in objs.iter().enumerate() {
                let Some(col) = material_col else { continue };
                if col.is_null(row) {
                    continue;
                }
                let mat: Value = serde_json::from_str(col.value(row)).unwrap();
                for (lod, themes) in mat.as_object().unwrap() {
                    for (theme, inner) in themes.as_object().unwrap() {
                        let mut leaves = Vec::new();
                        flatten(inner.get("values").unwrap(), &mut leaves);
                        let deref: Vec<Option<String>> = leaves
                            .iter()
                            .map(|o| o.map(|g| serde_json::to_string(&table[g]).unwrap()))
                            .collect();
                        out.insert((obj.id.clone(), lod.clone(), theme.clone()), deref);
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
        .map(|m| serde_json::to_string(m).unwrap())
        .collect()
}

#[test]
fn materials_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    let mut opts = ConvertOptions::new(data_fixture("building_with_materials.gml"), pkg.clone());
    opts.profile = Profile::Compatibility;
    convert(&opts).unwrap();
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

    let mut opts2 = ConvertOptions::new(out_gml.clone(), pkg2.clone());
    opts2.profile = Profile::Compatibility;
    convert(&opts2).unwrap();

    let before = face_materials(&pkg);
    let after = face_materials(&pkg2);

    // Sanity: BM's lod2 "visual" theme is [red, red, green, null] by definition.
    let visual = before
        .get(&("BM".to_string(), "2".to_string(), "visual".to_string()))
        .expect("BM lod2 visual materials");
    assert_eq!(visual.len(), 4, "four faces");
    assert!(
        visual[0].as_ref().unwrap().contains("\"red\""),
        "{visual:?}"
    );
    assert_eq!(visual[0], visual[1], "p0, p1 share red");
    assert!(
        visual[2].as_ref().unwrap().contains("\"green\""),
        "{visual:?}"
    );
    assert_eq!(visual[3], None, "p3 untargeted");

    assert_eq!(before, after, "per-face material definitions survive");
    assert_eq!(
        material_set(&pkg),
        material_set(&pkg2),
        "materials table equal as a canonical-JSON set"
    );
    assert_eq!(material_set(&pkg).len(), 2, "only red + green in the table");
}
