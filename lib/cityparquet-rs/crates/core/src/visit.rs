//! The native object visit: every city object in a [`RecordBatch`] read in
//! CityParquet's own representation — Arrow arrays and WKB bytes walked in
//! place — without converting rows into CityJSON-shaped objects.
//!
//! [`visit_batch`] reads, for every row of the batch:
//!
//! - every `geometry_lod*` WKB cell, every vertex as a real `f64` triple,
//!   through [`crate::wkb_read::visit_wkb`];
//! - every `geometry_properties_lod*` struct: `type`, `face_semantics`,
//!   `shells`, and `surfaces` (the semantic-surface objects), which is parsed
//!   as JSON;
//! - the `template` (implicit geometry) column as stored — its anchor and
//!   transformation — without expanding the referenced template;
//! - `id`, `feature_id`, `object_type`, `parents`, `children`,
//!   `children_roles`, `bbox`, `address`, every typed attribute column, and
//!   the `other` JSON column, which is parsed.
//!
//! It does **not** read the `material_lod*`/`texture_lod*` columns or the
//! appearance sidecars: appearance is outside the read benchmark's target
//! for every format.
//!
//! The walk is column-major — each column over every row of the batch — and
//! every value read either feeds [`VisitTotals`] or passes through
//! [`std::hint::black_box`], so the optimiser cannot drop the work.

use std::hint::black_box;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{Array, RecordBatch};
use arrow_schema::DataType;
use cityparquet_schema::{CityParquetError, Result};

use crate::wkb_read::{WkbVisitor, visit_wkb};

/// What a native visit read. The first group is comparable across formats;
/// the second depends on how each format encodes the same model and is
/// **not** comparable across formats.
///
/// Comparable:
///
/// - `objects`: rows visited (one per city object).
/// - `geometries`: non-null `geometry_lod*` cells. CityParquet stores one
///   geometry per LoD column (`geometry_lod2_2`, …), so this matches a
///   CityJSON source's geometry entries, with two documented exceptions:
///   the writer synthesises a LoD0 footprint into `geometry_lod0_0` by
///   default (counted here and separately in `lod0_geometries`, so a caller
///   whose source carries no LoD0 subtracts it), and a geometry whose every
///   surface the writer dropped as degenerate is stored as null and is not
///   counted. `GeometryInstance` entries live in `template`, not here.
/// - `lod0_geometries`: the subset of `geometries` in `geometry_lod0*`
///   columns.
/// - `semantic_faces`: non-null `face_semantics` items, i.e. faces carrying a
///   semantic-surface reference.
/// - `extent`: `[min x, min y, min z, max x, max y, max z]` over every
///   visited vertex; infinite bounds when nothing was visited.
///
/// Format-specific:
///
/// - `polygons`, `rings`, `coordinates`: WKB polygons, rings and stored
///   vertices. `coordinates` counts the WKB ring-closing vertex, and faces
///   or rings the writer dropped as degenerate are absent, so all three can
///   differ from the source's boundary counts.
/// - `semantic_surface_objects`: entries of the parsed `surfaces` arrays.
/// - `attribute_values`: non-null typed attribute cells plus the keys of each
///   parsed `other` object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisitTotals {
    pub objects: u64,
    pub geometries: u64,
    pub lod0_geometries: u64,
    pub semantic_faces: u64,
    pub extent: [f64; 6],
    pub polygons: u64,
    pub rings: u64,
    pub coordinates: u64,
    pub semantic_surface_objects: u64,
    pub attribute_values: u64,
}

