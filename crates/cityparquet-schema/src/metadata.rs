//! Dataset-level metadata: the spec's Parquet key-value table as a typed struct.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CityParquetError, Result};

pub const CITYPARQUET_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceFormat {
    #[serde(rename = "CityJSON")]
    CityJson,
    #[serde(rename = "CityJSONSeq")]
    CityJsonSeq,
    #[serde(rename = "CityGML")]
    CityGml,
}

/// Everything CityParquet stores in Parquet key-value metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityParquetMetadata {
    pub cityparquet_version: String,
    pub source_format: SourceFormat,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_version: Option<String>,
    /// CRS as PROJJSON (GeoParquet convention).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub crs: Option<Value>,
    /// Original CityJSON `transform`, kept for re-quantisation on export.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub transform: Option<Value>,
    /// CityJSON extension declarations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extensions: Option<Value>,
    /// Inferred source attribute columns (§6). Serialised under the §13.1 key
    /// `attributes`. A reader distinguishes reserved from attribute columns
    /// with this list alone: any column NOT named here is a reserved
    /// structural column whose name is fixed by the spec (§5.1, §13.1) — so
    /// no separate `reserved_columns` key is written.
    #[serde(rename = "attributes")]
    pub attribute_columns: Vec<String>,
    pub default_geometry: String,
    pub bbox_column: String,
    /// Sidecar table files actually present alongside the main table.
    /// Skipped from serialisation when empty (and read back as empty when
    /// absent), so a writer that only knows the final list after encoding —
    /// see `cityparquet::package::convert` — can omit it from the
    /// `WriterProperties` key-value set and append the real entry to the
    /// footer post-encode without creating a duplicate key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecar_files: Vec<String>,
    /// The source CityJSON header's `metadata` object, re-serialised
    /// verbatim (`title`, `geographicalExtent`, `pointOfContact`,
    /// `referenceDate`, `identifier`, `referenceSystem`). NOTE: cjseq's
    /// `Metadata` struct has no passthrough for unknown members, so any
    /// vendor extension the source header carries there (e.g. delft's
    /// `fullMetadataUrl`) is NOT preserved — it never survives the initial
    /// deserialisation of the source header into `cjseq::Metadata`, so this
    /// field can only ever re-serialise what that struct kept.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_metadata: Option<Value>,
    /// `{"default-theme-material": ..., "default-theme-texture": ...}` built
    /// from the source header's `appearance` default-theme members; `None`
    /// when the header set neither.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub appearance_defaults: Option<Value>,
    /// Free-form producer/user metadata not covered by the keys above (§13.1):
    /// e.g. the source CityJSON `transform` once it stops being a structural
    /// key (G18). **Informational only** — a reader MUST NOT need it to decode
    /// the file. Absent (`None`) is the common case today.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub other: Option<Value>,
}

impl CityParquetMetadata {
    /// Serialise to Parquet key-value pairs. Scalar strings are stored plain;
    /// structured values are stored as JSON text, matching the spec's key table.
    ///
    /// On-disk contract (binding for external implementers): a scalar string
    /// value is written verbatim UNLESS its content also parses as a
    /// standalone JSON value (a number, a boolean, `null`, an array, an
    /// object, or a quoted string) — in that ambiguous case it is JSON-encoded
    /// instead, so e.g. the string `"2.0"` is written as the JSON text `"\"2.0\""`
    /// rather than the bare text `2.0`, which would otherwise read back as a
    /// number. Every other (non-string) value is always JSON-encoded.
    /// [`Self::from_key_values`] applies the symmetric probe on read.
    pub fn to_key_values(&self) -> Result<Vec<(String, String)>> {
        let object = match serde_json::to_value(self)? {
            Value::Object(map) => map,
            _ => unreachable!("struct serialises to an object"),
        };
        let mut kvs = Vec::with_capacity(object.len());
        for (key, value) in object {
            let rendered = match &value {
                Value::String(s) => {
                    // If the string is a valid JSON value on its own, JSON-encode it
                    // to avoid ambiguity during parsing (e.g., "2.0" would parse as a number)
                    if serde_json::from_str::<serde_json::Value>(s).is_ok() {
                        serde_json::to_string(&value)?
                    } else {
                        s.clone()
                    }
                }
                _other => serde_json::to_string(&value)?,
            };
            kvs.push((key, rendered));
        }
        Ok(kvs)
    }

