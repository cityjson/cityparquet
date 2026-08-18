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

/// Export the package at `pkg` to CityJSONSeq and return every CityObject id
/// it holds paired with every `parents`/`children` id those objects reference.
///
/// Reference locality is a property of the PACKAGE, not of one Parquet file:
/// the by-module layout legitimately puts a `CityObjectGroup` in
/// `generics.parquet` and its members in `vegetation.parquet`. Exporting
/// collapses the package back to one stream, which is exactly the granularity
/// the invariant is about.
fn package_ids_and_refs(
    pkg: &std::path::Path,
    scratch: &std::path::Path,
) -> (
    std::collections::HashSet<String>,
    Vec<(String, &'static str, String)>,
) {
    use cityparquet::export::{ExportOptions, export};

    let jsonl = scratch.join(format!(
        "{}.export.city.jsonl",
        pkg.file_name().unwrap().to_string_lossy()
    ));
    export(&ExportOptions {
        package_dir: pkg.to_path_buf(),
        output: jsonl.clone(),
    })
    .unwrap();

    let mut ids = std::collections::HashSet::new();
    let mut refs = Vec::new();
    for line in std::fs::read_to_string(&jsonl).unwrap().lines() {
        if line.trim().is_empty() {
            continue;
        }
        let feature: serde_json::Value = serde_json::from_str(line).unwrap();
        // The first line is the CityJSONSeq header, which carries no objects.
        if feature["type"] == "CityJSON" {
            continue;
        }
        let objects = feature["CityObjects"].as_object().unwrap();
        for (id, co) in objects {
            ids.insert(id.clone());
            for (field, name) in [(&co["parents"], "parents"), (&co["children"], "children")] {
                for target in field.as_array().into_iter().flatten() {
                    refs.push((id.clone(), name, target.as_str().unwrap().to_string()));
                }
            }
        }
    }
    (ids, refs)
}

/// Assert the invariant the whole partition design rests on: a city object and
/// everything it points at through `parents`/`children` land in the SAME
/// partition package. `assign_partitions` keys on the feature index and a
/// `CityJSONFeature` is a top-level object plus its children, so this holds by
/// construction — this test is what stops a future partition method from
/// keying on the object index and quietly severing the hierarchy.
fn assert_references_are_package_local(out: &std::path::Path, labels: &[String], scenario: &str) {
    let mut checked = 0usize;
    for label in labels {
        let (ids, refs) = package_ids_and_refs(&out.join(label), out);
        for (from, field, target) in refs {
            assert!(
                ids.contains(&target),
                "{scenario}: {label} holds {from} whose {field} reference {target} \
                 resolves outside the package"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "{scenario}: no parent/child references were checked at all — the fixture \
         or the export lost the hierarchy, so this test proves nothing"
    );
}

/// Maximal split: one feature per partition. If reference locality survives
/// this it survives every coarser `count`/`features` split, because those only
/// ever put MORE features together. Railway is the fixture with the widest
/// hierarchy — a `CityObjectGroup` with 14 members, spanning two module files
/// (`generics.parquet` and `vegetation.parquet`) inside one package.
#[test]
fn parent_child_references_stay_package_local_under_maximal_split() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, src) = railway_source_with_crs();
    let opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(38), &opts).unwrap();
    let labels: Vec<String> = rep.partitions.iter().map(|(l, _)| l.clone()).collect();
    assert_eq!(
        labels.len(),
        38,
        "railway has 38 top-level features, so Count(38) is one feature per partition"
    );
    assert_references_are_package_local(out.path(), &labels, "railway Count(38)");
}

/// The same invariant under the spatial method, where the label comes from a
/// feature's own centroid rather than its position in the stream.
#[test]
fn parent_child_references_stay_package_local_under_box_partitioning() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = delft_opts(out.path());
    let rep = convert_partitioned(
        std::slice::from_ref(&src),
        &PartitionSpec::Box { cell: 1000.0 },
        &opts,
    )
    .unwrap();
    let labels: Vec<String> = rep.partitions.iter().map(|(l, _)| l.clone()).collect();
    assert!(labels.len() > 1, "delft spans >1 1000m cell");
    assert_references_are_package_local(out.path(), &labels, "delft Box(1000)");
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

