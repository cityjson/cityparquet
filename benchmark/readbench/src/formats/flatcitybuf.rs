//! The FlatCityBuf (FCB) [`FormatRunner`]: maps each [`Scenario`] onto
//! `fcb_core` 0.7's own native mechanisms — the packed R-tree spatial index,
//! the B+-tree attribute index (built by `fcb ser -A`, the read benchmark's
//! prep step), and a full [`fcb_core::FcbReader::select_all`] walk where FCB
//! has no better mechanism.
//!
//! **Storage and index granularity.** FCB stores one `CityFeature` per
//! top-level CityObject, bundling it with all of its children (the
//! CityJSONSeq convention; `lod3_railway.city.json`, 121 CityObjects,
//! becomes 38 features). Its two indexes are coarser than the CityObject:
//!
//! - the R-tree has one 2D entry per FEATURE (the feature's whole extent);
//!   the query window's z-components are dropped by the index;
//! - the B+-tree holds one entry per matching CityObject's own attribute
//!   value, each carrying its enclosing FEATURE's offset, sorted but never
//!   deduplicated (`select_attr_query`), so a feature with two matching
//!   objects is named twice. It indexes only the CityJSON `attributes` map:
//!   the reserved `object_type` and a CityObject's id are never in its
//!   schema, so a query on either takes a full walk (checked per call from
//!   the header's schema, and disclosed with a [`FALLBACK_MARKERS`] tag).
//!
//! **What each scenario reads, visits and returns.** Every walk reads the
//! raw FlatBuffers `CityFeature`, never `cur_cj_feature` (whose
//! `to_cj_feature` conversion into `serde_json` would measure the
//! conversion, not the format). Coordinates are dequantised from FCB's
//! `i32` vertices with the header transform and visited in place.
//!
//! - [`Scenario::Count`]: the header's `features_count`; a feature count.
//! - [`Scenario::FullRead`]: every feature, every field of every
//!   CityObject — every boundary vertex dequantised, the five flattened
//!   index arrays, every per-face semantic reference and every
//!   semantic-surface object, every attribute value (the packed blob read
//!   byte by byte), template instances by anchor, transformation and
//!   template index only. Appearance (`material`, `texture`, the feature
//!   `appearance`) is not read. `result_count` is the feature count; the
//!   comparable totals are object-level.
//! - [`Scenario::BBoxQuery`]: the R-tree selects the features whose 2D
//!   extent meets the window; each is read once (deduplicated by feature
//!   id) and every CityObject in it is tested with the one shared
//!   definition (its box is the min/max over every vertex of its `children`
//!   subtree; edges intersect). Walking the non-matching objects of a hit
//!   feature is FCB's honest cost. Returns the matching objects' ids and
//!   walks each one's highest-LoD geometry in place.
//! - [`Scenario::AttrFilter`]: the B+-tree selects the features holding a
//!   match; each is read once WITHOUT decoding geometry, and every
//!   CityObject in it is re-tested (one targeted blob scan, or the
//!   flatbuffer type enum for `object_type`) so only the matching objects'
//!   ids return. Without an index, a full walk tests every object.
//! - [`Scenario::AttrStats`]: always a full walk decoding that one column
//!   (FCB's B+-tree filters, it does not aggregate), folded per CityObject
//!   into `(min, max, sum, count)`.
//! - [`Scenario::IdLookup`]: the `id` B+-tree when present (with `-A` it
//!   is not, so a full walk), stopping at the feature holding the object;
//!   every field of that object is read as in read all.

use std::collections::{HashMap, HashSet};
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
    AttrQuery, CityFeature, CityObject, CityObjectType, ColumnType, FcbReader, FixedStringKey,
    Float, Geometry, GeometryInstance, Header, HttpFcbReader, KeyType, Operator, SpatialQuery,
};
use http_range_client::{AsyncBufferedHttpRangeClient, AsyncHttpRangeClient};

use super::cjvisit::MaybeBox;
use super::returned::{ComparableTotals, EMPTY_EXTENT, IdDigest, ReturnedGeometry};
use super::{Answer, AttrAggregates, FormatRunner, IoStats, RunOutcome, Source};
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
/// result is `black_box`ed by [`visit_object`] so the traversal cannot be
/// optimised away as dead code.
fn touch_indices(values: impl Iterator<Item = u32>) -> u64 {
    values.fold(0u64, |acc, n| acc.wrapping_add(u64::from(n)))
}

