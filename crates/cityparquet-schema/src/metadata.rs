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
    pub attribute_columns: Vec<String>,
    pub reserved_columns: Vec<String>,
    pub default_geometry: String,
    pub bbox_column: String,
    #[serde(default)]
    pub sidecar_files: Vec<String>,
}

impl CityParquetMetadata {
    /// Serialise to Parquet key-value pairs. Scalar strings are stored plain;
    /// structured values are stored as JSON text, matching the spec's key table.
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

    /// GeoParquet `geo` key payload so GeoParquet-ecosystem readers can open
    /// the file natively (WKB encoding, PROJJSON CRS).
    pub fn geoparquet_geo_value(&self) -> Result<Value> {
        let mut column = serde_json::Map::new();
        column.insert("encoding".to_string(), Value::String("WKB".to_string()));
        column.insert("geometry_types".to_string(), Value::Array(vec![]));
        if let Some(crs) = &self.crs {
            column.insert("crs".to_string(), crs.clone());
        }
        Ok(serde_json::json!({
            "version": "1.1.0",
            "primary_column": self.default_geometry,
            "columns": { self.default_geometry.clone(): Value::Object(column) },
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
            reserved_columns: vec![
                "id".to_string(),
                "object_type".to_string(),
                "bbox".to_string(),
            ],
            default_geometry: "geometry_lod2_2".to_string(),
            bbox_column: "bbox".to_string(),
            sidecar_files: vec![],
        }
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
        let back =
            CityParquetMetadata::from_key_values(kvs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .unwrap();
        assert_eq!(back, meta);
    }

    #[test]
    fn missing_version_is_an_error() {
        let err = CityParquetMetadata::from_key_values(std::iter::empty());
        assert!(err.is_err());
    }

    #[test]
    fn geo_key_names_primary_column_and_wkb() {
        let geo = sample().geoparquet_geo_value().unwrap();
        assert_eq!(geo["version"], "1.1.0");
        assert_eq!(geo["primary_column"], "geometry_lod2_2");
        assert_eq!(geo["columns"]["geometry_lod2_2"]["encoding"], "WKB");
        // CRS is propagated so GeoParquet readers see PROJJSON.
        assert_eq!(
            geo["columns"]["geometry_lod2_2"]["crs"]["id"]["code"],
            28992
        );
    }
}
