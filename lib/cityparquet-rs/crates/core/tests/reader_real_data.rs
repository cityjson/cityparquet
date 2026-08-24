//! RED (M3 task 4): the reader extension trait, exercised against a real
//! converted delft package — metadata/schema round-trip plus bbox row-group
//! pruning.

use std::path::PathBuf;

use arrow_array::Array;
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use cityparquet::recipe::WriterRecipe;
use cityparquet::scan::scan;
use cityparquet::source::Source;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Field metadata key CityParquet tags every reserved/attribute/extension
/// column with (`cityparquet_schema::model::ROLE_KEY`, not re-exported at the
/// crate root).
const ROLE_KEY: &str = "cityparquet:role";

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Convert delft with a small row_group_size (256) so the file has enough
/// row groups for pruning to be observable, and return the output dir plus
/// the writer-side scan/metadata for spot-checking against the reader.
fn convert_delft_small_row_groups() -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.recipe = WriterRecipe {
        row_group_size: 256,
        ..WriterRecipe::default()
    };
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);
    out
}

/// RED (G8 sol-review Finding 1): a source ATTRIBUTE named like a geometry
/// column for a LoD the dataset does not otherwise use (e.g. `geometry_lod3`
/// in a LoD-2-only dataset) is legal — only geometry columns for the dataset's
/// ACTUAL LoDs are reserved. The reader must not mistake that attribute for a
/// geometry column when it derives the LoD set from the file's own schema
/// (§13.1: a column listed in `attributes` is an attribute, not reserved).
/// Derived from delft by injecting the attribute; reading the package back
/// must not error.
#[test]
fn attribute_named_like_a_geometry_column_does_not_break_the_reader() {
    let mut doc: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(fixture("delft.city.jsonl"))
            .unwrap()
            .lines()
            .nth(1)
            .expect("delft has feature lines"),
    )
    .unwrap();
    for (_, co) in doc["CityObjects"].as_object_mut().unwrap() {
        co.as_object_mut()
            .unwrap()
            .entry("attributes")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .unwrap()
            .insert("geometry_lod3".to_string(), serde_json::json!("sneaky"));
    }
    let src_dir = tempfile::tempdir().unwrap();
    let path = src_dir.path().join("delft_attr_collide.city.jsonl");
    let header = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    std::fs::write(
        &path,
        format!(
            "{}\n{}",
            header.lines().next().unwrap(),
            serde_json::to_string(&doc).unwrap()
        ),
    )
    .unwrap();

    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(path, out.path().to_path_buf())).unwrap();

    // Reading the package back must reconstruct the schema without mistaking
    // the `geometry_lod3` ATTRIBUTE for a geometry LoD (which would collide).
    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let schema = builder
        .cityparquet_arrow_schema()
        .expect("reader must not mis-infer the geometry_lod3 attribute as a geometry column");
    // `geometry_lod3` is present exactly once, as an attribute (role=attribute).
    let field = schema.field_with_name("geometry_lod3").unwrap();
    assert_eq!(
        field.metadata().get(ROLE_KEY).map(String::as_str),
        Some("attribute"),
        "geometry_lod3 must remain an attribute column, not become a reserved geometry column"
    );
}

#[test]
fn cityparquet_metadata_matches_the_writer_side_scan() {
    let out = convert_delft_small_row_groups();

    // Writer side, independently: what the scan pass says the DATASET-WIDE
    // portion of the metadata is (spec-alignment M3: `columns`/`primary_column`
    // are per-FILE now, computed post-encode — see
    // `scan_real_data.rs::delft_city_and_geo_for_file_has_independent_primaries`
    // for that half).
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let writer_meta = scan_result.base_city_metadata().unwrap();

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let read_meta = builder.cityparquet_metadata().unwrap();

    // city.primary_column is the highest LoD present, solids included — for
    // delft that is 2.2 (a real Solid), never the 0.*-family preference
    // (that preference is `geo.primary_column`'s rule, not `city`'s — spec
    // "Why city.primary_column and geo.primary_column can differ").
    assert_eq!(read_meta.primary_column.as_deref(), Some("geometry_lod2_2"));
    assert_eq!(read_meta.attributes.len(), 50);
    assert_eq!(read_meta.attributes.len(), writer_meta.attributes.len());
    assert_eq!(read_meta.version, writer_meta.version);
}

