//! What one batch of top-level features reads from the database: the
//! features they contain, their properties, their geometries, the
//! appearances mapped onto those geometries, their addresses, and the
//! features their properties refer to.

use std::collections::{HashMap, HashSet};

use citygml_core::model::Address;
use citygml_core::{Error, Result};
use serde_json::Value as Json;

use crate::db::Session;

/// A timestamp as CityGML writes it: a date when it falls on midnight UTC
/// (CityGML's `xs:date` properties, which 3DCityDB stores as timestamps),
/// else a date and time in UTC.
fn timestamp(column: &str) -> String {
    format!(
        "CASE WHEN {column} IS NULL THEN NULL \
         WHEN ({column} AT TIME ZONE 'UTC')::time = '00:00' \
         THEN to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD') \
         ELSE to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') END"
    )
}

/// A date and time in UTC, as CityGML's `xs:dateTime` properties
/// (`creationDate`) are written.
fn date_time(column: &str) -> String {
    format!("to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')")
}

/// One row of `feature`.
pub(crate) struct FeatureRow {
    pub(crate) class: i32,
    pub(crate) objectid: Option<String>,
    pub(crate) envelope: Option<[f64; 6]>,
    pub(crate) creation_date: Option<String>,
    pub(crate) valid_from: Option<String>,
    pub(crate) valid_to: Option<String>,
}

/// One row of `property`.
pub(crate) struct PropertyRow {
    pub(crate) id: i64,
    pub(crate) datatype: i32,
    pub(crate) namespace: Option<i32>,
    pub(crate) name: String,
    pub(crate) int: Option<i64>,
    pub(crate) double: Option<f64>,
    pub(crate) string: Option<String>,
    pub(crate) timestamp: Option<String>,
    pub(crate) uri: Option<String>,
    pub(crate) array: Option<Json>,
    pub(crate) lod: Option<String>,
    pub(crate) geometry: Option<i64>,
    pub(crate) implicit: Option<i64>,
    pub(crate) reference_point: Option<[f64; 3]>,
    pub(crate) address: Option<i64>,
    pub(crate) feature: Option<i64>,
    pub(crate) relation: Option<i32>,
}

/// One row of `geometry_data`, as WKB and its GML structure.
pub(crate) struct GeometryRow {
    pub(crate) wkb: Vec<u8>,
    pub(crate) properties: Json,
}

/// One appearance's surface data mapped onto one geometry.
pub(crate) struct MappingRow {
    pub(crate) surface_data: i64,
    /// `{polygon id: true}`: the polygons a material colours.
    pub(crate) material: Option<Json>,
    /// `{polygon id: [[[u, v], …] per ring]}`, each ring closed.
    pub(crate) texture: Option<Json>,
    pub(crate) theme: String,
}

/// A `surface_data` row: an `X3DMaterial`, a `ParameterizedTexture` or a
/// `GeoreferencedTexture`.
pub(crate) struct SurfaceData {
    pub(crate) class: String,
    pub(crate) objectid: Option<String>,
    pub(crate) shininess: Option<f64>,
    pub(crate) transparency: Option<f64>,
    pub(crate) ambient_intensity: Option<f64>,
    pub(crate) specular_color: Option<String>,
    pub(crate) diffuse_color: Option<String>,
    pub(crate) emissive_color: Option<String>,
    pub(crate) is_smooth: Option<i32>,
    pub(crate) texture_type: Option<String>,
    pub(crate) wrap_mode: Option<String>,
    pub(crate) border_color: Option<String>,
    pub(crate) image_uri: Option<String>,
    pub(crate) mime_type: Option<String>,
}

/// Everything one batch reads.
#[derive(Default)]
pub(crate) struct Batch {
    pub(crate) features: HashMap<i64, FeatureRow>,
    /// A feature's top-level properties, in id order.
    pub(crate) properties: HashMap<i64, Vec<PropertyRow>>,
    /// A property's child properties, in id order.
    pub(crate) children: HashMap<i64, Vec<PropertyRow>>,
    pub(crate) geometries: HashMap<i64, GeometryRow>,
    pub(crate) mappings: HashMap<i64, Vec<MappingRow>>,
    pub(crate) surface_data: HashMap<i64, SurfaceData>,
    pub(crate) addresses: HashMap<i64, Address>,
    /// The `objectid` of each feature a property refers to without
    /// containing it (a group's member).
    pub(crate) referenced: HashMap<i64, Option<String>>,
}

impl Batch {
    /// The polygons of geometry `geometry` that an appearance gives texture
    /// coordinates.
    pub(crate) fn textured(&self, geometry: i64) -> HashSet<String> {
        textured(self.mappings.get(&geometry).map_or(&[][..], Vec::as_slice))
    }
}

