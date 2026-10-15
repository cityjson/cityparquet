//! A package exports to CityGML 2.0 or 3.0, and the document holds what its
//! source did.
//!
//! Each source converts to a package, which exports to a `.gml` file, valid
//! when `CITYGML_TOOLS` names `citygml-tools` to check it with; the file
//! compares equal with the source, read back by the same CityGML
//! readers `convert` uses. The sources are real: the PLATEAU tiles the
//! `citygml2` crate keeps (with their ADEs, read beside the i-UR schemas),
//! the `citygml3` crate's CityGML 3.0 files, a LoD 4 building, and 3DBAG's
//! Delft tile as CityJSON, whose LoDs and booleans CityGML cannot hold.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cityparquet::citygml::{read_document, sniff_citygml};
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{CityGmlVersion, ExportOptions, ExportReport, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::Source;

fn citygml2_data(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../citygml-rs/crates/citygml2/tests/data")
        .join(path)
}

fn citygml3_data(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../citygml-rs/crates/citygml3/tests/data")
        .join(path)
}

fn data(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(path)
}

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(path)
}

/// The relative `xsi:schemaLocation` of each ADE schema a CityGML `source`
/// uses, by namespace, made to name the same file from `out_dir` too: each
/// schema's directory is linked where the location reaches from there. The
/// document written into `out_dir` names the schemas as `source` does, so
/// reading it back types the ADEs' values and names their schemas as
/// reading `source` did.
fn ade_schemas_beside(source: &Path, out_dir: &Path) -> BTreeMap<String, String> {
    let Some(version) = sniff_citygml(source) else {
        return BTreeMap::new();
    };
    let document = read_document(source, &version).unwrap();
    let mut schemas = BTreeMap::new();
    for ade in document.ade().ades() {
        let Some(location) = ade.schema_location.as_deref() else {
            continue;
        };
        let schema = source.parent().unwrap().join(location);
        if location.contains("://") || !schema.is_file() {
            continue;
        }
        let link = lexical(&out_dir.join(location));
        let link = link.parent().unwrap();
        if !link.exists() {
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(schema.parent().unwrap().canonicalize().unwrap(), link)
                .unwrap();
        }
        schemas.insert(ade.namespace.clone(), location.to_string());
    }
    schemas
}

/// `path` with its `..` components applied.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Converts `source` into a package in `dir` and exports it as `version` to
/// a document deep enough inside `dir` that the source's relative schema
/// locations stay inside it.
fn export_gml(source: &Path, dir: &Path, version: CityGmlVersion) -> (PathBuf, ExportReport) {
    let pkg = dir.join("pkg");
    convert(&ConvertOptions::new(source.to_path_buf(), pkg.clone())).unwrap();
    let out_dir = dir.join("a/b/c/d");
    let out = out_dir.join("out.gml");
    let mut options = ExportOptions::new(pkg, out.clone());
    options.citygml.version = version;
    options.citygml.ade_schemas = ade_schemas_beside(source, &out_dir);
    let report = export(&options).unwrap();
    validate(&out);
    (out, report)
}

/// `path` is valid against the CityGML schemas and its ADEs', as
/// `citygml-tools validate` says, when `CITYGML_TOOLS` names that program.
fn validate(path: &Path) {
    let Some(program) = std::env::var_os("CITYGML_TOOLS") else {
        return;
    };
    let output = std::process::Command::new(program)
        .arg("validate")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} is not valid:\n{}",
        path.display(),
        String::from_utf8_lossy(&output.stdout)
    );
}

/// [`export_gml`], then the document must equal `source`, with nothing
/// left out.
fn round_trip(source: &Path, version: CityGmlVersion) -> ExportReport {
    let dir = tempfile::tempdir().unwrap();
    let (out, report) = export_gml(source, dir.path(), version);
    let cmp = compare_datasets(source, &out, &CompareOptions::default()).unwrap();
    assert!(cmp.equal, "{:#?}", cmp.differences);
    let citygml = report.citygml.as_ref().expect("a CityGML report");
    assert!(citygml.losses.is_empty(), "{:?}", citygml.losses);
    assert_eq!(citygml.features, report.feature_count);
    report
}

