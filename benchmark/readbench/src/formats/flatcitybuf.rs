//! The FlatCityBuf (FCB) [`FormatRunner`]: maps each [`Scenario`] onto
//! `fcb_core` 0.7's own native mechanisms — the R-tree spatial index, the
//! B+-tree attribute index (built by `fcb ser -A`, the read-benchmark's
//! prep step), and a full [`fcb_core::FcbReader::select_all`] scan where FCB
//! has no better mechanism at all.
//!
//! **Cross-format counting caveat — deliberately NOT papered over here (see
//! [`super::cityparquet`] and [`super::cityjsonseq`]'s own equivalent
//! notes).**
//!
//! FCB is FEATURE-oriented for *storage*: one `CityFeature` per record, each
//! bundling a top-level CityObject together with all of its children
//! (mirroring CityJSONSeq's own feature convention) — `fcb ser` builds
//! exactly one feature per top-level CityObject regardless of whether the
//! *source* was CityJSONSeq or a single-document CityJSON file (the
//! `lod3_railway.city.json` fixture, a single CityJSON document with 121
//! CityObjects, becomes an FCB file with 38 features — one per top-level
//! object, confirmed via `fcb info`). But that storage granularity does NOT
//! mean every scenario counts at feature level — each mechanism's own
//! result cardinality decides that, empirically confirmed against this
//! runner's own tests rather than assumed from the storage layout:
//!
//! - [`Scenario::Count`]/[`Scenario::FullRead`] are feature-level: FCB's
//!   header `features_count` and a full scan both operate on, and count,
//!   features.
//! - [`Scenario::BBoxQuery`]'s R-tree is built with one node per FEATURE
//!   (its overall, all-CityObjects-unioned extent), so a spatial query's
//!   match count is feature-level too.
//! - [`Scenario::AttrFilter`]/[`Scenario::IdLookup`]'s B+-tree attribute
//!   index is built from one entry per matching CityObject's OWN attribute
//!   occurrence (see [`fcb_core::reader::attr_query`]'s
//!   `build_attribute_index_for_attr`, called once per CityObject via
//!   `attribute_entries.values()`/`feature.index_entries`), each entry
//!   carrying its enclosing feature's offset — but those per-CityObject
//!   entries are never deduplicated by feature offset before being counted
//!   (`select_attr_query` sorts, never dedups, its result `Vec<u64>`).
//!   Its match count is therefore CityOBJECT-level in practice: querying
//!   `lod3_railway.city.json`'s FCB for `function == "1070"` returns 65 —
//!   exactly the fixture's 65 matching CityObjects, not a feature count
//!   (the fixture only has 38 features total; empirically confirmed in
//!   this module's own tests, not merely asserted). A feature with two
//!   matching CityObjects contributes its offset TWICE to the result.
//! - [`Scenario::AttrStats`]/[`Scenario::Project`] have no B+-tree fallback
//!   in FCB at all regardless of indexing (see below) — this runner's own
//!   `select_all` walk deliberately flattens to CityObject level too (one
//!   count per CityObject carrying the attribute, not one per feature),
//!   matching [`Scenario::AttrFilter`]'s own now-established granularity
//!   and [`super::cityjsonseq`]'s convention for these same four
//!   scenarios.
//!
//! Net effect: [`Scenario::Count`]/[`Scenario::FullRead`]/
//! [`Scenario::BBoxQuery`] are feature-level (their own genuinely-native
//! FCB mechanism); [`Scenario::AttrFilter`]/[`Scenario::AttrStats`]/
//! [`Scenario::Project`]/[`Scenario::IdLookup`] are CityObject-level (either
//! because that's what FCB's own B+-tree naturally returns, or — for the
//! two scenarios with no index at all — this runner's own deliberate
//! choice to match that same granularity). This still does NOT reproduce
//! CityParquet's own CityObject-row counts (2231 on delft, vs. FCB's own
//! 1115 features) — the milestone's methodology doc is responsible for
//! disclosing the feature-vs-object split alongside the numbers, not this
//! runner papering over it.
//!
//! **What the full walks actually materialise.** Every walk below reads
//! the RAW FlatBuffers `CityFeature` (`FeatureIter::cur_feature`), never
//! `cur_cj_feature`: FCB is a zero-copy format, and a comparison baseline
//! is owed its own best natural implementation. `cur_cj_feature` runs
//! `fcb_core`'s `to_cj_feature`, which decodes every geometry into nested
//! `serde_json` boundary arrays, converts every vertex, decodes every
//! attribute into a `serde_json::Map` and allocates a `String` id per
//! CityObject — to answer questions that need one enum comparison, one
//! borrowed `&str` comparison or one column. Measured on `3dbag_n10000`, a
//! type walk cost 0.389 s that way against 0.033 s through the raw
//! accessors, which made FCB's `attr-filter`/`attr-stats`/`project`/
//! `id-lookup` rows cost the same as its `full-read` row and measured the
//! conversion, not the format. So:
//!
//! - [`Scenario::FullRead`] reads every feature's geometry: for every
//!   CityObject, every standard geometry's five flattened index arrays
//!   (`solids`/`shells`/`surfaces`/`strings`/`boundaries`) plus its
//!   per-surface `semantics` indices, every template instance's own
//!   boundary array, and every one of the feature's quantised vertices.
//!   Those arrays ARE the nesting a CityJSON `boundaries` tree encodes, so
//!   this is the same read work `fcb_core`'s own `decode` does without the
//!   nested `Vec` it allocates on top. NOT read: `semantics_objects`,
//!   `material`, `texture` and the feature `appearance` — semantic-surface
//!   attribute tables and appearance mappings, not geometry.
//! - [`Scenario::AttrFilter`]'s fallback tests the reserved `object_type`
//!   column against `co.type_()` (a flatbuffer enum, mapped to its
//!   CityJSON type string exactly as `to_cj_feature`'s own `to_cj_co_type`
//!   does), and any other column by decoding ONLY that column's value out
//!   of the CityObject's packed attribute blob.
//! - [`Scenario::IdLookup`]'s fallback compares `co.id()`, a borrowed
//!   `&str`, and exits at the first hit.
//! - [`Scenario::AttrStats`]/[`Scenario::Project`] decode that one
//!   attribute column and nothing else: no geometry at all.
//!
//! The RESULT of every one of those is unchanged — same counting unit,
//! same predicate semantics, same numbers — only the work behind it is.
//!
//! **Attribute B+-tree vs. full scan.** FCB's B+-tree only indexes the
//! CityJSON `attributes` map (built by `fcb ser -A`); reserved/structural
//! fields like `object_type` ("type") and a CityObject's own id are never
//! part of that schema, so a query against either always falls back to a
//! full [`fcb_core::FcbReader::select_all`] walk here (checked once per
//! call via the header's own column schema, not assumed) — this is
//! expected given `-A`, not a bug. [`Scenario::AttrStats`] and
//! [`Scenario::Project`] always use that same full walk regardless of
//! whether the column is indexed: FCB's B+-tree only supports point/range
//! *filtering*, not columnar aggregation, so there is no faster native
//! mechanism to measure — the full scan IS the honest cost.
//!
//! **Spatial index dimensionality.** FCB's packed R-tree
//! ([`fcb_core::SpatialQuery::BBox`]) is 2D; [`Scenario::BBoxQuery`]'s
//! query window's z-components are dropped rather than approximated.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use bytes::Bytes;
// `CityObject`/`CityFeature`/`Geometry` here are FCB's own FLATBUFFER
// tables, not the `cjseq2` CityJSON types `fcb_core`'s `to_cj_feature`
// builds — every walk below reads the flatbuffer directly (see this
// module's own doc comment on what each scenario materialises).
use fcb_core::{
    AttrQuery, CityObject, CityObjectType, ColumnType, FcbReader, FixedStringKey, Float, Geometry,
    GeometryInstance, Header, HttpFcbReader, KeyType, Operator, SpatialQuery,
};
use http_range_client::{AsyncBufferedHttpRangeClient, AsyncHttpRangeClient};