    /// Parse back from Parquet key-value pairs, ignoring unrelated keys.
    pub fn from_key_values<'a>(kvs: impl Iterator<Item = (&'a str, &'a str)>) -> Result<Self> {
        let mut object = serde_json::Map::new();
        for (key, raw) in kvs {
            // Values were written either as plain strings or JSON text.
            let value = serde_json::from_str::<Value>(raw)
                .unwrap_or_else(|_| Value::String(raw.to_string()));
            object.insert(key.to_string(), value);
        }
        if !object.contains_key("cityparquet_version") {
            return Err(CityParquetError::Metadata(
                "missing key cityparquet_version".to_string(),
            ));
        }
        Ok(serde_json::from_value(Value::Object(object))?)
    }

    /// GeoParquet 1.1 `geo` metadata payload (§13.3, G1) so GeoParquet-ecosystem
    /// readers open the file natively. `columns` is `(name, geometry_types)`
    /// pairs, ascending by LoD, for the **GeoParquet-legal** geometry columns
    /// ONLY — a `PolyhedralSurfaceZ` (Solid-family) column must NOT be listed,
    /// or a reader parsing it eagerly fails the whole file (§1.3). Each declared
    /// column carries `encoding: "WKB"`, its `geometry_types` (with the `" Z"`
    /// 3D suffix), the dataset `crs` as PROJJSON (§13.3 resolves it at import),
    /// `edges: "planar"`, and the CityParquet extension `cityparquet:orientation`
    /// (3D right-hand winding, §7.1). `primary_column` is the highest `0.*`-family
    /// legal column when one is present — the `0.*` family (typically a
    /// footprint) is the most broadly GeoParquet-compatible geometry a writer
    /// can offer, so it is preferred as the primary even when a higher, also
    /// GeoParquet-legal LoD exists — else the highest-LoD legal column overall
    /// (last, ascending). Every LoD, including LoD0, is itself a suffixed
    /// column (spec "Levels of detail") — this is a *selection* preference
    /// among suffixed names, not a reintroduction of an un-suffixed column.
    /// Returns `None` when no column is GeoParquet-legal (e.g. a Solid-only
    /// dataset) — the caller then writes no `geo` key, and the file is simply
    /// not a GeoParquet file (still a valid CityParquet table).
    pub fn geoparquet_geo_value(&self, columns: &[(String, Vec<String>)]) -> Option<Value> {
        // `columns` is ascending by LoD, so the LAST `0.*`-family entry (if
        // any) is the highest one; otherwise the last entry overall.
        let primary = columns
            .iter()
            .rfind(|(name, _)| name.starts_with("geometry_lod0_"))
            .or_else(|| columns.last())
            .map(|(name, _)| name.clone())?;
        let mut cols = serde_json::Map::new();
        for (name, geometry_types) in columns {
            let mut column = serde_json::Map::new();
            column.insert("encoding".to_string(), Value::String("WKB".to_string()));
            column.insert(
                "geometry_types".to_string(),
                Value::Array(geometry_types.iter().cloned().map(Value::String).collect()),
            );
            if let Some(crs @ Value::Object(_)) = &self.crs {
                column.insert("crs".to_string(), crs.clone());
            }
            column.insert("edges".to_string(), Value::String("planar".to_string()));
            column.insert(
                "cityparquet:orientation".to_string(),
                Value::String("right-handed".to_string()),
            );
            cols.insert(name.clone(), Value::Object(column));
        }
        Some(serde_json::json!({
            "version": "1.1.0",
            "primary_column": primary,
            "columns": Value::Object(cols),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> CityParquetMetadata {
        CityParquetMetadata {
            cityparquet_version: CITYPARQUET_VERSION.to_string(),
            source_format: SourceFormat::CityJsonSeq,
            source_version: Some("2.0".to_string()),
            crs: Some(json!({"type": "ProjectedCRS", "id": {"authority": "EPSG", "code": 28992}})),
            transform: Some(json!({"scale": [0.001, 0.001, 0.001], "translate": [0.0, 0.0, 0.0]})),
            extensions: None,
            attribute_columns: vec!["yoc".to_string(), "height".to_string()],
            default_geometry: "geometry_lod2_2".to_string(),
            bbox_column: "bbox".to_string(),
            sidecar_files: vec![],
            source_metadata: Some(json!({"title": "x", "referenceDate": "2020-01-01"})),
            appearance_defaults: Some(json!({"default-theme-material": "t"})),
            other: None,
        }
    }

    /// The GeoParquet-legal columns (name + geometry_types) a writer passes to
    /// `geoparquet_geo_value`, matching `sample()`'s single LoD 2.2.
    fn sample_geometry_columns() -> Vec<(String, Vec<String>)> {
        vec![(
            "geometry_lod2_2".to_string(),
            vec!["MultiPolygon Z".to_string()],
        )]
    }

    /// RED (G8): footer key names must match §13.1 — `attributes` (not
    /// `attribute_columns`), and no `reserved_columns` key (§13.1: reserved
    /// names are fixed by the spec, so any column not in `attributes` is
    /// reserved — no separate list is needed).
    #[test]
    fn footer_keys_match_spec_13_1() {
        let kvs = sample().to_key_values().unwrap();
        let keys: Vec<&str> = kvs.iter().map(|(k, _)| k.as_str()).collect();
        assert!(
            keys.contains(&"attributes"),
            "spec §13.1 names the key `attributes`, got {keys:?}"
        );
        assert!(
            !keys.contains(&"attribute_columns"),
            "the legacy `attribute_columns` key must be gone"
        );
        assert!(
            !keys.contains(&"reserved_columns"),
            "§13.1 defines no `reserved_columns` key"
        );
    }

    #[test]
    fn key_value_round_trip() {
        let meta = sample();
        let kvs = meta.to_key_values().unwrap();
        // Simple strings stay plain; complex values are JSON-encoded.
        assert!(
            kvs.iter()
                .any(|(k, v)| k == "cityparquet_version" && v == "0.1.0")
        );
        assert!(
            kvs.iter()
                .any(|(k, v)| k == "source_format" && v == "CityJSONSeq")
        );
        assert!(kvs.iter().any(|(k, _)| k == "crs"));
        assert!(kvs.iter().any(|(k, _)| k == "source_metadata"));
        assert!(kvs.iter().any(|(k, _)| k == "appearance_defaults"));
        let back =
            CityParquetMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(back, meta);
        assert_eq!(
            back.source_metadata,
            Some(json!({"title": "x", "referenceDate": "2020-01-01"}))
        );
        assert_eq!(
            back.appearance_defaults,
            Some(json!({"default-theme-material": "t"}))
        );
    }

    #[test]
    fn missing_version_is_an_error() {
        let err = CityParquetMetadata::from_key_values(std::iter::empty());
        assert!(err.is_err());
    }

    #[test]
    fn geo_key_names_primary_column_and_full_column_metadata() {
        let geo = sample()
            .geoparquet_geo_value(&sample_geometry_columns())
            .unwrap();
        assert_eq!(geo["version"], "1.1.0");
        assert_eq!(geo["primary_column"], "geometry_lod2_2");
        let col = &geo["columns"]["geometry_lod2_2"];
        assert_eq!(col["encoding"], "WKB");
        assert_eq!(col["geometry_types"], json!(["MultiPolygon Z"]));
        assert_eq!(col["edges"], "planar");
        assert_eq!(col["cityparquet:orientation"], "right-handed");
        // CRS is propagated so GeoParquet readers see PROJJSON.
        assert_eq!(col["crs"]["id"]["code"], 28992);
    }

    #[test]
    fn geo_key_primary_is_the_highest_lod_and_lists_only_given_columns() {
        // The scan passes only the GeoParquet-legal columns, ascending by LoD.
        // No `0.*` LoD here, so the highest LoD overall wins (the `0.*`-family
        // preference is covered separately by
        // `primary_column_prefers_the_zero_family_when_lod0_present`).
        let columns = vec![
            (
                "geometry_lod1_2".to_string(),
                vec!["MultiPolygon Z".to_string()],
            ),
            (
                "geometry_lod2_2".to_string(),
                vec!["MultiPolygon Z".to_string()],
            ),
        ];
        let geo = sample().geoparquet_geo_value(&columns).unwrap();
        let cols = geo["columns"].as_object().unwrap();
        assert_eq!(cols["geometry_lod1_2"]["encoding"], "WKB");
        assert_eq!(cols["geometry_lod2_2"]["encoding"], "WKB");
        assert!(!cols.contains_key("geometry_properties_lod1_2"));
        // Highest LoD (last in the ascending list) is the primary column.
        assert_eq!(geo["primary_column"], "geometry_lod2_2");
    }

    /// spec "Levels of detail": LoD0 is a suffixed column like any other, but
    /// it is still preferred as the GeoParquet primary when present — the same
    /// "footprint wins" preference as before, just expressed as a selection
    /// among suffixed names rather than a picked-out un-suffixed one.
    #[test]
    fn primary_column_prefers_the_zero_family_when_lod0_present() {
        let columns = vec![
            (
                "geometry_lod0_0".to_string(),
                vec!["MultiPolygon Z".to_string()],
            ),
            (
                "geometry_lod2_2".to_string(),
                vec!["MultiPolygon Z".to_string()],
            ),
        ];
        let geo = sample().geoparquet_geo_value(&columns).unwrap();
        assert_eq!(geo["primary_column"], "geometry_lod0_0");
        assert!(geo["columns"].get("geometry_lod0_0").is_some());
    }

    #[test]
    fn primary_column_stays_highest_lod_without_lod0() {
        let columns = vec![(
            "geometry_lod2_2".to_string(),
            vec!["MultiPolygon Z".to_string()],
        )];
        let geo = sample().geoparquet_geo_value(&columns).unwrap();
        assert_eq!(geo["primary_column"], "geometry_lod2_2");
    }

    #[test]
    fn geo_key_is_none_when_no_column_is_legal() {
        // A Solid-only dataset has no GeoParquet-legal column, so no geo key.
        assert!(sample().geoparquet_geo_value(&[]).is_none());
    }

    #[test]
    fn geo_key_omits_crs_when_not_projjson_object() {
        let mut meta = sample();
        // GeoParquet requires "crs" to be a PROJJSON object or absent, so a
        // bare string value must not be copied into the geo key.
        meta.crs = Some(Value::String(
            "https://www.opengis.net/def/crs/EPSG/0/7415".to_string(),
        ));
        let geo = meta
            .geoparquet_geo_value(&sample_geometry_columns())
            .unwrap();
        assert!(
            !geo["columns"]["geometry_lod2_2"]
                .as_object()
                .unwrap()
                .contains_key("crs"),
            "non-PROJJSON crs must be omitted from the geo key, not copied verbatim"
        );
    }

    #[test]
    fn geo_key_propagates_crs_when_projjson_object() {
        let geo = sample()
            .geoparquet_geo_value(&sample_geometry_columns())
            .unwrap();
        assert!(
            geo["columns"]["geometry_lod2_2"]
                .as_object()
                .unwrap()
                .contains_key("crs")
        );
    }
}
