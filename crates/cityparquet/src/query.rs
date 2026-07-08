//! Read/query primitives over a CityParquet package: the first primitives
//! of the cross-format read-benchmark milestone (later tasks add
//! bbox/attribute/id queries on top of these).
//!
//! [`count`] is O(1) — it reads the row count straight out of the Parquet
//! file metadata, no row scan. [`full_read`] is the opposite extreme: a
//! single-threaded scan of every row group that decodes every row's WKB
//! geometry (via [`crate::decode`]/[`crate::wkb_read`]), forcing full
//! materialisation — the metric later cross-format comparisons key off.

use std::fs::File;
use std::path::Path;

use cityparquet_schema::{CityParquetError, CityParquetMetadata, Result};

use crate::decode::decode_batch;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
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
