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
use crate::types::{Lod, geometry_column_name};

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

/// The `geometry_properties[_lod*]` Arrow type (spec "Geometry properties and
/// semantics"):
///
/// ```text
/// STRUCT<
///   type            VARCHAR,          -- non-null
///   surfaces        JSON,             -- nullable
///   face_semantics  LIST<INT>,        -- nullable; items nullable
///   shells          LIST<LIST<INT>>   -- nullable; where non-null, both
///                                     -- nesting levels (inner LIST<INT> and
///                                     -- each INT) are themselves non-null
/// >
/// ```
///
/// There is no `lod` field — the column name carries the LoD. `shells` is
/// **always nested one inner list per solid**: a `Solid` (exactly one solid)
/// still gets one inner list (`[[12]]`, never the flat `[12]`), so a reader
/// never needs to special-case `Solid` vs `MultiSolid`/`CompositeSolid`.
pub fn geometry_properties_data_type() -> DataType {
    let face_semantics_item = Arc::new(Field::new("item", DataType::Int32, true));
    let face_semantics = DataType::List(face_semantics_item);

    // `shells`: non-null all the way down once populated (spec) — the inner
    // per-shell `INT` face count is non-nullable, and so is each solid's
    // inner `LIST<INT>`; only the whole `shells` column (absent for
    // non-solid types) is nullable.
    let shell_face_count = Arc::new(Field::new("item", DataType::Int32, false));
    let shell_list = DataType::List(shell_face_count);
    let per_solid_shells = Arc::new(Field::new("item", shell_list, false));
    let shells = DataType::List(per_solid_shells);

    DataType::Struct(Fields::from(vec![
        Field::new("type", DataType::Utf8, false),
        json_field("surfaces", true).as_ref().clone(),
        Field::new("face_semantics", face_semantics, true),
        Field::new("shells", shells, true),
    ]))
}

fn string_list(name: &str) -> Field {
    Field::new(
        name,
        DataType::List(Field::new("item", DataType::Utf8, true).into()),
        true,
    )
}

/// CityJSON extension attributes `+name` become `ex_name` columns (spec).
pub fn normalise_attribute_name(name: &str) -> String {
    match name.strip_prefix('+') {
        Some(rest) => format!("{EXTENSION_ATTR_PREFIX}{rest}"),
        None => name.to_string(),
    }
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
    "template",
    "other",
];

/// Every column name an attribute column must not collide with, for a schema
/// with these `lods`: the fixed reserved names plus the geometry/appearance
/// column names the LoDs realise. **Schema-relative** — every LoD reserves its
/// suffixed forms (`geometry_lod2_2`, …), and the zero-analysis-geometry case
/// (empty `lods`) reserves the bare names instead (no LoD to suffix by). This
/// is the single source of truth shared by [`CityParquetSchema::validate`]
/// (which errors on a collision) and the scan-time diversion of colliding
/// attributes into `other` (§5.2, G12), so the two can never diverge on what
/// "reserved" means.
pub fn reserved_and_geometry_column_names(lods: &[Lod]) -> HashSet<String> {
    let mut names: HashSet<String> = RESERVED_COLUMN_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
    if lods.is_empty() {
        names.insert("geometry".to_string());
        names.insert("geometry_properties".to_string());
        names.insert("material".to_string());
        names.insert("texture".to_string());
    } else {
        for lod in lods {
            names.insert(geometry_column_name("geometry", lod));
            names.insert(geometry_column_name("geometry_properties", lod));
            names.insert(geometry_column_name("material", lod));
            names.insert(geometry_column_name("texture", lod));
        }
    }
    names
}

impl CityParquetSchema {
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

