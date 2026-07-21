//! CityParquet profiles (core vs compatibility), sidecar table schemas,
//! and the `metadata.json` package manifest.

use arrow_schema::{DataType, Field, Schema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Core,
    Compatibility,
}

impl Profile {
    pub fn sidecar_files(&self) -> &'static [&'static str] {
        match self {
            Profile::Core => &[],
            Profile::Compatibility => &[
                "materials.parquet",
                "textures.parquet",
                "geometry_templates.parquet",
            ],
        }
    }
}

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
pub fn geometry_templates_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("geometry", DataType::Binary, false),
        json_col("geometry_properties"),
        json_col("material"),
        json_col("texture"),
        json_col("other"),
    ])
}

/// Contents of the package-level `metadata.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub cityparquet_version: String,
    pub profile: Profile,
    pub lods: Vec<String>,
    pub tables: Vec<String>,
    #[serde(default)]
    pub sidecar_files: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::DataType;

    #[test]
    fn core_profile_has_no_sidecars() {
        assert!(Profile::Core.sidecar_files().is_empty());
        assert_eq!(
            Profile::Compatibility.sidecar_files(),
            [
                "materials.parquet",
                "textures.parquet",
                "geometry_templates.parquet"
            ]
        );
    }

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
    }

    #[test]
    fn manifest_serialises_to_metadata_json() {
        let manifest = PackageManifest {
            cityparquet_version: crate::metadata::CITYPARQUET_VERSION.to_string(),
            profile: Profile::Compatibility,
            lods: vec!["1".to_string(), "2.2".to_string()],
            tables: vec!["building.parquet".to_string()],
            sidecar_files: vec!["materials.parquet".to_string()],
        };
        let text = serde_json::to_string_pretty(&manifest).unwrap();
        let back: PackageManifest = serde_json::from_str(&text).unwrap();
        assert_eq!(back, manifest);
        assert!(text.contains("\"profile\": \"compatibility\""));
    }
}
