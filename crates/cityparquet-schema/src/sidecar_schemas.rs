//! Sidecar table schemas (`materials.parquet`, `textures.parquet`,
//! `geometry_templates.parquet`) — spec-alignment M3 dropped the `Profile`
//! concept these used to live alongside (gap 19: a writer now emits a
//! sidecar whenever the source has content for it, never gated by a
//! core/compatibility profile choice), so this module keeps only what was
//! never profile-specific to begin with. `metadata.json` itself is a STAC
//! Item, built by `cityparquet::stac` — not a type defined in this crate.

use arrow_schema::{DataType, Field, Schema};

use crate::model::geometry_properties_data_type;

fn json_col(name: &str) -> Field {
    Field::new(name, DataType::Utf8, true)
        .with_metadata([("ARROW:extension:name".to_string(), "arrow.json".to_string())].into())
}

/// `materials.parquet` schema (spec § Appearance encoding). `id` is the
/// dataset-global row index assigned by the appearance interner (see
/// `cityparquet::appearance::AppearanceInterner`).
pub fn materials_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("ambientIntensity", DataType::Float64, true),
        json_col("diffuseColor"),
        json_col("specularColor"),
        json_col("emissiveColor"),
        Field::new("transparency", DataType::Float64, true),
        Field::new("shininess", DataType::Float64, true),
        Field::new("isSmooth", DataType::Boolean, true),
        json_col("other"),
    ])
}

/// `textures.parquet` schema (spec § Appearance encoding). `id` is the
/// dataset-global row index assigned by the appearance interner. No
/// `vertices_texture` column: UV coordinates are inlined directly into each
/// geometry's `texture` map by the interner rewrite, so the per-feature UV
/// vertex pool never needs a stable global identity of its own.
pub fn textures_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        // No `name`: a CityJSON `Texture` has no `name` member (§11.3, G16).
        Field::new("image_uri", DataType::Utf8, true),
        Field::new("image_data", DataType::Binary, true),
        Field::new("mime_type", DataType::Utf8, true),
        Field::new("wrapMode", DataType::Utf8, true),
        Field::new("textureType", DataType::Utf8, true),
        json_col("borderColor"),
        json_col("other"),
    ])
}

/// `geometry_templates.parquet` schema (spec § Geometry template encoding).
///
/// `geometry_properties` is the same `STRUCT<type, surfaces, face_semantics,
/// shells>` the main object table uses (spec: "Applies to the template
/// sidecar's `geometry_properties_lod*` too ... same struct, reused"), which
/// has no `lod` field — unlike the main table, this sidecar has no per-LoD
/// column name to carry a template's LoD in, so it gets its own `lod` column
/// instead.
pub fn geometry_templates_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("geometry", DataType::Binary, false),
        Field::new("geometry_properties", geometry_properties_data_type(), true),
        Field::new("lod", DataType::Utf8, true),
        json_col("material"),
        json_col("texture"),
        json_col("other"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::DataType;

    #[test]
    fn sidecar_schemas_match_spec_tables() {
        let m = materials_schema();
        for col in [
            "id",
            "name",
            "ambientIntensity",
            "diffuseColor",
            "specularColor",
            "emissiveColor",
            "transparency",
            "shininess",
            "isSmooth",
            "other",
        ] {
            assert!(m.field_with_name(col).is_ok(), "materials missing {col}");
        }
        assert_eq!(
            m.field_with_name("id").unwrap().data_type(),
            &DataType::Int64
        );
        assert!(!m.field_with_name("id").unwrap().is_nullable());

        let t = textures_schema();
        for col in [
            "id",
            "image_uri",
            "image_data",
            "mime_type",
            "wrapMode",
            "textureType",
            "borderColor",
            "other",
        ] {
            assert!(t.field_with_name(col).is_ok(), "textures missing {col}");
        }
        // §11.3/G16: a CityJSON Texture has no `name` member, so no `name` column.
        assert!(
            t.field_with_name("name").is_err(),
            "textures schema must not carry a `name` column (§11.3)"
        );
        assert!(
            t.field_with_name("vertices_texture").is_err(),
            "textures schema must not carry vertices_texture: UV coordinates are \
             inlined into each geometry's texture map instead"
        );
        assert_eq!(
            t.field_with_name("id").unwrap().data_type(),
            &DataType::Int64
        );
        assert!(!t.field_with_name("id").unwrap().is_nullable());
        assert_eq!(
            t.field_with_name("image_data").unwrap().data_type(),
            &DataType::Binary
        );
        let g = geometry_templates_schema();
        for col in [
            "id",
            "geometry",
            "geometry_properties",
            "lod",
            "material",
            "texture",
            "other",
        ] {
            assert!(g.field_with_name(col).is_ok(), "templates missing {col}");
        }
        assert_eq!(
            g.field_with_name("geometry").unwrap().data_type(),
            &DataType::Binary
        );
        assert!(matches!(
            g.field_with_name("geometry_properties")
                .unwrap()
                .data_type(),
            DataType::Struct(_)
        ));
        assert_eq!(
            g.field_with_name("lod").unwrap().data_type(),
            &DataType::Utf8
        );
    }
}
