//! The in-place WKB visitor and the native object visit, against packages
//! converted from the real fixtures.
//!
//! Requires `just fixtures` to have been run first.

use std::path::{Path, PathBuf};

use arrow_array::{Array, BinaryArray, RecordBatch};
use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet::wkb_read::{DecodedKind, WkbVisitor, visit_wkb, wkb_to_geometry};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn convert_fixture(name: &str) -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    // Source order pinned: the tallies below are order-independent, but the
    // row-group assignments some tests rely on are not.
    let mut opts = ConvertOptions::new(fixture(name), out.path().to_path_buf());
    opts.ordering = RowOrder::Source;
    convert(&opts).unwrap();
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

/// Independent tally of the geometry and semantics the visit must read,
/// taken from plain Arrow access and the visitor property-tested above.
fn independent_tally(table: &Path) -> (u64, u64, u64, u64, [f64; 6]) {
    use arrow_array::{ListArray, StructArray};
    let (mut objects, mut geoms, mut coords, mut sem_faces) = (0u64, 0u64, 0u64, 0u64);
    let mut ext = [f64::INFINITY; 6];
    ext[3..].fill(f64::NEG_INFINITY);
    for batch in batches(table) {
        objects += batch.num_rows() as u64;
        for (field, col) in batch.schema().fields().iter().zip(batch.columns()) {
            if field.name().starts_with("geometry_lod") {
                let wkb = col.as_any().downcast_ref::<BinaryArray>().unwrap();
                for row in (0..wkb.len()).filter(|&r| !wkb.is_null(r)) {
                    geoms += 1;
                    let mut t = Trace::default();
                    visit_wkb(wkb.value(row), &mut t).unwrap();
                    coords += t.coords.len() as u64;
                    for c in &t.coords {
                        for k in 0..3 {
                            let v = f64::from_bits(c[k]);
                            ext[k] = ext[k].min(v);
                            ext[k + 3] = ext[k + 3].max(v);
                        }
                    }
                }
            } else if field.name().starts_with("geometry_properties_lod") {
                let props = col.as_any().downcast_ref::<StructArray>().unwrap();
                let fs = props.column_by_name("face_semantics").unwrap();
                let list = fs.as_any().downcast_ref::<ListArray>().unwrap();
                for row in (0..list.len()).filter(|&r| !list.is_null(r)) {
                    let items = list.value(row);
                    sem_faces += (items.len() - items.null_count()) as u64;
                }
            }
        }
    }
    (objects, geoms, coords, sem_faces, ext)
}

/// `full_read_visit` reads every object natively: its comparable totals
/// equal an independent tally over the same delft table, it parses the
/// semantic-surface objects, and it reads the attribute values.
#[test]
fn full_read_visit_reads_every_object_of_delft() {
    let out = convert_fixture("delft.city.jsonl");
    let table = out.path().join("building.parquet");
    let totals = cityparquet::query::full_read_visit(&table).unwrap();
    let (objects, geoms, coords, sem_faces, ext) = independent_tally(&table);
    assert_eq!(totals.objects, 2231);
    assert_eq!(totals.objects, objects);
    assert_eq!(totals.geometries, geoms);
    assert_eq!(totals.coordinates, coords);
    assert_eq!(totals.semantic_faces, sem_faces);
    assert_eq!(totals.extent, ext);
    assert!(totals.semantic_surface_objects > 0);
    assert!(totals.attribute_values > 0);
    assert!(totals.polygons > 0 && totals.rings >= totals.polygons);
}

fn convert_delft_small_row_groups() -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.recipe = cityparquet::recipe::WriterRecipe {
        row_group_size: 256,
        ..cityparquet::recipe::WriterRecipe::default()
    };
    // Which row groups prune depends on the row order, so it is pinned.
    opts.ordering = RowOrder::Source;
    convert(&opts).unwrap();
    out
}

