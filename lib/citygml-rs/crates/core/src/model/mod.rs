//! The version-neutral CityGML model a reader fills and a conversion reads.
//!
//! It names no target format: no type here comes from cjseq or any other
//! CityJSON crate, so replacing the intermediate model a tool converts to
//! means writing one more [`crate::convert::FromGml`], not touching a reader.

mod address;
mod ade;
mod appearance;
mod geometry;
mod group;
mod object;
mod value;

pub use address::Address;
pub use ade::{Ade, AdeIndex, AdeIndexBuilder, AdeLayout, XmlAttribute, XmlContent, XmlNode};
pub use appearance::{Appearance, Material, ModelAppearance, RingUvs, Texture};
pub use geometry::{Face, Geometry, Instance, Line, Polygon, Ring, Shape, Template};
pub use group::{GroupIndex, GroupMember, MemberTarget};
pub use object::{BoundarySurface, GmlObject, Module, Namespace, QName};
pub use value::{Attribute, AttributeName, Value};
