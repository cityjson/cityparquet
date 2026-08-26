//! `--crs` lets an operator supply a CRS a source does not declare.
//!
//! Without it, a CRS-less source still converts — with an explicit
//! `city.crs: null` and no georeference (spec "CRS rules": "an unresolvable
//! CRS is declared, not fatal"). The override is what turns that unknown into
//! a real CRS. It is not a guess and not an absent CRS: it makes the CRS
//! resolvable before the writer runs, and is stamped as operator-supplied in
//! `city.other` so the output never implies the SOURCE declared it.

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert, convert_source};
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::Source;
use cityparquet_schema::{CityMetadata, CrsState};
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

/// RED (spec §metadata "CRS rules", as amended): a CRS-less source converts
/// WITHOUT the override, writing an explicit `city.crs: null` plus a report
/// diagnostic — it is no longer a hard conversion error.
///
/// This replaces `a_crs_less_source_still_fails_without_the_override`. The
/// state that matters is `null`, not absence: per GeoParquet an absent `crs`
/// asserts OGC:CRS84, which over Delft's RD New coordinates would place the
/// city off the coast of Africa.
#[test]
fn a_crs_less_source_converts_to_an_explicit_null_crs_and_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let source = Source::open(&input).unwrap();
    let opts = ConvertOptions::new(input.clone(), out.clone());
    let report = convert_source(&source, &opts)
        .expect("a CRS-less source is declared, not fatal (spec CRS rules)");

    let diagnostic = report
        .crs_diagnostic
        .as_deref()
        .expect("the writer SHOULD surface a conversion diagnostic");
    assert!(
        diagnostic.contains("declares no CRS") && diagnostic.contains("null"),
        "the diagnostic must explain the explicit null, got: {diagnostic}"
    );

    let table = out.join("building.parquet");
    let meta = footer(&table);
    assert_eq!(
        meta.crs,
        CrsState::Unknown,
        "city.crs must be an explicit null, never absent"
    );

    // The footer BYTES, not just the parsed shape: absence and null are the
    // two states `Option<Value>` used to conflate, and only the raw JSON can
    // tell them apart.
    let city: serde_json::Value = serde_json::from_str(&raw_footer_key(&table, "city")).unwrap();
    let members = city.as_object().unwrap();
    assert!(
        members.contains_key("crs"),
        "the `crs` key MUST be written: {members:?}"
    );
    assert!(members["crs"].is_null(), "{members:?}");

    // And the GeoParquet mirror says the same — a GeoParquet-only consumer
    // cannot read the foreign `city` key, so an absent `geo.columns[].crs`
    // would silently assert CRS84 to it.
    let geo: serde_json::Value = serde_json::from_str(&raw_footer_key(&table, "geo")).unwrap();
    let column = geo["columns"]["geometry_lod0_0"].as_object().unwrap();
    assert!(column.contains_key("crs"), "{column:?}");
    assert!(
        column["crs"].is_null(),
        "geo mirrors the null (GeoParquet-legal): {column:?}"
    );
}

/// A package written with an unknown CRS carries no `proj:*` STAC fields, and
/// no spatial extent either — the Item can only state a CRS, a `bbox` and a
/// `geometry` the package actually has.
///
/// The extent half is the one that needs saying twice (see
/// [`an_unknown_crs_in_small_local_coordinates_claims_no_wgs84_extent`]): here
/// the source is Delft in RD New metres, whose ~85 000 easting is outside
/// WGS84 range, so the extent would be dropped by the reprojection failing
/// even without the fix. That is exactly why this test alone is not a
/// sufficient pin.
#[test]
fn an_unknown_crs_writes_no_projection_extension_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let source = Source::open(&input).unwrap();
    convert_source(&source, &ConvertOptions::new(input, out.clone()))
        .expect("a CRS-less source converts");

    let item: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("metadata.json")).unwrap()).unwrap();
    let props = item["properties"].as_object().unwrap();
    let proj: Vec<&String> = props.keys().filter(|k| k.starts_with("proj:")).collect();
    assert!(
        proj.is_empty(),
        "an unknown CRS must claim no projection fields: {proj:?}"
    );
    assert_unlocated(&item);
}

