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

use std::collections::HashSet;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Appearance, CityJSON, CityJSONFeature, Transform};

use crate::source::Source;

/// One merged dataset: the shared header (merged transform, first input's
/// metadata sans `geographicalExtent`, the sole template carrier's templates),
/// every input's features concatenated (requantised onto the merged transform
/// where needed), the doc-level appearance the templates resolve against, and
/// how many feature ids collided across inputs.
#[derive(Debug, Clone)]
pub struct MergedDataset {
    pub header: CityJSON,
    pub features: Vec<CityJSONFeature>,
    pub doc_appearance: Option<Appearance>,
    pub duplicate_ids: usize,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

fn transform_eq(a: &Transform, b: &Transform) -> bool {
    a.scale == b.scale && a.translate == b.translate
}

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
pub fn merge_sources(sources: &[Source]) -> Result<MergedDataset> {
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

    // Concatenate features, requantising onto the merged transform where the
    // source's own transform differs from it.
    let mut features: Vec<CityJSONFeature> = Vec::new();
    for s in sources {
        let src_t = &s.header().transform;
        let needs_requantise = !transform_eq(src_t, &merged_transform);
        for f in s.features()? {
            let mut f = f?;
            if needs_requantise {
                requantise_vertices(&mut f.vertices, src_t, &merged_transform);
            }
            features.push(f);
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
            // Take the carrier's templates AND its header appearance together —
            // its templates' global appearance indices, and its default themes,
            // resolve against the carrier's own appearance, not the first
            // source's (which may differ or be absent).
            header.geometry_templates = sources[ci].header().geometry_templates.clone();
            header.appearance = sources[ci].header().appearance.clone();
            sources[ci].doc_appearance().cloned()
        }
        None => {
            header.geometry_templates = None;
            None
        }
    };

    // Duplicate feature ids across inputs: count + warn, keep all.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicate_ids = 0usize;
    for f in &features {
        if !seen.insert(f.id.as_str()) {
            duplicate_ids += 1;
        }
    }
    if duplicate_ids > 0 {
        eprintln!(
            "warning: {duplicate_ids} duplicate feature id(s) across inputs; all kept \
             (a package with duplicate ids cannot faithfully round-trip through export)"
        );
    }

    Ok(MergedDataset {
        header,
        features,
        doc_appearance,
        duplicate_ids,
    })
}
