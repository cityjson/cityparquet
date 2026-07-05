use std::path::PathBuf;

use arrow_array::{Array, BinaryArray, StringArray};
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

/// The writer drops structurally degenerate surfaces ([a,b,a]-shaped
/// exterior rings) at write time; the stored material per-surface arrays
/// and geometry_properties must be realigned/annotated to match.
/// GMLID_855011_330784_753 in lod3_railway has 101 source surfaces with a
/// per-surface material values array; surface 67 is degenerate.
#[test]
fn railway_realigns_material_values_for_dropped_surfaces() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 1024)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let mut found = false;
    for batch in &batches {
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let materials = batch
            .column_by_name("material")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let props = batch
            .column_by_name("geometry_properties_lod3")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for row in 0..batch.num_rows() {
            if ids.value(row) != "GMLID_855011_330784_753" {
                continue;
            }
            found = true;
            let material: serde_json::Value = serde_json::from_str(materials.value(row)).unwrap();
            let values = material["3"]["visual"]["values"]
                .as_array()
                .expect("per-surface material values array");
            assert_eq!(
                values.len(),
                100,
                "material values must be realigned after dropping surface 67 (source had 101)"
            );
            let p: serde_json::Value = serde_json::from_str(props.value(row)).unwrap();
            assert_eq!(
                p["dropped_degenerate"],
                serde_json::json!({"rings": 1, "surfaces": [67]}),
                "geometry_properties must record what was dropped"
            );
        }
    }
    assert!(found, "GMLID_855011_330784_753 row not found");
}

#[test]
fn delft_records_per_shell_face_partition_for_solids() {
    // delft.city.jsonl carries plain `Solid` geometry (no MultiSolid /
    // CompositeSolid), so its WKB is a single top-level PolyhedralSurfaceZ:
    // the face count sits at bytes 5..9 of the geometry_lod* bytes for the
    // SAME row, letting us check `solid_shell_faces` against ground truth
    // without re-deriving it from the CityJSON boundaries.
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 512)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let mut checked = 0usize;
    for batch in &batches {
        let schema = batch.schema();
        for field in schema.fields() {
            let name = field.name().as_str();
            let Some(suffix) = name.strip_prefix("geometry_properties") else {
                continue;
            };
            let geom_col_name = format!("geometry{suffix}");
            let Some(geom_col) = batch.column_by_name(&geom_col_name) else {
                continue;
            };
            let props = batch
                .column_by_name(name)
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let geom = geom_col.as_any().downcast_ref::<BinaryArray>().unwrap();

            for row in 0..batch.num_rows() {
                if props.is_null(row) {
                    continue;
                }
                let json: serde_json::Value = serde_json::from_str(props.value(row)).unwrap();
                if json.get("type").and_then(|t| t.as_str()) != Some("Solid") {
                    continue;
                }

                let faces = json
                    .get("solid_shell_faces")
                    .unwrap_or_else(|| {
                        panic!("solid_shell_faces missing for Solid row {row} in {name}")
                    })
                    .as_array()
                    .expect("solid_shell_faces must be a JSON array");
                assert!(!faces.is_empty(), "solid_shell_faces must be non-empty");
                let mut sum: u64 = 0;
                for f in faces {
                    let n = f
                        .as_u64()
                        .expect("solid_shell_faces entries must be positive integers");
                    assert!(n > 0, "solid_shell_faces entries must be positive");
                    sum += n;
                }

                assert!(!geom.is_null(row), "Solid row must carry geometry bytes");
                let wkb = geom.value(row);
                assert!(wkb.len() >= 9, "WKB too short to hold a header");
                let face_count = u32::from_le_bytes(wkb[5..9].try_into().unwrap()) as u64;
                assert_eq!(
                    sum, face_count,
                    "sum of solid_shell_faces must equal the WKB PolyhedralSurfaceZ face count"
                );

                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "expected at least one Solid row across all geometry_properties* columns"
    );
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
