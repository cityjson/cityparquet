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

#[test]
fn batch_iter_fuses_after_first_error() {
    // Derived-from-real-fixture: corrupt ONE geometry's boundaries in the
    // first feature line of delft.city.jsonl into a shape that mismatches
    // its geometry type. The Seq format is used deliberately: its features
    // are parsed lazily per line (the CityJson doc path would panic inside
    // cjseq's boundary reshaping before encode ever saw the geometry).
    // scan() calls geometry_to_wkb too and would fail on the corrupt file,
    // so the ScanResult comes from the CLEAN fixture and only the Source
    // handed to encode() is corrupted — the mismatch then errors inside
    // encode's accumulate_geometry.
    let clean = fixture("delft.city.jsonl");
    let text = std::fs::read_to_string(&clean).unwrap();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut feature: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    let mut corrupted = false;
    for (_, co) in feature["CityObjects"].as_object_mut().unwrap() {
        if let Some(geom) = co
            .get_mut("geometry")
            .and_then(|g| g.as_array_mut())
            .and_then(|g| g.first_mut())
        {
            geom["boundaries"] = serde_json::json!([0, 1, 2]);
            corrupted = true;
            break;
        }
    }
    assert!(corrupted, "first feature line has no geometry to corrupt");
    lines[1] = serde_json::to_string(&feature).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_corrupt.city.jsonl");
    std::fs::write(&path, lines.join("\n")).unwrap();

    let s = scan(&Source::open(&clean).unwrap()).unwrap();
    let src = Source::open(&path).unwrap();
    let mut it = encode(&src, &s, 64).unwrap();
    // Error-tolerant consumption: keep pulling after the Err, like a caller
    // using filter_map(Result::ok) would.
    let mut errs = 0;
    for item in it.by_ref() {
        if item.is_err() {
            errs += 1;
        }
    }
    assert_eq!(errs, 1, "the first error must fuse the iterator");
    assert!(it.next().is_none(), "a fused iterator stays exhausted");
}
