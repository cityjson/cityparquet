//! Whole-branch review finding 1: a CityParquet reader must resolve each
//! geometry column's physical encoding from the FOOTER's own declaration
//! (`city.columns[].encoding` — the field the arrow-native branch introduced
//! precisely to declare this), never by guessing it from the column's Arrow
//! `DataType` shape.
//!
//! Inferring it structurally has three failure modes, all covered below:
//!
//! 1. a footer that CONTRADICTS its own physical columns is silently ignored
//!    (the file decodes as whatever shape it happens to carry) instead of
//!    being rejected as the corrupt/mis-tagged file it is;
//! 2. any FUTURE list-based physical encoding — a `CityParquetArrowNative-v2`,
//!    say — is silently misread as this branch's exact v1 shape, because the
//!    dispatch only ever asked "is the outer type a `List`?";
//! 3. reaching [`decode_batch`] directly (as `export`, `query` and the CityGML
//!    writer all do) then walks an unverified nested shape, e.g. indexing
//!    vertex-struct fields 0/1/2 that a foreign file need not have.
//!
//! Every file here is a REAL converted package (`delft.city.jsonl`), with only
//! its footer mutated — the one thing that cannot be produced by converting a
//! real source, since this crate's own writer always keeps the two in step.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow_array::RecordBatch;
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet_schema::{CityMetadata, GeometryEncoding};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::KeyValue;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Convert the first `features` lines of the real `delft.city.jsonl` under
/// `encoding` and return the package directory (kept alive by the returned
/// `TempDir`) together with its `building.parquet` path. A subset keeps these
/// tests fast while staying real data — the geometry shapes under test
/// (`MultiSurface` at LoD0, `Solid` at LoD 1.2/1.3/2.2) are all present in
/// delft's very first features.
fn converted_delft_subset(
    encoding: GeometryEncoding,
    features: usize,
) -> (tempfile::TempDir, PathBuf) {
    let source = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let subset: String = source
        .lines()
        .take(features + 1) // + the header line
        .collect::<Vec<_>>()
        .join("\n");
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("subset.city.jsonl");
    std::fs::write(&input, subset).unwrap();

    let out = dir.path().join("package");
    let mut opts = ConvertOptions::new(input, out.clone());
    opts.geometry_encoding = encoding;
    convert(&opts).unwrap();
    let table = out.join("building.parquet");
    assert!(table.exists(), "expected a building.parquet in {out:?}");
    (dir, table)
}

/// The table's first `RecordBatch` read against its OWN physical schema
/// (deliberately NOT the reader's rendered schema — these tests are about
/// what happens when the footer and the physical columns disagree, so the
/// physical side must stay exactly as written), plus the file's footer.
fn first_batch_and_footer(table: &Path) -> (RecordBatch, CityMetadata) {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let mut reader = builder.build().unwrap();
    let batch = reader.next().expect("at least one batch").unwrap();
    (batch, meta)
}

/// Rewrite every `city.columns[].encoding` entry to `token`, asserting there
/// was at least one to rewrite (otherwise the test would pass vacuously).
fn declare_encoding(meta: &mut CityMetadata, token: &str) {
    assert!(
        !meta.columns.is_empty(),
        "the fixture's footer must declare at least one geometry column, else these tests \
         cannot express a contradiction at all"
    );
    for column in &mut meta.columns {
        column.encoding = token.to_string();
    }
}

/// Rewrite `src` into `dst` keeping its physical columns exactly as written
/// but replacing the `city` footer object with `meta` — the only way to build
/// the "footer contradicts its own physical columns" file this finding is
/// about, since this crate's writer always keeps the two in step.
fn rewrite_with_footer(src: &Path, dst: &Path, meta: &CityMetadata) {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(src).unwrap()).unwrap();
    let schema = builder.schema().clone();
    let batches: Vec<RecordBatch> = builder.build().unwrap().map(|b| b.unwrap()).collect();
    let mut writer = ArrowWriter::try_new(File::create(dst).unwrap(), schema, None).unwrap();
    for batch in &batches {
        writer.write(batch).unwrap();
    }
    for (key, value) in meta.to_key_values(None).unwrap() {
        writer.append_key_value_metadata(KeyValue::new(key, value));
    }
    writer.close().unwrap();
}

