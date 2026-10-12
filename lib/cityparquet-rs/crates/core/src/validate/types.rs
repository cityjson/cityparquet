//! Parquet logical types: matching a file's Parquet schema against the types
//! the spec states, as `cityparquet-schema` renders them.

use super::*;

// ---------------------------------------------------------------------------
// Parquet logical types
// ---------------------------------------------------------------------------

/// A primitive Parquet column's logical type, as far as CityParquet's
/// vocabulary distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Leaf {
    Varchar,
    Json,
    Blob,
    Int,
    Bigint,
    Double,
    Boolean,
}

pub(super) fn leaf_of(t: &Type) -> Option<Leaf> {
    if t.is_group() {
        return None;
    }
    let info = t.get_basic_info();
    let logical = info.logical_type_ref();
    let converted = info.converted_type();
    match t.get_physical_type() {
        PhysicalType::BYTE_ARRAY => match (logical, converted) {
            (Some(LogicalType::String), _) | (None, ConvertedType::UTF8) => Some(Leaf::Varchar),
            (Some(LogicalType::Json), _) | (None, ConvertedType::JSON) => Some(Leaf::Json),
            (None, ConvertedType::NONE) => Some(Leaf::Blob),
            _ => None,
        },
        PhysicalType::INT32 => match (logical, converted) {
            (
                None
                | Some(LogicalType::Integer {
                    bit_width: 32,
                    is_signed: true,
                }),
                ConvertedType::NONE | ConvertedType::INT_32,
            ) => Some(Leaf::Int),
            _ => None,
        },
        PhysicalType::INT64 => match (logical, converted) {
            (
                None
                | Some(LogicalType::Integer {
                    bit_width: 64,
                    is_signed: true,
                }),
                ConvertedType::NONE | ConvertedType::INT_64,
            ) => Some(Leaf::Bigint),
            _ => None,
        },
        PhysicalType::DOUBLE if logical.is_none() => Some(Leaf::Double),
        PhysicalType::BOOLEAN if logical.is_none() => Some(Leaf::Boolean),
        _ => None,
    }
}

pub(super) fn is_list(t: &Type) -> bool {
    let info = t.get_basic_info();
    matches!(info.logical_type_ref(), Some(LogicalType::List))
        || info.converted_type() == ConvertedType::LIST
}

pub(super) fn is_map(t: &Type) -> bool {
    let info = t.get_basic_info();
    matches!(info.logical_type_ref(), Some(LogicalType::Map))
        || matches!(
            info.converted_type(),
            ConvertedType::MAP | ConvertedType::MAP_KEY_VALUE
        )
}

/// The element type of a `LIST` column, in any layout Parquet's
/// backward-compatibility rules admit (`LogicalTypes.md`, "Lists"): a reader
/// MUST NOT require a particular child name (spec 02), and the two-level
/// forms are the same logical type.
///
/// - a repeated field with no `LIST`/`MAP` annotation is a required list of
///   required elements of its own type;
/// - in a `LIST`-annotated group, a repeated primitive is the element; so is
///   a repeated group with several fields, or with one field when it is named
///   `array` or `<list>_tuple`; any other repeated group wraps the element as
///   its single child (the standard three-level form).
pub(super) fn list_element(t: &Type) -> Option<&Type> {
    let info = t.get_basic_info();
    if info.has_repetition()
        && info.repetition() == Repetition::REPEATED
        && !is_list(t)
        && !is_map(t)
    {
        return Some(t);
    }
    if !t.is_group() || !is_list(t) {
        return None;
    }
    let [repeated] = t.get_fields() else {
        return None;
    };
    if repeated.get_basic_info().repetition() != Repetition::REPEATED {
        return None;
    }
    if !repeated.is_group() {
        return Some(repeated);
    }
    match repeated.get_fields() {
        [element]
            if repeated.name() != "array" && repeated.name() != format!("{}_tuple", t.name()) =>
        {
            Some(element)
        }
        [] => None,
        _ => Some(repeated),
    }
}

/// The key and value of a `MAP` group.
pub(super) fn map_entries(t: &Type) -> Option<(&Type, &Type)> {
    if !t.is_group() || !is_map(t) {
        return None;
    }
    let [repeated] = t.get_fields() else {
        return None;
    };
    if !repeated.is_group() || repeated.get_basic_info().repetition() != Repetition::REPEATED {
        return None;
    }
    match repeated.get_fields() {
        [key, value] => Some((key, value)),
        _ => None,
    }
}

pub(super) fn is_optional(t: &Type) -> bool {
    let info = t.get_basic_info();
    info.has_repetition() && info.repetition() == Repetition::OPTIONAL
}

/// How a Parquet type reads in a message.
pub(super) fn describe(t: &Type) -> String {
    let info = t.get_basic_info();
    let base = if t.is_group() {
        "a group".to_string()
    } else {
        t.get_physical_type().to_string()
    };
    match (info.logical_type_ref(), info.converted_type()) {
        (Some(logical), _) => format!("{base} ({logical:?})"),
        (None, ConvertedType::NONE) => base,
        (None, converted) => format!("{base} ({converted})"),
    }
}

pub(super) fn is_json_field(field: &Field) -> bool {
    field.extension_type_name() == Some("arrow.json")
}

