//! `metadata.json` — the package's STAC Item (spec 05 "metadata.json — STAC Item").

use super::*;

// ---------------------------------------------------------------------------
// metadata.json — STAC Item (spec 05 "metadata.json — STAC Item")
// ---------------------------------------------------------------------------

pub(super) fn check_stac(dir: &Path, parquet_files: &[String], r: &mut Reporter) {
    const FILE: Option<&str> = Some("metadata.json");
    let path = dir.join("metadata.json");
    // spec 05: the package-level `metadata.json` MUST be a valid STAC Item.
    let Ok(text) = fs::read_to_string(&path) else {
        r.error(
            "stac.missing",
            METADATA,
            None,
            "the package has no metadata.json".to_string(),
        );
        return;
    };
    let item: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            r.error(
                "stac.invalid-json",
                METADATA,
                FILE,
                format!("not JSON: {e}"),
            );
            return;
        }
    };
    let Some(item) = item.as_object() else {
        r.error(
            "stac.not-an-item",
            METADATA,
            FILE,
            "not a JSON object".to_string(),
        );
        return;
    };

    // The STAC Item's required fields.
    if item.get("type").and_then(Value::as_str) != Some("Feature") {
        r.error(
            "stac.not-an-item",
            METADATA,
            FILE,
            "`type` is not \"Feature\"".to_string(),
        );
    }
    for key in ["stac_version", "id"] {
        if !item.get(key).is_some_and(Value::is_string) {
            r.error(
                "stac.not-an-item",
                METADATA,
                FILE,
                format!("`{key}` is missing or not a string"),
            );
        }
    }
    let not_an_item = |r: &mut Reporter, what: &str| {
        r.error("stac.not-an-item", METADATA, FILE, what.to_string());
    };
    if item
        .get("stac_extensions")
        .is_some_and(|e| !e.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
    {
        not_an_item(r, "`stac_extensions` is not a list of schema URIs");
    }
    // The Item is a GeoJSON Feature: `geometry` a GeoJSON geometry or null,
    // and `bbox` — required with a geometry — 4 or 6 finite numbers.
    match item.get("geometry") {
        Some(Value::Null) => {}
        Some(g) if is_geojson_geometry(g) => {
            if item.get("bbox").is_none() {
                not_an_item(r, "a non-null `geometry` requires a `bbox`");
            }
        }
        _ => not_an_item(r, "`geometry` must be present, a GeoJSON geometry or null"),
    }
    if let Some(bbox) = item.get("bbox") {
        let ok = bbox.as_array().is_some_and(|a| {
            matches!(a.len(), 4 | 6) && a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))
        });
        if !ok {
            not_an_item(r, "`bbox` is not 4 or 6 finite numbers");
        }
    }
    // Each link is an object with a string `href` and `rel`.
    let links_ok = item
        .get("links")
        .and_then(Value::as_array)
        .is_some_and(|links| {
            links.iter().all(|l| {
                l.get("href").is_some_and(Value::is_string)
                    && l.get("rel").is_some_and(Value::is_string)
            })
        });
    if !links_ok {
        not_an_item(
            r,
            "`links` is not a list of links each with a string `href` and `rel`",
        );
    }
    match item.get("properties").and_then(Value::as_object) {
        None => r.error(
            "stac.not-an-item",
            METADATA,
            FILE,
            "`properties` is missing or not an object".to_string(),
        ),
        Some(props) => {
            // RFC 3339 date-times (STAC "datetime"): `datetime`, or a null one
            // with a `start_datetime`/`end_datetime` range.
            let instant = |key: &str| {
                props
                    .get(key)
                    .and_then(Value::as_str)
                    .is_some_and(is_rfc3339)
            };
            let range_ok = ["start_datetime", "end_datetime"]
                .iter()
                .all(|k| props.get(*k).is_none() || instant(k));
            let ok = range_ok
                && match props.get("datetime") {
                    Some(Value::String(_)) => instant("datetime"),
                    Some(Value::Null) => instant("start_datetime") && instant("end_datetime"),
                    _ => false,
                };
            if !ok {
                r.error(
                    "stac.datetime",
                    METADATA,
                    FILE,
                    "`properties.datetime` must be an RFC 3339 date-time, or null with \
                     RFC 3339 `start_datetime`/`end_datetime`"
                        .to_string(),
                );
            }
        }
    }

    // spec 05: it MUST use the 3D city models STAC extension, and SHOULD use
    // the File extension; it SHOULD NOT declare the Table extension merely to
    // publish a row count.
    let extensions: Vec<&str> = item
        .get("stac_extensions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !extensions.iter().any(|e| e.contains("stac-city3d")) {
        r.error(
            "stac.city3d-extension",
            METADATA,
            FILE,
            "`stac_extensions` does not declare the 3D city models extension (city3d)".to_string(),
        );
    }
    if !extensions
        .iter()
        .any(|e| e.contains("stac-extensions.github.io/file/"))
    {
        r.warn(
            "stac.file-extension",
            METADATA,
            FILE,
            "the Item does not use the STAC File extension".to_string(),
        );
    }
    if extensions
        .iter()
        .any(|e| e.contains("stac-extensions.github.io/table/"))
    {
        r.warn(
            "stac.table-extension",
            METADATA,
            FILE,
            "the Item declares the Table extension".to_string(),
        );
    }

    let Some(assets) = item.get("assets").and_then(Value::as_object) else {
        r.error(
            "stac.not-an-item",
            METADATA,
            FILE,
            "`assets` is missing or not an object".to_string(),
        );
        return;
    };
    let mut listed: BTreeSet<String> = BTreeSet::new();
    for (key, asset) in assets {
        let Some(href) = asset.get("href").and_then(Value::as_str) else {
            r.error(
                "stac.not-an-item",
                METADATA,
                FILE,
                format!("asset `{key}` has no `href`"),
            );
            continue;
        };
        if !href.ends_with(".parquet") {
            continue;
        }
        // The file an asset names is the last segment of its `href`, which
        // may be an absolute URL; only a bare relative name is checked
        // against the directory.
        let name = href.rsplit('/').next().unwrap_or(href);
        let local = !href.contains("://") && !href.trim_start_matches("./").contains('/');
        let roles: Vec<&str> = asset
            .get("roles")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let objects = roles.contains(&"cityparquet-objects");
        let sidecar = roles.contains(&"cityparquet-sidecar");
        // spec 05 "Asset roles carry the package's file inventory": every
        // `.parquet` asset MUST declare `cityparquet-objects` or
        // `cityparquet-sidecar`, each alongside the STAC `data` role.
        if !objects && !sidecar {
            r.error(
                "stac.asset-role",
                METADATA,
                FILE,
                format!("asset `{key}` ({href}) declares neither `cityparquet-objects` nor `cityparquet-sidecar`"),
            );
        } else if !roles.contains(&"data") {
            r.error(
                "stac.asset-role",
                METADATA,
                FILE,
                format!("asset `{key}` ({href}) lacks the `data` role"),
            );
        }
        let kind = FileKind::of(name);
        if (objects && kind != FileKind::ObjectTable) || (sidecar && kind == FileKind::ObjectTable)
        {
            r.error(
                "stac.asset-role-mismatch",
                METADATA,
                FILE,
                format!("asset `{key}` ({href}) declares the role of the other kind of file"),
            );
        }
        listed.insert(name.to_string());
        if local && !parquet_files.iter().any(|f| f == name) {
            r.warn(
                "stac.asset-missing-file",
                METADATA,
                FILE,
                format!("asset `{key}` refers to {href}, which is not in the package"),
            );
        }
    }
    // spec 05: each `.parquet` file SHOULD be a STAC Asset.
    for name in parquet_files {
        if !listed.contains(name) {
            r.warn(
                "stac.file-not-an-asset",
                METADATA,
                FILE,
                format!("{name} is not an asset of the Item"),
            );
        }
    }
}

/// A GeoJSON geometry object (RFC 7946 §3.1): one of the seven types, with
/// `coordinates` nested to its type's depth, or a GeometryCollection of
/// geometries.
fn is_geojson_geometry(g: &Value) -> bool {
    let depth = match g.get("type").and_then(Value::as_str) {
        Some("Point") => 0,
        Some("LineString" | "MultiPoint") => 1,
        Some("Polygon" | "MultiLineString") => 2,
        Some("MultiPolygon") => 3,
        Some("GeometryCollection") => {
            return g
                .get("geometries")
                .and_then(Value::as_array)
                .is_some_and(|gs| gs.iter().all(is_geojson_geometry));
        }
        _ => return false,
    };
    g.get("coordinates")
        .is_some_and(|c| is_nested_positions(c, depth))
}

/// `depth` levels of arrays around positions of two or three numbers.
fn is_nested_positions(v: &Value, depth: usize) -> bool {
    let Some(items) = v.as_array() else {
        return false;
    };
    if depth == 0 {
        return matches!(items.len(), 2 | 3) && items.iter().all(Value::is_number);
    }
    items
        .iter()
        .all(|item| is_nested_positions(item, depth - 1))
}

fn is_rfc3339(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok()
}
