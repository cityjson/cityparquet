//! The configuration-axis packages (`<base>.<id>.parquet`, e.g. the Bloom
//! axis's `cityparquet+nobloom`), built from the prepared CityJSONSeq by ONE
//! code path whether preparation (`variant-package`) or a `--variants` run
//! builds them, so a hosted copy and a local build are the same bytes.
//!
//! A built package is stamped `.readbench-chain/<base>.<id>` with
//! [`CHAIN_VERSION`]; a run reuses a package whose stamp is current and
//! builds (and stamps) one that is absent or stale.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet::variant::Variant;

use crate::CORPUS_DATETIME;

/// `CHAIN_VERSION` of `readbench_prepare.sh`: the preparation chain a
/// variant package must have been built by to be reused.
pub const CHAIN_VERSION: u32 = 8;

pub fn package_path(prepared_dir: &Path, base: &str, id: &str) -> PathBuf {
    prepared_dir.join(format!("{base}.{id}.parquet"))
}

pub fn stamp_path(prepared_dir: &Path, base: &str, id: &str) -> PathBuf {
    prepared_dir
        .join(".readbench-chain")
        .join(format!("{base}.{id}"))
}

/// The package exists and its stamp names the current chain.
pub fn is_current(prepared_dir: &Path, base: &str, id: &str) -> bool {
    package_path(prepared_dir, base, id).is_dir()
        && fs::read_to_string(stamp_path(prepared_dir, base, id))
            .is_ok_and(|s| s.trim() == CHAIN_VERSION.to_string())
}

/// Build the package for `variant` from `seq`, replace any previous one, and
/// stamp it.
pub fn build(
    base: &str,
    id: &str,
    variant: &Variant,
    prepared_dir: &Path,
    seq: &Path,
) -> Result<PathBuf> {
    let scratch = tempfile::Builder::new()
        .prefix(&format!(".{base}.{id}.build."))
        .tempdir_in(prepared_dir)
        .with_context(|| format!("creating a scratch directory in {}", prepared_dir.display()))?;
    let built = scratch.path().join("pkg");
    let mut opts = ConvertOptions::new(seq.to_path_buf(), built.clone());
    opts.recipe = variant.recipe();
    opts.ordering = RowOrder::Hilbert;
    opts.generate_lod0 = false;
    opts.datetime = Some(CORPUS_DATETIME.to_string());
    convert(&opts).with_context(|| format!("converting with variant '{id}'"))?;

    let target = package_path(prepared_dir, base, id);
    let stamp = stamp_path(prepared_dir, base, id);
    let _ = fs::remove_file(&stamp);
    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("removing the previous {}", target.display()))?;
    }
    fs::rename(&built, &target)
        .with_context(|| format!("moving the built package to {}", target.display()))?;
    fs::create_dir_all(stamp.parent().expect("a stamp has a parent"))?;
    fs::write(&stamp, format!("{CHAIN_VERSION}\n"))?;
    Ok(target)
}

/// The current package for `variant`: reused when [`is_current`], built
/// otherwise. The flag says whether it was reused.
pub fn reuse_or_build(
    base: &str,
    id: &str,
    variant: &Variant,
    prepared_dir: &Path,
    seq: &Path,
) -> Result<(PathBuf, bool)> {
    if is_current(prepared_dir, base, id) {
        return Ok((package_path(prepared_dir, base, id), true));
    }
    Ok((build(base, id, variant, prepared_dir, seq)?, false))
}
