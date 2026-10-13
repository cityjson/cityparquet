//! cjseq's typed names and LoDs as the strings and values this crate stores.

use cityparquet_schema::Lod;
use cjseq::{CityObjectType, Geometry};

/// A City Object type as CityJSON spells it, an Extension type with its `+`.
pub(crate) fn object_type_name(t: &CityObjectType) -> String {
    String::from(t.clone())
}

/// A geometry's LoD; `None` for a GeometryInstance, which has none.
pub(crate) fn geometry_lod(geom: &Geometry) -> Option<Lod> {
    geom.lod()
        .map(|l| Lod::parse(&l.to_string()).expect("a cjseq LoD is always a valid CityParquet LoD"))
}