/// Build a CityJSONSeq derivative of the real `delft.city.jsonl` whose FIRST
/// feature has been torn in two: the `Building` on one line and its
/// `BuildingPart` on the next, each carrying the original feature's whole
/// vertex array so both lines stay individually valid. Every other line is the
/// real fixture's, untouched.
///
/// CityJSONSeq requires a feature to be self-contained, so this is
/// non-conformant input — but it is exactly the shape a merge of neighbouring
/// tiles produces when a building's parts fall on the other side of a tile
/// boundary, which IS a legitimate workflow. A mutation of the real fixture,
/// never hand-written CityJSON (the `railway_source_with_crs` idiom).
///
/// With `keep_child` false the child line is dropped entirely while the
/// parent's `children` array still names it — an unrepairable dangling
/// reference, the partial-area-extract case.
///
/// The returned `TempDir` MUST outlive the path: `Source` streams lazily.
fn delft_with_split_parent_and_part(
    extra_features: usize,
    keep_child: bool,
) -> (tempfile::TempDir, PathBuf, String, String) {
    let source = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = source.lines();
    let header = lines.next().unwrap().to_string();
    let first: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();

    let objects = first["CityObjects"].as_object().unwrap();
    let parent = objects
        .iter()
        .find(|(_, co)| co["type"] == "Building")
        .map(|(id, _)| id.clone())
        .expect("delft's first feature is a Building + BuildingPart");
    let child = objects
        .keys()
        .find(|id| **id != parent)
        .expect("delft's first feature has a BuildingPart")
        .clone();

    let one_object_line = |id: &str| {
        serde_json::to_string(&serde_json::json!({
            "type": "CityJSONFeature",
            "id": id,
            "CityObjects": { id: objects[id] },
            "vertices": first["vertices"],
        }))
        .unwrap()
    };

    let mut out = vec![header, one_object_line(&parent)];
    if keep_child {
        out.push(one_object_line(&child));
    }
    out.extend(lines.take(extra_features).map(str::to_string));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("split_parent.city.jsonl");
    std::fs::write(&path, out.join("\n") + "\n").unwrap();
    (dir, path, parent, child)
}

/// RED (b): a parent and its child arriving as SEPARATE features must still be
/// written into one partition package. Before the reference-locality repair
/// they were assigned by feature index alone, so `Count(2)` put them in
/// `count-00000` and `count-00001` — two packages, each holding a dangling
/// reference, with nothing printed.
#[test]
fn a_parent_and_child_split_across_features_are_co_assigned_to_one_partition() {
    let out = tempfile::tempdir().unwrap();
    let (_dir, path, parent, child) = delft_with_split_parent_and_part(0, true);
    let src = Source::open(&path).unwrap();
    let opts = ConvertOptions::new(path.clone(), out.path().to_path_buf());
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();

    let labels: Vec<String> = rep.partitions.iter().map(|(l, _)| l.clone()).collect();
    assert_eq!(
        labels.len(),
        1,
        "the two features reference each other, so they must collapse to one \
         partition rather than being split by index; got {labels:?}"
    );
    let (ids, _) = package_ids_and_refs(&out.path().join(&labels[0]), out.path());
    assert!(
        ids.contains(&parent) && ids.contains(&child),
        "both objects in one package"
    );
    assert_references_are_package_local(out.path(), &labels, "split parent/child");

    assert_eq!(
        rep.co_assigned_features, 1,
        "one feature was moved off its index-derived label to keep the hierarchy together"
    );
    assert_eq!(rep.unresolvable_refs, 0);
}

