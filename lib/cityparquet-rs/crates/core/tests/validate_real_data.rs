//! `cityparquet::validate` against real packages: what this crate writes from
//! the real fixtures conforms, and a real package mutated the way a faulty
//! writer could have written it reports exactly the violated rule.

mod support;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::Int32Type;
use arrow_array::{Array, ArrayRef, BinaryArray, ListArray, StringArray, StructArray, make_array};
use arrow_buffer::OffsetBuffer;
use arrow_schema::{DataType, Field};
use cityparquet::validate::{Severity, ValidationReport, validate_package};
use serde_json::{Value, json};

use support::{FileContent, convert_fixture, convert_fixture_with};

fn validate(pkg: &Path) -> ValidationReport {
    validate_package(pkg).expect("the package directory is readable")
}

fn assert_error(report: &ValidationReport, code: &str) {
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == code && v.severity == Severity::Error),
        "expected an error `{code}`, got:\n{report}"
    );
}

#[test]
fn rs_written_packages_from_the_real_fixtures_conform() {
    for name in [
        "delft.city.jsonl",
        "lod3_railway.city.json",
        "b1_lod2_cs_w_sem.gml",
        "b1_lod2_s.gml",
    ] {
        // With and without the synthesised LoD0 footprint column.
        for generate_lod0 in [false, true] {
            let (_dir, pkg) = convert_fixture_with(name, |o| o.generate_lod0 = generate_lod0);
            let report = validate(&pkg);
            assert!(
                report.violations.is_empty(),
                "{name} (LoD0 synthesis {generate_lod0}) must validate clean:\n{report}"
            );
        }
    }
}

/// Conformance is at the Parquet logical-type level: the same content
/// rewritten with no `ARROW:schema` footer entry, a plain (undictionaried)
/// `object_type`, Parquet's default row-group sizing and compression, still
/// conforms.
#[test]
fn a_package_rewritten_by_a_writer_without_arrow_metadata_conforms() {
    let (_dir, pkg) = convert_fixture("lod3_railway.city.json");
    for entry in fs::read_dir(&pkg).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("parquet") {
            FileContent::read(&path).write(&path);
        }
    }
    let report = validate(&pkg);
    assert!(report.violations.is_empty(), "{report}");
}

fn rewrite(path: &Path, f: impl FnOnce(&mut FileContent)) {
    let mut content = FileContent::read(path);
    f(&mut content);
    content.write(path);
}

#[test]
fn a_footer_without_city_version_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        let mut city = c.key_json("city");
        city.as_object_mut().unwrap().remove("version");
        c.set_key_value("city", city.to_string());
    });
    assert_error(&validate(&pkg), "footer.version-missing");
}

#[test]
fn a_file_without_the_city_footer_key_is_reported() {
    let (_dir, pkg) = convert_fixture("lod3_railway.city.json");
    rewrite(&pkg.join("materials.parquet"), |c| {
        c.remove_key_value("city")
    });
    assert_error(&validate(&pkg), "footer.city-missing");
}

#[test]
fn a_json_column_stored_as_plain_utf8_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        c.untag_json(Some("other"))
    });
    let report = validate(&pkg);
    assert_error(&report, "column.json-type");
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "column.json-type" && v.message.contains("`other`")),
        "{report}"
    );
}

/// The declaration rule: a column carrying `PolyhedralSurface Z` MUST NOT be
/// declared in `geo.columns`.
#[test]
fn a_solid_column_declared_in_geo_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        let mut geo = c.key_json("geo");
        let columns = geo["columns"].as_object_mut().unwrap();
        let entry = columns.values().next().unwrap().clone();
        let mut solid = entry.clone();
        solid["geometry_types"] = json!(["PolyhedralSurface Z"]);
        columns.insert("geometry_lod2_2".to_string(), solid);
        c.set_key_value("geo", geo.to_string());
    });
    let report = validate(&pkg);
    assert!(
        report.violations.iter().any(|v| v.code == "geo.declaration"
            && v.severity == Severity::Error
            && v.message.contains("geometry_lod2_2")),
        "{report}"
    );
}

#[test]
fn a_null_feature_id_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        c.map_column("feature_id", |field, column, batch| {
            let values = column.as_string::<i32>();
            let replaced: StringArray = (0..values.len())
                .map(|i| (batch != 0 || i != 0).then(|| values.value(i)))
                .collect();
            (
                field.clone().with_nullable(true),
                Arc::new(replaced) as ArrayRef,
            )
        });
    });
    let report = validate(&pkg);
    assert_error(&report, "value.required-null");
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "column.nullability"
                && v.severity == Severity::Warning
                && v.message.contains("feature_id")),
        "{report}"
    );
}

