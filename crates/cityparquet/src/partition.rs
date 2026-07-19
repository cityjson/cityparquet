//! Spatial / count partitioning of a merged dataset into N self-contained
//! CityParquet packages.
//!
//! [`assign_partitions`] is the pure core: it groups feature indices by a
//! partition-key label (which becomes the package subdirectory name) under the
//! chosen [`PartitionSpec`]. It never drops a feature — every index lands in
//! exactly one group. [`convert_partitioned`] then runs the existing
//! `convert_source` pipeline once per group, stamping one canonical schema
//! across all partitions so `read_parquet('OUT/*/…')` sees a uniform layout.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cjseq::{CityJSONFeature, Transform};

use cityparquet_schema::{CityParquetError, Result};

use crate::merge::merge_sources;
use crate::order::vertices_minmax;
use crate::package::{CanonicalSchema, ConvertOptions, ConvertReport, convert_source_impl};
use crate::scan::scan;
use crate::source::{Source, SourceFormat};

/// How to split the merged dataset. Sizing is method-specific (no forced
/// "exactly N files" on the spatial method).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PartitionSpec {
    /// `N` contiguous chunks over the merged feature order (non-spatial
    /// baseline). `N` is clamped to at most the feature count.
    Count(usize),
    /// At most `M` features per partition (contiguous chunks).
    Features(usize),
    /// A square grid of `cell`-metre cells over feature centroids.
    Box { cell: f64 },
}

/// Group feature indices by partition-key label, sorted by label. The label is
/// the package subdirectory name. Every index in `0..features.len()` appears in
/// exactly one group.
pub fn assign_partitions(
    features: &[CityJSONFeature],
    spec: &PartitionSpec,
    transform: &Transform,
) -> Vec<(String, Vec<usize>)> {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let total = features.len();
    match spec {
        PartitionSpec::Count(n) => {
            // Clamp so N > total yields exactly `total` singleton partitions
            // rather than fewer, gap-labelled ones.
            let n = (*n).clamp(1, total.max(1));
            for i in 0..total {
                // u64 math so `i * n` cannot overflow usize on 32-bit targets.
                let part = ((i as u64 * n as u64) / total as u64) as usize; // floor, 0..n-1
                groups
                    .entry(format!("count-{part:05}"))
                    .or_default()
                    .push(i);
            }
        }
        PartitionSpec::Features(m) => {
            let m = (*m).max(1);
            for i in 0..total {
                let part = i / m;
                groups
                    .entry(format!("features-{part:05}"))
                    .or_default()
                    .push(i);
            }
        }
        PartitionSpec::Box { cell } => {
            let cell = *cell;
            for (i, f) in features.iter().enumerate() {
                let label = match vertices_minmax(&f.vertices, transform) {
                    Some((min, max)) => {
                        let cx = (min[0] + max[0]) / 2.0;
                        let cy = (min[1] + max[1]) / 2.0;
                        let ix = (cx / cell).floor() as i64;
                        let iy = (cy / cell).floor() as i64;
                        format!("box_x{ix}_y{iy}")
                    }
                    None => "box_none".to_string(),
                };
                groups.entry(label).or_default().push(i);
            }
        }
    }
    groups.into_iter().collect()
}

/// Outcome of a partitioned conversion: one [`ConvertReport`] per partition
/// package (keyed by its subdirectory label) plus the merge duplicate-id count.
#[derive(Debug, Clone)]
pub struct PartitionReport {
    pub partitions: Vec<(String, ConvertReport)>,
    pub duplicate_ids: usize,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

fn io_err(msg: String) -> CityParquetError {
    CityParquetError::Io(msg)
}

/// True only for the EXACT subdirectory-name shapes this driver produces
/// (`count-<digits>`, `features-<digits>`, `box_x<int>_y<int>`, `box_none`), so
/// overwrite never deletes an unrelated directory that merely shares a prefix
/// (e.g. `box-office`, `box_xylophone`, `counter`).
fn is_partition_dir_name(name: &str) -> bool {
    /// A signed integer with no leading `+` (matches `format!("{i}")` for i64).
    fn is_int(s: &str) -> bool {
        let digits = s.strip_prefix('-').unwrap_or(s);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    }
    fn indexed(prefix: &str, name: &str) -> bool {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    }
    fn is_box(name: &str) -> bool {
        // box_x<int>_y<int> — parse both components, don't prefix-match.
        name.strip_prefix("box_x")
            .and_then(|rest| rest.split_once("_y"))
            .is_some_and(|(x, y)| is_int(x) && is_int(y))
    }
    name == "box_none" || is_box(name) || indexed("count-", name) || indexed("features-", name)
}

/// The partition subdirectories currently under `dir` (its own prior output).
fn existing_partition_dirs(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    Ok(fs::read_dir(dir)
        .map_err(|e| {
            io_err(format!(
                "cannot read output directory {}: {e}",
                dir.display()
            ))
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_partition_dir_name)
        })
        .collect())
}

/// Create the parent dir and fail fast if it already holds partition packages
/// and `overwrite` is off. Returns the stale partition subdirectories to purge
/// LATER (after the canonical scan succeeds — see [`convert_partitioned`]), so
/// a bad-input failure never destroys a prior complete output before any new
/// work has succeeded.
fn ensure_parent_ready(dir: &Path, overwrite: bool) -> Result<Vec<std::path::PathBuf>> {
    fs::create_dir_all(dir).map_err(|e| {
        io_err(format!(
            "cannot create output directory {}: {e}",
            dir.display()
        ))
    })?;
    let existing = existing_partition_dirs(dir)?;
    if !existing.is_empty() && !overwrite {
        return Err(err(format!(
            "output directory {} already holds partition packages (pass overwrite to replace them)",
            dir.display()
        )));
    }
    Ok(existing)
}