/// RED (review Important #2): the trap the Delft test above cannot spring.
///
/// `BBox3D::to_wgs84` treats a **no-CRS** source as "maybe it is already
/// WGS84": if the extent happens to fall inside ±180/±90 it returns the
/// coordinates **unchanged** rather than erroring. Before this branch, a
/// coordinate-bearing CRS-less package was a hard conversion error, so that
/// heuristic was unreachable; with `city.crs: null` now the standard outcome
/// it became the normal path — and a model in small **local** coordinates
/// (a single building modelled about the origin, the commonest no-CRS shape
/// there is) gets a *fabricated* WGS84 bbox and footprint on its STAC Item,
/// flatly contradicting the footer's own `null` and the no-guessing rule.
///
/// Derived from the real Delft fixture: `referenceSystem` removed and the
/// header `transform` re-quantised about the origin, which pulls the same real
/// vertices into the ±180/±90 window. `transform` is an implementation-chosen
/// encoding parameter, never semantic content (see `crate::compare`), so this
/// stays a real model — no hand-written CityJSON. Delft is also the fixture
/// that makes the trap *reachable* at all: it is single-module and every row
/// carries geometry, so `package_bbox` yields an extent rather than the `None`
/// a multi-table package with a geometry-less table gives.
#[test]
fn an_unknown_crs_in_small_local_coordinates_claims_no_wgs84_extent() {
    let tmp = tempfile::tempdir().unwrap();
    let text = fs::read_to_string(delft()).expect("fixture must exist; run `just fixtures`");
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let metadata = header["metadata"].as_object_mut().unwrap();
    metadata.remove("referenceSystem");
    // The geographicalExtent is stated in the ORIGINAL coordinates and would
    // otherwise contradict the re-quantised vertices below.
    metadata.remove("geographicalExtent");
    // 0.1 mm quantisation about the origin: Delft's ~1.2 km extent lands in
    // roughly ±59, inside WGS84's ±180/±90 — which is what arms the trap.
    header["transform"] = serde_json::json!({
        "scale": [0.0001, 0.0001, 0.0001],
        "translate": [0.0, 0.0, 0.0],
    });
    let mut out_text = serde_json::to_string(&header).unwrap();
    for line in lines {
        out_text.push('\n');
        // Remove per-object geographicalExtent declarations: they are stated in
        // the ORIGINAL coordinates and would contradict the re-quantised vertices.
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        if let Some(objs) = feature["CityObjects"].as_object_mut() {
            for obj in objs.values_mut() {
                obj.as_object_mut().unwrap().remove("geographicalExtent");
            }
        }
        out_text.push_str(&serde_json::to_string(&feature).unwrap());
    }
    let input = tmp.path().join("local_coords_no_crs.city.jsonl");
    fs::write(&input, out_text).unwrap();

    let out = tmp.path().join("out");
    let source = Source::open(&input).unwrap();
    let report = convert_source(&source, &ConvertOptions::new(input, out.clone()))
        .expect("a CRS-less source converts");
    assert!(report.crs_diagnostic.is_some());
    assert_eq!(footer(&out.join("building.parquet")).crs, CrsState::Unknown);

    // The premise: this package's extent really does fall inside ±180/±90, so
    // `to_wgs84` would happily hand it back unchanged. If a future change to
    // the fixture or the quantiser moved it out of range, this test would go
    // green for the wrong reason.
    let tables = cityparquet::stac::properties::PackageTables::open(&out).unwrap();
    let extent = cityparquet::stac::package_bbox(&tables)
        .unwrap()
        .expect("delft is single-module with geometry on every row, so it HAS an extent");
    assert!(
        extent.xmin >= -180.0 && extent.xmax <= 180.0,
        "premise: x must be inside WGS84 range, got {}..{}",
        extent.xmin,
        extent.xmax
    );
    assert!(
        extent.ymin >= -90.0 && extent.ymax <= 90.0,
        "premise: y must be inside WGS84 range, got {}..{}",
        extent.ymin,
        extent.ymax
    );

    let item: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("metadata.json")).unwrap()).unwrap();
    assert_unlocated(&item);
}

/// An "unlocated" STAC Item: `geometry` present and **null**, no `bbox` key at
/// all. That is the valid STAC shape for an Item with no known extent (and the
/// same shape
/// `stac_derive_real_data::a_package_with_no_crs_converts_to_an_unlocated_but_schema_valid_item`
/// already validates against the city3d JSON schema) — GeoJSON allows a null
/// geometry, and STAC requires `bbox` only *when* `geometry` is non-null.
fn assert_unlocated(item: &serde_json::Value) {
    assert!(
        item.get("geometry").is_some_and(serde_json::Value::is_null),
        "an unknown CRS must yield a null geometry, never a fabricated \
         footprint: {}",
        item.get("geometry")
            .map(ToString::to_string)
            .unwrap_or_else(|| "<<absent>>".to_string())
    );
    assert!(
        item.get("bbox").is_none(),
        "an unknown CRS must yield NO bbox — the coordinates have no \
         georeference, so any WGS84 extent would be invented: {:?}",
        item.get("bbox")
    );
}

/// One raw Parquet footer key-value entry, as the writer wrote it. The typed
/// `footer` accessor below cannot answer "absent or null?" — that distinction
/// only exists in the bytes.
fn raw_footer_key(table: &Path, key: &str) -> String {
    let builder = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(table).unwrap()).unwrap();
    builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .expect("footer key-value metadata")
        .iter()
        .find(|kv| kv.key == key)
        .unwrap_or_else(|| panic!("footer must carry a {key:?} key"))
        .value
        .clone()
        .unwrap_or_else(|| panic!("{key:?} must have a value"))
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
        meta.crs.is_known(),
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
/// `crates/core/tests/footer_encoding_dispatch.rs`).
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
        meta.crs.known().and_then(|c| c.pointer("/id/code")),
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
    assert!(meta.crs.is_known(), "city.crs must be populated");
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
