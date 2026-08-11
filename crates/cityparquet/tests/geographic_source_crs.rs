//! A CityJSON source may declare a geographic (degree-valued) CRS itself.
//!
//! Nothing in this pipeline reprojects and the writer quantises at millimetre
//! scale, so a degree coordinate is destroyed by the encoding: 0.001° is about
//! 111 m, which collapses a whole city onto a handful of vertices. Every other
//! way of arriving at such a CRS is already refused — the CityGML `srsName`
//! resolver (`citygml/crs.rs`) and the operator's `--crs`
//! (`package::validate_crs_override`) — but the CityJSON `referenceSystem` was
//! not checked at all, so a degree-valued source converted with exit 0 and
//! wrote a corrupt package.
//!
//! That is the one defect the driver's ledger cannot catch: a failure recorded
//! as a success.

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
fn a_source_declaring_a_geographic_crs_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    // 4326 (WGS 84), 4979 (its 3D form — american-cities-3d declares exactly
    // this) and 4258 (ETRS89) are all degree-valued.
    for code in ["4326", "4979", "4258"] {
        let input = fixture_declaring(tmp.path(), code, &format!("geo{code}.city.jsonl"));
        let source = Source::open(&input).unwrap();
        let opts = ConvertOptions::new(input.clone(), tmp.path().join(format!("out{code}")));
        let err = convert_source(&source, &opts)
            .expect_err("a degree-valued source must not convert 'successfully'");
        // Lower-cased "geographic crs" is the substring the catalogue driver's
        // `convert.classify_error` bins as `geographic_crs`; the same wording
        // the CityGML resolver already uses. A different phrasing here would
        // silently land these items in the catch-all instead.
        let message = err.to_string().to_lowercase();
        assert!(
            message.contains("geographic crs"),
            "EPSG:{code}: got: {err}"
        );
    }
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

    let bad = fixture_declaring(tmp.path(), "4326", "bad.city.jsonl");
    let source = Source::open(&bad).unwrap();
    let mut opts = ConvertOptions::new(bad, out.clone());
    opts.overwrite = true;
    convert_source(&source, &opts).expect_err("a degree-valued source must fail");

    assert_eq!(
        fs::metadata(&table).unwrap().len(),
        before,
        "the prior good package was destroyed by a failing run"
    );
}
