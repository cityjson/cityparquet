//! Which physical [`GeometryEncoding`] a `geometry_lod*` column actually
//! carries — resolved from the file's OWN footer declaration
//! (`city.columns[].encoding`, spec `05-metadata.mdx`), then checked against
//! the physical column. Shared by [`crate::reader`] (schema rendering) and
//! [`crate::decode`] (row decoding) so the two can never disagree about what
//! a given file is.
//!
//! **Declaration first, shape second.** An earlier version of both call sites
//! inferred the encoding purely structurally — a `Binary` column was WKB, any
//! outer `List` column was arrow-native v1 — which is wrong in three distinct
//! ways:
//!
//! 1. a footer CONTRADICTING its own physical columns was silently ignored,
//!    so a mis-tagged or corrupt file decoded as whatever it happened to
//!    carry instead of being rejected;
//! 2. a FUTURE list-based encoding (a `CityParquetArrowNative-v2`, or another
//!    writer's nested layout entirely) would be decoded as though it were
//!    this crate's exact v1 shape, silently producing wrong geometry;
//! 3. only the OUTER type was ever checked, so [`crate::decode::decode_batch`]
//!    could walk into `arrow_geom_read`'s vertex-struct field indexing on a
//!    struct that need not have those three fields at all.
//!
//! So: the footer's declared token decides the encoding, an unknown token is
//! a hard error, and the physical geometry/vertex-pool `Field`s must then
//! match [`arrow_native_geometry_data_type`]/[`arrow_native_vertices_data_type`]
//! (or `Binary`, for WKB) exactly before anything decodes.

use arrow_schema::{DataType, Schema};

use cityparquet_schema::model::{arrow_native_geometry_data_type, arrow_native_vertices_data_type};
use cityparquet_schema::{CityMetadata, CityParquetError, GeometryEncoding, Result};

fn metadata_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Metadata(msg.into())
}

/// The encoding `meta.columns` declares for `geometry_name`, or `None` when
/// this footer declares nothing for that column at all (see
/// [`encoding_from_physical_shape`] for what happens then). A token this
/// build does not understand is an error, never a silent fall back to
/// shape-guessing — that fall back is exactly how a future `…-v2` list
/// encoding would be misread as v1.
fn declared_encoding(meta: &CityMetadata, geometry_name: &str) -> Result<Option<GeometryEncoding>> {
    let Some(entry) = meta.columns.iter().find(|c| c.name == geometry_name) else {
        return Ok(None);
    };
    match GeometryEncoding::from_footer_token(&entry.encoding) {
        Some(encoding) => Ok(Some(encoding)),
        None => Err(metadata_err(format!(
            "geometry column '{geometry_name}' declares city.columns[].encoding \"{}\", which \
             this build does not understand (known encodings: {}) — refusing to guess it from \
             the column's physical arrow type",
            entry.encoding,
            GeometryEncoding::KNOWN_FOOTER_TOKENS.join(", "),
        ))),
    }
}

/// The fall back for a file whose footer declares no `city.columns` entry for
/// this column — a legacy or foreign writer, or one of this crate's own
/// geometry-less/legacy bare-`geometry` shapes. Deliberately STRICTER than
/// the structural dispatch it replaces: `Binary` is WKB, and a nested `List`
/// is accepted only when it matches TODAY'S arrow-native shape exactly, so a
/// list column that is anything else is rejected rather than misread.
fn encoding_from_physical_shape(
    geometry_name: &str,
    data_type: &DataType,
) -> Result<GeometryEncoding> {
    if data_type == &DataType::Binary {
        return Ok(GeometryEncoding::Wkb);
    }
    if data_type == &arrow_native_geometry_data_type() {
        return Ok(GeometryEncoding::ArrowNative);
    }
    Err(metadata_err(format!(
        "geometry column '{geometry_name}' has no city.columns[] entry declaring its encoding, \
         and its arrow type matches neither encoding this build renders: {data_type:?} \
         (expected Binary for \"{}\", or the nested List shape for \"{}\")",
        GeometryEncoding::WKB_TOKEN,
        GeometryEncoding::ARROW_NATIVE_V1_TOKEN,
    )))
}

/// Whether `schema`'s physical columns really are what `encoding` says they
/// are. Under [`GeometryEncoding::Wkb`] that is the unchanged `Binary`
/// requirement; under [`GeometryEncoding::ArrowNative`] both the geometry
/// column and its `geometry_vertices_*` sibling must match this build's exact
/// declared types, so a malformed or future-version arrow-native-SHAPED
/// column is refused cleanly here rather than half-decoded downstream.
fn verify_physical_shape(
    schema: &Schema,
    geometry_name: &str,
    vertices_name: &str,
    encoding: GeometryEncoding,
) -> Result<()> {
    let field = geometry_field(schema, geometry_name)?;
    match encoding {
        GeometryEncoding::Wkb => {
            if field.data_type() != &DataType::Binary {
                return Err(metadata_err(format!(
                    "geometry column '{geometry_name}' is declared as \"{}\" but its arrow type \
                     is {:?}, not the Binary a WKB column must be",
                    encoding.footer_token(),
                    field.data_type(),
                )));
            }
        }
        GeometryEncoding::ArrowNative => {
            if field.data_type() != &arrow_native_geometry_data_type() {
                return Err(metadata_err(format!(
                    "geometry column '{geometry_name}' is declared as \"{}\" but its arrow type \
                     {:?} does not match this build's arrow-native geometry type",
                    encoding.footer_token(),
                    field.data_type(),
                )));
            }
            let vertices = schema.field_with_name(vertices_name).map_err(|_| {
                metadata_err(format!(
                    "geometry column '{geometry_name}' is declared as \"{}\" but its \
                     '{vertices_name}' vertex-pool sibling is absent from the file's schema",
                    encoding.footer_token(),
                ))
            })?;
            if vertices.data_type() != &arrow_native_vertices_data_type() {
                return Err(metadata_err(format!(
                    "vertex-pool column '{vertices_name}' has arrow type {:?}, which does not \
                     match this build's arrow-native vertex-pool type",
                    vertices.data_type(),
                )));
            }
        }
    }
    Ok(())
}

