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
    /// RFC 3339 timestamp for `properties.datetime`, overriding whatever the
    /// package itself carries.
    ///
    /// When this is `None`, the source CityJSON header's `referenceDate` is
    /// used if the footer preserved one (`source_metadata`) — deriving from
    /// the package rather than inventing, which is this module's whole premise.
    ///
    /// **There is no fallback beyond that.** Almost no real CityJSON carries
    /// temporal metadata — none of this repo's fixtures does — so a synthetic
    /// fallback would govern nearly every package rather than an edge case.
    /// Per the design decision of 2026-07-20 the CLI stamps the conversion
    /// time and the *library* does not; this is the library, so `datetime`
    /// simply stays unset.
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
        .city3d(props)
        .map_err(|e| CityParquetError::Metadata(format!("cannot build STAC item: {e}")))?;

    // An explicit value wins; otherwise fall back to the source header's
    // `referenceDate`, which the footer preserves verbatim in
    // `source_metadata`. Deriving it from the package is the same principle
    // the rest of this module follows.
    builder = match &opts.datetime {
        Some(dt) => builder.datetime(Some(dt.clone())),
        None => builder.datetime_from_reference_date(source_metadata(&tables)?.as_ref()),
    };

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

    // The first table also goes through `data_asset`, purely because that is
    // what declares the STAC File extension — but the asset it inserts is
    // keyed `"data"` with roles `["data"]`, so it is then *replaced* by a
    // properly keyed and roled one.
    //
    // Without that replacement the primary object table would be the one asset
    // a reader cannot map back to a file by key, and the only table missing
    // the `cityparquet-objects` role — which is exactly the role Plan 2b binds
    // `export` to when it drops the manifest's `sidecar_files` list. Iterating
    // that role would have silently skipped the first table.
    let mut declared_file_extension = false;
    for path in &tables.tables {
        let (size, checksum) = assets::file_facts(path);
        let key = assets::asset_key(path);
        let href = format!("./{key}");
        if !declared_file_extension {
            builder = builder.data_asset(
                href.clone(),
                assets::PARQUET_MEDIA_TYPE,
                size,
                checksum.clone(),
            );
            declared_file_extension = true;
        }
        builder = builder.asset(
            key,
            package_asset(&href, size, checksum, assets::AssetKind::ObjectTable),
        );
    }
    for name in &tables.sidecar_files {
        let path = dir.join(name);
        // A manifest promise the package cannot keep is corruption, not
        // something to describe: an Item claiming an asset that is not on disk
        // is worse than a failed derivation. `crate::export` treats a
        // listed-but-missing sidecar the same way.
        if !path.exists() {
            return Err(CityParquetError::Io(format!(
                "package manifest lists sidecar '{name}' but {} is not on disk",
                path.display()
            )));
        }
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

/// The source CityJSON header's `metadata` object, as the footer preserved it.
///
/// Carries `referenceDate` when the source had one, which is the only honest
/// source of a `datetime` for a package.
fn source_metadata(tables: &PackageTables) -> Result<Option<Value>> {
    let Some(path) = tables.tables.first() else {
        return Ok(None);
    };
    let file = fs::File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open {}: {e}", path.display())))?;
    Ok(builder.cityparquet_metadata()?.source_metadata)
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

    let Some(id) = meta.crs.as_ref().and_then(|c| c.get("id")) else {
        return Ok(None);
    };

    // The authority must actually be EPSG. `CRS` here is EPSG-based, so
    // labelling an OGC or IAU code as EPSG would produce a confidently wrong
    // reprojection rather than an honest failure.
    match id.get("authority").and_then(|a| a.as_str()) {
        Some("EPSG") => {}
        Some(other) => {
            return Err(CityParquetError::Metadata(format!(
                "package CRS authority is {other}, not EPSG; cannot resolve to an EPSG code"
            )));
        }
        None => return Ok(None),
    }

    // PROJJSON permits the code as a number or a digit string.
    let code = id.get("code").and_then(|code| {
        code.as_u64()
            .or_else(|| code.as_str().and_then(|s| s.parse::<u64>().ok()))
    });
    let Some(code) = code else {
        return Ok(None);
    };
    let code = u32::try_from(code).map_err(|_| {
        CityParquetError::Metadata(format!("package CRS code {code} is not a valid EPSG code"))
    })?;
    Ok(Some(CRS::from_epsg(code)))
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
                    // A row group that cannot report a leaf may hold geometry
                    // outside everything observed so far. Continuing would
                    // yield a bbox that is smaller than the data — the one
                    // failure mode a spatial extent must never have, because a
                    // too-small bbox causes false-negative pruning. Give up on
                    // the whole extent rather than emit an under-bound.
                    return Ok(None);
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
