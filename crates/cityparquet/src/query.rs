//! Read/query primitives over a CityParquet package: the first primitives
//! of the cross-format read-benchmark milestone (later tasks add
//! attribute/id queries on top of these).
//!
//! [`count`] is O(1) — it reads the row count straight out of the Parquet
//! file metadata, no row scan. [`full_read`] is the opposite extreme: a
//! single-threaded scan of every row group that decodes every row's WKB
//! geometry (via [`crate::decode`]/[`crate::wkb_read`]), forcing full
//! materialisation — the metric later cross-format comparisons key off.
//! [`bbox_query`] sits in between: it prunes row groups via
//! [`crate::reader::CityParquetReaderBuilder::with_bbox_row_groups`] (a
//! superset — never wrong, but may over-select) and then applies a row-level
//! 3D bbox-intersection test on every surviving row, so its result is exact.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{Array, Float64Array, StringArray, StructArray};
use cityparquet_schema::{CityParquetError, CityParquetMetadata, Result};
use parquet::arrow::ProjectionMask;

use crate::decode::decode_batch;
use crate::reader::{
    CityParquetReaderBuilder, CityParquetRecordBatchReader, box_intersects_query,
    row_group_intersects,
};
use crate::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn io_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

fn parquet_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Parquet(e.to_string())
}

/// The result of a [`full_read`]: the total feature (row) count and a
/// stable geometry-work metric (`boundary_count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FullReadResult {
    pub feature_count: u64,
    /// Total number of decoded surfaces/faces across every non-null
    /// geometry cell read (`DecodedKind::MultiPolygon`/`PolyhedralSurface`'s
    /// outer `Vec` length, summed, recursing into
    /// `DecodedKind::GeometryCollection` members). `MultiPoint`/
    /// `MultiLineString` geometries contribute 0 — they have no surfaces.
    /// Deliberately simple and deterministic: this is the metric later
    /// cross-format ("full read forces materialisation") comparisons key
    /// off, so its definition must be stable across formats, not a
    /// CityParquet-specific detail.
    pub boundary_count: u64,
}

/// Total surface/face count in `kind`, recursing into
/// [`DecodedKind::GeometryCollection`] members.
fn surface_count(kind: &DecodedKind) -> u64 {
    match kind {
        DecodedKind::MultiPoint(_) | DecodedKind::MultiLineString(_) => 0,
        DecodedKind::MultiPolygon(surfaces) | DecodedKind::PolyhedralSurface(surfaces) => {
            surfaces.len() as u64
        }
        DecodedKind::GeometryCollection(members) => members.iter().map(surface_count).sum(),
    }
}

/// Opens `table_path`, scans every row group single-threaded (the
/// `parquet` crate's synchronous [`ParquetRecordBatchReaderBuilder`] path
/// never spreads batch iteration across a thread pool, unlike its async
/// counterpart), and decodes each row's WKB geometry, accumulating
/// [`FullReadResult::feature_count`] (total rows) and
/// [`FullReadResult::boundary_count`] (total decoded surfaces/faces).
/// Forces full geometry materialisation.
pub fn full_read(table_path: &Path, meta: &CityParquetMetadata) -> Result<FullReadResult> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let parquet_reader = builder.build().map_err(parquet_err)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut feature_count = 0u64;
    let mut boundary_count = 0u64;
    for batch in reader {
        let batch = batch?;
        feature_count += batch.num_rows() as u64;
        let decoded = decode_batch(&batch, meta)?;
        for object in &decoded {
            for (_, geometry, _) in &object.geometries {
                boundary_count += surface_count(&geometry.kind);
            }
        }
    }
    Ok(FullReadResult {
        feature_count,
        boundary_count,
    })
}

/// The table's row count straight from Parquet file metadata — O(1), no
/// row scan.
pub fn count(table_path: &Path) -> Result<u64> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    Ok(builder.metadata().file_metadata().num_rows() as u64)
}