/// One geometry's full boundary traversal: `(leaves, checksum)`, where
/// `leaves` is the number of vertex indices in its `boundaries` array (the
/// leaves of the nested CityJSON form) and `checksum` folds every element of
/// EVERY one of FCB's five flattened arrays (`solids`, `shells`,
/// `surfaces`, `strings`, `boundaries`) plus the per-surface `semantics`
/// index array. Between them those arrays ARE the nesting structure a
/// CityJSON `boundaries` tree encodes, so touching all of them is the same
/// read work `fcb_core`'s own `decode` does, minus the nested `Vec`
/// allocation it builds on top.
///
/// The vertices themselves, `semantics_objects` and attributes are read by
/// [`visit_object`]; `material` and `texture` (appearance) are not read.
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

/// The header's vertex quantisation: a stored `i32` vertex is
/// `v * scale + translate` in real coordinates. A file without a transform
/// stores real coordinates directly.
#[derive(Clone, Copy, Debug)]
struct Quant {
    scale: [f64; 3],
    translate: [f64; 3],
}

impl Quant {
    fn of(header: &Header<'_>) -> Self {
        match header.transform() {
            Some(t) => {
                let (s, tr) = (t.scale(), t.translate());
                Self {
                    scale: [s.x(), s.y(), s.z()],
                    translate: [tr.x(), tr.y(), tr.z()],
                }
            }
            None => Self {
                scale: [1.0; 3],
                translate: [0.0; 3],
            },
        }
    }
}

/// Resolves every vertex index in `indices` against `feature`'s own vertex
/// list, dequantises it to real `f64` coordinates in place and folds it
/// into `extent`. Returns the number of coordinates visited.
fn walk(
    feature: &CityFeature<'_>,
    q: Quant,
    indices: impl Iterator<Item = u32>,
    extent: &mut [f64; 6],
) -> Result<u64> {
    let vertices = feature.vertices();
    let mut visited = 0u64;
    for index in indices {
        let i = index as usize;
        let Some(vertices) = vertices.as_ref().filter(|v| i < v.len()) else {
            bail!(
                "feature {} references vertex {i}, beyond its {} vertices",
                feature.id(),
                vertices.as_ref().map_or(0, |v| v.len())
            );
        };
        let v = vertices.get(i);
        let p = [
            f64::from(v.x()) * q.scale[0] + q.translate[0],
            f64::from(v.y()) * q.scale[1] + q.translate[1],
            f64::from(v.z()) * q.scale[2] + q.translate[2],
        ];
        for axis in 0..3 {
            extent[axis] = extent[axis].min(p[axis]);
            extent[axis + 3] = extent[axis + 3].max(p[axis]);
        }
        visited += 1;
    }
    Ok(visited)
}

/// Every byte of an attribute blob, folded: FlatCityBuf's attribute values
/// in their native (binary) form, each one read.
fn touch_bytes(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |acc, &b| acc.wrapping_add(u64::from(b)))
}