/// The visit-based bbox, attribute and id primitives visit exactly the rows
/// the id/count primitives select, and report the same pruning counts.
#[test]
fn bbox_attr_and_id_visits_select_the_same_rows_as_the_id_paths() {
    use cityparquet::query::{self, AttrPredicate};
    let out = convert_delft_small_row_groups();
    let table = out.path().join("building.parquet");
    let e = query::full_read_visit(&table).unwrap().extent;
    let window = [
        e[0],
        e[1],
        e[2],
        (e[0] + e[3]) / 2.0,
        (e[1] + e[4]) / 2.0,
        e[5],
    ];

    let ids = query::bbox_query(&table, window).unwrap();
    let visited = query::bbox_query_visit(&table, window).unwrap();
    assert!(!ids.ids.is_empty() && ids.ids.len() < 2231);
    assert_eq!(visited.totals.objects, ids.ids.len() as u64);
    assert!(visited.totals.geometries >= visited.totals.objects);
    assert_eq!(visited.row_groups_total, ids.row_groups_total);
    assert_eq!(visited.row_groups_touched, ids.row_groups_touched);

    let pred = AttrPredicate::Eq(serde_json::Value::String("BuildingPart".into()));
    let (attr, stats) = query::attr_filter_visit(&table, "object_type", &pred).unwrap();
    assert_eq!(attr.objects, 1116);
    assert_eq!(
        stats,
        query::attr_filter_with_stats(&table, "object_type", &pred)
            .unwrap()
            .1
    );

    let (hit, stats) = query::id_lookup_visit(&table, &ids.ids[0]).unwrap();
    assert_eq!(hit.objects, 1);
    assert!(hit.geometries >= 1);
    assert!(stats.bloom_pruned + stats.stats_pruned < stats.row_groups_total);
    let (miss, stats) = query::id_lookup_visit(&table, "no-such-id").unwrap();
    assert_eq!(miss.objects, 0);
    assert_eq!(
        stats.bloom_pruned + stats.stats_pruned,
        stats.row_groups_total,
        "every row group is ruled out for an absent id: {stats:?}"
    );
}

/// Row-group statistics pruning never drops a matching row: across a sweep
/// of thresholds over delft written with small row groups, the pruned count
/// equals an unpruned scan, and some thresholds do prune.
#[test]
fn attr_filter_statistics_pruning_matches_the_unpruned_scan() {
    use cityparquet::query::{self, AttrPredicate};
    let out = convert_delft_small_row_groups();
    let table = out.path().join("building.parquet");
    let years: Vec<f64> = batches(&table)
        .iter()
        .flat_map(|b| {
            let a = b
                .column_by_name("oorspronkelijkbouwjaar")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .unwrap()
                .clone();
            a.iter().flatten().map(|v| v as f64).collect::<Vec<_>>()
        })
        .collect();
    let mut pruned_any = false;
    for t in (1200..=2030).step_by(10).map(f64::from) {
        for (pred, want) in [
            (
                AttrPredicate::Ge(t),
                years.iter().filter(|&&y| y >= t).count(),
            ),
            (
                AttrPredicate::Le(t),
                years.iter().filter(|&&y| y <= t).count(),
            ),
            (
                AttrPredicate::Eq(t.into()),
                years.iter().filter(|&&y| y == t).count(),
            ),
            (
                AttrPredicate::Range(t, t + 25.0),
                years.iter().filter(|&&y| y >= t && y <= t + 25.0).count(),
            ),
        ] {
            let (count, got) =
                query::attr_filter_with_stats(&table, "oorspronkelijkbouwjaar", &pred).unwrap();
            assert_eq!(count, want as u64, "{pred:?}");
            assert_eq!(
                query::attr_filter(&table, "oorspronkelijkbouwjaar", &pred).unwrap(),
                want as u64
            );
            assert_eq!(
                got.bloom_pruned, 0,
                "no bloom filter answers a numeric predicate"
            );
            assert!(got.stats_pruned <= got.row_groups_total);
            pruned_any |= got.stats_pruned > 0;
        }
    }
    assert!(pruned_any, "no threshold pruned a row group");
    // String equality: a value outside every row group's [min, max] prunes
    // them all; a present value keeps its rows.
    let none = AttrPredicate::Eq("~~~~ not an id".into());
    let (count, got) = query::attr_filter_with_stats(&table, "id", &none).unwrap();
    assert_eq!(
        (count, got.bloom_pruned + got.stats_pruned),
        (0, got.row_groups_total)
    );
    let pred = AttrPredicate::Eq(serde_json::Value::String("BuildingPart".into()));
    let (count, _) = query::attr_filter_with_stats(&table, "object_type", &pred).unwrap();
    assert_eq!(count, 1116);
}

