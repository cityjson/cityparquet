//! Every query parameter a read-benchmark measurement is driven with,
//! derived once per dataset from the prepared artefacts themselves — never
//! hardcoded, never fabricated.
//!
//! This lives in the library rather than beside the coordinator so the
//! choices it makes are testable without spawning a single child process:
//! [`window_for_target`] is a pure function over an in-memory slice, and the
//! integration tests reach the rest directly.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail};
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, DictionaryArray, Float64Array, Int64Array, RecordBatch, StringArray,
    StructArray,
};
use arrow_schema::{DataType, Schema};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet_schema::CityMetadata;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::naming::strip_known_extension;

/// Opens `table` just far enough to read its embedded CityParquet key-value
/// metadata — the `attribute_columns` list `pick_numeric_attribute` chooses
/// from.
pub fn open_metadata(table: &Path) -> Result<CityMetadata> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet metadata from {}", table.display()))?;
    Ok(builder.cityparquet_metadata()?)
}

/// `table`'s Arrow schema — the types `pick_numeric_attribute` filters on.
pub fn open_arrow_schema(table: &Path) -> Result<Schema> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet schema from {}", table.display()))?;
    Ok((*builder.cityparquet_arrow_schema()?).clone())
}

/// Every row's bbox, plus their union.
///
/// The whole vector is kept — not just the union — because
/// [`window_for_target`] searches for a window that intersects a target
/// FRACTION of rows, which cannot be answered from the extent alone. One
/// `[f64; 6]` per row is 48 bytes; the largest corpus dataset holds roughly
/// 199,000 rows, so under 10 MB.
pub struct RowBoxes {
    pub boxes: Vec<[f64; 6]>,
    pub dataset: [f64; 6],
}

/// Appends every row's bbox in `batch` to `out`. A row with a null bbox
/// contributes nothing — it has no extent, so no window can intersect it.
fn collect_batch_bboxes(batch: &RecordBatch, out: &mut Vec<[f64; 6]>) {
    let Some(bbox_col) = batch.column_by_name("bbox") else {
        return;
    };
    let Some(bbox_col) = bbox_col.as_any().downcast_ref::<StructArray>() else {
        return;
    };
    let leaf = |name: &str| {
        bbox_col
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
    };
    let (Some(xmin), Some(ymin), Some(zmin), Some(xmax), Some(ymax), Some(zmax)) = (
        leaf("xmin"),
        leaf("ymin"),
        leaf("zmin"),
        leaf("xmax"),
        leaf("ymax"),
        leaf("zmax"),
    ) else {
        return;
    };

    for row in 0..batch.num_rows() {
        if bbox_col.is_null(row) {
            continue;
        }
        out.push([
            xmin.value(row),
            ymin.value(row),
            zmin.value(row),
            xmax.value(row),
            ymax.value(row),
            zmax.value(row),
        ]);
    }
}

/// Scans the whole `bbox` column of `table` (a single-column projection),
/// keeping every row's own box and unioning them into the dataset extent.
pub fn scan_row_bboxes(table: &Path) -> Result<RowBoxes> {
    let file =
        std::fs::File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["bbox"]);
    let reader = builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning bbox column of {}", table.display()))?;

    let mut boxes: Vec<[f64; 6]> = Vec::new();
    for batch in reader {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        collect_batch_bboxes(&batch, &mut boxes);
    }

    let mut iter = boxes.iter();
    let first = *iter.next().ok_or_else(|| {
        anyhow::anyhow!(
            "no row in {} has a bbox — cannot derive a query window",
            table.display()
        )
    })?;
    let dataset = iter.fold(first, |acc, row| {
        [
            acc[0].min(row[0]),
            acc[1].min(row[1]),
            acc[2].min(row[2]),
            acc[3].max(row[3]),
            acc[4].max(row[4]),
            acc[5].max(row[5]),
        ]
    });

    Ok(RowBoxes { boxes, dataset })
}

/// `(target fraction of rows, notes tag)` for the three bbox windows — one
/// CSV row per entry.
///
/// The targets are fractions of ROWS, not of the dataset's area. An
/// area-anchored window says nothing about how many objects it selects: the
/// retired lower-left construction returned zero rows for `bbox-1pct` on
/// every dataset in the corpus, on every format.
pub const BBOX_TARGETS: [(f64, &str); 3] = [
    (0.01, "bbox-1pct"),
    (0.05, "bbox-5pct"),
    (0.25, "bbox-25pct"),
];

/// How far the achieved fraction may sit from the target before the window
/// is disclosed as `approx`, as a fraction OF THE TARGET (so 10% of 1% is
/// one part in a thousand, not one in ten).
const BBOX_TOLERANCE: f64 = 0.1;

/// Bisection steps. The count of intersecting rows is a step function of the
/// half-extent, so the search converges on a jump rather than a point; 60
/// halvings take the bracket well below one row's width on any real extent.
const BBOX_SEARCH_STEPS: u32 = 60;

/// One resolved bbox window: which target it was searched for, what fraction
/// of rows it actually selects, and whether those two agree.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BboxWindow {
    /// The `notes` tag this window's CSV rows carry, e.g. `bbox-1pct`.
    pub tag: String,
    /// The fraction of rows the search aimed at.
    pub target: f64,
    /// The fraction of rows the returned window actually intersects.
    pub achieved: f64,
    /// `[minx, miny, minz, maxx, maxy, maxz]`.
    pub window: [f64; 6],
    /// `achieved` is outside [`BBOX_TOLERANCE`] of `target` — the target was
    /// not reachable on this data. Disclosed in `notes`, never silent.
    pub approx: bool,
}

/// The same 3D overlap test every format runner applies row-by-row
/// (`formats::cityjsonseq::intersects`), so `achieved` is exactly what the
/// CityParquet runner will report for this window rather than an estimate.
fn intersects(row: &[f64; 6], window: &[f64; 6]) -> bool {
    for axis in 0..3 {
        if row[axis + 3] < window[axis] || row[axis] > window[axis + 3] {
            return false;
        }
    }
    true
}

/// The median of `values` (must be non-empty).
fn median_of(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("bbox coordinates are finite"));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// A window centred on `centre`, extending `half` of each of the dataset's
/// own x/y spans, and always covering the dataset's FULL z range — a query
/// window's z must never exclude a row, because `readbench_duckdb.sh` tests
/// x/y overlap only and the two must agree.
fn window_at(centre: (f64, f64), half: f64, dataset: [f64; 6]) -> [f64; 6] {
    let span_x = dataset[3] - dataset[0];
    let span_y = dataset[4] - dataset[1];
    [
        centre.0 - half * span_x,
        centre.1 - half * span_y,
        dataset[2],
        centre.0 + half * span_x,
        centre.1 + half * span_y,
        dataset[5],
    ]
}

