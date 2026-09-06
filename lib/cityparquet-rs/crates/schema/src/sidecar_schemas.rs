//! Sidecar table schemas (`materials.parquet`, `textures.parquet`,
//! `geometry_templates.parquet`) — spec-alignment M3 dropped the `Profile`
//! concept these used to live alongside (gap 19: a writer now emits a
//! sidecar whenever the source has content for it, never gated by a
//! core/compatibility profile choice), so this module keeps only what was
//! never profile-specific to begin with. `metadata.json` itself is a STAC
//! Item, built by `cityparquet::stac` — not a type defined in this crate.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};

use crate::model::{geometry_properties_data_type, material_data_type, texture_data_type};
use crate::types::{Lod, geometry_column_name};

fn json_col(name: &str) -> Field {
    Field::new(name, DataType::Utf8, true)
        .with_metadata([("ARROW:extension:name".to_string(), "arrow.json".to_string())].into())
}

/// A nullable `LIST<DOUBLE>` column (spec "materials.parquet" /
/// "textures.parquet": `diffuseColor`/`specularColor`/`emissiveColor`/
/// `borderColor` — "`LIST<DOUBLE>` is used rather than a fixed-size list
/// because fixed-size lists are unevenly supported across Parquet readers;
/// the cardinality is stated as a constraint instead"). List items are
/// non-null: a colour component is always a real number when the list
/// itself is present.
fn double_list_col(name: &str) -> Field {
    Field::new(
        name,
        DataType::List(Arc::new(Field::new("item", DataType::Float64, false))),
        true,
    )
}

/// `materials.parquet` schema (spec "materials.parquet"). `id` is the
/// dataset-global row index assigned by the appearance interner (see
/// `cityparquet::appearance::AppearanceInterner`). `diffuseColor`/
/// `specularColor`/`emissiveColor` are `LIST<DOUBLE>` (each, when non-null,
/// exactly 3 values in `[0,1]` — enforced by `cityparquet::sidecar`'s
/// writer, not by the Arrow type itself).
pub fn materials_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("ambientIntensity", DataType::Float64, true),
        double_list_col("diffuseColor"),
        double_list_col("specularColor"),
        double_list_col("emissiveColor"),
        Field::new("transparency", DataType::Float64, true),
        Field::new("shininess", DataType::Float64, true),
        Field::new("isSmooth", DataType::Boolean, true),
        json_col("other"),
    ])
}

/// `textures.parquet` schema (spec "textures.parquet"). `id` is the
/// dataset-global row index assigned by the appearance interner. No
/// `vertices_texture` column: UV coordinates are inlined directly into each
/// geometry's `texture` map by the interner rewrite, so the per-feature UV
/// vertex pool never needs a stable global identity of its own.
///
/// `image_type` (spec: "not a MIME type" — a format token like `"JPG"`/
/// `"PNG"`, verbatim from the source, so no enum validation applies to it,
/// unlike `wrapMode`/`textureType`, which are enumerations `cityparquet::
/// sidecar`'s writer validates). `borderColor` is `LIST<DOUBLE>` (exactly 4
/// values in `[0,1]` when non-null, enforced by the writer).
pub fn textures_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        // No `name`: a CityJSON `Texture` has no `name` member (§11.3, G16).
        Field::new("image_uri", DataType::Utf8, true),
        Field::new("image_data", DataType::Binary, true),
        Field::new("image_type", DataType::Utf8, true),
        Field::new("wrapMode", DataType::Utf8, true),
        Field::new("textureType", DataType::Utf8, true),
        double_list_col("borderColor"),
        json_col("other"),
    ])
}

