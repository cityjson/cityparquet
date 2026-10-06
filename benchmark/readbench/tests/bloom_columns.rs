//! `bloom_columns`: which columns of a real package carry a Bloom filter,
//! against the delft fixture written the way the bloom axis writes its
//! packages (Hilbert order, no LoD 0 synthesis).

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet_readbench::bloom_columns::{
    WRITER_RATIO, main_table, render, require_filtered_text, survey,
};

fn delft_package() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures/delft.city.jsonl");
    assert!(
        input.exists(),
        "missing delft.city.jsonl; run `just fixtures`"
    );
    let out = dir.path().join("delft.parquet");
    let mut opts = ConvertOptions::new(input, out.clone());
    opts.ordering = RowOrder::Hilbert;
    opts.generate_lod0 = false;
    convert(&opts).unwrap();
    (dir, out)
}

#[test]
fn lists_the_filtered_columns_and_the_text_columns_without_one() {
    let (_dir, package) = delft_package();
    let s = survey(&main_table(&package).unwrap()).unwrap();
    assert_eq!(s.row_groups, 1);
    let filtered: Vec<&str> = s.with_filter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        filtered,
        ["id", "feature_id", "documentnummer", "identificatie"]
    );
    let identificatie = s.column("identificatie").unwrap();
    assert_eq!(identificatie.non_null, Some(1115));
    assert_eq!(identificatie.distinct, Some(1115));
    assert!(identificatie.filter_bytes > 0);
    // The writer's rule, seen on the exact counts: every filtered attribute
    // clears the threshold, every text column without a filter falls short.
    for c in s
        .with_filter()
        .filter(|c| c.name != "id" && c.name != "feature_id")
    {
        assert!(c.ratio().unwrap() >= WRITER_RATIO, "{c:?}");
    }
    for c in s.text_without_filter() {
        assert!(c.ratio().unwrap_or(0.0) < WRITER_RATIO, "{c:?}");
    }
    let listing = render(&s);
    assert!(listing.contains("columns WITH a Bloom filter"), "{listing}");
    assert!(listing.contains("b3_dak_type"), "{listing}");
}

#[test]
fn a_probe_column_must_exist_be_text_and_carry_a_filter() {
    let (_dir, package) = delft_package();
    let s = survey(&main_table(&package).unwrap()).unwrap();
    require_filtered_text(&s, "identificatie").unwrap();
    let err = |column: &str| require_filtered_text(&s, column).unwrap_err().to_string();
    assert!(
        err("status").contains("'status' carries no Bloom filter"),
        "{}",
        err("status")
    );
    assert!(
        err("no_such").contains("'no_such' is not a column"),
        "{}",
        err("no_such")
    );
    assert!(
        err("b3_h_maaiveld").contains("not a text column"),
        "{}",
        err("b3_h_maaiveld")
    );
}
