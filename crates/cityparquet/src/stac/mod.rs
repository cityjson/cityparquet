//! Derive a STAC Item from a written CityParquet package.
//!
//! Every field is read back from the files on disk — the Parquet footer, the
//! Arrow schema, and the dictionary-encoded `object_type` column. Nothing is
//! accumulated during conversion. This makes spec §13.2's rule ("where the
//! STAC Item and the Parquet footer disagree, the footer is authoritative")
//! true by construction rather than by discipline, and means a package written
//! by any conformant writer can be described, not just this crate's own.

pub mod assets;
pub mod attribute_type;
pub mod properties;

use std::fs;
use std::path::Path;

use city3d_stac_types::metadata::{BBox3D, CRS};
use city3d_stac_types::stac::StacItemBuilder;
use city3d_stac_types::stac::types::{Asset, Item};
use cityparquet_schema::{CityParquetError, Result};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::RowGroupMetaData;
use parquet::file::statistics::Statistics;
use parquet::schema::types::ColumnPath;
use serde_json::Value;

use crate::reader::CityParquetReaderBuilder;
use crate::stac::properties::PackageTables;

/// How to fill the parts of an Item that a package cannot supply itself.
#[derive(Debug, Clone, Default)]
pub struct ItemOptions {
    /// Item id. Defaults to the package directory's name.
    pub id: Option<String>,
    /// RFC 3339 timestamp for `properties.datetime`.
    ///
    /// **There is no fallback here.** A CityJSON source rarely carries
    /// temporal metadata — none of this repo's fixtures does — so a fallback
    /// would govern almost every package rather than an edge case. Per the
    /// design decision of 2026-07-20, the CLI stamps the conversion time and
    /// the *library* requires an explicit value; this is the library, so an
    /// absent value simply leaves `datetime` unset rather than inventing one.
    pub datetime: Option<String>,
}

/// Build a STAC Item describing a written CityParquet package.
///
/// Reads the package back rather than relying on anything remembered from
/// conversion, so the Item cannot drift from the files it describes.
///
/// Fails rather than guessing when the extent cannot be expressed in WGS84:
/// STAC requires `bbox`/`geometry` in WGS84, and a silently wrong extent is
/// worse than a failed conversion.
pub fn item_for_package(dir: &Path, opts: &ItemOptions) -> Result<Item> {
    let tables = PackageTables::open(dir)?;
    let props = properties::derive_from_footer(&tables)?;
    let crs = package_crs(&tables)?;

    let id = opts.id.clone().unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cityparquet".to_string())
    });

    let mut builder = StacItemBuilder::new(id)
        .datetime(opts.datetime.clone())
        .city3d(props)
        .map_err(|e| CityParquetError::Metadata(format!("cannot build STAC item: {e}")))?;

    if let Some(crs) = &crs {
        builder = builder.crs(crs);
    }

    // STAC requires WGS84; the package extent is in the source CRS.
    if let Some(bbox) = package_bbox(&tables)? {
        let wgs84 = bbox
            .to_wgs84(crs.as_ref().unwrap_or(&CRS::unknown()))
            .map_err(|e| {
                CityParquetError::Metadata(format!(
                    "cannot express the package extent in WGS84, which STAC requires: {e}"
                ))
            })?;
        builder = builder.bbox(wgs84).geometry_from_bbox();
    }

    // The primary object table goes through `data_asset`, which is also what
    // declares the STAC File extension; every other file is added by name.
    let mut first = true;
    for path in &tables.tables {
        let (size, checksum) = assets::file_facts(path);
        let href = format!("./{}", assets::asset_key(path));
        if first {
            builder = builder.data_asset(href, assets::PARQUET_MEDIA_TYPE, size, checksum);
            first = false;
        } else {
            builder = builder.asset(
                assets::asset_key(path),
                package_asset(&href, size, checksum, assets::AssetKind::ObjectTable),
            );
        }
    }
    for name in &tables.sidecar_files {
        let path = dir.join(name);
        let (size, checksum) = assets::file_facts(&path);
        builder = builder.asset(
            name.clone(),
            package_asset(
                &format!("./{name}"),
                size,
                checksum,
                assets::AssetKind::Sidecar,
            ),
        );
    }

    builder
        .build()
        .map_err(|e| CityParquetError::Metadata(format!("cannot build STAC item: {e}")))
}

