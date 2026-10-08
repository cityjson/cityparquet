//! A real CityGML 2.0 LoD4 building — the SIG3D "Simple 3D city model LOD4"
//! example as citygml4j ships it — survives CityGML → package → CityGML and
//! package → CityJSON at LoD 4.
//!
//! Its `bldg:lod4Solid` composes its shell from `xlink:href`s to the
//! `gml:CompositeSurface` each `boundedBy` surface wraps its polygons in, so
//! reading it also proves an xlink to a surface aggregate resolves to the
//! aggregate's member polygons.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow_array::cast::AsArray;
use arrow_array::{Array, RecordBatch};
use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::validate::validate_package;
use cityparquet::wkb_read::{DecodedKind, wkb_to_geometry};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn convert_to(input: &Path, out: &Path) {
    convert(&ConvertOptions::new(input.to_path_buf(), out.to_path_buf()))
        .unwrap_or_else(|e| panic!("convert {}: {e}", input.display()));
}

fn building_batches(pkg: &Path) -> Vec<RecordBatch> {
    let file = File::open(pkg.join("building.parquet")).unwrap();
    ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The LoD4 geometry of the `Building` row: its CM type and WKB face count.
fn building_lod4(pkg: &Path) -> (String, usize) {
    for batch in building_batches(pkg) {
        let types = batch.column_by_name("object_type").unwrap();
        let types = arrow_cast_strings(types);
        let geometry = batch
            .column_by_name("geometry_lod4_0")
            .expect("a geometry_lod4_0 column")
            .as_binary::<i32>();
        let props = batch
            .column_by_name("geometry_properties_lod4_0")
            .unwrap()
            .as_struct();
        let cm_types = props.column_by_name("type").unwrap().as_string::<i32>();
        for (i, ty) in types.iter().enumerate() {
            if ty != "Building" || geometry.is_null(i) {
                continue;
            }
            let decoded = wkb_to_geometry(geometry.value(i)).unwrap();
            let DecodedKind::PolyhedralSurface(faces) = decoded.kind else {
                panic!("the LoD4 solid is a PolyhedralSurface")
            };
            return (cm_types.value(i).to_string(), faces.len());
        }
    }
    panic!("no Building row with LoD4 geometry");
}

fn arrow_cast_strings(array: &arrow_array::ArrayRef) -> Vec<String> {
    let dict = array.as_dictionary::<arrow_array::types::Int32Type>();
    let values = dict.values().as_string::<i32>();
    dict.keys()
        .iter()
        .map(|k| values.value(k.unwrap() as usize).to_string())
        .collect()
}

#[test]
fn a_citygml_lod4_solid_round_trips_through_the_package() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    convert_to(&fixture("lod4_building_v2.gml"), &pkg);

    let (cm_type, faces) = building_lod4(&pkg);
    assert_eq!(cm_type, "Solid");
    // Ten shell members, each an xlink to a boundedBy surface's
    // CompositeSurface of several polygons.
    assert!(
        faces > 10,
        "the composites expand to their polygons: {faces}"
    );
    let report = validate_package(&pkg).unwrap();
    assert!(report.is_conformant(), "{report}");

    // Package -> CityGML keeps the LoD4 solid as `bldg:lod4Solid`, and that
    // document reads back to the same LoD4 geometry.
    let gml = dir.path().join("written.gml");
    write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: gml.clone(),
    })
    .unwrap();
    assert!(
        std::fs::read_to_string(&gml)
            .unwrap()
            .contains("<bldg:lod4Solid>")
    );
    let again = dir.path().join("again");
    convert_to(&gml, &again);
    assert_eq!(building_lod4(&again), ("Solid".to_string(), faces));

    // Package -> CityJSON writes the LoD as "4.0" and counts it, so the CLI
    // can warn that CityJSON 2.0 defines no LoD 4.
    let json = dir.path().join("exported.city.json");
    let exported = export(&ExportOptions {
        package_dir: pkg,
        output: json.clone(),
    })
    .unwrap();
    assert!(
        std::fs::read_to_string(&json)
            .unwrap()
            .contains("\"lod\":\"4.0\"")
    );
    assert!(exported.lod4_geometries >= 1, "{exported:?}");
}
