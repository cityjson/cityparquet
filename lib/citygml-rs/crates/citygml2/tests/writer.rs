//! CityGML 2.0 written from CityJSON: each real file's features, written
//! as a CityGML 2.0 document and read back, say what they said before.

#![cfg(feature = "cjseq")]

#[path = "../../../test-support/canonical.rs"]
mod canonical;
#[path = "../../../test-support/oracle.rs"]
mod oracle;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use canonical::{canonical, canonical_templates, diff};
use citygml_core::convert::cjseq::{
    Extensions, FeatureContext, GmlContext, extensions, geometry_templates,
};
use citygml_core::convert::{FromGml, Losses, ToGml};
use citygml_core::reader::ObjectStream;
use citygml2::FeatureReader;
use citygml2::writer::{Report, Vocabulary2, Writer, WriterOptions};
use cjseq::{CityJSON, CityJSONFeature, Transform};
use serde_json::Value;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// Where a test writes `name`.
fn scratch(name: &str) -> PathBuf {
    let file = name.replace('/', "_");
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("citygml2_writer_{file}.gml"))
}

fn json(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap()
}

/// The CityGML at `path` as CityJSON, quantised against `transform`: a
/// header declaring its ADEs and geometry templates, and its features.
fn read(path: &Path, transform: &Transform) -> (CityJSON, Vec<CityJSONFeature>) {
    let mut reader = FeatureReader::open(path).unwrap();
    let document = reader.document().clone();
    let mut header = CityJSON::new();
    header.transform = transform.clone();
    let declared = extensions(document.ade());
    if !declared.is_empty() {
        header.extensions = Some(declared.into_iter().collect());
    }
    if let Some((templates, appearance)) = geometry_templates(document.templates()).unwrap() {
        header.geometry_templates = Some(templates);
        header.appearance = appearance;
    }
    let mut features = Vec::new();
    for (index, object) in (1..).zip(reader.by_ref()) {
        let context = FeatureContext {
            transform: transform.clone(),
            index,
            ade: document.ade().clone(),
        };
        features.push(CityJSONFeature::from_gml(&object.unwrap(), &context).unwrap());
    }
    (header, features)
}

/// `header` and `features` written as CityGML 2.0 at `out`, with the ADE
/// schemas `schemas` gives by namespace.
fn write(
    header: &CityJSON,
    features: &[CityJSONFeature],
    out: &Path,
    schemas: BTreeMap<String, String>,
) -> Report {
    let file = BufWriter::new(File::create(out).unwrap());
    let options = WriterOptions {
        ade_schemas: schemas,
        ..WriterOptions::default()
    };
    let mut writer = Writer::new(file, header, options).unwrap();
    for f in features {
        writer.write_feature(f).unwrap();
    }
    let (file, report) = writer.finish().unwrap();
    drop(file.into_inner().unwrap());
    validate(out);
    report
}

/// `path` is valid against the CityGML 2.0 schemas and its ADEs', as
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

/// The schemas of the ADEs the CityGML at `path` uses, by namespace, as
/// files: what its relative `xsi:schemaLocation`s name.
fn ade_schemas(path: &Path) -> BTreeMap<String, String> {
    let document = citygml2::read_document(path).unwrap();
    document
        .ade()
        .ades()
        .iter()
        .filter_map(|a| {
            let location = a.schema_location.as_deref()?;
            let file = path.parent()?.join(location);
            file.is_file()
                .then(|| (a.namespace.clone(), file.display().to_string()))
        })
        .collect()
}

/// How many of `what` `losses` counts.
fn count(losses: &Losses, what: &str) -> usize {
    losses
        .iter()
        .find(|(w, _)| *w == what)
        .map_or(0, |(_, n)| n)
}

/// The losses of `report` other than those `lost` names.
fn unexpected<'a>(losses: &'a Losses, lost: &[&str]) -> Vec<(&'a str, usize)> {
    losses
        .iter()
        .filter(|(what, _)| !lost.contains(what))
        .collect()
}

