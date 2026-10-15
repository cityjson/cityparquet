//! CityGML 3.0 written from CityJSON: each real file's features, written
//! as a CityGML 3.0 document and read back, say what they said before.

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
use citygml3::writer::{Report, Vocabulary3, Writer, WriterOptions};
use citygml3::{FeatureReader, ObjectStream};
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
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("citygml3_writer_{file}.gml"))
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

/// `header` and `features` written as CityGML 3.0 at `out`, with the ADE
/// schemas `schemas` gives by namespace, and valid.
fn write(
    header: &CityJSON,
    features: &[CityJSONFeature],
    out: &Path,
    schemas: BTreeMap<String, String>,
) -> Report {
    let report = write_unvalidated(header, features, out, schemas);
    validate(out);
    report
}

/// [`write`], for a document whose ADE has no schema to validate against.
fn write_unvalidated(
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
    report
}

/// `path` is valid against the CityGML 3.0 schemas and its ADEs', as
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
    let document = citygml3::read_document(path).unwrap();
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
    let transform = Transform {
        scale: [1e-9, 1e-9, 1e-3],
        translate: [0.0; 3],
    };
    let (header, before) = read(&data(gml), &transform);
    let out = scratch(gml);
    let report = write(&header, &before, &out, ade_schemas(&data(gml)));
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
    survives("plateau/53394519_bldg_6697_op.gml", &[]);
}

#[test]
fn plateau_bridges_survive_the_writer() {
    survives("plateau/53394610_brid_6697_op.gml", &[]);
}

#[test]
fn plateau_transportation_survives_the_writer() {
    survives("plateau/53394529_tran_6697_op.gml", &[]);
}

#[test]
fn plateau_vegetation_survives_the_writer() {
    survives("plateau/53394600_veg_6697_op.gml", &[]);
}

#[test]
fn a_lod3_building_and_relief_survive_the_writer() {
    survives("lod4_building_v2.gml", &[]);
    survives("tin_relief_fragment.gml", &[]);
}

#[test]
fn groups_and_implicit_geometries_survive_the_writer() {
    survives("railway_trees_fragment.gml", &[]);
}

#[test]
fn a_composite_solid_survives_the_writer() {
    survives("b1_lod2_cs_w_sem.gml", &[]);
}

#[test]
fn the_ogc_samples_survive_the_writer() {
    survives("ogc/BuildingUnits_Storeys_xlink.gml", &[]);
    survives("ogc/citygml3_individual_section_1carriageway.gml", &[]);
}

