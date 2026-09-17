//! SQL construction for the CityParquet package operations.
//!
//! Every statement CityLake issues is built here, so this is the only place
//! quoting can go wrong. Nothing in this module touches a database: it maps
//! arguments to text, which is what makes it testable without one.
//!
//! Two rules govern everything below. Identifiers — schema, table — cannot be
//! parameterised, so they are validated and quoted. Values are rendered through
//! [`literal`], which doubles apostrophes so a path or predicate carrying one
//! cannot end the literal early and let the rest continue the statement.

use thiserror::Error;

/// The CityGML modules that hold feature objects, one object table each.
/// The specification fixes this set; a name outside it is not a module.
pub const OBJECT_MODULES: [&str; 11] = [
    "building",
    "bridge",
    "tunnel",
    "construction",
    "transportation",
    "vegetation",
    "relief",
    "water_body",
    "land_use",
    "city_furniture",
    "generics",
];

/// The optional sidecars, written only when the source has something for them.
pub const SIDECAR_TABLES: [&str; 3] = ["materials", "textures", "geometry_templates"];

/// The one object table `create_dataset_impl` always seeds before ingest —
/// there being no pragma that bootstraps a package from nothing. Named here
/// once so [`seed_table`] and the seed's post-ingest cleanup in `dataset.rs`
/// cannot drift apart.
pub const SEED_TABLE: &str = "building";

#[derive(Debug, Error)]
pub enum SqlError {
    #[error(
        "invalid dataset name {0:?}: a dataset becomes a schema name, which cannot be \
         parameterised, so it must match [a-zA-Z0-9_]+"
    )]
    InvalidDataset(String),
    #[error("unknown module {0:?}: not one of the CityGML object modules or sidecars")]
    UnknownModule(String),
}

/// A dataset name becomes a schema name. It cannot be bound as a parameter, so
/// it is validated rather than escaped.
pub fn validate_dataset(name: &str) -> Result<(), SqlError> {
    let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(())
        .ok_or_else(|| SqlError::InvalidDataset(name.to_string()))
}

/// A module name is checked against the closed set the specification defines —
/// a stronger check than a character class, and one that rejects a plausible
/// misspelling like `buildings`.
pub fn validate_module(name: &str) -> Result<(), SqlError> {
    let known = OBJECT_MODULES.contains(&name) || SIDECAR_TABLES.contains(&name);
    known
        .then_some(())
        .ok_or_else(|| SqlError::UnknownModule(name.to_string()))
}

/// Render a value as a single-quoted SQL literal, doubling apostrophes.
pub fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Render an identifier as a double-quoted name, doubling embedded quotes.
pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Join already-validated identifier parts into a dotted, quoted name.
pub fn qualified(parts: &[&str]) -> String {
    parts.iter().map(|p| ident(p)).collect::<Vec<_>>().join(".")
}

/// Point the search path at one or more schemas, so the package pragmas
/// resolve their bare schema argument inside the attached catalog. The
/// caller composes the path, because a merge needs two schemas reachable
/// at once.
pub fn set_search_path(path: &str) -> String {
    format!("SET search_path={}", literal(path))
}

/// Which reader and insert pragma a source path calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    CityJson,
    CityJsonSeq,
    FlatCityBuf,
}

impl SourceFormat {
    pub fn read_fn(&self) -> &'static str {
        match self {
            SourceFormat::CityJson => "read_cityjson",
            SourceFormat::CityJsonSeq => "read_cityjsonseq",
            SourceFormat::FlatCityBuf => "read_flatcitybuf",
        }
    }

    /// Named `insert_fn` to mirror `read_fn`, and so as not to share a name
    /// with the free `insert_pragma` below, which builds the whole statement.
    pub fn insert_fn(&self) -> &'static str {
        match self {
            SourceFormat::CityJson => "insert_cityjson",
            SourceFormat::CityJsonSeq => "insert_cityjsonseq",
            SourceFormat::FlatCityBuf => "insert_flatcitybuf",
        }
    }
}

/// Pick the reader from the file extension. This is not format detection —
/// it does not look inside the file, which is the extension's job.
pub fn reader_for(path: &str) -> SourceFormat {
    let lower = path.to_ascii_lowercase();
    // `.jsonl` before `.json`: the shorter suffix is a prefix of the longer one.
    if lower.ends_with(".jsonl") {
        SourceFormat::CityJsonSeq
    } else if lower.ends_with(".fcb") {
        SourceFormat::FlatCityBuf
    } else {
        SourceFormat::CityJson
    }
}

pub fn create_schema(catalog: &str, dataset: &str) -> String {
    format!("CREATE SCHEMA {}", qualified(&[catalog, dataset]))
}