/// `before` and `after`, made canonical, differ nowhere.
fn same_features(name: &str, before: &[Value], after: &[Value]) {
    assert_eq!(before.len(), after.len(), "{name}: feature count");
    let mut differences = Vec::new();
    for (b, a) in before.iter().zip(after) {
        for d in diff(b, a, 5) {
            differences.push(format!("{}: {d}", b["id"]));
        }
    }
    assert!(
        differences.is_empty(),
        "{name}: {} differences, first: {:#?}",
        differences.len(),
        &differences[..differences.len().min(20)]
    );
}

/// `before` and `after` hold each feature's materials and textures in the
/// same order: a document read back numbers them as the first read did.
fn same_palettes(name: &str, before: &[CityJSONFeature], after: &[CityJSONFeature]) {
    let palettes = |fs: &[CityJSONFeature]| -> Vec<(Value, Value)> {
        fs.iter()
            .map(|f| {
                let f = json(f);
                (
                    f["appearance"]["materials"].clone(),
                    f["appearance"]["textures"].clone(),
                )
            })
            .collect()
    };
    let (before, after) = (palettes(before), palettes(after));
    let differing = before.iter().zip(&after).filter(|(b, a)| b != a).count();
    assert_eq!(
        differing, 0,
        "{name}: features whose palettes are reordered"
    );
}

/// The CityGML at `gml`, read, written and read again, says what it said
/// the first time, losing nothing but what `lost` names.
fn survives(gml: &str, lost: &[&str]) {
    survives_with(gml, lost, ade_schemas(&data(gml)));
}

/// [`survives`], the writer given the ADE schemas `schemas` names.
fn survives_with(gml: &str, lost: &[&str], schemas: BTreeMap<String, String>) {
    let transform = Transform {
        scale: [1e-9, 1e-9, 1e-3],
        translate: [0.0; 3],
    };
    let (header, before) = read(&data(gml), &transform);
    let out = scratch(gml);
    let report = write(&header, &before, &out, schemas);
    assert!(
        unexpected(&report.losses, lost).is_empty(),
        "{gml}: {:?}",
        report.losses
    );
    assert_eq!(report.features, before.len());
    let (header_again, after) = read(&out, &transform);
    let canonical_all =
        |fs: &[CityJSONFeature]| -> Vec<Value> { fs.iter().map(|f| canonical(&json(f))).collect() };
    same_features(gml, &canonical_all(&before), &canonical_all(&after));
    same_palettes(gml, &before, &after);
    let templates = |h: &CityJSON| {
        canonical_templates(
            &json(&h.geometry_templates),
            &h.appearance.as_ref().map_or(Value::Null, json),
        )
    };
    assert_eq!(
        templates(&header),
        templates(&header_again),
        "{gml}: templates"
    );
}

#[test]
fn plateau_buildings_survive_the_writer() {
    survives("plateau/udx/bldg/53394519_bldg_6697_op.gml", &[]);
}

#[test]
fn plateau_bridges_survive_the_writer() {
    survives("plateau/udx/brid/53394610_brid_6697_op.gml", &[]);
}

#[test]
fn plateau_city_furniture_survives_the_writer() {
    survives("plateau/udx/frn/53394631_frn_6697_op.gml", &[]);
}

#[test]
fn plateau_transportation_survives_the_writer() {
    survives("plateau/udx/tran/53394529_tran_6697_op.gml", &[]);
}

#[test]
fn plateau_vegetation_survives_the_writer() {
    survives("plateau/udx/veg/53394600_veg_6697_op.gml", &[]);
}

#[test]
fn plateau_water_bodies_survive_the_writer() {
    survives("plateau/udx/wtr/533945_wtr_6697_op.gml", &[]);
}

#[test]
fn plateau_ade_objects_survive_the_writer() {
    survives("plateau/udx/urf/533946_urf_6697_08_op.gml", &[]);
    survives("plateau/udx/ubld/53394611_ubld_6697_op.gml", &[]);
}

#[test]
fn plateau_ade_objects_survive_a_writer_told_no_schema() {
    // A writer is told where an ADE's schema is only for the document's
    // `xsi:schemaLocation`; what it nests where comes from the header.
    survives_with(
        "plateau/udx/ubld/53394611_ubld_6697_op.gml",
        &[],
        BTreeMap::new(),
    );
}

