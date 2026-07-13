//! Native CityGML 2.0 input reader.
//!
//! Streams a CityGML 2.0 document (quick-xml) into `cjseq::CityJSONFeature`
//! values that flow through the existing `scan -> encode -> Parquet` pipeline
//! unchanged, so [`crate::source::Source`] is the only integration surface.
//!
//! ## Supported profile (M1)
//! - `bldg:Building` with LoD1/LoD2 `gml:Solid` geometry; coordinates in
//!   `gml:posList` or per-point `gml:pos`. Semantics, attributes, BuildingParts,
//!   MultiSurface geometry, appearance, and non-building modules arrive in later
//!   milestones.
//! - **CRS:** accept by default (the pipeline never reprojects — CRS is
//!   provenance). Resolve `srsName` to an EPSG URL only via an explicit
//!   allowlist ([`crs`]); an unresolved name advertises no CRS. Reject only a
//!   name that resolves to a geographic (degree) CRS — the 1 mm quantiser would
//!   destroy degrees.
//! - **Transform:** one global `scale = [1 mm; 3]`, `translate = envelope lower
//!   corner` (or `[0, 0, 0]` when the document has no `gml:Envelope`).

pub mod crs;

mod building;
mod geometry;
mod header;
pub mod reader;
mod sniff;
mod vertices;
mod xml;

pub use header::parse_header;
pub use reader::FeatureReader;
pub use sniff::is_citygml;
