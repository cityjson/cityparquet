//! Partitioned-conversion tests over the real `delft.city.jsonl` fixture.

use std::path::PathBuf;

use cityparquet::package::ConvertOptions;
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), tests below open a small on-disk COPY
/// with a CRS injected via JSON mutation of the real fixture — never
/// hand-written CityJSON. `Source` streams CityJSONSeq lazily from its own
/// path (see `crate::source::Source::features`), so the returned `TempDir`
/// MUST outlive the `Source` — callers keep it bound, never `_`-discarded.
fn railway_source_with_crs() -> (tempfile::TempDir, Source) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_with_crs.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    let src = Source::open(&path).unwrap();
    (dir, src)
}

/// `metadata.json`'s object-table file names for the package at `dir`
/// (`PackageTables::open`'s `cityparquet-objects`-role assets) — by-type is
/// the only, mandatory table layout, so this is 1..N main-table file names,
/// one per 1st-level CityObject family actually present.
fn manifest_tables(dir: &std::path::Path) -> Vec<String> {
    PackageTables::open(dir)
        .unwrap()
        .tables
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn delft_opts(out: &std::path::Path) -> ConvertOptions {
    ConvertOptions::new(fixture("delft.city.jsonl"), out.to_path_buf())
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
        // delft is a single 1st-level family, so every partition's by-type
        // conversion writes exactly one main table: building.parquet.
        assert!(
            out.path().join(label).join("building.parquet").exists(),
            "{label} package missing building.parquet"
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

/// Every box partition package is independently valid and round-trips: export
/// it to CityJSONSeq, reconvert + re-export, and the two exports are
/// semantically equal (an idempotent round-trip through the whole pipeline on
/// the partition's own feature subset).
#[test]
fn box_partitions_each_round_trip_clean() {
    use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
    use cityparquet::export::{ExportOptions, export};
    use cityparquet::package::convert;

    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = delft_opts(out.path());
    let rep = convert_partitioned(
        std::slice::from_ref(&src),
        &PartitionSpec::Box { cell: 1000.0 },
        &opts,
    )
    .unwrap();

    let no_excl = CompareOptions {
        coord_tolerance: [0.0; 3],
        exclusions: Exclusions {
            appearance: false,
            geometry_instances: false,
        },
    };

    for (label, _) in &rep.partitions {
        let pkg = out.path().join(label);
        let export1 = out.path().join(format!("{label}_1.city.jsonl"));
        export(&ExportOptions {
            package_dir: pkg.clone(),
            output: export1.clone(),
        })
        .unwrap();

        // Reconvert the exported partition and re-export it; the two exports
        // must be semantically identical.
        let pkg2 = out.path().join(format!("{label}_pkg2"));
        let o2 = ConvertOptions::new(export1.clone(), pkg2.clone());
        convert(&o2).unwrap();
        let export2 = out.path().join(format!("{label}_2.city.jsonl"));
        export(&ExportOptions {
            package_dir: pkg2.clone(),
            output: export2.clone(),
        })
        .unwrap();

        let rc = compare_datasets(&export1, &export2, &no_excl).unwrap();
        assert!(
            rc.equal,
            "partition {label} not round-trip clean: {:?}",
            rc.differences
        );
    }
}

/// Overwrite must delete only this driver's own partition subdirs, never an
/// unrelated directory that merely shares a name prefix (e.g. `box-office`).
#[test]
fn overwrite_preserves_unrelated_sibling_directories() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let mut opts = delft_opts(out.path());
    convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();

    // Unrelated directories whose names collide with a partition prefix.
    for name in ["box-office", "counter", "features_backup"] {
        std::fs::create_dir(out.path().join(name)).unwrap();
        std::fs::write(out.path().join(name).join("keep.txt"), b"x").unwrap();
    }

    opts.overwrite = true;
    convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();

    for name in ["box-office", "counter", "features_backup"] {
        assert!(
            out.path().join(name).join("keep.txt").exists(),
            "unrelated dir {name} must survive overwrite"
        );
    }
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

/// A synthesised LoD0 footprint must be declared as the GeoParquet primary in
/// EVERY partition's `geo` metadata — the whole-dataset (canonical) legal
/// column set, not each partition's local set (railway partitions have no
/// GeoParquet-legal LoD locally, so without canonicalisation they would omit
/// the synthesised footprint and disagree with `default_geometry`).
#[test]
fn partitioned_synthesis_declares_the_footprint_as_primary_in_every_partition() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, src) = railway_source_with_crs();
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    opts.generate_lod0 = true;
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();
    assert!(!rep.partitions.is_empty());
    for (label, _) in &rep.partitions {
        // railway has 10 1st-level families, so each partition's by-type
        // conversion may write several main tables — every one of them must
        // carry the synthesised footprint as the GeoParquet primary column,
        // never a single hardcoded main-table name.
        let partition_dir = out.path().join(label);
        let tables = manifest_tables(&partition_dir);
        assert!(!tables.is_empty(), "{label} package lists no main tables");
        for table in &tables {
            let file = std::fs::File::open(partition_dir.join(table)).unwrap();
            let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
            let kvs = builder
                .metadata()
                .file_metadata()
                .key_value_metadata()
                .unwrap();
            let city: serde_json::Value = serde_json::from_str(
                kvs.iter()
                    .find(|kv| kv.key == "city")
                    .unwrap_or_else(|| panic!("{label}/{table} must carry a city key"))
                    .value
                    .as_deref()
                    .unwrap(),
            )
            .unwrap();
            // A module whose own objects carry NO analysis geometry at all
            // (railway's Vegetation module, real data — see
            // `synthesis_adds_a_primary_geometry_footprint_to_a_solid_only_dataset`'s
            // doc comment) has nothing for LoD0 synthesis to derive a
            // footprint FROM, so it legitimately carries no `city.columns`
            // and no `geo` key at all — skip it rather than assert a
            // footprint that was never eligible to exist.
            if city["columns"]
                .as_array()
                .is_none_or(std::vec::Vec::is_empty)
            {
                continue;
            }
            let geo = kvs.iter().find(|kv| kv.key == "geo").unwrap_or_else(|| {
                panic!("{label}/{table} must carry a geo key for the synthesised footprint")
            });
            let geo: serde_json::Value =
                serde_json::from_str(geo.value.as_deref().unwrap()).unwrap();
            assert_eq!(
                geo["primary_column"], "geometry_lod0_0",
                "{label}/{table}: synthesised footprint must be the GeoParquet primary_column"
            );
            assert!(
                geo["columns"].get("geometry_lod0_0").is_some(),
                "{label}/{table}: the geometry_lod0_0 column must be declared in geo.columns"
            );
        }
    }
}