#[test]
fn ade_properties_and_classes_survive_the_writer() {
    survives("ogc/test-ade/industrial_building_dataset_v3.gml", &[]);
    survives(
        "ogc/urban-planning-ade/53394517_bldg_6697_2_op_CityGML3.gml",
        &[],
    );
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
/// CityGML reader gives them in.
fn by_lod(mut feature: Value) -> Value {
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
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
/// 3.0 and read back, says what its features say once in the model, as
/// `held` reduces them to what CityGML 3.0 holds, losing nothing but what
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
        vocabulary: Arc::new(Vocabulary3),
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
            &format!("../../../citygml2/tests/data/oracle/{file}.city.jsonl"),
            &["attributes whose value is blank text"],
            without_blanks,
        );
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

/// A 3DBAG feature as CityGML 3.0 holds it: one geometry per whole LoD
/// (LoD 1.2 as LoD 1, LoD 1.3 left out), and its booleans as text, since a
/// generic attribute cannot be one.
fn as_citygml3_holds_3dbag(mut feature: Value) -> Value {
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

#[test]
fn three_d_bag_survives_the_writer_but_its_lods_booleans_and_nulls() {
    foreign_survives(
        "../../../citygml2/tests/data/3dbag/delft_100.city.jsonl",
        &[
            "attributes whose value is null",
            "generic attributes whose value is a boolean (written as text)",
            "geometries at a LoD their object already has one of that kind at",
            "geometries whose LoD is written as its whole number",
        ],
        as_citygml3_holds_3dbag,
    );
}

/// `feature`'s geometries at LoD `from` relabelled LoD `to`; how many.
fn relabel(feature: &mut Value, from: &str, to: &str) -> usize {
    let mut n = 0;
    for co in feature["CityObjects"].as_object_mut().unwrap().values_mut() {
        for g in co
            .get_mut("geometry")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if g["lod"] == from {
                g["lod"] = to.into();
                n += 1;
            }
        }
    }
    n
}

/// The LoDs of the geometries of the features of the CityGML at `path`.
fn lods(path: &Path) -> Vec<Vec<String>> {
    let transform = Transform {
        scale: [1e-3; 3],
        translate: [0.0; 3],
    };
    let (_, features) = read(path, &transform);
    features
        .iter()
        .map(|f| {
            let mut lods: Vec<String> = f
                .city_objects
                .values()
                .flat_map(|co| co.geometry.iter().flatten())
                .map(|g| json(g)["lod"].as_str().unwrap().to_string())
                .collect();
            lods.sort();
            lods
        })
        .collect()
}

/// CityGML 3.0 has no LoD 4: a LoD 4 geometry is written at LoD 3 when its
/// object has none there, and is left out, and counted, when it has.
#[test]
fn lod4_is_written_at_lod3_unless_its_object_has_lod3() {
    let (header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/3dbag/delft_100.city.jsonl",
    ));
    let mut features: Vec<Value> = features.into_iter().take(2).collect();
    let moved = relabel(&mut features[0], "2.2", "4");
    let dropped = relabel(&mut features[1], "2.2", "4");
    let lod3 = relabel(&mut features[1], "1.2", "3");
    assert!(moved > 0 && dropped > 0 && lod3 > 0);

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let features: Vec<CityJSONFeature> = features
        .into_iter()
        .map(|f| serde_json::from_value(f).unwrap())
        .collect();
    let out = scratch("lod4");
    let report = write(&header, &features, &out, BTreeMap::new());
    assert_eq!(
        count(&report.losses, "LoD 4 geometries written at LoD 3"),
        moved
    );
    assert_eq!(
        count(
            &report.losses,
            "LoD 4 geometries of objects that have geometry at LoD 3"
        ),
        dropped
    );
    let lods = lods(&out);
    assert_eq!(lods[0].iter().filter(|l| *l == "3").count(), moved);
    assert_eq!(lods[1].iter().filter(|l| *l == "3").count(), lod3);
    assert!(lods.iter().flatten().all(|l| l != "4"));
}

/// An ADE property whose hook is another CityGML version's says nothing
/// about where in CityGML 3.0 it goes: it is left out and counted, and the
/// document stays valid.
#[test]
fn ade_properties_for_another_version_are_left_out_and_counted() {
    let (mut header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/3dbag/delft_100.city.jsonl",
    ));
    header["extensions"] = serde_json::json!({
        "uro": {
            "url": "https://example.org/uro.ext.json",
            "version": "2.0",
            "xmlns": "https://www.geospatial.jp/iur/uro/2.0",
            "substitutionGroups": {
                "buildingDetails":
                    "{http://www.opengis.net/citygml/building/2.0}_GenericApplicationPropertyOfAbstractBuilding"
            }
        }
    });
    let mut feature = features.into_iter().next().unwrap();
    let id = feature["id"].as_str().unwrap().to_string();
    feature["CityObjects"][&id]["attributes"]["+buildingDetails"] = "2016".into();

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature: CityJSONFeature = serde_json::from_value(feature).unwrap();
    let out = scratch("ade_for_another_version");
    let report = write(
        &header,
        std::slice::from_ref(&feature),
        &out,
        BTreeMap::new(),
    );
    assert_eq!(
        count(
            &report.losses,
            "ADE properties of an ADE for another CityGML version"
        ),
        1,
        "{:?}",
        report.losses
    );
}