/// `children_roles`, when present, MUST have exactly one entry per child.
#[test]
fn children_roles_of_the_wrong_length_are_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        let children = c.batches[0]
            .column_by_name("children")
            .unwrap()
            .as_list::<i32>()
            .clone();
        c.map_column("children_roles", |field, column, batch| {
            if batch != 0 {
                return (field.clone(), column.clone());
            }
            // One role more than there are children, on every row with
            // children; null elsewhere.
            let mut offsets = vec![0i32];
            let mut nulls = Vec::new();
            for i in 0..children.len() {
                let n = if children.is_null(i) {
                    0
                } else {
                    children.value_length(i) + 1
                };
                offsets.push(offsets.last().unwrap() + n);
                nulls.push(!children.is_null(i));
            }
            let values = StringArray::from(vec![None::<&str>; *offsets.last().unwrap() as usize]);
            let DataType::List(item) = field.data_type() else {
                panic!("children_roles is a list")
            };
            let list = ListArray::new(
                item.clone(),
                OffsetBuffer::new(offsets.into()),
                Arc::new(values),
                Some(nulls.into()),
            );
            (field.clone(), Arc::new(list) as ArrayRef)
        });
    });
    assert_error(&validate(&pkg), "value.children-roles");
}

#[test]
fn an_object_table_named_after_the_wrong_module_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    fs::rename(
        pkg.join("building.parquet"),
        pkg.join("transportation.parquet"),
    )
    .unwrap();
    let item_path = pkg.join("metadata.json");
    let text = fs::read_to_string(&item_path)
        .unwrap()
        .replace("building.parquet", "transportation.parquet");
    fs::write(&item_path, text).unwrap();
    assert_error(&validate(&pkg), "package.object-type-module");
}

#[test]
fn a_parquet_asset_without_its_package_role_is_reported() {
    let (_dir, pkg) = convert_fixture("lod3_railway.city.json");
    let item_path = pkg.join("metadata.json");
    let mut item: Value = serde_json::from_str(&fs::read_to_string(&item_path).unwrap()).unwrap();
    for asset in item["assets"].as_object_mut().unwrap().values_mut() {
        if asset["href"] == "./textures.parquet" {
            asset["roles"] = json!(["data"]);
        }
    }
    fs::write(&item_path, item.to_string()).unwrap();
    let report = validate(&pkg);
    assert!(
        report.violations.iter().any(|v| v.code == "stac.asset-role"
            && v.severity == Severity::Error
            && v.message.contains("textures.parquet")),
        "{report}"
    );
}

#[test]
fn a_reserved_column_of_the_wrong_logical_type_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        c.map_column("id", |field, column, _| {
            let ids = column.as_string::<i32>();
            let blobs: BinaryArray = ids.iter().map(|v| v.map(str::as_bytes)).collect();
            (
                Field::new(field.name(), DataType::Binary, false),
                Arc::new(blobs) as ArrayRef,
            )
        });
    });
    let report = validate(&pkg);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "column.logical-type"
                && v.severity == Severity::Error
                && v.message.contains("`id`")),
        "{report}"
    );
}

/// `TIMESTAMP` columns MUST be UTC-adjusted.
#[test]
fn a_timestamp_attribute_that_is_not_utc_adjusted_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        c.map_column("tijdstipregistratie", |field, column, _| {
            let DataType::Timestamp(unit, _) = field.data_type() else {
                panic!("tijdstipregistratie is a timestamp")
            };
            let naive = DataType::Timestamp(*unit, None);
            let data = column
                .to_data()
                .into_builder()
                .data_type(naive.clone())
                .build()
                .unwrap();
            (
                Field::new(field.name(), naive, true).with_metadata(field.metadata().clone()),
                make_array(data),
            )
        });
    });
    let report = validate(&pkg);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "attribute.timestamp-utc"
                && v.severity == Severity::Error
                && v.message.contains("tijdstipregistratie")),
        "{report}"
    );
}