#[test]
fn cityparquet_arrow_schema_matches_the_writers_rendered_schema() {
    let out = convert_delft_small_row_groups();

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let writer_schema = scan_result.schema.to_arrow_schema().unwrap();

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let read_schema = builder.cityparquet_arrow_schema().unwrap();

    let writer_names: Vec<&str> = writer_schema
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    let read_names: Vec<&str> = read_schema
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(read_names, writer_names);

    for (writer_field, read_field) in writer_schema
        .fields()
        .iter()
        .zip(read_schema.fields().iter())
    {
        assert_eq!(
            writer_field.data_type(),
            read_field.data_type(),
            "field {} data type mismatch",
            writer_field.name()
        );
    }

    // Extension/role metadata present on geometry and JSON columns.
    let geom = read_schema.field_with_name("geometry_lod2_2").unwrap();
    assert_eq!(
        geom.metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("geoarrow.wkb")
    );
    assert_eq!(
        geom.metadata().get(ROLE_KEY).map(String::as_str),
        Some("reserved")
    );

    // geometry_properties_lod2_2 is a genuine Arrow STRUCT on the WRITTEN,
    // then re-read, Parquet file — not a single arrow.json-tagged Utf8 leaf
    // (spec "Geometry properties and semantics"). Checked against the
    // actual physical schema Parquet round-tripped, not just the schema the
    // writer intended to render.
    let props = read_schema
        .field_with_name("geometry_properties_lod2_2")
        .unwrap();
    assert!(
        !props.metadata().contains_key(EXTENSION_TYPE_NAME_KEY),
        "the outer geometry_properties field is a Struct, not itself arrow.json-tagged"
    );
    let arrow_schema::DataType::Struct(children) = props.data_type() else {
        panic!(
            "geometry_properties_lod2_2 must round-trip as a Struct, got {:?}",
            props.data_type()
        );
    };
    assert_eq!(
        children
            .iter()
            .map(|f| f.name().as_str())
            .collect::<Vec<_>>(),
        vec!["type", "surfaces", "face_semantics", "shells"]
    );
    let type_field = children.iter().find(|f| f.name() == "type").unwrap();
    assert_eq!(type_field.data_type(), &arrow_schema::DataType::Utf8);
    assert!(!type_field.is_nullable(), "type is non-null");

    let surfaces_field = children.iter().find(|f| f.name() == "surfaces").unwrap();
    assert_eq!(surfaces_field.data_type(), &arrow_schema::DataType::Utf8);
    assert!(surfaces_field.is_nullable());
    assert_eq!(
        surfaces_field
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("arrow.json"),
        "surfaces alone keeps the arrow.json tag (heterogeneous per-surface attributes)"
    );

    let fs_field = children
        .iter()
        .find(|f| f.name() == "face_semantics")
        .unwrap();
    assert!(fs_field.is_nullable());
    let arrow_schema::DataType::List(fs_item) = fs_field.data_type() else {
        panic!("face_semantics must round-trip as List");
    };
    assert_eq!(fs_item.data_type(), &arrow_schema::DataType::Int32);
    assert!(fs_item.is_nullable(), "face_semantics items are nullable");

    let shells_field = children.iter().find(|f| f.name() == "shells").unwrap();
    assert!(shells_field.is_nullable());
    let arrow_schema::DataType::List(solid_item) = shells_field.data_type() else {
        panic!("shells must round-trip as List");
    };
    assert!(
        !solid_item.is_nullable(),
        "each solid's inner shell-count list is non-null once shells is populated"
    );
    let arrow_schema::DataType::List(count_item) = solid_item.data_type() else {
        panic!("shells' items must themselves round-trip as List");
    };
    assert_eq!(count_item.data_type(), &arrow_schema::DataType::Int32);
    assert!(
        !count_item.is_nullable(),
        "each per-shell face count is non-null once shells is populated"
    );

    // An inferred attribute keeps its role metadata too.
    let some_attr = read_schema
        .fields()
        .iter()
        .find(|f| f.metadata().get(ROLE_KEY).map(String::as_str) == Some("attribute"))
        .expect("at least one attribute column");
    assert_eq!(
        some_attr.metadata().get(ROLE_KEY).map(String::as_str),
        Some("attribute")
    );
}