use super::{FormatRunner, IoStats, RunOutcome, Source};
use crate::scenario::{AttrPred, QueryParams, Scenario};

/// The markers every index-fallback message below carries, and the exact
/// tags the coordinator records in the CSV's `notes` column for a row whose
/// child printed one (`coordinator::child_disclosures`).
///
/// A fallback is a DIFFERENT MECHANISM — a full `select_all` walk where the
/// row's neighbours in the table used an index — so a row that took one is
/// not the indexed query the chart's axis label claims. Announcing that on
/// stderr alone left it out of the artefact entirely, where every reader of
/// the published CSV is.
pub(crate) const FALLBACK_MARKERS: [&str; 2] = ["no-attr-index", "attr-index-failed"];

/// This runner's `--attr-column`/params error for a scenario missing a
/// required field — mirrors [`super::cityparquet`] and
/// [`super::cityjsonseq`]'s own `require` helper.
fn require<'a, T>(opt: &'a Option<T>, flag: &str, scenario: Scenario) -> Result<&'a T> {
    opt.as_ref()
        .ok_or_else(|| anyhow!("scenario '{scenario}' requires --{flag}"))
}

/// Opens `input` fresh (this runner never keeps a reader across scenario
/// calls — see [`FormatRunner`]'s own doc comment on why).
fn open(input: &Path) -> Result<FcbReader<BufReader<File>>> {
    let file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    Ok(FcbReader::open(BufReader::new(file))?)
}

/// One attribute column of FCB's header schema, copied out of the
/// flatbuffer so it outlives the [`FcbReader`] the header was read from —
/// [`FcbReader::select_all`] consumes the reader, and the walks below need
/// the schema for every feature they then visit. Copying it once per call
/// costs one small `Vec` where borrowing would cost a second file open.
#[derive(Clone, Debug)]
struct ColumnMeta {
    /// The column's own `index` field, which is what the packed
    /// attribute blob's per-value tag carries — NOT its position in the
    /// schema vector (FCB's own `decode_attributes` looks it up the same
    /// way).
    index: u16,
    kind: ColumnType,
    name: String,
}

/// `header`'s attribute schema, owned. `None` when the header carries no
/// column schema at all (in which case only a CityObject's own
/// `columns()` override can describe its attribute blob — see
/// [`attribute_value`], which mirrors `to_cj_feature`'s own rule).
fn owned_columns(header: &Header<'_>) -> Option<Vec<ColumnMeta>> {
    header.columns().map(|columns| {
        columns
            .iter()
            .map(|c| ColumnMeta {
                index: c.index(),
                kind: c.type_(),
                name: c.name().to_string(),
            })
            .collect()
    })
}

/// `co`'s CityJSON type string, straight off the flatbuffer enum — the
/// zero-decode equivalent of the `to_cj_co_type(co.type_(),
/// co.extension_type())` call `fcb_core`'s own `to_cj_feature` makes, and
/// deliberately identical to it in every branch: an `ExtensionObject`
/// reads its `extension_type` string and falls back to `"Unknown"` when it
/// has none, and an enum discriminant `fcb_core` itself does not recognise
/// is `"Unknown"` too. Borrowed, never allocated: this is evaluated once
/// per CityObject on a full walk.
fn co_type_str<'a>(co: &CityObject<'a>) -> &'a str {
    if co.type_() == CityObjectType::ExtensionObject {
        return co.extension_type().unwrap_or("Unknown");
    }
    match co.type_().variant_name() {
        Some(name) => name,
        None => "Unknown",
    }
}

/// The reserved `object_type` column's predicate evaluation: exactly what
/// [`matches_predicate`] would return for `serde_json::Value::String(s)`,
/// without allocating that `Value` once per CityObject.
fn matches_str_predicate(s: &str, pred: &AttrPred) -> bool {
    match pred {
        AttrPred::Eq(want) => {
            if let Some(want_str) = want.as_str() {
                s == want_str
            } else if let Some(want_num) = want.as_f64() {
                // A JSON-string value's own `as_f64()` is always `None`, so
                // only the re-parse branch of `matches_predicate` can fire.
                s.parse::<f64>().ok() == Some(want_num)
            } else {
                false
            }
        }
        // Every remaining predicate goes through `Value::as_f64()`, which
        // is `None` for a string — a type string never matches a range.
        AttrPred::Ge(_) | AttrPred::Le(_) | AttrPred::Range(_, _) => false,
    }
}

/// Reads `target`'s value out of one CityObject's packed attribute blob,
/// decoding ONLY that column: every other column's value is skipped by its
/// encoded width, so nothing but the one answer is ever allocated. This is
/// the targeted counterpart of `fcb_core`'s own
/// [`fcb_core::reader::deserializer::decode_attributes`], whose layout it
/// follows byte for byte — a `u16` column tag followed by that column's
/// fixed-width value, or a `u32` length followed by that many bytes for the
/// variable-width `String`/`DateTime`/`Json` types.
///
/// The whole blob is scanned even after a hit, so a column encoded twice
/// resolves to its LAST occurrence — which is what `decode_attributes`'
/// own `serde_json::Map::insert` does. Skipping costs pointer arithmetic
/// only.
///
/// `type_of` maps a column tag to its type against whichever schema
/// applies (the header's, or the CityObject's own `columns()` override);
/// `column_count` is that schema's length, the same bound
/// `decode_attributes` checks. Where `decode_attributes` panics — an
/// out-of-range tag, an unhandled column type, malformed JSON — this
/// returns an error instead, so a malformed file fails the run with a
/// message rather than aborting the child process.
fn scan_attribute(
    bytes: &[u8],
    target: u16,
    column_count: usize,
    type_of: &dyn Fn(u16) -> Option<ColumnType>,
) -> Result<Option<serde_json::Value>> {
    /// `len` bytes at `offset`, or an error naming the truncation.
    fn take(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
        bytes
            .get(offset..offset + len)
            .ok_or_else(|| anyhow!("attribute blob truncated at byte {offset} (wanted {len})"))
    }

    let mut found = None;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let tag = take(bytes, offset, 2)?;
        let col_index = u16::from_le_bytes([tag[0], tag[1]]);
        offset += 2;
        if col_index as usize >= column_count {
            bail!("attribute column index {col_index} out of range ({column_count} columns)");
        }
        let kind = type_of(col_index)
            .ok_or_else(|| anyhow!("attribute column index {col_index} is not in the schema"))?;
        let wanted = col_index == target;

        macro_rules! fixed {
            ($width:expr, $decode:expr) => {{
                let raw = take(bytes, offset, $width)?;
                if wanted {
                    let decode: fn(&[u8]) -> Option<serde_json::Value> = $decode;
                    found = Some(decode(raw));
                }
                offset += $width;
            }};
        }

        match kind {
            ColumnType::Bool => fixed!(1, |b| Some(serde_json::Value::Bool(b[0] != 0))),
            ColumnType::Short => fixed!(2, |b| Some(serde_json::Value::from(i16::from_le_bytes(
                b.try_into().expect("2 bytes")
            )))),
            ColumnType::UShort => fixed!(2, |b| Some(serde_json::Value::from(u16::from_le_bytes(
                b.try_into().expect("2 bytes")
            )))),
            ColumnType::Int => fixed!(4, |b| Some(serde_json::Value::from(i32::from_le_bytes(
                b.try_into().expect("4 bytes")
            )))),
            ColumnType::UInt => fixed!(4, |b| Some(serde_json::Value::from(u32::from_le_bytes(
                b.try_into().expect("4 bytes")
            )))),
            ColumnType::Long => fixed!(8, |b| Some(serde_json::Value::from(i64::from_le_bytes(
                b.try_into().expect("8 bytes")
            )))),
            ColumnType::ULong => fixed!(8, |b| Some(serde_json::Value::from(u64::from_le_bytes(
                b.try_into().expect("8 bytes")
            )))),
            // A non-finite float has no `serde_json::Number`, so
            // `decode_attributes` leaves the key out of the map entirely —
            // i.e. the column reads as ABSENT, which `None` says here.
            ColumnType::Float => fixed!(4, |b| {
                let f = f32::from_le_bytes(b.try_into().expect("4 bytes"));
                serde_json::Number::from_f64(f as f64).map(serde_json::Value::Number)
            }),
            ColumnType::Double => fixed!(8, |b| {
                let f = f64::from_le_bytes(b.try_into().expect("8 bytes"));
                serde_json::Number::from_f64(f).map(serde_json::Value::Number)
            }),
            ColumnType::String | ColumnType::DateTime | ColumnType::Json => {
                let len_bytes = take(bytes, offset, 4)?;
                let len = u32::from_le_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
                offset += 4;
                let raw = take(bytes, offset, len)?;
                if wanted {
                    // `decode_attributes` is lossy the same way here: an
                    // invalid UTF-8 payload becomes the empty string.
                    let s = String::from_utf8(raw.to_vec()).unwrap_or_default();
                    found = Some(if kind == ColumnType::Json {
                        Some(serde_json::from_str(&s).with_context(|| {
                            format!("parsing a Json-typed attribute column's value: {s}")
                        })?)
                    } else {
                        Some(serde_json::Value::String(s))
                    });
                }
                offset += len;
            }
            other => bail!(
                "attribute column type {other:?} has no decoding in fcb_core 0.7.6 \
                 (its own `decode_attributes` panics on it)"
            ),
        }
    }
    Ok(found.flatten())
}

