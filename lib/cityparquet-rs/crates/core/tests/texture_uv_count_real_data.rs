//! A texture ring whose UV list is shorter than the ring, from the real
//! Helsinki textured CityJSONSeq (`Helsinki_tex.city.jsonl`, City of
//! Helsinki, CC BY 4.0). `tests/data/helsinki_tex_uv_count.city.jsonl` is its
//! header line and three of its features (lines 16875, 16509 and 43164),
//! each with one such roof ring:
//!
//! - `BID_087ea02f…`: ring `[0, 2, 4, 6, 0]` with 4 UVs — explicitly closed;
//!   the closing repeat is no vertex of its own, so 4 UVs are one per vertex.
//! - `BID_e47016b2…`: ring `[0, 4, 4, 6, 1]` with 4 UVs — a vertex repeated in
//!   place, 5 ring positions but 4 UVs.
//! - `BID_ce1e6c36…`: ring `[0, 4, 6, 8, 1]` with 4 UVs — five distinct
//!   vertices (the last 1 mm from the first), 4 UVs.
//!
//! CityJSON 2.0.1 §6.2: "each array representing a ring has one more value
//! than the number of vertices in the ring". The last two rings break that,
//! and which vertex lacks a UV is not stated, so their texture cannot be
//! encoded: the conversion refuses them, and with
//! `--tolerate-invalid-appearance` leaves those rings untextured, counted.

use std::path::{Path, PathBuf};

use arrow_array::Array;
use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::validate::validate_package;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/helsinki_tex_uv_count.city.jsonl")
}

/// `(object id, textured rings of face 1)` → whether face 1's ring 0 carries a
/// texture id, per object, from `texture_lod2_0` of `building.parquet`.
fn roof_ring_textured(pkg: &Path) -> Vec<(String, bool, usize)> {
    let file = std::fs::File::open(pkg.join("building.parquet")).unwrap();
    let mut out = Vec::new();
    for batch in ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap()
    {
        let batch = batch.unwrap();
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_string::<i32>()
            .clone();
        let texture = batch
            .column_by_name("texture_lod2_0")
            .unwrap()
            .as_map()
            .clone();
        for row in 0..batch.num_rows() {
            if texture.is_null(row) {
                continue;
            }
            let themes = texture.value(row);
            let faces = themes.column(1).as_list::<i32>().value(0);
            let faces = faces.as_list::<i32>();
            // Textured rings over every face, and face 1's ring 0.
            let textured = (0..faces.len())
                .map(|f| {
                    let rings = faces.value(f);
                    let ids = rings
                        .as_struct()
                        .column(0)
                        .as_primitive::<Int64Type>()
                        .clone();
                    (0..ids.len()).filter(|&r| !ids.is_null(r)).count()
                })
                .sum();
            let face1 = faces.value(1);
            let roof = face1
                .as_struct()
                .column(0)
                .as_primitive::<Int64Type>()
                .clone();
            out.push((ids.value(row).to_string(), !roof.is_null(0), textured));
        }
    }
    out.sort();
    out
}

#[test]
fn a_ring_with_fewer_uvs_than_vertices_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let err = convert(&ConvertOptions::new(fixture(), dir.path().join("pkg")))
        .expect_err("a texture ring one UV short is invalid CityJSON");
    assert!(err.to_string().contains("uv indices"), "{err}");
}

#[test]
fn tolerating_invalid_appearance_leaves_such_a_ring_untextured() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    let mut opts = ConvertOptions::new(fixture(), pkg.clone());
    opts.tolerate_invalid_appearance = true;
    let report = convert(&opts).expect("tolerated");
    assert_eq!(report.invalid_appearance_refs_dropped, 2);

    let rings = roof_ring_textured(&pkg);
    let by_id = |prefix: &str| {
        rings
            .iter()
            .find(|(id, _, _)| id.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix} in {rings:?}"))
            .clone()
    };
    // The closed ring keeps its texture; the other two lose exactly that
    // ring's, and every other ring of theirs keeps its own.
    let closed = by_id("BID_087ea02f");
    assert!(closed.1, "{closed:?}");
    for prefix in ["BID_e47016b2", "BID_ce1e6c36"] {
        let (_, roof, textured) = by_id(prefix);
        assert!(!roof, "{prefix}: the short ring is untextured");
        assert_eq!(
            textured,
            closed.2 - 1,
            "{prefix}: only that ring loses its texture"
        );
    }

    let validation = validate_package(&pkg).unwrap();
    assert!(validation.violations.is_empty(), "{validation}");
}
