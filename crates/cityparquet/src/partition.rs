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

use cjseq::{CityJSONFeature, Transform};

use crate::order::vertices_minmax;

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
                let part = i * n / total; // floor, 0..n-1
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
