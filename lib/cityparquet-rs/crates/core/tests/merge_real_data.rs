//! Merge tests over the real `delft.city.jsonl` fixture (and derived copies of
//! it — never hand-authored synthetic CityJSON).

use std::fs;
use std::path::{Path, PathBuf};

use arrow_array::{RecordBatch, RecordBatchReader};
use cityparquet::merge::merge_sources;
use cityparquet::package::{ConvertOptions, convert_source};
use cityparquet::source::{Source, SourceFormat};
use cjseq::{CityJSON, CityJSONFeature, Transform};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// A buffered source built from delft's header with a mutated transform.
fn delft_source_with_transform(scale: [f64; 3], translate: [f64; 3]) -> Source {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let mut header = src.header().clone();
    header.transform = Transform { scale, translate };
    let feats: Vec<CityJSONFeature> = src
        .features()
        .unwrap()
        .map(|f| f.unwrap())
        .take(2)
        .collect();
    Source::from_parts(header, feats, None, SourceFormat::CityJsonSeq)
}

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Real-world bbox of `features` dequantised through `t` (x,y,z min/max).
fn real_bbox(features: &[CityJSONFeature], t: &Transform) -> [f64; 6] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for f in features {
        for v in &f.vertices {
            for i in 0..3 {
                let r = v[i] as f64 * t.scale[i] + t.translate[i];
                b[i] = b[i].min(r);
                b[i + 3] = b[i + 3].max(r);
            }
        }
    }
    b
}

/// Write a derived CityJSONSeq copy of `orig`'s first `take` features: header
/// `transform.scale` divided by `scale_div`, every vertex multiplied by
/// `vert_mul`. With `(scale_div, vert_mul) = (10.0, 10)` this is an EXACT
/// re-quantisation (same real coordinates, finer grid).
fn write_derived_copy(orig: &Path, dst: &Path, scale_div: f64, vert_mul: i64, take: usize) {
    let src = Source::open(orig).unwrap();
    let mut header: CityJSON = src.header().clone();
    for s in header.transform.scale.iter_mut() {
        *s /= scale_div;
    }
    let feats: Vec<CityJSONFeature> = src
        .features()
        .unwrap()
        .map(|f| f.unwrap())
        .take(take)
        .collect();
    let mut out = String::new();
    out.push_str(&serde_json::to_string(&header).unwrap());
    out.push('\n');
    for mut f in feats {
        for v in f.vertices.iter_mut() {
            for c in v.iter_mut() {
                *c *= vert_mul;
            }
        }
        out.push_str(&serde_json::to_string(&f).unwrap());
        out.push('\n');
    }
    fs::write(dst, out).unwrap();
}

#[test]
fn single_source_merge_preserves_transform_and_count() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let n = src.features().unwrap().count();
    let scale = src.header().transform.scale;
    let translate = src.header().transform.translate;
    let merged = merge_sources(vec![src]).unwrap();
    assert_eq!(merged.header().transform.scale, scale);
    assert_eq!(merged.header().transform.translate, translate);
    assert_eq!(merged.features().unwrap().count(), n);
}

#[test]
fn heterogeneous_transforms_requantise_to_same_real_coords() {
    let d = tempfile::tempdir().unwrap();
    // A: first 5 features at the original scale. B: same 5, scale/10 + verts*10
    // (exact re-quantisation) so the two inputs describe identical real coords
    // on different grids -> the requantise path (not the all-equal fast path).
    let a = d.path().join("a.city.jsonl");
    let b = d.path().join("b.city.jsonl");
    write_derived_copy(&fixture("delft.city.jsonl"), &a, 1.0, 1, 5);
    write_derived_copy(&fixture("delft.city.jsonl"), &b, 10.0, 10, 5);

    let sa = Source::open(&a).unwrap();
    let sb = Source::open(&b).unwrap();
    let expect = real_bbox(
        &sa.features()
            .unwrap()
            .map(|f| f.unwrap())
            .collect::<Vec<_>>(),
        &sa.header().transform,
    );

    let merged = merge_sources(vec![sa, sb]).unwrap();
    // Merged transform must be the finer (min) scale, i.e. B's.
    let transform = &merged.header().transform;
    assert!(transform.scale[0] < 0.001 + 1e-12);
    let features: Vec<CityJSONFeature> = merged.features().unwrap().map(|f| f.unwrap()).collect();
    let got = real_bbox(&features, transform);
    let tol = transform.scale[0]; // <= mergedScale bound
    for i in 0..6 {
        assert!(
            (got[i] - expect[i]).abs() <= tol.max(1e-6),
            "axis {i}: {} vs {} (tol {tol})",
            got[i],
            expect[i]
        );
    }
}

