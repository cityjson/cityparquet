//! Reserved column names and the escape that keeps a colliding source
//! attribute in a column of its own (spec "Column naming and reservation
//! rules").
//!
//! A name is reserved when it matches, under ASCII case folding, one of the
//! fixed reserved columns (`other` among them) or a per-LoD form the LoD
//! suffix grammar generates (`geometry_lod2_2`, `material_lod0_0`, …). The
//! match is on the grammar, not on a dataset's LoDs: `Geometry_LoD2_2`
//! collides whether or not the dataset carries LoD 2.2.
//!
//! The *base* of a name is the name with every leading `_` removed. A writer
//! stores a source attribute whose base is reserved as `_` + its source name,
//! and a reader removes one `_` from an attribute column whose base is
//! reserved, so the two are exact inverses.

use crate::error::{CityParquetError, Result};
use crate::types::Lod;

/// The fixed reserved column names (spec "Reserved columns").
pub const FIXED_RESERVED_COLUMNS: &[&str] = &[
    "id",
    "feature_id",
    "object_type",
    "parents",
    "children",
    "children_roles",
    "address",
    "bbox",
    "implicit_geometry",
    "other",
];

/// The per-LoD column prefixes, longest first so `geometry_properties_…` is
/// never read as `geometry_…`.
const LOD_PREFIXES: &[&str] = &["geometry_properties", "geometry", "material", "texture"];

/// Whether `name` collides with a reserved column name, compared
/// case-insensitively (ASCII case folding). A per-LoD form collides only when
/// its suffix is one the grammar generates — `lod<major>_<minor>` with both
/// parts in canonical decimal — so `geometry_lod3` (no minor) and
/// `material_lod03` do not.
pub fn is_reserved_column_name(name: &str) -> bool {
    let folded = name.to_ascii_lowercase();
    if FIXED_RESERVED_COLUMNS.contains(&folded.as_str()) {
        return true;
    }
    LOD_PREFIXES.iter().any(|prefix| {
        folded
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(|suffix| {
                Lod::from_column_suffix(suffix).is_some_and(|lod| lod.column_suffix() == suffix)
            })
    })
}

/// The name with every leading `_` removed.
pub fn base_name(name: &str) -> &str {
    name.trim_start_matches('_')
}

/// The column a source attribute is stored under: `_` + the source name when
/// its base collides with a reserved name (`ID` → `_ID`, `_ID` → `__ID`),
/// the source name unchanged otherwise.
pub fn escape_attribute_name(source: &str) -> String {
    if is_reserved_column_name(base_name(source)) {
        format!("_{source}")
    } else {
        source.to_string()
    }
}

/// The source name of an attribute column: the column name with one leading
/// `_` removed when its base collides with a reserved name (`_ID` → `ID`,
/// `__ID` → `_ID`), the column name unchanged otherwise.
///
/// An attribute column whose own name collides with a reserved name cannot
/// come from a conforming writer and is invalid input.
pub fn restore_attribute_name(column: &str) -> Result<&str> {
    if !is_reserved_column_name(base_name(column)) {
        return Ok(column);
    }
    column.strip_prefix('_').ok_or_else(|| {
        CityParquetError::Schema(format!(
            "attribute column '{column}' collides with a reserved column name; a conforming \
             writer stores such an attribute as '_{column}'"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_names_collide_case_insensitively() {
        for name in [
            "id", "ID", "Id", "bbox", "Bbox", "CHILDREN", "other", "OTHER",
        ] {
            assert!(is_reserved_column_name(name), "{name} must be reserved");
        }
        for name in ["ident", "_id", "identificatie", "others", "geometry"] {
            assert!(
                !is_reserved_column_name(name),
                "{name} must not be reserved"
            );
        }
    }

    #[test]
    fn per_lod_forms_collide_by_the_suffix_grammar() {
        for name in [
            "geometry_lod2_2",
            "Geometry_LoD2_2",
            "geometry_properties_lod0_0",
            "MATERIAL_LOD3_0",
            "texture_lod1_3",
            "geometry_lod9_9",
        ] {
            assert!(is_reserved_column_name(name), "{name} must be reserved");
        }
        for name in [
            "geometry_lod3",
            "material_lod03",
            "geometry_lod02_2",
            "geometry_vertices_lod2_2",
            "geometry_lod2_2_extra",
            "material",
            "texture",
            "geometry_properties",
        ] {
            assert!(
                !is_reserved_column_name(name),
                "{name} must not be reserved"
            );
        }
    }

    #[test]
    fn escape_prefixes_one_underscore_when_the_base_collides() {
        assert_eq!(escape_attribute_name("ID"), "_ID");
        assert_eq!(escape_attribute_name("children"), "_children");
        assert_eq!(escape_attribute_name("_ID"), "__ID");
        assert_eq!(escape_attribute_name("other"), "_other");
        assert_eq!(escape_attribute_name("Geometry_LoD2_2"), "_Geometry_LoD2_2");
        assert_eq!(escape_attribute_name("measuredHeight"), "measuredHeight");
        assert_eq!(escape_attribute_name("_private"), "_private");
    }

    #[test]
    fn restore_removes_one_underscore_when_the_base_collides() {
        assert_eq!(restore_attribute_name("_ID").unwrap(), "ID");
        assert_eq!(restore_attribute_name("__ID").unwrap(), "_ID");
        assert_eq!(restore_attribute_name("_other").unwrap(), "other");
        assert_eq!(restore_attribute_name("_private").unwrap(), "_private");
        assert_eq!(
            restore_attribute_name("measuredHeight").unwrap(),
            "measuredHeight"
        );
    }

    #[test]
    fn a_column_whose_own_name_collides_cannot_be_restored() {
        for column in ["ID", "bbox", "geometry_lod2_2", "other"] {
            assert!(
                restore_attribute_name(column).is_err(),
                "{column} cannot come from a conforming writer"
            );
        }
    }

    #[test]
    fn escape_and_restore_are_inverses() {
        for source in [
            "ID",
            "_ID",
            "__ID",
            "children",
            "other",
            "_other",
            "geometry_lod0_0",
            "measuredHeight",
            "_private",
            "",
            "_",
        ] {
            let column = escape_attribute_name(source);
            assert!(
                !is_reserved_column_name(&column),
                "{column} is a legal column"
            );
            assert_eq!(restore_attribute_name(&column).unwrap(), source);
        }
    }
}
