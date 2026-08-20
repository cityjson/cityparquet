//! Cut fixed-cardinality CityJSONSeq prefixes ("scaling slices") out of one
//! source feature stream.
//!
//! The configuration-axis benchmarks (`just compression-bench`,
//! `just ordering-bench`) vary an encoding parameter — codec, row-group
//! size, row ordering — and want the DATASET axis held still. A corpus of
//! unrelated city models (3DBAG next to PLATEAU next to Vienna) confounds
//! that: every configuration delta is entangled with a data delta. A
//! scaling corpus instead takes ONE source and cuts it at several
//! CityObject cardinalities, so a measurement series shows the trend over
//! size with the data held constant — every slice is a strict prefix of
//! the next larger one, in source feature order.
//!
//! The slice boundary is the FEATURE, not the CityObject: a CityJSONSeq
//! feature is indivisible (one top-level CityObject plus all its children
//! in one line), so a slice takes whole features until its CityObject
//! count first reaches the target. The crossing feature is included, which
//! means a slice may hold slightly MORE CityObjects than its nominal
//! target; the exact count is in the returned [`SliceSummary`] and on
//! stdout, and the nominal target only names the file. A target the source
//! cannot reach is an ERROR, not a silently short file — a
//! `<stem>_n50000` slice holding 30k objects would be a lie in a
//! measurement corpus. Slices that did complete are kept.
//!
//! This module is deliberately ignorant of FlatCityBuf: it consumes any
//! pull source of `(serialised feature line, CityObject count)` pairs, so
//! its tests need no fixture and no network. The `scaling-corpus` binary
//! (`src/bin/scaling_corpus.rs`) is the FCB adapter.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// What one completed slice actually holds — the exact counts, not the
/// nominal target that names the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceSummary {
    /// The nominal CityObject target this slice was cut for.
    pub target: usize,
    /// Whole features written (excluding the header line).
    pub features: usize,
    /// CityObjects actually contained; `>= target`, and strictly less than
    /// `target` plus the crossing feature's own CityObject count.
    pub city_objects: usize,
    /// The finalised file, `<out_dir>/<stem>_n<target>.city.jsonl`.
    pub path: PathBuf,
}

/// One in-flight slice file. Written as `<final>.tmp` and renamed on
/// completion, so an interrupted run never leaves a plausible-looking
/// partial slice behind.
struct SliceWriter {
    target: usize,
    features: usize,
    city_objects: usize,
    tmp: PathBuf,
    path: PathBuf,
    out: BufWriter<File>,
}

impl SliceWriter {
    fn open(out_dir: &Path, stem: &str, target: usize, header_line: &str) -> Result<Self> {
        let path = out_dir.join(format!("{stem}_n{target}.city.jsonl"));
        let tmp = out_dir.join(format!("{stem}_n{target}.city.jsonl.tmp"));
        let file =
            File::create(&tmp).with_context(|| format!("creating slice file {}", tmp.display()))?;
        let mut out = BufWriter::new(file);
        writeln!(out, "{header_line}")
            .with_context(|| format!("writing header to {}", tmp.display()))?;
        Ok(Self {
            target,
            features: 0,
            city_objects: 0,
            tmp,
            path,
            out,
        })
    }

    fn push(&mut self, feature_line: &str, city_objects: usize) -> Result<()> {
        writeln!(self.out, "{feature_line}")
            .with_context(|| format!("writing feature to {}", self.tmp.display()))?;
        self.features += 1;
        self.city_objects += city_objects;
        Ok(())
    }

    fn done(&self) -> bool {
        self.city_objects >= self.target
    }

    fn finalise(mut self) -> Result<SliceSummary> {
        self.out
            .flush()
            .with_context(|| format!("flushing {}", self.tmp.display()))?;
        drop(self.out);
        fs::rename(&self.tmp, &self.path).with_context(|| {
            format!("renaming {} -> {}", self.tmp.display(), self.path.display())
        })?;
        Ok(SliceSummary {
            target: self.target,
            features: self.features,
            city_objects: self.city_objects,
            path: self.path,
        })
    }

    /// Remove the `.tmp` without finalising — the unreachable-target path.
    fn abandon(self) -> PathBuf {
        drop(self.out);
        let _ = fs::remove_file(&self.tmp);
        self.path
    }
}

