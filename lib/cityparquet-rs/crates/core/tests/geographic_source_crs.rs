//! A CityJSON source may declare a geographic (degree-valued) CRS itself.
//!
//! Such a source is **encodable**: nothing here reprojects, but the
//! quantisation step is derived per axis from the CRS's own declared units
//! ([`cityparquet_schema::crs::axis_scale`]), so a degree axis gets a
//! degree-sized step. What the writer still owes such a source is
//! GeoParquet's `(x, y) = (longitude, latitude)` order, since EPSG declares
//! `4326`/`4979`/`4258` latitude-first.
//!
//! What remains refused is a CRS whose units have no defined step at all, and
//! that refusal must land in the SCAN — before any output is touched —
//! because a failure written as a success is the worst outcome this pipeline
//! can produce.

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert_source};
use cityparquet::source::Source;

fn delft() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/delft.city.jsonl")
}

/// The real Delft fixture with its `referenceSystem` rewritten to `code` and
/// its coordinates left alone. Degree-valued input in the wild looks exactly
/// like this: a header that says WGS 84 over a body the writer will quantise.
fn fixture_declaring(dir: &Path, code: &str, name: &str) -> PathBuf {
    let text = fs::read_to_string(delft()).expect("fixture must exist; run `just fixtures`");
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    header["metadata"]
        .as_object_mut()
        .expect("delft header has metadata")
        .insert(
            "referenceSystem".to_string(),
            serde_json::json!(format!("https://www.opengis.net/def/crs/EPSG/0/{code}")),
        );
    let mut out = serde_json::to_string(&header).unwrap();
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    let dest = dir.join(name);
    fs::write(&dest, out).unwrap();
    dest
}

#[test]
fn a_source_declaring_a_geographic_crs_converts_longitude_first() {
    let tmp = tempfile::tempdir().unwrap();
    // 4326 (WGS 84), 4979 (its 3D form — american-cities-3d declares exactly
    // this) and 4258 (ETRS89) are all degree-valued, and all latitude-first.
    for code in ["4326", "4979", "4258"] {
        let input = fixture_declaring(tmp.path(), code, &format!("geo{code}.city.jsonl"));
        let source = Source::open(&input).unwrap();
        let out = tmp.path().join(format!("out{code}"));
        let opts = ConvertOptions::new(input.clone(), out.clone());
        convert_source(&source, &opts).unwrap_or_else(|e| panic!("EPSG:{code} must convert: {e}"));

        // The fixture's own first ordinate is what EPSG:{code} calls latitude,
        // so the WKB must carry it in `y`, not `x`. Comparing the package's
        // stored order against the SOURCE's own order is the whole assertion —
        // the fixture's coordinates are not really degrees, and do not need to
        // be for the axis rule to be testable.
        let (src_x, src_y) = first_source_ordinates(&input);
        let (wkb_x, wkb_y) = first_wkb_ordinates(&out);
        assert!(
            (wkb_x - src_y).abs() < 1e-3 && (wkb_y - src_x).abs() < 1e-3,
            "EPSG:{code}: source ({src_x}, {src_y}) must be stored swapped, got ({wkb_x}, {wkb_y})"
        );
    }
}

#[test]
fn a_source_in_an_unencodable_unit_is_refused() {
    // EPSG:4035 is in "degree minute second hemisphere" — a packed sexagesimal
    // spelling, not a scale anything can be quantised on. Refusing it is the
    // same stance the spec's CRS rules take on a source with no CRS at all.
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_declaring(tmp.path(), "4035", "sexagesimal.city.jsonl");
    let source = Source::open(&input).unwrap();
    let opts = ConvertOptions::new(input.clone(), tmp.path().join("out"));
    let err = convert_source(&source, &opts)
        .expect_err("a source in an unencodable unit must not convert 'successfully'");
    assert!(err.to_string().contains("minute second"), "got: {err}");
}