#[test]
fn record_batch_reader_yields_all_rows_with_schema_metadata_preserved() {
    let out = convert_delft_small_row_groups();

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let rendered_schema = builder.cityparquet_arrow_schema().unwrap();
    let parquet_reader = builder.build().unwrap();
    let reader = CityParquetRecordBatchReader::new(parquet_reader, rendered_schema.clone());

    assert_eq!(
        reader
            .schema()
            .field_with_name("geometry_lod2_2")
            .unwrap()
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("geoarrow.wkb")
    );

    let mut total_rows = 0usize;
    for batch in reader {
        let batch = batch.unwrap();
        assert_eq!(
            batch
                .schema()
                .field_with_name("geometry_lod2_2")
                .unwrap()
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            Some("geoarrow.wkb"),
            "every emitted batch must carry the rendered schema's field metadata"
        );
        total_rows += batch.num_rows();
    }
    assert_eq!(total_rows, 2231);
}

/// `parquet`'s own reconstructed schema (from the embedded `ARROW:schema`)
/// carries the Parquet footer's key-value metadata — the `city`/`geo` KVs —
/// on `RecordBatch::schema().metadata()`. `CityParquetRecordBatchReader::next`
/// rebuilds the batch's schema from scratch (`Schema::new(fields)`) to
/// re-attach field metadata by name; that rebuild must not drop the
/// SCHEMA-level footer metadata it did not touch.
#[test]
fn record_batch_reader_preserves_the_footers_schema_level_metadata() {
    let out = convert_delft_small_row_groups();

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let rendered_schema = builder.cityparquet_arrow_schema().unwrap();
    let parquet_reader = builder.build().unwrap();
    let reader = CityParquetRecordBatchReader::new(parquet_reader, rendered_schema);

    for batch in reader {
        let batch = batch.unwrap();
        assert!(
            batch.schema().metadata().contains_key("city"),
            "every emitted batch's schema must still carry the footer's own \
             'city' key-value metadata, got: {:?}",
            batch.schema().metadata()
        );
    }
}

#[test]
fn with_bbox_row_groups_prunes_a_tight_corner_query_but_keeps_the_whole_extent() {
    let out = convert_delft_small_row_groups();

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let dataset_bbox = scan_result
        .dataset_bbox
        .expect("delft has geometry, so a dataset bbox");

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let num_row_groups = builder.metadata().num_row_groups();
    assert!(
        num_row_groups > 1,
        "expected multiple row groups at row_group_size 256 for 2231 rows, got {num_row_groups}"
    );

    // A tiny box pinned to the dataset's minimum corner: strictly smaller
    // than the whole extent on every axis, so most row groups should not
    // intersect it.
    let span = [
        dataset_bbox[3] - dataset_bbox[0],
        dataset_bbox[4] - dataset_bbox[1],
        dataset_bbox[5] - dataset_bbox[2],
    ];
    let corner_bbox: [f64; 6] = [
        dataset_bbox[0],
        dataset_bbox[1],
        dataset_bbox[2],
        dataset_bbox[0] + span[0] * 0.01,
        dataset_bbox[1] + span[1] * 0.01,
        dataset_bbox[2] + span[2] * 0.01,
    ];

    let corner_builder = builder.with_bbox_row_groups(corner_bbox).unwrap();
    let corner_rows: usize = corner_builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert!(
        corner_rows < 2231,
        "corner bbox query should prune at least one row group, got {corner_rows} rows"
    );

    let file2 = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder2 = ParquetRecordBatchReaderBuilder::try_new(file2).unwrap();
    let whole_builder = builder2.with_bbox_row_groups(dataset_bbox).unwrap();
    let whole_rows: usize = whole_builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(
        whole_rows, 2231,
        "a bbox covering the whole dataset extent must keep every row"
    );
}