#[test]
fn a_lod4_building_and_relief_survive_the_writer() {
    survives("lod4_building_v2.gml", &[]);
    survives("tin_relief_fragment.gml", &[]);
}

#[test]
fn groups_and_implicit_geometries_survive_the_writer() {
    survives("railway_trees_fragment.gml", &[]);
    survives("plateau_trk_fragment.gml", &[]);
}

#[test]
fn a_composite_solid_survives_the_writer() {
    survives("b1_lod2_cs_w_sem.gml", &[]);
    survives("building_citymodel_appearance.gml", &[]);
}

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

/// `feature` with each object's geometries in LoD order, the order a
/// CityGML reader gives them in, and without what the CityGML 2.0 class of
/// an object says, which a reader restates: an installation's
/// `relationToConstruction`.
fn by_lod(mut feature: Value) -> Value {
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
        if let Some(geometries) = co.get_mut("geometry").and_then(Value::as_array_mut) {
            geometries.sort_by_key(|g| g["lod"].as_str().unwrap_or_default().to_string());
        }
    }
    feature
}

/// `feature`, made canonical, with its lines as a CityGML reader reads
/// them: a point repeated in a row once, as where two segments join, and a
/// line of fewer than two points, or a geometry of no line, left out.
fn lines_as_read(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        let Some(geometries) = co.get_mut("geometry").and_then(Value::as_array_mut) else {
            continue;
        };
        geometries.retain_mut(|g| {
            if g["type"] != "MultiLineString" {
                return true;
            }
            let lines = g["boundaries"].as_array_mut().unwrap();
            for line in lines.iter_mut() {
                line.as_array_mut().unwrap().dedup();
            }
            lines.retain(|l| l.as_array().unwrap().len() >= 2);
            !lines.is_empty()
        });
    }
    feature
}

/// The CityJSONSeq at `path`, which another tool wrote, written as CityGML
/// 2.0 and read back, says what its features say once in the model, as
/// `held` reduces them to what CityGML 2.0 holds, losing nothing but what
/// `lost` names.
fn foreign_survives(path: &str, lost: &[&str], held: fn(Value) -> Value) {
    let (header, features) = oracle::read_seq(&data(path));
    let header: CityJSON = serde_json::from_value(header).unwrap();
    let features: Vec<CityJSONFeature> = features
        .into_iter()
        .map(|f| serde_json::from_value(with_image_types(f)).unwrap())
        .collect();
    let out = scratch(path);
    let report = write(&header, &features, &out, BTreeMap::new());
    assert!(
        unexpected(&report.losses, lost).is_empty(),
        "{path}: {:?}",
        report.losses
    );
    // What the features say once in the model: each converted to it and
    // back, as the writer converts them.
    let declared = header.extensions.clone().unwrap_or_default();
    let cx = GmlContext {
        transform: header.transform.clone(),
        index: 0,
        vocabulary: Arc::new(Vocabulary2),
        extensions: Arc::new(Extensions::new(&declared, &BTreeMap::new())),
        template_lods: Arc::from(Vec::new()),
    };
    let mut before = Vec::new();
    for (index, feature) in (1..).zip(&features) {
        let model = feature
            .to_gml(&GmlContext {
                index,
                ..cx.clone()
            })
            .unwrap();
        let from = FeatureContext {
            transform: header.transform.clone(),
            index,
            ade: Default::default(),
        };
        for object in &model.objects {
            let again = CityJSONFeature::from_gml(object, &from).unwrap();
            before.push(lines_as_read(canonical(&by_lod(held(json(&again))))));
        }
    }
    let (_, after) = read(&out, &header.transform);
    let after: Vec<Value> = after.iter().map(|f| canonical(&by_lod(json(f)))).collect();
    same_features(path, &before, &after);
}