#[test]
fn crs_mismatch_is_error() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.city.jsonl");
    let b = d.path().join("b.city.jsonl");
    write_derived_copy(&fixture("delft.city.jsonl"), &a, 1.0, 1, 3);
    // B: strip referenceSystem so its CRS differs from A's.
    {
        let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
        let mut header = src.header().clone();
        if let Some(m) = header.metadata.as_mut() {
            m.reference_system = None;
        }
        let feats: Vec<CityJSONFeature> = src
            .features()
            .unwrap()
            .map(|f| f.unwrap())
            .take(3)
            .collect();
        let mut out = serde_json::to_string(&header).unwrap();
        out.push('\n');
        for f in feats {
            out.push_str(&serde_json::to_string(&f).unwrap());
            out.push('\n');
        }
        fs::write(&b, out).unwrap();
    }
    let sa = Source::open(&a).unwrap();
    let sb = Source::open(&b).unwrap();
    assert!(
        merge_sources(vec![sa, sb]).is_err(),
        "CRS mismatch must error"
    );
}

#[test]
fn zero_or_nonfinite_scale_is_rejected() {
    let zero = delft_source_with_transform([0.0, 0.001, 0.001], [0.0, 0.0, 0.0]);
    assert!(merge_sources(vec![zero]).is_err(), "zero scale must error");
    let inf = delft_source_with_transform([f64::INFINITY, 0.001, 0.001], [0.0, 0.0, 0.0]);
    assert!(
        merge_sources(vec![inf]).is_err(),
        "non-finite scale must error"
    );
}

/// Inputs repeating an object id cannot be one package (object ids are
/// unique across a package), so converting them is refused.
#[test]
fn inputs_repeating_an_object_id_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.city.jsonl");
    write_derived_copy(&fixture("delft.city.jsonl"), &a, 1.0, 1, 5);
    let merged = merge_sources(vec![Source::open(&a).unwrap(), Source::open(&a).unwrap()]).unwrap();
    assert_eq!(merged.features().unwrap().count(), 10);
    let err = convert_source(
        &merged,
        &ConvertOptions::new(PathBuf::new(), d.path().join("pkg")),
    )
    .expect_err("a repeated object id must be refused");
    assert!(format!("{err}").contains("more than once"), "{err}");
}

/// The rows of every object table of `pkg`, by file name, all in one batch,
/// and the table's fields.
fn tables(pkg: &Path) -> Vec<(String, Vec<arrow_schema::FieldRef>, RecordBatch)> {
    let mut out = Vec::new();
    let mut paths: Vec<PathBuf> = fs::read_dir(pkg)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .collect();
    paths.sort();
    for path in paths {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        let schema = reader.schema();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        let all = arrow_select::concat::concat_batches(&schema, &batches).unwrap();
        out.push((
            path.file_name().unwrap().to_string_lossy().to_string(),
            schema.fields().iter().cloned().collect(),
            all,
        ));
    }
    out
}

/// Delft cut into three consecutive CityJSONSeq files converts, as one
/// merged input, to the same rows and columns as delft itself. (The footers
/// differ: a merged header carries no `geographicalExtent`.)
#[test]
fn delft_split_into_three_files_converts_to_the_same_rows_as_delft() {
    let d = tempfile::tempdir().unwrap();
    let text = fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap();
    let features: Vec<&str> = lines.filter(|l| !l.trim().is_empty()).collect();
    let parts: Vec<Source> = features
        .chunks(features.len().div_ceil(3))
        .enumerate()
        .map(|(i, chunk)| {
            let path = d.path().join(format!("part{i}.city.jsonl"));
            fs::write(&path, format!("{header}\n{}\n", chunk.join("\n"))).unwrap();
            Source::open(&path).unwrap()
        })
        .collect();
    assert_eq!(parts.len(), 3);

    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), d.path().join("whole"));
    opts.datetime = Some("2026-10-13T00:00:00Z".to_string());
    convert_source(&Source::open(&fixture("delft.city.jsonl")).unwrap(), &opts).unwrap();
    opts.output_dir = d.path().join("merged");
    convert_source(&merge_sources(parts).unwrap(), &opts).unwrap();

    let whole = tables(&d.path().join("whole"));
    let merged = tables(&d.path().join("merged"));
    assert_eq!(whole.len(), merged.len());
    for ((name, fields, rows), (merged_name, merged_fields, merged_rows)) in
        whole.iter().zip(&merged)
    {
        assert_eq!(name, merged_name);
        assert_eq!(fields, merged_fields, "{name}: columns");
        assert!(rows == merged_rows, "{name}: rows differ");
    }
}
