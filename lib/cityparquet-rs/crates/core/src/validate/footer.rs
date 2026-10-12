//! The `city` and `geo` footer objects (spec 05 "Parquet key-value metadata").

use super::*;

// ---------------------------------------------------------------------------
// The `city` footer object (spec 05 "The `city` object")
// ---------------------------------------------------------------------------

/// `^[a-z][a-z0-9]*$` (spec 06 "The namespace").
pub(super) fn is_namespace(ns: &str) -> bool {
    let mut chars = ns.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

pub(super) fn parse_city(file: &PackageFile, r: &mut Reporter) -> Option<CityFooter> {
    let f = Some(file.name.as_str());
    // spec 05 "Parquet key-value metadata": the footer carries a `city`
    // object; its `version` is required in every file, sidecars included.
    let Some(text) = file.key_value("city") else {
        r.error(
            "footer.city-missing",
            METADATA,
            f,
            "the footer has no `city` key".to_string(),
        );
        return None;
    };
    let raw = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => map,
        _ => {
            r.error(
                "footer.city-invalid",
                METADATA,
                f,
                "the `city` key is not a JSON object".to_string(),
            );
            return None;
        }
    };

    match raw.get("version") {
        Some(Value::String(v)) => {
            // spec 01 "Versioning": this document specifies 0.1.0-draft.
            if v != CITYPARQUET_VERSION {
                r.warn(
                    "footer.version-unsupported",
                    METADATA,
                    f,
                    format!(
                        "`city.version` is \"{v}\"; this validator implements \"{CITYPARQUET_VERSION}\""
                    ),
                );
            }
        }
        _ => r.error(
            "footer.version-missing",
            METADATA,
            f,
            "`city.version` is missing or not a string".to_string(),
        ),
    }

    // spec 05 "CRS rules": a known CRS MUST be PROJJSON; the key is
    // tri-state — object, null, or absent.
    let crs = match raw.get("crs") {
        None => CrsKey::Absent,
        Some(Value::Null) => CrsKey::Null,
        Some(v @ Value::Object(_)) => CrsKey::Projjson(v.clone()),
        Some(other) => {
            r.error(
                "footer.crs-not-projjson",
                METADATA,
                f,
                format!("`city.crs` is {other}, not a PROJJSON object or null"),
            );
            CrsKey::Invalid
        }
    };

    let attributes = match raw.get("attributes") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(_) => {
            r.error(
                "footer.attributes",
                METADATA,
                f,
                "`city.attributes` is not a list of column names".to_string(),
            );
            Vec::new()
        }
    };

    let columns = match raw.get("columns") {
        None => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => {
            r.error(
                "footer.columns",
                METADATA,
                f,
                "`city.columns` is not a list".to_string(),
            );
            Vec::new()
        }
    };

    let mut extensions = BTreeMap::new();
    match raw.get("extensions") {
        None => {}
        Some(Value::Object(map)) => {
            for (ns, decl) in map {
                // spec 06 "The namespace": MUST match `^[a-z][a-z0-9]*$`.
                if !is_namespace(ns) {
                    r.error(
                        "extension.namespace",
                        EXTENSIONS,
                        f,
                        format!("namespace `{ns}` does not match ^[a-z][a-z0-9]*$"),
                    );
                }
                // spec 06 "The extension declaration": `name` and `url` are
                // required.
                let complete = ["name", "url"]
                    .iter()
                    .all(|k| decl.get(k).is_some_and(Value::is_string));
                if !complete {
                    r.error(
                        "extension.declaration",
                        EXTENSIONS,
                        f,
                        format!("the declaration of `{ns}` lacks a string `name` and `url`"),
                    );
                }
                extensions.insert(ns.clone(), decl.clone());
            }
        }
        Some(_) => r.error(
            "extension.declaration",
            EXTENSIONS,
            f,
            "`city.extensions` is not an object keyed by namespace".to_string(),
        ),
    }

    let primary_column = match raw.get("primary_column") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            r.error(
                "footer.primary-column",
                METADATA,
                f,
                "`city.primary_column` is not a string".to_string(),
            );
            None
        }
    };

    Some(CityFooter {
        raw,
        crs,
        primary_column,
        columns,
        attributes,
        extensions,
    })
}

