//! `--crs` lets an operator supply a CRS a source does not declare.
//!
//! Without it, a CRS-less source is a hard conversion error (spec
//! "CRS rules"). The override is not a guess and not an absent CRS: it makes
//! the CRS resolvable before the writer runs, and is stamped as
//! operator-supplied in `city.other` so the output never implies the SOURCE
//! declared it.

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert, convert_source};
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::Source;
use cityparquet_schema::CityMetadata;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn delft() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/delft.city.jsonl")
}

/// A real CityJSON fixture with its `referenceSystem` removed — the shape of
/// the four catalogue collections whose CityJSON carries no CRS at all.
fn crs_less_fixture(dir: &Path) -> PathBuf {
    let src = delft();
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

    let meta = footer(&out.join("building.parquet"));
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

/// The footer accessor idiom used throughout this crate's tests (see
/// `crates/cityparquet/tests/footer_encoding_dispatch.rs`).
fn footer(table: &Path) -> CityMetadata {
    let builder = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(table).unwrap()).unwrap();
    builder.cityparquet_metadata().expect("footer must parse")
}

/// `city.other.source_metadata` is documented as the source header `metadata`
/// **verbatim**. An operator-supplied CRS is injected into the in-memory
/// header so the scan can resolve it, but it must NOT reappear there: that
/// would have the output assert the SOURCE declared a CRS it never carried —
/// exactly the untruth the provenance stamp exists to prevent.
#[test]
fn the_injected_crs_never_leaks_into_the_verbatim_source_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let mut source = Source::open(&input).unwrap();
    let mut opts = ConvertOptions::new(input.clone(), out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    source.set_reference_system("EPSG:7415");
    convert_source(&source, &opts).expect("the override must make conversion succeed");

    let other = footer(&out.join("building.parquet"))
        .other
        .expect("city.other must exist");
    let source_metadata = other
        .get("source_metadata")
        .expect("delft carries header metadata, so source_metadata must be present");
    assert!(
        source_metadata.get("referenceSystem").is_none(),
        "the operator's CRS must not appear in the verbatim source metadata: {source_metadata}"
    );
    // The rest of the source's own metadata must survive untouched.
    assert!(
        source_metadata.get("title").is_some(),
        "source_metadata must still be the source's own header metadata: {source_metadata}"
    );
}

/// The provenance stamp must follow the SOURCE, never `opts.crs_override`
/// alone: an override is a no-op on a source that declares its own CRS, so a
/// caller that sets the option anyway must not get a footer claiming an
/// operator supplied a CRS the source carried itself.
#[test]
fn an_override_a_source_ignored_is_never_stamped_as_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let mut source = Source::open(&delft()).unwrap();
    let mut opts = ConvertOptions::new(delft(), out.clone());
    // A careless caller: the option is set even though applying it did nothing.
    opts.crs_override = Some("EPSG:28992".to_string());
    assert!(!source.set_reference_system("EPSG:28992"));
    convert_source(&source, &opts).expect("conversion must still succeed");

    let meta = footer(&out.join("building.parquet"));
    assert_eq!(
        meta.crs.as_ref().and_then(|c| c.pointer("/id/code")),
        Some(&serde_json::json!(7415)),
        "the source's own CRS must be the one written"
    );
    assert!(
        meta.other
            .as_ref()
            .and_then(|o| o.get("crs_source"))
            .is_none(),
        "a source-declared CRS must never be stamped operator-supplied: {:?}",
        meta.other
    );
}

/// The public one-call entry point re-opens the source from disk, so it must
/// apply the override itself — otherwise it either fails on a CRS-less source
/// it was explicitly given a CRS for, or (worse) stamps provenance onto a CRS
/// it never applied.
#[test]
fn the_library_convert_entry_point_applies_the_override_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let mut opts = ConvertOptions::new(input, out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    convert(&opts).expect("convert() must apply the override it was given");

    let meta = footer(&out.join("building.parquet"));
    assert!(meta.crs.is_some(), "city.crs must be populated");
    assert_eq!(
        meta.other
            .as_ref()
            .and_then(|o| o.get("crs_source"))
            .and_then(|v| v.as_str()),
        Some("operator-supplied"),
    );

    // ... and on a source that declares its own CRS it must stamp nothing.
    let out2 = tmp.path().join("out2");
    let mut opts = ConvertOptions::new(delft(), out2.clone());
    opts.crs_override = Some("EPSG:28992".to_string());
    convert(&opts).expect("conversion must still succeed");
    let meta = footer(&out2.join("building.parquet"));
    assert!(
        meta.other
            .as_ref()
            .and_then(|o| o.get("crs_source"))
            .is_none(),
        "convert() must not stamp an override it did not apply: {:?}",
        meta.other
    );
}

