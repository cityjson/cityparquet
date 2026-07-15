//! Native CityGML 2.0 output writer (CityParquet package -> .gml).
//!
//! W-M1: CityModel + envelope + srsName, and bldg:Building with LoD gml:Solid.
//! Standalone — reuses wkb_read/reader/export shell helpers, no cjseq document.

pub mod building;
pub mod document;
pub mod geometry;

/// Per-conversion counts, mirroring the export report's drop-counter style.
/// Populated by [`write_package`](crate::citygml::writer) and the sub-writers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// `bldg:Building` elements emitted.
    pub buildings_written: usize,
    /// Rows skipped because `object_type` is not `Building` (W-M1 scope).
    pub non_building_skipped: usize,
    /// Building rows with no emittable Solid in any major LoD — skipped whole.
    pub buildings_without_solid_skipped: usize,
    /// LoD columns skipped because the WKB was MultiSolid/CompositeSolid
    /// (`GeometryCollection`) — deferred to W-M2.
    pub composite_solids_skipped: usize,
    /// LoD columns skipped because they collided on a major LoD already kept
    /// for that building, or mapped to an unrepresentable LoD (0, >4, lodless),
    /// or held a non-Solid geometry.
    pub lod_columns_skipped: usize,
}
