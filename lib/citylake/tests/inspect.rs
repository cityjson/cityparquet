mod common;

use citylake::core::interface::types::{CityLakeError, DatasetName};

#[test]
fn a_freshly_created_dataset_validates_clean() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let findings = service.validate_impl(&name).expect("validate");
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == "error").collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn validation_findings_carry_their_check_and_table() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // Shape, not content: a clean dataset yields no rows, so this asserts the
    // call succeeds and returns a well-formed (possibly empty) list.
    let findings = service.validate_impl(&name).unwrap();
    for finding in &findings {
        assert!(!finding.check_name.is_empty());
        assert!(!finding.table_name.is_empty());
    }
}

#[test]
fn vacuum_runs_on_a_dataset_with_no_orphans() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let removed = service.vacuum_impl(&name).expect("vacuum");
    assert_eq!(
        removed, 0,
        "a fresh dataset has no unreferenced sidecar rows"
    );
}

#[test]
fn compaction_reports_what_it_merged() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // A dataset written in one go by `create_dataset_impl` (seed table plus one
    // insert) produces exactly one Parquet file per object table, so there is
    // nothing adjacent to merge -- confirmed by hand against a schema built the
    // same way (seed + insert_cityjsonseq), which returns zero rows from
    // `ducklake_merge_adjacent_files`. `stats.files_created <=
    // stats.files_processed.max(stats.files_created)` holds for every
    // possible pair of values and so cannot fail; the equality below is the
    // assertion that actually discriminates a working call from a broken one
    // (e.g. one that fabricated or miscounted rows).
    let stats = service.compact_impl(&name).expect("compact");
    assert_eq!(stats.files_processed, 0);
    assert_eq!(stats.files_created, 0);
}

#[test]
fn validating_an_absent_dataset_is_an_error() {
    let (service, _dir) = common::test_service();
    let err = service
        .validate_impl(&DatasetName::new("absent").unwrap())
        .expect_err("an absent dataset must not validate clean");
    // The variant, not just the message: a missing `schema_exists` guard would
    // let the pragma itself fail with a raw DuckDB "Schema with name absent
    // does not exist" error, whose Display also contains the substring
    // "absent" -- so a substring check alone cannot tell the two failure paths
    // apart.
    assert!(
        matches!(err, CityLakeError::DatasetNotFound(_)),
        "an absent dataset must report DatasetNotFound, got: {err}"
    );
    assert!(format!("{err}").contains("absent"));
}