/// Searches for a window intersecting `target` (a fraction in `(0, 1]`) of
/// `boxes`, centred on the median row centre so it lands where the data is
/// rather than at a bounding-box corner.
///
/// The half-extent scales with each axis's OWN span, so a long, thin tile
/// receives a long, thin window instead of a square one that misses on the
/// short axis. The row count is monotonically non-decreasing in the
/// half-extent, which is what makes bisection valid.
///
/// A target that cannot be reached — 1% of a 10-row dataset is 0.1 rows —
/// returns the nearest achievable window with `approx` set, never a silently
/// missed target.
pub fn window_for_target(
    boxes: &[[f64; 6]],
    dataset: [f64; 6],
    target: f64,
    tag: &str,
) -> BboxWindow {
    let total = boxes.len();
    assert!(total > 0, "window_for_target needs at least one row box");

    let mut xs: Vec<f64> = boxes.iter().map(|b| (b[0] + b[3]) / 2.0).collect();
    let mut ys: Vec<f64> = boxes.iter().map(|b| (b[1] + b[4]) / 2.0).collect();
    let centre = (median_of(&mut xs), median_of(&mut ys));

    let count_at = |half: f64| -> usize {
        let w = window_at(centre, half, dataset);
        boxes.iter().filter(|b| intersects(b, &w)).count()
    };

    let fraction = |count: usize| count as f64 / total as f64;
    let wanted = target * total as f64;

    // `hi` must select everything: half = 1.0 spans the full extent either
    // side of the centre, which covers the dataset whatever the centre is.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..BBOX_SEARCH_STEPS {
        let mid = (lo + hi) / 2.0;
        if (count_at(mid) as f64) < wanted {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    // `hi` is the smallest searched half-extent reaching the target; `lo` the
    // largest falling short. Whichever lands closer to the target wins, but
    // never an empty window — a zero-row window is the defect this function
    // replaces.
    let mut best = hi;
    let mut best_count = count_at(hi);
    let lo_count = count_at(lo);
    if lo_count > 0 && (fraction(lo_count) - target).abs() < (fraction(best_count) - target).abs() {
        best = lo;
        best_count = lo_count;
    }

    let achieved = fraction(best_count);
    BboxWindow {
        tag: tag.to_string(),
        target,
        achieved,
        window: window_at(centre, best, dataset),
        approx: (achieved - target).abs() > BBOX_TOLERANCE * target,
    }
}

/// Every feature's own top-level `id`, in the CityJSONSeq stream's order —
/// the canonical order the id deciles are cut from, because
/// `readbench_prepare.sh` builds the gzipped, FlatCityBuf and CityParquet
/// artefacts from this one file.
///
/// The first line of a `.city.jsonl` is the CityJSON metadata object, not a
/// feature; it is skipped.
pub fn seq_feature_ids(seq_path: &Path) -> Result<Vec<String>> {
    use std::io::BufRead as _;

    let file =
        std::fs::File::open(seq_path).with_context(|| format!("opening {}", seq_path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut ids = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("reading {}", seq_path.display()))?;
        if line.trim().is_empty() || index == 0 {
            continue;
        }
        let feature: cityparquet::cjseq::CityJSONFeature = serde_json::from_str(&line)
            .with_context(|| {
                format!(
                    "parsing feature on line {} of {}",
                    index + 1,
                    seq_path.display()
                )
            })?;
        ids.push(feature.id);
    }

    if ids.is_empty() {
        anyhow::bail!(
            "{} holds no features — cannot derive id probes",
            seq_path.display()
        );
    }
    Ok(ids)
}

/// Every CityObject key in a CityGML artefact.
///
/// The CityGML is synthesised from the source CityJSON by `citygml-tools`
/// rather than cut from the seq stream, so its member set is not guaranteed
/// to match: `benchmark/README.md` records that `3dbag_9-284-556` loses an
/// LoD in that round trip. An id probe absent here would be timed as a hit
/// and recorded as a miss, which is why every probe is checked against this
/// set.
pub fn citygml_ids(gml_path: &Path) -> Result<std::collections::HashSet<String>> {
    let source = cityparquet::source::Source::open(gml_path)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("opening {}", gml_path.display()))?;
    let transform = source.header().transform.clone();
    let mut reader =
        cityparquet::citygml::FeatureReader::open_without_appearance(gml_path, &transform)
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("streaming {}", gml_path.display()))?;

    let mut ids = std::collections::HashSet::new();
    for feature in reader.by_ref() {
        let feature = feature
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("reading a member of {}", gml_path.display()))?;
        ids.extend(feature.city_objects.keys().cloned());
    }
    Ok(ids)
}

/// `(position in the canonical order, notes tag)` for the three id-lookup
/// hit probes. A single target would make the published time a function of
/// where that one id happened to sit in the stream.
pub const ID_DECILES: [(f64, &str); 3] =
    [(0.10, "id-10pct"), (0.50, "id-50pct"), (0.90, "id-90pct")];

/// The tag of the fourth probe: an id verified absent from the dataset.
/// Position-free, and the number that actually separates a format with an id
/// index from one without.
pub const ID_MISS_TAG: &str = "id-miss";

/// One resolved id-lookup target.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdProbe {
    /// The `notes` tag this probe's CSV row carries.
    pub tag: String,
    pub id: String,
    /// Whether this id is expected to be found. False only for
    /// [`ID_MISS_TAG`].
    pub present: bool,
    /// The nominal decile id was not verifiable in every artefact and the
    /// nearest one that was has been used instead.
    pub substituted: bool,
}

