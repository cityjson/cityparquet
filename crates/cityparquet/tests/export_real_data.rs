//! RED (M3 task 6): export — package -> CityJSON/CityJSONSeq, exercised
//! against real converted delft/railway packages.

use std::path::PathBuf;

use cityparquet::CityParquetError;
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::Profile;
use cityparquet::sidecar::{read_templates, write_templates};
use cityparquet::source::{Source, SourceFormat};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `input` into a fresh tempdir package, then exports it back to
/// `.city.jsonl` in a second tempdir. Returns the export report plus the
/// re-opened export `Source` and the original `Source` (both kept alive so
/// callers can compare headers/features), alongside the tempdirs backing
/// them — the export `Source` re-opens its file on every `features()` call,
/// so the tempdir must outlive the whole test, not just this function.
fn convert_and_export(
    input: &str,
) -> (
    cityparquet::export::ExportReport,
    Source,
    Source,
    PathBuf,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture(input),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let exported = Source::open(&output).unwrap();
    let original = Source::open(&fixture(input)).unwrap();
    (report, exported, original, output, package_dir, export_dir)
}

/// Counts `"material"`/`"texture"` keys across every geometry of every
/// feature line of an exported Seq file, walked as raw JSON (not via
/// cjseq's typed `Geometry`, so nothing a lenient deserialiser might drop
/// can mask a present key).
fn count_geometry_appearance_keys(output: &std::path::Path) -> (usize, usize) {
    let text = std::fs::read_to_string(output).unwrap();
    let mut mat = 0usize;
    let mut tex = 0usize;
    for line in text.lines().skip(1) {
        let feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for co in feature["CityObjects"].as_object().unwrap().values() {
            let Some(geoms) = co.get("geometry").and_then(|g| g.as_array()) else {
                continue;
            };
            for geom in geoms {
                let geom = geom.as_object().unwrap();
                mat += usize::from(geom.contains_key("material"));
                tex += usize::from(geom.contains_key("texture"));
            }
        }
    }
    (mat, tex)
}

#[test]
fn delft_exports_back_to_a_seq_matching_the_source_header_and_counts() {
    let (report, exported, original, _output, _package_dir, _export_dir) =
        convert_and_export("delft.city.jsonl");

    assert_eq!(exported.format(), SourceFormat::CityJsonSeq);

    // Exact JSON equality on the transform (compares scale/translate f64
    // vectors verbatim, not just a re-derived Lod string).
    let exported_transform = serde_json::to_value(&exported.header().transform).unwrap();
    let original_transform = serde_json::to_value(&original.header().transform).unwrap();
    assert_eq!(
        exported_transform, original_transform,
        "exported header transform must equal the source header transform exactly"
    );

    // referenceSystem survives as the same URL string.
    let exported_rs = exported
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
        .map(cjseq::ReferenceSystem::to_url);
    let original_rs = original
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
        .map(cjseq::ReferenceSystem::to_url);
    assert!(
        exported_rs.is_some(),
        "expected delft's source to carry a referenceSystem that survives export"
    );
    assert_eq!(exported_rs, original_rs);

    assert_eq!(report.feature_count, 1115);
    assert_eq!(report.object_count, 2231);
    assert_eq!(
        report.instance_geometries_dropped, 0,
        "delft has no GeometryInstance geometries"
    );
    assert_eq!(
        report.appearance_refs_dropped, 0,
        "recounted from the fixture: no delft geometry carries material or texture"
    );

    // Every feature line parses via cjseq (Source::features() itself uses
    // CityJSONFeature::from_str, so a clean full iteration proves this) and
    // the feature/object counts recounted independently agree with the report.
    let mut feature_count = 0usize;
    let mut object_count = 0usize;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        feature_count += 1;
        object_count += feature.city_objects.len();
    }
    assert_eq!(feature_count, 1115);
    assert_eq!(object_count, 2231);
}

#[test]
fn railway_exports_dropping_instance_geometries_but_keeping_their_objects() {
    let (report, exported, _original, output, _package_dir, _export_dir) =
        convert_and_export("lod3_railway.city.json");

    assert_eq!(exported.format(), SourceFormat::CityJsonSeq);
    assert_eq!(report.object_count, 121);
    assert_eq!(
        report.instance_geometries_dropped, 15,
        "the recount in decode_real_data.rs: exactly 15 objects carry a template"
    );

    // Recounted with python3 over the fixture, replaying the writer's
    // binding rules (per-(object, LoD) first geometry kept, GeometryInstance
    // excluded — the dataset's only LoD is "3"): 105 stored geometries, of
    // which 24 carry `material`, 95 carry `texture`, and 105 carry at least
    // one of the two. Core-profile packages store the index maps but not the
    // appearance definitions (M4 sidecars), so export must DROP them all —
    // exporting a dangling index map would be invalid CityJSON.
    assert_eq!(
        report.appearance_refs_dropped, 105,
        "every stored railway geometry carries material or texture (the recount above)"
    );
    let (mat_keys, tex_keys) = count_geometry_appearance_keys(&output);
    assert_eq!(
        (mat_keys, tex_keys),
        (0, 0),
        "exported geometries must not carry dangling material/texture index maps"
    );

    let mut object_count = 0usize;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        object_count += feature.city_objects.len();
    }
    assert_eq!(object_count, 121);
}

