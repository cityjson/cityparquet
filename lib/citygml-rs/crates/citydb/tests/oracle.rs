//! The database reader against the CityGML 3.0 reader: a CityGML 3.0 file
//! imported into 3DCityDB by citydb-tool, read back from the database,
//! says what the citygml3 reader reads from the file itself, as CityJSON,
//! apart from what the database cannot keep.

#![cfg(feature = "cjseq")]

mod common;

#[path = "../../../test-support/canonical.rs"]
mod canonical;

use std::collections::BTreeMap;
use std::path::Path;

use canonical::{canonical, canonical_templates, diff};
use citygml_core::convert::FromGml;
use citygml_core::convert::cjseq::{FeatureContext, geometry_templates};
use citygml_core::reader::ObjectStream;
use cjseq::{CityJSONFeature, Transform};
use serde_json::Value;

/// Coordinates quantised finely enough for degrees, so that both sides
/// keep every digit the file states.
fn transform() -> Transform {
    Transform {
        scale: [1e-9, 1e-9, 1e-6],
        translate: [0.0; 3],
    }
}

fn json(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap()
}

/// The file's features, and its geometry templates, canonical.
fn from_file(path: &Path) -> (BTreeMap<String, Value>, Value) {
    let mut reader = citygml3::FeatureReader::open(path).unwrap();
    let document = reader.document().clone();
    let mut features = BTreeMap::new();
    for (index, object) in (1..).zip(reader.by_ref()) {
        let context = FeatureContext {
            transform: transform(),
            index,
            ade: document.ade().clone(),
        };
        let mut object = object.unwrap();
        unnamed(&mut object);
        let feature = CityJSONFeature::from_gml(&object, &context).unwrap();
        features.insert(feature.id.clone(), canonical(&json(&feature)));
    }
    let mut file_templates = document.templates().to_vec();
    for t in &mut file_templates {
        for m in &mut t.appearance.materials {
            m.name = None;
        }
    }
    (features, templates(&file_templates))
}

/// `object` without its materials' `gml:name`s, which 3DCityDB has no
/// column for.
fn unnamed(object: &mut citygml_core::GmlObject) {
    for m in &mut object.appearance.materials {
        m.name = None;
    }
    for child in &mut object.children {
        unnamed(child);
    }
}

/// The database's features, its geometry templates, canonical, and what
/// the reader dropped.
fn from_database(
    db: &citygml_citydb::CityDb,
) -> (BTreeMap<String, Value>, Value, BTreeMap<String, usize>) {
    let mut features = BTreeMap::new();
    let mut stream = db.features(transform()).unwrap();
    let document = stream.reader().document().clone();
    for feature in stream.by_ref() {
        let feature = feature.unwrap();
        features.insert(feature.id.clone(), canonical(&json(&feature)));
    }
    let dropped = stream.reader().dropped().clone();
    (features, templates(document.templates()), dropped)
}

fn templates(templates: &[citygml_core::model::Template]) -> Value {
    match geometry_templates(templates).unwrap() {
        Some((templates, appearance)) => canonical_templates(
            &json(&templates),
            &appearance.as_ref().map_or(Value::Null, json),
        ),
        None => Value::Null,
    }
}

/// `v` with what the database stores less precisely than a file said
/// reduced on both sides: a colour, which it keeps as 8 bits a component,
/// and the name the conversion makes up for a material that has neither a
/// name nor an id (`material-2`), which is its place in the feature's
/// palette, and the database keeps no document order of an object's
/// materials.
fn as_the_database_keeps(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v) {
                        (
                            "diffuseColor" | "emissiveColor" | "specularColor" | "borderColor",
                            Value::Array(c),
                        ) => Value::Array(
                            c.into_iter()
                                .map(|c| {
                                    c.as_f64()
                                        .map_or(c, |c| Value::from((c * 255.0).round() / 255.0))
                                })
                                .collect(),
                        ),
                        ("name", Value::String(name))
                            if name.strip_prefix("material-").is_some_and(|n| {
                                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
                            }) =>
                        {
                            Value::from("material")
                        }
                        (_, v) => as_the_database_keeps(v),
                    };
                    (k, v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(as_the_database_keeps).collect()),
        other => other,
    }
}

/// `v` without textures: what a database holds when citydb-tool imported
/// a file whose texture images are not beside it, as this repository
/// carries none of PLATEAU's.
fn without_textures(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| k != "texture")
                .map(|(k, v)| (k, without_textures(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_textures).collect()),
        other => other,
    }
}

/// `feature` from the database with what the database adds or makes up
/// taken out, wherever `file` (the same feature read from the file) has
/// nothing in its place: the creation date citydb-tool stamps on every
/// feature it imports, the envelope it computes for every feature, and the
/// id it generates for an object the file gives none, which the file
/// reader names after its parent instead.
fn as_the_file_states(feature: Value, file: &Value) -> Value {
    let mut renamed = BTreeMap::new();
    if let Some(id) = file["id"].as_str() {
        rename(&feature, file, id, id, &mut renamed);
    }
    let mut text = feature.to_string();
    for (generated, named) in &renamed {
        text = text.replace(&format!("\"{generated}\""), &format!("\"{named}\""));
    }
    let mut feature: Value = serde_json::from_str(&text).unwrap();
    if let Some(objects) = feature["CityObjects"].as_object_mut() {
        for (id, object) in objects.iter_mut() {
            let stated = &file["CityObjects"][id];
            let object = object.as_object_mut().unwrap();
            if stated.get("geographicalExtent").is_none() {
                object.remove("geographicalExtent");
            }
            let Some(attributes) = object.get_mut("attributes").and_then(Value::as_object_mut)
            else {
                continue;
            };
            for stamp in ["creationDate", "validFrom", "validTo"] {
                if stated["attributes"].get(stamp).is_none() {
                    attributes.remove(stamp);
                }
            }
            if attributes.is_empty() {
                object.remove("attributes");
            }
        }
    }
    canonical::sorted(feature)
}

