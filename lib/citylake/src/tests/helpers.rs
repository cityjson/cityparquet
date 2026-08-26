use std::path::PathBuf;

use crate::core::db::service::DuckLakeService;

/// Returns the path to the test data file `delft.city.jsonl`.
#[allow(dead_code)]
pub fn test_data_path() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tests/data/delft.city.jsonl");
    path.to_string_lossy().to_string()
}

/// Create a fresh DuckLakeService using in-memory DuckDB (no extensions needed).
pub fn setup() -> DuckLakeService {
    DuckLakeService::new_for_testing().expect("Failed to create test DuckLakeService")
}

/// SQL to create a test table with the schema that mirrors cityjson extension output.
/// Uses VARCHAR instead of JSON to avoid needing the json extension.
const CREATE_TABLE_SQL: &str = r#"
CREATE TABLE citylake.{TABLE} (
    id VARCHAR,
    type VARCHAR,
    attributes VARCHAR,
    geometry VARCHAR,
    vertices VARCHAR
);
"#;

/// SQL to insert the 3 Delft test buildings.
const INSERT_DATA_SQL: &str = r#"
INSERT INTO citylake.{TABLE} VALUES
    ('building_001', 'Building',
     '{"bouwjaar":1980,"status":"Pand in gebruik","gebruiksdoel":"woonfunctie"}',
     '[{"type":"Solid","lod":"2.2"}]',
     '[[1000,2000,0],[1000,2010,0]]'),
    ('building_002', 'Building',
     '{"bouwjaar":1995,"status":"Pand in gebruik","gebruiksdoel":"kantoorfunctie"}',
     '[{"type":"Solid","lod":"2.2"}]',
     '[[2000,3000,0],[2000,3020,0]]'),
    ('building_003', 'Building',
     '{"bouwjaar":2010,"status":"Pand in gebruik","gebruiksdoel":"winkelfunctie"}',
     '[{"type":"Solid","lod":"2.2"}]',
     '[[3000,4000,0],[3000,4015,0]]');
"#;

/// Create a DuckLakeService with a pre-populated test table.
pub fn setup_with_table(table_name: &str) -> DuckLakeService {
    let service = setup();
    let conn = service.connection().lock().unwrap();

    let create_sql = CREATE_TABLE_SQL.replace("{TABLE}", table_name);
    conn.execute_batch(&create_sql)
        .expect("Failed to create test table");

    let insert_sql = INSERT_DATA_SQL.replace("{TABLE}", table_name);
    conn.execute_batch(&insert_sql)
        .expect("Failed to insert test data");

    drop(conn);
    service
}