/// This row group's `bbox.<leaf>` Double statistic: `min` when `want_min`,
/// else `max`. Panics if the chunk or statistic is missing — the recipe
/// guarantees chunk statistics on every bbox leaf, so absence here is a
/// writer regression the test should surface, not silently keep.
fn bbox_stat(rg: &parquet::file::metadata::RowGroupMetaData, leaf: &str, want_min: bool) -> f64 {
    use parquet::file::statistics::Statistics;
    use parquet::schema::types::ColumnPath;
    let path = ColumnPath::new(vec!["bbox".to_string(), leaf.to_string()]);
    let stats = rg
        .columns()
        .iter()
        .find(|c| c.column_path() == &path)
        .unwrap_or_else(|| panic!("no column chunk for bbox.{leaf}"))
        .statistics()
        .unwrap_or_else(|| panic!("no statistics on bbox.{leaf}"));
    let Statistics::Double(v) = stats else {
        panic!("bbox.{leaf} statistics are not Double: {stats:?}");
    };
    let value = if want_min { v.min_opt() } else { v.max_opt() };
    *value.unwrap_or_else(|| {
        panic!(
            "no {} value on bbox.{leaf}",
            if want_min { "min" } else { "max" }
        )
    })
}

#[test]
fn with_bbox_row_groups_selects_a_strict_subset_for_a_partial_band_query() {
    // The corner/whole-extent test above only pins the extremes (0 kept /
    // all kept); a contains-vs-intersects or axis-swap regression could slip
    // through it. This test derives — from the file's own row-group
    // statistics, never hardcoded coordinates — a y-band that the stats say
    // intersects SOME but not ALL row groups, and asserts the reader keeps
    // exactly the rows of that strict subset.
    let out = convert_delft_small_row_groups();

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let dataset_bbox = scan_result.dataset_bbox.unwrap();

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let metadata = builder.metadata().clone();
    let num_row_groups = metadata.num_row_groups();
    assert!(
        num_row_groups > 1,
        "need multiple row groups to discriminate"
    );

    // Each row group's y interval per its own statistics.
    let y_intervals: Vec<(f64, f64)> = (0..num_row_groups)
        .map(|i| {
            let rg = metadata.row_group(i);
            (bbox_stat(rg, "ymin", true), bbox_stat(rg, "ymax", false))
        })
        .collect();

    // Band = [global y-min, halfway to the second-lowest row-group ymin]:
    // by construction it intersects the group(s) starting at the global
    // minimum and, per the stats, excludes every group starting above the
    // midpoint. Purely derived from the file, so it survives fixture and
    // layout changes as long as the groups do not all share one ymin.
    let mut starts: Vec<f64> = y_intervals.iter().map(|(lo, _)| *lo).collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let global_lo = starts[0];
    let next_lo = starts
        .iter()
        .copied()
        .find(|&lo| lo > global_lo)
        .expect("all row groups share the same bbox.ymin; no y-band can discriminate — fixture/layout no longer supports this test");
    let band_hi = global_lo + (next_lo - global_lo) / 2.0;
    let band: [f64; 6] = [
        dataset_bbox[0],
        global_lo,
        dataset_bbox[2],
        dataset_bbox[3],
        band_hi,
        dataset_bbox[5],
    ];

    // What the stats say the band should keep.
    let expected_kept: Vec<usize> = (0..num_row_groups)
        .filter(|&i| {
            let (lo, hi) = y_intervals[i];
            hi >= band[1] && lo <= band[4]
        })
        .collect();
    let expected_rows: usize = expected_kept
        .iter()
        .map(|&i| metadata.row_group(i).num_rows() as usize)
        .sum();
    assert!(
        !expected_kept.is_empty() && expected_kept.len() < num_row_groups,
        "derived band must select a strict, non-empty subset per the stats; \
         got {}/{num_row_groups} groups — fixture/layout no longer supports this test",
        expected_kept.len()
    );
    eprintln!(
        "partial y-band [{}, {}] keeps groups {expected_kept:?} of {num_row_groups} ({expected_rows} rows)",
        band[1], band[4]
    );

    let band_builder = builder.with_bbox_row_groups(band).unwrap();
    let band_rows: usize = band_builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert!(
        band_rows > 0 && band_rows < 2231,
        "partial band should keep some but not all rows, got {band_rows}"
    );
    assert_eq!(
        band_rows, expected_rows,
        "reader kept different row groups than the statistics predict \
         (expected groups {expected_kept:?} = {expected_rows} rows)"
    );
}

