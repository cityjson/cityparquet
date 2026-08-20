//! Tolerant-decode helpers shared by every site that reads a value out of a
//! column whose PHYSICAL Arrow representation is a writer's choice, not a
//! reader's assumption (spec: readers tolerate physical variation rather
//! than normalising it away — `cityparquet_arrow_schema` reports the file's
//! own types, so decoding must accept them as they actually are).
//!
//! Two shapes recur across `decode`, `package`, and `query_core`, so each
//! gets exactly one helper here rather than three independent `match`es that
//! could drift apart:
//!
//! - a string column that may be plain `Utf8` or `Dictionary<Int32, Utf8>`
//!   (this crate's own writer always dictionary-encodes `object_type`; a
//!   foreign writer may leave it plain) — [`string_view`];
//! - a `Timestamp(_, UTC)` column in either unit CityParquet permits
//!   (MILLIS or MICROS) — [`timestamp_utc_value`].

use arrow_array::types::Int32Type;
use arrow_array::{Array, ArrayAccessor, DictionaryArray, StringArray, TypedDictionaryArray};
use arrow_schema::TimeUnit;

use cityparquet_schema::{CityParquetError, Result};

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

/// A string column's per-row view, tolerant of either physical
/// representation a writer may have chosen for it.
pub(crate) enum StringView<'a> {
    Plain(&'a StringArray),
    Dictionary(TypedDictionaryArray<'a, Int32Type, StringArray>),
}

impl StringView<'_> {
    pub(crate) fn is_null(&self, row: usize) -> bool {
        match self {
            Self::Plain(a) => a.is_null(row),
            Self::Dictionary(a) => a.is_null(row),
        }
    }

    /// The row's string value. Callers must check [`Self::is_null`] first —
    /// mirrors `TypedDictionaryArray::value`'s own null-unchecked contract
    /// (a null key position may point at any dictionary entry, or none).
    pub(crate) fn value(&self, row: usize) -> &str {
        match self {
            Self::Plain(a) => a.value(row),
            Self::Dictionary(a) => a.value(row),
        }
    }
}

/// Resolve `array` (named `column`, for error messages) into a
/// [`StringView`] — a plain `Utf8` array, or a `Dictionary<Int32, Utf8>`
/// array with `Utf8` values. Errors (never panics) on any other physical
/// shape, or on a dictionary whose values are not `Utf8`.
pub(crate) fn string_view<'a>(array: &'a dyn Array, column: &str) -> Result<StringView<'a>> {
    if let Some(a) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(StringView::Plain(a));
    }
    if let Some(a) = array.as_any().downcast_ref::<DictionaryArray<Int32Type>>() {
        let typed = a
            .downcast_dict::<StringArray>()
            .ok_or_else(|| err(format!("column '{column}' dictionary values are not Utf8")))?;
        return Ok(StringView::Dictionary(typed));
    }
    Err(err(format!(
        "column '{column}' has an unexpected array type (expected Utf8 or Dictionary<Int32, Utf8>)"
    )))
}

/// The wall-clock (naive UTC) instant a `Timestamp(unit, UTC)` cell holds,
/// tolerant of either physical unit CityParquet permits (spec: a writer's
/// choice of MILLIS or MICROS). `None` only for an out-of-range epoch value,
/// mirroring arrow's own `value_as_datetime`; the row's nullness is the
/// caller's responsibility to check first.
pub(crate) fn timestamp_utc_value(
    array: &dyn Array,
    unit: TimeUnit,
    row: usize,
    column: &str,
) -> Result<Option<chrono::NaiveDateTime>> {
    match unit {
        TimeUnit::Millisecond => {
            let a = array
                .as_any()
                .downcast_ref::<arrow_array::TimestampMillisecondArray>()
                .ok_or_else(|| {
                    err(format!(
                        "column '{column}' is not a Millisecond timestamp array"
                    ))
                })?;
            Ok(a.value_as_datetime(row))
        }
        TimeUnit::Microsecond => {
            let a = array
                .as_any()
                .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
                .ok_or_else(|| {
                    err(format!(
                        "column '{column}' is not a Microsecond timestamp array"
                    ))
                })?;
            Ok(a.value_as_datetime(row))
        }
        other => Err(err(format!(
            "column '{column}' has an unsupported timestamp unit: {other:?}"
        ))),
    }
}
