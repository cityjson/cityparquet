//! Spatial / count partitioning of a merged dataset into N self-contained
//! CityParquet packages.
//!
//! [`assign_partitions`] is the pure core: it groups feature indices by a
//! partition-key label (which becomes the package subdirectory name) under the
//! chosen [`PartitionSpec`]. It never drops a feature — every index lands in
//! exactly one group. [`repair_reference_locality`] then pulls features that
//! reference each other onto a shared label, and [`convert_partitioned`] runs
//! the existing `convert_source` pipeline once per group, stamping one
//! canonical schema across all partitions so `read_parquet('OUT/*/…')` sees a
//! uniform layout.

use std::collections::{BTreeMap, HashMap};
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
    /// baseline). `N` is clamped to at most the feature count. Contiguous as
    /// assigned; [`repair_reference_locality`] may afterwards pull a feature
    /// out of its chunk to keep a hierarchy whole (non-conformant input only).
    Count(usize),
    /// At most `M` features per partition (contiguous chunks — with the same
    /// [`repair_reference_locality`] caveat as [`PartitionSpec::Count`], which
    /// is also the one thing that can push a partition past `M`).
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

/// What [`repair_reference_locality`] had to do, for the caller to report.
/// Both counts are `0` for conformant input, which is the overwhelming case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalityRepair {
    /// Features moved off their [`assign_partitions`] label so a hierarchy
    /// split across features would not be split across packages.
    pub co_assigned_features: usize,
    /// `parents`/`children` references whose target id is absent from the
    /// merged dataset entirely. Co-assignment cannot fix these — there is
    /// nothing to co-assign with — and they are NOT an error: a partial-area
    /// extract legitimately carries them.
    pub unresolvable_refs: usize,
}

/// A disjoint-set over feature indices, union by size with path halving.
struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            size: vec![1; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] += self.size[b];
    }
}

/// Pull every set of features joined by a `parents`/`children` reference onto
/// ONE label, so a city object and everything it points at always land in the
/// same package.
///
/// A conformant `CityJSONFeature` is a top-level object plus its children, so
/// [`assign_partitions`] already keeps hierarchies whole and this pass is a
/// no-op. It exists for the two shapes that break that assumption: a
/// CityJSONSeq file whose lines are not self-contained (non-conformant, but
/// silently produced by some writers), and — the legitimate case — a merge of
/// neighbouring tiles where a building's parts fall on the far side of a tile
/// boundary. Without it those partition into packages that each hold a
/// dangling reference.
///
/// Repair is at the LABEL level only: feature bodies are never merged, so
/// vertex pools and appearance indices are untouched. The objects stay
/// separate rows carrying their own `feature_id`, which is what `export`
/// regroups on, so a repaired package still exports as separate features. The
/// guarantee is that references resolve within the package — not that the
/// input's non-conformant feature split is rewritten.
///
/// Empty groups are dropped, so the returned labels are exactly the packages
/// to write. Ordering is preserved: groups stay sorted by label and each
/// group's indices stay ascending.
///
/// Two consequences of moving a feature, both confined to non-conformant
/// input. A `Box` label stops describing the centroid of everything filed
/// under it (and because `box_none` sorts first, a component holding one
/// vertexless feature adopts `box_none`) — harmless while labels are just
/// directory names and every package recomputes its own bbox from its own
/// features, but it would matter the day labels became spatial discovery
/// keys. And a `Features(M)` partition can exceed `M`: a fully chained input
/// collapses to a single package, however large.
pub fn repair_reference_locality(
    features: &[CityJSONFeature],
    groups: &mut Vec<(String, Vec<usize>)>,
) -> LocalityRepair {
    let mut repair = LocalityRepair::default();

    // Object id -> the feature that holds it. First occurrence wins if the
    // same OBJECT id appears in two features — which nothing else here
    // detects: `MergedDataset::duplicate_ids` counts duplicate *feature* ids
    // (`f.id`), a different thing. That input is pathological either way (the
    // package ends up with two rows for one object), and first-wins does not
    // make it worse: the union loop below walks every feature's objects, not
    // `owner`, so references FROM either copy are still joined. Only which
    // holder a referrer is pulled towards is affected, and both choices leave
    // the reference package-local.
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (i, f) in features.iter().enumerate() {
        for id in f.city_objects.keys() {
            owner.entry(id.as_str()).or_insert(i);
        }
    }

    let mut uf = UnionFind::new(features.len());
    for (i, f) in features.iter().enumerate() {
        for co in f.city_objects.values() {
            let refs = co
                .parents
                .iter()
                .chain(co.children.iter())
                .flat_map(|ids| ids.iter());
            for target in refs {
                match owner.get(target.as_str()) {
                    // A reference inside the same feature needs nothing.
                    Some(&j) if j != i => uf.union(i, j),
                    Some(_) => {}
                    None => repair.unresolvable_refs += 1,
                }
            }
        }
    }

    // Feature -> its current group. Every index appears in exactly one group
    // (`assign_partitions`' documented invariant), so this is total.
    let mut group_of: Vec<usize> = vec![0; features.len()];
    let mut assigned = vec![false; features.len()];
    for (g, (_, idxs)) in groups.iter().enumerate() {
        for &i in idxs {
            group_of[i] = g;
            assigned[i] = true;
        }
    }
    // Without this, a partition method that DROPPED an index would have the
    // feature silently resurrected into group 0 by the move below: complete
    // output, green tests, hidden bug — exactly what the locality tests exist
    // to prevent. Fail loudly in debug instead of papering over it.
    debug_assert!(
        assigned.iter().all(|&a| a),
        "assign_partitions must place every feature index in exactly one group"
    );

    // Each component adopts the lowest-indexed group any of its members sits
    // in. `groups` is sorted by label, so that is the alphabetically first
    // label of the component — deterministic, and independent of the order
    // the union-find happened to build the component in.
    let mut target_of_root: HashMap<usize, usize> = HashMap::new();
    for (i, &g) in group_of.iter().enumerate() {
        let root = uf.find(i);
        target_of_root
            .entry(root)
            .and_modify(|t| *t = (*t).min(g))
            .or_insert(g);
    }

    let mut moved: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for (i, &g) in group_of.iter().enumerate() {
        let target = target_of_root[&uf.find(i)];
        if target != g {
            repair.co_assigned_features += 1;
        }
        moved[target].push(i);
    }

    for (g, (_, idxs)) in groups.iter_mut().enumerate() {
        // `moved[g]` is built by ascending `i`, so it is already sorted.
        *idxs = std::mem::take(&mut moved[g]);
    }
    groups.retain(|(_, idxs)| !idxs.is_empty());

    repair
}