/// A CityGML 3.0 group member only refers to its member: a member the
/// group's feature holds is written as an object of its own, referred to,
/// and counted. Derived from the railway fragment's CityJSON, whose group
/// refers to trees that are features of their own: here the group's
/// feature holds them, as a CityJSON document's group does.
#[test]
fn members_a_group_holds_are_written_as_objects_of_their_own() {
    let (header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/oracle/railway_trees_fragment.city.jsonl",
    ));
    let (mut groups, trees): (Vec<Value>, Vec<Value>) = features
        .into_iter()
        .partition(|f| f["CityObjects"][f["id"].as_str().unwrap()]["type"] == "CityObjectGroup");
    let mut group = groups.remove(0);
    let group_id = group["id"].as_str().unwrap().to_string();
    let members: Vec<String> = group["CityObjects"][&group_id]["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    // Each tree's objects move into the group's feature, their vertex
    // indices past the vertices already there.
    let mut held = 0;
    for tree in trees {
        let offset = group["vertices"].as_array().unwrap().len() as u64;
        for (id, mut object) in tree["CityObjects"].as_object().unwrap().clone() {
            if !members.contains(&id) {
                continue;
            }
            for g in object["geometry"].as_array_mut().unwrap() {
                assert_eq!(g["type"], "GeometryInstance");
                let v = g["boundaries"][0].as_u64().unwrap();
                g["boundaries"][0] = (v + offset).into();
            }
            object["parents"] = serde_json::json!([group_id]);
            group["CityObjects"][&id] = object;
            held += 1;
        }
        let vertices = tree["vertices"].as_array().unwrap().clone();
        group["vertices"].as_array_mut().unwrap().extend(vertices);
    }
    assert!(held > 1);

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature: CityJSONFeature = serde_json::from_value(group).unwrap();
    let out = scratch("group_holding_its_members");
    let report = write(
        &header,
        std::slice::from_ref(&feature),
        &out,
        BTreeMap::new(),
    );
    assert_eq!(
        count(
            &report.losses,
            "group members written as objects of their own, referred to"
        ),
        held,
        "{:?}",
        report.losses
    );
    let (_, after) = read(&out, &header.transform);
    let mut referred = Vec::new();
    for f in &after {
        for (id, object) in &f.city_objects {
            if *id == group_id {
                assert_eq!(object.children.as_deref().unwrap(), &members[..]);
            } else if members.contains(id) {
                assert_eq!(
                    object.parents.as_deref().unwrap(),
                    std::slice::from_ref(&group_id)
                );
                referred.push(id.clone());
            }
        }
    }
    assert_eq!(referred.len(), held);
}

/// CityGML keeps one geometry of a kind per whole LoD: of two
/// MultiSurfaces at LoDs 2.1 and 2.2 the second is left out, and counted,
/// rather than written as another boundary surface that reads back merged
/// into the first. Derived from 3DBAG's first building: its LoD 0
/// footprint, copied to LoDs 2.1 and 2.2.
#[test]
fn a_second_multisurface_at_a_whole_lod_is_left_out_and_counted() {
    let (header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/3dbag/delft_100.city.jsonl",
    ));
    let header: CityJSON = serde_json::from_value(header).unwrap();
    let original = features.into_iter().next().unwrap();
    let mut feature = original.clone();
    let id = feature["id"].as_str().unwrap().to_string();
    let geometries = feature["CityObjects"][&id]["geometry"]
        .as_array_mut()
        .unwrap();
    let footprint = geometries
        .iter()
        .find(|g| g["lod"] == "0" && g["type"] == "MultiSurface")
        .unwrap()
        .clone();
    for lod in ["2.1", "2.2"] {
        let mut copy = footprint.clone();
        copy["lod"] = lod.into();
        geometries.push(copy);
    }

    // What the feature loses, and holds at LoD 2, without the copies and
    // with them.
    let written = |feature: Value, name: &str| {
        let feature: CityJSONFeature = serde_json::from_value(feature).unwrap();
        let out = scratch(name);
        let report = write(
            &header,
            std::slice::from_ref(&feature),
            &out,
            BTreeMap::new(),
        );
        let lod2 = lods(&out)[0].iter().filter(|l| *l == "2").count();
        let lost = count(
            &report.losses,
            "geometries at a LoD their object already has one of that kind at",
        );
        (lost, lod2)
    };
    let (lost_before, lod2_before) = written(original, "one_multisurface_at_lod2");
    let (lost, lod2) = written(feature, "two_multisurfaces_at_lod2");
    assert_eq!(lost, lost_before + 1);
    assert_eq!(lod2, lod2_before + 1);
}

