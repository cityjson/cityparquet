//! The CityGML 2.0 reader on real documents: the model it fills, and the
//! CityJSON features it converts that model to.

use std::collections::BTreeSet;
use std::path::PathBuf;

use citygml_core::model::{Module, QName, Shape};
use citygml2::FeatureReader;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// libcitygml's `b1_lod2_cs_w_sem`: one id-less building whose LoD2
/// CompositeSolid of two solids is tagged by `boundedBy` surfaces through
/// xlinks.
#[test]
fn a_composite_solid_building_keeps_its_solids_and_boundary_surfaces() {
    let mut reader = FeatureReader::open(&data("b1_lod2_cs_w_sem.gml")).unwrap();
    let object = reader.next().unwrap().unwrap();
    assert!(reader.next().is_none());
    assert!(reader.skipped_members().is_empty());
    assert_eq!(reader.emitted_members(), 1);

    assert_eq!(object.class, QName::module(Module::Building, "Building"));
    assert_eq!(object.id, None);
    assert_eq!(object.geometries.len(), 1);
    let geometry = &object.geometries[0];
    assert_eq!(geometry.lod, "2");
    let Shape::CompositeSolid(solids) = &geometry.shape else {
        panic!("expected a CompositeSolid, got {:?}", geometry.shape);
    };
    assert_eq!(solids.len(), 2);
    let classes: BTreeSet<&str> = object
        .boundary_surfaces
        .iter()
        .map(|s| s.class.local.as_str())
        .collect();
    assert_eq!(
        classes,
        BTreeSet::from(["GroundSurface", "RoofSurface", "WallSurface"])
    );
    // Nine of the eleven faces are tagged, each by its own surface; the two
    // faces where the solids meet are not.
    assert_eq!(object.boundary_surfaces.len(), 9);
    assert_eq!(geometry.shape.faces().count(), 11);
    let tagged: Vec<usize> = geometry.shape.faces().filter_map(|f| f.surface).collect();
    assert_eq!(tagged, (0..9).collect::<Vec<_>>());
}

/// A material and a texture declared at the CityModel level reach the
/// building whose faces and rings they target, and only when the reader is
/// opened with that appearance.
#[cfg(feature = "cjseq")]
#[test]
fn citymodel_appearance_reaches_the_feature_it_targets() {
    let path = data("building_citymodel_appearance.gml");
    let transform = cjseq::Transform {
        scale: [0.001; 3],
        translate: [0.0; 3],
    };
    let with: Vec<_> = FeatureReader::open(&path)
        .unwrap()
        .features(transform.clone())
        .collect::<Result<_, _>>()
        .unwrap();
    let without: Vec<_> = FeatureReader::open_without_appearance(&path)
        .unwrap()
        .features(transform)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!((with.len(), without.len()), (1, 1));
    let (with, without) = (&with[0], &without[0]);

    assert_eq!(with.id, "BM");
    assert_eq!(with.vertices, without.vertices);
    assert_eq!(with.vertices.len(), 4);
    assert!(without.appearance.is_none());
    let appearance = with.appearance.as_ref().expect("appearance");
    assert_eq!(appearance.materials.as_ref().map(Vec::len), Some(1));
    assert_eq!(appearance.textures.as_ref().map(Vec::len), Some(1));
    assert_eq!(appearance.vertices_texture.as_ref().map(Vec::len), Some(3));
}
