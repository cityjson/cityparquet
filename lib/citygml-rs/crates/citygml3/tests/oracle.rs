//! The reader against an oracle it did not write: `citygml-tools
//! to-cityjson` (citygml4j) on the same CityGML 3.0 files, committed under
//! `tests/data/oracle`, each reduced to what both converters must agree on
//! and the two reductions compared whole.

#![cfg(feature = "cjseq")]

#[path = "../../../test-support/oracle.rs"]
mod support;

use std::path::PathBuf;

use citygml3::FeatureReader;
use serde_json::Value;
use support::{Summary, instances, read_seq, templates};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn ours(gml: &str) -> Summary {
    let transform = cjseq::Transform {
        scale: [1e-9, 1e-9, 1e-3],
        translate: [0.0; 3],
    };
    let mut summary = Summary::default();
    for feature in FeatureReader::open(&data(gml)).unwrap().features(transform) {
        summary.add(&serde_json::to_value(feature.unwrap()).unwrap());
    }
    summary
}

/// The oracle's summary, with two of citygml4j's choices undone:
///
/// - it types a `gen:GenericThematicSurface` `+GenericSurface`, which no
///   extension declares, and writes the geometry twice; the reader leaves
///   the surface untyped, once;
/// - it writes a solid and a MultiSurface at one LoD when an object has
///   both (a `lodNMultiSurface` beside a `lodNSolid`, or boundary surfaces
///   that are not faces of the solid); a CityParquet object has one geometry
///   per LoD, and the reader keeps the solid.
fn oracle(jsonl: &str) -> Summary {
    let mut summary = Summary::default();
    for mut feature in read_seq(&data(jsonl)).1 {
        for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
            if let Some(Value::Array(geometries)) = co.get_mut("geometry") {
                let mut seen: Vec<Value> = Vec::new();
                geometries.retain(|g| {
                    let first = !seen.contains(g);
                    seen.push(g.clone());
                    first
                });
                let solid_lods: Vec<Value> = geometries
                    .iter()
                    .filter(|g| g["type"].as_str().unwrap().ends_with("Solid"))
                    .map(|g| g["lod"].clone())
                    .collect();
                geometries.retain(|g| {
                    g["type"].as_str().unwrap().ends_with("Solid")
                        || !solid_lods.contains(&g["lod"])
                });
                for g in geometries.iter_mut() {
                    let generic = g["semantics"]["surfaces"]
                        .as_array()
                        .is_some_and(|s| s.iter().all(|s| s["type"] == "+GenericSurface"));
                    if generic {
                        g.as_object_mut().unwrap().remove("semantics");
                    }
                }
            }
        }
        summary.add(&feature);
    }
    summary
}

fn oracle_of(gml: &str) -> Summary {
    let name = gml.rsplit('/').next().unwrap().trim_end_matches(".gml");
    oracle(&format!("oracle/{name}.city.jsonl"))
}

fn agrees(gml: &str) {
    assert_eq!(ours(gml), oracle_of(gml), "{gml}");
}

#[test]
fn a_building_file_agrees_with_citygml_tools() {
    agrees("plateau/53394519_bldg_6697_op.gml");
}

#[test]
fn a_bridge_file_agrees_with_citygml_tools() {
    agrees("plateau/53394610_brid_6697_op.gml");
}

#[test]
fn a_transportation_file_agrees_with_citygml_tools() {
    agrees("plateau/53394529_tran_6697_op.gml");
}

#[test]
fn a_vegetation_file_agrees_with_citygml_tools() {
    agrees("plateau/53394600_veg_6697_op.gml");
}

/// A building with a room holding a chair, doors and windows filling its
/// walls, an xAL 3 address with a location, and a TIN relief beside it.
#[test]
fn a_lod4_building_agrees_with_citygml_tools() {
    agrees("lod4_building_v2.gml");
}

/// A building whose LoD 2 CompositeSolid is tagged by xlinked boundary
/// surfaces.
#[test]
fn a_composite_solid_building_agrees_with_citygml_tools() {
    agrees("b1_lod2_cs_w_sem.gml");
}

/// A building's rooms, storeys and building units are its children; a
/// unit's or storey's `xlink:href` to a room or unit nests nothing.
///
/// Each storey's MultiSurface xlinks three polygons of a room's
/// `InteriorWallSurface`s. citygml4j tags them with the room's surface; the
/// reader tags a face only with a boundary surface of its own object.
#[test]
fn storeys_and_building_units_agree_with_citygml_tools() {
    let gml = "ogc/BuildingUnits_Storeys_xlink.gml";
    let mut oracle = oracle_of(gml);
    let tagged = oracle
        .surfaces
        .remove(&("BuildingStorey".into(), "InteriorWallSurface".into()));
    assert_eq!(tagged, Some(6));
    assert_eq!(ours(gml), oracle);
}

/// Fifteen trees instantiate three relative geometries through
/// `ImplicitGeometry`, coloured and textured by the CityModel's appearance.
/// The templates, the appearance they index, and every instance agree with
/// citygml-tools.
#[test]
fn implicit_geometries_agree_with_citygml_tools() {
    let gml = data("railway_trees_fragment.gml");
    let transform = cjseq::Transform {
        scale: [1e-3; 3],
        translate: [0.0; 3],
    };
    let document = citygml3::read_document(&gml).unwrap();
    let (geometry_templates, appearance) =
        citygml_core::convert::cjseq::geometry_templates(document.templates())
            .unwrap()
            .expect("the trees' relative geometries");
    let header = serde_json::json!({
        "geometry-templates": geometry_templates,
        "appearance": appearance,
    });
    let our_features: Vec<Value> = FeatureReader::open(&gml)
        .unwrap()
        .features(transform.clone())
        .map(|f| {
            let mut f = serde_json::to_value(f.unwrap()).unwrap();
            f["transform"] = serde_json::to_value(&transform).unwrap();
            f
        })
        .collect();
    let (oracle_header, oracle_features) =
        read_seq(&data("oracle/railway_trees_fragment.city.jsonl"));

    assert_eq!(templates(&header), templates(&oracle_header));
    let instances_ours = instances(&our_features);
    assert_eq!(instances_ours.len(), 15);
    assert_eq!(instances_ours, instances(&oracle_features));
    agrees("railway_trees_fragment.gml");
}
