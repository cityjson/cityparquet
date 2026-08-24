//! Which physical [`GeometryEncoding`] a `geometry_lod*` column actually
//! carries — resolved from the file's OWN footer declaration
//! (`city.columns[].encoding`, spec `05-metadata.mdx`), then checked against
//! the physical column. Shared by [`crate::reader`] (schema rendering) and
//! [`crate::decode`] (row decoding) so the two can never disagree about what
//! a given file is.
//!
//! **Declaration first, shape second.** The footer's declared token decides
//! the encoding; a token this build does not understand is a hard error, never
//! licence to guess from the column's physical Arrow type. Inferring the
//! encoding structurally would be wrong in two ways: a footer contradicting
//! its own columns would be silently ignored rather than rejected, and a
//! future encoding declaring a token this build has never heard of would be
//! decoded as though it were WKB. Only once the encoding is settled is the
//! physical `Field` required to match it.

use arrow_schema::{DataType, Schema};

use cityparquet_schema::{CityMetadata, CityParquetError, GeometryEncoding, Result};

fn metadata_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Metadata(msg.into())
}

/// The encoding `meta.columns` declares for `geometry_name`, or `None` when
/// this footer declares nothing for that column at all (see
/// [`encoding_from_physical_shape`] for what happens then). A token this
/// build does not understand is an error, never a silent fall back to
/// shape-guessing.
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
/// geometry-less/legacy bare-`geometry` shapes. `Binary` is WKB; anything
/// else is rejected rather than misread.
fn encoding_from_physical_shape(
    geometry_name: &str,
    data_type: &DataType,
) -> Result<GeometryEncoding> {
    if data_type == &DataType::Binary {
        return Ok(GeometryEncoding::Wkb);
    }
    Err(metadata_err(format!(
        "geometry column '{geometry_name}' has no city.columns[] entry declaring its encoding, \
         and its arrow type is {data_type:?}, not the Binary a \"{}\" column must be",
        GeometryEncoding::WKB_TOKEN,
    )))
}

/// Whether `schema`'s physical column really is what `encoding` says it is.
fn verify_physical_shape(
    schema: &Schema,
    geometry_name: &str,
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
/// column), and the physical column must then match that encoding exactly.
pub(crate) fn resolve_geometry_encoding(
    meta: &CityMetadata,
    schema: &Schema,
    geometry_name: &str,
) -> Result<GeometryEncoding> {
    let field = geometry_field(schema, geometry_name)?;
    let encoding = match declared_encoding(meta, geometry_name)? {
        Some(declared) => declared,
        None => encoding_from_physical_shape(geometry_name, field.data_type())?,
    };
    verify_physical_shape(schema, geometry_name, encoding)?;
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
            crs: cityparquet_schema::CrsState::Unspecified,
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

    #[test]
    fn a_declared_encoding_matching_its_columns_resolves() {
        let meta = meta_declaring("geometry_lod2_2", GeometryEncoding::WKB_TOKEN);
        let resolved = resolve_geometry_encoding(&meta, &wkb_schema(), "geometry_lod2_2").unwrap();
        assert_eq!(resolved, GeometryEncoding::Wkb);
    }

    /// An undeclared column that is not `Binary` must be refused rather than
    /// assumed to be WKB.
    #[test]
    fn an_undeclared_non_binary_column_is_refused() {
        let schema = Schema::new(vec![Field::new(
            "geometry_lod2_2",
            DataType::List(Field::new("item", DataType::Int32, false).into()),
            true,
        )]);
        let err = resolve_geometry_encoding(&empty_meta(), &schema, "geometry_lod2_2")
            .expect_err("a non-Binary column must not be assumed to be WKB");
        assert!(err.to_string().contains("not the Binary"));
    }

    /// A token this build does not understand is a hard error, so a future
    /// encoding is never silently misread as WKB.
    #[test]
    fn an_unknown_declared_token_is_refused() {
        let meta = meta_declaring("geometry_lod2_2", "SomeFutureEncoding-v3");
        let err = resolve_geometry_encoding(&meta, &wkb_schema(), "geometry_lod2_2")
            .expect_err("an unknown encoding token must be an error");
        assert!(err.to_string().contains("does not understand"));
    }
}
