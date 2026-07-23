use std::path::PathBuf;

use arrow_array::{
    Array, BinaryArray, Int32Array, ListArray, RecordBatch, StringArray, StructArray,
};
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

/// The `geometry_properties[_lod*]` `STRUCT` column of `batch`, by name.
fn props_struct<'a>(batch: &'a RecordBatch, name: &str) -> &'a StructArray {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"))
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap_or_else(|| panic!("{name} is not a Struct array"))
}

/// One row's `type` child value (the struct itself is non-null at `row`).
fn props_type(s: &StructArray, row: usize) -> &str {
    let a = s.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    a.value(row)
}

/// One row's `face_semantics` child (`LIST<INT>`, items nullable).
fn props_face_semantics(s: &StructArray, row: usize) -> Option<Vec<Option<i32>>> {
    if s.is_null(row) {
        return None;
    }
    let a = s.column(2).as_any().downcast_ref::<ListArray>().unwrap();
    if a.is_null(row) {
        return None;
    }
    let items = a.value(row);
    let ints = items.as_any().downcast_ref::<Int32Array>().unwrap();
    Some(
        (0..ints.len())
            .map(|i| (!ints.is_null(i)).then(|| ints.value(i)))
            .collect(),
    )
}

/// One row's `shells` child (`LIST<LIST<INT>>`, non-null at both nesting
/// levels once populated) — one inner `Vec<i32>` per solid.
fn props_shells(s: &StructArray, row: usize) -> Option<Vec<Vec<i32>>> {
    if s.is_null(row) {
        return None;
    }
    let a = s.column(3).as_any().downcast_ref::<ListArray>().unwrap();
    if a.is_null(row) {
        return None;
    }
    let solids = a.value(row);
    let solids = solids.as_any().downcast_ref::<ListArray>().unwrap();
    Some(
        (0..solids.len())
            .map(|i| {
                let counts = solids.value(i);
                let counts = counts.as_any().downcast_ref::<Int32Array>().unwrap();
                (0..counts.len()).map(|j| counts.value(j)).collect()
            })
            .collect(),
    )
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), tests below open a small on-disk COPY
/// with a CRS injected via JSON mutation of the real fixture — never
/// hand-written CityJSON. `Source` streams CityJSONSeq lazily from `path`
/// (see `crate::source::Source::features`), so the returned `TempDir` MUST
/// outlive the `Source` — callers keep it bound, never `_`-discarded.
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

#[test]
fn delft_encodes_all_objects_in_batches() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 512, false)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(rows, 2231);
    assert!(batches.len() >= 2231 / 512);
    let schema = s.schema.to_arrow_schema_tagged(false).unwrap();
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

/// delft carries a real LoD `0` footprint. It must land in the un-suffixed
/// `geometry` column (the GeoParquet-legal primary slot, §9), and — because a
/// bare column name cannot encode the LoD — its `geometry_properties` must
/// carry `"lod":"0"` (§12's additional-keys mechanism), so decode/export can
/// recover the LoD.
/// spec "Levels of detail": every LoD, including LoD0, is a suffixed column —
/// there is no un-suffixed `geometry` column, and the LoD is never re-stored
/// as a value inside `geometry_properties` (it lives only in the column name).
#[test]
fn delft_lod0_lands_in_a_suffixed_column_with_no_lod_in_properties() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 4096, false)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let b = &batches[0];
    assert!(
        b.column_by_name("geometry").is_none(),
        "no bare/un-suffixed geometry column must ever appear"
    );
    let geom = b
        .column_by_name("geometry_lod0_0")
        .expect("geometry_lod0_0 column exists for LoD0");
    assert!(
        geom.null_count() < b.num_rows(),
        "some LoD0 geometry present in the geometry_lod0_0 column"
    );
    let props = props_struct(b, "geometry_properties_lod0_0");
    let first = (0..b.num_rows())
        .find(|&i| props.is_valid(i))
        .expect("at least one non-null geometry_properties_lod0_0");
    // The struct has no `lod` field at all (spec: it declares exactly
    // type/surfaces/face_semantics/shells, structurally — a reader never
    // needs to check for a stray value, only the schema).
    let field_names: Vec<&str> = props.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        field_names,
        vec!["type", "surfaces", "face_semantics", "shells"],
        "the LoD lives only in the column name, never a geometry_properties field"
    );
    assert!(!props_type(props, first).is_empty());
}