/// CityGML 3.0 has one closure surface class, which on a water body is
/// CityJSON's `WaterClosureSurface`. Derived from PLATEAU's first water
/// body: its LoD 1 surface, at LoD 2, as a water closure surface.
#[test]
fn a_water_closure_surface_reads_back_as_one() {
    let (header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/oracle/533945_wtr_6697_op.city.jsonl",
    ));
    let mut feature = features.into_iter().next().unwrap();
    let id = feature["id"].as_str().unwrap().to_string();
    let surface = feature["CityObjects"][&id]["geometry"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|g| g["type"] == "MultiSurface")
        .unwrap();
    let faces = surface["boundaries"].as_array().unwrap().len();
    surface["lod"] = "2".into();
    surface["semantics"] = serde_json::json!({
        "surfaces": [{"type": "WaterClosureSurface"}],
        "values": vec![0; faces],
    });

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature: CityJSONFeature = serde_json::from_value(feature).unwrap();
    let out = scratch("water_closure_surface");
    let report = write(
        &header,
        std::slice::from_ref(&feature),
        &out,
        BTreeMap::new(),
    );
    assert!(report.losses.is_empty(), "{:?}", report.losses);
    let (_, after) = read(&out, &header.transform);
    let types: Vec<Value> = after
        .iter()
        .flat_map(|f| f.city_objects.values())
        .flat_map(|o| o.geometry.iter().flatten())
        .flat_map(|g| {
            json(g)["semantics"]["surfaces"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .map(|s| s["type"].clone())
        .collect();
    assert_eq!(types, ["WaterClosureSurface"]);
}

/// A curve beside a surface at one LoD is a geometry of another kind, and
/// reads back as its own. Derived from 3DBAG's first building: a line along
/// the first edge of its LoD 0 footprint, at LoD 0.
#[test]
fn a_curve_beside_a_surface_at_one_lod_reads_back() {
    let (header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/3dbag/delft_100.city.jsonl",
    ));
    let mut feature = features.into_iter().next().unwrap();
    let id = feature["id"].as_str().unwrap().to_string();
    let geometries = feature["CityObjects"][&id]["geometry"]
        .as_array_mut()
        .unwrap();
    let ring = geometries
        .iter()
        .find(|g| g["lod"] == "0" && g["type"] == "MultiSurface")
        .unwrap()["boundaries"][0][0]
        .clone();
    geometries.push(serde_json::json!({
        "type": "MultiLineString",
        "lod": "0",
        "boundaries": [[ring[0], ring[1]]],
    }));

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature: CityJSONFeature = serde_json::from_value(feature).unwrap();
    let out = scratch("curve_beside_surface");
    write(
        &header,
        std::slice::from_ref(&feature),
        &out,
        BTreeMap::new(),
    );
    let (_, after) = read(&out, &header.transform);
    let at_lod0: Vec<String> = after[0].city_objects[&id]
        .geometry
        .iter()
        .flatten()
        .map(json)
        .filter(|g| g["lod"] == "0")
        .map(|g| g["type"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(at_lod0, ["MultiSurface", "MultiLineString"]);
}

/// An object of an ADE class whose CityGML class is not known has a city
/// object's boundary surfaces. Derived from 3DBAG's first building: its
/// part, as an object of a class of an ADE whose schema is not at hand,
/// and so cannot be validated.
#[test]
fn an_ade_object_of_unknown_class_keeps_its_boundary_surfaces() {
    let (mut header, features) = oracle::read_seq(&data(
        "../../../citygml2/tests/data/3dbag/delft_100.city.jsonl",
    ));
    header["extensions"] = serde_json::json!({
        "Storage": {
            "url": "https://example.org/storage.xsd",
            "version": "1.0",
            "xmlns": "https://example.org/storage/1.0",
            "extraCityObjects": {"+Warehouse": {}}
        }
    });
    let mut feature = features.into_iter().next().unwrap();
    let id = feature["id"].as_str().unwrap().to_string();
    let part = format!("{id}-0");
    feature["CityObjects"][&part]["type"] = "+Warehouse".into();

    let header: CityJSON = serde_json::from_value(header).unwrap();
    let feature: CityJSONFeature = serde_json::from_value(feature).unwrap();
    let out = scratch("ade_object_of_unknown_class");
    let report = write_unvalidated(
        &header,
        std::slice::from_ref(&feature),
        &out,
        BTreeMap::new(),
    );
    assert!(
        report
            .losses
            .iter()
            .all(|(what, _)| !what.contains("surface")),
        "{:?}",
        report.losses
    );
    let (_, after) = read(&out, &header.transform);
    let roofs = after
        .iter()
        .flat_map(|f| f.city_objects.iter())
        .filter(|(id, _)| **id == part)
        .flat_map(|(_, o)| o.geometry.iter().flatten())
        .flat_map(|g| {
            json(g)["semantics"]["surfaces"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter(|s| s["type"] == "RoofSurface")
        .count();
    assert!(roofs > 0);
}