/// Reads every field of one CityObject natively and folds it into
/// `totals`, counted as `cityparquet_core::visit::VisitTotals` counts:
/// one object; one geometry per standard geometry (a template instance is
/// read by its anchor, transformation and template index, never expanded,
/// and not counted); every boundary vertex dequantised into the extent;
/// every non-null per-face semantic reference; every semantic-surface
/// object (type, attributes, children, parent); every attribute value.
/// Appearance (`material`, `texture`) is not read.
fn visit_object(
    feature: &CityFeature<'_>,
    co: &CityObject<'_>,
    q: Quant,
    totals: &mut ComparableTotals,
) -> Result<()> {
    totals.objects += 1;
    let mut work = 0u64;
    std::hint::black_box((co.id(), co.type_(), co.geographical_extent()));
    if let Some(attributes) = co.attributes() {
        work = work.wrapping_add(touch_bytes(attributes.bytes()));
    }
    if let Some(columns) = co.columns() {
        for column in columns.iter() {
            std::hint::black_box(column.name());
        }
    }
    if let Some(children) = co.children() {
        for child in children.iter() {
            std::hint::black_box(child);
        }
    }
    if let Some(parents) = co.parents() {
        for parent in parents.iter() {
            std::hint::black_box(parent);
        }
    }
    if let Some(geometries) = co.geometry() {
        for geom in geometries.iter() {
            totals.geometries += 1;
            std::hint::black_box((geom.type_(), geom.lod()));
            work = work.wrapping_add(geometry_work(&geom).1);
            if let Some(boundaries) = geom.boundaries() {
                walk(feature, q, boundaries.iter(), &mut totals.extent)?;
            }
            if let Some(semantics) = geom.semantics() {
                totals.semantic_faces += semantics.iter().filter(|&s| s != u32::MAX).count() as u64;
            }
            if let Some(objects) = geom.semantics_objects() {
                for surface in objects.iter() {
                    std::hint::black_box((surface.type_(), surface.parent()));
                    if let Some(attributes) = surface.attributes() {
                        work = work.wrapping_add(touch_bytes(attributes.bytes()));
                    }
                    if let Some(children) = surface.children() {
                        work = work.wrapping_add(touch_indices(children.iter()));
                    }
                }
            }
        }
    }
    if let Some(instances) = co.geometry_instances() {
        for instance in instances.iter() {
            work = work.wrapping_add(geometry_instance_work(&instance).1);
            std::hint::black_box((instance.template(), instance.transformation()));
        }
    }
    std::hint::black_box(work);
    Ok(())
}

/// `co`'s most detailed standard geometry, LoDs ordered numerically as
/// `(major, minor)` exactly as the text formats order them.
fn highest_lod<'a>(co: &CityObject<'a>) -> Option<Geometry<'a>> {
    co.geometry()?
        .iter()
        .max_by_key(|g| super::cjvisit::lod_rank(g.lod()))
}

/// The box over `co`'s own geometries (template-instance anchors
/// included), as `cjvisit` boxes a CityJSON object.
fn own_box(feature: &CityFeature<'_>, co: &CityObject<'_>, q: Quant) -> Result<MaybeBox> {
    let mut e = EMPTY_EXTENT;
    if let Some(geometries) = co.geometry() {
        for geom in geometries.iter() {
            if let Some(boundaries) = geom.boundaries() {
                walk(feature, q, boundaries.iter(), &mut e)?;
            }
        }
    }
    if let Some(instances) = co.geometry_instances() {
        for instance in instances.iter() {
            if let Some(boundaries) = instance.boundaries() {
                walk(feature, q, boundaries.iter(), &mut e)?;
            }
        }
    }
    Ok(e[0]
        .is_finite()
        .then_some(([e[0], e[1], e[2]], [e[3], e[4], e[5]])))
}

/// The box over object `i`'s whole `children` subtree inside one feature,
/// memoised; a cycle is cut where it closes.
fn subtree_box(
    i: usize,
    own: &[MaybeBox],
    children: &[Vec<usize>],
    memo: &mut [Option<MaybeBox>],
    on_path: &mut [bool],
) -> MaybeBox {
    if let Some(done) = memo[i] {
        return done;
    }
    on_path[i] = true;
    let mut acc = own[i];
    for &child in &children[i] {
        if !on_path[child] {
            acc = super::cjvisit::union(acc, subtree_box(child, own, children, memo, on_path));
        }
    }
    on_path[i] = false;
    memo[i] = Some(acc);
    acc
}

/// What one scenario does with each feature it reads.
enum Step<'p> {
    /// Read all: every object, every field.
    All,
    /// The spatial window: ids and highest-LoD geometry of the matching
    /// CityObjects.
    Window(&'p [f64; 6]),
    /// The attribute filter: ids of the matching CityObjects.
    Filter(&'p str, &'p AttrPred),
    /// The identifier lookup: every field of the one object found.
    Lookup(&'p str),
}

/// One scenario's fold over the features a selection yields.
struct Fold<'p> {
    step: Step<'p>,
    q: Quant,
    root: Option<Vec<ColumnMeta>>,
    /// Feature ids already read: an index can name a feature more than once
    /// (one entry per matching CityObject), and each is read once.
    seen: Option<HashSet<String>>,
    features: u64,
    ids: IdDigest,
    totals: ComparableTotals,
    geometry: ReturnedGeometry,
    done: bool,
}

