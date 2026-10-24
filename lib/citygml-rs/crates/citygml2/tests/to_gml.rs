//! CityJSON back to the CityGML model: each real file's features, turned
//! into CityGML 2.0 objects and converted to CityJSON again, say what they
//! said before, index for index once palettes are resolved.

#![cfg(feature = "cjseq")]

#[path = "../../../test-support/canonical.rs"]
mod canonical;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use canonical::{canonical, canonical_templates, diff};
use citygml_core::convert::cjseq::{
    Extensions, FeatureContext, GmlContext, extensions, geometry_templates, template_lods,
    templates_to_gml,
};
use citygml_core::convert::{FromGml, ToGml};
use citygml_core::reader::ObjectStream;
use citygml2::FeatureReader;
use citygml2::writer::Vocabulary2;
use cjseq::{CityJSONFeature, Transform};
use serde_json::Value;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// Every feature of `gml`, and the same after a trip through the model.
fn round_trip(gml: &str) -> (Vec<Value>, Vec<Value>) {
    let transform = Transform {
        scale: [1e-9, 1e-9, 1e-3],
        translate: [0.0; 3],
    };
    let mut reader = FeatureReader::open(&data(gml)).unwrap();
    let document = reader.document().clone();
    let declared: BTreeMap<String, cjseq::Extension> =
        extensions(document.ade()).into_iter().collect();
    let templates = geometry_templates(document.templates()).unwrap();
    let lods = templates
        .as_ref()
        .map(|(t, _)| template_lods(t))
        .unwrap_or_else(|| Arc::from(Vec::new()));
    let cx = GmlContext {
        transform: transform.clone(),
        index: 0,
        vocabulary: Arc::new(Vocabulary2),
        extensions: Arc::new(Extensions::new(&declared, &BTreeMap::new())),
        template_lods: lods,
    };
    let (mut before, mut after) = (Vec::new(), Vec::new());
    for (index, object) in (1..).zip(reader.by_ref()) {
        let from = FeatureContext {
            transform: transform.clone(),
            index,
            ade: document.ade().clone(),
        };
        let feature = CityJSONFeature::from_gml(&object.unwrap(), &from).unwrap();
        let model = feature
            .to_gml(&GmlContext {
                index,
                ..cx.clone()
            })
            .unwrap();
        assert!(model.losses.is_empty(), "{gml}: {:?}", model.losses);
        before.push(canonical(&without_implied(json(&feature))));
        for object in &model.objects {
            let again = CityJSONFeature::from_gml(object, &from).unwrap();
            after.push(canonical(&serde_json::to_value(&again).unwrap()));
        }
    }
    if let Some((templates, appearance)) = templates {
        let (model, losses) = templates_to_gml(&templates, appearance.as_ref()).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let (again, again_appearance) = geometry_templates(&model).unwrap().unwrap();
        assert_eq!(
            canonical_templates(&json(&again), &json(&again_appearance)),
            canonical_templates(&json(&templates), &json(&appearance)),
        );
    }
    (before, after)
}

/// `feature` without what the CityGML 2.0 class of an object says, which
/// the model leaves to the reader to restate: an installation's
/// `relationToConstruction`.
fn without_implied(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        if co["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("Installation"))
            && let Some(attributes) = co.get_mut("attributes").and_then(Value::as_object_mut)
        {
            attributes.remove("relationToConstruction");
            if attributes.is_empty() {
                co.as_object_mut().unwrap().remove("attributes");
            }
        }
    }
    feature
}

fn json(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap()
}