/// Every row group in `dir`'s `building.parquet` whose `bbox` chunk
/// statistics 3D-intersect `query` — a from-scratch recount straight off
/// the file's own row-group stats (mirrors `crate::reader`'s private
/// `row_group_intersects`, which this integration-test crate cannot reach),
/// used only to COUNT groups touched rather than to build a reader.
fn row_groups_touching(dir: &std::path::Path, query: [f64; 6]) -> usize {
    let file = std::fs::File::open(dir.join("building.parquet")).unwrap();
    let metadata = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .metadata()
        .clone();
    (0..metadata.num_row_groups())
        .filter(|&i| {
            let rg = metadata.row_group(i);
            let lo = [
                bbox_stat(rg, "xmin", true),
                bbox_stat(rg, "ymin", true),
                bbox_stat(rg, "zmin", true),
            ];
            let hi = [
                bbox_stat(rg, "xmax", false),
                bbox_stat(rg, "ymax", false),
                bbox_stat(rg, "zmax", false),
            ];
            (0..3).all(|k| lo[k] <= query[k + 3] && hi[k] >= query[k])
        })
        .count()
}

/// The first row with a non-null `bbox` a plain (unfiltered) read of `dir`'s
/// `building.parquet` yields, in on-disk order. Used to derive a query
/// bbox anchored to a REAL, existing geometry — unlike the corner-bbox in
/// `with_bbox_row_groups_prunes_a_tight_corner_query_but_keeps_the_whole_extent`
/// above (which independently takes the dataset bbox's per-axis minima and
/// combines them into one corner point), that combined point is not
/// necessarily where any actual geometry sits: delft's x-minimum and
/// y-minimum vertices belong to two different buildings, so the corner
/// formed by combining them can fall in an empty gap with NO row group
/// statistics box actually covering it, at all — which is exactly what
/// makes it unsuitable for A PAYOFF comparison (both orderings would
/// legitimately prune every group, a tie, not evidence of anything).
fn first_real_bbox_row(dir: &std::path::Path) -> [f64; 6] {
    use arrow_array::StructArray;
    let file = std::fs::File::open(dir.join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    for batch in builder.build().unwrap() {
        let batch = batch.unwrap();
        let bbox_col: &StructArray = batch
            .column_by_name("bbox")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        for row in 0..batch.num_rows() {
            if bbox_col.is_null(row) {
                continue;
            }
            let leaf = |name: &str| -> f64 {
                bbox_col
                    .column_by_name(name)
                    .unwrap()
                    .as_any()
                    .downcast_ref::<arrow_array::Float64Array>()
                    .unwrap()
                    .value(row)
            };
            return [
                leaf("xmin"),
                leaf("ymin"),
                leaf("zmin"),
                leaf("xmax"),
                leaf("ymax"),
                leaf("zmax"),
            ];
        }
    }
    panic!("no row in {} has a non-null bbox", dir.display());
}

/// M5 task 4's headline pruning payoff: converting delft with
/// `RowOrder::Hilbert` instead of `RowOrder::Source` (same small
/// `row_group_size` as `convert_delft_small_row_groups`, so both packages
/// have the same number of row groups to discriminate between) must make a
/// spatially-tight query around a REAL geometry (see
/// [`first_real_bbox_row`]'s doc comment for why the independent-per-axis
/// "corner" the earlier pruning tests use is unsuitable for a PAYOFF
/// comparison specifically) touch STRICTLY FEWER row groups: Hilbert
/// ordering clusters spatially nearby features together, so the query
/// should hit a smaller, more contiguous run of row groups than the plain
/// document-order (lexicographic feature id) package leaves scattered
/// across the whole file.
#[test]
fn hilbert_ordering_prunes_more_row_groups_than_source_ordering_on_a_real_neighbourhood_query() {
    let source_out = convert_delft_small_row_groups();

    let hilbert_out = tempfile::tempdir().unwrap();
    let mut hilbert_opts = ConvertOptions::new(
        fixture("delft.city.jsonl"),
        hilbert_out.path().to_path_buf(),
    );
    hilbert_opts.recipe = WriterRecipe {
        row_group_size: 256,
        ..WriterRecipe::default()
    };
    hilbert_opts.ordering = RowOrder::Hilbert;
    let hilbert_report = convert(&hilbert_opts).unwrap();
    assert_eq!(hilbert_report.object_count, 2231);

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let dataset_bbox = scan_result
        .dataset_bbox
        .expect("delft has geometry, so a dataset bbox");
    let span = [
        dataset_bbox[3] - dataset_bbox[0],
        dataset_bbox[4] - dataset_bbox[1],
        dataset_bbox[5] - dataset_bbox[2],
    ];

    // Anchor the query at a REAL geometry's own bbox centre (the first
    // non-null `bbox` row of the Source-ordered package — both packages
    // encode the exact same set of real-world geometries, just in a
    // different row order, so this point exists in the Hilbert package
    // too), padded by 2% of the dataset span per axis: wide enough to
    // sweep in a handful of spatially NEARBY buildings (delft's building
    // footprints are metres across against a ~1.2 km dataset span), narrow
    // enough to stay a small fraction of the whole extent.
    let anchor = first_real_bbox_row(source_out.path());
    let centre = [
        (anchor[0] + anchor[3]) / 2.0,
        (anchor[1] + anchor[4]) / 2.0,
        (anchor[2] + anchor[5]) / 2.0,
    ];
    let pad = [span[0] * 0.02, span[1] * 0.02, span[2] * 0.02];
    let neighbourhood_bbox: [f64; 6] = [
        centre[0] - pad[0],
        centre[1] - pad[1],
        centre[2] - pad[2],
        centre[0] + pad[0],
        centre[1] + pad[1],
        centre[2] + pad[2],
    ];

    let source_file = std::fs::File::open(source_out.path().join("building.parquet")).unwrap();
    let source_num_row_groups = ParquetRecordBatchReaderBuilder::try_new(source_file)
        .unwrap()
        .metadata()
        .num_row_groups();
    let hilbert_file = std::fs::File::open(hilbert_out.path().join("building.parquet")).unwrap();
    let hilbert_num_row_groups = ParquetRecordBatchReaderBuilder::try_new(hilbert_file)
        .unwrap()
        .metadata()
        .num_row_groups();
    assert_eq!(
        source_num_row_groups, hilbert_num_row_groups,
        "both packages must have the same number of row groups (same row_group_size, same row \
         count) for the touched-group counts below to be a fair comparison"
    );

    let source_groups_touched = row_groups_touching(source_out.path(), neighbourhood_bbox);
    let hilbert_groups_touched = row_groups_touching(hilbert_out.path(), neighbourhood_bbox);
    eprintln!(
        "real-neighbourhood query row groups touched (of {source_num_row_groups}): \
         source={source_groups_touched} hilbert={hilbert_groups_touched}"
    );
    assert!(
        hilbert_groups_touched < source_groups_touched,
        "Hilbert ordering must prune more aggressively than Source ordering on the same \
         real-neighbourhood query: source touched \
         {source_groups_touched}/{source_num_row_groups} row groups, Hilbert touched \
         {hilbert_groups_touched}/{hilbert_num_row_groups} — expected \
         hilbert_groups_touched < source_groups_touched"
    );
}

/// The reader resolves conformance from the file itself, never from a claim
/// about its writer, and accepts all three shapes it can legitimately meet.
///
/// The GeoParquet 1.1 and CityParquet-only cases are built with a bare
/// `ArrowWriter` rather than by disabling something in this crate's writer:
/// they stand in for a foreign writer, and a fixture produced by the code
/// under test could only ever confirm that code agrees with itself.
mod conformance {
    use super::*;
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use cityparquet::reader::GeoConformance;
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;
    use std::sync::Arc;

    /// One row of `MultiPolygon Z` WKB — a single closed triangle.
    fn wkb_multipolygon_z() -> Vec<u8> {
        let coords = [
            [0.0f64, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 2.0],
            [0.0, 0.0, 0.0],
        ];
        let mut poly = vec![1u8];
        poly.extend(1003u32.to_le_bytes());
        poly.extend(1u32.to_le_bytes());
        poly.extend(4u32.to_le_bytes());
        poly.extend(
            coords
                .iter()
                .flat_map(|p| p.iter().flat_map(|c| c.to_le_bytes())),
        );
        let mut b = vec![1u8];
        b.extend(1006u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(poly);
        b
    }

    /// A minimal one-column file whose `geometry_lod0_0` is a plain
    /// `BYTE_ARRAY` — no logical type — carrying `city`, and `geo` only when
    /// `with_geo`.
    fn foreign_file(dir: &std::path::Path, with_geo: bool) -> std::path::PathBuf {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry_lod0_0",
            DataType::Binary,
            true,
        )]));
        let col: ArrayRef = Arc::new(BinaryArray::from(vec![Some(
            wkb_multipolygon_z().as_slice(),
        )]));
        let batch = RecordBatch::try_new(schema.clone(), vec![col]).unwrap();

        let city = r#"{"version":"0.1.0-draft","columns":[{"name":"geometry_lod0_0","encoding":"WKB","geometry_types":["MultiPolygon Z"],"orientation_3d":"right-handed"}]}"#;
        let mut kvs = vec![parquet::file::metadata::KeyValue::new(
            "city".to_string(),
            city.to_string(),
        )];
        if with_geo {
            kvs.push(parquet::file::metadata::KeyValue::new(
                "geo".to_string(),
                r#"{"version":"1.1.0","primary_column":"geometry_lod0_0","columns":{"geometry_lod0_0":{"encoding":"WKB","geometry_types":["MultiPolygon Z"]}}}"#
                    .to_string(),
            ));
        }
        let props = WriterProperties::builder()
            .set_key_value_metadata(Some(kvs))
            .build();

        let path = dir.join(if with_geo {
            "gp1.parquet"
        } else {
            "cponly.parquet"
        });
        let f = std::fs::File::create(&path).unwrap();
        let mut w = ArrowWriter::try_new(f, schema, Some(props)).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        path
    }

    fn conformance_of(path: &std::path::Path) -> cityparquet::Result<GeoConformance> {
        let f = std::fs::File::open(path).unwrap();
        ParquetRecordBatchReaderBuilder::try_new(f)
            .unwrap()
            .geometry_conformance()
    }

    /// This crate's own writer annotates, so its output is GeoParquet 2.0.
    #[test]
    fn our_own_output_is_geoparquet_2() {
        let out = tempfile::tempdir().unwrap();
        let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
        opts.overwrite = true;
        convert(&opts).unwrap();
        assert_eq!(
            conformance_of(&out.path().join("building.parquet")).unwrap(),
            GeoConformance::GeoParquet2
        );
    }

    /// An unannotated column described by `geo` is the 1.1 fail-safe.
    #[test]
    fn an_unannotated_column_with_geo_reads_as_geoparquet_1() {
        let dir = tempfile::tempdir().unwrap();
        let path = foreign_file(dir.path(), true);
        assert_eq!(conformance_of(&path).unwrap(), GeoConformance::GeoParquet1);
    }

    /// Neither annotation nor `geo` — the normal state of a solid-only table.
    #[test]
    fn an_unannotated_column_without_geo_reads_as_cityparquet_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = foreign_file(dir.path(), false);
        assert_eq!(
            conformance_of(&path).unwrap(),
            GeoConformance::CityParquetOnly
        );
    }
}