impl<'p> Fold<'p> {
    fn new(step: Step<'p>, header: &Header<'_>, dedupe: bool) -> Self {
        Self {
            step,
            q: Quant::of(header),
            root: owned_columns(header),
            seen: dedupe.then(HashSet::new),
            features: 0,
            ids: IdDigest::default(),
            totals: ComparableTotals::default(),
            geometry: ReturnedGeometry::default(),
            done: false,
        }
    }

    fn feature(&mut self, feature: &CityFeature<'_>) -> Result<()> {
        if let Some(seen) = &mut self.seen
            && !seen.insert(feature.id().to_owned())
        {
            return Ok(());
        }
        self.features += 1;
        let q = self.q;
        let Some(objects) = feature.objects() else {
            return Ok(());
        };
        match self.step {
            Step::All => {
                for co in objects.iter() {
                    visit_object(feature, &co, q, &mut self.totals)?;
                }
            }
            Step::Window(window) => {
                let n = objects.len();
                let mut index = HashMap::with_capacity(n);
                let mut own = Vec::with_capacity(n);
                for (i, co) in objects.iter().enumerate() {
                    index.insert(co.id(), i);
                    own.push(own_box(feature, &co, q)?);
                }
                let children: Vec<Vec<usize>> = objects
                    .iter()
                    .map(|co| {
                        co.children()
                            .map(|c| c.iter().filter_map(|id| index.get(id).copied()).collect())
                            .unwrap_or_default()
                    })
                    .collect();
                let mut memo = vec![None; n];
                let mut on_path = vec![false; n];
                for (i, co) in objects.iter().enumerate() {
                    if let Some((min, max)) =
                        subtree_box(i, &own, &children, &mut memo, &mut on_path)
                        && super::cityjsonseq::intersects(min, max, window)
                    {
                        self.ids.push(co.id());
                        if let Some(geom) = highest_lod(&co) {
                            self.geometry.geometries += 1;
                            if let Some(boundaries) = geom.boundaries() {
                                std::hint::black_box(walk(
                                    feature,
                                    q,
                                    boundaries.iter(),
                                    &mut self.geometry.extent,
                                )?);
                            }
                        }
                    }
                }
            }
            Step::Filter(column, pred) => {
                for co in objects.iter() {
                    if co_matches(&co, self.root.as_deref(), column, pred)? {
                        self.ids.push(co.id());
                    }
                }
            }
            Step::Lookup(id) => {
                if let Some(co) = objects.iter().find(|co| co.id() == id) {
                    visit_object(feature, &co, q, &mut self.totals)?;
                    self.done = true;
                }
            }
        }
        Ok(())
    }

    fn answer(self) -> Answer {
        match self.step {
            Step::All => Answer::reading(self.features, self.totals),
            Step::Window(_) => Answer::returning(self.ids, Some(self.geometry)),
            Step::Filter(..) => Answer::returning(self.ids, None),
            Step::Lookup(_) => Answer::reading(self.totals.objects, self.totals),
        }
    }
}