/// Merge and partition rebuild a `Source` from parts, so the provenance must
/// travel with the header it describes — otherwise a partitioned run of a
/// CRS-less dataset writes the operator's CRS with no record of where it came
/// from, which is the dishonesty this stamp exists to prevent.
#[test]
fn the_provenance_survives_partitioning() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let mut source = Source::open(&input).unwrap();
    assert!(source.set_reference_system("EPSG:7415"));
    let mut opts = ConvertOptions::new(input, out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    let report = convert_partitioned(
        std::slice::from_ref(&source),
        &PartitionSpec::Count(2),
        &opts,
    )
    .expect("partitioned conversion must succeed");

    for (label, _) in &report.partitions {
        let other = footer(&out.join(label).join("building.parquet"))
            .other
            .expect("city.other must exist");
        assert_eq!(
            other.get("crs_source").and_then(|v| v.as_str()),
            Some("operator-supplied"),
            "partition {label} lost the CRS provenance"
        );
        assert!(
            other
                .get("source_metadata")
                .and_then(|m| m.get("referenceSystem"))
                .is_none(),
            "partition {label} leaked the operator CRS into source_metadata"
        );
    }
}

/// The partitioned path asks the same question of the same inputs, so it must
/// reach the same answer: a batch in which ANY input declared its own CRS has
/// a source-declared merged CRS, because `merge_sources` enforces one shared
/// CRS across them all. Under `.any()` every partition of a mixed batch was
/// stamped operator-supplied and lost the source's `referenceSystem` from its
/// verbatim passthrough.
#[test]
fn a_mixed_batch_of_partitions_is_not_stamped_operator_supplied() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    // The real fixture declares EPSG:7415; its CRS-less twin is given the same
    // code by an operator, which is what lets the two merge at all.
    let declared = Source::open(&delft()).expect("fixture must exist; run `just fixtures`");
    let input = crs_less_fixture(tmp.path());
    let mut supplied = Source::open(&input).unwrap();
    assert!(supplied.set_reference_system("EPSG:7415"));

    let mut opts = ConvertOptions::new(input, out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    let report = convert_partitioned(&[declared, supplied], &PartitionSpec::Count(2), &opts)
        .expect("partitioned conversion must succeed");

    for (label, _) in &report.partitions {
        let other = footer(&out.join(label).join("building.parquet"))
            .other
            .expect("city.other must exist");
        assert!(
            other.get("crs_source").is_none(),
            "partition {label} claims an operator supplied a CRS an input declared: {other}"
        );
        assert!(
            other
                .get("source_metadata")
                .and_then(|m| m.get("referenceSystem"))
                .is_some(),
            "partition {label} dropped the source's own referenceSystem: {other}"
        );
    }
}

/// A bad `--crs` must be caught BEFORE `convert_partitioned` purges the
/// previous run's partitions — the invariant `ensure_parent_ready` documents
/// (a bad-input failure never destroys a prior complete output).
#[test]
fn a_bad_override_fails_before_the_stale_partitions_are_purged() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let mut source = Source::open(&input).unwrap();
    assert!(source.set_reference_system("EPSG:7415"));
    let mut opts = ConvertOptions::new(input.clone(), out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    let good = convert_partitioned(
        std::slice::from_ref(&source),
        &PartitionSpec::Count(2),
        &opts,
    )
    .expect("the first run must succeed");
    let labels: Vec<String> = good.partitions.iter().map(|(l, _)| l.clone()).collect();

    // Rerun with an unusable override over the good output.
    let mut source = Source::open(&input).unwrap();
    assert!(source.set_reference_system("EPSG:4326"));
    let mut opts = ConvertOptions::new(input, out.clone());
    opts.crs_override = Some("EPSG:4326".to_string());
    opts.overwrite = true;
    convert_partitioned(
        std::slice::from_ref(&source),
        &PartitionSpec::Count(2),
        &opts,
    )
    .map(|_| ())
    .expect_err("a geographic override must fail the partitioned run");

    for label in &labels {
        assert!(
            out.join(label).join("building.parquet").exists(),
            "the prior good partition {label} was destroyed by a failing run"
        );
    }
}