/// M4 task 5: the source header's `metadata` object (title,
/// geographicalExtent, etc.) is captured verbatim into the package's KV
/// metadata (`source_metadata`) and restored into the exported header.
/// `fullMetadataUrl` is a documented exception — cjseq's `Metadata` struct
/// has no passthrough for unknown members, so it never survives even the
/// initial parse of the source header, let alone the round trip.
#[test]
fn delft_source_metadata_reaches_kv_metadata_and_the_exported_header() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let file = std::fs::File::open(package_dir.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let source_metadata = meta
        .source_metadata
        .as_ref()
        .expect("delft's header sets metadata; source_metadata must be populated");
    assert_eq!(source_metadata["title"], serde_json::json!("3DBAG"));
    assert!(
        source_metadata.get("geographicalExtent").is_some(),
        "expected geographicalExtent in {source_metadata}"
    );
    assert!(
        source_metadata.get("fullMetadataUrl").is_none(),
        "fullMetadataUrl is not part of cjseq::Metadata and cannot survive"
    );

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let exported_header_line = std::fs::read_to_string(&output)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let exported_header: serde_json::Value = serde_json::from_str(&exported_header_line).unwrap();
    let source_header_line = std::fs::read_to_string(fixture("delft.city.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let source_header: serde_json::Value = serde_json::from_str(&source_header_line).unwrap();

    assert_eq!(
        exported_header["metadata"]["title"], source_header["metadata"]["title"],
        "exported header title must match the source"
    );
    assert_eq!(
        exported_header["metadata"]["geographicalExtent"],
        source_header["metadata"]["geographicalExtent"],
        "exported header geographicalExtent must match the source"
    );
}

/// Recursively asserts every integer in a *localised* material tree (the
/// `values`/`value` payload attached to a restored `cjseq::Material`) is a
/// feature-local index `< limit`; `null` (no material) is skipped.
fn assert_material_indices_below(v: &Value, limit: usize) {
    match v {
        Value::Null => {}
        Value::Number(n) => {
            let idx = n
                .as_u64()
                .unwrap_or_else(|| panic!("expected a non-negative material index, got {n}"))
                as usize;
            assert!(
                idx < limit,
                "material index {idx} must be < feature-local materials len {limit}"
            );
        }
        Value::Array(items) => {
            for x in items {
                assert_material_indices_below(x, limit);
            }
        }
        other => panic!("unexpected node in a localised material tree: {other}"),
    }
}

/// Recursively asserts every innermost ring in a *localised* texture tree is
/// back to plain INDEX form `[t, uv_idx0, uv_idx1, ...]` (or `[null]`): `t` (a
/// feature-local texture index) `< tex_limit`, and every following element is
/// a bare integer `< uv_limit` — NOT an inlined `[u, v]` pair, which would
/// mean the encoder's UV-inlining rewrite was never undone.
fn assert_texture_rings_are_index_form(v: &Value, tex_limit: usize, uv_limit: usize) {
    match v {
        Value::Array(items) => {
            let is_ring = !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null);
            if is_ring {
                if let Value::Number(n) = &items[0] {
                    let t = n
                        .as_u64()
                        .unwrap_or_else(|| panic!("expected a non-negative texture index, got {n}"))
                        as usize;
                    assert!(t < tex_limit, "texture id {t} must be < {tex_limit}");
                }
                for uv in &items[1..] {
                    assert!(
                        !uv.is_array(),
                        "texture ring must be back to index form, found an inlined [u, v] pair: {uv}"
                    );
                    let idx = uv
                        .as_u64()
                        .unwrap_or_else(|| panic!("expected a UV index, got {uv}"))
                        as usize;
                    assert!(idx < uv_limit, "UV index {idx} must be < {uv_limit}");
                }
            } else {
                for x in items {
                    assert_texture_rings_are_index_form(x, tex_limit, uv_limit);
                }
            }
        }
        other => panic!("unexpected node in a localised texture tree: {other}"),
    }
}

/// M4 task 9: on a Compatibility-profile package (sidecars present), export
/// restores `material`/`texture` from the sidecars instead of dropping them —
/// each feature gets its own self-contained, feature-local `appearance`
/// block (the inverse of the encoder's global interning + UV inlining).
#[test]
fn railway_compatibility_export_restores_appearance_feature_local() {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    opts.profile = Profile::Compatibility;
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    // (a)
    assert_eq!(
        report.appearance_refs_dropped, 0,
        "compatibility-profile export must restore appearance, not drop it"
    );

    let exported = Source::open(&output).unwrap();
    let mut found_material = false;
    let mut found_texture = false;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        let Some(appearance) = &feature.appearance else {
            continue;
        };
        let n_materials = appearance.materials.as_ref().map_or(0, Vec::len);
        let n_textures = appearance.textures.as_ref().map_or(0, Vec::len);
        let n_uvs = appearance.vertices_texture.as_ref().map_or(0, Vec::len);

        for co in feature.city_objects.values() {
            let Some(geoms) = &co.geometry else { continue };
            for g in geoms {
                // (b)
                if let Some(material) = &g.material {
                    assert!(
                        !material.is_empty(),
                        "a geometry carrying a material member must not be empty"
                    );
                    for m in material.values() {
                        if let Some(idx) = m.value {
                            assert!(
                                idx < n_materials,
                                "material index {idx} must be < feature-local materials len {n_materials}"
                            );
                            found_material = true;
                        }
                        if let Some(values) = &m.values {
                            assert_material_indices_below(values, n_materials);
                            found_material = true;
                        }
                    }
                }
                // (c)
                if let Some(texture) = &g.texture {
                    for t in texture.values() {
                        if let Some(values) = &t.values {
                            assert_texture_rings_are_index_form(values, n_textures, n_uvs);
                            found_texture = true;
                        }
                    }
                }
            }
        }
    }
    assert!(
        found_material,
        "expected at least one restored material reference across the exported features"
    );
    assert!(
        found_texture,
        "expected at least one restored texture reference across the exported features"
    );
}