/// Outcome of a partitioned conversion: one [`ConvertReport`] per partition
/// package (keyed by its subdirectory label) plus the merge duplicate-id count
/// and the [`LocalityRepair`] counts.
#[derive(Debug, Clone)]
pub struct PartitionReport {
    pub partitions: Vec<(String, ConvertReport)>,
    pub duplicate_ids: usize,
    /// See [`LocalityRepair::co_assigned_features`].
    pub co_assigned_features: usize,
    /// See [`LocalityRepair::unresolvable_refs`].
    pub unresolvable_refs: usize,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
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
            CityParquetError::io_source(
                format!("cannot read output directory {}", dir.display()),
                e,
            )
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
        CityParquetError::io_source(
            format!("cannot create output directory {}", dir.display()),
            e,
        )
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
    // An unusable `--crs` is bad INPUT, so it must be rejected before
    // `ensure_parent_ready`/the purge below, not inside the per-partition
    // `convert_source_impl` that runs after it: a failure there would have
    // destroyed a prior complete output, the very invariant
    // [`ensure_parent_ready`] documents.
    if let Some(spec) = &opts.crs_override {
        crate::package::validate_crs_override(spec)?;
    }
    let merged = merge_sources(sources)?;
    // The provenance of the merged header's CRS: `merge_sources` enforces one
    // shared CRS across the inputs, so the merged header's CRS is the
    // operator's only if EVERY input's was — one input that declared that CRS
    // itself makes it source-declared, the operator's value having been a
    // no-op there. `from_parts` builds a fresh `Source`, so the fact has to be
    // carried over explicitly or every partition would write the operator's
    // CRS with no record of its origin.
    let crs_is_operator_supplied =
        !sources.is_empty() && sources.iter().all(Source::crs_is_operator_supplied);
    // Fail fast if the parent is non-empty without overwrite; the stale
    // partitions are only purged AFTER the scan below succeeds.
    let stale = ensure_parent_ready(&opts.output_dir, opts.overwrite)?;

    // Canonical schema: one scan over the entire merged dataset.
    let full = Source::from_parts(
        merged.header.clone(),
        merged.features.clone(),
        merged.doc_appearance.clone(),
        SourceFormat::CityJsonSeq,
    )
    .with_crs_operator_supplied(crs_is_operator_supplied);
    let mut full_scan = scan(&full, opts.geometry_encoding)?;
    if opts.generate_lod0 {
        // Reserve the synthesised LoD0 column on the whole-dataset scan so every
        // partition shares it (§9); per-partition synthesis is switched on via
        // `synthesize_lod0` in `convert_source_impl`.
        full_scan.add_synthesized_lod0_column();
    }
    let canonical = CanonicalSchema {
        schema: full_scan.schema.clone(),
        lods: full_scan.lods.clone(),
        module_lods: full_scan.module_lods.clone(),
        diverted_attribute_names: full_scan.diverted_attribute_names.clone(),
        geoparquet_columns: full_scan.geoparquet_columns.clone(),
        module_geo: full_scan.module_geo.clone(),
        crs: full_scan.crs.clone(),
        crs_diagnostic: full_scan.crs_diagnostic.clone(),
    };
    drop(full);

    // The scan (the likeliest early failure, e.g. bad geometry) has succeeded,
    // so it is now safe to purge the prior run's partitions. NOTE: the set of
    // partition packages is still not written atomically — a failure partway
    // through the sequential writes below leaves a partial replacement; re-run
    // to complete it (documented limitation for this research tool).
    for p in stale {
        fs::remove_dir_all(&p).map_err(|e| {
            CityParquetError::io_source(format!("cannot remove stale partition {}", p.display()), e)
        })?;
    }