#[test]
fn citygml_tools_cityjson_survives_the_writer() {
    for file in [
        "53394519_bldg_6697_op",
        "53394529_tran_6697_op",
        "533945_wtr_6697_op",
        "53394600_veg_6697_op",
        "53394610_brid_6697_op",
        "53394631_frn_6697_op",
        "plateau_trk_fragment",
    ] {
        foreign_survives(
            &format!("oracle/{file}.city.jsonl"),
            &[
                "attributes whose value is blank text",
                "dates given with their midnight time",
            ],
            without_blanks,
        );
    }
}

/// A 3DBAG feature as CityGML 2.0 holds it: one geometry per whole LoD
/// (LoD 1.2 as LoD 1, LoD 1.3 left out), no semantic surfaces at LoD 1,
/// where CityGML 2.0's boundary surfaces have no geometry, and its
/// booleans as text, since a generic attribute cannot be one.
fn as_citygml2_holds_3dbag(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        if let Some(geometries) = co.get_mut("geometry").and_then(Value::as_array_mut) {
            let mut seen = Vec::new();
            geometries.retain_mut(|g| {
                let lod = g["lod"]
                    .as_str()
                    .unwrap()
                    .split('.')
                    .next()
                    .unwrap()
                    .to_string();
                if seen.contains(&lod) {
                    return false;
                }
                if lod == "1" {
                    g.as_object_mut().unwrap().remove("semantics");
                }
                g["lod"] = lod.clone().into();
                seen.push(lod);
                true
            });
        }
        if let Some(attributes) = co.get_mut("attributes").and_then(Value::as_object_mut) {
            booleans_as_text(attributes);
        }
        let geometries = co.get_mut("geometry").and_then(Value::as_array_mut);
        for g in geometries.into_iter().flatten() {
            let surfaces = g
                .pointer_mut("/semantics/surfaces")
                .and_then(Value::as_array_mut);
            for s in surfaces.into_iter().flatten() {
                booleans_as_text(s.as_object_mut().unwrap());
            }
        }
    }
    feature
}

fn booleans_as_text(members: &mut serde_json::Map<String, Value>) {
    for v in members.values_mut() {
        if let Value::Bool(b) = v {
            *v = b.to_string().into();
        }
    }
}

/// `feature` without the attributes whose value is blank text, which a
/// CityGML reader takes for no value.
fn without_blanks(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        if let Some(attributes) = co.get_mut("attributes").and_then(Value::as_object_mut) {
            attributes.retain(|_, v| !v.as_str().is_some_and(|s| s.trim().is_empty()));
            if attributes.is_empty() {
                co.as_object_mut().unwrap().remove("attributes");
            }
        }
    }
    feature
}

#[test]
fn three_d_bag_survives_the_writer_but_its_lods_booleans_and_nulls() {
    foreign_survives(
        "3dbag/delft_100.city.jsonl",
        &[
            "attributes whose value is null",
            "faces of boundary surfaces whose class has no geometry at their LoD (kept untagged)",
            "generic attributes whose value is a boolean (written as text)",
            "geometries at a LoD their object already has one of that kind at",
            "geometries whose LoD is written as its whole number",
        ],
        as_citygml2_holds_3dbag,
    );
}