/// `geometry_templates.parquet` schema (spec "geometry_templates.parquet"),
/// per-LoD-suffixed exactly like the main object table's own geometry and
/// appearance columns (spec: "using the same geometry strategy as the
/// object table ... so a template's LoD is carried by its column name here
/// exactly as it is in an object table"). `lods` is the set of LoDs actually
/// used by the templates being rendered — not necessarily every LoD the
/// dataset's own object table carries.
///
/// A template row populates exactly the column set matching its own LoD and
/// leaves every other LoD's columns null (spec: "each row populates exactly
/// the column set matching its own LoD ... sparse by construction"). There
/// is no `lod` column (the column name already carries it, like the main
/// table) and no `other` column (spec: "a geometry template is a plain
/// geometry (WKB + properties + appearance) with no members left over to
/// preserve").
///
/// `geometry_properties_lod*` reuses the exact same `STRUCT<type, surfaces,
/// face_semantics, shells>` the main object table uses (spec: "same struct,
/// reused").
///
/// Unlike the main table's own geometry columns, a template's `geometry_lod*`
/// carries no `geoarrow.wkb`/CRS tagging: template coordinates are in the
/// template's own LOCAL frame, exempt from the file CRS (spec: "Templates
/// are in local coordinates, and are exempt from the file CRS").
pub fn geometry_templates_schema(lods: &[Lod]) -> Schema {
    // `id` is BIGINT, not the template's source label: sidecar ids are
    // renumbered by an integer offset (`dst_max + 1 - src_min`) when packages
    // merge, which a string cannot carry, and the object table's
    // `template.id` that references this column is BIGINT too. `name` is
    // where a source identifier survives — null for CityJSON, whose
    // templates are unnamed array entries.
    let mut fields = vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ];
    for lod in lods {
        fields.push(Field::new(
            geometry_column_name("geometry", lod),
            DataType::Binary,
            true,
        ));
        fields.push(Field::new(
            geometry_column_name("geometry_properties", lod),
            geometry_properties_data_type(),
            true,
        ));
        fields.push(Field::new(
            geometry_column_name("material", lod),
            material_data_type(),
            true,
        ));
        fields.push(Field::new(
            geometry_column_name("texture", lod),
            texture_data_type(),
            true,
        ));
    }
    Schema::new(fields)
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
        for col in ["diffuseColor", "specularColor", "emissiveColor"] {
            assert!(
                matches!(
                    m.field_with_name(col).unwrap().data_type(),
                    DataType::List(_)
                ),
                "materials.{col} must be LIST<DOUBLE>, got {:?}",
                m.field_with_name(col).unwrap().data_type()
            );
        }

        let t = textures_schema();
        for col in [
            "id",
            "image_uri",
            "image_data",
            "image_type",
            "wrapMode",
            "textureType",
            "borderColor",
            "other",
        ] {
            assert!(t.field_with_name(col).is_ok(), "textures missing {col}");
        }
        // The spec renamed this from `mime_type` to `image_type` (not a MIME
        // type — a format token verbatim from the source).
        assert!(
            t.field_with_name("mime_type").is_err(),
            "textures schema must not carry the old `mime_type` column name"
        );
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
        assert!(
            matches!(
                t.field_with_name("borderColor").unwrap().data_type(),
                DataType::List(_)
            ),
            "textures.borderColor must be LIST<DOUBLE>"
        );

        let g = geometry_templates_schema(&[Lod::parse("2.2").unwrap()]);
        for col in [
            "id",
            "name",
            "geometry_lod2_2",
            "geometry_properties_lod2_2",
            "material_lod2_2",
            "texture_lod2_2",
        ] {
            assert!(g.field_with_name(col).is_ok(), "templates missing {col}");
        }
        // Spec "geometry_templates.parquet": `id BIGINT` required, `name
        // VARCHAR` optional — the same id/name pair as materials and
        // textures, so all three sidecars remap identically on merge.
        assert_eq!(
            g.field_with_name("id").unwrap().data_type(),
            &DataType::Int64,
            "templates.id must be BIGINT so it can be offset-shifted on merge like the \
             other sidecars, and so it matches the object table's template.id"
        );
        assert!(!g.field_with_name("id").unwrap().is_nullable());
        assert_eq!(
            g.field_with_name("name").unwrap().data_type(),
            &DataType::Utf8
        );
        assert!(
            g.field_with_name("name").unwrap().is_nullable(),
            "templates.name is optional — CityJSON templates are array entries with no \
             identifier of their own"
        );
        for col in [
            "geometry",
            "geometry_properties",
            "material",
            "texture",
            "lod",
            "other",
        ] {
            assert!(
                g.field_with_name(col).is_err(),
                "templates schema must not carry an un-suffixed/lod/other column '{col}'"
            );
        }
        assert_eq!(
            g.field_with_name("geometry_lod2_2").unwrap().data_type(),
            &DataType::Binary
        );
        assert!(matches!(
            g.field_with_name("geometry_properties_lod2_2")
                .unwrap()
                .data_type(),
            DataType::Struct(_)
        ));

        // A different LoD set renders a different (disjoint) column set —
        // the schema is genuinely a function of `lods`, not a fixed shape.
        let g2 = geometry_templates_schema(&[Lod::parse("1").unwrap(), Lod::parse("0").unwrap()]);
        for col in [
            "geometry_lod0_0",
            "geometry_lod1_0",
            "geometry_properties_lod0_0",
            "geometry_properties_lod1_0",
            "material_lod0_0",
            "material_lod1_0",
            "texture_lod0_0",
            "texture_lod1_0",
        ] {
            assert!(g2.field_with_name(col).is_ok(), "templates missing {col}");
        }
        assert!(g2.field_with_name("geometry_lod2_2").is_err());
    }
}