#[test]
fn railway_encodes_with_semantics_and_templates() {
    let (_crs_dir, src) = railway_source_with_crs();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 1024, false)
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
    let (_crs_dir, src) = railway_source_with_crs();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 1024, false)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let mut found = 0usize;
    for batch in &batches {
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let materials = batch
            .column_by_name("material_lod3_0")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let geom = batch
            .column_by_name("geometry_lod3_0")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        for row in 0..batch.num_rows() {
            let id = ids.value(row);
            // The dropped positions themselves are no longer part of
            // geometry_properties (spec: exactly type/surfaces/
            // face_semantics/shells, no non-normative drop diagnostic) —
            // EncodeStats aggregates the counts instead. What's checked
            // here is that the STORED geometry itself lost exactly the
            // dropped faces, proving the realignment ran (checked against
            // the decoded WKB rather than face_semantics, since not every
            // one of these rows carries semantics).
            let expected_stored_faces = match id {
                "GMLID_855011_330784_753" | "GMLID_0373494_301709_129" => 100,
                "UUID_d96effed-08fe-4f74-b134-05b194aa3cff" => 22022 - 4,
                _ => continue,
            };
            found += 1;
            assert!(!geom.is_null(row), "{id}: expected geometry_lod3_0");
            let decoded = cityparquet::wkb_read::wkb_to_geometry(geom.value(row)).unwrap();
            let cityparquet::wkb_read::DecodedKind::MultiPolygon(faces) = decoded.kind else {
                panic!("{id}: expected a MultiPolygon-decoded WKB");
            };
            assert_eq!(
                faces.len(),
                expected_stored_faces,
                "the stored geometry must lose exactly the dropped faces for {id}"
            );
            if id != "UUID_d96effed-08fe-4f74-b134-05b194aa3cff" {
                let material: serde_json::Value =
                    serde_json::from_str(materials.value(row)).unwrap();
                let values = material["visual"]["values"]
                    .as_array()
                    .expect("per-surface material values array");
                assert_eq!(
                    values.len(),
                    100,
                    "material values must be realigned after dropping surface 67 (source had 101) for {id}"
                );
            }
        }
    }
    assert_eq!(found, 3, "all three degenerate-drop rows must be found");
}