/// The result of an exact [`bbox_query`]: the matching object `id`s, plus
/// how many of the table's row groups were pruned away vs. actually
/// touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BBoxQueryResult {
    /// `id`s of every row whose `bbox` truly 3D-intersects the query — an
    /// exact result, not the row-group-pruning superset.
    pub ids: Vec<String>,
    /// Total row groups in the table.
    pub row_groups_total: usize,
    /// Row groups [`row_group_intersects`] could not rule out (the same
    /// count [`crate::reader::CityParquetReaderBuilder::with_bbox_row_groups`]
    /// keeps for the scan below).
    pub row_groups_touched: usize,
}

/// One row's `bbox` struct leaves at `row`, or `None` if the struct itself
/// is null at that row (an object with no bbox has no extent to test, so it
/// can never match a bbox query).
fn row_bbox(bbox_col: &StructArray, row: usize) -> Result<Option<([f64; 3], [f64; 3])>> {
    if bbox_col.is_null(row) {
        return Ok(None);
    }
    let leaf = |name: &str| -> Result<f64> {
        Ok(bbox_col
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or_else(|| {
                CityParquetError::Schema(format!("bbox.{name} column missing or not Float64"))
            })?
            .value(row))
    };
    let min = [leaf("xmin")?, leaf("ymin")?, leaf("zmin")?];
    let max = [leaf("xmax")?, leaf("ymax")?, leaf("zmax")?];
    Ok(Some((min, max)))
}

/// Opens `table_path`, prunes row groups via
/// [`CityParquetReaderBuilder::with_bbox_row_groups`] (a superset — it never
/// wrongly drops a row group, but may keep groups with no true match), then
/// reads only the `id`/`bbox` columns of every surviving row and applies a
/// row-level 3D bbox-intersection test, so the returned `ids` are exact.
/// `row_groups_total`/`row_groups_touched` report the same pruning counts
/// the `cityparquet bench` CLI harness measures, via the identical shared
/// [`row_group_intersects`] predicate.
pub fn bbox_query(table_path: &Path, query_bbox: [f64; 6]) -> Result<BBoxQueryResult> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;

    // Row-group pruning counts, computed BEFORE `with_bbox_row_groups`
    // consumes `builder` — the exact same predicate `with_bbox_row_groups`
    // itself uses, so these counts can never drift from what the scan below
    // actually reads.
    let metadata = Arc::clone(builder.metadata());
    let row_groups_total = metadata.num_row_groups();
    let row_groups_touched = (0..row_groups_total)
        .filter(|&i| row_group_intersects(metadata.row_group(i), &query_bbox))
        .count();

    // Project down to just `id` and `bbox` — the row-level filter below
    // needs nothing else, and every other column (geometry, attributes) can
    // be arbitrarily large.
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["id", "bbox"]);
    let pruned = builder
        .with_projection(projection)
        .with_bbox_row_groups(query_bbox)?;
    let reader = pruned.build().map_err(parquet_err)?;

    let mut ids = Vec::new();
    for batch in reader {
        let batch = batch.map_err(parquet_err)?;
        let id_col = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .ok_or_else(|| {
                CityParquetError::Schema("'id' column missing or not Utf8".to_string())
            })?;
        let bbox_col = batch
            .column_by_name("bbox")
            .and_then(|c| c.as_any().downcast_ref::<StructArray>())
            .ok_or_else(|| {
                CityParquetError::Schema("'bbox' column missing or not a struct".to_string())
            })?;

        for row in 0..batch.num_rows() {
            let Some((row_min, row_max)) = row_bbox(bbox_col, row)? else {
                continue;
            };
            if box_intersects_query(row_min, row_max, &query_bbox) {
                ids.push(id_col.value(row).to_string());
            }
        }
    }

    Ok(BBoxQueryResult {
        ids,
        row_groups_total,
        row_groups_touched,
    })
}