/// CityParquet never writes `GEOGRAPHY`, so meeting one means the file came
/// from somewhere with a different edge model. It must be refused rather than
/// read as planar: geodesic edges change every area, volume and bbox this
/// crate computes, and nothing downstream would notice the difference.
#[test]
fn a_geography_annotated_column_is_refused() {
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use parquet_geospatial::{WkbEdges, WkbMetadata, WkbType};
    use std::sync::Arc;

    let mut field = Field::new("geometry_lod0_0", DataType::Binary, true);
    field
        .try_with_extension_type(WkbType::new(Some(WkbMetadata::new(
            Some("OGC:CRS84"),
            Some(WkbEdges::Spherical),
        ))))
        .unwrap();
    let schema = Arc::new(Schema::new(vec![field]));
    let col: ArrayRef = Arc::new(BinaryArray::from(vec![None::<&[u8]>]));
    let batch = RecordBatch::try_new(schema.clone(), vec![col]).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("geography.parquet");
    let f = std::fs::File::create(&path).unwrap();
    let mut w = ArrowWriter::try_new(f, schema, None).unwrap();
    w.write(&batch).unwrap();
    w.close().unwrap();

    let f = std::fs::File::open(&path).unwrap();
    let err = ParquetRecordBatchReaderBuilder::try_new(f)
        .unwrap()
        .geometry_conformance()
        .expect_err("a GEOGRAPHY-annotated geometry column must be refused");
    assert!(
        err.to_string().contains("GEOGRAPHY"),
        "the error must name what it refused, got: {err}"
    );
}
