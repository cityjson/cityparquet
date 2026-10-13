//! Export streams a package family by family (spec 02 "Feature families are
//! contiguous"): only a family whose rows span object tables is held back
//! until its other tables arrive, and a package that breaks contiguity is
//! refused rather than exported with a feature split in two.

mod support;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use arrow_array::cast::AsArray;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};

use support::{FileContent, convert_fixture, fixture};

/// The feature ids whose rows appear in more than one object table of `pkg`.
fn families_spanning_tables(pkg: &Path) -> usize {
    let mut tables_of: HashMap<String, HashSet<String>> = HashMap::new();
    for entry in fs::read_dir(pkg).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".parquet") {
            continue;
        }
        let content = FileContent::read(&path);
        if content.schema.field_with_name("feature_id").is_err() {
            continue;
        }
        for batch in &content.batches {
            let feature_ids = batch
                .column_by_name("feature_id")
                .unwrap()
                .as_string::<i32>();
            for feature_id in feature_ids.iter().flatten() {
                tables_of
                    .entry(feature_id.to_string())
                    .or_default()
                    .insert(name.clone());
            }
        }
    }
    tables_of.values().filter(|t| t.len() > 1).count()
}

/// Railway's CityObjectGroup sits in `generics.parquet` while its members sit
/// in their own modules' tables: those families, and only those, are parked
/// across tables, and the streamed export still round-trips.
#[test]
fn railway_parks_exactly_its_cross_table_families_and_round_trips() {
    let (dir, pkg) = convert_fixture("lod3_railway.city.json");
    let spanning = families_spanning_tables(&pkg);
    assert!(spanning > 0, "railway has a family spanning tables");

    let output = dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: pkg.clone(),
        output: output.clone(),
    })
    .unwrap();
    assert_eq!(report.parked_features, spanning);

    let compared = compare_datasets(
        &fixture("lod3_railway.city.json"),
        &output,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(compared.equal, "{:#?}", compared.differences);
}

/// Delft's first building moved to the end of `building.parquet` leaves its
/// BuildingPart behind: export refuses to write the feature in two pieces.
#[test]
fn a_family_split_within_a_table_is_refused() {
    let (dir, pkg) = convert_fixture("delft.city.jsonl");
    let table = pkg.join("building.parquet");
    let mut content = FileContent::read(&table);
    let all = arrow_select::concat::concat_batches(&content.schema, &content.batches).unwrap();
    let n = all.num_rows() as u64;
    let order = arrow_array::UInt64Array::from((1..n).chain([0]).collect::<Vec<_>>());
    content.batches = vec![arrow_select::take::take_record_batch(&all, &order).unwrap()];
    content.write(&table);

    let err = export(&ExportOptions {
        package_dir: pkg,
        output: dir.path().join("export.city.jsonl"),
    })
    .expect_err("a split family must not be exported");
    assert!(format!("{err}").contains("not contiguous"), "{err}");
}
