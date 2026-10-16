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

/// The CityGML version of a test file, which picks its reader.
#[derive(Clone, Copy)]
enum Version {
    V2,
    V3,
}

/// The file's features, and its geometry templates, canonical.
fn from_file(path: &Path, version: Version, lost: &[Lost]) -> (BTreeMap<String, Value>, Value) {
    match version {
        Version::V2 => read(citygml2::FeatureReader::open(path).unwrap(), lost),
        Version::V3 => read(citygml3::FeatureReader::open(path).unwrap(), lost),
    }
}

fn read(mut reader: impl ObjectStream, lost: &[Lost]) -> (BTreeMap<String, Value>, Value) {
    let ids_lost = lost.contains(&Lost::MaterialIds);
    let document = reader.document().clone();
    let mut features = BTreeMap::new();
    for (index, object) in (1..).zip(reader.by_ref()) {
        let context = FeatureContext {
            transform: transform(),
            index,
            ade: document.ade().clone(),
        };
        let mut object = object.unwrap();
        unnamed(&mut object, ids_lost);
        let feature = CityJSONFeature::from_gml(&object, &context).unwrap();
        features.insert(feature.id.clone(), canonical(&json(&feature)));
    }
    let mut file_templates = document.templates().to_vec();
    for t in &mut file_templates {
        for m in &mut t.appearance.materials {
            m.name = None;
            if ids_lost {
                m.id = None;
            }
        }
    }
    in_place(features, templates(&file_templates))
}

/// `features` with each geometry instance's template index replaced by the
/// template it places, and the templates sorted: the two sides number
/// their templates differently, the file in document order and the
/// database by `implicit_geometry` id.
fn in_place(
    mut features: BTreeMap<String, Value>,
    templates: Value,
) -> (BTreeMap<String, Value>, Value) {
    fn place(v: &mut Value, templates: &Value) {
        match v {
            Value::Object(map) => {
                if let Some(i) = map.get("template").and_then(Value::as_u64) {
                    map.insert("template".into(), templates[i as usize].clone());
                }
                map.values_mut().for_each(|v| place(v, templates));
            }
            Value::Array(items) => items.iter_mut().for_each(|v| place(v, templates)),
            _ => {}
        }
    }
    for f in features.values_mut() {
        place(f, &templates);
    }
    let mut sorted = templates.as_array().cloned().unwrap_or_default();
    sorted.sort_by_key(Value::to_string);
    (features, Value::Array(sorted))
}

/// `object` without its materials' `gml:name`s, which 3DCityDB has no
/// column for.
fn unnamed(object: &mut citygml_core::GmlObject, ids_lost: bool) {
    for m in &mut object.appearance.materials {
        m.name = None;
        if ids_lost {
            m.id = None;
        }
    }
    for child in &mut object.children {
        unnamed(child, ids_lost);
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
    let (features, templates) = in_place(features, templates(document.templates()));
    (features, templates, dropped)
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
/// a texture coordinate, which it keeps at single precision,
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
                        ("texture", v) => single_precision(v),
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

/// `v` with every number at single precision: citydb-tool stores texture
/// coordinates so.
fn single_precision(v: Value) -> Value {
    match v {
        Value::Number(n) => n
            .as_f64()
            .map_or(Value::Number(n), |x| Value::from(f64::from(x as f32))),
        Value::Array(items) => Value::Array(items.into_iter().map(single_precision).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, single_precision(v)))
                .collect(),
        ),
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
            if let Some(geometries) = object.get_mut("geometry").and_then(Value::as_array_mut) {
                for (i, g) in geometries.iter_mut().enumerate() {
                    if let Some(semantics) = g.get_mut("semantics") {
                        align_surfaces(semantics, &stated["geometry"][i]["semantics"]);
                    }
                }
            }
            let Some(attributes) = object.get_mut("attributes").and_then(Value::as_object_mut)
            else {
                continue;
            };
            align_stamps(attributes, &stated["attributes"]);
            for (year, date) in [
                ("yearOfConstruction", "dateOfConstruction"),
                ("yearOfDemolition", "dateOfDemolition"),
            ] {
                let first_day = stated["attributes"][year]
                    .as_str()
                    .map(|y| format!("{y}-01-01"));
                if first_day.is_some()
                    && attributes.get(date).and_then(Value::as_str) == first_day.as_deref()
                {
                    attributes.remove(date);
                    attributes.insert(year.into(), stated["attributes"][year].clone());
                }
            }
            if attributes.is_empty() {
                object.remove("attributes");
            }
        }
    }
    canonical::sorted(feature)
}