/// The polygons `mappings` give texture coordinates.
pub(crate) fn textured(mappings: &[MappingRow]) -> HashSet<String> {
    mappings
        .iter()
        .filter_map(|m| m.texture.as_ref()?.as_object())
        .flat_map(|o| o.keys().cloned())
        .collect()
}

/// The features `roots` contain, at any depth, and the roots themselves,
/// each once: a feature contained twice is still one feature.
const TREE: &str = "WITH RECURSIVE tree(id) AS ( \
        SELECT unnest($1::bigint[]) \
        UNION \
        SELECT p.val_feature_id FROM {s}.property p JOIN tree t ON p.feature_id = t.id \
        WHERE p.val_relation_type = 1 AND p.val_feature_id IS NOT NULL) ";

pub(crate) fn load(session: &mut Session, roots: &[i64]) -> Result<Batch> {
    let mut batch = Batch::default();
    let sql = format!(
        "{TREE} SELECT f.id, f.objectclass_id, f.objectid, \
                ST_XMin(f.envelope), ST_YMin(f.envelope), ST_ZMin(f.envelope), \
                ST_XMax(f.envelope), ST_YMax(f.envelope), ST_ZMax(f.envelope), \
                {}, {}, {} \
         FROM tree JOIN {{s}}.feature f ON f.id = tree.id \
         WHERE f.termination_date IS NULL",
        date_time("f.creation_date"),
        date_time("f.valid_from"),
        date_time("f.valid_to"),
    );
    for row in session.query(&sql, &[&roots])? {
        let id: i64 = row.get(0);
        let envelope: Option<Vec<f64>> = (3..9).map(|i| row.get::<_, Option<f64>>(i)).collect();
        batch.features.insert(
            id,
            FeatureRow {
                class: row.get(1),
                objectid: row.get(2),
                envelope: envelope.map(|e| [e[0], e[1], e[2], e[3], e[4], e[5]]),
                creation_date: row.get(9),
                valid_from: row.get(10),
                valid_to: row.get(11),
            },
        );
    }
    let ids: Vec<i64> = batch.features.keys().copied().collect();

    let sql = format!(
        "SELECT id, feature_id, parent_id, datatype_id, namespace_id, name, \
                val_int, val_double, val_string, {}, val_uri, val_array::text, val_lod, \
                val_geometry_id, val_implicitgeom_id, \
                ST_X(val_implicitgeom_refpoint), ST_Y(val_implicitgeom_refpoint), \
                ST_Z(val_implicitgeom_refpoint), \
                val_address_id, val_feature_id, val_relation_type \
         FROM {{s}}.property WHERE feature_id = ANY($1) ORDER BY id",
        timestamp("val_timestamp")
    );
    for row in session.query(&sql, &[&ids])? {
        let feature: i64 = row.get(1);
        let parent: Option<i64> = row.get(2);
        let array: Option<String> = row.get(11);
        let point: Option<Vec<f64>> = (15..18).map(|i| row.get::<_, Option<f64>>(i)).collect();
        let property = PropertyRow {
            id: row.get(0),
            datatype: row.get(3),
            namespace: row.get(4),
            name: row.get::<_, Option<String>>(5).unwrap_or_default(),
            int: row.get(6),
            double: row.get(7),
            string: row.get(8),
            timestamp: row.get(9),
            uri: row.get(10),
            array: array.map(|a| parse_json(&a)).transpose()?,
            lod: row.get(12),
            geometry: row.get(13),
            implicit: row.get(14),
            reference_point: point.map(|p| [p[0], p[1], p[2]]),
            address: row.get(18),
            feature: row.get(19),
            relation: row.get(20),
        };
        match parent {
            Some(parent) => batch.children.entry(parent).or_default().push(property),
            None => batch.properties.entry(feature).or_default().push(property),
        }
    }

    let all = || {
        batch
            .properties
            .values()
            .chain(batch.children.values())
            .flatten()
    };
    let geometry_ids: Vec<i64> = all().filter_map(|p| p.geometry).collect();
    let address_ids: Vec<i64> = all().filter_map(|p| p.address).collect();
    let referenced: Vec<i64> = all()
        .filter(|p| p.relation != Some(1))
        .filter_map(|p| p.feature)
        .collect();

    for row in session.query(
        "SELECT id, ST_AsBinary(geometry), geometry_properties::text \
         FROM {s}.geometry_data WHERE id = ANY($1) AND geometry IS NOT NULL",
        &[&geometry_ids],
    )? {
        let properties: Option<String> = row.get(2);
        batch.geometries.insert(
            row.get(0),
            GeometryRow {
                wkb: row.get(1),
                properties: properties
                    .map(|p| parse_json(&p))
                    .transpose()?
                    .unwrap_or(Json::Null),
            },
        );
    }
    load_mappings(
        session,
        &geometry_ids,
        &mut batch.mappings,
        &mut batch.surface_data,
    )?;

    for row in session.query(
        "SELECT id, street, house_number, po_box, zip_code, city, state, country, \
                free_text::text, ST_AsBinary(multi_point) \
         FROM {s}.address WHERE id = ANY($1)",
        &[&address_ids],
    )? {
        batch.addresses.insert(row.get(0), address(&row)?);
    }

    for row in session.query(
        "SELECT id, objectid FROM {s}.feature WHERE id = ANY($1)",
        &[&referenced],
    )? {
        batch.referenced.insert(row.get(0), row.get(1));
    }
    Ok(batch)
}