/// The seed object table a fresh package needs.
///
/// There is no pragma that creates a package from nothing: `insert_cityjson`
/// on an empty schema fails with "schema has no CityParquet object table", and
/// `create_tables = true` creates the *further* module tables a source needs,
/// not the first one. So one object table is created from the source's inferred
/// schema and no rows — `LIMIT 0`, because a seeded row is a row the insert
/// then duplicates. An empty object table yields no Parquet file on write, so a
/// seed that stays empty costs nothing.
pub fn seed_table(catalog: &str, dataset: &str, source: &str, format: SourceFormat) -> String {
    format!(
        "CREATE TABLE {} AS SELECT * FROM {}({}) LIMIT 0",
        qualified(&[catalog, dataset, SEED_TABLE]),
        format.read_fn(),
        literal(source)
    )
}

pub fn init_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_init({})", literal(dataset))
}

/// PRAGMA named parameters use `=`, never `:=`.
pub fn insert_pragma(
    dataset: &str,
    source: &str,
    format: SourceFormat,
    create_tables: bool,
) -> String {
    let mut sql = format!(
        "PRAGMA {}({}, {}",
        format.insert_fn(),
        literal(dataset),
        literal(source)
    );
    if create_tables {
        sql.push_str(", create_tables = true");
    }
    sql.push(')');
    sql
}

/// Delete by predicate. Cascade is the extension's default and walks `children`
/// transitively — never `feature_id` equality, so deleting a BuildingPart does
/// not take out the Building sharing its feature_id.
pub fn delete_pragma(dataset: &str, predicate: &str, cascade: bool) -> String {
    let mut sql = format!(
        "PRAGMA cityparquet_delete({}, {}",
        literal(dataset),
        literal(predicate)
    );
    if !cascade {
        sql.push_str(", cascade = false");
    }
    sql.push(')');
    sql
}

pub fn reconcile_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_reconcile({})", literal(dataset))
}

pub fn validate_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_validate({})", literal(dataset))
}

pub fn orphans_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_orphans({})", literal(dataset))
}

pub fn vacuum_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_vacuum({})", literal(dataset))
}

pub fn merge_pragma(dst: &str, src: &str) -> String {
    format!(
        "PRAGMA cityparquet_merge({}, {})",
        literal(dst),
        literal(src)
    )
}

/// Load a package directory into a schema, recovering each file's Parquet
/// footer — the one thing a hand-rolled `read_parquet` load throws away.
pub fn read_package_pragma(dir: &str, dataset: &str) -> String {
    format!(
        "PRAGMA cityparquet_read({}, {})",
        literal(dir),
        literal(dataset)
    )
}

/// Write the package out. Omitting `crs` is legal and writes an explicit
/// `"crs": null` plus a warning — the CRS unknown, said out loud.
pub fn write_package(dataset: &str, dir: &str, crs: Option<&str>) -> String {
    match crs {
        Some(crs) => format!(
            "SELECT * FROM cityparquet_write({}, {}, crs => {})",
            literal(dataset),
            literal(dir),
            literal(crs)
        ),
        None => format!(
            "SELECT * FROM cityparquet_write({}, {})",
            literal(dataset),
            literal(dir)
        ),
    }
}

/// DuckLake's own maintenance. Not CTAS-and-rename: a DuckLake table's files
/// are the catalog's business, and merging them is what compaction means here.
pub fn compact(catalog: &str, dataset: &str, table: &str) -> String {
    format!(
        "CALL ducklake_merge_adjacent_files({}, {}, schema => {})",
        literal(catalog),
        literal(table),
        literal(dataset)
    )
}