/// Merge `sources`, partition the merged features by `spec`, and write one
/// self-contained CityParquet package per non-empty partition into
/// `opts.output_dir/<label>/`. All partitions share ONE canonical schema
/// (computed from a single scan of the whole merged dataset) so a
/// `read_parquet('OUT/*/…')` glob sees a uniform column layout; each partition
/// still scans its own features for its own bbox/count/stats.
///
/// Memory: like `--ordering hilbert`, this buffers every feature (and clones
/// each partition's subset before encoding) — the documented full-load cost.
pub fn convert_partitioned(
    sources: &[Source],
    spec: &PartitionSpec,
    opts: &ConvertOptions,
) -> Result<PartitionReport> {
    let merged = merge_sources(sources)?;
    // Fail fast if the parent is non-empty without overwrite; the stale
    // partitions are only purged AFTER the scan below succeeds.
    let stale = ensure_parent_ready(&opts.output_dir, opts.overwrite)?;

    // Canonical schema: one scan over the entire merged dataset.
    let full = Source::from_parts(
        merged.header.clone(),
        merged.features.clone(),
        merged.doc_appearance.clone(),
        SourceFormat::CityJsonSeq,
    );
    let mut full_scan = scan(&full)?;
    if opts.generate_lod0 {
        // Reserve the synthesised LoD0 column on the whole-dataset scan so every
        // partition shares it (§9); per-partition synthesis is switched on via
        // `synthesize_lod0` in `convert_source_impl`.
        full_scan.add_synthesized_lod0_column();
    }
    let canonical = CanonicalSchema {
        schema: full_scan.schema.clone(),
        lods: full_scan.lods.clone(),
        diverted_attribute_names: full_scan.diverted_attribute_names.clone(),
    };
    drop(full);

    // The scan (the likeliest early failure, e.g. bad geometry) has succeeded,
    // so it is now safe to purge the prior run's partitions. NOTE: the set of
    // partition packages is still not written atomically — a failure partway
    // through the sequential writes below leaves a partial replacement; re-run
    // to complete it (documented limitation for this research tool).
    for p in stale {
        fs::remove_dir_all(&p).map_err(|e| {
            io_err(format!(
                "cannot remove stale partition {}: {e}",
                p.display()
            ))
        })?;
    }

    let groups = assign_partitions(&merged.features, spec, &merged.header.transform);
    let mut partitions = Vec::with_capacity(groups.len());
    for (label, idxs) in groups {
        let subset: Vec<CityJSONFeature> =
            idxs.iter().map(|&i| merged.features[i].clone()).collect();
        let sub_source = Source::from_parts(
            merged.header.clone(),
            subset,
            merged.doc_appearance.clone(),
            SourceFormat::CityJsonSeq,
        );
        let mut sub_opts = opts.clone();
        sub_opts.output_dir = opts.output_dir.join(&label);
        // The parent was prepared above; each partition subdir is fresh, so the
        // per-package overwrite check must not trip on a sibling.
        sub_opts.overwrite = true;
        let report = convert_source_impl(&sub_source, &sub_opts, Some(&canonical))?;
        partitions.push((label, report));
    }

    Ok(PartitionReport {
        partitions,
        duplicate_ids: merged.duplicate_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_feats() -> (Vec<CityJSONFeature>, Transform) {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/delft.city.jsonl");
        let s = crate::source::Source::open(&p).unwrap();
        (
            s.features().unwrap().map(|f| f.unwrap()).collect(),
            s.header().transform.clone(),
        )
    }

    fn all_indices(groups: &[(String, Vec<usize>)]) -> Vec<usize> {
        let mut all: Vec<usize> = groups.iter().flat_map(|(_, v)| v.clone()).collect();
        all.sort_unstable();
        all
    }

    #[test]
    fn count_splits_into_contiguous_chunks_covering_all() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Count(3), &t);
        assert_eq!(groups.len(), 3);
        assert_eq!(all_indices(&groups), (0..feats.len()).collect::<Vec<_>>());
        // Chunks are contiguous: each group's indices are a consecutive run.
        for (_, v) in &groups {
            assert!(v.windows(2).all(|w| w[1] == w[0] + 1), "not contiguous");
        }
    }

    #[test]
    fn count_clamps_when_n_exceeds_total() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Count(feats.len() + 100), &t);
        assert_eq!(groups.len(), feats.len(), "clamped to one feature each");
    }

    #[test]
    fn features_caps_each_group() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Features(1000), &t);
        assert!(groups.iter().all(|(_, v)| v.len() <= 1000));
        assert_eq!(all_indices(&groups), (0..feats.len()).collect::<Vec<_>>());
    }

    #[test]
    fn partition_dir_name_matches_only_exact_labels() {
        // Real labels this driver emits.
        for good in [
            "count-00000",
            "features-00042",
            "box_x3_y7",
            "box_x-3_y-5",
            "box_none",
        ] {
            assert!(is_partition_dir_name(good), "{good} should match");
        }
        // Prefix collisions that must NOT be treated as our output.
        for bad in [
            "box-office",
            "box_xylophone",
            "box_x3",
            "box_x_y5",
            "box_x3_yfoo",
            "counter",
            "count-",
            "count-1a",
            "features_backup",
        ] {
            assert!(!is_partition_dir_name(bad), "{bad} must NOT match");
        }
    }

    #[test]
    fn box_groups_are_disjoint_and_complete() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Box { cell: 500.0 }, &t);
        assert!(groups.len() > 1, "delft should span >1 500m cell");
        assert_eq!(all_indices(&groups), (0..feats.len()).collect::<Vec<_>>());
        assert!(
            groups.iter().all(|(label, _)| label.starts_with("box_")),
            "box labels"
        );
    }
}
