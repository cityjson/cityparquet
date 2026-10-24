//! GML geometry: parsing ([`read`]) and the assembly of one object's
//! geometry with its `xlink:href`s resolved ([`ObjectBuilder`]).

mod assemble;
pub mod read;

pub use assemble::{ObjectBuilder, XlinkBudget};