/// The appearances mapped onto `geometries`, by geometry, in appearance
/// order, and the surface data they map.
pub(crate) fn load_mappings(
    session: &mut Session,
    geometries: &[i64],
    mappings: &mut HashMap<i64, Vec<MappingRow>>,
    surface_data: &mut HashMap<i64, SurfaceData>,
) -> Result<()> {
    for row in session.query(
        "SELECT m.geometry_data_id, m.surface_data_id, m.material_mapping::text, \
                m.texture_mapping::text, coalesce(a.theme, ''), oc.classname, sd.objectid, \
                sd.x3d_shininess, sd.x3d_transparency, sd.x3d_ambient_intensity, \
                sd.x3d_specular_color, sd.x3d_diffuse_color, sd.x3d_emissive_color, \
                sd.x3d_is_smooth, sd.tex_texture_type, sd.tex_wrap_mode, sd.tex_border_color, \
                ti.image_uri, ti.mime_type \
         FROM {s}.surface_data_mapping m \
         JOIN {s}.surface_data sd ON sd.id = m.surface_data_id \
         JOIN {s}.objectclass oc ON oc.id = sd.objectclass_id \
         JOIN {s}.appear_to_surface_data ats ON ats.surface_data_id = sd.id \
         JOIN {s}.appearance a ON a.id = ats.appearance_id \
         LEFT JOIN {s}.tex_image ti ON ti.id = sd.tex_image_id \
         WHERE m.geometry_data_id = ANY($1) \
         ORDER BY a.id, ats.id, m.geometry_data_id",
        &[&geometries],
    )? {
        let surface: i64 = row.get(1);
        let material: Option<String> = row.get(2);
        let texture: Option<String> = row.get(3);
        mappings.entry(row.get(0)).or_default().push(MappingRow {
            surface_data: surface,
            material: material.map(|m| parse_json(&m)).transpose()?,
            texture: texture.map(|t| parse_json(&t)).transpose()?,
            theme: row.get(4),
        });
        surface_data.entry(surface).or_insert_with(|| SurfaceData {
            class: row.get::<_, Option<String>>(5).unwrap_or_default(),
            objectid: row.get(6),
            shininess: row.get(7),
            transparency: row.get(8),
            ambient_intensity: row.get(9),
            specular_color: row.get(10),
            diffuse_color: row.get(11),
            emissive_color: row.get(12),
            is_smooth: row.get(13),
            texture_type: row.get(14),
            wrap_mode: row.get(15),
            border_color: row.get(16),
            image_uri: row.get(17),
            mime_type: row.get(18),
        });
    }
    Ok(())
}

/// An `address` row as the xAL leaves the CityGML readers name: xAL 2's
/// names for the parts CityJSON's address conversion knows.
fn address(row: &postgres::Row) -> Result<Address> {
    let mut address = Address::default();
    let columns = [
        (1, "ThoroughfareName"),
        (2, "ThoroughfareNumber"),
        (3, "PostBoxNumber"),
        (4, "PostalCodeNumber"),
        (5, "LocalityName"),
        (6, "AdministrativeAreaName"),
        (7, "CountryName"),
    ];
    for (column, name) in columns {
        if let Some(value) = row.get::<_, Option<String>>(column) {
            address.fields.push((name.to_string(), value));
        }
    }
    if let Some(free) = row.get::<_, Option<String>>(8) {
        // Free-text address lines, a JSON array of strings.
        match parse_json(&free)? {
            Json::Array(lines) => {
                for line in lines {
                    if let Json::String(line) = line {
                        address.fields.push(("AddressLine".to_string(), line));
                    }
                }
            }
            Json::String(line) => address.fields.push(("AddressLine".to_string(), line)),
            _ => {}
        }
    }
    if let Some(points) = row.get::<_, Option<Vec<u8>>>(9) {
        for primitive in crate::wkb::primitives(&points)? {
            if let crate::wkb::Primitive::Point(p) = primitive {
                address.location.push(p);
            }
        }
    }
    Ok(address)
}

fn parse_json(text: &str) -> Result<Json> {
    serde_json::from_str(text).map_err(|e| Error::invalid(format!("unreadable JSON column: {e}")))
}
