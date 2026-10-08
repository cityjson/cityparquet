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
    match item.get("geometry") {
        Some(Value::Null) => {}
        Some(Value::Object(_)) => {
            if !item.get("bbox").is_some_and(Value::is_array) {
                r.error(
                    "stac.not-an-item",
                    METADATA,
                    FILE,
                    "a non-null `geometry` requires a `bbox`".to_string(),
                );
            }
        }
        _ => r.error(
            "stac.not-an-item",
            METADATA,
            FILE,
            "`geometry` must be present, a GeoJSON geometry or null".to_string(),
        ),
    }
    if !item.get("links").is_some_and(Value::is_array) {
        r.error(
            "stac.not-an-item",
            METADATA,
            FILE,
            "`links` is missing or not an array".to_string(),
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
            let ranged = props.get("start_datetime").is_some_and(Value::is_string)
                && props.get("end_datetime").is_some_and(Value::is_string);
            let ok = match props.get("datetime") {
                Some(Value::String(_)) => true,
                Some(Value::Null) => ranged,
                _ => false,
            };
            if !ok {
                r.error(
                    "stac.datetime",
                    METADATA,
                    FILE,
                    "`properties.datetime` must be a string, or null with \
                     `start_datetime`/`end_datetime`"
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
        let name = href.trim_start_matches("./");
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
        if !name.contains('/') {
            listed.insert(name.to_string());
            if !parquet_files.iter().any(|f| f == name) {
                r.warn(
                    "stac.asset-missing-file",
                    METADATA,
                    FILE,
                    format!("asset `{key}` refers to {href}, which is not in the package"),
                );
            }
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