/// An id guaranteed absent from `taken`, derived from `seed` so it is
/// reproducible across runs rather than random.
pub fn miss_id(seed: &str, taken: &std::collections::HashSet<String>) -> String {
    let base = format!("{seed}-readbench-absent");
    if !taken.contains(&base) {
        return base;
    }
    for suffix in 2u32.. {
        let candidate = format!("{base}-{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 exhausted while avoiding an id collision")
}

/// The four id probes for a dataset: three positioned hits plus a verified
/// miss.
///
/// `seq_ids` is the canonical order — the CityJSONSeq stream every other
/// artefact is cut from. `verifiable` is the set of ids confirmed to exist in
/// EVERY artefact that will be asked for them; a nominal decile id outside it
/// is replaced by the nearest index that is inside, with `substituted` set,
/// because timing a lookup for an id one artefact does not contain would
/// record a hit as a miss.
pub fn id_probes(
    seq_ids: &[String],
    verifiable: &std::collections::HashSet<String>,
) -> Vec<IdProbe> {
    assert!(!seq_ids.is_empty(), "id_probes needs at least one feature");

    let nearest_verifiable = |from: usize| -> Option<(usize, String)> {
        for offset in 0..seq_ids.len() {
            for index in [
                from.saturating_sub(offset),
                (from + offset).min(seq_ids.len() - 1),
            ] {
                if verifiable.contains(&seq_ids[index]) {
                    return Some((index, seq_ids[index].clone()));
                }
            }
        }
        None
    };

    let mut probes: Vec<IdProbe> = Vec::with_capacity(ID_DECILES.len() + 1);
    for (position, tag) in ID_DECILES {
        let nominal = ((position * seq_ids.len() as f64) as usize).min(seq_ids.len() - 1);
        let Some((index, id)) = nearest_verifiable(nominal) else {
            continue;
        };
        probes.push(IdProbe {
            tag: tag.to_string(),
            id,
            present: true,
            substituted: index != nominal,
        });
    }

    let taken: std::collections::HashSet<String> = seq_ids.iter().cloned().collect();
    let seed = probes
        .iter()
        .find(|p| p.tag == "id-50pct")
        .map(|p| p.id.clone())
        .unwrap_or_else(|| seq_ids[0].clone());
    probes.push(IdProbe {
        tag: ID_MISS_TAG.to_string(),
        id: miss_id(&seed, &taken),
        present: false,
        substituted: false,
    });

    probes
}

/// The feature-lookup probe tags: the feature at the 50 % position of the
/// canonical order, and a `feature_id` verified absent.
pub const FEATURE_50PCT_TAG: &str = "feature-50pct";
pub const FEATURE_MISS_TAG: &str = "feature-miss";

/// The two feature-lookup probes, taken from the id probes: a CityJSONSeq
/// feature's own id IS the `feature_id` of every row it contributes, so the
/// `id-50pct` probe names the middle feature and the verified `id-miss` is
/// absent from `feature_id` too (it is checked against every feature id).
pub fn feature_probes(id_probes: &[IdProbe]) -> Vec<IdProbe> {
    id_probes
        .iter()
        .filter_map(|probe| {
            let tag = match probe.tag.as_str() {
                "id-50pct" => FEATURE_50PCT_TAG,
                ID_MISS_TAG => FEATURE_MISS_TAG,
                _ => return None,
            };
            Some(IdProbe {
                tag: tag.to_string(),
                ..probe.clone()
            })
        })
        .collect()
}

/// `array`'s Utf8 values as `Option<String>` per row (`None` for a null
/// cell) — handles both a plain `Utf8` array and a `Dictionary<Int32,
/// Utf8>` array, because a string ATTRIBUTE column may be written as
/// either, mirroring `cityparquet::query::evaluate_attr_predicate`'s own
/// `Utf8`/`Dictionary` dispatch.
fn utf8_values(array: &dyn Array) -> Result<Vec<Option<String>>> {
    match array.data_type() {
        DataType::Utf8 => {
            let values = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow::anyhow!("expected a Utf8 array"))?;
            Ok((0..values.len())
                .map(|i| (!values.is_null(i)).then(|| values.value(i).to_string()))
                .collect())
        }
        DataType::Dictionary(key_type, value_type)
            if key_type.as_ref() == &DataType::Int32 && value_type.as_ref() == &DataType::Utf8 =>
        {
            let dict = array
                .as_any()
                .downcast_ref::<DictionaryArray<Int32Type>>()
                .ok_or_else(|| anyhow::anyhow!("expected a Dictionary<Int32, Utf8> array"))?;
            let values = dict
                .downcast_dict::<StringArray>()
                .ok_or_else(|| anyhow::anyhow!("dictionary values are not Utf8"))?;
            Ok((0..dict.len())
                .map(|i| (!dict.is_null(i)).then(|| values.value(i).to_string()))
                .collect())
        }
        other => bail!("expected a Utf8 or Dictionary<Int32, Utf8> array, got {other:?}"),
    }
}

/// `array`'s values as `Option<f64>` per row (`None` for a null cell), for
/// the two numeric attribute types [`pick_numeric_attribute`] admits.
fn f64_values(array: &dyn Array) -> Result<Vec<Option<f64>>> {
    match array.data_type() {
        DataType::Float64 => {
            let values = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| anyhow::anyhow!("expected a Float64 array"))?;
            Ok((0..values.len())
                .map(|i| (!values.is_null(i)).then(|| values.value(i)))
                .collect())
        }
        DataType::Int64 => {
            let values = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| anyhow::anyhow!("expected an Int64 array"))?;
            Ok((0..values.len())
                .map(|i| (!values.is_null(i)).then(|| values.value(i) as f64))
                .collect())
        }
        other => bail!("expected an Int64 or Float64 array, got {other:?}"),
    }
}

/// Opens `table` projected down to `columns`, in the order given.
fn projected_reader(
    table: &Path,
    columns: &[&str],
) -> Result<parquet::arrow::arrow_reader::ParquetRecordBatchReader> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), columns.iter().copied());
    builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning {columns:?} of {}", table.display()))
}

/// `(rows whose `column` equals `value`, total rows)` — a single-column
/// projected scan of `table`.
fn count_string_matches(table: &Path, column: &str, value: &str) -> Result<(u64, u64)> {
    let mut matched = 0u64;
    let mut total = 0u64;
    for batch in projected_reader(table, &[column])? {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        total += batch.num_rows() as u64;
        for cell in utf8_values(batch.column(0).as_ref())?.into_iter().flatten() {
            if cell == value {
                matched += 1;
            }
        }
    }
    Ok((matched, total))
}

/// Every non-null value of the numeric `column`, plus `table`'s total row
/// count — a single-column projected scan. One `f64` per non-null row: 8 MB
/// on the largest slice in the corpus.
fn numeric_column_values(table: &Path, column: &str) -> Result<(Vec<f64>, u64)> {
    let mut values: Vec<f64> = Vec::new();
    let mut total = 0u64;
    for batch in projected_reader(table, &[column])? {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        total += batch.num_rows() as u64;
        values.extend(f64_values(batch.column(0).as_ref())?.into_iter().flatten());
    }
    Ok((values, total))
}

/// `quantile` of `values` by LINEAR INTERPOLATION between the two
/// order statistics that bracket it — the "continuous" definition (numpy's
/// `linear`, DuckDB's `quantile_cont`), so the threshold the sidecar
/// publishes is one a reader can reproduce with a one-line SQL query.
/// `values` must be non-empty; it is sorted in place.
pub fn quantile_of(values: &mut [f64], quantile: f64) -> f64 {
    assert!(!values.is_empty(), "quantile_of needs at least one value");
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let position = quantile * (values.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        return values[lower];
    }
    values[lower] + (position - lower as f64) * (values[upper] - values[lower])
}

