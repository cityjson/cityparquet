//! `--crs` lets an operator supply a CRS a source does not declare.
//!
//! Without it, a CRS-less source is a hard conversion error (spec
//! "CRS rules"). The override is not a guess and not an absent CRS: it makes
//! the CRS resolvable before the writer runs, and is stamped as
//! operator-supplied in `city.other` so the output never implies the SOURCE
//! declared it.

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert_source};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::Source;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// A real CityJSON fixture with its `referenceSystem` removed — the shape of
/// the four catalog collections whose CityJSON carries no CRS at all.
fn crs_less_fixture(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/delft.city.jsonl");
    let text = fs::read_to_string(src).expect("fixture must exist; run `just fixtures`");
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    header["metadata"]
        .as_object_mut()
        .expect("delft header has metadata")
        .remove("referenceSystem");
    let dest = dir.join("no_crs.city.jsonl");
    let mut out = serde_json::to_string(&header).unwrap();
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    fs::write(&dest, out).unwrap();
    dest
}

#[test]
fn a_crs_less_source_still_fails_without_the_override() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let source = Source::open(&input).unwrap();
    let opts = ConvertOptions::new(input.clone(), tmp.path().join("out"));
    let err = convert_source(&source, &opts).expect_err("no CRS and no override must fail");
    assert!(err.to_string().contains("declares no CRS"), "got: {err}");
}

#[test]
fn an_override_never_relabels_a_source_that_declares_its_own_crs() {
    // The real fixture declares EPSG:7415 itself. An override must leave that
    // alone AND report that it did nothing, so the caller can keep
    // `crs_override` — and therefore the `crs_source` stamp — off a conversion
    // whose CRS in fact came from the source.
    let delft = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/delft.city.jsonl");
    let mut source = Source::open(&delft).expect("fixture must exist; run `just fixtures`");
    let applied = source.set_reference_system("EPSG:28992");
    assert!(
        !applied,
        "a source that declares its own CRS must not be relabelled"
    );
    let rs = source
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.clone())
        .expect("delft declares a CRS");
    assert_eq!(rs.code, "7415", "the source's own CRS must survive");
}

#[test]
fn a_geographic_or_unparseable_override_is_refused() {
    // The pipeline never reprojects, and the CityGML reader quantises at 1 mm:
    // an operator-supplied degree-valued CRS would silently destroy the
    // coordinates. Refuse it — and anything that is not an EPSG code at all —
    // rather than write a wrong package.
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    for (spec, needle) in [("EPSG:4326", "geographic"), ("banana", "EPSG")] {
        let mut source = Source::open(&input).unwrap();
        let mut opts = ConvertOptions::new(input.clone(), tmp.path().join("out"));
        opts.crs_override = Some(spec.to_string());
        source.set_reference_system(spec);
        let err = convert_source(&source, &opts)
            .expect_err("an unusable operator-supplied CRS must fail the conversion");
        assert!(err.to_string().contains(needle), "{spec}: got: {err}");
    }
}

#[test]
fn the_override_supplies_the_crs_and_records_its_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let mut source = Source::open(&input).unwrap();
    let mut opts = ConvertOptions::new(input.clone(), out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    if let Some(code) = &opts.crs_override {
        source.set_reference_system(code);
    }
    convert_source(&source, &opts).expect("the override must make conversion succeed");

    let table = out.join("building.parquet");
    // The footer accessor idiom used throughout this crate's tests (see
    // `crates/cityparquet/tests/footer_encoding_dispatch.rs`).
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&table).unwrap()).unwrap();
    let meta = builder.cityparquet_metadata().expect("footer must parse");
    assert!(
        meta.crs.is_some(),
        "city.crs must be populated from the override"
    );
    let other = meta.other.expect("city.other must exist");
    assert_eq!(
        other.get("crs_source").and_then(|v| v.as_str()),
        Some("operator-supplied"),
        "provenance must record that an operator supplied the CRS: {other}"
    );
}
