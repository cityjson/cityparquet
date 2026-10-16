//! Merge several [`Source`]s into one logical CityJSON dataset for a single
//! (optionally partitioned) conversion. The merged source holds the inputs,
//! not their features: it reads them one after another, so a merge costs no
//! more memory than its largest input's streaming read.
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
//! Object ids must be unique across the inputs; the conversion's scan refuses
//! a repeated one (`crate::scan::scan`).

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Appearance, CityJSON, Transform};

use crate::source::Source;

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

pub(crate) fn transform_eq(a: &Transform, b: &Transform) -> bool {
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
pub(crate) fn requantise_vertices(vertices: &mut [[i64; 3]], src: &Transform, merged: &Transform) {
    let mut ratio = [0f64; 3];
    let mut offset = [0f64; 3];
    for i in 0..3 {
        ratio[i] = src.scale[i] / merged.scale[i];
        offset[i] = (src.translate[i] - merged.translate[i]) / merged.scale[i];
    }
    for v in vertices.iter_mut() {
        for (i, c) in v.iter_mut().enumerate() {
            *c = (*c as f64 * ratio[i] + offset[i]).round() as i64;
        }
    }
}

/// True when this source contributes doc-level geometry templates (which carry
/// global appearance indices resolved against its `doc_appearance`).
fn is_template_carrier(source: &Source) -> bool {
    source.header().geometry_templates.is_some()
}

/// Merge `sources` (non-empty) into one [`Source`] that reads them in turn.
/// See the module docs for the CRS / transform / template rules.
///
/// The merged CRS is the operator's ([`Source::crs_is_operator_supplied`])
/// only if every input's was: the inputs share one CRS, so a single input
/// that declared it itself makes it source-declared.
pub fn merge_sources(sources: Vec<Source>) -> Result<Source> {
    let (header, doc_appearance) = merged_header(&sources)?;
    let crs_is_operator_supplied = sources.iter().all(Source::crs_is_operator_supplied);
    Ok(Source::chained(header, sources, doc_appearance)
        .with_crs_operator_supplied(crs_is_operator_supplied))
}

/// The merged header of `sources` (non-empty), after checking that they can
/// be merged, and the doc-level appearance its templates resolve against.
pub(crate) fn merged_header(sources: &[Source]) -> Result<(CityJSON, Option<Appearance>)> {
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
    // can divide by scale without producing NaN.
    for s in sources {
        let t = &s.header().transform;
        if t.scale.iter().any(|&x| !x.is_finite() || x <= 0.0) {
            return Err(err(
                "CityJSON transform scale must be finite and positive".to_string()
            ));
        }
        if t.translate.iter().any(|&x| !x.is_finite()) {
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
        let mut scale = transforms[0].scale;
        let mut translate = transforms[0].translate;
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

    Ok((header, doc_appearance))
}