/// Pair the children of object `ours` in the database's `feature` with
/// those of object `theirs` in the file's, position by position, and
/// record each generated id of ours that the file names otherwise.
fn rename(
    feature: &Value,
    file: &Value,
    ours: &str,
    theirs: &str,
    renamed: &mut BTreeMap<String, String>,
) {
    let children = |f: &Value, id: &str| -> Vec<String> {
        f["CityObjects"][id]["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_string))
            .collect()
    };
    for (o, t) in children(feature, ours).iter().zip(children(file, theirs)) {
        if *o != t && o.starts_with("ID_") {
            renamed.insert(o.clone(), t.clone());
        }
        rename(feature, file, o, &t, renamed);
    }
}

/// Whether the file's textures reached the database.
#[derive(Clone, Copy, PartialEq)]
enum Textures {
    Kept,
    /// Their images are not in this repository, so citydb-tool stored
    /// textures without an image, which the reader drops and counts.
    Lost,
}

/// The dataset `database` (loaded by `tests/setup-db.sh` from the
/// citygml3 crate's `file`) reads as the file does.
///
/// `xlinked` names the objects whose plain surface geometry has
/// `gml:surfaceMember xlink:href` members: the citygml3 reader leaves them
/// out, and citydb-tool stores them resolved. Each such geometry from the
/// database must hold every face the file reader reads and more, and is
/// then compared no further.
fn reads_as_the_file(database: &str, file: &str, textures: Textures, xlinked: &[&str]) {
    let Some(db) = common::database(database) else {
        return;
    };
    let (mut expected, mut expected_templates) =
        from_file(&common::data(&format!("citygml3/tests/data/{file}")));
    let (actual, actual_templates, dropped) = from_database(&db);
    if textures == Textures::Lost {
        assert!(
            dropped.contains_key("app:ParameterizedTexture without an image"),
            "{database}: no texture counted as dropped: {dropped:?}"
        );
        for f in expected.values_mut() {
            *f = without_textures(f.take());
        }
        expected_templates = without_textures(expected_templates);
    }
    let expected: BTreeMap<String, Value> = expected
        .into_iter()
        .map(|(id, f)| (id, as_the_database_keeps(f)))
        .collect();
    let expected_templates = as_the_database_keeps(expected_templates);
    let actual_templates = as_the_database_keeps(actual_templates);
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        actual.keys().collect::<Vec<_>>(),
        "{database}: feature ids"
    );
    let mut differences = Vec::new();
    for (id, file_feature) in &expected {
        let mut db_feature =
            as_the_database_keeps(as_the_file_states(actual[id].clone(), file_feature));
        for object in xlinked {
            let (Some(ours), Some(theirs)) = (
                db_feature["CityObjects"].get_mut(*object),
                file_feature["CityObjects"].get(*object),
            ) else {
                continue;
            };
            let faces = |o: &Value| -> Vec<Value> {
                o["geometry"][0]["boundaries"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            };
            let (resolved, read) = (faces(ours), faces(theirs));
            assert!(
                resolved.len() > read.len() && read.iter().all(|f| resolved.contains(f)),
                "{database}: {object}: the database holds {} faces, the file reader {}",
                resolved.len(),
                read.len()
            );
            ours["geometry"] = theirs["geometry"].clone();
        }
        for d in diff(file_feature, &db_feature, 5) {
            differences.push(format!("{id}: {d}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{database}: {} differences, first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
    let differences = diff(&expected_templates, &actual_templates, 10);
    assert!(
        differences.is_empty(),
        "{database}: templates differ: {differences:#?}"
    );
}

#[test]
fn plateau_buildings() {
    reads_as_the_file(
        "plateau3_bldg",
        "plateau/53394519_bldg_6697_op.gml",
        Textures::Lost,
        &[],
    );
}

#[test]
fn plateau_transportation() {
    reads_as_the_file(
        "plateau3_tran",
        "plateau/53394529_tran_6697_op.gml",
        Textures::Kept,
        &[],
    );
}

#[test]
fn plateau_vegetation() {
    reads_as_the_file(
        "plateau3_veg",
        "plateau/53394600_veg_6697_op.gml",
        Textures::Kept,
        &[],
    );
}

#[test]
fn plateau_bridges() {
    reads_as_the_file(
        "plateau3_brid",
        "plateau/53394610_brid_6697_op.gml",
        Textures::Lost,
        &[],
    );
}

#[test]
fn a_lod4_building_with_rooms_and_furniture() {
    reads_as_the_file(
        "lod4_building3",
        "lod4_building_v2.gml",
        Textures::Kept,
        &[],
    );
}

#[test]
fn groups_and_implicit_geometries() {
    reads_as_the_file(
        "railway_trees3",
        "railway_trees_fragment.gml",
        Textures::Lost,
        &[],
    );
}

#[test]
fn a_composite_solid() {
    reads_as_the_file("b1_lod2", "b1_lod2_cs_w_sem.gml", Textures::Kept, &[]);
}

#[test]
fn storeys_and_building_units() {
    reads_as_the_file(
        "storeys",
        "ogc/BuildingUnits_Storeys_xlink.gml",
        Textures::Kept,
        &["storey1", "storey2"],
    );
}

#[test]
fn a_road_section_with_traffic_spaces() {
    reads_as_the_file(
        "carriageway",
        "ogc/citygml3_individual_section_1carriageway.gml",
        Textures::Kept,
        &[],
    );
}
