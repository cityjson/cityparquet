//! LOD-aware helpers for working with the duckdb-cityjson extension.
//!
//! The extension exposes a per-LOD geometry column for every LOD it finds in the
//! source file (e.g. `geom_lod2_2`). We use `DESCRIBE SELECT * FROM read_*('path')`
//! to enumerate them — no Rust-side CityJSON parsing.

use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::{InputFormat, LodKey};

/// Discover every LOD present in a CityJSON source file by introspecting the schema
/// the cityjson extension would produce. Returns the LODs in the order DuckDB
/// reports them (typically alphabetical by column name).
pub fn discover_lods(
    connection: &Arc<Mutex<Connection>>,
    source_path: &str,
    format: InputFormat,
) -> RepositoryResult<Vec<LodKey>> {
    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    // DESCRIBE returns one row per output column with a `column_name` field.
    // We do not interpolate `source_path` defensively quoted because DuckDB does
    // not support binding parameters inside table-function arguments. Callers must
    // ensure the path comes from a trusted boundary or is otherwise validated.
    let sql = format!(
        "DESCRIBE SELECT * FROM {read_fn}('{path}')",
        read_fn = format.read_function(),
        path = source_path.replace('\'', "''"),
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare schema discovery: {e}"))?;

    let column_iter = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            Ok(name)
        })
        .map_err(|e| format!("Failed to execute schema discovery: {e}"))?;

    let mut lods = Vec::new();
    for col in column_iter {
        let name = col.map_err(|e| format!("Failed to read column row: {e}"))?;
        if let Some(lod) = parse_lod_column(&name) {
            lods.push(lod);
        }
    }

    if lods.is_empty() {
        return Err(format!(
            "No LOD geometry columns (geom_lodX_Y) found in '{source_path}'"
        )
        .into());
    }

    Ok(lods)
}

/// Build the per-LOD table name from a base name and a LOD.
/// `derive_table_name("buildings", &LodKey("2.2")) == "buildings_lod_2_2"`.
pub fn derive_table_name(base: &str, lod: &LodKey) -> String {
    format!("{base}_{}", lod.as_suffix())
}

/// Reverse of `derive_table_name` — recover the LOD from a table name with a
/// `_lod_X_Y` suffix. Returns `None` when no suffix is present, which is how
/// callers detect a non-LOD-suffixed table.
pub fn lod_from_table_name(table_name: &str) -> Option<LodKey> {
    let idx = table_name.rfind("_lod_")?;
    let suffix = &table_name[idx + "_lod_".len()..];
    if suffix.is_empty() {
        return None;
    }
    let lod_str: String = suffix.replace('_', ".");
    LodKey::parse(&lod_str).ok()
}

/// Parse a column name like `geom_lod2_2` into a LodKey ("2.2"). Returns None if
/// the column is not a LOD geometry column.
fn parse_lod_column(name: &str) -> Option<LodKey> {
    let rest = name.strip_prefix("geom_lod")?;
    if rest.is_empty() {
        return None;
    }
    let parts: Vec<&str> = rest.split('_').collect();
    let lod_str = match parts.as_slice() {
        [single] => (*single).to_string(),
        [major, minor] => format!("{major}.{minor}"),
        _ => return None,
    };
    LodKey::parse(&lod_str).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lod_column_decimal() {
        let lod = parse_lod_column("geom_lod2_2").unwrap();
        assert_eq!(lod.as_str(), "2.2");
    }

    #[test]
    fn test_parse_lod_column_integer() {
        let lod = parse_lod_column("geom_lod1").unwrap();
        assert_eq!(lod.as_str(), "1");
    }

    #[test]
    fn test_parse_lod_column_rejects_non_geom() {
        assert!(parse_lod_column("attributes").is_none());
        assert!(parse_lod_column("geometry").is_none());
        assert!(parse_lod_column("geom_lod").is_none());
        assert!(parse_lod_column("geom_lod2_2_2").is_none());
    }

    #[test]
    fn test_derive_table_name() {
        let lod = LodKey::parse("2.2").unwrap();
        assert_eq!(derive_table_name("buildings", &lod), "buildings_lod_2_2");
        assert_eq!(derive_table_name("city_objects", &lod), "city_objects_lod_2_2");
    }

    #[test]
    fn test_lod_from_table_name_decimal() {
        let lod = lod_from_table_name("buildings_lod_2_2").unwrap();
        assert_eq!(lod.as_str(), "2.2");
    }

    #[test]
    fn test_lod_from_table_name_integer() {
        let lod = lod_from_table_name("city_objects_lod_1").unwrap();
        assert_eq!(lod.as_str(), "1");
    }

    #[test]
    fn test_lod_from_table_name_no_suffix() {
        assert!(lod_from_table_name("buildings").is_none());
        assert!(lod_from_table_name("citylake_metadata").is_none());
    }
}