/// The database's creation date and validity of an object or a surface,
/// `ours`, as the file states them, `theirs`: none where the file states
/// none, citydb-tool stamping a creation date on every feature it imports,
/// and CityGML 2.0's day where the database has the instant that day
/// starts, CityGML 2.0's dates being days and 3.0's instants.
fn align_stamps(ours: &mut serde_json::Map<String, Value>, theirs: &Value) {
    for stamp in ["creationDate", "terminationDate", "validFrom", "validTo"] {
        let Some(stated) = theirs.get(stamp) else {
            ours.remove(stamp);
            continue;
        };
        let day = stated.as_str().map(|d| format!("{d}T00:00:00Z"));
        if day.is_some() && ours.get(stamp).and_then(Value::as_str) == day.as_deref() {
            ours.insert(stamp.into(), stated.clone());
        }
    }
}

/// [`align_stamps`] on every semantic surface of a canonical geometry's
/// `semantics`, paired face by face with the file's.
fn align_surfaces(ours: &mut Value, theirs: &Value) {
    match ours {
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                align_surfaces(item, &theirs[i]);
            }
        }
        Value::Object(surface) => {
            align_stamps(surface, theirs);
            if let Some(parent) = surface.get_mut("parent") {
                align_surfaces(parent, &theirs["parent"]);
            }
        }
        _ => {}
    }
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

/// What of a file citydb-tool did not store.
#[derive(Clone, Copy, PartialEq)]
enum Lost {
    /// Textures: their images are not in this repository, so citydb-tool
    /// stored textures without an image, which the reader drops and
    /// counts.
    Textures,
    /// ADE content: citydb-tool imports an ADE only with its extension
    /// installed, and the test server has none.
    Ade,
    /// Texture image names: citydb-tool names each image it stores afresh
    /// (`tex_10.jpg`).
    ImageNames,
    /// Envelopes: citydb-tool computes each feature's from its geometry,
    /// where the file states some other.
    Envelopes,
    /// Material ids: citydb-tool copies a CityModel-level appearance to
    /// each feature it targets, each copy under an id of its own.
    MaterialIds,
    /// Image bytes: the database holds each texture's image, which the
    /// model has no place for, so the reader keeps the name and counts it.
    ImageData,
}

/// `v` without texture image names, on both sides.
fn without_image_names(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| k != "image")
                .map(|(k, v)| (k, without_image_names(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_image_names).collect()),
        other => other,
    }
}

/// `v` without ADE attributes (`+…`), on every object.
fn without_ade(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| !k.starts_with('+'))
                .map(|(k, v)| (k, without_ade(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_ade).collect()),
        other => other,
    }
}

/// The dataset `database` (loaded by `tests/setup-db.sh` from the
/// citygml3 crate's `file`) reads as the citygml3 reader reads the file.
/// `lost` is what the database does not keep (see [`Lost`]).
fn reads_as_the_file(database: &str, file: &str, lost: &[Lost]) {
    compare(
        database,
        &format!("citygml3/tests/data/{file}"),
        Version::V3,
        lost,
    );
}

/// The dataset `database` (loaded by `tests/setup-db.sh` from the
/// citygml2 crate's CityGML 2.0 `file`) reads as the citygml2 reader reads
/// the file.
fn reads_as_the_2_0_file(database: &str, file: &str, lost: &[Lost]) {
    compare(
        database,
        &format!("citygml2/tests/data/{file}"),
        Version::V2,
        lost,
    );
}

/// Compare the dataset `database` with what `file` (relative to the
/// crates directory) reads as.
fn compare(database: &str, file: &str, version: Version, lost: &[Lost]) {
    let Some(db) = common::database(database) else {
        return;
    };
    let (mut expected, mut expected_templates) = from_file(&common::data(file), version, lost);
    let (actual, actual_templates, dropped) = from_database(&db);
    if lost.contains(&Lost::Textures) {
        assert!(
            dropped.contains_key("app:ParameterizedTexture without an image"),
            "{database}: no texture counted as dropped: {dropped:?}"
        );
        for f in expected.values_mut() {
            *f = without_textures(f.take());
        }
        expected_templates = without_textures(expected_templates);
    }
    if lost.contains(&Lost::ImageData) {
        assert!(
            dropped.get("app:ParameterizedTexture image data") > Some(&0),
            "{database}: no texture image counted as left out: {dropped:?}"
        );
    }
    if lost.contains(&Lost::Ade) {
        for f in expected.values_mut() {
            *f = without_ade(f.take());
        }
    }
    if lost.contains(&Lost::Envelopes) {
        for f in expected.values_mut() {
            for object in f["CityObjects"].as_object_mut().unwrap().values_mut() {
                object.as_object_mut().unwrap().remove("geographicalExtent");
            }
        }
    }
    let (actual, actual_templates) = if lost.contains(&Lost::ImageNames) {
        for f in expected.values_mut() {
            *f = without_image_names(f.take());
        }
        expected_templates = without_image_names(expected_templates);
        (
            actual
                .into_iter()
                .map(|(id, f)| (id, without_image_names(f)))
                .collect::<BTreeMap<_, _>>(),
            without_image_names(actual_templates),
        )
    } else {
        (actual, actual_templates)
    };
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
        let db_feature =
            as_the_database_keeps(as_the_file_states(actual[id].clone(), file_feature));
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
        &[Lost::Textures],
    );
}

