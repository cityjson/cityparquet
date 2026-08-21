//! `bbox` is the union over the object's whole subtree, not its own geometry
//! alone (spec "Object table schema" — "a consumer pruning on a parent's
//! `bbox` never misses geometry held by its descendants").

use arrow_array::{Array, StructArray, Float64Array, RecordBatch, StringArray};
use cityparquet::package::{ConvertOptions, convert};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn read_table(path: &std::path::Path) -> Vec<RecordBatch> {
    let file = std::fs::File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    builder
        .build()
        .unwrap()
        .map(|b| b.unwrap())
        .collect()
}

/// A 3DBAG `Building` carries only a flat LoD0 footprint while its
/// `BuildingPart`s carry the solids. Its `bbox` must still span the parts'
/// full z-range, or a z-filtered range query prunes the building away.
#[test]
fn parent_bbox_spans_descendant_solids() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.clone());
    convert(&opts).unwrap();

    let batches = read_table(&out.join("building.parquet"));
    let (parent_zmax, child_zmax) = zmax_for_pair(
        &batches,
        "NL.IMBAG.Pand.0503100000030621",
        "NL.IMBAG.Pand.0503100000030621-0",
    );

    assert!(
        parent_zmax >= child_zmax,
        "parent bbox zmax {parent_zmax} must cover its part's {child_zmax}"
    );
}

/// `(zmax of parent_id, zmax of child_id)` from the `bbox` struct column.
fn zmax_for_pair(batches: &[RecordBatch], parent_id: &str, child_id: &str) -> (f64, f64) {
    let mut parent = None;
    let mut child = None;
    for batch in batches {
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let bbox = batch
            .column_by_name("bbox")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let zmax = bbox
            .column_by_name("zmax")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for row in 0..ids.len() {
            if bbox.is_null(row) {
                continue;
            }
            if ids.value(row) == parent_id {
                parent = Some(zmax.value(row));
            } else if ids.value(row) == child_id {
                child = Some(zmax.value(row));
            }
        }
    }
    (
        parent.expect("parent row present"),
        child.expect("child row present"),
    )
}

/// A declared `geographicalExtent` may only ever widen `bbox`, never narrow
/// it. 3DBAG declares an extent that fails to contain its own geometry, so
/// the computed box must win on every bound it is larger on. This test pins
/// the union: the declared zmax (16.19086265563965) is strictly above the
/// computed subtree zmax (16.19), and the final bbox must include both.
#[test]
fn declared_extent_never_narrows_bbox() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.clone());
    convert(&opts).unwrap();

    let batches = read_table(&out.join("building.parquet"));
    let (parent_zmax, child_zmax) = zmax_for_pair(
        &batches,
        "NL.IMBAG.Pand.0503100000030621",
        "NL.IMBAG.Pand.0503100000030621-0",
    );
    // Delft fixture declares an extent for this Building with
    // zmax = 16.19086265563965 (strictly above the geometry's 16.19).
    // The computed subtree must reach at least the declared extent,
    // proving the union actually happened.
    const DECLARED_ZMAX: f64 = 16.19086265563965;
    assert!(parent_zmax >= child_zmax);
    assert!(
        parent_zmax >= DECLARED_ZMAX,
        "declared extent must be unioned in; parent_zmax={parent_zmax} must be >= declared {DECLARED_ZMAX}"
    );
}

/// `geographicalExtent` is carried by `bbox`, not by `other`.
#[test]
fn geographical_extent_does_not_ride_other() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.clone());
    convert(&opts).unwrap();

    let batches = read_table(&out.join("building.parquet"));
    for batch in &batches {
        let col = batch.column_by_name("other").unwrap();
        let col = col.as_any().downcast_ref::<StringArray>().unwrap();
        for row in 0..col.len() {
            assert!(
                col.is_null(row),
                "delft has no unmapped members, got: {}",
                col.value(row)
            );
        }
    }
}