/// `column`'s value on the flatbuffer CityObject `co`, decoded from `co`'s
/// own attribute blob alone — the raw-accessor replacement for the
/// `to_cj_feature` round trip the full walks used to make. `root` is the
/// header's own schema (see [`owned_columns`]).
///
/// The reserved `object_type` column is NOT handled here: it is never part
/// of FCB's attribute schema, and its callers read [`co_type_str`]
/// directly.
///
/// Every branch mirrors `to_cj_feature`'s own attribute handling: no
/// schema at all (neither header nor per-object) means no attributes; a
/// CityObject carrying its own `columns()` is decoded against THAT schema;
/// an empty blob is an empty map; and a JSON-`null` value reads as absent,
/// as [`super::cityjsonseq`]'s own `column_value` also has it.
fn attribute_value(
    co: &CityObject<'_>,
    root: Option<&[ColumnMeta]>,
    column: &str,
) -> Result<Option<serde_json::Value>> {
    let own_columns = co.columns();
    if root.is_none() && own_columns.is_none() {
        return Ok(None);
    }
    let Some(attributes) = co.attributes() else {
        return Ok(None);
    };
    if attributes.is_empty() {
        return Ok(None);
    }
    let value = if let Some(columns) = own_columns {
        let Some(target) = columns
            .iter()
            .find(|c| c.name() == column)
            .map(|c| c.index())
        else {
            return Ok(None);
        };
        scan_attribute(attributes.bytes(), target, columns.len(), &|index| {
            columns
                .iter()
                .find(|c| c.index() == index)
                .map(|c| c.type_())
        })?
    } else {
        let columns = root.expect("checked above");
        let Some(target) = columns.iter().find(|c| c.name == column).map(|c| c.index) else {
            return Ok(None);
        };
        scan_attribute(attributes.bytes(), target, columns.len(), &|index| {
            columns.iter().find(|c| c.index == index).map(|c| c.kind)
        })?
    };
    Ok(value.filter(|v| !v.is_null()))
}

/// Whether `co` matches `pred` on `column`, reading only what the answer
/// needs: one flatbuffer enum for the reserved `object_type` column, one
/// targeted blob scan for anything else.
fn co_matches(
    co: &CityObject<'_>,
    root: Option<&[ColumnMeta]>,
    column: &str,
    pred: &AttrPred,
) -> Result<bool> {
    if column == "object_type" {
        return Ok(matches_str_predicate(co_type_str(co), pred));
    }
    Ok(matches_predicate(
        attribute_value(co, root, column)?.as_ref(),
        pred,
    ))
}

/// Folds every index of one flatbuffer `u32` vector into a checksum: the
/// point is the TOUCH, not the sum — every element is read off the buffer
/// (little-endian, one at a time, exactly as a decoder would), and the
/// result is `black_box`ed by [`full_read`] so the traversal cannot be
/// optimised away as dead code.
fn touch_indices(values: impl Iterator<Item = u32>) -> u64 {
    values.fold(0u64, |acc, n| acc.wrapping_add(u64::from(n)))
}

/// One geometry's full boundary traversal: `(leaves, checksum)`, where
/// `leaves` is the number of vertex indices in its `boundaries` array —
/// the flattened equivalent of [`super::cityjsonseq::count_boundary_leaves`]
/// over the nested CityJSON form — and `checksum` folds every element of
/// EVERY one of FCB's five flattened arrays (`solids`, `shells`,
/// `surfaces`, `strings`, `boundaries`) plus the per-surface `semantics`
/// index array. Between them those arrays ARE the nesting structure a
/// CityJSON `boundaries` tree encodes, so touching all of them is the same
/// read work `fcb_core`'s own `decode` does, minus the nested `Vec`
/// allocation it builds on top.
///
/// Deliberately NOT touched: `semantics_objects`, `material` and `texture`.
/// Those are semantic-surface attribute tables and appearance mappings, not
/// geometry — see this module's own doc comment, which states exactly what
/// [`Scenario::FullRead`] materialises.
fn geometry_work(geom: &Geometry<'_>) -> (u64, u64) {
    let mut checksum = 0u64;
    let mut leaves = 0u64;
    if let Some(solids) = geom.solids() {
        checksum = checksum.wrapping_add(touch_indices(solids.iter()));
    }
    if let Some(shells) = geom.shells() {
        checksum = checksum.wrapping_add(touch_indices(shells.iter()));
    }
    if let Some(surfaces) = geom.surfaces() {
        checksum = checksum.wrapping_add(touch_indices(surfaces.iter()));
    }
    if let Some(strings) = geom.strings() {
        checksum = checksum.wrapping_add(touch_indices(strings.iter()));
    }
    if let Some(semantics) = geom.semantics() {
        checksum = checksum.wrapping_add(touch_indices(semantics.iter()));
    }
    if let Some(boundaries) = geom.boundaries() {
        leaves += boundaries.len() as u64;
        checksum = checksum.wrapping_add(touch_indices(boundaries.iter()));
    }
    (leaves, checksum)
}

/// A template instance's own boundary array (its single reference point),
/// counted and touched exactly as [`geometry_work`] counts a standard
/// geometry's — `to_cj_feature` decodes instances alongside standard
/// geometries, so a full read that skipped them would read less than the
/// path it replaces.
fn geometry_instance_work(instance: &GeometryInstance<'_>) -> (u64, u64) {
    match instance.boundaries() {
        Some(boundaries) => (boundaries.len() as u64, touch_indices(boundaries.iter())),
        None => (0, 0),
    }
}

