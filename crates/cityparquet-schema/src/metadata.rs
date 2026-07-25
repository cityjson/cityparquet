//! Dataset-level metadata: the spec's two Parquet footer key-value JSON
//! objects (`city`, `geo`) as typed structs (`documents/docs/03-specification/05-metadata.mdx`).
//!
//! The footer carries exactly one `city` key (this file's own `CityMetadata`,
//! nested JSON) and, conditionally, one `geo` key (pure GeoParquet 1.1,
//! [`GeoMetadata`]) — no flat scalar keys, no `cityparquet:`-namespaced field
//! inside `geo`. Both objects describe **the file they live in** — nothing
//! wider (a per-module table's `city`/`geo` legitimately differs from its
//! siblings').

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CityParquetError, Result};
use crate::types::GeometryEncoding;

/// CityParquet format version this crate writes (`city.version`) — the
/// spec's stated draft version (`01-dataset-package.mdx` "Versioning").
pub const CITYPARQUET_VERSION: &str = "0.1.0-draft";

/// GeoParquet spec version this crate's `geo` object implements.
pub const GEOPARQUET_VERSION: &str = "1.1.0";

/// `city.source_format` (spec §metadata): one of the three named source
/// tokens, or an open-ended other-source string this document doesn't
/// enumerate (e.g. `"3DCityDB"`). Optional at the `CityMetadata` level: a
/// table authored natively, with no single source, omits the field entirely
/// rather than writing a placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFormat {
    CityJson,
    CityJsonSeq,
    CityGml,
    /// An other-source token this document doesn't name, e.g. `"3DCityDB"`.
    Other(String),
}

impl SourceFormat {
    fn as_str(&self) -> &str {
        match self {
            SourceFormat::CityJson => "CityJSON",
            SourceFormat::CityJsonSeq => "CityJSONSeq",
            SourceFormat::CityGml => "CityGML",
            SourceFormat::Other(s) => s.as_str(),
        }
    }
}

impl Serialize for SourceFormat {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SourceFormat {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "CityJSON" => SourceFormat::CityJson,
            "CityJSONSeq" => SourceFormat::CityJsonSeq,
            "CityGML" => SourceFormat::CityGml,
            _ => SourceFormat::Other(s),
        })
    }
}

/// 3D surface winding a `city.columns` entry declares (spec
/// `city.columns[].orientation_3d`) — **always** stated explicitly by a
/// conforming writer, never relying on an absent-field default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation3d {
    RightHanded,
    LeftHanded,
}

/// One `city.columns` entry: describes one geometry column, `Solid`-family
/// included (unlike `geo.columns`, which only ever carries the GeoParquet-legal
/// subset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityColumnEntry {
    pub name: String,
    /// `"WKB"` for a normative [`GeometryEncoding::Wkb`] column, or
    /// `"CityParquetArrowNative-v1"` for the experimental
    /// [`GeometryEncoding::ArrowNative`] nested-Arrow encoding — see
    /// [`CityColumnEntry::new`].
    pub encoding: String,
    pub geometry_types: Vec<String>,
    /// PROJJSON; defaults to the file-level `city.crs` when absent.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub crs: Option<Value>,
    pub orientation_3d: Orientation3d,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edges: Option<String>,
    /// Dataset-level bounding box of the column's geometries.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bbox: Option<Value>,
    /// Coordinate epoch (decimal year) for a dynamic CRS, when relevant.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub epoch: Option<f64>,
}

impl CityColumnEntry {
    /// A column entry with every optional field absent — the writer
    /// currently only ever produces right-handed winding. `encoding` records
    /// the REAL physical encoding the column was rendered under (spec
    /// "the footer describes the file it lives in"): `Wkb` ->
    /// `"WKB"`, `ArrowNative` -> `"CityParquetArrowNative-v1"`.
    pub fn new(
        name: impl Into<String>,
        geometry_types: Vec<String>,
        encoding: GeometryEncoding,
    ) -> Self {
        Self {
            name: name.into(),
            encoding: match encoding {
                GeometryEncoding::Wkb => "WKB",
                GeometryEncoding::ArrowNative => "CityParquetArrowNative-v1",
            }
            .to_string(),
            geometry_types,
            crs: None,
            orientation_3d: Orientation3d::RightHanded,
            edges: None,
            bbox: None,
            epoch: None,
        }
    }
}