/// The footer checks that need the file's columns and values.
pub(super) fn check_object_footer(
    r: &mut Reporter,
    file: &PackageFile,
    geometry: &BTreeMap<String, Option<Option<String>>>,
    stats: &BTreeMap<String, GeometryStats>,
    has_coordinates: bool,
) {
    let name = file.name.as_str();
    let f = Some(name);
    let Some(city) = &file.city else { return };
    let columns: BTreeSet<String> = geometry.keys().map(|s| format!("geometry_{s}")).collect();

    // spec 05 "The `city` object": `primary_column` is required whenever
    // the table has a geometry column, and names one.
    match &city.primary_column {
        None if !columns.is_empty() => r.error(
            "footer.primary-column",
            METADATA,
            f,
            "the table has geometry columns but no `city.primary_column`".to_string(),
        ),
        Some(p) if !columns.contains(p) => r.error(
            "footer.primary-column",
            METADATA,
            f,
            format!("`city.primary_column` names `{p}`, which is not a geometry column"),
        ),
        _ => {}
    }

    // spec 05 "CRS rules": whenever the file holds a CRS-bearing coordinate
    // a writer MUST write `city.crs`, as PROJJSON or null.
    if city.crs == CrsKey::Absent && (has_coordinates || !columns.is_empty()) {
        r.error(
            "footer.crs-missing",
            METADATA,
            f,
            "the file holds CRS-bearing coordinates but `city.crs` is absent (read as OGC:CRS84)"
                .to_string(),
        );
    }

    // spec 05 "`city.columns` entries": one entry per geometry column, each
    // with `name`, `encoding: "WKB"`, `geometry_types` and an explicit
    // `orientation_3d`.
    let mut described: BTreeSet<String> = BTreeSet::new();
    for entry in &city.columns {
        let Some(col) = entry.get("name").and_then(Value::as_str) else {
            r.error(
                "footer.columns",
                METADATA,
                f,
                "a `city.columns` entry has no `name`".to_string(),
            );
            continue;
        };
        if !columns.contains(col) {
            r.error(
                "footer.columns",
                METADATA,
                f,
                format!("`city.columns` describes `{col}`, which is not a geometry column"),
            );
            continue;
        }
        if !described.insert(col.to_string()) {
            r.error(
                "footer.columns",
                METADATA,
                f,
                format!("`city.columns` describes `{col}` twice"),
            );
        }
        if entry.get("encoding").and_then(Value::as_str) != Some("WKB") {
            r.error(
                "footer.columns",
                METADATA,
                f,
                format!("`city.columns` entry `{col}`: `encoding` is not \"WKB\""),
            );
        }
        if !matches!(
            entry.get("orientation_3d").and_then(Value::as_str),
            Some("right-handed" | "left-handed")
        ) {
            r.error(
                "footer.columns",
                METADATA,
                f,
                format!(
                    "`city.columns` entry `{col}`: `orientation_3d` is not stated as \
                     \"right-handed\" or \"left-handed\""
                ),
            );
        }
        check_column_crs(r, name, &format!("city.columns `{col}`"), entry, &city.crs);
        let declared: Option<BTreeSet<&str>> = entry
            .get("geometry_types")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect());
        let Some(declared) = declared else {
            r.error(
                "footer.columns",
                METADATA,
                f,
                format!("`city.columns` entry `{col}` has no `geometry_types` list"),
            );
            continue;
        };
        let suffix = col.trim_start_matches("geometry_");
        if let Some(stat) = stats.get(suffix) {
            for &code in &stat.top_level_types {
                let type_name = wkb_type_name(code).unwrap_or("?");
                if !declared.contains(type_name) {
                    r.error(
                        "footer.geometry-types",
                        METADATA,
                        f,
                        format!("`{col}` carries {type_name}, absent from its `geometry_types`"),
                    );
                }
            }
            // An all-null column has no type to contradict its declaration.
            for type_name in declared.iter().filter(|_| stat.non_null > 0) {
                let present = stat
                    .top_level_types
                    .iter()
                    .any(|&c| wkb_type_name(c) == Some(type_name));
                if !present {
                    r.warn(
                        "footer.geometry-types",
                        METADATA,
                        f,
                        format!("`{col}` lists {type_name} in `geometry_types` but carries none"),
                    );
                }
            }
        }
    }
    for col in columns.difference(&described) {
        r.error(
            "footer.columns",
            METADATA,
            f,
            format!("geometry column `{col}` has no `city.columns` entry"),
        );
    }

    // The `geo` object and the GEOMETRY annotation (spec 05 "The `geo`
    // object — GeoParquet compatibility").
    let geo_columns = check_geo(r, file, &columns, &city.crs);
    for (suffix, annotation) in geometry {
        let col = format!("geometry_{suffix}");
        let declared = geo_columns.contains(&col);
        let annotated = annotation.is_some();
        // The declaration rule, applied at the second level: annotated if
        // and only if declared.
        if declared != annotated {
            r.error(
                "geo.annotation",
                METADATA,
                f,
                format!(
                    "`{col}` is {} in `geo.columns` but {} the GEOMETRY logical type",
                    if declared { "declared" } else { "not declared" },
                    if annotated {
                        "carries"
                    } else {
                        "does not carry"
                    }
                ),
            );
        }
        // A writer MUST write the annotation's `crs` as inline PROJJSON,
        // agreeing with `city.crs`.
        if let Some(crs) = annotation {
            check_annotation_crs(r, name, &col, crs.as_deref(), &city.crs);
        }
        // The declaration rule: declared if and only if every WKB type code
        // the column carries is within [1001, 1007].
        let legal = stats.get(suffix).and_then(GeometryStats::geoparquet_legal);
        let claims_solid = city.columns.iter().any(|e| {
            e.get("name").and_then(Value::as_str) == Some(col.as_str())
                && e.get("geometry_types")
                    .and_then(Value::as_array)
                    .is_some_and(|t| t.iter().any(|n| n.as_str() == Some("PolyhedralSurface Z")))
        });
        let illegal = legal == Some(false) || (legal.is_none() && claims_solid);
        if declared && illegal {
            r.error(
                "geo.declaration",
                METADATA,
                f,
                format!(
                    "`{col}` carries geometry outside GeoParquet's [1001, 1007] subset \
                     but is declared in `geo.columns`"
                ),
            );
        } else if !declared && legal == Some(true) {
            r.error(
                "geo.declaration",
                METADATA,
                f,
                format!(
                    "`{col}` carries only GeoParquet-legal geometry but is not declared in \
                     `geo.columns`"
                ),
            );
        }
    }
}

