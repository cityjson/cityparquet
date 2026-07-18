//! Resolve a source CRS identifier (an EPSG code or an OGC CRS URL) to
//! PROJJSON, using a vendored offline lookup table (spec §13.3, gap G1).
//!
//! GeoParquet 1.1 requires each geometry column's CRS to be **PROJJSON**, and
//! no maintained pure-Rust crate emits PROJJSON from an EPSG code (even
//! geoarrow-rs defers to pyproj). So the table is generated offline by
//! `tools/gen_projjson.py` from PROJ's `proj.db` — byte-for-byte the same
//! definitions GDAL/GeoPandas write — and committed gzipped, kept out of the
//! build so `cargo build` needs no C toolchain or network.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde_json::Value;

use crate::error::{CityParquetError, Result};

/// The committed EPSG -> PROJJSON table, gzipped. Regenerate with
/// `tools/gen_projjson.py` when the pinned PROJ/EPSG dataset is bumped (the
/// version pins live in the asset's `_meta`).
static ASSET: &[u8] = include_bytes!("../assets/epsg_projjson.json.gz");

/// The parsed table: `"7415"` / `"OGC:CRS84"` -> PROJJSON object. Decompressed
/// and parsed once, lazily (only the first CRS resolution pays for it).
fn table() -> &'static HashMap<String, Value> {
    static TABLE: OnceLock<HashMap<String, Value>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut gz = flate2::read::GzDecoder::new(ASSET);
        let mut json = String::new();
        gz.read_to_string(&mut json)
            .expect("vendored PROJJSON asset must gunzip");
        let mut map: HashMap<String, Value> =
            serde_json::from_str(&json).expect("vendored PROJJSON asset must parse");
        map.remove("_meta");
        map
    })
}

/// Extract the lookup key (an EPSG code, or `OGC:CRS84`/`OGC:CRS84h`) from a
/// source CRS identifier: a bare code `7415`, `EPSG:7415`,
/// `urn:ogc:def:crs:EPSG::7415`, an OGC CRS URL
/// `https://www.opengis.net/def/crs/EPSG/0/7415`, or a CRS84 URL.
fn lookup_key(source: &str) -> Option<String> {
    let s = source.trim().trim_end_matches('/');
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    if let Some(rest) = s.strip_prefix("OGC:") {
        return Some(format!("OGC:{rest}"));
    }
    // The last `/`- or `:`-delimited segment: the code for EPSG URLs/urns, or
    // `CRS84`/`CRS84h` for the OGC CRS84 URLs.
    let tail = s.rsplit(['/', ':']).next()?.trim();
    if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
        return Some(tail.to_string());
    }
    if tail == "CRS84" || tail == "CRS84h" {
        return Some(format!("OGC:{tail}"));
    }
    None
}

/// Resolve a source CRS identifier to its PROJJSON from the vendored EPSG
/// table. Errors if the identifier is unparseable or its code is not in the
/// table — it MUST NOT silently omit the CRS (§13.3: an absent GeoParquet
/// `crs` is taken to mean OGC:CRS84, silently mis-georeferencing a projected
/// national CRS).
pub fn resolve_to_projjson(source: &str) -> Result<Value> {
    // Already PROJJSON? (a CityGML source may hand one straight through.)
    if let Ok(value) = serde_json::from_str::<Value>(source)
        && value.is_object()
    {
        return Ok(value);
    }
    let key = lookup_key(source).ok_or_else(|| {
        CityParquetError::Schema(format!(
            "cannot extract an EPSG/OGC code from CRS {source:?}"
        ))
    })?;
    table().get(&key).cloned().ok_or_else(|| {
        CityParquetError::Schema(format!(
            "CRS {source:?} (code {key}) is not in the vendored EPSG->PROJJSON table; \
             regenerate it with tools/gen_projjson.py if the code is valid"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_key_extracts_the_epsg_code() {
        assert_eq!(lookup_key("7415").as_deref(), Some("7415"));
        assert_eq!(lookup_key("EPSG:7415").as_deref(), Some("7415"));
        assert_eq!(
            lookup_key("urn:ogc:def:crs:EPSG::7415").as_deref(),
            Some("7415")
        );
        assert_eq!(
            lookup_key("https://www.opengis.net/def/crs/EPSG/0/7415").as_deref(),
            Some("7415")
        );
        assert_eq!(lookup_key("OGC:CRS84").as_deref(), Some("OGC:CRS84"));
        assert_eq!(
            lookup_key("http://www.opengis.net/def/crs/OGC/1.3/CRS84").as_deref(),
            Some("OGC:CRS84")
        );
        assert_eq!(lookup_key("not a crs").as_deref(), None);
    }

    #[test]
    fn resolves_a_compound_national_crs() {
        // EPSG:7415 = Amersfoort/RD New + NAP height — a CompoundCRS.
        let crs = resolve_to_projjson("https://www.opengis.net/def/crs/EPSG/0/7415").unwrap();
        assert_eq!(crs["type"], "CompoundCRS");
        assert_eq!(crs["id"]["authority"], "EPSG");
        assert_eq!(crs["id"]["code"], 7415);
    }

    #[test]
    fn resolves_a_projected_crs() {
        let crs = resolve_to_projjson("EPSG:28992").unwrap();
        assert_eq!(crs["type"], "ProjectedCRS");
        assert_eq!(crs["id"]["code"], 28992);
    }

    #[test]
    fn unknown_code_is_an_error_not_a_silent_omission() {
        assert!(resolve_to_projjson("EPSG:999999999").is_err());
        assert!(resolve_to_projjson("garbage").is_err());
    }
}