#[test]
fn plateau_transportation() {
    reads_as_the_file("plateau3_tran", "plateau/53394529_tran_6697_op.gml", &[]);
}

#[test]
fn plateau_vegetation() {
    reads_as_the_file("plateau3_veg", "plateau/53394600_veg_6697_op.gml", &[]);
}

#[test]
fn plateau_bridges() {
    reads_as_the_file(
        "plateau3_brid",
        "plateau/53394610_brid_6697_op.gml",
        &[Lost::Textures],
    );
}

#[test]
fn a_lod4_building_with_rooms_and_furniture() {
    reads_as_the_file("lod4_building3", "lod4_building_v2.gml", &[]);
}

#[test]
fn groups_and_implicit_geometries() {
    reads_as_the_file(
        "railway_trees3",
        "railway_trees_fragment.gml",
        &[Lost::Textures],
    );
}

#[test]
fn a_composite_solid() {
    reads_as_the_file("b1_lod2", "b1_lod2_cs_w_sem.gml", &[]);
}

#[test]
fn storeys_and_building_units() {
    reads_as_the_file("storeys", "ogc/BuildingUnits_Storeys_xlink.gml", &[]);
}

#[test]
fn a_road_section_with_traffic_spaces() {
    reads_as_the_file(
        "carriageway",
        "ogc/citygml3_individual_section_1carriageway.gml",
        &[],
    );
}

#[test]
fn plateau_2_0_buildings() {
    reads_as_the_2_0_file(
        "plateau_bldg",
        "plateau/udx/bldg/53394519_bldg_6697_op.gml",
        &[Lost::Textures, Lost::Ade],
    );
}

/// What the database holds beyond the model is counted, not dropped
/// silently: the PLATEAU buildings' code spaces and their measured
/// heights' unit of measure, one of each per building.
#[test]
fn code_spaces_and_units_are_counted() {
    let Some(db) = common::database("plateau_bldg") else {
        return;
    };
    let (features, _, dropped) = from_database(&db);
    for what in [
        "bldg:class codeSpace",
        "bldg:usage codeSpace",
        "con:value uom",
    ] {
        assert_eq!(
            dropped.get(what),
            Some(&features.len()),
            "{what}: {dropped:?}"
        );
    }
}

/// A ReliefFeature is passed through to its components, which are what
/// CityJSON has; its own properties are counted.
#[test]
fn a_relief_feature_s_own_properties_are_counted() {
    let Some(db) = common::database("railway_scene") else {
        return;
    };
    let (_, _, dropped) = from_database(&db);
    assert_eq!(
        dropped.get("dem:ReliefFeature dem:lod"),
        Some(&1),
        "{dropped:?}"
    );
}

#[test]
fn plateau_2_0_bridges() {
    reads_as_the_2_0_file(
        "plateau_brid",
        "plateau/udx/brid/53394610_brid_6697_op.gml",
        &[Lost::Textures, Lost::Ade],
    );
}

#[test]
fn plateau_2_0_city_furniture() {
    reads_as_the_2_0_file(
        "plateau_frn",
        "plateau/udx/frn/53394631_frn_6697_op.gml",
        &[Lost::Textures, Lost::Ade],
    );
}

#[test]
fn plateau_2_0_transportation() {
    reads_as_the_2_0_file(
        "plateau_tran",
        "plateau/udx/tran/53394529_tran_6697_op.gml",
        &[Lost::Ade],
    );
}

#[test]
fn plateau_2_0_vegetation() {
    reads_as_the_2_0_file(
        "plateau_veg",
        "plateau/udx/veg/53394600_veg_6697_op.gml",
        &[Lost::Ade],
    );
}

#[test]
fn a_2_0_lod4_building() {
    reads_as_the_2_0_file("lod4_building", "lod4_building_v2.gml", &[]);
}

#[test]
fn groups_and_implicit_geometries_in_2_0() {
    reads_as_the_2_0_file(
        "railway_trees",
        "railway_trees_fragment.gml",
        &[Lost::Textures],
    );
}

