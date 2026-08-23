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

/// The **tri-state** `crs` key (spec §metadata "CRS rules"), shared by
/// `city.crs`, `city.columns[].crs` and `geo.columns[].crs` — exactly
/// GeoParquet's own convention, which is why one type serves both objects:
///
/// - [`CrsState::Known`] — the key holds a **PROJJSON** object.
/// - [`CrsState::Unknown`] — the key is present and **`null`**: the file holds
///   CRS-bearing coordinates whose CRS is unknown or unresolvable. A reader
///   treats them as bare Cartesian values; export carries no reference system.
/// - [`CrsState::Unspecified`] — the key is **absent**. Per GeoParquet an
///   absent `crs` is read as OGC:CRS84, so a conforming writer never relies on
///   it: absence is legitimate **only** for a file with no CRS-bearing
///   coordinate at all (the `geometry_templates.parquet` sidecar, whose
///   templates are unplaced local coordinates, and the attributes-only object
///   table).
///
/// `Option<Value>` cannot express this: it collapses "absent" and "null" onto
/// the same `None`, and a writer that omitted the key where it meant `null`
/// would silently assert CRS84 over a projected national city model. The three
/// states are therefore a type of their own, with hand-rolled serde (no
/// `serde_with` double-option dependency) so the distinction survives a footer
/// round trip byte-for-byte.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum CrsState {
    /// The key is absent from the object entirely.
    #[default]
    Unspecified,
    /// The key is present and `null` — CRS unknown or unresolvable.
    Unknown,
    /// The key is present and holds PROJJSON.
    Known(Value),
}

impl CrsState {
    /// The PROJJSON, when the CRS is known — the accessor every consumer that
    /// only cares about a *usable* CRS (export's `referenceSystem`, the
    /// CityGML writer's `srsName`, the STAC proj extension, the `geoarrow.wkb`
    /// field extension) reads, so `Unknown` and `Unspecified` alike degrade to
    /// "no CRS to state" without any of them having to know the difference.
    pub fn known(&self) -> Option<&Value> {
        match self {
            CrsState::Known(value) => Some(value),
            _ => None,
        }
    }

    /// Whether the key is absent (the serde `skip_serializing_if` predicate).
    pub fn is_unspecified(&self) -> bool {
        matches!(self, CrsState::Unspecified)
    }

    /// Whether the key is an explicit `null` — CRS unknown/unresolvable.
    pub fn is_unknown(&self) -> bool {
        matches!(self, CrsState::Unknown)
    }

    /// Whether the key holds PROJJSON.
    pub fn is_known(&self) -> bool {
        matches!(self, CrsState::Known(_))
    }

    /// The spec's writer rule as code: a CRS the writer resolved to PROJJSON
    /// is [`CrsState::Known`]; otherwise the key is an explicit `null`
    /// whenever the file holds **any** CRS-bearing coordinate (object
    /// geometry, an address `location`, a `bbox`, a geometry-template
    /// instance's `point`), and absent only when it holds none.
    ///
    /// "A writer never relies on the absent-CRS default": the
    /// `has_crs_bearing_coordinate` argument is the whole of that rule, so no
    /// caller can accidentally omit the key over data that needs
    /// georeferencing.
    pub fn from_resolution(resolved: Option<Value>, has_crs_bearing_coordinate: bool) -> Self {
        match resolved {
            Some(projjson) => CrsState::Known(projjson),
            None if has_crs_bearing_coordinate => CrsState::Unknown,
            None => CrsState::Unspecified,
        }
    }
}