#[test]
fn a_plateau_building_tile_exports_to_citygml_2() {
    round_trip(
        &citygml2_data("plateau/udx/bldg/53394519_bldg_6697_op.gml"),
        CityGmlVersion::V2_0,
    );
}

#[test]
fn a_plateau_road_tile_exports_to_citygml_2() {
    round_trip(
        &citygml2_data("plateau/udx/tran/53394529_tran_6697_op.gml"),
        CityGmlVersion::V2_0,
    );
}

#[test]
fn a_plateau_bridge_tile_exports_to_citygml_2() {
    round_trip(
        &citygml2_data("plateau/udx/brid/53394610_brid_6697_op.gml"),
        CityGmlVersion::V2_0,
    );
}

/// Every object of the tile is of an ADE class, in the ADE's own table.
#[test]
fn a_plateau_ade_tile_exports_to_citygml_2() {
    round_trip(
        &citygml2_data("plateau/udx/urf/533946_urf_6697_08_op.gml"),
        CityGmlVersion::V2_0,
    );
}

/// Groups whose members are features of their own, and trees placed by
/// implicit geometry.
#[test]
fn groups_and_implicit_geometries_export_to_citygml_2() {
    round_trip(
        &citygml2_data("railway_trees_fragment.gml"),
        CityGmlVersion::V2_0,
    );
}

#[test]
fn a_lod4_building_exports_to_citygml_2() {
    round_trip(&citygml2_data("lod4_building_v2.gml"), CityGmlVersion::V2_0);
}

/// Buildings with solids at LoD 2, and attributes of every kind.
#[test]
fn lod2_solids_and_attributes_export_to_citygml_2() {
    round_trip(&data("savenow_ingolstadt_lod2.gml"), CityGmlVersion::V2_0);
}

/// A railway's MultiSurfaces with semantic surfaces.
#[test]
fn semantic_multisurfaces_export_to_citygml_2() {
    round_trip(&data("railway_lod3_fragment.gml"), CityGmlVersion::V2_0);
}

/// A composite solid whose faces are its boundary surfaces'.
#[test]
fn a_composite_solid_exports_to_citygml_2() {
    round_trip(&data("b1_lod2_cs_w_sem.gml"), CityGmlVersion::V2_0);
}

#[test]
fn building_parts_export_to_citygml_2() {
    round_trip(&data("building_with_parts.gml"), CityGmlVersion::V2_0);
}

/// Materials, and textures with their coordinates, on the faces they
/// colour.
#[test]
fn appearances_export_to_both_versions() {
    for version in [CityGmlVersion::V2_0, CityGmlVersion::V3_0] {
        round_trip(&data("building_with_materials.gml"), version);
        round_trip(&data("building_with_appearance.gml"), version);
    }
}