        let reserved = reserved_and_geometry_column_names(&self.lods);
        for (name, _) in &self.attributes {
            if reserved.contains(name) {
                return Err(CityParquetError::Schema(format!(
                    "attribute column '{name}' collides with a reserved or geometry column name"
                )));
            }
        }
        Ok(())
    }

    fn geometry_field(&self, name: &str, lod: Option<&Lod>, geoarrow: bool) -> Field {
        let mut field = Field::new(name, DataType::Binary, true);
        if geoarrow {
            let crs = match &self.crs {
                Some(projjson) => Crs::from_projjson(projjson.clone()),
                None => Crs::default(),
            };
            let wkb = WkbType::new(Arc::new(GeoMetadata::new(crs, None)));
            field = field.with_extension_type(wkb);
        }
        let mut field = reserved(field);
        if let Some(lod) = lod {
            field = with_meta(field, &[(LOD_KEY, &lod.to_string())]);
        }
        field
    }

    /// Render the Arrow schema, columns in spec order. Geometry columns carry
    /// the `geoarrow.wkb` extension type (and CRS) iff `geoarrow` — the write
    /// path passes the caller's `--geoarrow` choice; every other caller wants
    /// the self-describing (tagged) form.
    pub fn to_arrow_schema_tagged(&self, geoarrow: bool) -> Result<Schema> {
        self.validate()?;

        let mut fields: Vec<Field> = vec![
            reserved(Field::new("id", DataType::Utf8, false)),
            // Non-null per spec §5.1: `feature_id` is the key a consumer
            // groups a feature family by, so a null orphans the row from its
            // own family. The encoder has always satisfied this (it appends
            // the source feature's non-optional id for every row); only the
            // declaration lagged. Readers stay tolerant of nulls — see
            // `cityparquet::decode` — since a foreign or pre-G4 file may
            // still carry them.
            reserved(Field::new("feature_id", DataType::Utf8, false)),
            reserved(Field::new(
                "object_type",
                DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
                false,
            )),
            reserved(string_list("parents")),
            reserved(string_list("children")),
            reserved(string_list("children_roles")),
            reserved(Field::new("bbox", bbox_data_type(), true)),
        ];

        if self.lods.is_empty() {
            fields.push(self.geometry_field("geometry", None, geoarrow));
            fields.push(with_meta(
                Field::new("geometry_properties", geometry_properties_data_type(), true),
                &[(ROLE_KEY, ROLE_RESERVED)],
            ));
            // Appearance parallels geometry (§11.1): the un-suffixed pair for
            // the zero-analysis-geometry fallback (a dataset with only
            // GeometryInstances, or none — §9). A lod-less non-instance
            // geometry never reaches here; scan rejects it (§9, G3).
            for name in ["material", "texture"] {
                fields.push(with_meta(
                    json_field(name, true).as_ref().clone(),
                    &[(ROLE_KEY, ROLE_RESERVED)],
                ));
            }
        } else {
            // Per-LoD columns, grouped so each LoD's geometry, semantics and
            // appearance sit adjacent: geometry_lodX, geometry_properties_lodX,
            // material_lodX, texture_lodX (§11.1). Appearance pairs to the
            // geometry it decorates by column name, not by a JSON LoD key.
            // Every LoD — including LoD0 — is suffixed; there is no
            // un-suffixed "footprint" column (spec "Levels of detail").
            for lod in &self.lods {
                fields.push(self.geometry_field(
                    &geometry_column_name("geometry", lod),
                    Some(lod),
                    geoarrow,
                ));
                fields.push(with_meta(
                    Field::new(
                        geometry_column_name("geometry_properties", lod),
                        geometry_properties_data_type(),
                        true,
                    ),
                    &[(ROLE_KEY, ROLE_RESERVED), (LOD_KEY, &lod.to_string())],
                ));
                for prefix in ["material", "texture"] {
                    fields.push(with_meta(
                        json_field(&geometry_column_name(prefix, lod), true)
                            .as_ref()
                            .clone(),
                        &[(ROLE_KEY, ROLE_RESERVED), (LOD_KEY, &lod.to_string())],
                    ));
                }
            }
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

    /// Tagged rendering — the self-describing GeoParquet/GeoArrow form every
    /// non-write caller (reader schema rebuild, `column_lists`, recipe
    /// geometry-column detection) expects.
    pub fn to_arrow_schema(&self) -> Result<Schema> {
        self.to_arrow_schema_tagged(true)
    }

    /// (reserved incl. geometry columns, attribute columns), derived from the
    /// rendered Arrow schema's `cityparquet:role` metadata so the metadata
    /// column lists can never drift from the schema itself.
    pub fn column_lists(&self) -> Result<(Vec<String>, Vec<String>)> {
        let schema = self.to_arrow_schema()?;
        let mut reserved = Vec::new();
        let mut attributes = Vec::new();
        for field in schema.fields() {
            match field.metadata().get(ROLE_KEY).map(String::as_str) {
                Some(ROLE_RESERVED) => reserved.push(field.name().clone()),
                _ => attributes.push(field.name().clone()),
            }
        }
        Ok((reserved, attributes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::AttributeType;
    use crate::types::{Lod, geometry_column_name};
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
                "geometry_lod1_0",
                "geometry_properties_lod1_0",
                "material_lod1_0",
                "texture_lod1_0",
                "geometry_lod2_2",
                "geometry_properties_lod2_2",
                "material_lod2_2",
                "texture_lod2_2",
                "template",
                "other",
                "yoc",
                "ex_height",
            ]
        );
    }

    /// spec "Levels of detail": LoD0 is suffixed exactly like any other LoD —
    /// no bare `geometry`/`geometry_properties`/`material`/`texture` column.
    #[test]
    fn lod0_is_suffixed_like_any_other_lod() {
        let schema = CityParquetSchema {
            lods: vec![Lod::parse("0").unwrap(), Lod::parse("2.2").unwrap()],
            attributes: vec![],
            crs: None,
        };
        let arrow = schema.to_arrow_schema().unwrap();
        assert!(arrow.field_with_name("geometry_lod0_0").is_ok());
        assert!(arrow.field_with_name("geometry_properties_lod0_0").is_ok());
        assert!(arrow.field_with_name("material_lod0_0").is_ok());
        assert!(arrow.field_with_name("texture_lod0_0").is_ok());
        assert!(arrow.field_with_name("geometry_lod2_2").is_ok());
        // No bare/un-suffixed column ever appears.
        assert!(arrow.field_with_name("geometry").is_err());
        assert!(arrow.field_with_name("geometry_properties").is_err());
        assert!(arrow.field_with_name("material").is_err());
        assert!(arrow.field_with_name("texture").is_err());
        let f = arrow.field_with_name("geometry_lod0_0").unwrap();
        assert_eq!(f.metadata().get(LOD_KEY).map(String::as_str), Some("0.0"));
    }

    #[test]
    fn reserved_names_suffix_every_lod_including_zero() {
        let names = reserved_and_geometry_column_names(&[Lod::parse("0").unwrap()]);
        assert!(names.contains("geometry_lod0_0"));
        assert!(names.contains("geometry_properties_lod0_0"));
        assert!(names.contains("material_lod0_0"));
        assert!(names.contains("texture_lod0_0"));
        assert!(!names.contains("geometry"));
        // A mixed schema reserves every LoD's suffixed forms.
        let mixed = reserved_and_geometry_column_names(&[
            Lod::parse("0").unwrap(),
            Lod::parse("2").unwrap(),
        ]);
        assert!(mixed.contains("geometry_lod0_0"));
        assert!(mixed.contains("geometry_lod2_0"));
    }

    #[test]
    fn geometry_column_name_helper_is_wired() {
        let lod0 = Lod::parse("0").unwrap();
        assert_eq!(geometry_column_name("geometry", &lod0), "geometry_lod0_0");
    }

    /// A dataset with LoD 0.1, 0.3, and 2.2: every one of them, including
    /// both members of the `0.*` family, keeps its own suffixed column — no
    /// single 0.* LoD is picked out to go unsuffixed.
    #[test]
    fn every_zero_family_lod_keeps_its_own_suffix() {
        let schema = CityParquetSchema {
            lods: vec![
                Lod::parse("0.1").unwrap(),
                Lod::parse("0.3").unwrap(),
                Lod::parse("2.2").unwrap(),
            ],
            attributes: vec![],
            crs: None,
        };
        let arrow = schema.to_arrow_schema().unwrap();
        assert!(arrow.field_with_name("geometry_lod0_1").is_ok());
        assert!(arrow.field_with_name("geometry_lod0_3").is_ok());
        assert!(arrow.field_with_name("geometry_lod2_2").is_ok());
        assert!(arrow.field_with_name("geometry").is_err());
    }

    /// RED (G20): spec §11.1 makes appearance per-LoD columns
    /// (`material_lod*` / `texture_lod*`), one pair per `geometry_lod*`,
    /// so the LoD-to-appearance pairing rides on the column name rather than
    /// a JSON key. No bare `material` / `texture` column exists.
    #[test]
    fn appearance_columns_are_per_lod() {
        let schema = sample().to_arrow_schema().unwrap();
        for lod_suffix in ["lod1_0", "lod2_2"] {
            assert!(
                schema
                    .field_with_name(&format!("material_{lod_suffix}"))
                    .is_ok(),
                "expected a material_{lod_suffix} column"
            );
            assert!(
                schema
                    .field_with_name(&format!("texture_{lod_suffix}"))
                    .is_ok(),
                "expected a texture_{lod_suffix} column"
            );
        }
        assert!(
            schema.field_with_name("material").is_err(),
            "the bare `material` column must not exist (§11.1)"
        );
        assert!(
            schema.field_with_name("texture").is_err(),
            "the bare `texture` column must not exist (§11.1)"
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

    /// RED (G4): spec §5.1 marks `feature_id` required and non-null on every
    /// row — it is the key a consumer groups a feature family by (§5.1's
    /// `feature_id` rule), so a null would silently orphan a row from its own
    /// family. The encoder already never writes one (it always appends the
    /// source feature's non-optional id), so only the declaration is wrong.
    #[test]
    fn required_columns_are_non_nullable() {
        let schema = sample().to_arrow_schema().unwrap();
        for name in ["id", "feature_id", "object_type"] {
            assert!(
                !schema.field_with_name(name).unwrap().is_nullable(),
                "spec §5.1 requires '{name}' to be non-null on every row"
            );
        }
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
    fn geometry_field_tag_is_toggleable() {
        let schema = sample();

        // Tagged (default zero-arg and explicit true): geoarrow.wkb present.
        for tagged in [
            schema.to_arrow_schema().unwrap(),
            schema.to_arrow_schema_tagged(true).unwrap(),
        ] {
            let field = tagged.field_with_name("geometry_lod2_2").unwrap();
            assert_eq!(
                field
                    .metadata()
                    .get("ARROW:extension:name")
                    .map(String::as_str),
                Some("geoarrow.wkb"),
                "tagged schema must advertise geoarrow.wkb"
            );
        }

        // Untagged: NO geoarrow extension, but the binary type and the
        // cityparquet role/lod metadata that decode relies on must survive.
        let untagged = schema.to_arrow_schema_tagged(false).unwrap();
        let field = untagged.field_with_name("geometry_lod2_2").unwrap();
        assert_eq!(field.data_type(), &arrow_schema::DataType::Binary);
        assert!(
            !field.metadata().contains_key("ARROW:extension:name"),
            "untagged geometry field must not advertise any Arrow extension type"
        );
        assert_eq!(
            field.metadata().get("cityparquet:role").map(String::as_str),
            Some("reserved"),
            "role metadata must survive so decode still classifies the column"
        );
        assert_eq!(
            field.metadata().get("cityparquet:lod").map(String::as_str),
            Some("2.2"),
            "lod metadata must survive"
        );
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
            get("geometry_properties_lod1_0", LOD_KEY).as_deref(),
            Some("1.0")
        );
    }

    #[test]
    fn json_columns_carry_arrow_json_extension() {
        let schema = sample().to_arrow_schema().unwrap();
        for name in ["material_lod1_0", "texture_lod2_2", "other"] {
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

    /// spec "Geometry properties and semantics": `geometry_properties_lod*`
    /// is a genuine Arrow `STRUCT` with typed children — `type` (non-null
    /// `Utf8`), `surfaces` (nullable `Utf8` tagged `arrow.json`),
    /// `face_semantics` (nullable `List<Int32>`, items nullable), `shells`
    /// (nullable `List<List<Int32 non-null> non-null>`) — not a JSON-tagged
    /// Utf8 blob. Asserted via Arrow's own `Field`/`DataType`
    /// introspection, not decoded row values.
    #[test]
    fn geometry_properties_is_a_typed_struct_not_json() {
        let schema = sample().to_arrow_schema().unwrap();
        let field = schema.field_with_name("geometry_properties_lod2_2").unwrap();
        assert!(
            !field.metadata().contains_key("ARROW:extension:name"),
            "the outer geometry_properties field must not itself be tagged arrow.json"
        );
        let DataType::Struct(children) = field.data_type() else {
            panic!("geometry_properties must be a Struct, got {:?}", field.data_type());
        };
        assert_eq!(
            children.iter().map(|f| f.name().as_str()).collect::<Vec<_>>(),
            vec!["type", "surfaces", "face_semantics", "shells"],
            "exactly these four fields, in this order"
        );

        let type_field = children.iter().find(|f| f.name() == "type").unwrap();
        assert_eq!(type_field.data_type(), &DataType::Utf8);
        assert!(!type_field.is_nullable(), "type is non-null");

        let surfaces_field = children.iter().find(|f| f.name() == "surfaces").unwrap();
        assert_eq!(surfaces_field.data_type(), &DataType::Utf8);
        assert!(surfaces_field.is_nullable());
        assert_eq!(
            surfaces_field
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str),
            Some("arrow.json"),
            "surfaces stays JSON (heterogeneous per-surface attributes)"
        );

        let fs_field = children.iter().find(|f| f.name() == "face_semantics").unwrap();
        assert!(fs_field.is_nullable());
        let DataType::List(fs_item) = fs_field.data_type() else {
            panic!("face_semantics must be List, got {:?}", fs_field.data_type());
        };
        assert_eq!(fs_item.data_type(), &DataType::Int32);
        assert!(fs_item.is_nullable(), "face_semantics items are nullable");

        let shells_field = children.iter().find(|f| f.name() == "shells").unwrap();
        assert!(shells_field.is_nullable());
        let DataType::List(solid_item) = shells_field.data_type() else {
            panic!("shells must be List, got {:?}", shells_field.data_type());
        };
        assert!(
            !solid_item.is_nullable(),
            "each solid's inner shell-count list is non-null once shells is populated"
        );
        let DataType::List(count_item) = solid_item.data_type() else {
            panic!(
                "shells' items must themselves be List, got {:?}",
                solid_item.data_type()
            );
        };
        assert_eq!(count_item.data_type(), &DataType::Int32);
        assert!(
            !count_item.is_nullable(),
            "each per-shell face count is non-null once shells is populated"
        );
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

    #[test]
    fn bbox_is_nullable() {
        let schema = sample().to_arrow_schema().unwrap();
        assert!(schema.field_with_name("bbox").unwrap().is_nullable());
    }

    #[test]
    fn normalises_extension_attribute_names() {
        assert_eq!(normalise_attribute_name("+height"), "ex_height");
        assert_eq!(normalise_attribute_name("height"), "height");
        assert_eq!(normalise_attribute_name("+"), "ex_");
    }

    #[test]
    fn column_lists_partition_by_role() {
        let (reserved, attrs) = sample().column_lists().unwrap();
        assert!(reserved.contains(&"id".to_string()));
        assert!(reserved.contains(&"geometry_lod2_2".to_string()));
        assert!(!reserved.contains(&"yoc".to_string()));
        assert_eq!(attrs, vec!["yoc".to_string(), "ex_height".to_string()]);
    }
}
