//! Every query parameter a read-benchmark measurement is driven with,
//! derived once per dataset from the prepared artefacts themselves — never
//! hardcoded, never fabricated.
//!
//! This lives in the library rather than beside the coordinator so the
//! choices it makes are testable without spawning a single child process:
//! [`area_window`] is a pure function over an in-memory slice, and the
//! integration tests reach the rest directly.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail};
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, DictionaryArray, Float64Array, RecordBatch, StringArray, StructArray,
};
use arrow_schema::{DataType, Schema};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet_schema::CityMetadata;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Opens `table` just far enough to read its embedded CityParquet key-value
/// metadata — the `attribute_columns` list `pick_numeric_attribute` chooses
/// from.
fn open_metadata(table: &Path) -> Result<CityMetadata> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet metadata from {}", table.display()))?;
    Ok(builder.cityparquet_metadata()?)
}

/// `table`'s Arrow schema — the types `pick_numeric_attribute` filters on.
fn open_arrow_schema(table: &Path) -> Result<Schema> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet schema from {}", table.display()))?;
    Ok((*builder.cityparquet_arrow_schema()?).clone())
}

/// Every row's bbox, plus their union.
///
/// The whole vector is kept — not just the union — because
/// [`area_window`] reports the fraction of ROWS its area window actually
/// intersects, which cannot be answered from the extent alone. One
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

/// `(fraction of the dataset's x/y area, notes tag)` for the three bbox
/// windows — one CSV row per entry.
///
/// The fraction is of the dataset's x/y AREA, anchored at the lower-left
/// corner: the same construction `benchmark/databases`'s `BBox.window` uses,
/// so the tags mean the same thing in both harnesses. How many ROWS such a
/// window selects is a property of the data, not of the tag; the realised
/// fraction is measured ([`BboxWindow::achieved`]) and reported next to the
/// tag rather than assumed.
pub const BBOX_TARGETS: [(f64, &str); 3] = [
    (0.01, "bbox-1pct"),
    (0.05, "bbox-5pct"),
    (0.25, "bbox-25pct"),
];

/// One resolved bbox window: which area fraction it covers, what fraction of
/// rows it actually intersects, and the window itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BboxWindow {
    /// The `notes` tag this window's CSV rows carry, e.g. `bbox-1pct`.
    pub tag: String,
    /// The fraction of the dataset's x/y AREA the window covers.
    pub target: f64,
    /// The fraction of rows whose bbox intersects the window. Data-dependent:
    /// a 1% area window need not select 1% of the objects.
    pub achieved: f64,
    /// `[minx, miny, minz, maxx, maxy, maxz]`.
    pub window: [f64; 6],
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