/// The alphabetically-first `Int64`/`Float64` attribute column in `meta`'s
/// own `attribute_columns` list (never a geometry/reserved column), or
/// `None` if the dataset has no numeric attribute at all — deterministic
/// across runs, never fabricated when no such column exists (e.g.
/// `lod3_railway.city.json`, whose attributes are all strings).
fn pick_numeric_attribute(meta: &CityMetadata, schema: &Schema) -> Option<String> {
    let mut candidates: Vec<String> = meta
        .attributes
        .iter()
        .filter(|name| {
            schema
                .field_with_name(name)
                .map(|f| matches!(f.data_type(), DataType::Int64 | DataType::Float64))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// The `attr-filter` predicate: either a string equality or a numeric lower
/// bound, on a real CityJSON ATTRIBUTE.
///
/// Serialised into the sidecar externally tagged and lower-cased, so the
/// DuckDB baseline can dispatch on it with `jq` alone
/// (`{"eq": "slanted"}` / `{"ge": 2.45}`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttrFilterPred {
    /// `column == value`, always a STRING comparison (see
    /// `main.rs::build_attr_pred` for why `--attr-eq` never means numeric
    /// equality).
    Eq(String),
    /// `column >= bound`.
    Ge(f64),
}

/// One resolved `attr-filter` predicate, and what it matched when it was
/// derived.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttrFilterSpec {
    /// The attribute column the predicate runs against — a member of the
    /// CityParquet package's own `attributes` list, never a reserved
    /// structural column.
    pub column: String,
    pub pred: AttrFilterPred,
    /// CityObject rows the predicate matched on the CityParquet table at
    /// derivation time — the count every format's own `result_count` must
    /// agree with (the coordinator's self-consistency check).
    pub matched: u64,
    /// `matched` as a fraction of the table's rows.
    pub share: f64,
    /// The column and predicate came from [`HAND_PICKED`] rather than from
    /// [`fallback_attr_filter`]'s derived rule.
    pub hand_picked: bool,
}

impl AttrFilterSpec {
    /// This predicate's `notes` tag: `attr=<column>=<value>` for an
    /// equality, `attr=<column>>=<bound>` for a numeric lower bound.
    pub fn notes_tag(&self) -> String {
        match &self.pred {
            AttrFilterPred::Eq(value) => format!("attr={}={}", self.column, value),
            AttrFilterPred::Ge(bound) => format!("attr={}>={}", self.column, bound),
        }
    }
}

/// What a [`HAND_PICKED`] entry asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pick {
    /// Equality against this exact string value.
    Eq(&'static str),
    /// `>=` the column's own quantile at this position, computed from the
    /// data at derivation time.
    Quantile(f64),
}

/// How a [`HAND_PICKED`] entry is matched against a dataset's base name.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Key {
    /// The dataset IS this name, or a `-<suffix>` ordering variant of it
    /// (`rotterdam_delfshaven-hilbert`).
    Dataset(&'static str),
    /// Every dataset whose name starts with this prefix — the 3DBAG
    /// scaling slices (`3dbag_n1000` … `3dbag_n1000000`, each also with a
    /// `-hilbert` variant), which are prefixes of one source stream and so
    /// all carry the same attributes.
    Family(&'static str),
}

/// The per-dataset `attr-filter` predicate for the corpus this benchmark
/// actually measures, chosen once by hand from a survey of the prepared
/// packages.
///
/// The derived rule below would pick *a* workable attribute on each of
/// these, but not the one a reader of the paper would recognise as the
/// query the format is for: a roof type on 3DBAG, a building class on
/// Zurich, a roof type on Vienna. Every entry names a column that is a
/// member of the CityJSON `attributes` map (so FlatCityBuf's `fcb ser -A`
/// B+-tree indexes it) and that every format carries, and every predicate
/// was verified to return the same `result_count` on every runner.
const HAND_PICKED: [(Key, &str, Pick); 6] = [
    // The natural roof-type query; 3DBAG carries it on the Building, one
    // per feature.
    (Key::Family("3dbag_"), "b3_dak_type", Pick::Eq("slanted")),
    (
        Key::Dataset("zurich_building_lod2"),
        "class",
        Pick::Eq("BB01"),
    ),
    (
        Key::Dataset("vienna_102081"),
        "roofType",
        Pick::Eq("FLACHDACH"),
    ),
    (
        Key::Dataset("ingolstadt"),
        "klumMaterialClass",
        Pick::Eq("Wood"),
    ),
    // NYC's only categorical attributes are identifiers; `1000000` is the
    // placeholder BIN, a legitimate low-selectivity equality.
    (
        Key::Dataset("nyc_da13_buildings"),
        "BIN",
        Pick::Eq("1000000"),
    ),
    // Rotterdam's string attributes are constant, so a numeric range is
    // the only selective predicate it has.
    (
        Key::Dataset("rotterdam_delfshaven"),
        "TerrainHeight",
        Pick::Quantile(FALLBACK_QUANTILE),
    ),
];

/// The share of rows [`fallback_attr_filter`]'s string branch aims a
/// predicate at — selective enough that an index can help, common enough
/// that the result is not a rounding error.
const FALLBACK_TARGET_SHARE: f64 = 0.25;

/// The quantile the numeric branch (and Rotterdam's hand-picked entry)
/// thresholds at, so that `>=` selects about [`FALLBACK_TARGET_SHARE`] of
/// the non-null rows.
const FALLBACK_QUANTILE: f64 = 0.75;

/// A string attribute needs at least this many distinct values to be a
/// candidate — a constant column has no selective equality.
const FALLBACK_MIN_DISTINCT: usize = 2;

/// …and at most this many, above which the column is an identifier rather
/// than a category. Also the cap on how many distinct values are tallied
/// per column, so a near-unique column costs a bounded amount of memory.
const FALLBACK_MAX_DISTINCT: usize = 1000;

/// Characters that would corrupt the results CSV's `notes` column (or its
/// `;`-separated disclosure list) if they appeared in a chosen value.
const NOTES_HOSTILE: [char; 5] = [';', ',', '"', '\n', '\r'];

/// The [`HAND_PICKED`] entry for `base`, if any.
fn hand_picked_for(base: &str) -> Option<(&'static str, Pick)> {
    HAND_PICKED.iter().find_map(|(key, column, pick)| {
        let hit = match key {
            Key::Dataset(name) => base == *name || base.starts_with(&format!("{name}-")),
            Key::Family(prefix) => base.starts_with(prefix),
        };
        hit.then_some((*column, *pick))
    })
}

/// `column` is an attribute column of this package (never a reserved
/// structural column) whose Arrow type is `wanted`.
fn attribute_of_type(
    meta: &CityMetadata,
    schema: &Schema,
    column: &str,
    wanted: fn(&DataType) -> bool,
) -> bool {
    meta.attributes.iter().any(|name| name == column)
        && schema
            .field_with_name(column)
            .map(|field| wanted(field.data_type()))
            .unwrap_or(false)
}

/// Utf8 or `Dictionary<Int32, Utf8>` — the two shapes [`utf8_values`] reads.
fn is_string_type(data_type: &DataType) -> bool {
    match data_type {
        DataType::Utf8 => true,
        DataType::Dictionary(key, value) => {
            key.as_ref() == &DataType::Int32 && value.as_ref() == &DataType::Utf8
        }
        _ => false,
    }
}

fn is_numeric_type(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Int64 | DataType::Float64)
}

/// Resolves one [`Pick`] against the data: counts what it matches, and for
/// a [`Pick::Quantile`] computes the threshold first. `None` when the pick
/// matches nothing at all — a zero-result `attr-filter` measures the cost
/// of proving an absence, not of indexed attribute access.
fn resolve_pick(
    table: &Path,
    column: &str,
    pick: Pick,
    hand_picked: bool,
) -> Result<Option<AttrFilterSpec>> {
    let (pred, matched, total) = match pick {
        Pick::Eq(value) => {
            let (matched, total) = count_string_matches(table, column, value)?;
            (AttrFilterPred::Eq(value.to_string()), matched, total)
        }
        Pick::Quantile(quantile) => {
            let (mut values, total) = numeric_column_values(table, column)?;
            if values.is_empty() {
                return Ok(None);
            }
            let bound = quantile_of(&mut values, quantile);
            let matched = values.iter().filter(|v| **v >= bound).count() as u64;
            (AttrFilterPred::Ge(bound), matched, total)
        }
    };
    if matched == 0 || total == 0 {
        return Ok(None);
    }
    Ok(Some(AttrFilterSpec {
        column: column.to_string(),
        pred,
        matched,
        share: matched as f64 / total as f64,
        hand_picked,
    }))
}

/// One surveyed string attribute column: its most frequent CSV-safe value,
/// that value's row count, and how many distinct values the column holds
/// (saturating at [`FALLBACK_MAX_DISTINCT`] + 1, at which point the column
/// is disqualified and no longer tallied).
struct StringSurvey {
    column: String,
    distinct: usize,
    top: Option<(String, u64)>,
}

/// Tallies every candidate string attribute column of `table` in ONE
/// projected scan. A column that passes [`FALLBACK_MAX_DISTINCT`] distinct
/// values stops being tallied, so a near-unique identifier column costs a
/// bounded amount of memory rather than one `String` per row.
fn survey_string_columns(table: &Path, columns: &[String]) -> Result<(Vec<StringSurvey>, u64)> {
    let names: Vec<&str> = columns.iter().map(String::as_str).collect();
    let mut counts: Vec<Option<HashMap<String, u64>>> =
        columns.iter().map(|_| Some(HashMap::new())).collect();
    let mut over_cap: Vec<bool> = columns.iter().map(|_| false).collect();
    let mut total = 0u64;

    for batch in projected_reader(table, &names)? {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        total += batch.num_rows() as u64;
        // The projection preserves the file's own column order, not
        // `names`', so each column is found by NAME rather than by index.
        for (index, column) in columns.iter().enumerate() {
            let Some(array) = batch.column_by_name(column) else {
                continue;
            };
            let Some(tally) = counts[index].as_mut() else {
                continue;
            };
            for value in utf8_values(array.as_ref())?.into_iter().flatten() {
                *tally.entry(value).or_insert(0) += 1;
            }
            if tally.len() > FALLBACK_MAX_DISTINCT {
                counts[index] = None;
                over_cap[index] = true;
            }
        }
    }

    Ok((
        columns
            .iter()
            .enumerate()
            .map(|(index, column)| match counts[index].take() {
                None => StringSurvey {
                    column: column.clone(),
                    distinct: FALLBACK_MAX_DISTINCT + 1,
                    top: None,
                },
                Some(tally) => {
                    let distinct = tally.len();
                    // Ties on count are broken by the LEXICOGRAPHICALLY
                    // SMALLEST value, never by `HashMap` iteration order
                    // (SipHash-randomised per process), so the derived
                    // predicate is reproducible run-to-run.
                    let top = tally
                        .into_iter()
                        // The empty string is excluded for the same reason:
                        // it renders as a bare `attr=<column>=`, which
                        // `readbench_duckdb.sh` reads back as "no predicate"
                        // and refuses.
                        .filter(|(value, _)| !value.is_empty() && !value.contains(NOTES_HOSTILE))
                        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
                    StringSurvey {
                        column: column.clone(),
                        distinct,
                        top,
                    }
                }
            })
            .collect(),
        total,
    ))
}

/// The derived rule, for a dataset [`HAND_PICKED`] does not name.
///
/// FIRST the string attribute column with between [`FALLBACK_MIN_DISTINCT`]
/// and [`FALLBACK_MAX_DISTINCT`] distinct values whose most frequent value's
/// share of rows lies closest to [`FALLBACK_TARGET_SHARE`], ties broken by
/// column name; its predicate is equality with that value. FAILING THAT, the
/// alphabetically-first numeric attribute, thresholded at its own
/// [`FALLBACK_QUANTILE`]. FAILING THAT, `None` — `attr-filter` is skipped
/// and the coordinator says so, exactly as `attr-stats` already is for a
/// dataset with no numeric attribute. Never fabricated.
fn fallback_attr_filter(
    meta: &CityMetadata,
    schema: &Schema,
    table: &Path,
) -> Result<Option<AttrFilterSpec>> {
    let mut string_columns: Vec<String> = meta
        .attributes
        .iter()
        .filter(|name| attribute_of_type(meta, schema, name, is_string_type))
        .cloned()
        .collect();
    string_columns.sort();
    string_columns.dedup();

    if !string_columns.is_empty() {
        let (surveys, total) = survey_string_columns(table, &string_columns)?;
        let best = surveys
            .iter()
            .filter(|survey| {
                (FALLBACK_MIN_DISTINCT..=FALLBACK_MAX_DISTINCT).contains(&survey.distinct)
            })
            .filter_map(|survey| {
                let (value, count) = survey.top.as_ref()?;
                let share = *count as f64 / total.max(1) as f64;
                Some((survey, value, *count, share))
            })
            .min_by(|a, b| {
                (a.3 - FALLBACK_TARGET_SHARE)
                    .abs()
                    .partial_cmp(&(b.3 - FALLBACK_TARGET_SHARE).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.column.cmp(&b.0.column))
            });
        if let Some((survey, value, matched, share)) = best {
            return Ok(Some(AttrFilterSpec {
                column: survey.column.clone(),
                pred: AttrFilterPred::Eq(value.clone()),
                matched,
                share,
                hand_picked: false,
            }));
        }
    }

    match pick_numeric_attribute(meta, schema) {
        Some(column) => resolve_pick(table, &column, Pick::Quantile(FALLBACK_QUANTILE), false),
        None => Ok(None),
    }
}

/// The `attr-filter` predicate for `dataset`: the [`HAND_PICKED`] entry when
/// there is one and the package actually carries it, the derived rule
/// otherwise.
///
/// `dataset` is the input's FILE NAME (extension included, as
/// [`resolve`] receives it), so the ordering variants share their source
/// dataset's pick: `3dbag_n1000-hilbert.city.jsonl` is the same data as
/// `3dbag_n1000.city.jsonl` in a different row order, and measuring the two
/// with different predicates would compare nothing.
///
/// The predicate is never a reserved structural column. `object_type` — the
/// column this scenario used to be driven with — is not a member of the
/// CityJSON `attributes` map, which is exactly what FlatCityBuf's B+-tree
/// attribute index covers (`fcb_core`'s `writer::attribute`), so every
/// FlatCityBuf row of every committed run fell back to a full walk and the
/// scenario compared an indexed read against nothing.
pub fn pick_attr_filter(
    dataset: &str,
    meta: &CityMetadata,
    schema: &Schema,
    table: &Path,
) -> Result<Option<AttrFilterSpec>> {
    let base = strip_known_extension(dataset);

    if let Some((column, pick)) = hand_picked_for(base) {
        let wanted: fn(&DataType) -> bool = match pick {
            Pick::Eq(_) => is_string_type,
            Pick::Quantile(_) => is_numeric_type,
        };
        if attribute_of_type(meta, schema, column, wanted) {
            match resolve_pick(table, column, pick, true)? {
                Some(spec) => return Ok(Some(spec)),
                None => eprintln!(
                    "cityparquet-readbench: the hand-picked attr-filter predicate on \
                     '{column}' matches no row of '{base}' — falling back to the derived rule"
                ),
            }
        } else {
            eprintln!(
                "cityparquet-readbench: '{base}' has a hand-picked attr-filter column \
                 '{column}', but this package carries no attribute of that name and type — \
                 falling back to the derived rule"
            );
        }
    }

    fallback_attr_filter(meta, schema, table)
}

/// Every query parameter one dataset's whole (format x scenario) matrix is
/// driven with, derived once from the prepared artefacts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedParams {
    /// The dataset name as it appears in the results CSV's `dataset` column.
    pub dataset: String,
    pub windows: Vec<BboxWindow>,
    /// EMPTY when the CityJSONSeq artefact was not present to cut the
    /// deciles from — the caller then skips `id-lookup` and says so.
    pub id_probes: Vec<IdProbe>,
    /// The `attr-filter` predicate: an indexable CityJSON ATTRIBUTE and the
    /// comparison to run against it (see [`pick_attr_filter`]). `None` when
    /// the dataset carries no attribute this rule can build a selective
    /// predicate from — `attr-filter` is then skipped and the caller says
    /// so, never fabricated.
    pub attr_filter: Option<AttrFilterSpec>,
    /// `feature-50pct` and `feature-miss`, from [`feature_probes`]; EMPTY
    /// exactly when `id_probes` is.
    pub feature_probes: Vec<IdProbe>,
    /// The alphabetically-first Int64/Float64 attribute column, or `None`
    /// when the dataset has no numeric attribute at all. Never fabricated:
    /// `attr-stats` and `project` are skipped when this is `None`.
    pub numeric_attr: Option<String>,
    /// The dataset-global CityObject total — the SHARED selectivity
    /// denominator for every CityObject-level scenario.
    pub cp_object_total: u64,
}