/// Geometry is little-endian ISO WKB.
#[test]
fn a_big_endian_geometry_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        c.map_column("geometry_lod2_2", |field, column, batch| {
            let blobs = column.as_binary::<i32>();
            let mut flipped = batch != 0;
            let rewritten: BinaryArray = blobs
                .iter()
                .map(|cell| {
                    cell.map(|bytes| {
                        let mut bytes = bytes.to_vec();
                        if !flipped {
                            bytes[0] = 0x00;
                            flipped = true;
                        }
                        bytes
                    })
                })
                .collect();
            (field.clone(), Arc::new(rewritten) as ArrayRef)
        });
    });
    assert_error(&validate(&pkg), "value.wkb");
}

#[test]
fn a_package_with_no_object_table_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    fs::remove_file(pkg.join("building.parquet")).unwrap();
    assert_error(&validate(&pkg), "package.no-object-table");
}

/// `len(face_semantics)` MUST equal the WKB face count.
#[test]
fn face_semantics_shorter_than_the_face_count_are_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        c.map_column("geometry_properties_lod2_2", |field, column, batch| {
            let props = column.as_struct();
            let semantics = props
                .column_by_name("face_semantics")
                .unwrap()
                .as_list::<i32>();
            let mut shortened = batch != 0;
            let rows: Vec<Option<Vec<Option<i32>>>> = (0..semantics.len())
                .map(|i| {
                    (!semantics.is_null(i)).then(|| {
                        let mut faces: Vec<Option<i32>> = semantics
                            .value(i)
                            .as_primitive::<Int32Type>()
                            .iter()
                            .collect();
                        if !shortened {
                            faces.pop();
                            shortened = true;
                        }
                        faces
                    })
                })
                .collect();
            let replaced = ListArray::from_iter_primitive::<Int32Type, _, _>(rows);
            let DataType::Struct(fields) = field.data_type() else {
                panic!("geometry_properties is a struct")
            };
            let columns: Vec<ArrayRef> = fields
                .iter()
                .zip(props.columns())
                .map(|(f, col)| {
                    if f.name() == "face_semantics" {
                        Arc::new(replaced.clone()) as ArrayRef
                    } else {
                        col.clone()
                    }
                })
                .collect();
            let rebuilt = StructArray::new(fields.clone(), columns, props.nulls().cloned());
            (field.clone(), Arc::new(rebuilt) as ArrayRef)
        });
    });
    assert_error(&validate(&pkg), "value.face-semantics");
}

/// Every non-null material id MUST match an `id` in `materials.parquet`.
#[test]
fn a_material_reference_into_a_missing_sidecar_is_reported() {
    let (_dir, pkg) = convert_fixture("lod3_railway.city.json");
    fs::remove_file(pkg.join("materials.parquet")).unwrap();
    let item_path = pkg.join("metadata.json");
    let mut item: Value = serde_json::from_str(&fs::read_to_string(&item_path).unwrap()).unwrap();
    item["assets"]
        .as_object_mut()
        .unwrap()
        .retain(|_, asset| asset["href"] != "./materials.parquet");
    fs::write(&item_path, item.to_string()).unwrap();
    assert_error(&validate(&pkg), "value.material");
}

/// `data_type` with every `Int32`/`Int64` leaf named `leaf` (or every one,
/// when `leaf` is `None`) turned into `Utf8` — the shape of a writer that
/// stored the integers as text.
fn ints_as_text(data_type: &DataType, leaf: Option<&str>, here: &str) -> DataType {
    use arrow_schema::Fields;
    let field = |f: &Field| {
        let name = f.name().as_str();
        Field::new(
            name,
            ints_as_text(f.data_type(), leaf, name),
            f.is_nullable(),
        )
    };
    match data_type {
        DataType::Int32 | DataType::Int64 if leaf.is_none_or(|l| l == here) => DataType::Utf8,
        DataType::List(item) => {
            let inner = if leaf == Some(here) { None } else { leaf };
            DataType::List(Arc::new(Field::new(
                item.name(),
                ints_as_text(item.data_type(), inner, item.name()),
                item.is_nullable(),
            )))
        }
        DataType::Struct(fields) => DataType::Struct(Fields::from(
            fields.iter().map(|f| field(f)).collect::<Vec<_>>(),
        )),
        DataType::Map(entries, sorted) => DataType::Map(Arc::new(field(entries)), *sorted),
        other => other.clone(),
    }
}

