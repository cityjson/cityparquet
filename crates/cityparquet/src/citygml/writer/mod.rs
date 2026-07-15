//! Native CityGML 2.0 output writer (CityParquet package -> .gml).
//!
//! W-M1: CityModel + envelope + srsName, and bldg:Building with LoD gml:Solid.
//! Standalone — reuses wkb_read/reader/export shell helpers, no cjseq document.

pub mod document;
pub mod geometry;
