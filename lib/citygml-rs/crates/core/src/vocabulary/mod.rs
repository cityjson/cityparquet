//! CityGML names and their CityJSON counterparts, as data.
//!
//! The model keeps CityGML's own names; a conversion looks the target name up
//! here rather than spelling the mapping out in code.

mod classes;

pub use classes::{cityjson_type, relation_to_construction};