/// The dataset `database` (loaded by `tests/setup-db.sh` from the citygml2
/// crate's CityJSONSeq `file`, coordinates in metres) reads as the file
/// states, but for what citydb-tool's CityJSON importer changes: it keeps
/// one geometry per LoD tier, the last of `1.2` and `1.3`, as LoD `1`; it
/// stores a boolean attribute as the text `true` or `false`, and a null
/// attribute as the text `null`, CityGML's generic attributes having
/// neither.
fn reads_as_the_cityjson_file(database: &str, file: &str) {
    let Some(db) = common::database(database) else {
        return;
    };
    let text =
        std::fs::read_to_string(common::data(&format!("citygml2/tests/data/{file}"))).unwrap();
    let mut lines = text.lines();
    let header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let ours = Transform {
        scale: [0.001; 3],
        translate: [0.0; 3],
    };
    let expected: BTreeMap<String, Value> = lines
        .map(|line| {
            let mut feature: Value = serde_json::from_str(line).unwrap();
            requantise(&mut feature, &header["transform"], &ours);
            for object in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
                as_citydb_tool_imports(object);
            }
            (
                feature["id"].as_str().unwrap().to_string(),
                canonical(&feature),
            )
        })
        .collect();
    let mut actual = BTreeMap::new();
    for feature in db.features(ours).unwrap() {
        let feature = feature.unwrap();
        actual.insert(feature.id.clone(), canonical(&json(&feature)));
    }
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        actual.keys().collect::<Vec<_>>(),
        "{database}: feature ids"
    );
    let mut differences = Vec::new();
    for (id, file_feature) in &expected {
        let db_feature = as_the_file_states(actual[id].clone(), file_feature);
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
}

/// `feature`'s vertices, quantised by `from`, quantised by `to` instead.
fn requantise(feature: &mut Value, from: &Value, to: &Transform) {
    let axis = |key: &str, i: usize| from[key][i].as_f64().unwrap();
    for v in feature["vertices"].as_array_mut().unwrap() {
        for (i, c) in v.as_array_mut().unwrap().iter_mut().enumerate() {
            let real = c.as_i64().unwrap() as f64 * axis("scale", i) + axis("translate", i);
            *c = Value::from(((real - to.translate[i]) / to.scale[i]).round() as i64);
        }
    }
}

/// A CityJSON object as citydb-tool's importer stores it.
fn as_citydb_tool_imports(object: &mut Value) {
    if let Some(geometries) = object.get_mut("geometry").and_then(Value::as_array_mut) {
        let mut kept: Vec<Value> = Vec::new();
        for mut g in geometries.drain(..) {
            let tier = g["lod"]
                .as_str()
                .unwrap()
                .split('.')
                .next()
                .unwrap()
                .to_string();
            g["lod"] = Value::from(tier.clone());
            kept.retain(|k| k["lod"] != g["lod"]);
            kept.push(g);
        }
        *geometries = kept;
    }
    if let Some(attributes) = object.get_mut("attributes").and_then(Value::as_object_mut) {
        attributes.values_mut().for_each(as_generic_attribute);
    }
    for g in object["geometry"].as_array_mut().into_iter().flatten() {
        let surfaces = g
            .pointer_mut("/semantics/surfaces")
            .and_then(Value::as_array_mut);
        for surface in surfaces.into_iter().flatten() {
            for (k, v) in surface.as_object_mut().unwrap() {
                if !matches!(k.as_str(), "type" | "parent" | "children") {
                    as_generic_attribute(v);
                }
            }
        }
    }
}

fn as_generic_attribute(v: &mut Value) {
    match v {
        Value::Bool(b) => *v = Value::from(b.to_string()),
        Value::Null => *v = Value::from("null"),
        _ => {}
    }
}

#[test]
fn delft_from_3dbag() {
    reads_as_the_cityjson_file("delft", "3dbag/delft_100.city.jsonl");
}

#[test]
fn plateau_2_0_water_bodies() {
    reads_as_the_2_0_file(
        "plateau_wtr",
        "plateau/udx/wtr/533945_wtr_6697_op.gml",
        &[Lost::Ade],
    );
}

/// The KIT LoD 3 railway scene citydb-tool ships, a CityGML 2.0 document
/// with textures, which `tests/setup-db.sh` imports and also unpacks to
/// `target/citydb-test` for this test to read.
#[test]
fn the_railway_scene() {
    let file = "../target/citydb-test/railway_scene/railway_scene_lod3_v2.gml";
    if common::database("railway_scene").is_some() {
        assert!(
            common::data(file).exists(),
            "{file} missing: run tests/setup-db.sh"
        );
    }
    compare(
        "railway_scene",
        file,
        Version::V2,
        &[
            Lost::ImageNames,
            Lost::ImageData,
            Lost::MaterialIds,
            Lost::Envelopes,
        ],
    );
}
