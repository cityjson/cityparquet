//! The CityParquet object-table schema: reserved columns, per-LoD geometry,
//! and inferred attributes, rendered as an Arrow schema with self-describing
//! field metadata.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{DataType, Field, Fields, Schema};
use geoarrow_schema::{Crs, Metadata as GeoMetadata, WkbType};

use crate::attributes::AttributeType;
use crate::error::{CityParquetError, Result};
use crate::types::Lod;

pub const ROLE_KEY: &str = "cityparquet:role";
pub const LOD_KEY: &str = "cityparquet:lod";

pub const ROLE_RESERVED: &str = "reserved";
pub const ROLE_ATTRIBUTE: &str = "attribute";
pub const ROLE_EXTENSION: &str = "extension";

/// Extension-attribute columns are renamed from `+name` to `ex_name` (spec).
pub const EXTENSION_ATTR_PREFIX: &str = "ex_";

/// Logical description of one CityParquet object table.
#[derive(Debug, Clone, PartialEq)]
pub struct CityParquetSchema {
    /// LoDs present, ascending. Empty means a single un-suffixed `geometry` column.
    pub lods: Vec<Lod>,
    /// Inferred attribute columns in first-seen order.
    pub attributes: Vec<(String, AttributeType)>,
    /// Dataset CRS as PROJJSON.
    pub crs: Option<serde_json::Value>,
}

pub fn bbox_data_type() -> DataType {
    DataType::Struct(Fields::from(
        ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"]
            .map(|n| Field::new(n, DataType::Float64, false))
            .to_vec(),
    ))
}

pub fn template_data_type() -> DataType {
    DataType::Struct(Fields::from(vec![
        Field::new("id", DataType::Utf8, true),
        Field::new("point", DataType::Binary, true),
        json_field("transformationMatrix", true).as_ref().clone(),
    ]))
}

