//! Merge several [`Source`]s into one logical CityJSON dataset for a single
//! (optionally partitioned) conversion.
//!
//! All inputs must share a CRS (else [`merge_sources`] errors). CityJSON
//! quantisation (`transform`) may differ per input — 3DBAG tiles each carry
//! their own `translate` — so when transforms differ every feature's `vertices`
//! pool is **requantised** onto one merged transform. The merged transform uses
//! the componentwise-minimum `scale` and `translate` across inputs; requantising
//! real coordinates onto that grid is exact only when the shift is integral, so
//! the bound is `≤ merged.scale/2` per axis (CityJSON quantisation is already
//! lossy at its own `scale`, and the merged scale is the finest present, so this
//! never loses more than the coarsest input already had).
//!
//! Doc-level geometry templates + their appearance array are supported from at
//! most ONE input (they carry global appearance indices that cannot be merged
//! across inputs in this milestone); more than one carrier is an error.
//! `geographicalExtent` is stripped from the merged header so a partition's
//! footer never advertises another input's extent.
//!
//! Two [`MergeOptions`] serve tiled exports, whose tiles are cut from one
//! model. `dedupe_identical` keeps one copy of a feature that several tiles
//! carry whole; `prefix_ids_by_input` makes ids that only mean something within
//! their tile unique across the package.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Appearance, CityJSON, CityJSONFeature, Transform};

use crate::source::Source;

/// How [`merge_sources`] reconciles inputs cut from one model.
#[derive(Debug, Clone, Copy, Default)]
pub struct MergeOptions {
    /// Keep one copy of a feature that another input already carries
    /// identically: the same id, and the same feature once requantised onto
    /// the merged transform — except that a texture's image may sit at a
    /// different path, provided the two image files (resolved beside their
    /// inputs) hold the same bytes. A tile exporter that repeats a straddling
    /// building ships its texture in each tile's own folder. The first copy is
    /// kept, image path and all. Same id with any other difference is kept and
    /// counted in [`MergedDataset::duplicate_ids`].
    pub dedupe_identical: bool,
    /// Rename every CityObject `<input stem>.<id>` — the feature id and every
    /// `parents`/`children` reference with it — so ids an exporter numbers per
    /// tile stay distinct once tiles are merged. Applied after
    /// `dedupe_identical`, so a kept copy takes its first input's stem. `.` is
    /// the separator because it is legal in an XML NCName (a `gml:id`), where
    /// `:` is not. Inputs sharing a stem are refused. A round trip compares
    /// equal to the source only once the prefix is stripped.
    pub prefix_ids_by_input: bool,
}