/// Cut one slice per target out of `next_feature`, a pull source yielding
/// `Ok(Some((serialised feature line, CityObject count)))` per feature in
/// source order and `Ok(None)` at end of stream.
///
/// Targets are deduplicated and sorted; zero targets and an empty target
/// list are rejected. Every still-open slice receives every feature, so
/// each slice is a strict prefix of the next larger one; the source is
/// only pulled until the largest target completes. Returns one
/// [`SliceSummary`] per target, ascending. If the stream ends before every
/// target is reached, completed slices are kept, unreachable slices'
/// `.tmp` files are removed, and the error names what was dropped.
pub fn write_scaling_slices(
    header_line: &str,
    mut next_feature: impl FnMut() -> Result<Option<(String, usize)>>,
    targets: &[usize],
    out_dir: &Path,
    stem: &str,
) -> Result<Vec<SliceSummary>> {
    let mut sizes: Vec<usize> = targets.to_vec();
    sizes.sort_unstable();
    sizes.dedup();
    if sizes.is_empty() {
        bail!("scaling: no slice sizes given");
    }
    if sizes.first() == Some(&0) {
        bail!("scaling: a slice size of 0 is meaningless");
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output directory {}", out_dir.display()))?;

    let mut open: Vec<SliceWriter> = sizes
        .iter()
        .map(|&target| SliceWriter::open(out_dir, stem, target, header_line))
        .collect::<Result<_>>()?;
    let mut finished: Vec<SliceSummary> = Vec::with_capacity(sizes.len());

    while !open.is_empty() {
        let Some((line, city_objects)) = next_feature()? else {
            break;
        };
        for writer in &mut open {
            writer.push(&line, city_objects)?;
        }
        // Ascending order means completed writers are always a prefix.
        while open.first().is_some_and(SliceWriter::done) {
            finished.push(open.remove(0).finalise()?);
        }
    }

    if !open.is_empty() {
        let dropped: Vec<String> = open
            .drain(..)
            .map(|w| {
                let target = w.target;
                let short = w.city_objects;
                w.abandon();
                format!("n{target} (source ended at {short} CityObjects)")
            })
            .collect();
        bail!(
            "scaling: source has too few CityObjects for: {} — completed slices were kept, \
             the short ones were not written",
            dropped.join(", ")
        );
    }

    Ok(finished)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pull source over synthetic features: feature `i` is the line
    /// `F<i>` and carries `counts[i]` CityObjects.
    fn source(counts: &[usize]) -> impl FnMut() -> Result<Option<(String, usize)>> {
        let counts = counts.to_vec();
        let mut i = 0;
        move || {
            let item = counts.get(i).map(|&c| (format!("F{i}"), c));
            i += 1;
            Ok(item)
        }
    }

    fn lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn slices_are_prefixes_and_include_the_crossing_feature() {
        let dir = tempfile::tempdir().unwrap();
        let got =
            write_scaling_slices("H", source(&[2, 2, 2, 2, 2]), &[5, 3], dir.path(), "x").unwrap();

        // Ascending, whole features, crossing feature included: n3 takes
        // features 0..=1 (4 objects), n5 takes 0..=2 (6 objects).
        assert_eq!(
            got,
            vec![
                SliceSummary {
                    target: 3,
                    features: 2,
                    city_objects: 4,
                    path: dir.path().join("x_n3.city.jsonl"),
                },
                SliceSummary {
                    target: 5,
                    features: 3,
                    city_objects: 6,
                    path: dir.path().join("x_n5.city.jsonl"),
                },
            ]
        );
        assert_eq!(lines(&got[0].path), ["H", "F0", "F1"]);
        assert_eq!(lines(&got[1].path), ["H", "F0", "F1", "F2"]);
        // No stray .tmp files survive.
        assert!(!dir.path().join("x_n3.city.jsonl.tmp").exists());
        assert!(!dir.path().join("x_n5.city.jsonl.tmp").exists());
    }

    #[test]
    fn source_is_not_pulled_past_the_largest_target() {
        let dir = tempfile::tempdir().unwrap();
        let counts = [1usize; 4];
        let mut pulls = 0usize;
        let mut i = 0usize;
        let got = write_scaling_slices(
            "H",
            || {
                pulls += 1;
                let item = counts.get(i).map(|&c| (format!("F{i}"), c));
                i += 1;
                Ok(item)
            },
            &[2],
            dir.path(),
            "x",
        )
        .unwrap();
        assert_eq!(got[0].features, 2);
        // Two features complete the only target; the loop stops without a
        // third pull.
        assert_eq!(pulls, 2);
    }

    #[test]
    fn unreachable_target_errors_but_keeps_completed_slices() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            write_scaling_slices("H", source(&[2, 2]), &[3, 100], dir.path(), "x").unwrap_err();
        assert!(err.to_string().contains("n100"), "unexpected error: {err}");
        assert!(
            err.to_string().contains("4 CityObjects"),
            "unexpected error: {err}"
        );
        // n3 completed before the stream ended and is kept …
        assert_eq!(
            lines(&dir.path().join("x_n3.city.jsonl")),
            ["H", "F0", "F1"]
        );
        // … while n100 leaves neither a final nor a .tmp file behind.
        assert!(!dir.path().join("x_n100.city.jsonl").exists());
        assert!(!dir.path().join("x_n100.city.jsonl.tmp").exists());
    }

    #[test]
    fn duplicate_targets_collapse_and_zero_or_empty_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let got = write_scaling_slices("H", source(&[3, 3]), &[2, 2], dir.path(), "x").unwrap();
        assert_eq!(got.len(), 1);

        assert!(write_scaling_slices("H", source(&[1]), &[], dir.path(), "y").is_err());
        assert!(write_scaling_slices("H", source(&[1]), &[0, 2], dir.path(), "z").is_err());
    }

    #[test]
    fn source_errors_propagate() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            write_scaling_slices("H", || bail!("stream broke"), &[1], dir.path(), "x").unwrap_err();
        assert!(err.to_string().contains("stream broke"));
    }
}