/// The lower-left window covering `area_fraction` of `dataset`'s x/y extent,
/// with the fraction of `boxes` it intersects.
///
/// `side = sqrt(area_fraction)` of each x/y span, anchored at the dataset's
/// lower-left corner — exactly the construction `benchmark/databases`'s
/// `BBox.window` uses, so `bbox-1pct` names the same window in both
/// harnesses. The z range is never narrowed, so every row is in range
/// vertically. The realised row fraction is measured ([`BboxWindow::achieved`]),
/// never targeted.
pub fn area_window(
    boxes: &[[f64; 6]],
    dataset: [f64; 6],
    area_fraction: f64,
    tag: &str,
) -> BboxWindow {
    assert!(
        area_fraction > 0.0 && area_fraction <= 1.0,
        "area_fraction must be in (0, 1]"
    );
    let side = area_fraction.sqrt();
    let window = [
        dataset[0],
        dataset[1],
        dataset[2],
        dataset[0] + (dataset[3] - dataset[0]) * side,
        dataset[1] + (dataset[4] - dataset[1]) * side,
        dataset[5],
    ];
    let total = boxes.len();
    let hit = boxes.iter().filter(|row| intersects(row, &window)).count();
    let achieved = if total == 0 {
        0.0
    } else {
        hit as f64 / total as f64
    };
    BboxWindow {
        tag: tag.to_string(),
        target: area_fraction,
        achieved,
        window,
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
/// Utf8>` array. The reserved `object_type` column is ALWAYS
/// `Dictionary<Int32, Utf8>` per `cityparquet_schema::model`'s own schema
/// (never plain `Utf8`), but this accepts either shape rather than assuming
/// one, mirroring `cityparquet::query::evaluate_attr_predicate`'s own
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

/// The most-frequent `object_type` value in `table` (and its count) — a
/// single-column projected scan, tallied in memory (the reserved
/// `object_type` column is always present, so this never needs the
/// attribute-column machinery). Ties are broken deterministically by the
/// `object_type` string itself (rather than `HashMap` iteration order, which
/// is SipHash-randomised per process) so the derived `AttrFilter` predicate —
/// and therefore the whole run — is reproducible run-to-run.
fn most_frequent_object_type(table: &Path) -> Result<(String, u64)> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["object_type"]);
    let reader = builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning object_type column of {}", table.display()))?;

    let mut counts: HashMap<String, u64> = HashMap::new();
    for batch in reader {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        for value in utf8_values(batch.column(0).as_ref())?.into_iter().flatten() {
            *counts.entry(value).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .ok_or_else(|| anyhow::anyhow!("{} has no object_type values", table.display()))
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
    /// `feature-50pct` and `feature-miss`, from [`feature_probes`]; EMPTY
    /// exactly when `id_probes` is.
    pub feature_probes: Vec<IdProbe>,
    /// The most-frequent `object_type` value — the `attr-filter` predicate.
    pub object_type: String,
    pub object_type_count: u64,
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
        .map(|(target, tag)| area_window(&rows.boxes, rows.dataset, *target, tag))
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
    let (object_type, object_type_count) = most_frequent_object_type(cp_table)?;
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
        feature_probes,
        object_type,
        object_type_count,
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
    fn the_window_is_the_lower_left_area_fraction() {
        let boxes = grid(100, 100); // uniform, so area fraction ~ row fraction
        for (target, tag) in BBOX_TARGETS {
            let w = area_window(&boxes, FIELD, target, tag);
            assert_eq!(w.window[0], FIELD[0], "{tag}: anchored at xmin");
            assert_eq!(w.window[1], FIELD[1], "{tag}: anchored at ymin");
            let side = target.sqrt();
            assert!(
                (w.window[3] - (FIELD[0] + (FIELD[3] - FIELD[0]) * side)).abs() < 1e-9,
                "{tag}: x extent must be span * sqrt(target)"
            );
            assert!(
                (w.window[4] - (FIELD[1] + (FIELD[4] - FIELD[1]) * side)).abs() < 1e-9,
                "{tag}: y extent must be span * sqrt(target)"
            );
            assert!(
                (w.achieved - target).abs() <= 0.05,
                "{tag}: on a uniform grid the realised row fraction {} should be near {target}",
                w.achieved
            );
        }
    }

    #[test]
    fn never_returns_an_empty_window() {
        let boxes = grid(100, 100);
        for (target, tag) in BBOX_TARGETS {
            let w = area_window(&boxes, FIELD, target, tag);
            assert!(w.achieved > 0.0, "{tag} selected no rows at all");
        }
    }

    /// The realised row fraction is a property of the data, not the tag: a
    /// dataset clustered into the lower-left corner has a 1% area window that
    /// selects nearly every row. The window itself is still the lower-left
    /// area window.
    #[test]
    fn reports_the_realised_row_fraction_of_a_clustered_dataset() {
        let mut boxes = Vec::new();
        for i in 0..1000 {
            let x = (i % 10) as f64 * 0.01;
            let y = ((i / 10) % 10) as f64 * 0.01;
            boxes.push([x, y, 0.0, x + 0.001, y + 0.001, 1.0]);
        }
        let w = area_window(&boxes, FIELD, 0.01, "bbox-1pct");
        assert!(
            w.achieved > 0.9,
            "a 1% area window over a lower-left cluster should select nearly every row, got {}",
            w.achieved
        );
    }

    /// The window scales with each axis's own span, so a long, thin dataset
    /// gets a long, thin window rather than a square one.
    #[test]
    fn scales_the_window_to_the_datasets_aspect_ratio() {
        let boxes = grid(10, 1000);
        let thin: [f64; 6] = [0.0, 0.0, 0.0, 1000.0, 1.0, 10.0];
        let w = area_window(&boxes, thin, 0.25, "bbox-25pct");
        let side = 0.25_f64.sqrt();
        assert!((w.window[3] - 1000.0 * side).abs() < 1e-6, "{w:?}");
        assert!((w.window[4] - 1.0 * side).abs() < 1e-6, "{w:?}");
    }

    #[test]
    fn the_window_always_spans_the_datasets_full_z_range() {
        let boxes = grid(50, 50);
        for (target, tag) in BBOX_TARGETS {
            let w = area_window(&boxes, FIELD, target, tag);
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
