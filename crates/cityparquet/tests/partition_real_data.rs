//! Partitioned-conversion tests over the real `delft.city.jsonl` fixture.

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, TableLayout};
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::source::Source;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn delft_opts(out: &std::path::Path) -> ConvertOptions {
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.to_path_buf());
    opts.layout = TableLayout::Single;
    opts
}

#[test]
fn partitioned_convert_is_lossless_over_delft() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = delft_opts(out.path());
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(4), &opts).unwrap();
    assert_eq!(rep.partitions.len(), 4);
    // Completeness: union of per-partition object counts == single-package count.
    let total: usize = rep.partitions.iter().map(|(_, r)| r.object_count).sum();
    assert_eq!(
        total, 2231,
        "no object lost or duplicated across partitions"
    );
    for (label, _) in &rep.partitions {
        assert!(
            out.path().join(label).join("metadata.json").exists(),
            "{label} package missing metadata.json"
        );
        assert!(
            out.path().join(label).join("cityobjects.parquet").exists(),
            "{label} package missing cityobjects.parquet"
        );
    }
}

#[test]
fn box_partitions_cover_delft_completely() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = delft_opts(out.path());
    let rep = convert_partitioned(
        std::slice::from_ref(&src),
        &PartitionSpec::Box { cell: 1000.0 },
        &opts,
    )
    .unwrap();
    assert!(rep.partitions.len() > 1, "delft spans >1 1000m cell");
    let total: usize = rep.partitions.iter().map(|(_, r)| r.object_count).sum();
    assert_eq!(total, 2231);
    for (label, _) in &rep.partitions {
        assert!(label.starts_with("box"), "box label, got {label}");
    }
}

#[test]
fn rerun_with_overwrite_purges_stale_partitions() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let mut opts = delft_opts(out.path());
    convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(5), &opts).unwrap();
    assert!(out.path().join("count-00004").exists());

    opts.overwrite = true;
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();
    assert_eq!(rep.partitions.len(), 2);
    assert!(
        !out.path().join("count-00004").exists(),
        "stale count-00004 must be purged on overwrite"
    );
    assert!(!out.path().join("count-00002").exists());
    assert!(out.path().join("count-00001").exists());
}

#[test]
fn non_empty_parent_without_overwrite_errors() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = delft_opts(out.path());
    convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();
    // Second run into the same parent without overwrite must fail.
    assert!(
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).is_err(),
        "re-run into a parent with partitions needs overwrite"
    );
}
