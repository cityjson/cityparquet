use std::path::PathBuf;

use cityparquet::encode::encode;
use cityparquet::scan::scan;
use cityparquet::source::Source;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn delft_encodes_all_objects_in_batches() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 512)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(rows, 2231);
    assert!(batches.len() >= 2231 / 512);
    let schema = s.schema.to_arrow_schema().unwrap();
    assert_eq!(batches[0].schema().fields(), schema.fields());
    // Spot checks on real content:
    let b = &batches[0];
    let ids = b.column_by_name("id").unwrap();
    assert!(ids.null_count() == 0);
    let geom22 = b.column_by_name("geometry_lod2_2").unwrap();
    assert!(geom22.null_count() < b.num_rows()); // some LoD2.2 geometry present
    let bbox = b.column_by_name("bbox").unwrap();
    assert!(bbox.null_count() < b.num_rows());
}

#[test]
fn railway_encodes_with_semantics_and_templates() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 1024)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(rows, 121);
}
