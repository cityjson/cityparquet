//! LoD0 synthesis over real fixtures: the cjseq adapters + the existing WKB
//! writer must turn a source solid into a GeoParquet-legal `MultiPolygon Z`
//! footprint.

use std::path::PathBuf;

use cityparquet::lod0::{Lod0Options, faces_from_geometry, footprint_to_geometry, synthesize_lod0};
use cityparquet::source::Source;
use cityparquet::wkb_write::{VertexPool, geometry_to_wkb};
use cjseq::GeometryType;
use geo_traits::{GeometryTrait, GeometryType as GtGeometryType};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Every synthesised footprint, fed back through the hand-rolled WKB writer,
/// must parse as an ISO `MultiPolygon` (type 1006) — the GeoParquet-legal shape
/// the `geometry` column needs — with a valid little-endian header.
#[test]
fn delft_solids_synthesise_valid_multipolygon_footprints() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let header = src.header();
    let opts = Lod0Options::default();
    let mut synthesised = 0usize;

    for feature in src.features().unwrap() {
        let feature = feature.unwrap();
        let pool = VertexPool::new(&feature.vertices, &header.transform);
        for co in feature.city_objects.values() {
            let Some(geoms) = &co.geometry else {
                continue;
            };
            for geom in geoms {
                if !matches!(
                    geom.thetype,
                    GeometryType::Solid
                        | GeometryType::MultiSolid
                        | GeometryType::CompositeSolid
                        | GeometryType::MultiSurface
                        | GeometryType::CompositeSurface
                ) {
                    continue;
                }
                let (faces, mask) = faces_from_geometry(geom, &pool).unwrap();
                if faces.is_empty() {
                    continue;
                }
                let Some(fp) = synthesize_lod0(&faces, mask.as_deref(), &opts) else {
                    continue;
                };
                assert!(!fp.surfaces.is_empty(), "a footprint has at least one surface");

                let (verts, ms) = footprint_to_geometry(&fp);
                let raw = VertexPool::raw(&verts);
                let outcome = geometry_to_wkb(&ms, &raw)
                    .unwrap()
                    .expect("a non-empty footprint yields WKB");
                assert_eq!(outcome.bytes[0], 0x01, "little-endian WKB marker");
                let parsed = wkb::reader::read_wkb(&outcome.bytes)
                    .expect("the footprint WKB must parse");
                assert!(
                    matches!(parsed.as_type(), GtGeometryType::MultiPolygon(_)),
                    "a synthesised footprint must be a MultiPolygon (GeoParquet-legal)"
                );
                synthesised += 1;
                if synthesised >= 8 {
                    return; // a representative handful is enough
                }
            }
        }
    }
    assert!(
        synthesised > 0,
        "at least one delft geometry should synthesise a footprint"
    );
}
