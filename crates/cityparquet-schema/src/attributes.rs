//! Attribute type inference: the spec's primitive-mapping table
//! (`notes/spec.md` § Attribute encoding) as code.

use arrow_schema::{DataType, Field, TimeUnit};
use chrono::{DateTime, NaiveDate};
use serde_json::Value;

/// Inferred CityParquet attribute column type, ordered roughly by specificity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributeType {
    Boolean,
    Int64,
    Float64,
    Date,
    Timestamp,
    String,
    StringList,
    /// Objects, heterogeneous arrays, or irreconcilable mixes; stored as JSON text.
    Json,
}

/// Cheap shape pre-check (`YYYY-MM-DD`) before the real parse below, so
/// obviously-non-date strings short-circuit without invoking chrono.
fn looks_like_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| b[i].is_ascii_digit())
}

/// Cheap shape pre-check before the real parse below.
/// `get` (not slicing) so a multi-byte char straddling byte 10 yields None
/// instead of panicking on a non-char-boundary index.
fn looks_like_timestamp(s: &str) -> bool {
    s.len() >= 19 && s.get(..10).is_some_and(looks_like_date) && s.as_bytes()[10] == b'T'
}

/// True only if `s` is both the right shape AND a real calendar date —
/// the SAME parser `encode.rs` uses to write `Date` values, so inference and
/// encoding can never disagree (a value that infers Date is guaranteed to
/// encode, never silently null out).
fn is_date(s: &str) -> bool {
    looks_like_date(s) && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// True only if `s` is both the right shape AND a real instant — the SAME
/// parser `encode.rs` uses to write `Timestamp` values.
fn is_timestamp(s: &str) -> bool {
    looks_like_timestamp(s) && DateTime::parse_from_rfc3339(s).is_ok()
}

impl AttributeType {
    /// Infer the type of a single JSON value. `None` for JSON null (no signal).
    pub fn infer(value: &Value) -> Option<Self> {
        match value {
            Value::Null => None,
            Value::Bool(_) => Some(Self::Boolean),
            // `is_u64()` alone would also match values above `i64::MAX`, which
            // `encode.rs`'s Int64 path (`Value::as_i64`) cannot represent and
            // would silently null out; only route through Int64 when the
            // value actually fits in an i64. Everything else numeric —
            // including u64 values beyond i64::MAX, lossy above 2^53 once
            // stored as Float64 — goes through Float64's `as_f64`, which
            // handles the full u64 range.
            Value::Number(n) if n.is_i64() => Some(Self::Int64),
            Value::Number(_) => Some(Self::Float64),
            Value::String(s) if is_timestamp(s) => Some(Self::Timestamp),
            Value::String(s) if is_date(s) => Some(Self::Date),
            Value::String(_) => Some(Self::String),
            Value::Array(items) if items.iter().all(|v| v.is_string()) => Some(Self::StringList),
            Value::Array(_) | Value::Object(_) => Some(Self::Json),
        }
    }

    /// Combine two observed types: promote when safe, otherwise fall back to Json.
    pub fn promote(self, other: Self) -> Self {
        use AttributeType::*;
        if self == other {
            return self;
        }
        match (self, other) {
            (Int64, Float64) | (Float64, Int64) => Float64,
            (Date, Timestamp) | (Timestamp, Date) => Timestamp,
            (Date | Timestamp, String) | (String, Date | Timestamp) => String,
            _ => Json,
        }
    }

    pub fn to_arrow(&self) -> DataType {
        match self {
            Self::Boolean => DataType::Boolean,
            Self::Int64 => DataType::Int64,
            Self::Float64 => DataType::Float64,
            Self::Date => DataType::Date32,
            // CityJSON date-times are ISO 8601 instants with an offset (e.g. `Z`);
            // the M2 encoder normalises every value to UTC before writing, so the
            // column's declared timezone must say "UTC" — leaving it `None` would
            // misdeclare these as timezone-naive wall-clock values in Parquet.
            Self::Timestamp => DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            Self::String => DataType::Utf8,
            Self::StringList => DataType::List(Field::new("item", DataType::Utf8, true).into()),
            Self::Json => DataType::Utf8,
        }
    }

    /// Reverse of [`Self::to_arrow`]: the `AttributeType` a stored column's
    /// arrow `DataType` was encoded from (the M3 reader's schema rebuild).
    ///
    /// `DataType::Utf8` is inherently ambiguous — both [`Self::String`] and
    /// [`Self::Json`] serialise to it — so this always resolves that case to
    /// `String`. A caller that can see the field's own metadata (the
    /// `arrow.json` `ARROW:extension:name`, attached by
    /// `cityparquet_schema::model`'s `json_field`) must upgrade the result to
    /// `Json` itself; a bare `DataType` carries no such signal.
    pub fn from_arrow(data_type: &DataType) -> Option<Self> {
        match data_type {
            DataType::Boolean => Some(Self::Boolean),
            DataType::Int64 => Some(Self::Int64),
            DataType::Float64 => Some(Self::Float64),
            DataType::Date32 => Some(Self::Date),
            DataType::Timestamp(TimeUnit::Millisecond, Some(tz)) if tz.as_ref() == "UTC" => {
                Some(Self::Timestamp)
            }
            DataType::Utf8 => Some(Self::String),
            DataType::List(field) if field.data_type() == &DataType::Utf8 => Some(Self::StringList),
            _ => None,
        }
    }
}

/// Accumulates attribute observations across features (writer pass 1).
#[derive(Debug, Default)]
pub struct AttributeInferer {
    // Insertion-ordered: Vec of (name, Option<AttributeType>).
    columns: Vec<(String, Option<AttributeType>)>,
}

impl AttributeInferer {
    pub fn observe(&mut self, name: &str, value: &Value) {
        let observed = AttributeType::infer(value);
        match self.columns.iter_mut().find(|(n, _)| n == name) {
            Some((_, slot)) => {
                *slot = match (*slot, observed) {
                    (Some(a), Some(b)) => Some(a.promote(b)),
                    (existing, new) => existing.or(new),
                };
            }
            None => self.columns.push((name.to_string(), observed)),
        }
    }

    /// Final column list in first-seen order; all-null columns fall back to String.
    pub fn finish(self) -> Vec<(String, AttributeType)> {
        self.columns
            .into_iter()
            .map(|(n, t)| (n, t.unwrap_or(AttributeType::String)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::DataType;
    use serde_json::json;

    #[test]
    fn infers_primitives() {
        assert_eq!(
            AttributeType::infer(&json!(true)),
            Some(AttributeType::Boolean)
        );
        assert_eq!(AttributeType::infer(&json!(42)), Some(AttributeType::Int64));
        assert_eq!(
            AttributeType::infer(&json!(4.2)),
            Some(AttributeType::Float64)
        );
        assert_eq!(
            AttributeType::infer(&json!("hi")),
            Some(AttributeType::String)
        );
        assert_eq!(AttributeType::infer(&json!(null)), None);
    }

    #[test]
    fn u64_beyond_i64_max_infers_float64() {
        // 18446744073709551615 == u64::MAX, well beyond i64::MAX: encoding this
        // as Int64 (via as_i64()) would silently yield null.
        assert_eq!(
            AttributeType::infer(&json!(18446744073709551615u64)),
            Some(AttributeType::Float64)
        );
    }

    #[test]
    fn detects_dates_and_timestamps() {
        assert_eq!(
            AttributeType::infer(&json!("2015-03-21")),
            Some(AttributeType::Date)
        );
        assert_eq!(
            AttributeType::infer(&json!("2015-03-21T10:30:00Z")),
            Some(AttributeType::Timestamp)
        );
        // Not a date: wrong shape.
        assert_eq!(
            AttributeType::infer(&json!("2015-3-21")),
            Some(AttributeType::String)
        );
    }

    #[test]
    fn rejects_dates_and_timestamps_that_have_the_right_shape_but_are_invalid() {
        // Right shape (YYYY-MM-DD / YYYY-MM-DDThh:mm:ssZ) but not a real
        // calendar date/time: encode.rs's chrono parse would fail and null
        // the value out, so inference must not claim Date/Timestamp here.
        assert_eq!(
            AttributeType::infer(&json!("2026-99-99")),
            Some(AttributeType::String)
        );
        assert_eq!(
            AttributeType::infer(&json!("2015-03-21T99:99:99Z")),
            Some(AttributeType::String)
        );
    }

    #[test]
    fn arrays_and_objects() {
        assert_eq!(
            AttributeType::infer(&json!(["a", "b"])),
            Some(AttributeType::StringList)
        );
        assert_eq!(
            AttributeType::infer(&json!([1, "b"])),
            Some(AttributeType::Json)
        );
        assert_eq!(
            AttributeType::infer(&json!({"k": 1})),
            Some(AttributeType::Json)
        );
        assert_eq!(
            AttributeType::infer(&json!([])),
            Some(AttributeType::StringList)
        );
    }

    #[test]
    fn promotion_lattice() {
        use AttributeType::*;
        assert_eq!(Int64.promote(Float64), Float64);
        assert_eq!(Float64.promote(Int64), Float64);
        assert_eq!(Date.promote(Timestamp), Timestamp);
        assert_eq!(Date.promote(String), String);
        assert_eq!(Int64.promote(String), Json);
        assert_eq!(Boolean.promote(Int64), Json);
        assert_eq!(StringList.promote(String), Json);
        assert_eq!(Json.promote(Boolean), Json);
        assert_eq!(Int64.promote(Int64), Int64);
    }

    #[test]
    fn inferer_accumulates_across_features() {
        let mut inf = AttributeInferer::default();
        inf.observe("yoc", &json!(1990));
        inf.observe("height", &json!(12.5));
        inf.observe("yoc", &json!(1985.5)); // int then float → Float64
        inf.observe("name", &json!(null)); // null alone → String fallback
        let cols = inf.finish();
        assert_eq!(
            cols,
            vec![
                ("yoc".to_string(), AttributeType::Float64),
                ("height".to_string(), AttributeType::Float64),
                ("name".to_string(), AttributeType::String),
            ]
        );
    }

    #[test]
    fn non_ascii_strings_do_not_panic() {
        assert_eq!(
            AttributeType::infer(&json!("日本語の文字列データです")),
            Some(AttributeType::String)
        );
        assert_eq!(
            AttributeType::infer(&json!("Ключевая строка данных")),
            Some(AttributeType::String)
        );
    }

    #[test]
    fn maps_to_arrow_types() {
        assert_eq!(AttributeType::Int64.to_arrow(), DataType::Int64);
        assert_eq!(AttributeType::Date.to_arrow(), DataType::Date32);
        assert!(matches!(
            AttributeType::StringList.to_arrow(),
            DataType::List(_)
        ));
        assert_eq!(AttributeType::Json.to_arrow(), DataType::Utf8);
        assert_eq!(
            AttributeType::Timestamp.to_arrow(),
            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );
    }

    #[test]
    fn from_arrow_round_trips_every_variant_except_the_documented_json_ambiguity() {
        // Json and String both serialise to Utf8 (see `maps_to_arrow_types`
        // above); everything else round-trips through to_arrow -> from_arrow
        // back to itself.
        use AttributeType::*;
        for variant in [Boolean, Int64, Float64, Date, Timestamp, String, StringList] {
            assert_eq!(
                AttributeType::from_arrow(&variant.to_arrow()),
                Some(variant),
                "{variant:?} should round-trip through to_arrow/from_arrow"
            );
        }
    }

    #[test]
    fn from_arrow_resolves_the_json_utf8_ambiguity_to_string() {
        // `from_arrow` alone cannot distinguish Json from String (both are
        // bare Utf8) — it documents falling back to String. Callers that can
        // see the field's `arrow.json` extension metadata (the M3 reader)
        // upgrade to Json themselves.
        assert_eq!(AttributeType::Json.to_arrow(), DataType::Utf8);
        assert_eq!(
            AttributeType::from_arrow(&DataType::Utf8),
            Some(AttributeType::String)
        );
    }

    #[test]
    fn from_arrow_rejects_types_cityparquet_cannot_represent() {
        assert_eq!(AttributeType::from_arrow(&DataType::Int32), None);
        assert_eq!(AttributeType::from_arrow(&DataType::Float32), None);
        // Right variant, wrong timezone: CityParquet timestamps are always
        // normalised to UTC at encode time (see `to_arrow`'s doc comment).
        assert_eq!(
            AttributeType::from_arrow(&DataType::Timestamp(TimeUnit::Millisecond, None)),
            None
        );
        assert_eq!(
            AttributeType::from_arrow(&DataType::List(
                Field::new("item", DataType::Int64, true).into()
            )),
            None
        );
    }
}