fn with_meta(field: Field, pairs: &[(&str, &str)]) -> Field {
    let mut meta: HashMap<String, String> = field.metadata().clone();
    meta.extend(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    field.with_metadata(meta)
}

fn reserved(field: Field) -> Field {
    with_meta(field, &[(ROLE_KEY, ROLE_RESERVED)])
}

/// A Utf8 field tagged with the canonical `arrow.json` extension type.
fn json_field(name: &str, nullable: bool) -> Arc<Field> {
    Arc::new(with_meta(
        Field::new(name, DataType::Utf8, nullable),
        &[(EXTENSION_TYPE_NAME_KEY, "arrow.json")],
    ))
}

fn string_list(name: &str) -> Field {
    Field::new(
        name,
        DataType::List(Field::new("item", DataType::Utf8, true).into()),
        true,
    )
}

/// Fixed reserved column names, independent of `lods`/`attributes`.
const RESERVED_COLUMN_NAMES: &[&str] = &[
    "id",
    "feature_id",
    "object_type",
    "parents",
    "children",
    "children_roles",
    "bbox",
    "material",
    "texture",
    "template",
    "other",
];

impl CityParquetSchema {
    /// The reserved + per-LoD geometry column names for this schema instance,
    /// i.e. every name an attribute column must not collide with.
    fn reserved_and_geometry_column_names(&self) -> HashSet<String> {
        let mut names: HashSet<String> = RESERVED_COLUMN_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        if self.lods.is_empty() {
            names.insert("geometry".to_string());
            names.insert("geometry_properties".to_string());
        } else {
            for lod in &self.lods {
                let suffix = lod.column_suffix();
                names.insert(format!("geometry_{suffix}"));
                names.insert(format!("geometry_properties_{suffix}"));
            }
        }
        names
    }

    /// Reject schemas that can't be rendered unambiguously: duplicate LoDs, or
    /// an attribute name colliding with a reserved or geometry column name.
    fn validate(&self) -> Result<()> {
        let mut seen_lods = HashSet::new();
        for lod in &self.lods {
            if !seen_lods.insert(*lod) {
                return Err(CityParquetError::Schema(format!(
                    "duplicate LoD in schema: {lod}"
                )));
            }
        }

        let reserved = self.reserved_and_geometry_column_names();
        for (name, _) in &self.attributes {
            if reserved.contains(name) {
                return Err(CityParquetError::Schema(format!(
                    "attribute column '{name}' collides with a reserved or geometry column name"
                )));
            }
        }
        Ok(())
    }

    fn geometry_field(&self, name: &str, lod: Option<&Lod>) -> Field {
        let crs = match &self.crs {
            Some(projjson) => Crs::from_projjson(projjson.clone()),
            None => Crs::default(),
        };
        let wkb = WkbType::new(Arc::new(GeoMetadata::new(crs, None)));
        let field = Field::new(name, DataType::Binary, true).with_extension_type(wkb);
        let mut field = reserved(field);
        if let Some(lod) = lod {
            field = with_meta(field, &[(LOD_KEY, &lod.to_string())]);
        }
        field
    }

    /// Render the Arrow schema, columns in spec order.
    pub fn to_arrow_schema(&self) -> Result<Schema> {
        self.validate()?;

        let mut fields: Vec<Field> = vec![
            reserved(Field::new("id", DataType::Utf8, false)),
            reserved(Field::new("feature_id", DataType::Utf8, true)),
            reserved(Field::new(
                "object_type",
                DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
                false,
            )),
            reserved(string_list("parents")),
            reserved(string_list("children")),
            reserved(string_list("children_roles")),
            reserved(Field::new("bbox", bbox_data_type(), false)),
        ];

        if self.lods.is_empty() {
            fields.push(self.geometry_field("geometry", None));
            fields.push(with_meta(
                json_field("geometry_properties", true).as_ref().clone(),
                &[(ROLE_KEY, ROLE_RESERVED)],
            ));
        } else {
            for lod in &self.lods {
                let suffix = lod.column_suffix();
                fields.push(self.geometry_field(&format!("geometry_{suffix}"), Some(lod)));
                fields.push(with_meta(
                    json_field(&format!("geometry_properties_{suffix}"), true)
                        .as_ref()
                        .clone(),
                    &[(ROLE_KEY, ROLE_RESERVED), (LOD_KEY, &lod.to_string())],
                ));
            }
        }

        for name in ["material", "texture"] {
            fields.push(with_meta(
                json_field(name, true).as_ref().clone(),
                &[(ROLE_KEY, ROLE_RESERVED)],
            ));
        }
        fields.push(reserved(Field::new("template", template_data_type(), true)));
        fields.push(with_meta(
            json_field("other", true).as_ref().clone(),
            &[(ROLE_KEY, ROLE_RESERVED)],
        ));

        for (name, attr_type) in &self.attributes {
            let role = if name.starts_with(EXTENSION_ATTR_PREFIX) {
                ROLE_EXTENSION
            } else {
                ROLE_ATTRIBUTE
            };
            let mut field = with_meta(
                Field::new(name, attr_type.to_arrow(), true),
                &[(ROLE_KEY, role)],
            );
            if *attr_type == AttributeType::Json {
                field = with_meta(field, &[(EXTENSION_TYPE_NAME_KEY, "arrow.json")]);
            }
            fields.push(field);
        }

        Ok(Schema::new(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::AttributeType;
    use crate::types::Lod;
    use arrow_schema::DataType;

    fn sample() -> CityParquetSchema {
        CityParquetSchema {
            lods: vec![Lod::parse("1").unwrap(), Lod::parse("2.2").unwrap()],
            attributes: vec![
                ("yoc".to_string(), AttributeType::Int64),
                ("ex_height".to_string(), AttributeType::Float64),
            ],
            crs: Some(serde_json::json!({"id": {"authority": "EPSG", "code": 28992}})),
        }
    }

    #[test]
    fn reserved_columns_in_spec_order() {
        let schema = sample().to_arrow_schema().unwrap();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec![
                "id",
                "feature_id",
                "object_type",
                "parents",
                "children",
                "children_roles",
                "bbox",
                "geometry_lod1",
                "geometry_properties_lod1",
                "geometry_lod2_2",
                "geometry_properties_lod2_2",
                "material",
                "texture",
                "template",
                "other",
                "yoc",
                "ex_height",
            ]
        );
    }

    #[test]
    fn key_column_types() {
        let schema = sample().to_arrow_schema().unwrap();
        assert_eq!(
            schema.field_with_name("id").unwrap().data_type(),
            &DataType::Utf8
        );
        assert!(!schema.field_with_name("id").unwrap().is_nullable());
        assert!(matches!(
            schema.field_with_name("object_type").unwrap().data_type(),
            DataType::Dictionary(_, _)
        ));
        assert!(
            matches!(schema.field_with_name("bbox").unwrap().data_type(), DataType::Struct(f) if f.len() == 6)
        );
        assert_eq!(
            schema
                .field_with_name("geometry_lod2_2")
                .unwrap()
                .data_type(),
            &DataType::Binary
        );
    }

    #[test]
    fn geometry_field_is_geoarrow_wkb() {
        let schema = sample().to_arrow_schema().unwrap();
        let field = schema.field_with_name("geometry_lod2_2").unwrap();
        assert_eq!(
            field
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str),
            Some("geoarrow.wkb")
        );
        // CRS travels in the extension metadata.
        let ext_meta = field.metadata().get("ARROW:extension:metadata").unwrap();
        assert!(ext_meta.contains("28992"));
    }

    #[test]
    fn role_and_lod_field_metadata() {
        let schema = sample().to_arrow_schema().unwrap();
        let get = |name: &str, key: &str| {
            schema
                .field_with_name(name)
                .unwrap()
                .metadata()
                .get(key)
                .cloned()
        };
        assert_eq!(get("id", ROLE_KEY).as_deref(), Some("reserved"));
        assert_eq!(get("yoc", ROLE_KEY).as_deref(), Some("attribute"));
        assert_eq!(get("ex_height", ROLE_KEY).as_deref(), Some("extension"));
        assert_eq!(get("geometry_lod2_2", LOD_KEY).as_deref(), Some("2.2"));
        assert_eq!(
            get("geometry_properties_lod1", LOD_KEY).as_deref(),
            Some("1")
        );
    }

    #[test]
    fn json_columns_carry_arrow_json_extension() {
        let schema = sample().to_arrow_schema().unwrap();
        for name in ["geometry_properties_lod1", "material", "texture", "other"] {
            let field = schema.field_with_name(name).unwrap();
            assert_eq!(
                field
                    .metadata()
                    .get("ARROW:extension:name")
                    .map(String::as_str),
                Some("arrow.json"),
                "{name} should be arrow.json"
            );
            assert_eq!(field.data_type(), &DataType::Utf8);
        }
    }

    #[test]
    fn no_lods_yields_plain_geometry_column() {
        let schema = CityParquetSchema {
            lods: vec![],
            attributes: vec![],
            crs: None,
        }
        .to_arrow_schema()
        .unwrap();
        assert!(schema.field_with_name("geometry").is_ok());
        assert!(schema.field_with_name("geometry_properties").is_ok());
    }

    #[test]
    fn attribute_colliding_with_reserved_column_is_an_error() {
        for bad_name in ["id", "material"] {
            let schema = CityParquetSchema {
                lods: vec![],
                attributes: vec![(bad_name.to_string(), AttributeType::String)],
                crs: None,
            };
            let err = schema.to_arrow_schema().unwrap_err();
            assert!(
                matches!(err, crate::error::CityParquetError::Schema(_)),
                "expected Schema error for attribute {bad_name}, got {err:?}"
            );
        }
    }

    #[test]
    fn duplicate_lods_are_an_error() {
        let schema = CityParquetSchema {
            lods: vec![Lod::parse("2").unwrap(), Lod::parse("2").unwrap()],
            attributes: vec![],
            crs: None,
        };
        let err = schema.to_arrow_schema().unwrap_err();
        assert!(matches!(err, crate::error::CityParquetError::Schema(_)));
    }

    #[test]
    fn attribute_colliding_with_geometry_column_is_an_error() {
        let schema = CityParquetSchema {
            lods: vec![Lod::parse("2.2").unwrap()],
            attributes: vec![("geometry_lod2_2".to_string(), AttributeType::String)],
            crs: None,
        };
        let err = schema.to_arrow_schema().unwrap_err();
        assert!(matches!(err, crate::error::CityParquetError::Schema(_)));
    }
}