impl Serialize for CrsState {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            CrsState::Known(value) => value.serialize(serializer),
            // `Unspecified` never reaches a serialiser through the footer
            // objects (every `crs` field carries
            // `skip_serializing_if = "CrsState::is_unspecified"`); serialising
            // the value standalone renders it as the `null` it is closest to,
            // since a self-describing format has no way to spell "absent".
            CrsState::Unknown | CrsState::Unspecified => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for CrsState {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // A present key: `null` -> `Unknown`, anything else -> `Known`. An
        // ABSENT key never reaches here at all — the field's `#[serde(default)]`
        // supplies `Unspecified` instead, which is precisely how the third
        // state is recovered.
        Ok(match Option::<Value>::deserialize(deserializer)? {
            None | Some(Value::Null) => CrsState::Unknown,
            Some(value) => CrsState::Known(value),
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
    /// Tri-state, mirroring the file-level `city.crs` ([`CrsState`]).
    /// [`CrsState::Unspecified`] — this writer's own choice for every entry —
    /// means "defaults to the file-level `city.crs`", a sibling in the SAME
    /// object, so a CityParquet reader never needs it repeated.
    #[serde(skip_serializing_if = "CrsState::is_unspecified", default)]
    pub crs: CrsState,
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
            // The SAME token vocabulary a reader resolves the encoding back
            // out of (`GeometryEncoding::from_footer_token`) — one source of
            // truth, so writer and reader can never drift apart on spelling.
            encoding: encoding.footer_token().to_string(),
            geometry_types,
            crs: CrsState::Unspecified,
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
    /// The file CRS, **tri-state** ([`CrsState`]): PROJJSON when known, an
    /// explicit `null` when the file holds CRS-bearing coordinates (object
    /// geometry, an address `location`, a `bbox`, a geometry-template
    /// instance's `point`) whose CRS is unknown or unresolvable, and absent
    /// only when the file holds no CRS-bearing coordinate at all.
    #[serde(skip_serializing_if = "CrsState::is_unspecified", default)]
    pub crs: CrsState,
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
            crs: CrsState::Unspecified,
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
    /// Tri-state, exactly as GeoParquet defines it ([`CrsState`]) and always
    /// mirroring the file-level `city.crs` — **including** a `null`, which is
    /// GeoParquet-legal and means "unknown". A GeoParquet-only consumer cannot
    /// see the foreign `city` key, so this must be stated explicitly rather
    /// than left absent (absence would assert OGC:CRS84 over a projected city
    /// model).
    #[serde(skip_serializing_if = "CrsState::is_unspecified", default)]
    pub crs: CrsState,
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
            crs: CrsState::Known(
                json!({"type": "ProjectedCRS", "id": {"authority": "EPSG", "code": 28992}}),
            ),
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

    /// The serialised `city` object, as a raw `serde_json::Map` — so a test
    /// can tell an ABSENT key from one present and `null`, which
    /// `Value::get(...).is_none()` alone cannot (it answers `None` for both).
    fn city_members(city: &CityMetadata) -> serde_json::Map<String, Value> {
        let kvs = city.to_key_values(None).unwrap();
        match serde_json::from_str::<Value>(&kvs[0].1).unwrap() {
            Value::Object(map) => map,
            other => panic!("city must serialise to an object, got {other}"),
        }
    }

    /// RED (spec §metadata "CRS rules"): `city.crs` is tri-state and all three
    /// states must survive a footer round trip. `Option<Value>` collapsed
    /// "absent" and "null" onto one `None`, so a writer that meant `null`
    /// silently emitted absence — which per GeoParquet asserts OGC:CRS84 over
    /// a projected national city model.
    #[test]
    fn crs_known_serialises_as_the_projjson_object_and_round_trips() {
        let city = sample_city();
        let members = city_members(&city);
        assert!(
            members["crs"].is_object(),
            "a known CRS is a PROJJSON object: {:?}",
            members["crs"]
        );
        assert_eq!(members["crs"]["id"]["code"], 28992);

        let kvs = city.to_key_values(None).unwrap();
        let (back, _) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(back.crs, city.crs);
        assert!(back.crs.is_known());
    }

    #[test]
    fn crs_unknown_serialises_as_an_explicit_null_and_round_trips() {
        let mut city = sample_city();
        city.crs = CrsState::Unknown;
        let members = city_members(&city);
        assert!(
            members.contains_key("crs"),
            "an unknown CRS is DECLARED, never omitted: {members:?}"
        );
        assert_eq!(
            members["crs"],
            Value::Null,
            "an unknown CRS is an explicit JSON null"
        );

        let kvs = city.to_key_values(None).unwrap();
        let (back, _) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(
            back.crs,
            CrsState::Unknown,
            "null must not decode as absent"
        );
    }

    #[test]
    fn crs_unspecified_stays_absent_and_round_trips() {
        let mut city = sample_city();
        city.crs = CrsState::Unspecified;
        let members = city_members(&city);
        assert!(
            !members.contains_key("crs"),
            "an unspecified CRS writes NO key at all: {members:?}"
        );

        let kvs = city.to_key_values(None).unwrap();
        let (back, _) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(
            back.crs,
            CrsState::Unspecified,
            "absent must not decode as null"
        );
    }

    /// The three states are genuinely distinguishable at the byte level —
    /// pinned as one assertion so a regression that collapses any two of them
    /// cannot pass by satisfying the individual tests above in isolation.
    #[test]
    fn the_three_crs_states_serialise_to_three_different_footers() {
        let known = sample_city();
        let mut unknown = sample_city();
        unknown.crs = CrsState::Unknown;
        let mut unspecified = sample_city();
        unspecified.crs = CrsState::Unspecified;

        let json = |c: &CityMetadata| c.to_key_values(None).unwrap()[0].1.clone();
        let (k, u, a) = (json(&known), json(&unknown), json(&unspecified));
        assert!(k != u && u != a && k != a, "\n{k}\n{u}\n{a}");
        assert!(u.contains("\"crs\":null"), "{u}");
        assert!(!a.contains("\"crs\""), "{a}");
    }

    /// `geo`'s mirror is GeoParquet's OWN tri-state, so a `null` there is
    /// legal and must be written explicitly: a GeoParquet-only consumer
    /// cannot fall back to the foreign `city` key, and an absent column `crs`
    /// means OGC:CRS84 to it.
    #[test]
    fn geo_column_crs_carries_the_explicit_null_too() {
        let mut geo = sample_geo();
        geo.columns.get_mut("geometry_lod2_2").unwrap().crs = CrsState::Unknown;
        let kvs = sample_city().to_key_values(Some(&geo)).unwrap();
        let parsed: Value = serde_json::from_str(&kvs[1].1).unwrap();
        let column = parsed["columns"]["geometry_lod2_2"].as_object().unwrap();
        assert!(column.contains_key("crs"), "{column:?}");
        assert_eq!(column["crs"], Value::Null);

        let (_, back) =
            CityMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(
            back.unwrap().columns["geometry_lod2_2"].crs,
            CrsState::Unknown
        );
    }

    /// The spec's writer rule, as the schema crate encodes it: resolved ->
    /// `Known`; unresolved WITH CRS-bearing coordinates -> `Unknown` (never
    /// absent, which would assert CRS84); unresolved WITHOUT them -> absent.
    #[test]
    fn from_resolution_implements_the_specs_writer_rule() {
        let projjson = json!({"type": "ProjectedCRS"});
        assert_eq!(
            CrsState::from_resolution(Some(projjson.clone()), true),
            CrsState::Known(projjson.clone())
        );
        // A resolved CRS is declared whether or not this file has coordinates
        // of its own (a partition's empty module table still shares the
        // dataset CRS).
        assert_eq!(
            CrsState::from_resolution(Some(projjson.clone()), false),
            CrsState::Known(projjson)
        );
        assert_eq!(CrsState::from_resolution(None, true), CrsState::Unknown);
        assert_eq!(
            CrsState::from_resolution(None, false),
            CrsState::Unspecified
        );
    }
}