/// A page of objects as JSON. The filter is a caller-supplied SQL predicate —
/// see the trust model in the specification's §10.
pub fn select_objects(
    catalog: &str,
    dataset: &str,
    module: &str,
    filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> String {
    let mut sql = format!(
        "SELECT to_json(t) FROM {} t",
        qualified(&[catalog, dataset, module])
    );
    if let Some(predicate) = filter {
        sql.push_str(&format!(" WHERE {predicate}"));
    }
    // Paging needs a total order: without one, two successive pages may repeat
    // a row and skip another. `id` is unique across the whole package, so it
    // orders the page deterministically.
    sql.push_str(" ORDER BY id");
    sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
    sql
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_survive_an_apostrophe() {
        // A path or predicate carrying an apostrophe must not close the literal
        // early and let the rest of the string continue the statement.
        assert_eq!(literal("O'Hara"), "'O''Hara'");
        assert_eq!(literal("plain"), "'plain'");
    }

    #[test]
    fn the_search_path_takes_a_composed_path() {
        assert_eq!(
            set_search_path("lake.delft"),
            "SET search_path='lake.delft'"
        );
        // A merge needs both schemas reachable at once.
        assert_eq!(
            set_search_path("lake.dst,lake.src"),
            "SET search_path='lake.dst,lake.src'"
        );
    }

    #[test]
    fn identifiers_are_double_quoted() {
        assert_eq!(ident("building"), "\"building\"");
        assert_eq!(ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn qualified_names_quote_every_part() {
        assert_eq!(
            qualified(&["lake", "delft", "building"]),
            "\"lake\".\"delft\".\"building\""
        );
    }

    #[test]
    fn dataset_names_reject_anything_but_word_characters() {
        assert!(validate_dataset("delft_2026").is_ok());
        // A schema name cannot be parameterised, so it is validated instead.
        for bad in [
            "delft; DROP SCHEMA x",
            "del ft",
            "delft-1",
            "",
            "lake.delft",
        ] {
            assert!(validate_dataset(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn module_names_are_checked_against_the_closed_set() {
        assert!(validate_module("building").is_ok());
        assert!(validate_module("water_body").is_ok());
        assert!(validate_module("materials").is_ok());
        // Stronger than a character class: the specification defines the set.
        assert!(validate_module("buildings").is_err());
        assert!(validate_module("Building").is_err());
    }

    #[test]
    fn the_reader_follows_the_file_extension() {
        assert_eq!(reader_for("a/b.city.json").read_fn(), "read_cityjson");
        assert_eq!(reader_for("a/b.city.jsonl").read_fn(), "read_cityjsonseq");
        assert_eq!(reader_for("a/b.fcb").read_fn(), "read_flatcitybuf");
        // .jsonl must not be mistaken for .json — check the longer suffix first.
        assert_eq!(
            reader_for("a/b.city.jsonl").insert_fn(),
            "insert_cityjsonseq"
        );
    }

    #[test]
    fn the_seed_table_selects_no_rows() {
        let sql = seed_table(
            "lake",
            "delft",
            "/d/x.city.jsonl",
            SourceFormat::CityJsonSeq,
        );
        assert!(sql.contains("\"lake\".\"delft\".\"building\""));
        assert!(sql.contains("read_cityjsonseq('/d/x.city.jsonl')"));
        // Schema only. A seeded row would be a row the insert then duplicates.
        assert!(sql.contains("LIMIT 0"));
    }

    #[test]
    fn pragma_named_parameters_use_equals_not_walrus() {
        let sql = insert_pragma("delft", "/d/x.city.json", SourceFormat::CityJson, true);
        assert!(sql.contains("create_tables = true"), "got {sql}");
        assert!(!sql.contains(":="));
    }

    #[test]
    fn delete_defaults_to_cascading() {
        let cascading = delete_pragma("delft", "id = 'x'", true);
        assert!(cascading.contains("cityparquet_delete"));
        assert!(!cascading.contains("cascade ="), "cascade is the default");
        assert!(delete_pragma("delft", "id = 'x'", false).contains("cascade = false"));
        // The predicate is a literal argument, so its own quotes must be doubled.
        assert!(cascading.contains("'id = ''x'''"));
    }

    #[test]
    fn writing_a_package_omits_crs_when_it_is_unknown() {
        assert!(write_package("delft", "/out", None).contains("cityparquet_write('delft', '/out')"));
        assert!(write_package("delft", "/out", Some("EPSG:7415")).contains("crs => 'EPSG:7415'"));
    }

    #[test]
    fn compaction_targets_one_table_in_one_schema() {
        assert_eq!(
            compact("lake", "delft", "building"),
            "CALL ducklake_merge_adjacent_files('lake', 'building', schema => 'delft')"
        );
    }

    #[test]
    fn selects_page_and_filter() {
        let sql = select_objects("lake", "delft", "building", None, 10, 20);
        assert_eq!(
            sql,
            "SELECT to_json(t) FROM \"lake\".\"delft\".\"building\" t \
             ORDER BY id LIMIT 10 OFFSET 20"
        );
        let filtered = select_objects(
            "lake",
            "delft",
            "building",
            Some("b3_h_dak_max > 20"),
            10,
            0,
        );
        // ORDER BY must sit between WHERE and LIMIT: a builder that emitted
        // them out of order would produce invalid SQL that only an
        // integration test would catch.
        assert_eq!(
            filtered,
            "SELECT to_json(t) FROM \"lake\".\"delft\".\"building\" t \
             WHERE b3_h_dak_max > 20 ORDER BY id LIMIT 10 OFFSET 0"
        );
    }
}
