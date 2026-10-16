//! CityGML names and their CityJSON counterparts, as data.
//!
//! The model keeps CityGML's own names; a conversion looks the target name up
//! here rather than spelling the mapping out in code.

mod ade;
mod classes;

pub use ade::{known_ade_schema, module_of_prefix, module_prefix};
pub use classes::{cityjson_definition, cityjson_surface, cityjson_type};