/// What XML cannot hold is left out, and counted, rather than written as a
/// document no reader can read: a `+` name that is not an XML name, and a
/// generic attribute set with nothing in it. An object left out leaves its
/// children to be written on their own.
#[test]
fn what_xml_cannot_hold_is_left_out_and_counted() {
    let (mut header, features) = oracle::read_seq(&data("3dbag/delft_100.city.jsonl"));
    header["extensions"] = serde_json::json!({
        "bad": {"url": "https://example.org/bad.ext.json", "version": "1.0",
                "xmlns": "https://example.org/bad"}
    });
    let mut feature = features.into_iter().next().unwrap();
    let id = feature["id"].as_str().unwrap().to_string();
    let objects = feature["CityObjects"].as_object_mut().unwrap();
    let parts = objects[&id]["children"].as_array().unwrap().len();
    assert!(parts > 0);
    let building = objects.get_mut(&id).unwrap();
    building["type"] = "+Bad Type".into();
    let part = objects
        .values_mut()
        .find(|o| o["type"] == "BuildingPart")
        .unwrap();
    let attributes = part["attributes"].as_object_mut().unwrap();
    attributes.insert("+bad name".into(), "x".into());
    attributes.insert("empty".into(), serde_json::json!({}));
    attributes.insert("blanks".into(), serde_json::json!({"a": " "}));

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature: CityJSONFeature = serde_json::from_value(feature).unwrap();
    let out = scratch("what_xml_cannot_hold");
    let report = write(
        &header,
        std::slice::from_ref(&feature),
        &out,
        BTreeMap::new(),
    );
    for (loss, n) in [
        ("objects of an ADE class whose name is not an XML name", 1),
        ("ADE elements whose name is not an XML name", 1),
        ("generic attribute sets with no member to write", 2),
    ] {
        assert_eq!(
            count(&report.losses, loss),
            n,
            "{loss}: {:?}",
            report.losses
        );
    }
    assert_eq!(
        count(
            &report.losses,
            "objects their parent's class cannot hold (written on their own)"
        ),
        parts
    );
    assert_eq!(report.objects, parts);
    // The document reads, and holds the parts as members of their own,
    // which this reader, like CityJSON, does not take for objects.
    let mut reader = FeatureReader::open(&out).unwrap();
    assert_eq!(reader.by_ref().map(Result::unwrap).count(), 0);
    assert_eq!(
        reader.skipped_members().get("bldg:BuildingPart"),
        Some(&parts)
    );
}

/// A 3DBAG feature split in two, its Building in one feature and a
/// BuildingPart in another (the vertices and appearance in both), in the
/// order given.
fn split(feature: &Value, part_first: bool) -> Vec<CityJSONFeature> {
    let id = feature["id"].as_str().unwrap();
    let objects = feature["CityObjects"].as_object().unwrap();
    let (part_id, _) = objects
        .iter()
        .find(|(_, o)| o["type"] == "BuildingPart")
        .unwrap();
    let mut building = feature.clone();
    building["CityObjects"]
        .as_object_mut()
        .unwrap()
        .remove(part_id);
    let mut part = feature.clone();
    part["id"] = part_id.clone().into();
    part["CityObjects"]
        .as_object_mut()
        .unwrap()
        .retain(|k, _| k == part_id);
    assert_ne!(id, part_id);
    let mut features = vec![building, part];
    if part_first {
        features.reverse();
    }
    features
        .into_iter()
        .map(|f| serde_json::from_value(f).unwrap())
        .collect()
}

/// A part whose parent is in another feature is written inside its
/// parent, whichever of the two comes first, as if they were one feature.
#[test]
fn a_part_in_a_feature_of_its_own_is_written_inside_its_parent() {
    let (header, features) = oracle::read_seq(&data("3dbag/delft_100.city.jsonl"));
    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature = features
        .into_iter()
        .find(|f| {
            let objects = f["CityObjects"].as_object().unwrap();
            objects.len() == 2 && objects.values().any(|o| o["type"] == "BuildingPart")
        })
        .unwrap();
    let whole: CityJSONFeature = serde_json::from_value(feature.clone()).unwrap();
    let out = scratch("a_part_whole");
    let expected = write(&header, std::slice::from_ref(&whole), &out, BTreeMap::new());
    let (_, expected_back) = read(&out, &header.transform);
    for part_first in [false, true] {
        let out = scratch(&format!("a_part_split_{part_first}"));
        let report = write(&header, &split(&feature, part_first), &out, BTreeMap::new());
        assert_eq!(report.objects, 1, "part first: {part_first}");
        assert_eq!(report.losses, expected.losses, "part first: {part_first}");
        let (_, back) = read(&out, &header.transform);
        let canonical_all = |fs: &[CityJSONFeature]| -> Vec<Value> {
            fs.iter().map(|f| canonical(&json(f))).collect()
        };
        same_features(
            "a part in a feature of its own",
            &canonical_all(&expected_back),
            &canonical_all(&back),
        );
    }
}