fn geometry_field<'a>(schema: &'a Schema, geometry_name: &str) -> Result<&'a arrow_schema::Field> {
    schema.field_with_name(geometry_name).map_err(|_| {
        metadata_err(format!(
            "geometry column '{geometry_name}' is expected by this file's metadata but absent \
             from its actual schema"
        ))
    })
}

/// Resolve one geometry column's [`GeometryEncoding`]: the footer's own
/// `city.columns[].encoding` declaration decides it (falling back to a strict
/// physical-shape match only when the footer declares nothing for this
/// column), and the physical geometry/vertex-pool columns must then match
/// that encoding exactly. `vertices_name` is the `geometry_vertices_*`
/// sibling's name under the same LoD-suffix grammar — consulted only under
/// [`GeometryEncoding::ArrowNative`], so a WKB file (which has no such
/// column) is completely unaffected.
pub(crate) fn resolve_geometry_encoding(
    meta: &CityMetadata,
    schema: &Schema,
    geometry_name: &str,
    vertices_name: &str,
) -> Result<GeometryEncoding> {
    let field = geometry_field(schema, geometry_name)?;
    let encoding = match declared_encoding(meta, geometry_name)? {
        Some(declared) => declared,
        None => encoding_from_physical_shape(geometry_name, field.data_type())?,
    };
    verify_physical_shape(schema, geometry_name, vertices_name, encoding)?;
    Ok(encoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::Field;
    use cityparquet_schema::{CITYPARQUET_VERSION, CityColumnEntry};

    fn empty_meta() -> CityMetadata {
        CityMetadata {
            version: CITYPARQUET_VERSION.to_string(),
            source_format: None,
            source_version: None,
            crs: None,
            primary_column: None,
            columns: Vec::new(),
            attributes: Vec::new(),
            extensions: None,
            appearance_defaults: None,
            other: None,
        }
    }

    fn meta_declaring(name: &str, token: &str) -> CityMetadata {
        let mut meta = empty_meta();
        let mut entry = CityColumnEntry::new(name, vec![], GeometryEncoding::Wkb);
        entry.encoding = token.to_string();
        meta.columns.push(entry);
        meta
    }

    fn wkb_schema() -> Schema {
        Schema::new(vec![Field::new("geometry_lod2_2", DataType::Binary, true)])
    }

    fn arrow_native_schema() -> Schema {
        Schema::new(vec![
            Field::new("geometry_lod2_2", arrow_native_geometry_data_type(), true),
            Field::new(
                "geometry_vertices_lod2_2",
                arrow_native_vertices_data_type(),
                true,
            ),
        ])
    }

    #[test]
    fn a_declared_encoding_matching_its_columns_resolves() {
        for (token, schema, expected) in [
            (
                GeometryEncoding::WKB_TOKEN,
                wkb_schema(),
                GeometryEncoding::Wkb,
            ),
            (
                GeometryEncoding::ARROW_NATIVE_V1_TOKEN,
                arrow_native_schema(),
                GeometryEncoding::ArrowNative,
            ),
        ] {
            let meta = meta_declaring("geometry_lod2_2", token);
            let resolved = resolve_geometry_encoding(
                &meta,
                &schema,
                "geometry_lod2_2",
                "geometry_vertices_lod2_2",
            )
            .unwrap();
            assert_eq!(resolved, expected);
        }
    }

    /// An arrow-native-SHAPED outer `List` that is not this build's exact
    /// shape (here: `List<Int32>`, one level deep) must be refused, not
    /// treated as v1 — the "future list encoding" failure mode.
    #[test]
    fn an_undeclared_list_column_that_is_not_todays_shape_is_refused() {
        let schema = Schema::new(vec![Field::new(
            "geometry_lod2_2",
            DataType::List(Field::new("item", DataType::Int32, false).into()),
            true,
        )]);
        let err = resolve_geometry_encoding(
            &empty_meta(),
            &schema,
            "geometry_lod2_2",
            "geometry_vertices_lod2_2",
        )
        .expect_err("a list column of an unknown shape must not be assumed to be v1");
        assert!(err.to_string().contains("matches neither encoding"));
    }

    /// A declared arrow-native column whose vertex-pool sibling is missing
    /// must error before any decode indexes into it (`arrow_geom_read`
    /// reads vertex-struct fields 0/1/2 unconditionally).
    #[test]
    fn a_declared_arrow_native_column_without_its_vertex_pool_is_refused() {
        let schema = Schema::new(vec![Field::new(
            "geometry_lod2_2",
            arrow_native_geometry_data_type(),
            true,
        )]);
        let meta = meta_declaring("geometry_lod2_2", GeometryEncoding::ARROW_NATIVE_V1_TOKEN);
        let err = resolve_geometry_encoding(
            &meta,
            &schema,
            "geometry_lod2_2",
            "geometry_vertices_lod2_2",
        )
        .expect_err("a missing vertex-pool sibling must be an error");
        assert!(err.to_string().contains("vertex-pool sibling is absent"));
    }
}