/// Check the `geo` object and return the column names it declares.
pub(super) fn check_geo(
    r: &mut Reporter,
    file: &PackageFile,
    geometry_columns: &BTreeSet<String>,
    city_crs: &CrsKey,
) -> BTreeSet<String> {
    let f = Some(file.name.as_str());
    let Some(text) = file.key_value("geo") else {
        return BTreeSet::new();
    };
    let invalid = |r: &mut Reporter, what: &str| {
        r.error(
            "geo.invalid",
            METADATA,
            f,
            format!("the `geo` object {what}"),
        );
    };
    let Ok(Value::Object(geo)) = serde_json::from_str::<Value>(text) else {
        invalid(r, "is not a JSON object");
        return BTreeSet::new();
    };
    // spec 05: `version`, `primary_column` and `columns` are required;
    // GeoParquet requires at least one column.
    if !geo.get("version").is_some_and(Value::is_string) {
        invalid(r, "has no string `version`");
    }
    let Some(columns) = geo.get("columns").and_then(Value::as_object) else {
        invalid(r, "has no `columns` object");
        return BTreeSet::new();
    };
    if columns.is_empty() {
        invalid(
            r,
            "declares no column (a table with no legal column writes no `geo` key)",
        );
    }
    match geo.get("primary_column").and_then(Value::as_str) {
        Some(p) if columns.contains_key(p) => {}
        Some(p) => r.error(
            "geo.primary-column",
            METADATA,
            f,
            format!("`geo.primary_column` `{p}` is not one of the declared `geo.columns`"),
        ),
        None => invalid(r, "has no string `primary_column`"),
    }
    for (col, entry) in columns {
        if !geometry_columns.contains(col) {
            r.error(
                "geo.column-unknown",
                METADATA,
                f,
                format!("`geo.columns` declares `{col}`, which is not a geometry column"),
            );
        }
        if entry.get("encoding").and_then(Value::as_str) != Some("WKB") {
            invalid(
                r,
                &format!("column `{col}` has an `encoding` other than \"WKB\""),
            );
        }
        check_column_crs(
            r,
            &file.name,
            &format!("geo.columns `{col}`"),
            entry,
            city_crs,
        );
    }
    columns.keys().cloned().collect()
}

/// spec 05 "CRS rules": a per-column `crs`, when present, mirrors the
/// file-level `city.crs` — including a `null`.
pub(super) fn check_column_crs(
    r: &mut Reporter,
    file: &str,
    what: &str,
    entry: &Value,
    city_crs: &CrsKey,
) {
    let Some(crs) = entry.get("crs") else { return };
    let agrees = match city_crs {
        CrsKey::Null => crs.is_null(),
        CrsKey::Projjson(v) => crs == v,
        CrsKey::Absent | CrsKey::Invalid => true,
    };
    if !agrees {
        r.error(
            "geo.column-crs",
            METADATA,
            Some(file),
            format!("{what}: `crs` differs from `city.crs`"),
        );
    }
}

/// spec 05 "CRS rules": the GEOMETRY annotation's `crs` is inline PROJJSON
/// and agrees with `city.crs`.
pub(super) fn check_annotation_crs(
    r: &mut Reporter,
    file: &str,
    column: &str,
    crs: Option<&str>,
    city_crs: &CrsKey,
) {
    let parsed = crs.map(serde_json::from_str::<Value>);
    let problem = match (parsed, city_crs) {
        (Some(Ok(Value::Object(a))), CrsKey::Projjson(c)) if Value::Object(a.clone()) == *c => None,
        (Some(Ok(Value::Object(_))), _) => Some("differs from `city.crs`"),
        (Some(_), _) => Some("is not inline PROJJSON"),
        (None, CrsKey::Projjson(_)) => {
            Some("is absent (the GEOMETRY default, OGC:CRS84), disagreeing with `city.crs`")
        }
        (None, _) => None,
    };
    if let Some(problem) = problem {
        r.error(
            "geo.annotation-crs",
            METADATA,
            Some(file),
            format!("the GEOMETRY annotation's `crs` on `{column}` {problem}"),
        );
    }
}
