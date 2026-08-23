//! Round-trip integration tests against real Delft sample data.
//!
//! Verifies that data ingested from a public CityJSON file can be exported and
//! re-ingested without losing CityObjects. We assert structural equality (row
//! counts, presence of LOD tables, persisted metadata) rather than byte
//! equality — the cityjson extension is free to re-quantise vertices and
//! re-order objects on `COPY TO`, so byte equality is not part of its contract.

use std::collections::BTreeMap;

use crate::core::db::service::DuckLakeService;
use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{
    CityLakeConfig, ExportFormat, LodKey, QueryParams, METADATA_TABLE,
};

const DELFT_JSONL_URL: &str = "https://storage.googleapis.com/cityjson/delft.city.jsonl";
const DELFT_JSON_URL: &str = "https://storage.googleapis.com/cityjson/delft.city.json";

/// Build a fresh service rooted at a temp directory so tests stay isolated.
fn fresh_service() -> (DuckLakeService, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let storage = tmp.path().join("data");
    let catalog = tmp.path().join("metadata.ducklake");
    let config = CityLakeConfig {
        storage_path: storage.to_string_lossy().to_string(),
        catalog_path: catalog.to_string_lossy().to_string(),
        ..Default::default()
    };
    let svc = DuckLakeService::new(config).expect("init DuckLakeService");
    (svc, tmp)
}

fn empty_params() -> QueryParams {
    QueryParams {
        filter: None,
        limit: None,
        offset: None,
    }
}

/// Count rows in each created table; returns a map keyed by table name.
async fn row_counts(svc: &DuckLakeService, tables: &[String]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for t in tables {
        let rows = svc
            .query_objects(t, &empty_params())
            .await
            .unwrap_or_else(|e| panic!("query {t}: {e}"));
        out.insert(t.clone(), rows.len());
    }
    out
}

#[tokio::test]
#[ignore = "downloads from network on first run; opt-in with --ignored"]
async fn test_load_delft_jsonl_creates_tables() {
    let (svc, _tmp) = fresh_service();
    let tables = svc
        .create_table(Some("delft"), DELFT_JSONL_URL, None)
        .await
        .expect("create_table from URL");

    assert!(!tables.is_empty(), "expected at least one LOD table");
    for t in &tables {
        assert!(
            t.starts_with("delft_lod_"),
            "table '{t}' should be LOD-suffixed"
        );
    }

    let counts = row_counts(&svc, &tables).await;
    for (t, n) in &counts {
        assert!(*n > 0, "table {t} should not be empty");
    }
}

#[tokio::test]
#[ignore = "downloads from network on first run; opt-in with --ignored"]
async fn test_load_delft_json_creates_tables() {
    let (svc, _tmp) = fresh_service();
    let tables = svc
        .create_table(Some("delft_json"), DELFT_JSON_URL, None)
        .await
        .expect("create_table from URL");

    assert!(!tables.is_empty());
    for t in &tables {
        assert!(t.starts_with("delft_json_lod_"));
    }
}

#[tokio::test]
#[ignore = "downloads from network on first run; opt-in with --ignored"]
async fn test_metadata_persisted_on_create() {
    let (svc, _tmp) = fresh_service();
    svc.create_table(Some("delft"), DELFT_JSONL_URL, None)
        .await
        .expect("create_table");

    // The shared metadata table holds one row per ingest; we should see exactly one
    // for "delft" with its source URL preserved.
    let rows = svc
        .query_objects(METADATA_TABLE, &empty_params())
        .await
        .expect("query metadata table");
    assert_eq!(rows.len(), 1, "expected one metadata row");

    let row = &rows[0];
    assert_eq!(row["dataset"], "delft");
    assert_eq!(row["source_path"], DELFT_JSONL_URL);
    assert!(row.get("version").is_some(), "metadata should carry version");
}

#[tokio::test]
#[ignore = "downloads from network on first run; opt-in with --ignored"]
async fn test_round_trip_preserves_row_counts() {
    // Stage 1 — ingest from the URL and remember row counts per LOD table.
    let (svc1, tmp1) = fresh_service();
    let tables = svc1
        .create_table(Some("delft"), DELFT_JSONL_URL, None)
        .await
        .expect("initial create_table");
    let original = row_counts(&svc1, &tables).await;
    assert!(!original.is_empty());

    // Stage 2 — export each LOD table to a CityJSONSeq file in a fresh tempdir.
    let export_dir = tempfile::tempdir().expect("create export dir");
    let mut exported_paths = BTreeMap::new();
    for t in &tables {
        let out = export_dir.path().join(format!("{t}.city.jsonl"));
        let out_str = out.to_string_lossy().to_string();
        svc1.export_table(t, &out_str, ExportFormat::CityJsonSeq)
            .await
            .unwrap_or_else(|e| panic!("export {t}: {e}"));
        assert!(out.exists(), "export file {} should exist", out.display());
        exported_paths.insert(t.clone(), out_str);
    }
    drop(svc1);
    drop(tmp1);

    // Stage 3 — re-ingest each exported file into a brand-new service and assert
    // the row counts come out identical. We pin the LOD explicitly so the
    // re-ingest does not depend on DESCRIBE returning the same column order.
    for (table, path) in &exported_paths {
        let lod = LodKey::parse(
            &table
                .strip_prefix("delft_lod_")
                .unwrap()
                .replace('_', "."),
        )
        .expect("derive lod from table name");

        let (svc2, _tmp2) = fresh_service();
        let new_tables = svc2
            .create_table(Some("reloaded"), path, Some(&lod))
            .await
            .unwrap_or_else(|e| panic!("reload {path}: {e}"));
        assert_eq!(new_tables.len(), 1, "single LOD reload should make one table");

        let after = row_counts(&svc2, &new_tables).await;
        let new_count = *after.values().next().unwrap();
        assert_eq!(
            new_count, original[table],
            "row count drifted on round-trip for {table}"
        );
    }
}

#[tokio::test]
#[ignore = "downloads from network on first run; opt-in with --ignored"]
async fn test_insert_into_existing_lod_table() {
    let (svc, _tmp) = fresh_service();
    let tables = svc
        .create_table(Some("delft"), DELFT_JSONL_URL, None)
        .await
        .expect("create_table");
    let counts_before = row_counts(&svc, &tables).await;

    // Insert the same source again (no LOD pinned → fans out across all tables)
    // and assert every table's row count exactly doubles.
    let total_inserted = svc
        .insert_objects("delft", DELFT_JSONL_URL, None)
        .await
        .expect("re-insert");
    assert!(total_inserted > 0);

    let counts_after = row_counts(&svc, &tables).await;
    for (t, before) in &counts_before {
        assert_eq!(
            counts_after[t],
            before * 2,
            "row count for {t} should double after a duplicate ingest"
        );
    }
}