/// `feature_lookup_visit` visits exactly the rows `feature_lookup` decodes —
/// a building and all its parts — with the same pruning counts.
#[test]
fn feature_lookup_visit_visits_every_row_of_the_feature() {
    use cityparquet::query;
    use cityparquet::reader::CityParquetReaderBuilder;
    let out = convert_delft_small_row_groups();
    let table = out.path().join("building.parquet");
    let meta = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap();
    let ids = query::bbox_query(
        &table,
        [f64::MIN, f64::MIN, f64::MIN, f64::MAX, f64::MAX, f64::MAX],
    )
    .unwrap()
    .ids;
    let part = ids
        .iter()
        .find(|id| id.contains('-'))
        .expect("delft has parts");
    let feature = part.split('-').next().unwrap();
    let (objects, want) = query::feature_lookup_with_stats(&table, &meta, feature).unwrap();
    assert!(objects.len() >= 2, "a building with a part");
    let (totals, stats) = query::feature_lookup_visit(&table, feature).unwrap();
    assert_eq!(totals.objects, objects.len() as u64);
    assert_eq!(stats, want);
    let (miss, stats) = query::feature_lookup_visit(&table, "NL.IMBAG.Pand.absent").unwrap();
    assert_eq!(miss.objects, 0);
    assert_eq!(
        stats.bloom_pruned + stats.stats_pruned,
        stats.row_groups_total
    );
}

/// The geometries a CityJSON source carries, excluding `GeometryInstance`
/// (stored in `template`, not in a `geometry_lod*` column).
fn source_geometry_count(source: &Path) -> u64 {
    let text = std::fs::read_to_string(source).unwrap();
    let docs: Vec<serde_json::Value> = if source.extension().is_some_and(|e| e == "jsonl") {
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    } else {
        vec![serde_json::from_str(&text).unwrap()]
    };
    docs.iter()
        .filter_map(|d| d.get("CityObjects").and_then(|c| c.as_object()))
        .flat_map(|objects| objects.values())
        .filter_map(|o| o.get("geometry").and_then(|g| g.as_array()))
        .flatten()
        .filter(|g| g["type"] != "GeometryInstance")
        .count() as u64
}

/// A `geometry_lod*` column holds one geometry per object per LoD; its
/// `lod<major>_<minor>` suffix is the LoD, not an ordinal. A second geometry
/// at the same LoD on one object is not stored — the writer keeps the first
/// and counts the rest in `ConvertReport::skipped_same_lod_geometries` — so
/// with LoD0 synthesis off `VisitTotals::geometries` equals the source's
/// geometry entries less that count.
#[test]
fn visit_geometries_equal_source_geometries_less_same_lod_skips() {
    use cityparquet::query;
    for name in ["delft.city.jsonl", "lod3_railway.city.json"] {
        let out = tempfile::tempdir().unwrap();
        let mut opts = ConvertOptions::new(fixture(name), out.path().to_path_buf());
        opts.generate_lod0 = false;
        let report = convert(&opts).unwrap();
        // The object tables only: the `geometry_templates` sidecar holds the
        // templates `GeometryInstance` entries refer to.
        let visited: u64 = cityparquet::stac::properties::PackageTables::open(out.path())
            .unwrap()
            .tables
            .iter()
            .map(|t| query::full_read_visit(t).unwrap().geometries)
            .sum();
        assert_eq!(
            visited + report.skipped_same_lod_geometries as u64,
            source_geometry_count(&fixture(name)),
            "{name}"
        );
    }
}