/// The `city` Parquet key-value metadata object (spec §metadata "The `city`
/// object") — CityParquet's own metadata: version, provenance, CRS, the
/// geometry-column registry, the attribute list, and extensions. Describes
/// **the file it lives in**, not the whole dataset: a by-module package's
/// `building.parquet` and `transportation.parquet` each carry their own,
/// genuinely different, `CityMetadata`.
///
/// Requirements depend on the file's role: an object table carries
/// `source_format`/`attributes`/`primary_column`/`columns` when it has the
/// data to back them; a sidecar (`materials.parquet`, `textures.parquet`,
/// `geometry_templates.parquet`) carries only `version` plus `crs` when it has
/// CRS-bearing coordinates — none of the object-table-only fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityMetadata {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_format: Option<SourceFormat>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_version: Option<String>,
    /// The file CRS as PROJJSON — present whenever the file holds any
    /// CRS-bearing coordinate (object geometry, an address `location`, a
    /// `bbox`, or a geometry-template instance's `point`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub crs: Option<Value>,
    /// Name of the primary geometry column — the one a reader uses with no
    /// LoD preference. MAY name a `Solid` column; independent of
    /// `GeoMetadata::primary_column`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub primary_column: Option<String>,
    /// One entry per geometry column — **every** geometry column,
    /// `Solid`-family included.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub columns: Vec<CityColumnEntry>,
    /// The list of inferred attribute columns (object tables). Every other
    /// column is a reserved structural column.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attributes: Vec<String>,
    /// Extension / ADE declarations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extensions: Option<Value>,
    /// Source default material / texture theme, when present.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub appearance_defaults: Option<Value>,
    /// Free-form object for anything not covered above — the source
    /// `transform`, the source's own dataset-level metadata header, or any
    /// producer-defined field. **Informational only:** a reader MUST NOT
    /// need this to decode the file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub other: Option<Value>,
}

impl CityMetadata {
    /// A minimal `CityMetadata` carrying only `version` — the shape every
    /// sidecar and empty-schema table starts from.
    pub fn new() -> Self {
        Self {
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

    /// Serialise to the footer's `city` key (and, when `geo` is `Some`, the
    /// `geo` key too) — one JSON-valued key-value pair each, per file.
    pub fn to_key_values(&self, geo: Option<&GeoMetadata>) -> Result<Vec<(String, String)>> {
        let mut kvs = vec![("city".to_string(), serde_json::to_string(self)?)];
        if let Some(geo) = geo {
            kvs.push(("geo".to_string(), serde_json::to_string(geo)?));
        }
        Ok(kvs)
    }

    /// Parse the footer's `city` (required) and `geo` (conditional) keys back
    /// into their typed forms. Any other key is ignored — the plain-string-
    /// vs-JSON heuristic this replaces is gone: there is exactly one
    /// JSON-valued `city` key (plus, conditionally, one JSON-valued `geo`
    /// key) to look for.
    pub fn from_key_values<'a>(
        kvs: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Result<(Self, Option<GeoMetadata>)> {
        let mut city: Option<&str> = None;
        let mut geo: Option<&str> = None;
        for (key, value) in kvs {
            match key {
                "city" => city = Some(value),
                "geo" => geo = Some(value),
                _ => {}
            }
        }
        let city =
            city.ok_or_else(|| CityParquetError::Metadata("missing key city".to_string()))?;
        let city: CityMetadata = serde_json::from_str(city)?;
        let geo = geo.map(serde_json::from_str::<GeoMetadata>).transpose()?;
        Ok((city, geo))
    }
}

impl Default for CityMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// One `geo.columns` entry (spec §metadata "The `geo` object") — pure
/// GeoParquet 1.1, no CityParquet extension fields (3D winding lives only in
/// `CityColumnEntry::orientation_3d`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoColumnEntry {
    pub encoding: String,
    pub geometry_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub crs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edges: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bbox: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub epoch: Option<f64>,
}

/// The `geo` Parquet key-value metadata object: a GeoParquet 1.1-conformant
/// object so GeoParquet-aware tools recognise the geometry columns they can
/// read. Restricted to GeoParquet-legal columns — a column carrying any
/// solid-family WKB type MUST NOT appear here (`CityColumnEntry` is where
/// it's described). Omitted entirely (no `geo` key at all) when zero columns
/// qualify (a solid-only table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoMetadata {
    pub version: String,
    /// The geometry column a GeoParquet reader uses by default; MUST be one
    /// of the declared (legal) `columns`. Independent of
    /// `CityMetadata::primary_column`.
    pub primary_column: String,
    /// Map from a legal geometry column name to its GeoParquet column
    /// metadata. `BTreeMap` for deterministic (name-sorted) serialisation.
    pub columns: std::collections::BTreeMap<String, GeoColumnEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_city() -> CityMetadata {
        CityMetadata {
            version: CITYPARQUET_VERSION.to_string(),
            source_format: Some(SourceFormat::CityJsonSeq),
            source_version: Some("2.0".to_string()),
            crs: Some(json!({"type": "ProjectedCRS", "id": {"authority": "EPSG", "code": 28992}})),
            primary_column: Some("geometry_lod2_2".to_string()),
            columns: vec![CityColumnEntry::new(
                "geometry_lod2_2",
                vec!["MultiPolygon Z".to_string()],
                GeometryEncoding::Wkb,
            )],
            attributes: vec!["yoc".to_string(), "height".to_string()],
            extensions: None,
            appearance_defaults: Some(json!({"default-theme-material": "t"})),
            other: Some(json!({"transform": {"scale": [0.001, 0.001, 0.001]}})),
        }
    }

