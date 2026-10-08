//! A real CityGML 2.0 LoD4 building — the SIG3D "Simple 3D city model LOD4"
//! example as citygml4j ships it — survives CityGML → package → CityGML and
//! package → CityJSON at LoD 4.
//!
//! Its `bldg:lod4Solid` composes its shell from `xlink:href`s, two of them to
//! the `gml:CompositeSurface` a `boundedBy` wall wraps its polygons in, so
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

/// The faces of the Building's `bldg:lod4Solid`. Its exterior shell has ten
/// `xlink:href` members: eight name a `gml:Polygon`, and two name a wall's
/// `gml:CompositeSurface`, of 9 (`GML_1d350a50…`, Wall South) and 5
/// (`GML_6286ffa9…`, Wall East) inline polygons — 8 + 9 + 5 = 22. Counted
/// with an XPath walk over the fixture: for each `surfaceMember` of
/// `bldg:Building/bldg:lod4Solid//gml:exterior/gml:CompositeSurface`, 1 for a
/// `gml:Polygon` target, else the target's `gml:Polygon` descendants.
const LOD4_SOLID_FACES: usize = 22;

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
    assert_eq!(faces, LOD4_SOLID_FACES);
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

/// An xlink to a `gml:CompositeSurface` whose members are themselves
/// `xlink:href`s stands for the polygons those name. Derived from the real
/// LoD4 fixture: Wall South's CompositeSurface (`GML_1d350a50…`) keeps its id
/// but its nine inline polygons move out beside it, each replaced by an
/// `xlink:href` to the moved polygon — the same geometry, spelled the other
/// way GML permits.
#[test]
fn a_composite_surface_of_xlinked_members_expands_to_the_polygons_they_name() {
    let text = std::fs::read_to_string(fixture("lod4_building_v2.gml")).unwrap();
    let open = r#"<gml:CompositeSurface gml:id="GML_1d350a50-6acc-4d3c-8c28-326ca4305fd1">"#;
    let start = text.find(open).expect("Wall South's CompositeSurface");
    let end = start + text[start..].find("</gml:CompositeSurface>").unwrap();
    let inner = &text[start + open.len()..end];
    let mut polygons = Vec::new();
    let mut xlinks = String::new();
    let mut rest = inner;
    while let Some(at) = rest.find("<gml:Polygon gml:id=\"") {
        let close = at + rest[at..].find("</gml:Polygon>").unwrap() + "</gml:Polygon>".len();
        let polygon = &rest[at..close];
        let id_start = "<gml:Polygon gml:id=\"".len();
        let id = &polygon[id_start..id_start + polygon[id_start..].find('"').unwrap()];
        xlinks.push_str(&format!(r##"<gml:surfaceMember xlink:href="#{id}"/>"##));
        polygons.push(format!("<gml:surfaceMember>{polygon}</gml:surfaceMember>"));
        rest = &rest[close..];
    }
    assert_eq!(polygons.len(), 9);
    // The polygons follow the surfaceMember that held the composite.
    let member_end =
        end + text[end..].find("</gml:surfaceMember>").unwrap() + "</gml:surfaceMember>".len();
    let derived = format!(
        "{}{open}{xlinks}</gml:CompositeSurface>{}{}{}",
        &text[..start],
        &text[end + "</gml:CompositeSurface>".len()..member_end],
        polygons.concat(),
        &text[member_end..]
    );
    let dir = tempfile::tempdir().unwrap();
    let gml = dir.path().join("xlinked_members.gml");
    std::fs::write(&gml, derived).unwrap();

    let pkg = dir.path().join("pkg");
    convert_to(&gml, &pkg);
    assert_eq!(building_lod4(&pkg), ("Solid".to_string(), LOD4_SOLID_FACES));
}

const WALL_SOUTH: &str = "GML_1d350a50-6acc-4d3c-8c28-326ca4305fd1";

/// The real LoD4 fixture with `members` added as further `surfaceMember`s of
/// Wall South's `gml:MultiSurface`, right after its CompositeSurface.
fn with_extra_members(members: &str) -> String {
    let text = std::fs::read_to_string(fixture("lod4_building_v2.gml")).unwrap();
    let start = text.find(&format!(r#"gml:id="{WALL_SOUTH}""#)).unwrap();
    let close = start + text[start..].find("</gml:MultiSurface>").unwrap();
    format!("{}{members}{}", &text[..close], &text[close..])
}

fn convert_error(gml: String) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derived.gml");
    std::fs::write(&path, gml).unwrap();
    let out = dir.path().join("pkg");
    match convert(&ConvertOptions::new(path, out)) {
        Ok(_) => panic!("the conversion must be refused"),
        Err(e) => e.to_string(),
    }
}

/// A cycle of aggregate xlinks names no polygon and would never end. No real
/// file carries one — a GML aggregate cannot contain itself — so this is the
/// real fixture with Wall South's CompositeSurface given a member that is an
/// xlink to a MultiSurface whose member xlinks back to it.
#[test]
fn an_xlink_cycle_between_aggregates_is_refused_by_name() {
    let gml = with_extra_members(&format!(
        r##"<gml:surfaceMember><gml:MultiSurface gml:id="LOOP"><gml:surfaceMember xlink:href="#{WALL_SOUTH}"/></gml:MultiSurface></gml:surfaceMember>"##
    ))
    .replacen(
        &format!(r#"<gml:CompositeSurface gml:id="{WALL_SOUTH}">"#),
        &format!(
            r##"<gml:CompositeSurface gml:id="{WALL_SOUTH}"><gml:surfaceMember xlink:href="#LOOP"/>"##
        ),
        1,
    );
    let error = convert_error(gml);
    assert!(
        error.contains("cycle") && error.contains(WALL_SOUTH),
        "{error}"
    );
}

/// Aggregates nested `k` deep, each xlinking twice to the next, stand for
/// 2^k copies of one polygon — an exponential expansion from a few lines of
/// XML. No real file does this: a solid repeating one face is not a solid.
/// Derived from the real fixture by adding such a chain (k = 20, about a
/// million faces) and pointing the lod4Solid's Wall South member at its head;
/// the reader must refuse it rather than allocate every copy.
#[test]
fn an_exponential_xlink_fan_out_is_refused() {
    const DEPTH: usize = 20;
    let polygon = "PolyID10204_1916_571790_369478";
    let mut chain = String::new();
    for level in 0..DEPTH {
        let next = if level + 1 == DEPTH {
            polygon.to_string()
        } else {
            format!("FAN{}", level + 1)
        };
        chain.push_str(&format!(
            r##"<gml:surfaceMember><gml:CompositeSurface gml:id="FAN{level}"><gml:surfaceMember xlink:href="#{next}"/><gml:surfaceMember xlink:href="#{next}"/></gml:CompositeSurface></gml:surfaceMember>"##
        ));
    }
    let shell_member = format!(r##"<gml:surfaceMember xlink:href="#{WALL_SOUTH}"/>"##);
    let gml = with_extra_members(&chain).replacen(
        &shell_member,
        r##"<gml:surfaceMember xlink:href="#FAN0"/>"##,
        1,
    );
    let error = convert_error(gml);
    assert!(error.contains("FAN0"), "{error}");
}

/// A chain of aggregates, each xlinking once to the next, is linear in the
/// document but as deep as it is long; resolving it recursively would
/// overflow the stack before any budget is spent. No real file nests
/// xlinked aggregates more than a few levels — GML has no reason to — so
/// this is the real fixture with a chain of 100 000 MultiSurfaces added and
/// the lod4Solid's Wall South member pointed at its head. It must be
/// refused by a depth limit, with an error, not a crash.
#[test]
fn a_deep_chain_of_xlinked_aggregates_is_refused() {
    const LENGTH: usize = 100_000;
    let polygon = "PolyID10204_1916_571790_369478";
    let mut chain = String::with_capacity(LENGTH * 120);
    for i in 0..LENGTH {
        let next = if i + 1 == LENGTH {
            polygon.to_string()
        } else {
            format!("CHAIN{}", i + 1)
        };
        chain.push_str(&format!(
            r##"<gml:surfaceMember><gml:MultiSurface gml:id="CHAIN{i}"><gml:surfaceMember xlink:href="#{next}"/></gml:MultiSurface></gml:surfaceMember>"##
        ));
    }
    let shell_member = format!(r##"<gml:surfaceMember xlink:href="#{WALL_SOUTH}"/>"##);
    let gml = with_extra_members(&chain).replacen(
        &shell_member,
        r##"<gml:surfaceMember xlink:href="#CHAIN0"/>"##,
        1,
    );
    let error = convert_error(gml);
    assert!(
        error.contains("CHAIN0") && error.contains("deep"),
        "{error}"
    );
}

/// The budget is a multiple of the polygons the document defines, each
/// counted once. Wrapping one polygon in many nested identified aggregates
/// adds tags, not polygons, and must not raise it: otherwise those wrappers
/// would pay for a fan-out of the same order. Derived from the real fixture
/// like the fan-out test, with one polygon wrapped 2 000 deep beside a
/// 2^11-copy fan-out of it — far beyond the document's few dozen polygons,
/// but within a budget that counted the polygon once per wrapper.
#[test]
fn nested_aggregate_wrappers_do_not_raise_the_expansion_budget() {
    const WRAPPERS: usize = 2_000;
    const FAN_DEPTH: usize = 11;
    let mut members = String::from("<gml:surfaceMember>");
    for i in 0..WRAPPERS {
        members.push_str(&format!(
            r#"<gml:MultiSurface gml:id="WRAP{i}"><gml:surfaceMember>"#
        ));
    }
    members.push_str(
        r#"<gml:Polygon gml:id="WRAPPED"><gml:exterior><gml:LinearRing><gml:posList>458878.5 5438350 113.2 458878.5 5438350 114.2 458878.5 5438350.1 114.2 458878.5 5438350 113.2</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon>"#,
    );
    for _ in 0..WRAPPERS {
        members.push_str("</gml:surfaceMember></gml:MultiSurface>");
    }
    members.push_str("</gml:surfaceMember>");
    for level in 0..FAN_DEPTH {
        let next = if level + 1 == FAN_DEPTH {
            "WRAPPED".to_string()
        } else {
            format!("FAN{}", level + 1)
        };
        members.push_str(&format!(
            r##"<gml:surfaceMember><gml:CompositeSurface gml:id="FAN{level}"><gml:surfaceMember xlink:href="#{next}"/><gml:surfaceMember xlink:href="#{next}"/></gml:CompositeSurface></gml:surfaceMember>"##
        ));
    }
    let shell_member = format!(r##"<gml:surfaceMember xlink:href="#{WALL_SOUTH}"/>"##);
    let gml = with_extra_members(&members).replacen(
        &shell_member,
        r##"<gml:surfaceMember xlink:href="#FAN0"/>"##,
        1,
    );
    let error = convert_error(gml);
    assert!(error.contains("FAN0"), "{error}");
}