/// One feature's whole geometry traversal: every CityObject's standard
/// geometries and template instances, plus every one of the feature's own
/// quantised vertices (read as the three `i32`s FCB stores, the same
/// coordinates `to_cj_vertices` would copy into a `Vec<Vec<i64>>`).
fn feature_geometry_work(feature: &fcb_core::CityFeature<'_>) -> (u64, u64) {
    let mut leaves = 0u64;
    let mut checksum = 0u64;
    if let Some(vertices) = feature.vertices() {
        for v in vertices.iter() {
            checksum = checksum
                .wrapping_add(v.x() as i64 as u64)
                .wrapping_add(v.y() as i64 as u64)
                .wrapping_add(v.z() as i64 as u64);
        }
    }
    if let Some(objects) = feature.objects() {
        for co in objects.iter() {
            if let Some(geometries) = co.geometry() {
                for geom in geometries.iter() {
                    let (l, c) = geometry_work(&geom);
                    leaves += l;
                    checksum = checksum.wrapping_add(c);
                }
            }
            if let Some(instances) = co.geometry_instances() {
                for instance in instances.iter() {
                    let (l, c) = geometry_instance_work(&instance);
                    leaves += l;
                    checksum = checksum.wrapping_add(c);
                }
            }
        }
    }
    (leaves, checksum)
}

/// This runner's own attribute-predicate evaluation, used only by the
/// full-scan fallback paths — the FlatCityBuf analogue of
/// [`super::cityjsonseq`]'s own `matches_predicate`.
fn matches_predicate(value: Option<&serde_json::Value>, pred: &AttrPred) -> bool {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return false;
    };
    match pred {
        AttrPred::Eq(want) => {
            if let Some(want_str) = want.as_str() {
                // `--attr-eq` always produces a string `want` now (see
                // `main.rs::build_attr_pred`), so a String-typed attribute
                // value (e.g. `"1070"`) is compared against `want` here
                // directly; the `as_f64` branch below only matters for a
                // non-CLI caller passing a genuine JSON-number `want`
                // against a numeric-looking string cell.
                value.as_str() == Some(want_str)
            } else if let Some(want_num) = want.as_f64() {
                value.as_f64() == Some(want_num)
                    || value
                        .as_str()
                        .is_some_and(|s| s.parse::<f64>().ok() == Some(want_num))
            } else {
                false
            }
        }
        AttrPred::Ge(bound) => value.as_f64().is_some_and(|v| v >= *bound),
        AttrPred::Le(bound) => value.as_f64().is_some_and(|v| v <= *bound),
        AttrPred::Range(lo, hi) => value.as_f64().is_some_and(|v| v >= *lo && v <= *hi),
    }
}

/// `column`'s [`ColumnType`] in `header`'s attribute schema, or `None` if
/// `column` isn't part of it at all (e.g. `object_type`/`id`, which FCB's
/// attribute B+-tree never indexes since they aren't part of the CityJSON
/// `attributes` map, or any attribute genuinely absent from this dataset).
fn column_type_for(header: &Header<'_>, column: &str) -> Option<ColumnType> {
    header
        .columns()?
        .iter()
        .find(|c| c.name() == column)
        .map(|c| c.type_())
}

/// A numeric predicate bound cast to `column_type`'s matching [`KeyType`]
/// variant — every `ColumnType` FCB's own writer ever infers for a numeric
/// or boolean CityJSON attribute value (see `fcb_core`'s
/// `writer::attribute::detect_column_type`); `ColumnType::String` and the
/// remaining non-numeric variants (`DateTime`/`Json`/`Binary`) have no
/// meaningful cast and are rejected — a query against one of those with a
/// numeric bound falls back to a full scan (see [`build_attr_query`]'s
/// caller).
fn numeric_key(column_type: ColumnType, value: f64) -> Result<KeyType> {
    Ok(match column_type {
        ColumnType::Bool => KeyType::Bool(value != 0.0),
        ColumnType::Byte => KeyType::Int8(value as i8),
        ColumnType::UByte => KeyType::UInt8(value as u8),
        ColumnType::Short => KeyType::Int16(value as i16),
        ColumnType::UShort => KeyType::UInt16(value as u16),
        ColumnType::Int => KeyType::Int32(value as i32),
        ColumnType::UInt => KeyType::UInt32(value as u32),
        ColumnType::Long => KeyType::Int64(value as i64),
        ColumnType::ULong => KeyType::UInt64(value as u64),
        ColumnType::Float => KeyType::Float32(Float(value as f32)),
        ColumnType::Double => KeyType::Float64(Float(value)),
        other => bail!("column type {other:?} has no numeric key mapping"),
    })
}

/// An [`AttrPred::Eq`] value cast to `column_type`'s matching [`KeyType`]
/// variant: a string value against a `ColumnType::String` column becomes a
/// `StringKey50` (the fixed width FCB's own `build_attribute_index_for_attr`
/// always uses for `ColumnType::String`, regardless of the source string's
/// own length); a numeric value against any numeric/bool column goes
/// through [`numeric_key`]; any other combination (e.g. a string value
/// against a numeric column) has no meaningful cast.
///
/// One historical exception, now DEAD via the CLI but kept as harmless
/// defensive handling for any direct (non-CLI) caller: many real CityJSON
/// attributes are String-typed numeric *codes* (e.g. this benchmark's own
/// `lod3_railway.city.json` fixture has `"function": "1070"` — a string,
/// not a number). `main.rs::build_attr_pred` used to eagerly parse any
/// numeric-looking `--attr-eq` value into a JSON number, losing the fact
/// that it was meant to match a string column — that bug is now fixed at
/// its one source (`--attr-eq` always produces `Eq(Value::String(_))`, see
/// `build_attr_pred`'s own doc comment), so every CLI-driven call into this
/// function now always hits the `value.as_str()` branch directly. The
/// `value.as_f64()` branch below only still matters if some future non-CLI
/// caller passes a `ColumnType::String` column paired with a genuine JSON
/// number `value`: it re-stringifies that number back to its canonical
/// form (integer formatting when the value has no fractional part) before
/// building the `StringKey50` — recovering the original string code's exact
/// bytes for any code that round-trips through `f64`.
fn eq_key(column_type: ColumnType, value: &serde_json::Value) -> Result<KeyType> {
    match column_type {
        ColumnType::String => {
            let owned_from_number;
            let s: &str = if let Some(s) = value.as_str() {
                s
            } else if let Some(n) = value.as_f64() {
                owned_from_number = if n.fract() == 0.0 && n.abs() < 1e15 {
                    (n as i64).to_string()
                } else {
                    n.to_string()
                };
                &owned_from_number
            } else {
                bail!(
                    "--attr-eq value is neither a string nor a number, but column is String-typed"
                );
            };
            Ok(KeyType::StringKey50(FixedStringKey::from_str(s)))
        }
        _ => {
            let n = value.as_f64().ok_or_else(|| {
                anyhow!("--attr-eq value is not numeric, but column type is {column_type:?}")
            })?;
            numeric_key(column_type, n)
        }
    }
}

/// `pred` translated into `fcb_core`'s own `AttrQuery` (`Vec<(String,
/// Operator, KeyType)>`) against `column`, using `column_type` to build the
/// matching `KeyType` variant. [`AttrPred::Range`] becomes two ANDed
/// conditions (`Ge` + `Le`), matching `AttrQuery`'s own all-conditions-AND
/// contract.
fn build_attr_query(column: &str, pred: &AttrPred, column_type: ColumnType) -> Result<AttrQuery> {
    Ok(match pred {
        AttrPred::Eq(value) => vec![(
            column.to_string(),
            Operator::Eq,
            eq_key(column_type, value)?,
        )],
        AttrPred::Ge(bound) => vec![(
            column.to_string(),
            Operator::Ge,
            numeric_key(column_type, *bound)?,
        )],
        AttrPred::Le(bound) => vec![(
            column.to_string(),
            Operator::Le,
            numeric_key(column_type, *bound)?,
        )],
        AttrPred::Range(lo, hi) => vec![
            (
                column.to_string(),
                Operator::Ge,
                numeric_key(column_type, *lo)?,
            ),
            (
                column.to_string(),
                Operator::Le,
                numeric_key(column_type, *hi)?,
            ),
        ],
    })
}