/// Derives every query parameter for `dataset`.
///
/// The CityParquet package's main table (`cp_table`) is the only hard
/// requirement: it is the source of the row bboxes, the attribute choices
/// and the shared denominator.
///
/// `seq_path` is the CityJSONSeq artefact — the canonical order the id
/// deciles are cut from. `None` (the artefact is not present in this run's
/// prepared directory) yields NO id probes at all, and the caller skips
/// `id-lookup` with a logged message, exactly as it already does for a
/// dataset with no numeric attribute. Deriving the deciles from some other
/// order instead would quietly redefine what a probe's position means, and a
/// number nobody can interpret is worse than a number nobody has. A
/// `seq_path` that IS given but cannot be read is a hard failure.
///
/// `gml_path` is checked when present so an id probe absent from the
/// synthesised CityGML is substituted rather than silently timed as a miss.
pub fn resolve(
    dataset: &str,
    cp_table: &Path,
    seq_path: Option<&Path>,
    gml_path: Option<&Path>,
) -> Result<ResolvedParams> {
    let rows = scan_row_bboxes(cp_table)?;
    let windows = BBOX_TARGETS
        .iter()
        .map(|(target, tag)| window_for_target(&rows.boxes, rows.dataset, *target, tag))
        .collect();

    let id_probes = match seq_path {
        None => Vec::new(),
        Some(seq_path) => {
            let seq_ids = seq_feature_ids(seq_path)?;
            let mut verifiable: std::collections::HashSet<String> =
                seq_ids.iter().cloned().collect();
            if let Some(gml) = gml_path {
                let gml_ids = citygml_ids(gml)?;
                verifiable.retain(|id| gml_ids.contains(id));
                if verifiable.is_empty() {
                    anyhow::bail!(
                        "no feature id in {} also appears in {} — the two artefacts \
                         describe different data",
                        seq_path.display(),
                        gml.display()
                    );
                }
            }
            id_probes(&seq_ids, &verifiable)
        }
    };

    let feature_probes = feature_probes(&id_probes);
    let meta = open_metadata(cp_table)?;
    let schema = open_arrow_schema(cp_table)?;
    let attr_filter = pick_attr_filter(dataset, &meta, &schema, cp_table)?;
    let numeric_attr = pick_numeric_attribute(&meta, &schema);

    let file =
        std::fs::File::open(cp_table).with_context(|| format!("opening {}", cp_table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", cp_table.display()))?;
    let cp_object_total = builder.metadata().file_metadata().num_rows() as u64;

    Ok(ResolvedParams {
        dataset: dataset.to_string(),
        windows,
        id_probes,
        attr_filter,
        feature_probes,
        numeric_attr,
        cp_object_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("obj-{i}")).collect()
    }

    #[test]
    fn deciles_land_at_their_nominal_positions() {
        let all = ids(100);
        let present: HashSet<String> = all.iter().cloned().collect();
        let probes = id_probes(&all, &present);

        let hit = |tag: &str| {
            probes
                .iter()
                .find(|p| p.tag == tag)
                .unwrap_or_else(|| panic!("no probe tagged {tag}"))
                .id
                .clone()
        };
        assert_eq!(hit("id-10pct"), "obj-10");
        assert_eq!(hit("id-50pct"), "obj-50");
        assert_eq!(hit("id-90pct"), "obj-90");
    }

    #[test]
    fn every_decile_probe_is_present_and_the_miss_probe_is_not() {
        let all = ids(100);
        let present: HashSet<String> = all.iter().cloned().collect();
        let probes = id_probes(&all, &present);

        assert_eq!(probes.len(), 4, "three deciles plus one miss");
        for probe in &probes {
            if probe.tag == "id-miss" {
                assert!(!probe.present, "the miss probe must be absent");
                assert!(
                    !present.contains(&probe.id),
                    "the miss id must not be in the dataset"
                );
            } else {
                assert!(probe.present, "{} must be a real id", probe.tag);
            }
        }
    }

    /// A decile id missing from the CityGML artefact would be timed as a hit
    /// and recorded as a miss. It must be replaced by the nearest verifiable
    /// feature, and the substitution disclosed.
    #[test]
    fn substitutes_the_nearest_verifiable_id_and_says_so() {
        let all = ids(100);
        let mut verifiable: HashSet<String> = all.iter().cloned().collect();
        verifiable.remove("obj-50");
        let probes = id_probes(&all, &verifiable);

        let mid = probes
            .iter()
            .find(|p| p.tag == "id-50pct")
            .expect("id-50pct");
        assert_ne!(mid.id, "obj-50", "the unverifiable id must be replaced");
        assert!(mid.present, "the replacement must itself be verifiable");
        assert!(mid.substituted, "the substitution must be disclosed");
        assert!(
            verifiable.contains(&mid.id),
            "the replacement must be in the verifiable set"
        );
    }

    #[test]
    fn miss_id_avoids_a_collision_with_an_existing_id() {
        let mut taken = HashSet::new();
        taken.insert("obj-1".to_string());
        taken.insert("obj-1-readbench-absent".to_string());
        taken.insert("obj-1-readbench-absent-2".to_string());

        let miss = miss_id("obj-1", &taken);
        assert!(
            !taken.contains(&miss),
            "miss_id returned a taken id: {miss}"
        );
    }

    /// A `rows x cols` grid of unit boxes on a 100 x 100 field.
    fn grid(rows: usize, cols: usize) -> Vec<[f64; 6]> {
        let mut out = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let x = c as f64 * (100.0 / cols as f64);
                let y = r as f64 * (100.0 / rows as f64);
                out.push([x, y, 0.0, x + 0.1, y + 0.1, 1.0]);
            }
        }
        out
    }

    const FIELD: [f64; 6] = [0.0, 0.0, 0.0, 100.0, 100.0, 10.0];

    #[test]
    fn hits_every_target_on_a_uniform_grid() {
        let boxes = grid(100, 100); // 10,000 boxes
        for (target, tag) in BBOX_TARGETS {
            let w = window_for_target(&boxes, FIELD, target, tag);
            assert!(
                !w.approx,
                "{tag}: a uniform 10,000-box grid can hit {target} exactly, got {}",
                w.achieved
            );
            assert!(
                (w.achieved - target).abs() <= 0.1 * target,
                "{tag}: achieved {} is outside the tolerance around {target}",
                w.achieved
            );
        }
    }

    #[test]
    fn never_returns_an_empty_window() {
        let boxes = grid(100, 100);
        for (target, tag) in BBOX_TARGETS {
            let w = window_for_target(&boxes, FIELD, target, tag);
            assert!(w.achieved > 0.0, "{tag} selected no rows at all");
        }
    }

    /// The median centroid of a bimodal cloud falls in the gap between the
    /// two clusters. The search must still find a populated window rather
    /// than converging on the empty middle.
    #[test]
    fn finds_rows_when_the_median_falls_between_two_clusters() {
        let mut boxes = Vec::new();
        for i in 0..500 {
            let x = i as f64 * 0.02; // 0..10
            boxes.push([x, x, 0.0, x + 0.1, x + 0.1, 1.0]);
        }
        for i in 0..500 {
            let x = 90.0 + i as f64 * 0.02; // 90..100
            boxes.push([x, x, 0.0, x + 0.1, x + 0.1, 1.0]);
        }
        let w = window_for_target(&boxes, FIELD, 0.05, "bbox-5pct");
        assert!(
            w.achieved > 0.0,
            "a bimodal cloud must still yield a populated window, got {w:?}"
        );
    }

    /// A long, thin dataset must not receive a window whose y half-extent is
    /// so small it selects nothing: the half-extents scale with each axis's
    /// own span.
    #[test]
    fn scales_the_window_to_the_datasets_aspect_ratio() {
        let mut boxes = Vec::new();
        for i in 0..1000 {
            let x = i as f64; // 0..1000
            boxes.push([x, 0.0, 0.0, x + 0.5, 1.0, 1.0]);
        }
        let thin: [f64; 6] = [0.0, 0.0, 0.0, 1000.0, 1.0, 10.0];
        let w = window_for_target(&boxes, thin, 0.25, "bbox-25pct");
        assert!(w.achieved > 0.0, "thin dataset selected nothing: {w:?}");
        assert!(
            (w.achieved - 0.25).abs() <= 0.1 * 0.25,
            "thin dataset achieved {}, expected near 0.25",
            w.achieved
        );
    }

    /// 1% of 10 rows is 0.1 rows — unreachable. The search must disclose that
    /// with `approx` rather than silently reporting a missed target as met.
    #[test]
    fn flags_approx_when_the_target_is_unreachable() {
        let boxes = grid(2, 5); // 10 boxes
        let w = window_for_target(&boxes, FIELD, 0.01, "bbox-1pct");
        assert!(
            w.approx,
            "1% of 10 rows cannot be hit within tolerance; expected approx, got {w:?}"
        );
        assert!(
            w.achieved > 0.0,
            "even an unreachable target must yield a populated window, got {w:?}"
        );
    }

    // --- attr-filter predicate derivation ---------------------------------
    //
    // `pick_attr_filter` itself needs a real table to count against (the
    // integration tests in `tests/params.rs` cover that leg). What is pure,
    // and what the fairness of this scenario actually rests on, is WHICH
    // column and predicate each rule names — so that is what these pin.

    #[test]
    fn every_3dbag_slice_and_ordering_variant_gets_the_same_hand_picked_pick() {
        for base in [
            "3dbag_n1000",
            "3dbag_n5000",
            "3dbag_n1000000",
            "3dbag_n1000-hilbert",
        ] {
            assert_eq!(
                hand_picked_for(base),
                Some(("b3_dak_type", Pick::Eq("slanted"))),
                "{base} must share the 3DBAG family's roof-type predicate"
            );
        }
    }

    #[test]
    fn a_hand_picked_dataset_is_matched_through_its_ordering_variant_suffix() {
        assert_eq!(
            hand_picked_for("rotterdam_delfshaven"),
            Some(("TerrainHeight", Pick::Quantile(FALLBACK_QUANTILE)))
        );
        assert_eq!(
            hand_picked_for("rotterdam_delfshaven-hilbert"),
            Some(("TerrainHeight", Pick::Quantile(FALLBACK_QUANTILE))),
            "an ordering variant is the same data and must get the same predicate"
        );
        assert_eq!(
            hand_picked_for("zurich_building_lod2"),
            Some(("class", Pick::Eq("BB01")))
        );
    }

    #[test]
    fn a_dataset_outside_the_table_gets_no_hand_picked_entry() {
        assert_eq!(hand_picked_for("delft"), None);
        assert_eq!(
            hand_picked_for("rotterdam"),
            None,
            "a PREFIX of a table key is a different dataset, not a variant of it"
        );
    }

    /// Nothing in [`HAND_PICKED`] may name `object_type` (or any other
    /// reserved structural column): it is not a member of the CityJSON
    /// `attributes` map, which is exactly what FlatCityBuf's B+-tree index
    /// covers — the defect this table exists to fix.
    #[test]
    fn no_hand_picked_column_is_a_reserved_structural_column() {
        for (_, column, _) in HAND_PICKED {
            assert!(
                !["object_type", "id", "bbox", "parents", "children"].contains(&column),
                "{column} is a reserved structural column, never an indexable attribute"
            );
        }
    }

    /// The `notes` tag must stay inside one CSV field and one `;`-separated
    /// disclosure slot.
    #[test]
    fn the_notes_tag_renders_both_predicate_shapes_without_csv_hostile_characters() {
        let eq = AttrFilterSpec {
            column: "b3_dak_type".to_string(),
            pred: AttrFilterPred::Eq("slanted".to_string()),
            matched: 356,
            share: 0.356,
            hand_picked: true,
        };
        assert_eq!(eq.notes_tag(), "attr=b3_dak_type=slanted");

        let ge = AttrFilterSpec {
            column: "TerrainHeight".to_string(),
            pred: AttrFilterPred::Ge(2.45),
            matched: 217,
            share: 0.2544,
            hand_picked: true,
        };
        assert_eq!(ge.notes_tag(), "attr=TerrainHeight>=2.45");

        for tag in [eq.notes_tag(), ge.notes_tag()] {
            assert!(
                !tag.contains(NOTES_HOSTILE),
                "{tag} would corrupt the CSV notes column"
            );
        }
    }

    /// The sidecar shape the DuckDB baseline dispatches on with `jq`
    /// alone — `.attr_filter.pred.eq` / `.attr_filter.pred.ge`.
    #[test]
    fn the_predicate_serialises_jq_dispatchably() {
        let eq = serde_json::to_value(AttrFilterPred::Eq("BB01".to_string())).unwrap();
        assert_eq!(eq["eq"], serde_json::json!("BB01"));
        let ge = serde_json::to_value(AttrFilterPred::Ge(2.45)).unwrap();
        assert_eq!(ge["ge"], serde_json::json!(2.45));
    }

    /// A package whose object table carries NO attribute column at all has
    /// no predicate to derive, and none is invented: `attr-filter` is
    /// skipped and the coordinator says so, exactly as `attr-stats` already
    /// is for a dataset with no numeric attribute. The table is never opened
    /// on this path, which is what lets this stay a unit test.
    #[test]
    fn a_dataset_with_no_attribute_column_yields_no_predicate() {
        let meta = CityMetadata::new();
        let schema = Schema::empty();
        let picked = pick_attr_filter(
            "a-dataset-outside-the-table.city.jsonl",
            &meta,
            &schema,
            Path::new("/nonexistent/table.parquet"),
        )
        .expect("an attribute-less package is not an error");
        assert_eq!(picked, None, "never fabricated");
    }

    /// The continuous ("linear") definition, so the published threshold is
    /// reproducible with one line of SQL (`quantile_cont`).
    #[test]
    fn the_quantile_interpolates_between_the_bracketing_order_statistics() {
        let mut values = vec![4.0, 1.0, 3.0, 2.0];
        // position = 0.75 * 3 = 2.25 -> values[2] + 0.25 * (values[3] - values[2])
        assert!((quantile_of(&mut values, 0.75) - 3.25).abs() < 1e-12);

        let mut exact = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        // position = 0.75 * 4 = 3.0, an order statistic exactly.
        assert!((quantile_of(&mut exact, 0.75) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn the_window_always_spans_the_datasets_full_z_range() {
        let boxes = grid(50, 50);
        for (target, tag) in BBOX_TARGETS {
            let w = window_for_target(&boxes, FIELD, target, tag);
            assert_eq!(w.window[2], FIELD[2], "{tag} must keep the dataset zmin");
            assert_eq!(w.window[5], FIELD[5], "{tag} must keep the dataset zmax");
        }
    }

    #[test]
    fn feature_probes_reuse_the_middle_feature_and_the_verified_miss() {
        let seq = ids(10);
        let verifiable: HashSet<String> = seq.iter().cloned().collect();
        let id_probes = id_probes(&seq, &verifiable);
        let features = feature_probes(&id_probes);
        let tags: Vec<&str> = features.iter().map(|p| p.tag.as_str()).collect();
        assert_eq!(tags, vec![FEATURE_50PCT_TAG, FEATURE_MISS_TAG]);
        let middle = id_probes.iter().find(|p| p.tag == "id-50pct").unwrap();
        assert_eq!(features[0].id, middle.id);
        assert!(features[0].present);
        assert!(!features[1].present);
        assert!(!seq.contains(&features[1].id));
    }
}