/// M4 task 10: on a Compatibility-profile package (`geometry_templates.parquet`
/// present), export rebuilds the header's `geometry-templates` and each
/// object's `GeometryInstance` geometry, instead of dropping them. Exercised
/// via the DOC (`.city.json`) output path deliberately — `cjseq_to_cj`'s
/// `add_cjfeature` merges each feature's own appearance into the document
/// appearance, and template `material`/`texture` are localised at HEADER
/// scope beforehand, so this path is the one that could show the header's
/// template indices getting disturbed by that merge (they must not: the
/// header's own materials/textures are installed before any feature is
/// merged in, so their positions never move — see `cjseq::Appearance::
/// add_material`'s dedup-by-value-equality, which only ever APPENDS new
/// entries after existing ones).
#[test]
fn railway_compatibility_export_rebuilds_geometry_templates_and_instances() {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    opts.profile = Profile::Compatibility;
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.json");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    // (a)
    assert_eq!(
        report.instance_geometries_dropped, 0,
        "templates sidecar is present: instances must be rebuilt, not dropped"
    );

    let text = std::fs::read_to_string(&output).unwrap();
    let doc = cjseq::CityJSON::from_str(&text).expect("cjseq must parse the exported .city.json");

    // Read the SOURCE's own geometry-templates straight from the fixture —
    // types/lods are asserted against this, never hardcoded.
    let source_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
    let source_doc = cjseq::CityJSON::from_str(&source_text).unwrap();
    let source_templates = source_doc
        .geometry_templates
        .as_ref()
        .expect("railway fixture has geometry-templates");
    let source_verts: Vec<Vec<f64>> =
        serde_json::from_value(source_templates.vertices_templates.clone()).unwrap();

    let rebuilt_templates = doc
        .geometry_templates
        .as_ref()
        .expect("exported doc must carry geometry-templates");

    // (b)
    assert_eq!(
        rebuilt_templates.templates.len(),
        source_templates.templates.len(),
        "rebuilt template count must match the source"
    );
    for (i, (rebuilt, source)) in rebuilt_templates
        .templates
        .iter()
        .zip(&source_templates.templates)
        .enumerate()
    {
        assert_eq!(
            rebuilt.thetype, source.thetype,
            "template {i}: type must match the source"
        );
        assert_eq!(
            rebuilt.lod, source.lod,
            "template {i}: lod must match the source"
        );
    }

    // (c)
    let mut instance_count = 0usize;
    for co in doc.city_objects.values() {
        let Some(geoms) = &co.geometry else { continue };
        for g in geoms {
            if g.thetype != cjseq::GeometryType::GeometryInstance {
                continue;
            }
            instance_count += 1;
            assert!(
                g.template.is_some(),
                "a GeometryInstance geometry must carry a template index"
            );
            let matrix = g
                .transformation_matrix
                .as_ref()
                .expect("GeometryInstance must carry a transformationMatrix")
                .as_array()
                .expect("transformationMatrix must be a JSON array");
            assert_eq!(
                matrix.len(),
                16,
                "transformationMatrix must have 16 elements"
            );
            let boundaries: Vec<usize> = serde_json::from_value(g.boundaries.clone())
                .expect("GeometryInstance boundaries must be a flat array of vertex indices");
            assert_eq!(
                boundaries.len(),
                1,
                "GeometryInstance boundaries must reference exactly one vertex"
            );
        }
    }
    assert_eq!(
        instance_count, 15,
        "the recount in decode_real_data.rs: exactly 15 objects carry a template"
    );

    // (d) walk EVERY template's first ring (all 3 of railway's templates are
    // MultiSurface-shaped, per the fixture) through both the rebuilt and
    // source boundary trees and compare coordinates bitwise; index identity
    // is NOT required (the rebuilt vertices-templates pool may assign
    // different positions), only coordinate identity. Widened from
    // template 0 only (the controller's M4 task 10 review) so a bug
    // isolated to template 1 or 2's own coordinate rewrite can't hide behind
    // template 0 happening to round-trip correctly.
    let rebuilt_verts: Vec<Vec<f64>> =
        serde_json::from_value(rebuilt_templates.vertices_templates.clone()).unwrap();
    for t in 0..source_templates.templates.len() {
        let rebuilt_boundaries: Vec<Vec<Vec<usize>>> =
            serde_json::from_value(rebuilt_templates.templates[t].boundaries.clone())
                .unwrap_or_else(|e| {
                    panic!("template {t} must be a MultiSurface-shaped boundary tree: {e}")
                });
        let source_boundaries: Vec<Vec<Vec<usize>>> =
            serde_json::from_value(source_templates.templates[t].boundaries.clone()).unwrap();
        let rebuilt_ring0 = &rebuilt_boundaries[0][0];
        let source_ring0 = &source_boundaries[0][0];
        assert_eq!(
            rebuilt_ring0.len(),
            source_ring0.len(),
            "template {t}'s first ring must keep its source vertex count"
        );
        for (i, &rebuilt_idx) in rebuilt_ring0.iter().enumerate() {
            let source_idx = source_ring0[i];
            assert_eq!(
                rebuilt_verts[rebuilt_idx], source_verts[source_idx],
                "template {t} ring vertex {i}: rebuilt coordinate must equal the source's bitwise"
            );
        }
    }
}