/// Baseline: with the footer left exactly as the writer produced it, BOTH
/// encodings decode. Guards the fix against over-rejection — the checks the
/// three tests below demand must not make a genuine, self-consistent file
/// unreadable.
#[test]
fn a_footer_agreeing_with_its_physical_columns_decodes_under_either_encoding() {
    for encoding in [GeometryEncoding::Wkb, GeometryEncoding::ArrowNative] {
        let (_dir, table) = converted_delft_subset(encoding, 8);
        let (batch, meta) = first_batch_and_footer(&table);
        let objects = decode_batch(&batch, &meta)
            .unwrap_or_else(|e| panic!("{encoding:?} package must decode cleanly, got: {e}"));
        assert!(
            objects.iter().any(|o| !o.geometries.is_empty()),
            "{encoding:?} baseline must actually decode geometry, else the contradiction tests \
             below prove nothing"
        );
    }
}

/// The headline case: physical columns are the arrow-native nested `List`
/// shape, but the footer declares `"WKB"`. Structural inference silently
/// believes the columns and decodes anyway; the footer is the declaration a
/// reader is supposed to trust, so the disagreement must be an error.
#[test]
fn a_footer_declaring_wkb_over_arrow_native_columns_is_rejected() {
    let (_dir, table) = converted_delft_subset(GeometryEncoding::ArrowNative, 8);
    let (batch, mut meta) = first_batch_and_footer(&table);
    declare_encoding(&mut meta, "WKB");

    let err = decode_batch(&batch, &meta).expect_err(
        "a footer declaring WKB over physically arrow-native columns must be rejected, not \
         silently decoded as arrow-native",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("WKB"),
        "the error should name the declared encoding, got: {msg}"
    );
}

/// The mirror image, protecting the WKB path's own declaration: physical
/// columns are WKB `Binary`, footer claims the arrow-native encoding.
#[test]
fn a_footer_declaring_arrow_native_over_wkb_columns_is_rejected() {
    let (_dir, table) = converted_delft_subset(GeometryEncoding::Wkb, 8);
    let (batch, mut meta) = first_batch_and_footer(&table);
    declare_encoding(&mut meta, "CityParquetArrowNative-v1");

    let err = decode_batch(&batch, &meta).expect_err(
        "a footer declaring the arrow-native encoding over physically WKB columns must be \
         rejected, not silently decoded as WKB",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("CityParquetArrowNative-v1"),
        "the error should name the declared encoding, got: {msg}"
    );
}

/// Failure mode 2: a hypothetical future list-based encoding. Its physical
/// shape may well still be an outer `List`, so a structural dispatch would
/// decode it as this branch's v1 shape and quietly produce wrong geometry.
/// An encoding token this build does not understand must be refused outright.
#[test]
fn a_footer_declaring_an_unrecognised_encoding_is_rejected_not_guessed() {
    let (_dir, table) = converted_delft_subset(GeometryEncoding::ArrowNative, 8);
    let (batch, mut meta) = first_batch_and_footer(&table);
    declare_encoding(&mut meta, "CityParquetArrowNative-v2");

    let err = decode_batch(&batch, &meta).expect_err(
        "an unrecognised city.columns[].encoding must be refused, never guessed from the \
         column's physical shape",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("CityParquetArrowNative-v2"),
        "the error should name the unrecognised encoding token, got: {msg}"
    );
}

/// The same rule one layer up: `cityparquet_arrow_schema` renders the schema
/// every reader hands to `decode_batch`, and it too used to pick the encoding
/// off the physical column type. A file whose footer contradicts its own
/// columns must fail there as well, rather than render a schema the file
/// cannot actually satisfy.
#[test]
fn reader_schema_rejects_a_footer_that_contradicts_its_own_physical_columns() {
    let (dir, table) = converted_delft_subset(GeometryEncoding::ArrowNative, 8);
    let (_batch, mut meta) = first_batch_and_footer(&table);
    declare_encoding(&mut meta, "WKB");

    let mislabelled = dir.path().join("mislabelled.parquet");
    rewrite_with_footer(&table, &mislabelled, &meta);

    let builder =
        ParquetRecordBatchReaderBuilder::try_new(File::open(&mislabelled).unwrap()).unwrap();
    let err = builder.cityparquet_arrow_schema().expect_err(
        "a footer declaring WKB over physically arrow-native columns must not render a schema",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("WKB"),
        "the error should name the declared encoding, got: {msg}"
    );
}