/// A package whose nested integer lists hold text instead is non-conformant,
/// and the validator says so for every column family — it never panics on
/// the values after the schema check has already rejected their type.
#[test]
fn nested_columns_of_the_wrong_type_are_reported_not_a_panic() {
    for (fixture, column, leaf) in [
        (
            "delft.city.jsonl",
            "geometry_properties_lod2_2",
            "face_semantics",
        ),
        ("delft.city.jsonl", "geometry_properties_lod2_2", "shells"),
        ("lod3_railway.city.json", "material_lod3_0", "value"),
        ("lod3_railway.city.json", "texture_lod3_0", "id"),
    ] {
        let (_dir, pkg) = convert_fixture(fixture);
        rewrite(&pkg.join("building.parquet"), |c| {
            let data_type = c
                .schema
                .field_with_name(column)
                .unwrap()
                .data_type()
                .clone();
            c.retype(column, &ints_as_text(&data_type, Some(leaf), column));
        });
        let report = validate(&pkg);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.code == "column.logical-type"
                    && v.severity == Severity::Error
                    && v.message.contains(column)),
            "{column}.{leaf}: {report}"
        );
    }
}

/// Parquet's backward-compatibility rules admit a two-level `LIST` — a
/// `LIST`-annotated group whose single repeated child is the element itself
/// — and the spec types a column by its logical type, not by that layout.
/// Delft's `parents` rewritten that way (elements required, which its values
/// allow) still conforms.
#[test]
fn a_two_level_list_column_conforms() {
    use parquet::basic::{LogicalType, Repetition, Type as PhysicalType};
    use parquet::schema::types::Type;

    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    let table = pkg.join("building.parquet");
    let mut content = FileContent::read(&table);
    content.map_column("parents", |field, column, _| {
        let item = Arc::new(Field::new("element", DataType::Utf8, false));
        let data_type = DataType::List(item);
        let data = column
            .to_data()
            .into_builder()
            .data_type(data_type.clone())
            .build()
            .unwrap();
        (Field::new(field.name(), data_type, true), make_array(data))
    });
    content.write_with_schema(&table, |fields| {
        let element = Type::primitive_type_builder("element", PhysicalType::BYTE_ARRAY)
            .with_repetition(Repetition::REPEATED)
            .with_logical_type(Some(LogicalType::String))
            .build()
            .unwrap();
        let parents = Type::group_type_builder("parents")
            .with_repetition(Repetition::OPTIONAL)
            .with_logical_type(Some(LogicalType::List))
            .with_fields(vec![Arc::new(element)])
            .build()
            .unwrap();
        let at = fields.iter().position(|f| f.name() == "parents").unwrap();
        fields[at] = Arc::new(parents);
    });

    // The file really holds the two-level form.
    let reader =
        parquet::file::reader::SerializedFileReader::new(fs::File::open(&table).unwrap()).unwrap();
    use parquet::file::reader::FileReader;
    let descr = reader.metadata().file_metadata().schema_descr_ptr();
    assert!(
        (0..descr.num_columns()).any(|i| descr.column(i).path().string() == "parents.element"),
        "parents is written as a two-level list"
    );

    let report = validate(&pkg);
    assert!(report.violations.is_empty(), "{report}");
}

/// Rebuild the struct column `name` with its child `child` replaced by
/// `f(child array)`.
fn replace_struct_child(
    c: &mut FileContent,
    name: &str,
    child: &str,
    mut f: impl FnMut(&ArrayRef) -> ArrayRef,
) {
    c.map_column(name, |field, column, _| {
        let s = column.as_struct();
        let DataType::Struct(fields) = field.data_type() else {
            panic!("{name} is a struct")
        };
        let columns: Vec<ArrayRef> = fields
            .iter()
            .zip(s.columns())
            .map(|(f_, col)| {
                if f_.name() == child {
                    f(col)
                } else {
                    col.clone()
                }
            })
            .collect();
        let rebuilt = StructArray::new(fields.clone(), columns, s.nulls().cloned());
        (field.clone(), Arc::new(rebuilt) as ArrayRef)
    });
}

/// An implicit geometry's reference point is one WKB PointZ, with nothing
/// after it.
#[test]
fn an_implicit_reference_point_with_trailing_bytes_is_reported() {
    let (_dir, pkg) = convert_fixture("lod3_railway.city.json");
    let table = fs::read_dir(&pkg)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.extension().is_some_and(|e| e == "parquet")
                && FileContent::read(p).batches.iter().any(|b| {
                    b.column_by_name("implicit_geometry")
                        .is_some_and(|c| c.null_count() < c.len())
                })
        })
        .expect("railway has an object with an implicit geometry");
    rewrite(&table, |c| {
        replace_struct_child(c, "implicit_geometry", "point", |points| {
            let points = points.as_binary::<i32>();
            let padded: BinaryArray = points
                .iter()
                .map(|p| {
                    p.map(|bytes| {
                        let mut bytes = bytes.to_vec();
                        bytes.push(0);
                        bytes
                    })
                })
                .collect();
            Arc::new(padded) as ArrayRef
        });
    });
    assert_error(&validate(&pkg), "value.implicit-geometry");
}