#[test]
fn delft_records_per_shell_face_partition_for_solids() {
    // delft.city.jsonl carries plain `Solid` geometry (no MultiSolid /
    // CompositeSolid), so its WKB is a single top-level PolyhedralSurfaceZ:
    // the face count sits at bytes 5..9 of the geometry_lod* bytes for the
    // SAME row, letting us check `shells` against ground truth
    // without re-deriving it from the CityJSON boundaries.
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 512, false)
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
            let props = props_struct(batch, name);
            let geom = geom_col.as_any().downcast_ref::<BinaryArray>().unwrap();

            for row in 0..batch.num_rows() {
                if props.is_null(row) {
                    continue;
                }
                if props_type(props, row) != "Solid" {
                    continue;
                }

                let solids = props_shells(props, row)
                    .unwrap_or_else(|| panic!("shells missing for Solid row {row} in {name}"));
                // A Solid always nests exactly one inner list — its own
                // (spec: nested one inner list per solid, even for a lone
                // Solid — never the old flat [n0, n1, ...] shape).
                assert_eq!(
                    solids.len(),
                    1,
                    "a Solid's shells must nest exactly one inner list, row {row} in {name}"
                );
                let faces = &solids[0];
                assert!(!faces.is_empty(), "shells must be non-empty");
                let mut sum: u64 = 0;
                for &n in faces {
                    assert!(n > 0, "shells entries must be positive");
                    sum += n as u64;
                }

                assert!(!geom.is_null(row), "Solid row must carry geometry bytes");
                let wkb = geom.value(row);
                assert!(wkb.len() >= 9, "WKB too short to hold a header");
                let face_count = u32::from_le_bytes(wkb[5..9].try_into().unwrap()) as u64;
                assert_eq!(
                    sum, face_count,
                    "sum of shells must equal the WKB PolyhedralSurfaceZ face count"
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

/// M4 task 4: Solid-family semantics/material/texture realignment when the
/// writer drops a degenerate face. Derived-from-real-fixture: delft's
/// `NL.IMBAG.Pand.0503100000012869-0` carries a real lod-1.2 Solid — a
/// single shell of 6 faces with `semantics.values == [[0,2,2,2,2,1]]` (real
/// delft carries no material/texture anywhere, so both are added here to
/// exercise the fix; the values arrays are shaped exactly like the real
/// semantics array). Face 2's exterior ring ([5,2,3,6] in the real fixture)
/// is degenerated to `[a, b, a]` from its own first two indices, so the
/// writer drops exactly that face; every per-face array must lose exactly
/// that one entry, in order, while the shell nesting survives.
#[test]
fn delft_derived_solid_realigns_semantics_material_and_texture_for_dropped_face() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let header_line = lines.next().unwrap().to_string();

    const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
    let mut mutated_line = None;
    for line in lines {
        if !line.contains(OBJ_ID) {
            continue;
        }
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        let geom = feature["CityObjects"][OBJ_ID]["geometry"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["lod"] == "1.2" && g["type"] == "Solid")
            .expect("delft's Pand-0 must carry a lod 1.2 Solid");

        let sem_values: Vec<i64> =
            serde_json::from_value(geom["semantics"]["values"][0].clone()).unwrap();
        assert_eq!(sem_values.len(), 6, "fixture fact: shell 0 has 6 faces");

        let ring = &mut geom["boundaries"][0][2][0];
        let indices: Vec<i64> = serde_json::from_value(ring.clone()).unwrap();
        let (a, b) = (indices[0], indices[1]);
        *ring = serde_json::json!([a, b, a]);

        geom["material"] = serde_json::json!({"visual": {"values": [[0, 1, 0, 1, 0, 1]]}});
        geom["texture"] = serde_json::json!({"visual": {"values": [[
            [[0, 0, 1, 2, 3]], [[0, 4, 5, 6, 7]], [[0, 8, 9, 10, 11]],
            [[0, 12, 13, 14, 15]], [[0, 16, 17, 18, 19]], [[0, 20, 21, 22, 23]]
        ]]}});
        feature["appearance"] = serde_json::json!({
            "materials": [
                {"name": "mat0", "ambientIntensity": 0.5},
                {"name": "mat1", "ambientIntensity": 0.8}
            ],
            "textures": [{"type": "PNG", "image": "tex0.png"}],
            "vertices-texture":
                (0..24).map(|i| serde_json::json!([i as f64 / 24.0, 0.0])).collect::<Vec<_>>()
        });

        mutated_line = Some(serde_json::to_string(&feature).unwrap());
        break;
    }
    let mutated_line = mutated_line.expect("delft.city.jsonl must contain the target object");

    // The UV coordinates the interner inlines are read back from whatever
    // `Source::open` re-parses out of the written file's JSON text, not
    // from the `i as f64 / 24.0` Rust expression that built them: JSON
    // number parsing is not guaranteed bit-exact for every literal (some
    // values round-trip 1 ULP off), so the expected values below are
    // re-parsed from the SAME text via the same JSON parse path, rather
    // than recomputed independently.
    let mutated_value: serde_json::Value = serde_json::from_str(&mutated_line).unwrap();
    let vertices_texture: Vec<serde_json::Value> = mutated_value["appearance"]["vertices-texture"]
        .as_array()
        .expect("mutated feature carries vertices-texture")
        .clone();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_solid_derived.city.jsonl");
    std::fs::write(&path, format!("{header_line}\n{mutated_line}\n")).unwrap();

    let src = Source::open(&path).unwrap();
    let s = scan(&src).unwrap();
    let batches: Vec<_> = encode(&src, &s, 64, false)
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
        let Some(row) = (0..batch.num_rows()).find(|&r| ids.value(r) == OBJ_ID) else {
            continue;
        };
        found = true;

        let props = props_struct(batch, "geometry_properties_lod1_2");
        // The dropped ring/surface positions themselves are no longer part
        // of geometry_properties (spec: exactly type/surfaces/
        // face_semantics/shells) — EncodeStats aggregates that instead;
        // what's checked here is that shells/face_semantics were actually
        // realigned to the STORED (post-drop) geometry.
        assert_eq!(
            props_shells(props, row),
            Some(vec![vec![5]]),
            "the single shell drops from 6 to 5 faces, nested one list per solid"
        );
        assert_eq!(
            props_face_semantics(props, row),
            Some(vec![Some(0), Some(2), Some(2), Some(2), Some(1)]),
            "face_semantics is the flat per-face list, losing face 2's entry"
        );

        let material = batch
            .column_by_name("material_lod1_2")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let material: serde_json::Value = serde_json::from_str(material.value(row)).unwrap();
        assert_eq!(
            material["visual"]["values"][0],
            serde_json::json!([0, 1, 1, 0, 1]),
            "material values must be realigned within the shell nesting"
        );

        let texture = batch
            .column_by_name("texture_lod1_2")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let texture: serde_json::Value = serde_json::from_str(texture.value(row)).unwrap();
        // Post-interner: the texture index (only one def in this isolated
        // fixture, so its dataset-global id is 0) and every UV index are
        // inlined as `[u, v]` pairs from the re-parsed vertices-texture
        // pool (see the comment where `vertices_texture` is built above).
        let uv = |i: usize| vertices_texture[i].clone();
        assert_eq!(
            texture["visual"]["values"][0],
            serde_json::json!([
                [[0, uv(0), uv(1), uv(2), uv(3)]],
                [[0, uv(4), uv(5), uv(6), uv(7)]],
                [[0, uv(12), uv(13), uv(14), uv(15)]],
                [[0, uv(16), uv(17), uv(18), uv(19)]],
                [[0, uv(20), uv(21), uv(22), uv(23)]]
            ]),
            "texture values must be realigned within the shell nesting, globally rewritten, and UV-inlined"
        );
    }
    assert!(
        found,
        "the target object row must be present in the encoded batches"
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
    let mut it = encode(&src, &s, 64, false).unwrap();
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