/// One merged dataset: the shared header (merged transform, first input's
/// metadata sans `geographicalExtent`, the sole template carrier's templates),
/// every input's features concatenated (requantised onto the merged transform
/// where needed), the doc-level appearance the templates resolve against, how
/// many identical copies [`MergeOptions::dedupe_identical`] dropped, and how
/// many feature ids still collide.
#[derive(Debug, Clone)]
pub struct MergedDataset {
    pub header: CityJSON,
    pub features: Vec<CityJSONFeature>,
    pub doc_appearance: Option<Appearance>,
    pub deduplicated: usize,
    pub duplicate_ids: usize,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

fn transform_eq(a: &Transform, b: &Transform) -> bool {
    a.scale == b.scale && a.translate == b.translate
}

/// CRS identity is compared by the referenceSystem's serialised form. This is
/// strict string equality — the same authority/code spelled differently (e.g.
/// `http` vs `https` OGC URLs) is treated as a mismatch. Adequate here: tiles
/// from one producer carry byte-identical CRS strings; normalise if a
/// heterogeneous-spelling corpus ever needs merging (gpt-5.6-sol review note).
fn crs_key(header: &CityJSON) -> Result<Option<serde_json::Value>> {
    match header
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
    {
        Some(rs) => Ok(Some(serde_json::to_value(rs)?)),
        None => Ok(None),
    }
}

/// Requantise `vertices` (originally quantised against `src`) onto `merged`.
/// `v' = round(v·(srcScale/mergedScale) + (srcTranslate − mergedTranslate)/mergedScale)`
/// — the two ratios are precomputed per axis so a clean integral ratio
/// (`merged` finer by an integer factor, same translate) round-trips exactly.
fn requantise_vertices(vertices: &mut [Vec<i64>], src: &Transform, merged: &Transform) {
    let mut ratio = [0f64; 3];
    let mut offset = [0f64; 3];
    for i in 0..3 {
        ratio[i] = src.scale[i] / merged.scale[i];
        offset[i] = (src.translate[i] - merged.translate[i]) / merged.scale[i];
    }
    for v in vertices.iter_mut() {
        for (i, c) in v.iter_mut().enumerate().take(3) {
            *c = (*c as f64 * ratio[i] + offset[i]).round() as i64;
        }
    }
}

/// True when this source contributes doc-level geometry templates (which carry
/// global appearance indices resolved against its `doc_appearance`).
fn is_template_carrier(source: &Source) -> bool {
    source.header().geometry_templates.is_some()
}

/// Merge `sources` (non-empty) into one [`MergedDataset`]. See the module docs
/// for the CRS / transform / template rules.
pub fn merge_sources(sources: &[Source], opts: &MergeOptions) -> Result<MergedDataset> {
    let first = sources
        .first()
        .ok_or_else(|| err("merge_sources: no sources".to_string()))?;

    // CRS: every input's referenceSystem must serialise identically.
    let crs0 = crs_key(first.header())?;
    for s in &sources[1..] {
        if crs_key(s.header())? != crs0 {
            return Err(err(
                "inputs declare different reference systems (CRS); refusing to merge".to_string(),
            ));
        }
    }

    // Validate every transform up front so the min/requantise arithmetic below
    // can index [0..3] and divide by scale without panicking or producing NaN.
    for s in sources {
        let t = &s.header().transform;
        if t.scale.len() < 3 || t.translate.len() < 3 {
            return Err(err(
                "CityJSON transform scale/translate must have 3 components".to_string(),
            ));
        }
        if t.scale.iter().take(3).any(|&x| !x.is_finite() || x <= 0.0) {
            return Err(err(
                "CityJSON transform scale must be finite and positive".to_string()
            ));
        }
        if t.translate.iter().take(3).any(|&x| !x.is_finite()) {
            return Err(err(
                "CityJSON transform translate must be finite".to_string()
            ));
        }
    }

    // Transform: adopt the shared one if all equal (features untouched);
    // otherwise componentwise-min scale + translate and requantise below.
    let transforms: Vec<&Transform> = sources.iter().map(|s| &s.header().transform).collect();
    let all_equal = transforms.windows(2).all(|w| transform_eq(w[0], w[1]));
    let merged_transform = if all_equal {
        transforms[0].clone()
    } else {
        let mut scale = transforms[0].scale.clone();
        let mut translate = transforms[0].translate.clone();
        for t in &transforms[1..] {
            for i in 0..3 {
                scale[i] = scale[i].min(t.scale[i]);
                translate[i] = translate[i].min(t.translate[i]);
            }
        }
        Transform { scale, translate }
    };

    // Doc-level template carrier: at most one.
    let carriers: Vec<usize> = sources
        .iter()
        .enumerate()
        .filter(|(_, s)| is_template_carrier(s))
        .map(|(i, _)| i)
        .collect();
    if carriers.len() > 1 {
        return Err(err(
            "more than one input carries doc-level geometry templates; \
             merging their global appearance indices is not supported in this milestone"
                .to_string(),
        ));
    }

    let stems = if opts.prefix_ids_by_input {
        Some(input_stems(sources)?)
    } else {
        None
    };

    // Concatenate features, requantising onto the merged transform where the
    // source's own transform differs from it; with `dedupe_identical`, a
    // feature an earlier input already carries identically is dropped.
    let mut features: Vec<CityJSONFeature> = Vec::new();
    let mut origin: Vec<usize> = Vec::new();
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    let mut deduplicated = 0usize;
    for (si, s) in sources.iter().enumerate() {
        let src_t = &s.header().transform;
        let needs_requantise = !transform_eq(src_t, &merged_transform);
        for f in s.features()? {
            let mut f = f?;
            if needs_requantise {
                requantise_vertices(&mut f.vertices, src_t, &merged_transform);
            }
            if opts.dedupe_identical {
                let seen = by_id.entry(f.id.clone()).or_default();
                let mut identical = false;
                for &k in seen.iter() {
                    if same_feature(&features[k], sources[origin[k]].path(), &f, s.path())? {
                        identical = true;
                        break;
                    }
                }
                if identical {
                    deduplicated += 1;
                    continue;
                }
                seen.push(features.len());
            }
            features.push(f);
            origin.push(si);
        }
    }
    if let Some(stems) = &stems {
        for (f, &si) in features.iter_mut().zip(&origin) {
            prefix_ids(f, &stems[si]);
        }
    }

    // Merged header: first input's, with the merged transform, no
    // geographicalExtent, and the sole carrier's templates (if any).
    let mut header = first.header().clone();
    header.transform = merged_transform;
    if let Some(m) = header.metadata.as_mut() {
        m.geographical_extent = None;
    }
    let doc_appearance = match carriers.first() {
        Some(&ci) => {
            // Take the carrier's templates AND its DOC-LEVEL appearance
            // together — its templates' global material/texture indices
            // resolve against `doc_appearance()`, NOT `header().appearance`
            // (see that method's own doc comment: for a whole-document
            // source, `cjseq`'s `get_metadata()` slices/reindexes a SEPARATE
            // clone to build `header().appearance`, in a different index
            // space than the templates it hands back untouched — using it
            // here would silently corrupt every template's material/texture
            // reference). The merged header's `appearance` is written out as
            // the CityJSONSeq stream's header line, which only ever backs
            // geometry-templates (features carry their own local appearance
            // in Seq form) — so it must be the SAME value as `doc_appearance`
            // below, not a mismatched reslice.
            header.geometry_templates = sources[ci].header().geometry_templates.clone();
            header.appearance = sources[ci].doc_appearance().cloned();
            sources[ci].doc_appearance().cloned()
        }
        None => {
            header.geometry_templates = None;
            None
        }
    };

    // Duplicate feature ids across inputs: count, keep all. The count travels
    // out on `MergedDataset::duplicate_ids` — warning about it is the caller's
    // job, not this library's.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicate_ids = 0usize;
    for f in &features {
        if !seen.insert(f.id.as_str()) {
            duplicate_ids += 1;
        }
    }

    Ok(MergedDataset {
        header,
        features,
        doc_appearance,
        deduplicated,
        duplicate_ids,
    })
}

/// Each input's file stem, refusing an input with none and two inputs with
/// the same one (a prefix naming two inputs keeps nothing apart).
fn input_stems(sources: &[Source]) -> Result<Vec<String>> {
    let mut seen: HashMap<String, &Path> = HashMap::new();
    let mut stems = Vec::with_capacity(sources.len());
    for s in sources {
        let stem = s
            .path()
            .file_stem()
            .and_then(|x| x.to_str())
            .filter(|x| !x.is_empty())
            .ok_or_else(|| {
                err(format!(
                    "cannot prefix ids by input: {} has no file stem",
                    s.path().display()
                ))
            })?
            .to_string();
        if let Some(other) = seen.insert(stem.clone(), s.path()) {
            return Err(err(format!(
                "cannot prefix ids by input: {} and {} share the stem '{stem}'",
                other.display(),
                s.path().display()
            )));
        }
        stems.push(stem);
    }
    Ok(stems)
}

/// Rename every CityObject of `f` `<stem>.<id>`, with the feature id and every
/// `parents`/`children` reference.
fn prefix_ids(f: &mut CityJSONFeature, stem: &str) {
    let rename = |id: &str| format!("{stem}.{id}");
    f.id = rename(&f.id);
    f.city_objects = std::mem::take(&mut f.city_objects)
        .into_iter()
        .map(|(id, mut co)| {
            for refs in [co.parents.as_mut(), co.children.as_mut()]
                .into_iter()
                .flatten()
            {
                for r in refs.iter_mut() {
                    *r = rename(r);
                }
            }
            (rename(&id), co)
        })
        .collect();
}

/// Whether `b` (read from `b_path`) is an identical copy of `a` (from
/// `a_path`); see [`MergeOptions::dedupe_identical`].
fn same_feature(
    a: &CityJSONFeature,
    a_path: &Path,
    b: &CityJSONFeature,
    b_path: &Path,
) -> Result<bool> {
    let mut va = serde_json::to_value(a)?;
    let mut vb = serde_json::to_value(b)?;
    let images_a = take_images(&mut va);
    let images_b = take_images(&mut vb);
    if va != vb || images_a.len() != images_b.len() {
        return Ok(false);
    }
    for (ia, ib) in images_a.iter().zip(&images_b) {
        if ia != ib && !same_bytes(&resolve(a_path, ia), &resolve(b_path, ib)) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Blank every texture's `image` in a feature's JSON, returning them in order.
fn take_images(v: &mut serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(textures) = v
        .pointer_mut("/appearance/textures")
        .and_then(|t| t.as_array_mut())
    {
        for t in textures {
            if let Some(img) = t.get_mut("image") {
                out.push(img.as_str().unwrap_or_default().to_string());
                *img = serde_json::Value::Null;
            }
        }
    }
    out
}

/// An image URI resolved beside the input document that names it.
fn resolve(input: &Path, uri: &str) -> std::path::PathBuf {
    input.parent().unwrap_or(Path::new("")).join(uri)
}

/// True when both files exist and hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}