/// spec 02 "The `other` column": an `other` entry MUST NOT duplicate an
/// attribute the same row carries in a column.
#[test]
fn an_other_entry_duplicating_an_attribute_column_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        let status: Vec<bool> = c
            .batches
            .iter()
            .flat_map(|b| {
                let col = b.column_by_name("status").unwrap();
                (0..col.len()).map(|i| !col.is_null(i)).collect::<Vec<_>>()
            })
            .collect();
        let target = status.iter().position(|&s| s).expect("a row with a status");
        let mut offset = 0;
        c.map_column("other", |field, column, _| {
            let values = column.as_string::<i32>();
            let replaced: StringArray = (0..values.len())
                .map(|i| {
                    if offset + i == target {
                        Some(r#"{"status":"duplicate"}"#.to_string())
                    } else {
                        (!values.is_null(i)).then(|| values.value(i).to_string())
                    }
                })
                .collect();
            offset += values.len();
            (field.clone(), Arc::new(replaced) as ArrayRef)
        });
    });
    assert_error(&validate(&pkg), "value.other-collision");
}

/// Set `bbox.<field>` of the rows `pick` selects (by object type) to
/// `value(that row's bbox)`.
fn edit_bbox(
    c: &mut FileContent,
    field: &str,
    pick: impl Fn(&str) -> bool,
    value: impl Fn([f64; 6]) -> f64,
) {
    use arrow_array::Float64Array;
    use arrow_array::types::Float64Type;
    const NAMES: [&str; 6] = ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"];
    let picked: Vec<Vec<bool>> = c
        .batches
        .iter()
        .map(|b| {
            let types = b.column_by_name("object_type").unwrap();
            let types = arrow_cast::cast(types, &DataType::Utf8).unwrap();
            let types = types.as_string::<i32>();
            (0..types.len()).map(|i| pick(types.value(i))).collect()
        })
        .collect();
    let at = NAMES.iter().position(|n| *n == field).unwrap();
    c.map_column("bbox", |f_, column, batch| {
        let s = column.as_struct();
        let DataType::Struct(fields) = f_.data_type() else {
            panic!("bbox is a struct")
        };
        let leaf = |n: &str| {
            s.column_by_name(n)
                .unwrap()
                .as_primitive::<Float64Type>()
                .clone()
        };
        let leaves: Vec<_> = NAMES.iter().map(|n| leaf(n)).collect();
        let edited: Float64Array = (0..s.len())
            .map(|i| {
                let current = leaves[at].value(i);
                if s.is_null(i) || !picked[batch][i] {
                    return current;
                }
                let bbox: [f64; 6] = std::array::from_fn(|k| leaves[k].value(i));
                value(bbox)
            })
            .map(Some)
            .collect();
        let columns: Vec<ArrayRef> = fields
            .iter()
            .zip(s.columns())
            .map(|(f, col)| {
                if f.name() == field {
                    Arc::new(edited.clone()) as ArrayRef
                } else {
                    col.clone()
                }
            })
            .collect();
        let rebuilt = StructArray::new(fields.clone(), columns, s.nulls().cloned());
        (f_.clone(), Arc::new(rebuilt) as ArrayRef)
    });
}

/// spec 03 "Spatial metadata": `bbox` is a superset of everything the row
/// stores. A BuildingPart box narrowed to zero width leaves its solids
/// outside it.
#[test]
fn a_bbox_not_containing_its_rows_geometry_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        edit_bbox(c, "xmax", |t| t == "BuildingPart", |b| b[0]);
    });
    assert_error(&validate(&pkg), "value.bbox");
}

/// spec 03 "Spatial metadata": `bbox` is the union over the object's whole
/// subtree. A Building box cut down to its lowest point no longer contains
/// its BuildingParts' boxes.
#[test]
fn a_bbox_not_containing_its_childrens_boxes_is_reported() {
    let (_dir, pkg) = convert_fixture("delft.city.jsonl");
    rewrite(&pkg.join("building.parquet"), |c| {
        edit_bbox(c, "zmax", |t| t == "Building", |b| b[2]);
    });
    assert_error(&validate(&pkg), "value.bbox-subtree");
}