/// Builds the `AttrQuery` for `column`/`pred` against `input`'s own
/// attribute schema (a fresh open, closed again immediately after — the
/// schema read is a few KiB off the front of the file, negligible next to
/// an actual index traversal or full scan). `None` if `column` isn't part
/// of the schema at all, or if `pred`'s value doesn't cast onto the
/// column's actual [`ColumnType`] — either way, the caller's job is then a
/// full [`select_all`] walk instead.
fn attr_query_for(input: &Path, column: &str, pred: &AttrPred) -> Result<Option<AttrQuery>> {
    let reader = open(input)?;
    let column_type = {
        let header = reader.header();
        column_type_for(&header, column)
    };
    let Some(column_type) = column_type else {
        return Ok(None);
    };
    Ok(build_attr_query(column, pred, column_type).ok())
}

/// Every feature in `input`, counting one FEATURE per record and decoding
/// every one of its geometries through the raw flatbuffer accessors (see
/// [`feature_geometry_work`] for exactly which arrays are read). The
/// boundary-leaf and checksum totals are `black_box`ed rather than
/// returned: they exist to force the traversal to happen, and the returned
/// metric stays feature-level per this module's own doc comment — the same
/// arrangement [`super::cityjsonseq`]'s own `FullRead` uses.
fn full_read(input: &Path) -> Result<u64> {
    let reader = open(input)?;
    let mut iter = reader.select_all()?;
    let mut feature_count = 0u64;
    let mut leaves = 0u64;
    let mut checksum = 0u64;
    while let Some(feat) = iter.next()? {
        let (l, c) = feature_geometry_work(&feat.cur_feature());
        feature_count += 1;
        leaves += l;
        checksum = checksum.wrapping_add(c);
    }
    std::hint::black_box((leaves, checksum));
    Ok(feature_count)
}

/// A full `select_all` walk, counting every CityObject (across every
/// feature, parents AND children) matching `pred` on `column` — CityObject
/// level, matching [`attr_filter`]'s own indexed-path granularity (see this
/// module's own doc comment) and [`super::cityjsonseq`]'s convention for
/// this same scenario. Used as [`Scenario::AttrFilter`]'s fallback when
/// `column` isn't indexed.
///
/// No geometry is decoded and no CityJSON feature is built: each CityObject
/// is tested through [`co_matches`], which reads one flatbuffer enum for
/// the reserved `object_type` column and otherwise decodes only `column`'s
/// own value out of that object's attribute blob.
fn full_walk_attr_filter(input: &Path, column: &str, pred: &AttrPred) -> Result<u64> {
    let reader = open(input)?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all()?;
    let mut matched = 0u64;
    while let Some(feat) = iter.next()? {
        let Some(objects) = feat.cur_feature().objects() else {
            continue;
        };
        for co in objects.iter() {
            if co_matches(&co, root.as_deref(), column, pred)? {
                matched += 1;
            }
        }
    }
    Ok(matched)
}

/// A full `select_all` walk, short-circuiting as soon as a CityObject whose
/// own `id` is `id` is found — one borrowed `&str` comparison per object,
/// no decoding of anything else.
fn full_walk_id_lookup(input: &Path, id: &str) -> Result<u64> {
    let reader = open(input)?;
    let mut iter = reader.select_all()?;
    while let Some(feat) = iter.next()? {
        if let Some(objects) = feat.cur_feature().objects()
            && objects.iter().any(|co| co.id() == id)
        {
            return Ok(1);
        }
    }
    Ok(0)
}

/// [`Scenario::AttrFilter`]: tries `column`'s B+-tree attribute index first
/// (built by `fcb ser -A`); falls back to [`full_walk_attr_filter`] when
/// `column` isn't indexed (see this module's own doc comment — expected
/// for reserved fields like `object_type`, not a bug) or when the index
/// query itself errors for any reason.
fn attr_filter(input: &Path, column: &str, pred: &AttrPred) -> Result<u64> {
    if let Some(query) = attr_query_for(input, column, pred)? {
        let reader = open(input)?;
        match reader.select_attr_query(query) {
            Ok(iter) => return Ok(iter.features_count().unwrap_or(0) as u64),
            Err(e) => eprintln!(
                "cityparquet-readbench: flatcitybuf: indexed attr-filter query on '{column}' \
                 failed ({e}) (attr-index-failed); falling back to a full scan"
            ),
        }
    } else {
        eprintln!(
            "cityparquet-readbench: flatcitybuf: attribute '{column}' has no B+-tree index \
             (no-attr-index); falling back to a full scan"
        );
    }
    full_walk_attr_filter(input, column, pred)
}

/// [`Scenario::IdLookup`]: tries `id`'s B+-tree attribute index first (in
/// case a future `fcb_core` release indexes it), but in practice `id` is a
/// CityObject's map key, never part of the CityJSON `attributes` map FCB's
/// schema covers, so this always takes the [`full_walk_id_lookup`]
/// fallback on real data — documented, not a bug (see this module's own
/// doc comment).
fn id_lookup(input: &Path, id: &str) -> Result<u64> {
    let pred = AttrPred::Eq(serde_json::Value::String(id.to_string()));
    if let Some(query) = attr_query_for(input, "id", &pred)? {
        let reader = open(input)?;
        if let Ok(mut iter) = reader.select_attr_query(query) {
            return Ok(if iter.next()?.is_some() { 1 } else { 0 });
        }
        eprintln!(
            "cityparquet-readbench: flatcitybuf: indexed id-lookup query failed \
             (attr-index-failed); falling back to a full scan"
        );
    } else {
        eprintln!(
            "cityparquet-readbench: flatcitybuf: 'id' has no B+-tree index (no-attr-index); \
             falling back to a full scan"
        );
    }
    full_walk_id_lookup(input, id)
}