    fn sample_geo() -> GeoMetadata {
        let mut columns = std::collections::BTreeMap::new();
        columns.insert(
            "geometry_lod2_2".to_string(),
            GeoColumnEntry {
                encoding: "WKB".to_string(),
                geometry_types: vec!["MultiPolygon Z".to_string()],
                crs: sample_city().crs,
                edges: Some("planar".to_string()),
                bbox: None,
                epoch: None,
            },
        );
        GeoMetadata {
            version: GEOPARQUET_VERSION.to_string(),
            primary_column: "geometry_lod2_2".to_string(),
            columns,
        }
    }

    /// RED (spec-alignment M3, gap 16): the footer has exactly one `city` key
    /// holding the whole nested object, and no flat scalar keys survive.
    #[test]
    fn to_key_values_writes_exactly_one_city_key_when_geo_is_none() {
        let kvs = sample_city().to_key_values(None).unwrap();
        assert_eq!(kvs.len(), 1);
        assert_eq!(kvs[0].0, "city");
        let parsed: Value = serde_json::from_str(&kvs[0].1).unwrap();
        assert_eq!(parsed["version"], CITYPARQUET_VERSION);
        assert_eq!(parsed["source_format"], "CityJSONSeq");
        assert!(parsed.get("cityparquet_version").is_none());
        assert!(parsed.get("default_geometry").is_none());
        assert!(parsed.get("bbox_column").is_none());
        assert!(parsed.get("sidecar_files").is_none());
    }

    #[test]
    fn to_key_values_adds_one_geo_key_when_given() {
        let kvs = sample_city().to_key_values(Some(&sample_geo())).unwrap();
        let keys: Vec<&str> = kvs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["city", "geo"]);
        let geo: Value = serde_json::from_str(&kvs[1].1).unwrap();
        assert_eq!(geo["version"], GEOPARQUET_VERSION);
        assert_eq!(geo["primary_column"], "geometry_lod2_2");
        assert!(
            geo.get("cityparquet:orientation").is_none(),
            "geo must carry no cityparquet:-namespaced field"
        );
        assert!(
            geo["columns"]["geometry_lod2_2"]
                .as_object()
                .unwrap()
                .get("orientation_3d")
                .is_none(),
            "orientation_3d lives only in city.columns"
        );
    }

    #[test]
    fn round_trips_through_key_values() {
        let city = sample_city();
        let geo = sample_geo();
        let kvs = city.to_key_values(Some(&geo)).unwrap();
        let (back_city, back_geo) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(back_city, city);
        assert_eq!(back_geo, Some(geo));
    }

    #[test]
    fn from_key_values_without_geo_key_returns_none() {
        let city = sample_city();
        let kvs = city.to_key_values(None).unwrap();
        let (_back, geo) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert!(geo.is_none());
    }

    #[test]
    fn missing_city_key_is_an_error() {
        let err = CityMetadata::from_key_values(std::iter::empty());
        assert!(err.is_err());
    }

    #[test]
    fn orientation_3d_is_always_explicit_right_handed_by_default() {
        let entry = CityColumnEntry::new(
            "geometry_lod2_2",
            vec!["MultiPolygon Z".to_string()],
            GeometryEncoding::Wkb,
        );
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value["orientation_3d"], "right-handed");
    }

    /// RED (this plan's Task 2, step 4b): `CityColumnEntry::new` must record
    /// the REAL encoding a column was rendered under, not silently hardcode
    /// `"WKB"` regardless of caller — the footer must agree with the
    /// physical Arrow schema Task 1 threads `GeometryEncoding` through
    /// (`CityParquetSchema::to_arrow_schema_tagged`).
    #[test]
    fn city_column_entry_records_the_real_encoding_not_always_wkb() {
        let entry = CityColumnEntry::new(
            "geometry_lod2_2".to_string(),
            vec!["Solid".to_string()],
            GeometryEncoding::ArrowNative,
        );
        assert_eq!(entry.encoding, "CityParquetArrowNative-v1");

        let wkb_entry = CityColumnEntry::new(
            "geometry_lod2_2".to_string(),
            vec!["MultiPolygon Z".to_string()],
            GeometryEncoding::Wkb,
        );
        assert_eq!(wkb_entry.encoding, "WKB");
    }

    /// `source_format` is open-ended: an unrecognised string round-trips as
    /// `Other`, never rejected.
    #[test]
    fn source_format_other_round_trips() {
        let mut city = sample_city();
        city.source_format = Some(SourceFormat::Other("3DCityDB".to_string()));
        let kvs = city.to_key_values(None).unwrap();
        let (back, _) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(
            back.source_format,
            Some(SourceFormat::Other("3DCityDB".to_string()))
        );
    }

    /// `source_format` is optional: a table authored natively omits it
    /// entirely, not as an `"Other"` placeholder.
    #[test]
    fn source_format_absent_is_omitted_not_placeholdered() {
        let mut city = sample_city();
        city.source_format = None;
        let kvs = city.to_key_values(None).unwrap();
        let parsed: Value = serde_json::from_str(&kvs[0].1).unwrap();
        assert!(parsed.get("source_format").is_none());
    }
}
