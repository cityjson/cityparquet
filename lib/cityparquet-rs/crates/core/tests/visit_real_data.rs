//! The in-place WKB visitor and the native object visit, against packages
//! converted from the real fixtures.
//!
//! Requires `just fixtures` to have been run first.

use std::path::{Path, PathBuf};

use arrow_array::{Array, BinaryArray, RecordBatch};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::wkb_read::{DecodedKind, WkbVisitor, visit_wkb, wkb_to_geometry};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn convert_fixture(name: &str) -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture(name),
        out.path().to_path_buf(),
    ))
    .unwrap();
    out
}

fn tables(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .collect();
    v.sort();
    v
}

fn batches(table: &Path) -> Vec<RecordBatch> {
    let file = std::fs::File::open(table).unwrap();
    ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap()
        .map(|b| b.unwrap())
        .collect()
}

#[derive(Default, Debug, PartialEq)]
struct Trace {
    coords: Vec<[u64; 3]>,
    rings: usize,
    polygons: usize,
}

impl WkbVisitor for Trace {
    fn coord(&mut self, c: [f64; 3]) {
        self.coords
            .push([c[0].to_bits(), c[1].to_bits(), c[2].to_bits()]);
    }
    fn ring_end(&mut self, _n: usize) {
        self.rings += 1;
    }
    fn polygon_end(&mut self, _n: usize) {
        self.polygons += 1;
    }
}

/// The owned decoder's geometry expanded to the trace the visitor must
/// produce: rings re-closed (the decoder strips the WKB closing vertex).
fn expand(kind: &DecodedKind, pool: &[[f64; 3]], t: &mut Trace) {
    let push = |t: &mut Trace, i: usize| {
        let c = pool[i];
        t.coords
            .push([c[0].to_bits(), c[1].to_bits(), c[2].to_bits()]);
    };
    match kind {
        DecodedKind::MultiPoint(pts) => pts.iter().for_each(|&i| push(t, i)),
        DecodedKind::MultiLineString(lines) => {
            for l in lines {
                l.iter().for_each(|&i| push(t, i));
                t.rings += 1;
            }
        }
        DecodedKind::MultiPolygon(polys) | DecodedKind::PolyhedralSurface(polys) => {
            for poly in polys {
                for ring in poly {
                    ring.iter().for_each(|&i| push(t, i));
                    push(t, ring[0]);
                    t.rings += 1;
                }
                t.polygons += 1;
            }
        }
        DecodedKind::GeometryCollection(members) => {
            members.iter().for_each(|m| expand(m, pool, t));
        }
    }
}

/// Property: on every geometry cell of every table converted from the delft
/// and railway fixtures, the visitor's vertex sequence equals the owned
/// decoder's expanded coordinates, and its ring/polygon counts match.
#[test]
fn visitor_matches_owned_decoder_on_every_fixture_geometry() {
    for name in ["delft.city.jsonl", "lod3_railway.city.json"] {
        let out = convert_fixture(name);
        let mut checked = 0usize;
        for table in tables(out.path()) {
            for batch in batches(&table) {
                for (field, col) in batch.schema().fields().iter().zip(batch.columns()) {
                    if !field.name().starts_with("geometry_lod") {
                        continue;
                    }
                    let wkb = col.as_any().downcast_ref::<BinaryArray>().unwrap();
                    for row in 0..wkb.len() {
                        if wkb.is_null(row) {
                            continue;
                        }
                        let bytes = wkb.value(row);
                        let owned = wkb_to_geometry(bytes).unwrap();
                        let mut want = Trace::default();
                        expand(&owned.kind, &owned.coords, &mut want);
                        let mut got = Trace::default();
                        visit_wkb(bytes, &mut got).unwrap();
                        assert_eq!(got, want, "{name} {} row {row}", field.name());
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "{name}: no geometry visited");
    }
}

/// Malformed input is an error, never a panic: every truncation of a real
/// geometry, and a corrupted type code, are rejected.
#[test]
fn visitor_rejects_truncated_and_corrupted_wkb() {
    let out = convert_fixture("delft.city.jsonl");
    let batch = &batches(&out.path().join("building.parquet"))[0];
    let col = batch
        .schema()
        .fields()
        .iter()
        .position(|f| f.name().starts_with("geometry_lod2"))
        .unwrap();
    let wkb = batch
        .column(col)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    let row = (0..wkb.len()).find(|&r| !wkb.is_null(r)).unwrap();
    let bytes = wkb.value(row);
    for cut in 0..bytes.len() {
        assert!(visit_wkb(&bytes[..cut], &mut Trace::default()).is_err());
    }
    let mut corrupt = bytes.to_vec();
    corrupt[1] = 0xFF;
    assert!(visit_wkb(&corrupt, &mut Trace::default()).is_err());
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(visit_wkb(&trailing, &mut Trace::default()).is_err());
}