/// A part whose parent is in no feature is written on its own at the end,
/// and counted.
#[test]
fn a_part_whose_parent_never_comes_is_written_on_its_own() {
    let (header, features) = oracle::read_seq(&data("3dbag/delft_100.city.jsonl"));
    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature = features
        .into_iter()
        .find(|f| f["CityObjects"].as_object().unwrap().len() == 2)
        .unwrap();
    let part = split(&feature, true).remove(0);
    let out = scratch("a_part_alone");
    let report = write(&header, std::slice::from_ref(&part), &out, BTreeMap::new());
    assert_eq!(report.objects, 1);
    assert_eq!(
        count(
            &report.losses,
            "parts whose parent was not written as their parent (written on their own)"
        ),
        1,
        "{:?}",
        report.losses
    );
    // The part is in the document, where a reader finds it but, like
    // CityJSON, does not take it for an object.
    let mut reader = FeatureReader::open(&out).unwrap();
    assert_eq!(reader.by_ref().map(Result::unwrap).count(), 0);
    assert_eq!(reader.skipped_members().get("bldg:BuildingPart"), Some(&1));
}

/// A 3DBAG feature whose BuildingPart is put under a BuildingStorey (a
/// copy of the part, with no geometry), a type CityGML 2.0 has no class
/// for: Building, storey and part.
fn with_storey(feature: &Value) -> (Value, [String; 3]) {
    let mut feature = feature.clone();
    let id = feature["id"].as_str().unwrap().to_string();
    let objects = feature["CityObjects"].as_object_mut().unwrap();
    let part = objects
        .iter()
        .find(|(_, o)| o["type"] == "BuildingPart")
        .map(|(k, _)| k.clone())
        .unwrap();
    let storey = format!("{part}-storey");
    let mut s = objects[&part].clone();
    s["type"] = "BuildingStorey".into();
    s.as_object_mut().unwrap().remove("geometry");
    s.as_object_mut().unwrap().remove("attributes");
    s["children"] = serde_json::json!([part]);
    objects.insert(storey.clone(), s);
    objects[&part]["parents"] = serde_json::json!([storey]);
    objects[&id]["children"] = serde_json::json!([storey]);
    (feature, [id, storey, part])
}

/// The feature `with_storey` gives, split into one feature per object.
fn one_per_object(feature: &Value, ids: &[String; 3]) -> Vec<Value> {
    ids.iter()
        .map(|id| {
            let mut f = feature.clone();
            f["id"] = id.clone().into();
            f["CityObjects"]
                .as_object_mut()
                .unwrap()
                .retain(|k, _| k == id);
            f
        })
        .collect()
}

/// A part whose parent has no class in the version takes that parent's
/// place in its grandparent, across features as within one, whatever the
/// order the three come in.
#[test]
fn a_part_under_a_classless_parent_in_another_feature_lands_in_its_grandparent() {
    let (header, features) = oracle::read_seq(&data("3dbag/delft_100.city.jsonl"));
    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature = features
        .into_iter()
        .find(|f| f["CityObjects"].as_object().unwrap().len() == 2)
        .unwrap();
    let (whole, ids) = with_storey(&feature);
    let whole: CityJSONFeature = serde_json::from_value(whole.clone()).unwrap();
    let out = scratch("a_storey_whole");
    let expected = write(&header, std::slice::from_ref(&whole), &out, BTreeMap::new());
    let (_, expected_back) = read(&out, &header.transform);
    let (whole, _) = with_storey(&feature);
    let parts = one_per_object(&whole, &ids);
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let features: Vec<CityJSONFeature> = order
            .iter()
            .map(|&i| serde_json::from_value(parts[i].clone()).unwrap())
            .collect();
        let out = scratch(&format!("a_storey_{order:?}"));
        let report = write(&header, &features, &out, BTreeMap::new());
        assert_eq!(report.objects, 1, "order {order:?}");
        assert_eq!(report.losses, expected.losses, "order {order:?}");
        let (_, back) = read(&out, &header.transform);
        let canonical_all = |fs: &[CityJSONFeature]| -> Vec<Value> {
            fs.iter().map(|f| canonical(&json(f))).collect()
        };
        same_features(
            &format!("a storey, order {order:?}"),
            &canonical_all(&expected_back),
            &canonical_all(&back),
        );
    }
}