    let mut groups = assign_partitions(&merged.features, spec, &merged.header.transform);
    let repair = repair_reference_locality(&merged.features, &mut groups);
    let mut partitions = Vec::with_capacity(groups.len());
    for (label, idxs) in groups {
        let subset: Vec<CityJSONFeature> =
            idxs.iter().map(|&i| merged.features[i].clone()).collect();
        let sub_source = Source::from_parts(
            merged.header.clone(),
            subset,
            merged.doc_appearance.clone(),
            SourceFormat::CityJsonSeq,
        )
        .with_crs_operator_supplied(crs_is_operator_supplied);
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
        co_assigned_features: repair.co_assigned_features,
        unresolvable_refs: repair.unresolvable_refs,
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

    /// Tear the real delft feature at `idx` into two features — its `Building`
    /// alone, then its `BuildingPart` alone — sharing the original's vertex
    /// pool. A mutation of real fixture data, not hand-written CityJSON.
    fn split_feature_in_two(f: &CityJSONFeature) -> (CityJSONFeature, CityJSONFeature) {
        let parent_id = f
            .city_objects
            .iter()
            .find(|(_, co)| co.thetype == "Building")
            .map(|(id, _)| id.clone())
            .expect("delft features are Building + BuildingPart");
        let child_id = f
            .city_objects
            .keys()
            .find(|id| **id != parent_id)
            .expect("delft features have a BuildingPart")
            .clone();

        let one = |id: &str| {
            let mut out = CityJSONFeature::new();
            out.id = id.to_string();
            out.add_co(id.to_string(), f.city_objects[id].clone());
            out.vertices = f.vertices.clone();
            out
        };
        (one(&parent_id), one(&child_id))
    }

    /// Conformant input must come out of the repair byte-identical: delft's
    /// features are each self-contained, so nothing may move and nothing may
    /// be counted.
    #[test]
    fn repair_is_a_noop_on_self_contained_features() {
        let (feats, t) = load_feats();
        let mut groups = assign_partitions(&feats, &PartitionSpec::Count(3), &t);
        let before = groups.clone();
        let repair = repair_reference_locality(&feats, &mut groups);
        assert_eq!(repair, LocalityRepair::default());
        assert_eq!(groups, before, "conformant input must not be perturbed");
    }

    /// Two features that reference each other collapse onto one label, and it
    /// is the alphabetically first of the labels they started in — so the
    /// outcome does not depend on union-find internals.
    #[test]
    fn mutually_referencing_features_collapse_onto_the_first_label() {
        let (feats, t) = load_feats();
        let (parent, child) = split_feature_in_two(&feats[0]);
        // Put the two halves at opposite ends so `Count(2)` really does split
        // them: [parent, ..other conformant features.., child].
        let mut torn = vec![parent];
        torn.extend(feats[1..5].iter().cloned());
        torn.push(child);

        let mut groups = assign_partitions(&torn, &PartitionSpec::Count(2), &t);
        assert_eq!(groups.len(), 2, "index assignment splits them");
        assert!(
            groups[0].1.contains(&0) && groups[1].1.contains(&(torn.len() - 1)),
            "the two halves start in different chunks"
        );

        let repair = repair_reference_locality(&torn, &mut groups);
        assert_eq!(repair.co_assigned_features, 1);
        assert_eq!(repair.unresolvable_refs, 0);
        let holder = groups
            .iter()
            .find(|(_, idxs)| idxs.contains(&0))
            .expect("the parent is still assigned");
        assert_eq!(
            holder.0, "count-00000",
            "the alphabetically first label wins"
        );
        assert!(
            holder.1.contains(&(torn.len() - 1)),
            "the child moved onto the parent's label"
        );
        // Every index still lands exactly once, and each group stays ascending.
        assert_eq!(all_indices(&groups), (0..torn.len()).collect::<Vec<_>>());
        for (_, idxs) in &groups {
            assert!(
                idxs.windows(2).all(|w| w[0] < w[1]),
                "indices stay ascending"
            );
        }
    }

    /// A reference to an id that is nowhere in the dataset is counted, not
    /// repaired and not fatal — and it must not drag the orphan onto some
    /// other feature's label.
    #[test]
    fn a_reference_to_a_missing_object_is_counted_only() {
        let (feats, t) = load_feats();
        let (parent, _child) = split_feature_in_two(&feats[0]);
        let mut torn = vec![parent];
        torn.extend(feats[1..5].iter().cloned());

        let mut groups = assign_partitions(&torn, &PartitionSpec::Count(2), &t);
        let before = groups.clone();
        let repair = repair_reference_locality(&torn, &mut groups);
        assert_eq!(
            repair.unresolvable_refs, 1,
            "the named BuildingPart is absent"
        );
        assert_eq!(repair.co_assigned_features, 0);
        assert_eq!(groups, before, "nothing to co-assign, so nothing moves");
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