/// spec "Column naming and reservation rules": an attribute stored under an
/// escaped column name is written under its source name. The Helsinki
/// building carries `ID` next to the reserved `id`; the package stores it as
/// `_ID`, and the CityGML document names it `ID` again.
#[test]
fn an_escaped_attribute_is_written_under_its_source_name() {
    let dir = tempfile::tempdir().unwrap();
    let (out, _) = export_gml(
        &data("address_location.city.jsonl"),
        dir.path(),
        CityGmlVersion::V2_0,
    );
    let gml = std::fs::read_to_string(&out).unwrap();
    assert!(gml.contains(r#"name="ID""#));
    assert!(!gml.contains(r#"name="_ID""#));
}

#[test]
fn a_citygml_3_building_tile_exports_to_citygml_3() {
    round_trip(
        &citygml3_data("plateau/53394519_bldg_6697_op.gml"),
        CityGmlVersion::V3_0,
    );
}

/// The ADE's properties go back into the `adeOf…` hook they came from,
/// which only the package's extension declaration still names.
#[test]
fn a_citygml_3_ade_exports_to_citygml_3() {
    round_trip(
        &citygml3_data("ogc/test-ade/industrial_building_dataset_v3.gml"),
        CityGmlVersion::V3_0,
    );
}

/// Groups and implicit geometries in the other version's encoding.
#[test]
fn a_citygml_2_document_exports_to_citygml_3() {
    round_trip(
        &citygml2_data("railway_trees_fragment.gml"),
        CityGmlVersion::V3_0,
    );
}

/// 3DBAG's tile holds more than CityGML can: LoDs 1.2, 1.3 and 2.2, of which
/// CityGML keeps one geometry per whole LoD, boolean attributes, which it
/// writes as text, and, for CityGML 2.0, LoD 1 semantic surfaces. The report counts exactly these, and every object
/// reads back with the attributes it has a value for.
#[test]
fn a_cityjson_tile_exports_with_what_citygml_cannot_hold_counted() {
    let source = fixture("delft.city.jsonl");
    let objects = |path: &Path| -> BTreeMap<String, Vec<String>> {
        let source = Source::open(path).unwrap();
        let mut objects = BTreeMap::new();
        for feature in source.features().unwrap() {
            for (id, object) in feature.unwrap().city_objects {
                // A package holds a null attribute as no attribute.
                let names = object
                    .attributes
                    .map(|a| {
                        a.iter()
                            .filter(|(_, v)| !v.is_null())
                            .map(|(k, _)| k.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                objects.insert(id, names);
            }
        }
        objects
    };
    let before = objects(&source);
    for version in [CityGmlVersion::V2_0, CityGmlVersion::V3_0] {
        let dir = tempfile::tempdir().unwrap();
        let (out, report) = export_gml(&source, dir.path(), version);
        let citygml = report.citygml.unwrap();
        assert_eq!(citygml.features, report.feature_count);
        let mut lost = vec![
            "generic attributes whose value is a boolean (written as text)",
            "geometries at a LoD their object already has one of that kind at",
            "geometries whose LoD is written as its whole number",
        ];
        // CityGML 2.0's boundary surfaces have no LoD 1 geometry.
        if version == CityGmlVersion::V2_0 {
            lost.insert(
                0,
                "faces of boundary surfaces whose class has no geometry at their LoD \
                 (kept untagged)",
            );
        }
        let losses: Vec<&str> = citygml.losses.iter().map(|(what, _)| what).collect();
        assert_eq!(losses, lost, "{version:?}");
        assert_eq!(objects(&out), before, "{version:?}");
    }
}

/// CityGML 3.0 has no LoD 4: the building's LoD 4 geometries, on objects
/// with none at LoD 3, are written at LoD 3, and the report says so.
#[test]
fn citygml_3_reports_lod4_geometries_written_at_lod3() {
    let dir = tempfile::tempdir().unwrap();
    let (_, report) = export_gml(
        &citygml2_data("lod4_building_v2.gml"),
        dir.path(),
        CityGmlVersion::V3_0,
    );
    let losses = &report.citygml.unwrap().losses;
    assert!(
        losses
            .iter()
            .any(|(what, n)| what.contains("LoD 4") && n > 0),
        "{losses:?}"
    );
}

/// A CityJSON document's group holds its members, which a CityGML 3.0
/// group only refers to: they are written as objects of their own, and
/// counted, and read back as the group's members.
#[test]
fn a_group_holding_its_members_exports_to_both_versions() {
    let source = fixture("lod3_railway.city.json");
    for version in [CityGmlVersion::V2_0, CityGmlVersion::V3_0] {
        let dir = tempfile::tempdir().unwrap();
        let (out, report) = export_gml(&source, dir.path(), version);
        let citygml = report.citygml.unwrap();
        let referred = citygml
            .losses
            .iter()
            .find(|(what, _)| *what == "group members written as objects of their own, referred to")
            .map_or(0, |(_, n)| n);
        match version {
            CityGmlVersion::V2_0 => assert_eq!(referred, 0),
            CityGmlVersion::V3_0 => assert!(referred > 0),
        }
        let source = Source::open(&out).unwrap();
        let groups: Vec<usize> = source
            .features()
            .unwrap()
            .flat_map(|f| f.unwrap().city_objects.into_values())
            .filter(|o| String::from(o.thetype.clone()) == "CityObjectGroup")
            .map(|o| o.children.unwrap_or_default().len())
            .collect();
        assert_eq!(groups, [14], "{version:?}");
    }
}
