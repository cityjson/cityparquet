//! STAC assets describing the files a CityParquet package actually contains.
//!
//! Every asset here is built from a file on disk, so an Item can never claim
//! a file the package does not have.

use std::fs;
use std::path::Path;

use city3d_stac_types::checksum::file_checksum;

/// IANA media type for Parquet.
pub const PARQUET_MEDIA_TYPE: &str = "application/vnd.apache.parquet";

/// What a package file is, for STAC asset roles.
///
/// Plan 2b turns these into the binding `export` uses to find its sidecars,
/// replacing the manifest's `sidecar_files` list. They are introduced here
/// only as descriptive roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// An object table (§5).
    ObjectTable,
    /// A sidecar: materials, textures or geometry templates (§11, §12).
    Sidecar,
}

impl AssetKind {
    /// STAC asset roles for this kind.
    ///
    /// `data` is the conventional STAC role; the `cityparquet-` prefixed role
    /// is what distinguishes an object table from a sidecar without parsing
    /// filenames.
    pub fn roles(self) -> Vec<String> {
        let specific = match self {
            AssetKind::ObjectTable => "cityparquet-objects",
            AssetKind::Sidecar => "cityparquet-sidecar",
        };
        vec!["data".to_string(), specific.to_string()]
    }
}

/// The size and checksum of a package file, for the STAC File extension.
///
/// Both are `None` when the file cannot be read; the caller decides whether
/// that is an error. `file:checksum` is a content-derived hex multihash of the
/// file's SHA-256 digest.
pub fn file_facts(path: &Path) -> (Option<u64>, Option<String>) {
    let size = fs::metadata(path).ok().map(|m| m.len());
    (size, file_checksum(path))
}

/// The asset key for a package file: its file name, so a reader can map an
/// asset back to a file without guessing.
pub fn asset_key(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data".to_string())
}