/// The first vertex of the fixture's first feature, dequantised, as the SOURCE
/// orders its axes.
fn first_source_ordinates(path: &Path) -> (f64, f64) {
    let source = Source::open(path).unwrap();
    let t = &source.header().transform;
    let feature = source.features().unwrap().next().unwrap().unwrap();
    let v = &feature.vertices[0];
    (
        v[0] as f64 * t.scale[0] + t.translate[0],
        v[1] as f64 * t.scale[1] + t.translate[1],
    )
}

/// The first coordinate of the first geometry the package stores, as WKB
/// orders its axes.
fn first_wkb_ordinates(pkg: &Path) -> (f64, f64) {
    use arrow_array::{Array, BinaryArray};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    for entry in fs::read_dir(pkg).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("parquet") {
            continue;
        }
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for (i, field) in batch.schema().fields().iter().enumerate() {
                if !field.name().starts_with("geometry_lod") {
                    continue;
                }
                let Some(col) = batch.column(i).as_any().downcast_ref::<BinaryArray>() else {
                    continue;
                };
                for row in 0..col.len() {
                    if col.is_null(row) {
                        continue;
                    }
                    let d = cityparquet::wkb_read::wkb_to_geometry(col.value(row)).unwrap();
                    if let Some(c) = d.coords.first() {
                        return (c[0], c[1]);
                    }
                }
            }
        }
    }
    panic!("package carried no WKB geometry");
}

#[test]
fn an_unresolvable_crs_is_declared_null_not_fatal() {
    // Spec "CRS rules": "An unresolvable CRS is declared, not fatal." A source
    // whose identifier the writer cannot resolve to PROJJSON converts, and the
    // package says so with an explicit `city.crs: null` plus a diagnostic —
    // deriving the quantisation step from a CRS must not turn an unresolvable
    // one into a refusal.
    let tmp = tempfile::tempdir().unwrap();
    // 1 is not an EPSG CRS code, and is not in the vendored table.
    let input = fixture_declaring(tmp.path(), "1", "unresolvable.city.jsonl");
    let source = Source::open(&input).unwrap();
    let out = tmp.path().join("out");
    let report = convert_source(&source, &ConvertOptions::new(input, out.clone()))
        .expect("an unresolvable CRS must not fail the conversion");
    assert!(
        report.crs_diagnostic.is_some(),
        "the writer SHOULD surface a conversion diagnostic"
    );
}

#[test]
fn a_projected_source_still_converts() {
    // The guard must not catch a projected national CRS: 7415 is
    // Amersfoort/RD New + NAP, what the real fixture declares.
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_declaring(tmp.path(), "7415", "projected.city.jsonl");
    let source = Source::open(&input).unwrap();
    let opts = ConvertOptions::new(input.clone(), tmp.path().join("out"));
    convert_source(&source, &opts).expect("a projected source must still convert");
}

#[test]
fn the_refusal_comes_before_a_prior_package_is_destroyed() {
    // The refusal is a scan-time one, so it lands before the overwrite: an
    // operator who re-runs a good conversion with a bad source keeps the good
    // output. (The same invariant `ensure_parent_ready` documents for
    // partitions, and the reason the check belongs in the scan rather than in
    // the writer.)
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let good = fixture_declaring(tmp.path(), "7415", "good.city.jsonl");
    let source = Source::open(&good).unwrap();
    convert_source(&source, &ConvertOptions::new(good, out.clone())).expect("the first run");
    let table = out.join("building.parquet");
    let before = fs::metadata(&table).unwrap().len();

    let bad = fixture_declaring(tmp.path(), "4035", "bad.city.jsonl");
    let source = Source::open(&bad).unwrap();
    let mut opts = ConvertOptions::new(bad, out.clone());
    opts.overwrite = true;
    convert_source(&source, &opts).expect_err("an unencodable source must fail");

    assert_eq!(
        fs::metadata(&table).unwrap().len(),
        before,
        "the prior good package was destroyed by a failing run"
    );
}