/// [`Scenario::AttrStats`]: always a full `select_all` walk (FCB's B+-tree
/// has no columnar aggregation mechanism — see this module's own doc
/// comment), counting every CityObject (across every feature) carrying a
/// numeric value for `column` — CityObject level, matching [`attr_filter`]'s
/// own granularity.
///
/// Attributes only: no geometry is decoded, and only `column`'s own value
/// is read out of each object's attribute blob. The reserved `object_type`
/// column is a type STRING, whose `as_f64()` is `None`, so — exactly as
/// before — it contributes nothing here.
fn attr_stats(input: &Path, column: &str) -> Result<u64> {
    let reader = open(input)?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all()?;
    let mut count = 0u64;
    while let Some(feat) = iter.next()? {
        let Some(objects) = feat.cur_feature().objects() else {
            continue;
        };
        for co in objects.iter() {
            // The reserved column shadows any same-named attribute, and a
            // type string is never numeric — so it contributes nothing.
            let numeric = column != "object_type"
                && attribute_value(&co, root.as_deref(), column)?
                    .and_then(|v| v.as_f64())
                    .is_some();
            if numeric {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// [`Scenario::Project`]: always a full `select_all` walk (same rationale
/// as [`attr_stats`]), counting every CityObject (across every feature)
/// carrying a non-null value for `column` — CityObject level.
///
/// Attributes only: no geometry is decoded. The reserved `object_type`
/// column is present on every CityObject, so — exactly as before — it
/// counts every one of them.
fn project(input: &Path, column: &str) -> Result<u64> {
    let reader = open(input)?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all()?;
    let mut count = 0u64;
    while let Some(feat) = iter.next()? {
        let Some(objects) = feat.cur_feature().objects() else {
            continue;
        };
        for co in objects.iter() {
            let present = if column == "object_type" {
                true
            } else {
                attribute_value(&co, root.as_deref(), column)?.is_some()
            };
            if present {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Request count + bytes tally shared (via `Arc`) across every
/// [`CountingRangeClient`] clone created for one [`run_http`] call —
/// `fcb_core`'s HTTP reader reopens a fresh reader per scenario stage (see
/// [`open_http`], mirroring the local runner's own per-call `open`), so the
/// tally must outlive any single reader to accumulate across all of them.
#[derive(Debug, Clone, Default)]
struct RangeTally {
    bytes: Arc<AtomicU64>,
    requests: Arc<AtomicU64>,
}

impl RangeTally {
    fn snapshot(&self) -> (u64, u64) {
        (
            self.bytes.load(Ordering::Relaxed),
            self.requests.load(Ordering::Relaxed),
        )
    }
}

/// Wraps a `reqwest::Client` (or any `AsyncHttpRangeClient`), tallying
/// request count and bytes returned by every `get_range` call — the
/// FlatCityBuf analogue of `cityparquet::counting_store::CountingObjectStore`.
/// `head_response_header` is a metadata-only call (no body bytes) and is
/// passed through untallied — `fcb_core`'s own HTTP reader only uses it to
/// probe response headers, not to fetch data.
#[derive(Debug, Clone)]
struct CountingRangeClient<T> {
    inner: T,
    tally: RangeTally,
}

#[async_trait]
impl<T: AsyncHttpRangeClient + Send + Sync> AsyncHttpRangeClient for CountingRangeClient<T> {
    async fn get_range(&self, url: &str, range: &str) -> http_range_client::Result<Bytes> {
        let bytes = self.inner.get_range(url, range).await?;
        self.tally.requests.fetch_add(1, Ordering::Relaxed);
        self.tally
            .bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        Ok(bytes)
    }

    async fn head_response_header(
        &self,
        url: &str,
        header: &str,
    ) -> http_range_client::Result<Option<String>> {
        self.inner.head_response_header(url, header).await
    }
}

/// Opens a fresh `HttpFcbReader` against `url`, tallying every range
/// request onto `tally` — the async, HTTP-sourced mirror of [`open`].
async fn open_http(
    url: &str,
    tally: RangeTally,
) -> Result<HttpFcbReader<CountingRangeClient<reqwest::Client>>> {
    let client = CountingRangeClient {
        inner: reqwest::Client::new(),
        tally,
    };
    let buffered = AsyncBufferedHttpRangeClient::with(client, url);
    Ok(HttpFcbReader::new(buffered).await?)
}

/// The async, HTTP-sourced mirror of [`full_read`] — the same raw-accessor
/// traversal, so the two transports measure the same decode work.
async fn full_read_http(url: &str, tally: RangeTally) -> Result<u64> {
    let reader = open_http(url, tally).await?;
    let mut iter = reader.select_all().await?;
    let mut feature_count = 0u64;
    let mut leaves = 0u64;
    let mut checksum = 0u64;
    while iter.next().await?.is_some() {
        let (l, c) = feature_geometry_work(&iter.cur_feature().feature());
        feature_count += 1;
        leaves += l;
        checksum = checksum.wrapping_add(c);
    }
    std::hint::black_box((leaves, checksum));
    Ok(feature_count)
}

/// The async, HTTP-sourced mirror of [`full_walk_attr_filter`].
async fn full_walk_attr_filter_http(
    url: &str,
    tally: RangeTally,
    column: &str,
    pred: &AttrPred,
) -> Result<u64> {
    let reader = open_http(url, tally).await?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all().await?;
    let mut matched = 0u64;
    while iter.next().await?.is_some() {
        let feature = iter.cur_feature().feature();
        let Some(objects) = feature.objects() else {
            continue;
        };
        for co in objects.iter() {
            if co_matches(&co, root.as_deref(), column, pred)? {
                matched += 1;
            }
        }
    }
    Ok(matched)
}

/// The async, HTTP-sourced mirror of [`full_walk_id_lookup`].
async fn full_walk_id_lookup_http(url: &str, tally: RangeTally, id: &str) -> Result<u64> {
    let reader = open_http(url, tally).await?;
    let mut iter = reader.select_all().await?;
    while iter.next().await?.is_some() {
        let feature = iter.cur_feature().feature();
        if let Some(objects) = feature.objects()
            && objects.iter().any(|co| co.id() == id)
        {
            return Ok(1);
        }
    }
    Ok(0)
}

/// The async, HTTP-sourced mirror of [`attr_query_for`].
async fn attr_query_for_http(
    url: &str,
    tally: RangeTally,
    column: &str,
    pred: &AttrPred,
) -> Result<Option<AttrQuery>> {
    let reader = open_http(url, tally).await?;
    let column_type = {
        let header = reader.header();
        column_type_for(&header, column)
    };
    let Some(column_type) = column_type else {
        return Ok(None);
    };
    Ok(build_attr_query(column, pred, column_type).ok())
}

/// The async, HTTP-sourced mirror of [`attr_filter`].
async fn attr_filter_http(
    url: &str,
    tally: RangeTally,
    column: &str,
    pred: &AttrPred,
) -> Result<u64> {
    if let Some(query) = attr_query_for_http(url, tally.clone(), column, pred).await? {
        let reader = open_http(url, tally.clone()).await?;
        match reader.select_attr_query(&query).await {
            Ok(iter) => return Ok(iter.features_count().unwrap_or(0) as u64),
            Err(e) => eprintln!(
                "cityparquet-readbench: flatcitybuf: indexed attr-filter query on '{column}' \
                 failed ({e}) (attr-index-failed); falling back to a full scan"
            ),
        }
    } else {
        eprintln!(
            "cityparquet-readbench: flatcitybuf: attribute '{column}' has no B+-tree index \
             (no-attr-index); falling back to a full scan"
        );
    }
    full_walk_attr_filter_http(url, tally, column, pred).await
}

/// The async, HTTP-sourced mirror of [`id_lookup`].
async fn id_lookup_http(url: &str, tally: RangeTally, id: &str) -> Result<u64> {
    let pred = AttrPred::Eq(serde_json::Value::String(id.to_string()));
    if let Some(query) = attr_query_for_http(url, tally.clone(), "id", &pred).await? {
        let reader = open_http(url, tally.clone()).await?;
        if let Ok(mut iter) = reader.select_attr_query(&query).await {
            return Ok(if iter.next().await?.is_some() { 1 } else { 0 });
        }
        eprintln!(
            "cityparquet-readbench: flatcitybuf: indexed id-lookup query failed \
             (attr-index-failed); falling back to a full scan"
        );
    } else {
        eprintln!(
            "cityparquet-readbench: flatcitybuf: 'id' has no B+-tree index (no-attr-index); \
             falling back to a full scan"
        );
    }
    full_walk_id_lookup_http(url, tally, id).await
}

/// The async, HTTP-sourced mirror of [`attr_stats`].
async fn attr_stats_http(url: &str, tally: RangeTally, column: &str) -> Result<u64> {
    let reader = open_http(url, tally).await?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all().await?;
    let mut count = 0u64;
    while iter.next().await?.is_some() {
        let feature = iter.cur_feature().feature();
        let Some(objects) = feature.objects() else {
            continue;
        };
        for co in objects.iter() {
            let numeric = column != "object_type"
                && attribute_value(&co, root.as_deref(), column)?
                    .and_then(|v| v.as_f64())
                    .is_some();
            if numeric {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// The async, HTTP-sourced mirror of [`project`].
async fn project_http(url: &str, tally: RangeTally, column: &str) -> Result<u64> {
    let reader = open_http(url, tally).await?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all().await?;
    let mut count = 0u64;
    while iter.next().await?.is_some() {
        let feature = iter.cur_feature().feature();
        let Some(objects) = feature.objects() else {
            continue;
        };
        for co in objects.iter() {
            let present = if column == "object_type" {
                true
            } else {
                attribute_value(&co, root.as_deref(), column)?.is_some()
            };
            if present {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Joins `base_url`/`key` into one URL via `Url::path_segments_mut`
/// (percent-encodes each segment) — not a plain `format!("{base}/{key}")`,
/// which would send a `key` containing a character like `#`/`?`/`%` or a
/// space to the wrong resource (or fail) once actually sent over HTTP,
/// unlike the `ObjectPath`-based CityParquet/CityJSONSeq runners, which
/// handle this internally.
fn join_url(base_url: &str, key: &str) -> Result<String> {
    let mut parsed =
        url::Url::parse(base_url).with_context(|| format!("parsing --base-url '{base_url}'"))?;
    parsed
        .path_segments_mut()
        .map_err(|()| anyhow!("--base-url '{base_url}' cannot be a base for a relative key"))?
        .pop_if_empty()
        .extend(key.split('/'));
    Ok(parsed.to_string())
}

/// The HTTP-transport body of [`FlatCityBufRunner::run`]: `base_url`/`key`
/// join into one URL (`fcb_core`'s HTTP reader targets a single object, no
/// package manifest to resolve first — unlike the CityParquet runner), then
/// dispatches `scenario` onto the matching `*_http` mirror above, reporting
/// the shared [`RangeTally`] as [`IoStats`].
async fn run_http(
    base_url: &str,
    key: &str,
    scenario: Scenario,
    params: &QueryParams,
) -> Result<RunOutcome> {
    let url = join_url(base_url, key)?;
    let tally = RangeTally::default();

    let result_count = match scenario {
        Scenario::Count => {
            let reader = open_http(&url, tally.clone()).await?;
            reader.header().features_count()
        }
        Scenario::FullRead => full_read_http(&url, tally.clone()).await?,
        Scenario::BBoxQuery => {
            let bbox = *require(&params.bbox, "bbox", scenario)?;
            let reader = open_http(&url, tally.clone()).await?;
            // FCB's packed R-tree is 2D; drop the z components (indices
            // 2/5) rather than approximate them — same as the local branch.
            let query = SpatialQuery::BBox(bbox[0], bbox[1], bbox[3], bbox[4]);
            let iter = reader.select_query(query).await?;
            iter.features_count().unwrap_or(0) as u64
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            attr_filter_http(&url, tally.clone(), column, pred).await?
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            attr_stats_http(&url, tally.clone(), column).await?
        }
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            id_lookup_http(&url, tally.clone(), id).await?
        }
        Scenario::FeatureLookup => bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY),
        Scenario::Project => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            project_http(&url, tally.clone(), column).await?
        }
    };

    let (bytes, requests) = tally.snapshot();
    Ok(RunOutcome {
        result_count,
        io: Some(IoStats { bytes, requests }),
        lookup: None,
    })
}

/// The FlatCityBuf backend (see this module's own doc comment for which
/// scenarios count features vs. CityObjects).
pub struct FlatCityBufRunner;

impl FormatRunner for FlatCityBufRunner {
    fn run(&self, source: &Source, scenario: Scenario, params: &QueryParams) -> Result<RunOutcome> {
        let (base_url, key) = match source {
            Source::Local(path) => {
                let input = path.as_path();
                let result_count = match scenario {
                    Scenario::Count => {
                        let reader = open(input)?;
                        reader.header().features_count()
                    }
                    Scenario::FullRead => full_read(input)?,
                    Scenario::BBoxQuery => {
                        let bbox = *require(&params.bbox, "bbox", scenario)?;
                        let reader = open(input)?;
                        // FCB's packed R-tree is 2D; drop the z components
                        // (indices 2/5) rather than approximate them.
                        let query = SpatialQuery::BBox(bbox[0], bbox[1], bbox[3], bbox[4]);
                        let iter = reader.select_query(query, None, None)?;
                        iter.features_count().unwrap_or(0) as u64
                    }
                    Scenario::AttrFilter => {
                        let column = require(&params.attr_column, "attr-column", scenario)?;
                        let pred =
                            require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
                        attr_filter(input, column, pred)?
                    }
                    Scenario::AttrStats => {
                        let column = require(&params.attr_column, "attr-column", scenario)?;
                        attr_stats(input, column)?
                    }
                    Scenario::IdLookup => {
                        let id = require(&params.target_id, "target-id", scenario)?;
                        id_lookup(input, id)?
                    }
                    Scenario::FeatureLookup => bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY),
                    Scenario::Project => {
                        let column = require(&params.attr_column, "attr-column", scenario)?;
                        project(input, column)?
                    }
                };
                return Ok(RunOutcome {
                    result_count,
                    io: None,
                    lookup: None,
                });
            }
            Source::Http { base_url, key } => (base_url, key),
        };

        let handle = tokio::runtime::Handle::current();
        handle.block_on(run_http(base_url, key, scenario, params))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use anyhow::Result;

    use super::{
        AttrPred, attr_stats, full_read, full_walk_attr_filter, full_walk_id_lookup, join_url,
        matches_predicate, open, project,
    };

    // ---------------------------------------------------------------
    // The `cur_cj_feature` path these walks replaced, kept HERE and only
    // here: it is the oracle the raw-accessor walks are checked against,
    // not a fallback anything ships. Every helper below is a verbatim copy
    // of what the runner did before, so "the new walk agrees with the old
    // one" is a claim about this file's own history, not a restatement of
    // the new code.
    // ---------------------------------------------------------------

    /// The OLD `column_value`: `object_type` reads the decoded CityJSON
    /// type string, every other column the decoded `attributes` map, with a
    /// JSON-`null` entry treated as absent.
    fn column_value_cj(co: &cjseq2::CityObject, column: &str) -> Option<serde_json::Value> {
        if column == "object_type" {
            return Some(serde_json::Value::String(co.thetype.clone()));
        }
        co.attributes
            .as_ref()?
            .get(column)
            .filter(|v| !v.is_null())
            .cloned()
    }

    /// The OLD `full_walk_attr_filter`.
    fn cj_walk_attr_filter(input: &Path, column: &str, pred: &AttrPred) -> Result<u64> {
        let reader = open(input)?;
        let mut iter = reader.select_all()?;
        let mut matched = 0u64;
        while let Some(feat) = iter.next()? {
            let cj = feat.cur_cj_feature()?;
            matched += cj
                .city_objects
                .values()
                .filter(|co| matches_predicate(column_value_cj(co, column).as_ref(), pred))
                .count() as u64;
        }
        Ok(matched)
    }

    /// The OLD `attr_stats`.
    fn cj_attr_stats(input: &Path, column: &str) -> Result<u64> {
        let reader = open(input)?;
        let mut iter = reader.select_all()?;
        let mut count = 0u64;
        while let Some(feat) = iter.next()? {
            let cj = feat.cur_cj_feature()?;
            count += cj
                .city_objects
                .values()
                .filter(|co| {
                    column_value_cj(co, column)
                        .and_then(|v| v.as_f64())
                        .is_some()
                })
                .count() as u64;
        }
        Ok(count)
    }

    /// The OLD `project`.
    fn cj_project(input: &Path, column: &str) -> Result<u64> {
        let reader = open(input)?;
        let mut iter = reader.select_all()?;
        let mut count = 0u64;
        while let Some(feat) = iter.next()? {
            let cj = feat.cur_cj_feature()?;
            count += cj
                .city_objects
                .values()
                .filter(|co| column_value_cj(co, column).is_some())
                .count() as u64;
        }
        Ok(count)
    }

    /// The OLD `full_walk_id_lookup`.
    fn cj_id_lookup(input: &Path, id: &str) -> Result<u64> {
        let reader = open(input)?;
        let mut iter = reader.select_all()?;
        while let Some(feat) = iter.next()? {
            let cj = feat.cur_cj_feature()?;
            if cj.city_objects.contains_key(id) {
                return Ok(1);
            }
        }
        Ok(0)
    }

    /// The OLD `full_read`.
    fn cj_full_read(input: &Path) -> Result<u64> {
        let reader = open(input)?;
        let mut iter = reader.select_all()?;
        let mut feature_count = 0u64;
        while let Some(feat) = iter.next()? {
            let _ = feat.cur_cj_feature()?;
            feature_count += 1;
        }
        Ok(feature_count)
    }

    /// `delft.city.jsonl` — the one fixture `just fixtures` always
    /// fetches, and the file Caveat 1 of `benchmark/formats/READ_BENCHMARK.md`
    /// quotes its counting-grain numbers from (1115 features, 2231
    /// CityObjects, 1116 of them `BuildingPart`s).
    fn delft_fixture() -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../lib/cityparquet-rs/tests/fixtures/delft.city.jsonl");
        p.exists().then_some(p)
    }

    /// Whether the `fcb` CLI is on PATH — mirrors
    /// `tests/flatcitybuf_runner.rs`'s own guard, so these tests skip
    /// gracefully rather than fail when the optional external tool is not
    /// installed.
    fn fcb_cli_missing() -> bool {
        Command::new("fcb")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
    }

    /// Builds a `.fcb` from `src` with the same `fcb ser -A` invocation
    /// `readbench_prepare.sh` uses. `.fcb` files are never committed.
    fn generate_fcb(src: &Path, out_dir: &Path) -> PathBuf {
        let out = out_dir.join("fixture.fcb");
        let output = Command::new("fcb")
            .arg("ser")
            .arg(src)
            .arg(&out)
            .arg("-A")
            .output()
            .expect("failed to run `fcb ser` (PATH availability already checked)");
        assert!(
            output.status.success(),
            "fcb ser failed; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        out
    }

    /// A `.fcb` cut from `delft.city.jsonl`, or `None` when either the
    /// fixture or the `fcb` CLI is missing (both are fetched, neither is
    /// committed). The [`tempfile::TempDir`] is returned alongside so the
    /// caller keeps it alive for the test's duration.
    fn delft_fcb() -> Option<(tempfile::TempDir, PathBuf)> {
        let src = delft_fixture()?;
        if fcb_cli_missing() {
            eprintln!("skipping: `fcb` CLI not found on PATH");
            return None;
        }
        let tmp = tempfile::tempdir().unwrap();
        let fcb = generate_fcb(&src, tmp.path());
        Some((tmp, fcb))
    }

    #[test]
    fn raw_attr_filter_agrees_with_the_cityjson_feature_walk() {
        let Some((_tmp, input)) = delft_fcb() else {
            eprintln!("skipping: delft fixture or `fcb` CLI unavailable");
            return;
        };

        // Every column shape the blob scanner has to handle: the reserved
        // type enum, an indexed String attribute, a Double, a Bool, an
        // attribute that is JSON-`null` on many objects, and a column that
        // is not in the schema at all.
        let cases: [(&str, AttrPred); 6] = [
            (
                "object_type",
                AttrPred::Eq(serde_json::Value::String("BuildingPart".into())),
            ),
            (
                "b3_dak_type",
                AttrPred::Eq(serde_json::Value::String("horizontal".into())),
            ),
            ("b3_h_dak_50p", AttrPred::Ge(3.0)),
            (
                "b3_kas_warenhuis",
                AttrPred::Eq(serde_json::Value::String("false".into())),
            ),
            ("b3_bouwlagen", AttrPred::Ge(1.0)),
            (
                "no_such_column",
                AttrPred::Eq(serde_json::Value::String("x".into())),
            ),
        ];

        for (column, pred) in &cases {
            let raw = full_walk_attr_filter(&input, column, pred).unwrap();
            let cj = cj_walk_attr_filter(&input, column, pred).unwrap();
            assert_eq!(
                raw, cj,
                "attr-filter on '{column}' must count the same CityObjects \
                 through the raw flatbuffer accessors as through \
                 `cur_cj_feature` (raw {raw}, cur_cj_feature {cj})"
            );
        }

        // One absolute anchor, so the pair cannot agree on a wrong number:
        // delft's own 1116 BuildingParts (Caveat 1 of READ_BENCHMARK.md).
        let building_parts = full_walk_attr_filter(
            &input,
            "object_type",
            &AttrPred::Eq(serde_json::Value::String("BuildingPart".into())),
        )
        .unwrap();
        assert_eq!(
            building_parts, 1116,
            "delft carries 1116 BuildingParts across its 1115 features"
        );
    }

    #[test]
    fn raw_attr_stats_and_project_agree_with_the_cityjson_feature_walk() {
        let Some((_tmp, input)) = delft_fcb() else {
            eprintln!("skipping: delft fixture or `fcb` CLI unavailable");
            return;
        };

        for column in [
            "object_type",
            "b3_h_dak_50p",
            "b3_dak_type",
            "b3_bouwlagen",
            "no_such_column",
        ] {
            let raw = attr_stats(&input, column).unwrap();
            let cj = cj_attr_stats(&input, column).unwrap();
            assert_eq!(
                raw, cj,
                "attr-stats on '{column}' must count the same CityObjects \
                 (raw {raw}, cur_cj_feature {cj})"
            );

            let raw = project(&input, column).unwrap();
            let cj = cj_project(&input, column).unwrap();
            assert_eq!(
                raw, cj,
                "project on '{column}' must count the same CityObjects \
                 (raw {raw}, cur_cj_feature {cj})"
            );
        }

        // The reserved column is present on every CityObject, so project
        // counts delft's own 2231 of them (Caveat 1 of READ_BENCHMARK.md).
        assert_eq!(
            project(&input, "object_type").unwrap(),
            2231,
            "delft carries 2231 CityObjects across its 1115 features"
        );
    }

    #[test]
    fn raw_id_lookup_and_full_read_agree_with_the_cityjson_feature_walk() {
        let Some((_tmp, input)) = delft_fcb() else {
            eprintln!("skipping: delft fixture or `fcb` CLI unavailable");
            return;
        };

        // A parent Building id, one of its BuildingPart children, and a
        // miss (which drains the whole file).
        for id in [
            "NL.IMBAG.Pand.0503100000012869",
            "NL.IMBAG.Pand.0503100000012869-0",
            "NL.IMBAG.Pand.0503100000012869-absent",
        ] {
            let raw = full_walk_id_lookup(&input, id).unwrap();
            let cj = cj_id_lookup(&input, id).unwrap();
            assert_eq!(
                raw, cj,
                "id-lookup for '{id}' must agree (raw {raw}, cur_cj_feature {cj})"
            );
        }

        let raw = full_read(&input).unwrap();
        let cj = cj_full_read(&input).unwrap();
        assert_eq!(
            raw, cj,
            "full-read must count the same features (raw {raw}, cur_cj_feature {cj})"
        );
        assert_eq!(raw, 1115, "delft's FCB carries 1115 features");
    }

    #[test]
    fn join_url_appends_a_plain_key_under_the_base() {
        assert_eq!(
            join_url("http://127.0.0.1:8080", "delft.fcb").unwrap(),
            "http://127.0.0.1:8080/delft.fcb"
        );
        // A base URL already ending in `/` must not produce a doubled slash.
        assert_eq!(
            join_url("http://127.0.0.1:8080/", "delft.fcb").unwrap(),
            "http://127.0.0.1:8080/delft.fcb"
        );
    }

    #[test]
    fn join_url_percent_encodes_special_characters_in_the_key() {
        let url = join_url("http://127.0.0.1:8080", "a file#1?.fcb").unwrap();
        // `#`/`?`/space are all percent-encoded, not left to be
        // misinterpreted as a URL fragment/query/separator.
        assert!(!url.contains(' '), "space must be percent-encoded: {url}");
        assert!(
            url.ends_with("a%20file%231%3F.fcb"),
            "expected percent-encoded key, got: {url}"
        );
    }

    #[test]
    fn join_url_rejects_a_base_that_cannot_be_a_base() {
        assert!(join_url("not a url", "delft.fcb").is_err());
    }
}