/// The spec's spelling of an expected type.
pub(super) fn spec_type(field: &Field) -> String {
    match field.data_type() {
        DataType::Utf8 | DataType::Dictionary(..) if is_json_field(field) => "JSON".to_string(),
        DataType::Utf8 | DataType::Dictionary(..) => "VARCHAR".to_string(),
        DataType::Binary => "BLOB".to_string(),
        DataType::Int32 => "INT".to_string(),
        DataType::Int64 => "BIGINT".to_string(),
        DataType::Float64 => "DOUBLE".to_string(),
        DataType::Boolean => "BOOLEAN".to_string(),
        DataType::List(item) => format!("LIST<{}>", spec_type(item)),
        DataType::Map(entries, _) => match entries.data_type() {
            DataType::Struct(kv) => format!("MAP<{}, {}>", spec_type(&kv[0]), spec_type(&kv[1])),
            other => format!("{other}"),
        },
        DataType::Struct(fields) => format!(
            "STRUCT<{}>",
            fields
                .iter()
                .map(|f| format!("{} {}", f.name(), spec_type(f)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other}"),
    }
}

/// Check that the Parquet column `actual` has the logical type and
/// nullability of the spec's `expected` (rendered by `cityparquet-schema`).
pub(super) fn check_type(
    r: &mut Reporter,
    file: &str,
    spec: &'static str,
    path: &str,
    expected: &Field,
    actual: &Type,
) {
    // A non-null column or field declared nullable: the value-level rule
    // may still hold, so the declaration alone is a warning.
    if !expected.is_nullable() && is_optional(actual) {
        r.warn(
            "column.nullability",
            spec,
            Some(file),
            format!("`{path}` is declared nullable; the spec makes it non-null"),
        );
    }
    let mismatch = |r: &mut Reporter| {
        r.error(
            "column.logical-type",
            spec,
            Some(file),
            format!(
                "`{path}` is {}; the spec types it {}",
                describe(actual),
                spec_type(expected)
            ),
        );
    };
    let want_leaf = match expected.data_type() {
        DataType::Utf8 | DataType::Dictionary(..) if is_json_field(expected) => Some(Leaf::Json),
        DataType::Utf8 | DataType::Dictionary(..) => Some(Leaf::Varchar),
        DataType::Binary => Some(Leaf::Blob),
        DataType::Int32 => Some(Leaf::Int),
        DataType::Int64 => Some(Leaf::Bigint),
        DataType::Float64 => Some(Leaf::Double),
        DataType::Boolean => Some(Leaf::Boolean),
        _ => None,
    };
    if let Some(want) = want_leaf {
        match leaf_of(actual) {
            Some(got) if got == want => {}
            // spec 02/03/04: the column is typed `JSON`, the Parquet JSON
            // logical type, not merely a string holding JSON text.
            Some(Leaf::Varchar) if want == Leaf::Json => r.error(
                "column.json-type",
                spec,
                Some(file),
                format!("`{path}` is a STRING column; the spec types it JSON (the Parquet JSON logical type)"),
            ),
            _ => mismatch(r),
        }
        return;
    }
    match expected.data_type() {
        DataType::List(item) => match list_element(actual) {
            Some(element) => check_type(r, file, spec, &format!("{path}[]"), item, element),
            None => mismatch(r),
        },
        DataType::Map(entries, _) => {
            let (Some((key, value)), DataType::Struct(kv)) =
                (map_entries(actual), entries.data_type())
            else {
                mismatch(r);
                return;
            };
            check_type(r, file, spec, &format!("{path}.key"), &kv[0], key);
            check_type(r, file, spec, &format!("{path}.value"), &kv[1], value);
        }
        DataType::Struct(fields) => {
            if !actual.is_group() || is_list(actual) || is_map(actual) {
                mismatch(r);
                return;
            }
            for field in fields {
                match actual
                    .get_fields()
                    .iter()
                    .find(|c| c.name() == field.name())
                {
                    Some(child) => check_type(
                        r,
                        file,
                        spec,
                        &format!("{path}.{}", field.name()),
                        field,
                        child,
                    ),
                    None => r.error(
                        "column.logical-type",
                        spec,
                        Some(file),
                        format!("`{path}` has no field `{}`", field.name()),
                    ),
                }
            }
            for child in actual.get_fields() {
                if !fields.iter().any(|f| f.name() == child.name()) {
                    r.warn(
                        "column.unknown-field",
                        spec,
                        Some(file),
                        format!(
                            "`{path}` has a field `{}` the spec does not define",
                            child.name()
                        ),
                    );
                }
            }
        }
        _ => mismatch(r),
    }
}

/// A `geometry_lod*` column's Parquet annotation: `None` when plain
/// `BYTE_ARRAY`, `Some(crs)` when it carries the `GEOMETRY` logical type.
/// Reports anything else.
pub(super) fn geometry_annotation(
    r: &mut Reporter,
    file: &str,
    name: &str,
    actual: &Type,
) -> Option<Option<String>> {
    if actual.is_group() || actual.get_physical_type() != PhysicalType::BYTE_ARRAY {
        r.error(
            "column.logical-type",
            GEOMETRY,
            Some(file),
            format!(
                "`{name}` is {}; the spec types it BLOB (WKB)",
                describe(actual)
            ),
        );
        return None;
    }
    let info = actual.get_basic_info();
    match info.logical_type_ref() {
        Some(LogicalType::Geometry { crs }) => Some(crs.clone()),
        // spec 03/05: CityParquet geometry has planar edges; `GEOGRAPHY`
        // declares geodesic ones and is not the `GEOMETRY` type the
        // declaration rule allows.
        Some(LogicalType::Geography { .. }) => {
            r.error(
                "geo.geography",
                METADATA,
                Some(file),
                format!("`{name}` carries the GEOGRAPHY logical type; only GEOMETRY is allowed"),
            );
            None
        }
        None if info.converted_type() == ConvertedType::NONE => None,
        _ => {
            r.error(
                "column.logical-type",
                GEOMETRY,
                Some(file),
                format!(
                    "`{name}` is {}; the spec types it BLOB (WKB)",
                    describe(actual)
                ),
            );
            None
        }
    }
}