fn same(gml: &str) {
    let (before, after) = round_trip(gml);
    assert_eq!(before.len(), after.len(), "{gml}: feature count");
    let mut differences = Vec::new();
    for (b, a) in before.iter().zip(&after) {
        for d in diff(b, a, 5) {
            differences.push(format!("{}: {d}", b["id"]));
        }
    }
    assert!(
        differences.is_empty(),
        "{gml}: {} differences, first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
}

#[test]
fn plateau_buildings_survive_the_model() {
    same("plateau/udx/bldg/53394519_bldg_6697_op.gml");
}

#[test]
fn plateau_bridges_survive_the_model() {
    same("plateau/udx/brid/53394610_brid_6697_op.gml");
}

#[test]
fn plateau_city_furniture_survives_the_model() {
    same("plateau/udx/frn/53394631_frn_6697_op.gml");
}

#[test]
fn plateau_transportation_survives_the_model() {
    same("plateau/udx/tran/53394529_tran_6697_op.gml");
}

#[test]
fn plateau_vegetation_survives_the_model() {
    same("plateau/udx/veg/53394600_veg_6697_op.gml");
}

#[test]
fn plateau_water_bodies_survive_the_model() {
    same("plateau/udx/wtr/533945_wtr_6697_op.gml");
}

#[test]
fn plateau_ade_objects_survive_the_model() {
    same("plateau/udx/urf/533946_urf_6697_08_op.gml");
    same("plateau/udx/ubld/53394611_ubld_6697_op.gml");
}

#[test]
fn a_lod4_building_and_relief_survive_the_model() {
    same("lod4_building_v2.gml");
}

#[test]
fn groups_and_implicit_geometries_survive_the_model() {
    same("railway_trees_fragment.gml");
}

#[test]
fn a_composite_solid_survives_the_model() {
    same("b1_lod2_cs_w_sem.gml");
    same("building_citymodel_appearance.gml");
}

#[path = "../../../test-support/oracle.rs"]
mod oracle;

/// `feature` with each texture's type taken from its image's extension
/// where citygml-tools, given no MIME type, wrote `unknown`, which
/// CityJSON does not allow.
fn with_image_types(mut feature: Value) -> Value {
    for t in feature["appearance"]["textures"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        if t["type"] == "unknown" {
            let image = t["image"].as_str().unwrap().to_ascii_lowercase();
            t["type"] = if image.ends_with(".png") {
                "PNG"
            } else {
                "JPG"
            }
            .into();
        }
    }
    feature
}

/// Every feature of the CityJSONSeq at `path`, which another tool wrote,
/// and the same after a trip through the model and made canonical, with
/// what was lost.
fn foreign(path: &std::path::Path) -> (Vec<Value>, Vec<Value>, citygml_core::convert::Losses) {
    let (header, features) = oracle::read_seq(path);
    let declared: BTreeMap<String, cjseq::Extension> = header
        .get("extensions")
        .map(|e| serde_json::from_value(e.clone()).unwrap())
        .unwrap_or_default();
    let transform: Transform = serde_json::from_value(header["transform"].clone()).unwrap();
    let cx = GmlContext {
        transform: transform.clone(),
        index: 0,
        vocabulary: Arc::new(Vocabulary2),
        extensions: Arc::new(Extensions::new(&declared, &BTreeMap::new())),
        template_lods: Arc::from(Vec::new()),
    };
    let (mut before, mut after) = (Vec::new(), Vec::new());
    let mut losses = citygml_core::convert::Losses::default();
    for (index, f) in features.into_iter().enumerate() {
        let feature: CityJSONFeature = serde_json::from_value(with_image_types(f)).unwrap();
        let model = feature
            .to_gml(&GmlContext {
                index,
                ..cx.clone()
            })
            .unwrap();
        losses.merge(model.losses);
        before.push(json(&feature));
        let from = FeatureContext {
            transform: transform.clone(),
            index,
            ade: Default::default(),
        };
        for object in &model.objects {
            let again = CityJSONFeature::from_gml(object, &from).unwrap();
            after.push(canonical(&json(&again)));
        }
    }
    (before, after, losses)
}

/// `path`'s features say after the trip what they said before, with
/// nothing lost but what `lost` names.
fn same_foreign(path: &str, lost: &[&str]) {
    let (before, after, losses) = foreign(&data(path));
    let unexpected: Vec<_> = losses
        .iter()
        .filter(|(what, _)| !lost.contains(what))
        .collect();
    assert!(unexpected.is_empty(), "{path}: {unexpected:?}");
    assert_eq!(before.len(), after.len(), "{path}: feature count");
    let mut differences = Vec::new();
    for (b, a) in before.iter().zip(&after) {
        let b = with_plain_dates(without_nulls(without_implied(b.clone())));
        for d in diff(&canonical(&b), a, 5) {
            differences.push(format!("{}: {d}", b["id"]));
        }
    }
    assert!(
        differences.is_empty(),
        "{path}: {} differences, first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
}

/// `feature` without the attributes holding null, which CityGML cannot
/// state.
fn without_nulls(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        if let Some(attributes) = co.get_mut("attributes").and_then(Value::as_object_mut) {
            attributes.retain(|_, v| !v.is_null());
        }
    }
    feature
}

/// `feature` with its creation and termination dates given as midnight
/// `xs:dateTime`s (as citygml-tools writes them) cut to the `xs:date` the
/// CityGML property holds.
fn with_plain_dates(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        let Some(attributes) = co.get_mut("attributes").and_then(Value::as_object_mut) else {
            continue;
        };
        for key in ["creationDate", "terminationDate"] {
            if let Some(Value::String(s)) = attributes.get_mut(key) {
                *s = s.replacen("T00:00:00", "", 1);
            }
        }
    }
    feature
}

#[test]
fn citygml_tools_cityjson_survives_the_model() {
    for file in [
        "53394519_bldg_6697_op",
        "53394529_tran_6697_op",
        "533945_wtr_6697_op",
        "53394600_veg_6697_op",
        "53394610_brid_6697_op",
        "53394631_frn_6697_op",
        "plateau_trk_fragment",
    ] {
        same_foreign(
            &format!("oracle/{file}.city.jsonl"),
            &["dates given with their midnight time"],
        );
    }
}

#[test]
fn three_d_bag_survives_the_model_but_its_nulls() {
    same_foreign(
        "3dbag/delft_100.city.jsonl",
        &["attributes whose value is null"],
    );
}

#[test]
fn an_id_that_is_not_an_xml_name_is_renamed_and_counted() {
    let (before, after, losses) = foreign(&data("rotterdam_braced_ids.city.jsonl"));
    let renamed = losses
        .iter()
        .find(|(what, _)| what.starts_with("ids that are not XML names"))
        .map(|(_, n)| n);
    assert_eq!(renamed, Some(before.len()), "{losses:?}");
    for (b, a) in before.iter().zip(&after) {
        let id = b["id"].as_str().unwrap();
        assert_eq!(a["id"], id.replace(['{', '}'], "_").replacen("", "_", 1));
    }
}