impl Default for VisitTotals {
    fn default() -> Self {
        Self {
            objects: 0,
            geometries: 0,
            lod0_geometries: 0,
            semantic_faces: 0,
            extent: [
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            polygons: 0,
            rings: 0,
            coordinates: 0,
            semantic_surface_objects: 0,
            attribute_values: 0,
        }
    }
}

impl WkbVisitor for VisitTotals {
    fn coord(&mut self, c: [f64; 3]) {
        self.coordinates += 1;
        for k in 0..3 {
            self.extent[k] = self.extent[k].min(c[k]);
            self.extent[k + 3] = self.extent[k + 3].max(c[k]);
        }
    }
    fn ring_end(&mut self, _n_points: usize) {
        self.rings += 1;
    }
    fn polygon_end(&mut self, _n_rings: usize) {
        self.polygons += 1;
    }
}

/// Reserved non-geometry columns: read in full, not counted as attributes.
const RESERVED: &[&str] = &[
    "id",
    "feature_id",
    "object_type",
    "parents",
    "children",
    "children_roles",
    "address",
    "bbox",
    "template",
];

/// Visits every row of `batch` natively and folds what it read into
/// `totals` (see the module documentation for what is and is not read).
pub fn visit_batch(batch: &RecordBatch, totals: &mut VisitTotals) -> Result<()> {
    totals.objects += batch.num_rows() as u64;
    for (field, col) in batch.schema().fields().iter().zip(batch.columns()) {
        let name = field.name().as_str();
        if name.starts_with("material") || name.starts_with("texture") {
            continue;
        } else if name.starts_with("geometry_properties_lod") {
            visit_properties(name, col.as_ref(), totals)?;
        } else if name.starts_with("geometry_lod") {
            visit_geometry(name, col.as_ref(), totals)?;
        } else if name == "other" {
            let view = crate::arrow_compat::string_view(col.as_ref(), name)?;
            for row in 0..col.len() {
                if view.is_null(row) {
                    continue;
                }
                let value: serde_json::Value = parse_json(name, view.value(row))?;
                if let Some(obj) = value.as_object() {
                    totals.attribute_values += obj.len() as u64;
                }
                black_box(&value);
            }
        } else {
            let leaves = walk(col.as_ref(), 0..col.len());
            if !RESERVED.contains(&name) {
                totals.attribute_values += (col.len() - col.null_count()) as u64;
            }
            black_box(leaves);
        }
    }
    Ok(())
}

fn visit_geometry(name: &str, col: &dyn Array, totals: &mut VisitTotals) -> Result<()> {
    let lod0 = name.starts_with("geometry_lod0");
    let mut each = |bytes: &[u8]| -> Result<()> {
        totals.geometries += 1;
        totals.lod0_geometries += u64::from(lod0);
        visit_wkb(bytes, totals)
    };
    match col.data_type() {
        DataType::Binary => {
            for v in col.as_binary::<i32>().iter().flatten() {
                each(v)?;
            }
        }
        DataType::LargeBinary => {
            for v in col.as_binary::<i64>().iter().flatten() {
                each(v)?;
            }
        }
        DataType::BinaryView => {
            for v in col.as_binary_view().iter().flatten() {
                each(v)?;
            }
        }
        other => {
            return Err(CityParquetError::Schema(format!(
                "geometry column '{name}' is not binary WKB: {other:?}"
            )));
        }
    }
    Ok(())
}

fn visit_properties(name: &str, col: &dyn Array, totals: &mut VisitTotals) -> Result<()> {
    let props = col.as_struct_opt().ok_or_else(|| {
        CityParquetError::Schema(format!("'{name}' is not a geometry_properties struct"))
    })?;
    for (field, child) in props.fields().iter().zip(props.columns()) {
        match field.name().as_str() {
            "surfaces" => {
                let view = crate::arrow_compat::string_view(child.as_ref(), name)?;
                for row in 0..child.len() {
                    if props.is_null(row) || view.is_null(row) {
                        continue;
                    }
                    let value = parse_json(name, view.value(row))?;
                    if let Some(arr) = value.as_array() {
                        totals.semantic_surface_objects += arr.len() as u64;
                    }
                    black_box(&value);
                }
            }
            "face_semantics" => {
                let list = child.as_list_opt::<i32>().ok_or_else(|| {
                    CityParquetError::Schema(format!("'{name}.face_semantics' is not a List"))
                })?;
                for row in 0..list.len() {
                    if props.is_null(row) || list.is_null(row) {
                        continue;
                    }
                    let (start, end) = (
                        list.value_offsets()[row] as usize,
                        list.value_offsets()[row + 1] as usize,
                    );
                    totals.semantic_faces += walk(list.values().as_ref(), start..end);
                }
            }
            _ => {
                black_box(walk(child.as_ref(), 0..child.len()));
            }
        }
    }
    Ok(())
}

fn parse_json(column: &str, text: &str) -> Result<serde_json::Value> {
    serde_json::from_str(text)
        .map_err(|e| CityParquetError::Schema(format!("column '{column}' holds invalid JSON: {e}")))
}

/// Reads every non-null leaf value of `array` in `rows` through
/// [`black_box`], recursing into structs, lists and dictionaries, and
/// returns how many leaves it read.
fn walk(array: &dyn Array, rows: std::ops::Range<usize>) -> u64 {
    macro_rules! prim {
        ($t:ty) => {{
            let a = array.as_primitive::<$t>();
            rows.filter(|&i| a.is_valid(i))
                .map(|i| {
                    black_box(a.value(i));
                })
                .count() as u64
        }};
    }
    match array.data_type() {
        DataType::Int8 => prim!(Int8Type),
        DataType::Int16 => prim!(Int16Type),
        DataType::Int32 => prim!(Int32Type),
        DataType::Int64 => prim!(Int64Type),
        DataType::UInt8 => prim!(UInt8Type),
        DataType::UInt16 => prim!(UInt16Type),
        DataType::UInt32 => prim!(UInt32Type),
        DataType::UInt64 => prim!(UInt64Type),
        DataType::Float32 => prim!(Float32Type),
        DataType::Float64 => prim!(Float64Type),
        DataType::Boolean => {
            let a = array.as_boolean();
            rows.filter(|&i| a.is_valid(i))
                .map(|i| {
                    black_box(a.value(i));
                })
                .count() as u64
        }
        DataType::Utf8 => {
            let a = array.as_string::<i32>();
            rows.filter(|&i| a.is_valid(i))
                .map(|i| {
                    black_box(a.value(i));
                })
                .count() as u64
        }
        DataType::LargeUtf8 => {
            let a = array.as_string::<i64>();
            rows.filter(|&i| a.is_valid(i))
                .map(|i| {
                    black_box(a.value(i));
                })
                .count() as u64
        }
        DataType::Binary => {
            let a = array.as_binary::<i32>();
            rows.filter(|&i| a.is_valid(i))
                .map(|i| {
                    black_box(a.value(i));
                })
                .count() as u64
        }
        DataType::Dictionary(_, _) => {
            let d = array.as_any_dictionary();
            let keys = d.normalized_keys();
            let values = d.values();
            rows.filter(|&i| d.keys().is_valid(i))
                .map(|i| walk(values.as_ref(), keys[i]..keys[i] + 1))
                .sum()
        }
        DataType::Struct(_) => {
            let s = array.as_struct();
            s.columns()
                .iter()
                .map(|c| {
                    rows.clone()
                        .filter(|&i| s.is_valid(i))
                        .map(|i| walk(c.as_ref(), i..i + 1))
                        .sum::<u64>()
                })
                .sum()
        }
        DataType::List(_) => {
            let l = array.as_list::<i32>();
            let off = l.value_offsets();
            rows.filter(|&i| l.is_valid(i))
                .map(|i| walk(l.values().as_ref(), off[i] as usize..off[i + 1] as usize))
                .sum()
        }
        DataType::LargeList(_) => {
            let l = array.as_list::<i64>();
            let off = l.value_offsets();
            rows.filter(|&i| l.is_valid(i))
                .map(|i| walk(l.values().as_ref(), off[i] as usize..off[i + 1] as usize))
                .sum()
        }
        DataType::FixedSizeList(_, n) => {
            let l = array.as_fixed_size_list();
            let n = *n as usize;
            rows.filter(|&i| l.is_valid(i))
                .map(|i| {
                    let start = l.value_offset(i) as usize;
                    walk(l.values().as_ref(), start..start + n)
                })
                .sum()
        }
        _ => {
            black_box(array.len());
            0
        }
    }
}