/// Controller addition A (M4 task 10 review): a `template` reference that
/// names a row `geometry_templates.parquet` no longer carries must be a
/// `Schema` error naming both the dangling object and the missing template
/// id — never a panic or a silently-dropped/fabricated geometry. Derived
/// from a real converted railway package (sanctioned): the fixture's own
/// python recount (`{0: 10, 1: 4, 2: 1}` GeometryInstance-per-template
/// counts) confirms template id `"0"` is referenced by 10 real objects, so
/// removing its sidecar row is guaranteed to hit the dangling-reference path
/// on export, not silently succeed because nothing happened to reference it.
#[test]
fn export_errors_on_a_dangling_template_id_reference() {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    opts.profile = Profile::Compatibility;
    convert(&opts).unwrap();

    let templates_path = package_dir.path().join("geometry_templates.parquet");
    let rows = read_templates(&templates_path).unwrap();
    assert_eq!(
        rows.len(),
        3,
        "railway must carry exactly 3 geometry templates (pinned elsewhere)"
    );
    let corrupted: Vec<_> = rows.into_iter().filter(|r| r.id != "0").collect();
    assert_eq!(
        corrupted.len(),
        2,
        "removing template id \"0\" must leave exactly the other 2 rows"
    );
    write_templates(&templates_path, &corrupted).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let err = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output,
    })
    .unwrap_err();

    assert!(
        matches!(err, CityParquetError::Schema(_)),
        "expected a Schema error, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("object "),
        "the error must name the dangling object, got: {msg}"
    );
    assert!(
        msg.contains("\"0\""),
        "the error must name the missing template id \"0\", got: {msg}"
    );
}

#[test]
fn delft_also_exports_as_a_single_whole_city_json_document() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.json");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();
    assert_eq!(report.feature_count, 1115);

    let text = std::fs::read_to_string(&output).unwrap();
    let doc = cjseq::CityJSON::from_str(&text).expect("cjseq must parse the exported .city.json");
    assert_eq!(doc.thetype, "CityJSON");
    assert_eq!(doc.version, "2.0");
    assert_eq!(doc.number_of_city_objects(), 1115, "top-level objects only");
}