/// Which FlatCityBuf mechanism selects the features a fold reads.
enum Select {
    /// `select_all`: every feature, in file order.
    All,
    /// The packed 2D R-tree (`SpatialQuery::BBox`), one entry per feature.
    BBox([f64; 4]),
    /// A B+-tree attribute index; on failure, a full `select_all` walk with
    /// the `attr-index-failed` disclosure.
    Attr(AttrQuery, &'static str),
}

/// Runs `step` over the features `select` yields from a local file.
fn fold_local(input: &Path, select: Select, step: Step<'_>) -> Result<Answer> {
    let reader = open(input)?;
    let features = reader.header().features_count();
    let mut fold = Fold::new(step, &reader.header(), !matches!(select, Select::All));
    let mut iter = match select {
        Select::All => reader.select_all()?,
        Select::BBox(b) => {
            reader.select_query(SpatialQuery::BBox(b[0], b[1], b[2], b[3]), None, None)?
        }
        Select::Attr(query, label) => match reader.select_attr_query(query) {
            Ok(iter) if !truncates(iter.features_count(), features) => iter,
            Ok(_) => {
                index_failed(label, &TRUNCATED);
                fold.seen = None;
                open(input)?.select_all()?
            }
            Err(e) => {
                index_failed(label, &e);
                fold.seen = None;
                open(input)?.select_all()?
            }
        },
    };
    while let Some(feat) = iter.next()? {
        fold.feature(&feat.cur_feature())?;
        if fold.done {
            break;
        }
    }
    Ok(fold.answer())
}

/// Runs `step` over the features `select` yields over HTTP range requests.
async fn fold_http(url: &str, tally: RangeTally, select: Select, step: Step<'_>) -> Result<Answer> {
    let reader = open_http(url, tally.clone()).await?;
    let features = reader.header().features_count();
    let mut fold = Fold::new(step, &reader.header(), !matches!(select, Select::All));
    let mut iter = match select {
        Select::All => reader.select_all().await?,
        Select::BBox(b) => {
            reader
                .select_query(SpatialQuery::BBox(b[0], b[1], b[2], b[3]))
                .await?
        }
        Select::Attr(query, label) => match reader.select_attr_query(&query).await {
            Ok(iter) if !truncates(iter.features_count(), features) => iter,
            Ok(_) => {
                index_failed(label, &TRUNCATED);
                fold.seen = None;
                open_http(url, tally).await?.select_all().await?
            }
            Err(e) => {
                index_failed(label, &e);
                fold.seen = None;
                open_http(url, tally).await?.select_all().await?
            }
        },
    };
    while iter.next().await?.is_some() {
        fold.feature(&iter.cur_feature().feature())?;
        if fold.done {
            break;
        }
    }
    Ok(fold.answer())
}

/// The disclosure for an attribute-index query that failed and fell back
/// to a full walk (see [`FALLBACK_MARKERS`]).
fn index_failed(label: &str, e: &dyn std::fmt::Display) {
    eprintln!(
        "cityparquet-readbench: flatcitybuf: indexed {label} query failed ({e}) \
         (attr-index-failed); falling back to a full scan"
    );
}

/// Why a B+-tree hit list cannot be iterated: it holds one entry per
/// matching CityObject, never deduplicated by feature offset, and
/// `fcb_core` 0.7.6's feature iterator stops after the header's
/// `features_count` entries. A list longer than that would silently lose
/// its tail (on `lod3_railway.city.json`, `function = "1070"` names 65
/// objects in 8 features; the iterator yields 38 entries covering 6), so
/// such a query is answered by the full walk and disclosed as a fallback.
const TRUNCATED: &str = "the hit list is longer than the file's feature count, \
                         which fcb_core 0.7.6's iterator truncates";

/// Whether an attribute-index hit list of `hits` entries would be cut short
/// by the iterator over a file of `features` features (see [`TRUNCATED`]).
fn truncates(hits: Option<usize>, features: u64) -> bool {
    hits.is_some_and(|n| n as u64 > features)
}

/// The disclosure for an attribute without a B+-tree index.
fn no_index(column: &str) {
    eprintln!(
        "cityparquet-readbench: flatcitybuf: attribute '{column}' has no B+-tree index \
         (no-attr-index); falling back to a full scan"
    );
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

/// [`Scenario::AttrFilter`]: the B+-tree attribute index selects the
/// features holding a match, each read once WITHOUT decoding geometry, and
/// every CityObject of each is re-tested so only the matching objects' ids
/// return. Without an index, a full `select_all` walk tests every object.
fn attr_filter(input: &Path, column: &str, pred: &AttrPred) -> Result<Answer> {
    let step = Step::Filter(column, pred);
    match attr_query_for(input, column, pred)? {
        Some(query) => fold_local(input, Select::Attr(query, "attr-filter"), step),
        None => {
            no_index(column);
            fold_local(input, Select::All, step)
        }
    }
}

/// [`Scenario::IdLookup`]: the `id` B+-tree selects the feature, which is
/// read until the object is found; every field of that object is read.
/// Without an index, a full `select_all` walk stops at the hit.
fn id_lookup(input: &Path, id: &str) -> Result<Answer> {
    let pred = AttrPred::Eq(serde_json::Value::String(id.to_string()));
    match attr_query_for(input, "id", &pred)? {
        Some(query) => fold_local(input, Select::Attr(query, "id-lookup"), Step::Lookup(id)),
        None => {
            no_index("id");
            fold_local(input, Select::All, Step::Lookup(id))
        }
    }
}

/// [`Scenario::AttrStats`]: always a full `select_all` walk (FCB's B+-tree
/// has no columnar aggregation mechanism — see this module's own doc
/// comment), aggregating `(min, max, sum, count)` over every CityObject
/// (across every feature) carrying a numeric value for `column` —
/// CityObject level, matching [`attr_filter`]'s own granularity.
///
/// Attributes only: no geometry is decoded, and only `column`'s own value
/// is read out of each object's attribute blob.
fn attr_stats(input: &Path, column: &str) -> Result<AttrAggregates> {
    let reader = open(input)?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all()?;
    let mut stats = AttrAggregates::EMPTY;
    while let Some(feat) = iter.next()? {
        let Some(objects) = feat.cur_feature().objects() else {
            continue;
        };
        for co in objects.iter() {
            push_numeric(&mut stats, &co, root.as_deref(), column)?;
        }
    }
    // Pinned so the aggregation cannot be reduced to the count.
    Ok(std::hint::black_box(stats))
}

/// Folds `co`'s value for `column` into `stats` when it is numeric, with the
/// same `as_f64` rule as [`super::cityjsonseq`]'s own `push_numeric`. The
/// reserved `object_type` column shadows any same-named attribute and is a
/// type STRING, never numeric — so it contributes nothing.
fn push_numeric(
    stats: &mut AttrAggregates,
    co: &CityObject<'_>,
    root: Option<&[ColumnMeta]>,
    column: &str,
) -> Result<()> {
    if column == "object_type" {
        return Ok(());
    }
    if let Some(value) = attribute_value(co, root, column)?.and_then(|v| v.as_f64()) {
        stats.push(value);
    }
    Ok(())
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
        inner: cityparquet_readbench::http_client::reqwest_client(),
        tally,
    };
    let buffered = AsyncBufferedHttpRangeClient::with(client, url);
    Ok(HttpFcbReader::new(buffered).await?)
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
) -> Result<Answer> {
    let step = Step::Filter(column, pred);
    match attr_query_for_http(url, tally.clone(), column, pred).await? {
        Some(query) => fold_http(url, tally, Select::Attr(query, "attr-filter"), step).await,
        None => {
            no_index(column);
            fold_http(url, tally, Select::All, step).await
        }
    }
}

/// The async, HTTP-sourced mirror of [`id_lookup`].
async fn id_lookup_http(url: &str, tally: RangeTally, id: &str) -> Result<Answer> {
    let pred = AttrPred::Eq(serde_json::Value::String(id.to_string()));
    match attr_query_for_http(url, tally.clone(), "id", &pred).await? {
        Some(query) => {
            fold_http(
                url,
                tally,
                Select::Attr(query, "id-lookup"),
                Step::Lookup(id),
            )
            .await
        }
        None => {
            no_index("id");
            fold_http(url, tally, Select::All, Step::Lookup(id)).await
        }
    }
}

/// The async, HTTP-sourced mirror of [`attr_stats`].
async fn attr_stats_http(url: &str, tally: RangeTally, column: &str) -> Result<AttrAggregates> {
    let reader = open_http(url, tally).await?;
    let root = owned_columns(&reader.header());
    let mut iter = reader.select_all().await?;
    let mut stats = AttrAggregates::EMPTY;
    while iter.next().await?.is_some() {
        let feature = iter.cur_feature().feature();
        let Some(objects) = feature.objects() else {
            continue;
        };
        for co in objects.iter() {
            push_numeric(&mut stats, &co, root.as_deref(), column)?;
        }
    }
    Ok(std::hint::black_box(stats))
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

    let answer: Answer = match scenario {
        Scenario::Count => {
            let reader = open_http(&url, tally.clone()).await?;
            reader.header().features_count().into()
        }
        Scenario::FullRead => fold_http(&url, tally.clone(), Select::All, Step::All).await?,
        Scenario::BBoxQuery => {
            let bbox = *require(&params.bbox, "bbox", scenario)?;
            let select = Select::BBox([bbox[0], bbox[1], bbox[3], bbox[4]]);
            fold_http(&url, tally.clone(), select, Step::Window(&bbox)).await?
        }
        Scenario::AttrFilter => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
            attr_filter_http(&url, tally.clone(), column, pred).await?
        }
        Scenario::AttrStats => {
            let column = require(&params.attr_column, "attr-column", scenario)?;
            attr_stats_http(&url, tally.clone(), column).await?.into()
        }
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            id_lookup_http(&url, tally.clone(), id).await?
        }
        Scenario::FeatureLookup | Scenario::AttrLookup => {
            bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY)
        }
    };

    let (bytes, requests) = tally.snapshot();
    Ok(outcome(answer, Some(IoStats { bytes, requests })))
}