/// A STAC asset for one package file.
fn package_asset(
    href: &str,
    size: Option<u64>,
    checksum: Option<String>,
    kind: assets::AssetKind,
) -> Asset {
    let mut asset = Asset::new(href);
    asset.media_type = Some(assets::PARQUET_MEDIA_TYPE.to_string());
    asset.roles = kind.roles();
    if let Some(size) = size {
        asset
            .additional_fields
            .insert("file:size".to_string(), Value::Number(size.into()));
    }
    if let Some(checksum) = checksum {
        asset
            .additional_fields
            .insert("file:checksum".to_string(), Value::String(checksum));
    }
    asset
}

/// The package's CRS, as an EPSG code lifted out of the stored PROJJSON.
///
/// The footer stores the dataset CRS as PROJJSON (§13.3). `CRS` here is
/// EPSG-based, so the authority code is read from the PROJJSON `id`. A package
/// with no CRS yields `None`, and the caller must then either find its
/// coordinates already in WGS84 range or fail — never assume.
fn package_crs(tables: &PackageTables) -> Result<Option<CRS>> {
    let Some(path) = tables.tables.first() else {
        return Ok(None);
    };
    let file = fs::File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open {}: {e}", path.display())))?;
    let meta = builder.cityparquet_metadata()?;

    Ok(meta
        .crs
        .as_ref()
        .and_then(|c| c.get("id"))
        .and_then(|id| id.get("code"))
        .and_then(|code| code.as_u64())
        .map(|code| CRS::from_epsg(code as u32)))
}

/// The `bbox` struct's six leaf columns, in the order [`BBox3D`] uses.
/// Mirrors `crate::reader`'s `BBOX_LEAVES` and
/// `cityparquet_schema::model::bbox_data_type`.
const BBOX_LEAVES: [&str; 6] = ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"];

/// The package's spatial extent, in the **source** CRS.
///
/// Unioned from the `bbox` column's per-row-group leaf statistics across every
/// object table, so this is O(footer) — unlike the appearance and semantics
/// flags, the bbox leaves are plain `f64` columns and do carry statistics
/// (`crate::recipe` only disables them for JSON columns).
///
/// Returns `None` when no row group carries the leaf statistics — a package
/// with no geometry at all, or one written without them. Callers must not
/// substitute a guess: a STAC Item with a wrong bbox is worse than one with
/// none.
///
/// Reprojection to WGS84 is deliberately *not* done here; the Item assembly
/// step does it, because that is where the CRS is known.
pub fn package_bbox(tables: &PackageTables) -> Result<Option<BBox3D>> {
    // (min, max) accumulators per leaf, in BBOX_LEAVES order.
    let mut acc: [Option<(f64, f64)>; 6] = [None; 6];

    for path in &tables.tables {
        let file = fs::File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::Parquet(format!("cannot open {}: {e}", path.display()))
        })?;

        for rg in builder.metadata().row_groups() {
            for (i, leaf) in BBOX_LEAVES.iter().enumerate() {
                let Some((lo, hi)) = bbox_leaf_bounds(rg, leaf) else {
                    continue;
                };
                acc[i] = Some(match acc[i] {
                    Some((min, max)) => (min.min(lo), max.max(hi)),
                    None => (lo, hi),
                });
            }
        }
    }

    // Every leaf must have contributed; a partial extent is not an extent.
    let mut bounds = [(0.0f64, 0.0f64); 6];
    for (i, slot) in acc.iter().enumerate() {
        match slot {
            Some(v) => bounds[i] = *v,
            None => return Ok(None),
        }
    }

    Ok(Some(BBox3D {
        // A min leaf contributes its minimum, a max leaf its maximum — taking
        // the min of `xmin` and the max of `xmax` is what makes the union a
        // superset rather than an intersection.
        xmin: bounds[0].0,
        ymin: bounds[1].0,
        zmin: bounds[2].0,
        xmax: bounds[3].1,
        ymax: bounds[4].1,
        zmax: bounds[5].1,
    }))
}

/// The `(min, max)` of the `bbox.<leaf>` column chunk in `rg`, if it exists
/// and carries f64 statistics.
///
/// The column path is built with `ColumnPath::new` over the two nested parts:
/// `ColumnPath::from("bbox.xmin")` does **not** split on `.` in parquet 58 and
/// would silently match nothing. `crate::reader`'s `bbox_leaf_statistics`
/// documents the same trap — it bit the writer side once already.
fn bbox_leaf_bounds(rg: &RowGroupMetaData, leaf: &str) -> Option<(f64, f64)> {
    let path = ColumnPath::new(vec!["bbox".to_string(), leaf.to_string()]);
    let stats = rg
        .columns()
        .iter()
        .find(|c| c.column_path() == &path)?
        .statistics()?;
    match stats {
        Statistics::Double(s) => Some((*s.min_opt()?, *s.max_opt()?)),
        _ => None,
    }
}
