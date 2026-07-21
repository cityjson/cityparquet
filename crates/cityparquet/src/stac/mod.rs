//! Derive a STAC Item from a written CityParquet package.
//!
//! Every field is read back from the files on disk — the Parquet footer, the
//! Arrow schema, and the dictionary-encoded `object_type` column. Nothing is
//! accumulated during conversion. This makes spec §13.2's rule ("where the
//! STAC Item and the Parquet footer disagree, the footer is authoritative")
//! true by construction rather than by discipline, and means a package written
//! by any conformant writer can be described, not just this crate's own.

pub mod attribute_type;
pub mod properties;

use std::fs;

use city3d_stac_types::metadata::BBox3D;
use cityparquet_schema::{CityParquetError, Result};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::RowGroupMetaData;
use parquet::file::statistics::Statistics;
use parquet::schema::types::ColumnPath;

use crate::stac::properties::PackageTables;

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