/// One scenario's [`Answer`] as the [`RunOutcome`] the child reports.
fn outcome(answer: Answer, io: Option<IoStats>) -> RunOutcome {
    RunOutcome {
        result_count: answer.result_count,
        io,
        lookup: None,
        attr_stats: answer.attr_stats,
        returned: answer.returned,
    }
}

pub struct FlatCityBufRunner;

impl FormatRunner for FlatCityBufRunner {
    fn run(&self, source: &Source, scenario: Scenario, params: &QueryParams) -> Result<RunOutcome> {
        let (base_url, key) = match source {
            Source::Local(path) => {
                let input = path.as_path();
                let answer: Answer = match scenario {
                    Scenario::Count => open(input)?.header().features_count().into(),
                    Scenario::FullRead => fold_local(input, Select::All, Step::All)?,
                    Scenario::BBoxQuery => {
                        let bbox = *require(&params.bbox, "bbox", scenario)?;
                        let select = Select::BBox([bbox[0], bbox[1], bbox[3], bbox[4]]);
                        fold_local(input, select, Step::Window(&bbox))?
                    }
                    Scenario::AttrFilter => {
                        let column = require(&params.attr_column, "attr-column", scenario)?;
                        let pred =
                            require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
                        attr_filter(input, column, pred)?
                    }
                    Scenario::AttrStats => {
                        let column = require(&params.attr_column, "attr-column", scenario)?;
                        attr_stats(input, column)?.into()
                    }
                    Scenario::IdLookup => {
                        let id = require(&params.target_id, "target-id", scenario)?;
                        id_lookup(input, id)?
                    }
                    Scenario::FeatureLookup | Scenario::AttrLookup => {
                        bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY)
                    }
                };
                return Ok(outcome(answer, None));
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
        AttrPred, Select, Step, attr_stats, fold_local, join_url, matches_predicate, open,
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

    /// The attribute filter through `cur_cj_feature`, the oracle for the raw walk.
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

    /// The identifier lookup through `cur_cj_feature`, the oracle for the raw walk.
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

    /// Read all through `cur_cj_feature`, the oracle for the raw walk.
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
            let raw = fold_local(&input, Select::All, Step::Filter(column, pred))
                .unwrap()
                .result_count;
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
        let building_part = AttrPred::Eq(serde_json::Value::String("BuildingPart".into()));
        let building_parts = fold_local(
            &input,
            Select::All,
            Step::Filter("object_type", &building_part),
        )
        .unwrap()
        .result_count;
        assert_eq!(
            building_parts, 1116,
            "delft carries 1116 BuildingParts across its 1115 features"
        );
    }

    #[test]
    fn raw_attr_stats_agrees_with_the_cityjson_feature_walk() {
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
            let raw = attr_stats(&input, column).unwrap().count;
            let cj = cj_attr_stats(&input, column).unwrap();
            assert_eq!(
                raw, cj,
                "attr-stats on '{column}' must count the same CityObjects \
                 (raw {raw}, cur_cj_feature {cj})"
            );
        }
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
            let raw = fold_local(&input, Select::All, Step::Lookup(id))
                .unwrap()
                .result_count;
            let cj = cj_id_lookup(&input, id).unwrap();
            assert_eq!(
                raw, cj,
                "id-lookup for '{id}' must agree (raw {raw}, cur_cj_feature {cj})"
            );
        }

        let raw = fold_local(&input, Select::All, Step::All)
            .unwrap()
            .result_count;
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