/// Independent oracle for the spatial geometry query: for every row whose id
/// is in `ids`, the most detailed non-null `geometry_lod*` column (ranked by
/// the parsed `Lod`, the un-suffixed primary `geometry` lowest), walked with
/// a coordinate counter. Returns `(rows with a geometry, coordinates)`.
fn highest_lod_oracle(table: &Path, ids: &std::collections::HashSet<String>) -> (u64, u64) {
    use cityparquet_schema::Lod;
    struct Count(u64);
    impl WkbVisitor for Count {
        fn coord(&mut self, _c: [f64; 3]) {
            self.0 += 1;
        }
    }
    let (mut geometries, mut coords) = (0u64, Count(0));
    for batch in batches(table) {
        let schema = batch.schema();
        let id = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        let mut lods: Vec<(Option<Lod>, usize)> = schema
            .fields()
            .iter()
            .enumerate()
            .filter_map(|(i, f)| match f.name().as_str() {
                "geometry" => Some((None, i)),
                n => n
                    .strip_prefix("geometry_")
                    .and_then(Lod::from_column_suffix)
                    .map(|l| (Some(l), i)),
            })
            .collect();
        lods.sort();
        for row in 0..batch.num_rows() {
            if !ids.contains(id.value(row)) {
                continue;
            }
            for &(_, i) in lods.iter().rev() {
                let col = batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                if col.is_valid(row) {
                    geometries += 1;
                    visit_wkb(col.value(row), &mut coords).unwrap();
                    break;
                }
            }
        }
    }
    (geometries, coords.0)
}

/// The spatial geometry query returns exactly the ids the id-only bbox query
/// returns, and one geometry per object that has any — its most detailed
/// LoD — walked in place; an object without geometry keeps its id.
#[test]
fn bbox_query_geometry_returns_ids_and_the_highest_lod_geometry() {
    use cityparquet::query;
    let mut geometry_less = 0u64;
    for (out, table) in [
        {
            let out = convert_delft_small_row_groups();
            let t = out.path().join("building.parquet");
            (out, t)
        },
        {
            let out = convert_fixture("lod3_railway.city.json");
            let t = tables(out.path()).remove(0);
            (out, t)
        },
    ] {
        let e = query::full_read_visit(&table).unwrap().extent;
        let half = [
            e[0],
            e[1],
            e[2],
            (e[0] + e[3]) / 2.0,
            (e[1] + e[4]) / 2.0,
            e[5],
        ];
        for window in [e, half] {
            let ids = query::bbox_query(&table, window).unwrap();
            let got = query::bbox_query_geometry(&table, window).unwrap();
            assert_eq!(got.ids, ids.ids, "{}", table.display());
            assert_eq!(got.row_groups_total, ids.row_groups_total);
            assert_eq!(got.row_groups_touched, ids.row_groups_touched);
            let set: std::collections::HashSet<String> = ids.ids.iter().cloned().collect();
            let (geometries, coordinates) = highest_lod_oracle(&table, &set);
            assert_eq!(got.geometries, geometries);
            assert_eq!(got.coordinates, coordinates);
            assert!(got.geometries <= got.ids.len() as u64);
            geometry_less += got.ids.len() as u64 - got.geometries;
            if got.coordinates > 0 {
                assert!(got.extent[0] <= got.extent[3] && got.extent[2] <= got.extent[5]);
            }
        }
        drop(out);
    }
    // Every object of both fixtures carries a geometry; the geometry-less
    // case is covered by the next test.
    assert_eq!(geometry_less, 0);
}