/// Conformant input must not be perturbed: every delft feature is already
/// self-contained, so the repair moves nothing and reports nothing, and the
/// index-derived partition count is unchanged.
#[test]
fn self_contained_features_are_never_co_assigned() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = delft_opts(out.path());
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(4), &opts).unwrap();
    assert_eq!(
        rep.partitions.len(),
        4,
        "conformant input keeps its 4 index chunks"
    );
    assert_eq!(rep.co_assigned_features, 0);
    assert_eq!(rep.unresolvable_refs, 0);
}

/// A reference whose target is absent from the whole dataset (a partial-area
/// extract) cannot be repaired by co-assignment. It must be counted and
/// reported — never silently ignored, and never fatal: refusing would block a
/// legitimate extract.
#[test]
fn a_dangling_reference_is_counted_not_fatal() {
    let out = tempfile::tempdir().unwrap();
    let (_dir, path, _parent, _child) = delft_with_split_parent_and_part(3, false);
    let src = Source::open(&path).unwrap();
    let opts = ConvertOptions::new(path.clone(), out.path().to_path_buf());
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();
    assert_eq!(
        rep.unresolvable_refs, 1,
        "the orphaned parent still names a child that is nowhere in the dataset"
    );
    assert_eq!(
        rep.co_assigned_features, 0,
        "there is nothing to co-assign it with"
    );
    assert!(!rep.partitions.is_empty(), "the conversion still succeeds");
}

