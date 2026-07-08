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

use anyhow::{Context, Result, anyhow, bail};
// fcb_core 0.7's own `CityJSONFeature`/`CityObject` come from the `cjseq2`
// crate (aliased internally to `cjseq` inside `fcb_core`'s own source), NOT
// the `cjseq` 0.4 crate `super::cityjsonseq` uses — see this crate's own
// `Cargo.toml` comment on the `cjseq2` dependency.
use cjseq2::CityObject;
use fcb_core::{
    AttrQuery, ColumnType, FcbReader, FixedStringKey, Float, Header, KeyType, Operator,
    SpatialQuery,
};

use super::FormatRunner;
use crate::scenario::{AttrPred, QueryParams, Scenario};

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

/// `column`'s value on `co`: the reserved `object_type` column reads
/// `co.thetype` (never part of FCB's attribute schema); every other column
/// name is looked up in `co.attributes`, with a JSON-`null` entry treated
/// the same as an absent one. This is the CityObject-level analogue of
/// [`super::cityjsonseq`]'s own `column_value` — kept as its own copy per
/// this module's own doc convention, since the two runners walk different
/// underlying feature types.
fn column_value(co: &CityObject, column: &str) -> Option<serde_json::Value> {
    if column == "object_type" {
        return Some(serde_json::Value::String(co.thetype.clone()));
    }
    co.attributes
        .as_ref()?
        .get(column)
        .filter(|v| !v.is_null())
        .cloned()
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
                // Same numeric-looking-string-code exception as `eq_key`'s
                // own doc comment: the shared `--attr-eq` CLI only coerces
                // the *predicate* value eagerly, never the attribute's own
                // stored value, so a String-typed attribute value (e.g.
                // `"1070"`) is compared against `want`'s string form here
                // directly; the symmetric case (an actual JSON number
                // stored value against a numeric-looking `want` string) is
                // handled by the `as_f64` branch below.
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
/// One deliberate exception: many real CityJSON attributes are
/// String-typed numeric *codes* (e.g. this benchmark's own
/// `lod3_railway.city.json` fixture has `"function": "1070"` — a string,
/// not a number), yet this crate's shared `--attr-eq` CLI flag
/// (`main.rs::build_attr_pred`) eagerly parses any numeric-looking value
/// into a JSON number, losing the fact that it was meant to match a string
/// column. Rather than depend on that shared parsing (used by every format
/// runner) never doing this, a `ColumnType::String` column paired with a
/// numeric `value` re-stringifies it back to its canonical form (integer
/// formatting when the value has no fractional part) before building the
/// `StringKey50` — recovering the original string code's exact bytes for
/// any code that round-trips through `f64` (true for every string-encoded
/// integer code these fixtures use).
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

/// Every feature in `input`, counting one FEATURE per successfully decoded
/// `cur_cj_feature`; `touch` (the total number of CityObjects across every
/// feature) forces the CityJSON-feature deserialisation to actually
/// complete rather than being optimised away, but is otherwise discarded —
/// the returned metric stays feature-level per this module's own doc
/// comment.
fn full_read(input: &Path) -> Result<u64> {
    let reader = open(input)?;
    let mut iter = reader.select_all()?;
    let mut feature_count = 0u64;
    let mut touch = 0u64;
    while let Some(feat) = iter.next()? {
        let cj = feat.cur_cj_feature()?;
        feature_count += 1;
        touch += cj.city_objects.len() as u64;
    }
    let _ = touch;
    Ok(feature_count)
}

/// A full `select_all` walk, counting every CityObject (across every
/// feature, parents AND children) matching `pred` on `column` — CityObject
/// level, matching [`attr_filter`]'s own indexed-path granularity (see this
/// module's own doc comment) and [`super::cityjsonseq`]'s convention for
/// this same scenario. Used as [`Scenario::AttrFilter`]'s fallback when
/// `column` isn't indexed.
fn full_walk_attr_filter(input: &Path, column: &str, pred: &AttrPred) -> Result<u64> {
    let reader = open(input)?;
    let mut iter = reader.select_all()?;
    let mut matched = 0u64;
    while let Some(feat) = iter.next()? {
        let cj = feat.cur_cj_feature()?;
        matched += cj
            .city_objects
            .values()
            .filter(|co| matches_predicate(column_value(co, column).as_ref(), pred))
            .count() as u64;
    }
    Ok(matched)
}

/// A full `select_all` walk, short-circuiting as soon as `id` is found
/// among any feature's `city_objects` keys.
fn full_walk_id_lookup(input: &Path, id: &str) -> Result<u64> {
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
                 failed ({e}); falling back to a full scan"
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
    }
    full_walk_id_lookup(input, id)
}

/// [`Scenario::AttrStats`]: always a full `select_all` walk (FCB's B+-tree
/// has no columnar aggregation mechanism — see this module's own doc
/// comment), counting every CityObject (across every feature) carrying a
/// numeric value for `column` — CityObject level, matching [`attr_filter`]'s
/// own granularity.
fn attr_stats(input: &Path, column: &str) -> Result<u64> {
    let reader = open(input)?;
    let mut iter = reader.select_all()?;
    let mut count = 0u64;
    while let Some(feat) = iter.next()? {
        let cj = feat.cur_cj_feature()?;
        count += cj
            .city_objects
            .values()
            .filter(|co| column_value(co, column).and_then(|v| v.as_f64()).is_some())
            .count() as u64;
    }
    Ok(count)
}

/// [`Scenario::Project`]: always a full `select_all` walk (same rationale
/// as [`attr_stats`]), counting every CityObject (across every feature)
/// carrying a non-null value for `column` — CityObject level.
fn project(input: &Path, column: &str) -> Result<u64> {
    let reader = open(input)?;
    let mut iter = reader.select_all()?;
    let mut count = 0u64;
    while let Some(feat) = iter.next()? {
        let cj = feat.cur_cj_feature()?;
        count += cj
            .city_objects
            .values()
            .filter(|co| column_value(co, column).is_some())
            .count() as u64;
    }
    Ok(count)
}

/// The FlatCityBuf backend (see this module's own doc comment for which
/// scenarios count features vs. CityObjects).
pub struct FlatCityBufRunner;

impl FormatRunner for FlatCityBufRunner {
    fn run(&self, input: &Path, scenario: Scenario, params: &QueryParams) -> Result<u64> {
        match scenario {
            Scenario::Count => {
                let reader = open(input)?;
                Ok(reader.header().features_count())
            }
            Scenario::FullRead => full_read(input),
            Scenario::BBoxQuery => {
                let bbox = *require(&params.bbox, "bbox", scenario)?;
                let reader = open(input)?;
                // FCB's packed R-tree is 2D; drop the z components
                // (indices 2/5) rather than approximate them.
                let query = SpatialQuery::BBox(bbox[0], bbox[1], bbox[3], bbox[4]);
                let iter = reader.select_query(query, None, None)?;
                Ok(iter.features_count().unwrap_or(0) as u64)
            }
            Scenario::AttrFilter => {
                let column = require(&params.attr_column, "attr-column", scenario)?;
                let pred = require(&params.attr_pred, "attr-eq/--attr-ge/--attr-le", scenario)?;
                attr_filter(input, column, pred)
            }
            Scenario::AttrStats => {
                let column = require(&params.attr_column, "attr-column", scenario)?;
                attr_stats(input, column)
            }
            Scenario::IdLookup => {
                let id = require(&params.target_id, "target-id", scenario)?;
                id_lookup(input, id)
            }
            Scenario::Project => {
                let column = require(&params.attr_column, "attr-column", scenario)?;
                project(input, column)
            }
        }
    }
}