/// An object without geometry is returned with its id and no geometry. The
/// table is delft converted, then rewritten with every `Building` row's
/// geometry columns nulled — the shape of a 3DBAG parent without LoD 0.
#[test]
fn bbox_query_geometry_returns_a_geometry_less_object_by_id() {
    use arrow_array::{BooleanArray, cast::AsArray};
    use cityparquet::query;
    let out = convert_delft_small_row_groups();
    let source = out.path().join("building.parquet");
    let stripped = out.path().join("stripped.parquet");
    let input = batches(&source);
    let schema = input[0].schema();
    let file = std::fs::File::create(&stripped).unwrap();
    let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema.clone(), None).unwrap();
    let mut buildings = 0u64;
    for batch in &input {
        let ty = batch.column_by_name("object_type").unwrap();
        let is_building = |row: usize| -> bool {
            if let Some(d) = ty.as_dictionary_opt::<arrow_array::types::Int32Type>() {
                let values = d.values().as_string::<i32>();
                d.is_valid(row) && values.value(d.keys().value(row) as usize) == "Building"
            } else {
                let plain = ty.as_string::<i32>();
                plain.is_valid(row) && plain.value(row) == "Building"
            }
        };
        let strip: BooleanArray = (0..batch.num_rows())
            .map(|r| Some(is_building(r)))
            .collect();
        buildings += strip.true_count() as u64;
        let columns = schema
            .fields()
            .iter()
            .zip(batch.columns())
            .map(|(f, c)| {
                if f.name().starts_with("geometry_lod") {
                    arrow_select::nullif::nullif(c, &strip).unwrap()
                } else {
                    c.clone()
                }
            })
            .collect();
        writer
            .write(&RecordBatch::try_new(schema.clone(), columns).unwrap())
            .unwrap();
    }
    writer.close().unwrap();
    assert!(buildings > 0);

    let e = query::full_read_visit(&source).unwrap().extent;
    let full = query::bbox_query_geometry(&source, e).unwrap();
    let got = query::bbox_query_geometry(&stripped, e).unwrap();
    assert_eq!(got.ids, full.ids, "a geometry-less object keeps its id");
    let set: std::collections::HashSet<String> = got.ids.iter().cloned().collect();
    let (geometries, coordinates) = highest_lod_oracle(&stripped, &set);
    assert_eq!(got.geometries, geometries);
    assert_eq!(got.coordinates, coordinates);
    assert_eq!(got.ids.len() as u64 - got.geometries, buildings);
}

/// The id-returning attribute filter selects the rows the counting filter
/// counts, under the same bloom-then-statistics pruning.
#[test]
fn attr_filter_ids_returns_the_matching_ids_with_the_same_pruning() {
    use cityparquet::query::{self, AttrPredicate};
    let out = convert_delft_small_row_groups();
    let table = out.path().join("building.parquet");
    let pred = AttrPredicate::Eq(serde_json::Value::String("BuildingPart".into()));
    let (ids, stats) = query::attr_filter_ids(&table, "object_type", &pred).unwrap();
    let (count, expected) = query::attr_filter_with_stats(&table, "object_type", &pred).unwrap();
    assert_eq!(ids.len() as u64, count);
    assert_eq!(count, 1116);
    assert_eq!(stats, expected);
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());

    let some = ids[0].clone();
    let eq = AttrPredicate::Eq(serde_json::Value::String(some.clone()));
    let (hit, stats) = query::attr_filter_ids(&table, "id", &eq).unwrap();
    assert_eq!(hit, vec![some]);
    assert!(stats.bloom_pruned + stats.stats_pruned < stats.row_groups_total);
    let miss = AttrPredicate::Eq(serde_json::Value::String("no-such-id".into()));
    let (none, stats) = query::attr_filter_ids(&table, "id", &miss).unwrap();
    assert!(none.is_empty());
    assert_eq!(
        stats.bloom_pruned + stats.stats_pruned,
        stats.row_groups_total
    );
}