/// Build a small on-disk CityJSONSeq derivative of the real
/// `delft.city.jsonl`: its header plus the first `features` feature lines,
/// with — when `unsupported_type` is set — ONE object's LoD0 `MultiSurface`
/// rewritten into that geometry type over the SAME real vertex indices (each
/// surface's exterior ring becomes one line). Real coordinates, real
/// attributes, real semantics elsewhere; only the one geometry's `type` and
/// nesting depth are transformed, so this stays a mutation of the real
/// fixture rather than hand-written CityJSON (the same idiom
/// `railway_source_with_crs` above already uses).
///
/// The returned `TempDir` MUST outlive the path — `Source` streams lazily
/// from it.
fn delft_subset_with_optional_multilinestring(
    features: usize,
    unsupported_type: Option<&str>,
) -> (tempfile::TempDir, PathBuf) {
    let source = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines: Vec<String> = source
        .lines()
        .take(features + 1) // + the header line
        .map(str::to_string)
        .collect();

    if let Some(target_type) = unsupported_type {
        let mut rewritten = false;
        'lines: for line in lines.iter_mut().skip(1) {
            let mut doc: serde_json::Value = serde_json::from_str(line).unwrap();
            let objects = doc["CityObjects"].as_object_mut().unwrap();
            for (_id, co) in objects.iter_mut() {
                let Some(geometries) = co["geometry"].as_array_mut() else {
                    continue;
                };
                for geom in geometries.iter_mut() {
                    if geom["type"] != "MultiSurface" {
                        continue;
                    }
                    // MultiSurface boundaries are surface -> ring -> index;
                    // a MultiLineString's are line -> index, so taking each
                    // surface's exterior ring drops exactly one level and
                    // keeps every index pointing at a real vertex.
                    let lines_boundaries: Vec<serde_json::Value> = geom["boundaries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|surface| surface.as_array().unwrap()[0].clone())
                        .collect();
                    geom["type"] = serde_json::json!(target_type);
                    geom["boundaries"] = serde_json::Value::Array(lines_boundaries);
                    // Surface semantics/appearance are meaningless on a
                    // curve geometry — drop them rather than leave a
                    // mismatched index map behind.
                    let geom_obj = geom.as_object_mut().unwrap();
                    geom_obj.remove("semantics");
                    geom_obj.remove("material");
                    geom_obj.remove("texture");
                    rewritten = true;
                    break;
                }
                if rewritten {
                    break;
                }
            }
            if rewritten {
                *line = serde_json::to_string(&doc).unwrap();
                break 'lines;
            }
        }
        assert!(
            rewritten,
            "the delft subset must contain at least one MultiSurface to rewrite"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_subset.city.jsonl");
    std::fs::write(&path, lines.join("\n")).unwrap();
    (dir, path)
}

/// Whole-branch review finding 2 (data loss): the canonical scan
/// `convert_partitioned` runs BEFORE purging a previous run's partitions used
/// to validate every geometry through the WKB encoder regardless of the
/// conversion's chosen `GeometryEncoding`. A `MultiLineString` source is
/// perfectly valid WKB but explicitly outside the arrow-native encoding's
/// phase-1 type scope, so under `--geometry-encoding arrow-native` it passed
/// that scan, the previous run's complete partitions were deleted, and only
/// THEN did the encode pass reject the geometry — destroying valid output on
/// the strength of a scan that validated the wrong encoding.
///
/// The conversion must still fail (the type genuinely is unsupported); what
/// must not happen is the deletion.
#[test]
fn arrow_native_unsupported_type_fails_before_overwrite_deletes_existing_partitions() {
    let out = tempfile::tempdir().unwrap();

    // A complete, valid prior run under the same encoding.
    let (_clean_dir, clean) = delft_subset_with_optional_multilinestring(40, None);
    let clean_src = Source::open(&clean).unwrap();
    let mut opts = ConvertOptions::new(clean.clone(), out.path().to_path_buf());
    opts.geometry_encoding = cityparquet_schema::GeometryEncoding::ArrowNative;
    let prior = convert_partitioned(
        std::slice::from_ref(&clean_src),
        &PartitionSpec::Count(2),
        &opts,
    )
    .unwrap();
    let prior_labels: Vec<String> = prior.partitions.iter().map(|(l, _)| l.clone()).collect();
    assert!(!prior_labels.is_empty());
    for label in &prior_labels {
        assert!(
            out.path().join(label).join("building.parquet").exists(),
            "prior run must have written {label}/building.parquet"
        );
    }

    // The same data, one geometry rewritten to a type arrow-native phase 1
    // does not support.
    let (_bad_dir, bad) = delft_subset_with_optional_multilinestring(40, Some("MultiLineString"));
    let bad_src = Source::open(&bad).unwrap();
    let mut bad_opts = ConvertOptions::new(bad.clone(), out.path().to_path_buf());
    bad_opts.geometry_encoding = cityparquet_schema::GeometryEncoding::ArrowNative;
    bad_opts.overwrite = true;
    let err = convert_partitioned(
        std::slice::from_ref(&bad_src),
        &PartitionSpec::Count(2),
        &bad_opts,
    )
    .expect_err("MultiLineString is outside the arrow-native phase-1 type scope");
    let msg = err.to_string();
    assert!(
        msg.contains("MultiLineString"),
        "the error should name the unsupported geometry type, got: {msg}"
    );

    for label in &prior_labels {
        assert!(
            out.path().join(label).join("building.parquet").exists(),
            "{label}/building.parquet was destroyed by an overwrite whose canonical scan never \
             validated the geometry against the encoding it was actually converting to"
        );
    }
}

/// The control for the test above: the very same rewritten source converts
/// cleanly under the DEFAULT WKB encoding, where `MultiLineString` is fully
/// supported. This pins that the rejection is encoding-specific — the WKB
/// path's scan-time validation is untouched — and that the mutated fixture is
/// otherwise valid CityJSON, so the arrow-native failure above is about the
/// type scope and nothing else.
#[test]
fn the_same_multilinestring_source_partitions_cleanly_under_wkb() {
    let out = tempfile::tempdir().unwrap();
    let (_bad_dir, bad) = delft_subset_with_optional_multilinestring(40, Some("MultiLineString"));
    let src = Source::open(&bad).unwrap();
    let opts = ConvertOptions::new(bad.clone(), out.path().to_path_buf());
    let rep =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(2), &opts).unwrap();
    assert_eq!(rep.partitions.len(), 2);
}
