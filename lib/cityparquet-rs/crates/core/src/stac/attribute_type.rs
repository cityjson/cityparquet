//! Mapping from CityParquet's inferred column type to the `city3d`
//! extension's attribute type.

use city3d_stac_types::metadata::AttributeType as City3d;
use cityparquet_schema::attributes::AttributeType as Cp;

/// Map a CityParquet column type to the `city3d` extension's attribute type.
///
/// The two vocabularies differ: CityParquet distinguishes `Int64`/`Float64`
/// and `Date`/`Timestamp`, while the extension has a single `Number` and a
/// single `Date`. Both narrowings are lossless for the extension's purpose,
/// which is describing a dataset for discovery, not decoding it.
///
/// `Json` -> `Object` is the one lossy case. A `Json` column holds "objects,
/// heterogeneous arrays, or irreconcilable mixes", so some of its values may
/// really be arrays. The column type alone cannot tell them apart without
/// reading values, and `Object` is the extension's least-specific option, so
/// it is the honest choice.
pub fn to_city3d(t: Cp) -> City3d {
    match t {
        Cp::Boolean => City3d::Boolean,
        Cp::Int64 | Cp::Float64 => City3d::Number,
        Cp::Date | Cp::Timestamp => City3d::Date,
        Cp::String => City3d::String,
        Cp::StringList => City3d::Array,
        Cp::Json => City3d::Object,
    }
}

#[cfg(test)]
mod tests {
    use super::to_city3d;
    use city3d_stac_types::metadata::AttributeType as City3d;
    use cityparquet_schema::attributes::AttributeType as Cp;

    #[test]
    fn every_cityparquet_type_maps_to_a_city3d_type() {
        assert_eq!(to_city3d(Cp::Boolean), City3d::Boolean);
        assert_eq!(to_city3d(Cp::Int64), City3d::Number);
        assert_eq!(to_city3d(Cp::Float64), City3d::Number);
        assert_eq!(to_city3d(Cp::Date), City3d::Date);
        assert_eq!(to_city3d(Cp::Timestamp), City3d::Date);
        assert_eq!(to_city3d(Cp::String), City3d::String);
        assert_eq!(to_city3d(Cp::StringList), City3d::Array);
        assert_eq!(to_city3d(Cp::Json), City3d::Object);
    }
}
