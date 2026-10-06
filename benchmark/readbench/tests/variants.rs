//! `run --variants`: the configuration-axis run. Per variant, one untimed
//! conversion with the variant's recipe, the package kept under
//! `<prepared_dir>/<base>.<variant>.parquet`, then the ordinary read
//! children against it. The CSV shape is the read run's, with the variant id
//! in the `format` column and no row for the conversion itself.

use std::path::PathBuf;
use std::process::{Command, Output};

use cityparquet::package::{ConvertOptions, RowOrder, convert};

const HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_mean_s,\
time_std_s,time_median_s,time_min_s,time_max_s,time_q1_s,time_q3_s,peak_heap_bytes,\
peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,\
stats_pruned,filter_bytes";

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A prepared dir the way `readbench_prepare.sh` leaves it for a
/// CityJSONSeq input: `<base>.parquet` (the Hilbert-ordered package the query
/// parameters derive from) and `<base>.city.jsonl`.
fn prepared_delft() -> (tempfile::TempDir, PathBuf) {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    convert_delft(
        &input,
        &prepared.path().join("delft.parquet"),
        RowOrder::Hilbert,
    );
    std::fs::copy(&input, prepared.path().join("delft.city.jsonl")).unwrap();
    (prepared, input)
}

fn convert_delft(input: &std::path::Path, out: &std::path::Path, ordering: RowOrder) {
    let mut opts = ConvertOptions::new(input.to_path_buf(), out.to_path_buf());
    opts.generate_lod0 = false;
    opts.ordering = ordering;
    convert(&opts).unwrap();
}

/// The `id` column of a package's one table, in row order.
fn ids_in(package: &std::path::Path) -> Vec<String> {
    use arrow_array::{Array, StringArray};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let table = std::fs::read_dir(package)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "parquet"))
        .expect("the package holds a .parquet table");
    let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(table).unwrap())
        .unwrap()
        .build()
        .unwrap();
    let mut ids = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let column = batch.column_by_name("id").expect("an id column");
        let column = column
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("a Utf8 id");
        ids.extend((0..column.len()).map(|i| column.value(i).to_string()));
    }
    ids
}

/// Non-null values across every `geometry_lod0*` column of a package's tables.
fn lod0_geometries_in(package: &std::path::Path) -> usize {
    use arrow_array::Array;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let mut count = 0;
    for entry in std::fs::read_dir(package).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|x| x != "parquet") {
            continue;
        }
        let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
                if field.name().starts_with("geometry_lod0") {
                    count += column.len() - column.null_count();
                }
            }
        }
    }
    count
}

/// A variant package holds the source's geometries and no others, as the
/// prepare script's `--no-lod0` package does: Delft's 1,116 BuildingParts
/// carry no source LoD 0, so a synthesised footprint for each would be
/// content no other format's artefact holds.
#[test]
fn a_variant_package_synthesises_no_lod0() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "full-read",
        "--variants",
        "cityparquet",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let source_lod0 = 1115;
    assert_eq!(
        lod0_geometries_in(&prepared.path().join("delft.cityparquet.parquet")),
        source_lod0
    );
    assert_eq!(
        lod0_geometries_in(&prepared.path().join("delft.parquet")),
        source_lod0
    );
}

/// Every variant package is written in Hilbert order — the benchmark's only
/// CityParquet configuration — whatever its id says about the recipe.
#[test]
fn every_variant_package_is_written_in_hilbert_order() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "count",
        "--variants",
        "cityparquet,cityparquet+nobloom",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reference = tempfile::tempdir().unwrap();
    let source_order = reference.path().join("source.parquet");
    convert_delft(&input, &source_order, RowOrder::Source);
    let hilbert = ids_in(&prepared.path().join("delft.parquet"));
    assert_ne!(
        hilbert,
        ids_in(&source_order),
        "delft's Hilbert order must differ from its source order, or this test proves nothing"
    );
    for id in ["cityparquet", "cityparquet+nobloom"] {
        assert_eq!(
            ids_in(&prepared.path().join(format!("delft.{id}.parquet"))),
            hilbert,
            "{id} was not written in Hilbert order"
        );
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .arg("run")
        .args(args)
        .output()
        .expect("failed to run the built cityparquet-readbench binary")
}

fn field(row: &str, i: usize) -> &str {
    row.split(',').nth(i).unwrap()
}

/// A results-CSV row's field by its [`HEADER`] column name.
fn column<'a>(row: &'a str, name: &str) -> &'a str {
    let i = HEADER.split(',').position(|c| c == name).unwrap();
    field(row, i)
}

fn row_groups_in(package: &std::path::Path) -> usize {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let table = std::fs::read_dir(package)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "parquet"))
        .expect("the package holds a .parquet table");
    let reader = SerializedFileReader::new(std::fs::File::open(table).unwrap()).unwrap();
    reader.metadata().num_row_groups()
}

#[test]
fn a_variants_run_builds_reads_keeps_the_packages_and_records_sizes() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "full-read,bbox",
        "--variants",
        "cityparquet,cityparquet+rg512,cityparquet+zstd1",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), HEADER);
    let rows: Vec<&str> = lines.collect();

    // Grouped per variant, the reads in scenario order; the conversion that
    // built each package is not a row.
    let expected_order = [
        ("cityparquet", "full-read"),
        ("cityparquet", "bbox-query"),
        ("cityparquet", "bbox-query"),
        ("cityparquet", "bbox-query"),
        ("cityparquet+rg512", "full-read"),
        ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+zstd1", "full-read"),
        ("cityparquet+zstd1", "bbox-query"),
        ("cityparquet+zstd1", "bbox-query"),
        ("cityparquet+zstd1", "bbox-query"),
    ];
    let samples: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(prepared.path().join("out.csv.samples.json")).unwrap(),
    )
    .unwrap();
    let samples = samples.as_array().unwrap();
    assert_eq!(samples.len(), 24, "12 measurements x warmup + one sample");
    assert_eq!(
        samples
            .iter()
            .filter(|sample| sample["scenario"] == "write")
            .count(),
        0,
        "the conversion is not sampled"
    );
    assert_eq!(
        samples
            .iter()
            .filter(|sample| sample["warmup"] == true)
            .count(),
        12
    );

    assert_eq!(rows.len(), expected_order.len(), "rows:\n{text}");
    for (row, (label, scenario)) in rows.iter().zip(expected_order) {
        assert_eq!(field(row, 0), "delft.city.jsonl");
        assert_eq!(field(row, 1), label, "row: {row}");
        assert_eq!(field(row, 2), scenario, "row: {row}");
    }

    let full_reads: Vec<&&str> = rows.iter().filter(|r| field(r, 2) == "full-read").collect();
    assert!(full_reads.iter().all(|r| field(r, 4) == "2231"));

    for id in ["cityparquet", "cityparquet+rg512", "cityparquet+zstd1"] {
        let pkg = prepared.path().join(format!("delft.{id}.parquet"));
        assert!(
            pkg.join("metadata.json").is_file(),
            "package kept at {}",
            pkg.display()
        );
    }
    // 2231 rows / 512 per group = 5 groups: the rg512 recipe reached the
    // writer, and the baseline keeps the default single group.
    assert_eq!(
        row_groups_in(&prepared.path().join("delft.cityparquet+rg512.parquet")),
        5
    );
    assert_eq!(
        row_groups_in(&prepared.path().join("delft.cityparquet.parquet")),
        1
    );
    assert!(
        prepared.path().join("delft.parquet").is_dir(),
        "the prepare script's package is untouched"
    );
    let leftovers: Vec<String> = std::fs::read_dir(prepared.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        // `.readbench-chain` holds the chain stamps, one per built package.
        .filter(|n| n.starts_with('.') && n != ".readbench-chain")
        .collect();
    assert!(
        prepared
            .path()
            .join(".readbench-chain/delft.cityparquet")
            .is_file(),
        "a built variant package is stamped with the chain version"
    );
    assert!(
        leftovers.is_empty(),
        "scratch directories were not cleaned up: {leftovers:?}"
    );

    let sizes = std::fs::read_to_string(prepared.path().join("sizes.csv")).unwrap();
    let mut sizes = sizes.lines();
    assert_eq!(sizes.next().unwrap(), "dataset,format,bytes,mb_decimal");
    let size_rows: Vec<&str> = sizes.collect();
    assert_eq!(size_rows.len(), 3);
    for (row, id) in size_rows
        .iter()
        .zip(["cityparquet", "cityparquet+rg512", "cityparquet+zstd1"])
    {
        assert_eq!(
            row.split(',').count(),
            4,
            "bytes and decimal MB only: {row}"
        );
        assert_eq!(field(row, 0), "delft");
        assert_eq!(field(row, 1), id);
        let bytes = field(row, 2).parse::<u64>().unwrap();
        assert!(bytes > 0);
        assert_eq!(
            field(row, 3),
            format!("{:.6}", bytes as f64 / 1_000_000.0),
            "mb_decimal is bytes / 10^6: {row}"
        );
    }
    assert!(prepared.path().join("out.csv.params.json").is_file());
}

#[test]
fn a_rerun_replaces_its_own_sizes_rows_instead_of_appending() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let args = [
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "full-read",
        "--variants",
        "cityparquet,cityparquet+rg512",
    ];
    assert!(run(&args).status.success());
    assert!(run(&args).status.success());
    let sizes = std::fs::read_to_string(prepared.path().join("sizes.csv")).unwrap();
    assert_eq!(
        sizes.lines().count(),
        3,
        "header + two rows, not four:\n{sizes}"
    );
}

fn expect_rejection(args: &[&str], needle: &str) {
    let output = run(args);
    assert!(!output.status.success(), "must be rejected: {args:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(needle), "expected {needle:?} in:\n{stderr}");
}

fn with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    base.iter().copied().chain(extra.iter().copied()).collect()
}

#[test]
fn variants_and_formats_are_exclusive_and_the_list_is_validated() {
    let (prepared, input) = prepared_delft();
    let out = prepared.path().join("out.csv");
    let base: Vec<&str> = vec![
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "count",
    ];

    expect_rejection(
        &with(
            &base,
            &["--variants", "cityparquet", "--formats", "cityjsonseq"],
        ),
        "exclusive",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet+rg512"]),
        "baseline",
    );
    expect_rejection(
        &with(
            &base,
            &[
                "--variants",
                "cityparquet,cityparquet+rg512,cityparquet+rg512",
            ],
        ),
        "duplicate",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet,cityparquet+gzip6"]),
        "only zstd takes a level",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet,cityparquet+source"]),
        "asks for source order",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet", "--write-repeat", "1"]),
        "--write-repeat",
    );
    expect_rejection(
        &with(
            &base,
            &["--variants", "cityparquet", "--scenarios", "write"],
        ),
        "unknown scenario 'write'",
    );
    // `project` was retired from the format family, with no alias left.
    expect_rejection(
        &with(
            &base,
            &["--variants", "cityparquet", "--scenarios", "project"],
        ),
        "unknown scenario 'project'",
    );
    assert!(!out.exists(), "a rejected run writes no CSV");
}

/// The bloom pair: the same package with and without filters. Lookup rows
/// carry counters — no filter bytes and nothing pruned without filters. delft is ONE row group, so the pruning the
/// family exists to show is exactly visible: a `*-miss` probe rules that group
/// out (`bloom_pruned` 1) with filters and cannot without them, and a hit
/// probe never prunes on either side.
#[test]
fn a_bloom_pair_records_lookup_counters() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "id-lookup,feature-lookup",
        "--id-probes",
        "id-50pct,id-miss",
        "--variants",
        "cityparquet,cityparquet+nobloom",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), HEADER);
    let rows: Vec<&str> = lines.collect();
    // Per variant: id-50pct, id-miss, feature-50pct, feature-miss.
    assert_eq!(rows.len(), 8, "{text}");
    for row in &rows {
        let (label, scenario, notes) = (field(row, 1), field(row, 2), column(row, "notes"));
        let counters: Vec<&str> = [
            "row_groups_total",
            "bloom_pruned",
            "filter_bytes",
            "stats_pruned",
        ]
        .iter()
        .map(|name| column(row, name))
        .collect();
        assert_eq!(row.split(',').count(), HEADER.split(',').count(), "{row}");
        assert_ne!(scenario, "write", "{row}");
        assert_eq!(counters[0], "1", "delft is one row group: {row}");
        // The miss probes sit inside the group's identifier range, so the
        // statistics never reject them, with or without filters.
        assert_eq!(counters[3], "0", "stats_pruned: {row}");
        let is_miss = notes.starts_with("id-miss") || notes.starts_with("feature-miss");
        if label == "cityparquet+nobloom" {
            assert_eq!(counters[1], "0", "{row}");
            assert_eq!(counters[2], "0", "{row}");
        } else {
            assert_ne!(counters[2], "0", "{row}");
            assert_eq!(
                counters[1],
                if is_miss { "1" } else { "0" },
                "the pruning the bloom family exists to show: {row}"
            );
        }
    }
}

async fn spawn_server(dir: PathBuf) -> std::net::SocketAddr {
    let app = axum::Router::new().fallback_service(tower_http::services::ServeDir::new(dir));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// Over HTTP a `--variants` run reads the packages a local run built and an
/// operator uploaded — here, the prepared directory served as it is. No
/// sizes, and every lookup row carries its transport and
/// lookup counters. `multi_thread`: `run` blocks on a child process while the
/// server task must keep accepting.
#[tokio::test(flavor = "multi_thread")]
async fn a_variants_run_over_http_reads_the_uploaded_packages_without_building() {
    let (prepared, input) = prepared_delft();
    let common = |out: &PathBuf| -> Vec<String> {
        [
            "--input",
            input.to_str().unwrap(),
            "--prepared-dir",
            prepared.path().to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--repeat",
            "1",
            "--scenarios",
            "id-lookup",
            "--id-probes",
            "id-miss",
            "--variants",
            "cityparquet,cityparquet+nobloom",
        ]
        .map(String::from)
        .to_vec()
    };
    let local_csv = prepared.path().join("local.csv");
    let args = common(&local_csv);
    let local = run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        local.status.success(),
        "{}",
        String::from_utf8_lossy(&local.stderr)
    );
    let sizes = prepared.path().join("sizes.csv");
    let sizes_before = std::fs::read_to_string(&sizes).unwrap();

    let addr = spawn_server(prepared.path().to_path_buf()).await;
    let http_csv = prepared.path().join("http.csv");
    let mut args = common(&http_csv);
    args.extend(["--transport".to_string(), "http".to_string()]);
    args.extend(["--base-url".to_string(), format!("http://{addr}")]);
    let output = run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&http_csv).unwrap();
    let rows: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(rows.len(), 2, "one id-miss row per variant:\n{text}");
    for row in &rows {
        assert_eq!(field(row, 2), "id-lookup", "{row}");
        assert!(!column(row, "bytes_read").is_empty(), "bytes_read: {row}");
        assert!(
            !column(row, "http_requests").is_empty(),
            "http_requests: {row}"
        );
        assert_eq!(
            column(row, "row_groups_total"),
            "1",
            "row_groups_total: {row}"
        );
        // The async path prunes exactly as the sync one does: delft's single
        // row group is ruled out for the verified-absent probe with filters
        // and cannot be without them.
        assert_eq!(
            column(row, "bloom_pruned"),
            if field(row, 1) == "cityparquet+nobloom" {
                "0"
            } else {
                "1"
            },
            "bloom_pruned: {row}"
        );
    }
    assert_eq!(std::fs::read_to_string(&sizes).unwrap(), sizes_before);
}

/// The bloom axis on several row groups (delft written at 256 rows per
/// group): an identifier and a text attribute, each with a hit and a miss, on
/// the package with filters and on the one without. With filters a miss is
/// pruned in every group (bar false positives) and a hit in every group that
/// does not hold the value; without filters nothing is pruned by a filter;
/// both packages return the same counts.
#[test]
fn the_bloom_axis_prunes_identifier_and_attribute_probes_across_row_groups() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "id-lookup,attr-lookup",
        "--id-probes",
        "id-50pct,id-miss",
        "--bloom-attributes",
        "identificatie",
        "--variants",
        "cityparquet,cityparquet+rg256,cityparquet+nobloom+rg256",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&out_csv).unwrap();
    // The bare baseline (one row group) is required on every axis; the
    // pruning is read off the two 256-row variants.
    let rows: Vec<&str> = text
        .lines()
        .skip(1)
        .filter(|row| field(row, 1).contains("+rg256"))
        .collect();
    assert_eq!(rows.len(), 8, "{text}");
    let mut counts = std::collections::HashMap::new();
    for row in &rows {
        let (label, notes) = (field(row, 1), column(row, "notes"));
        let n = |name: &str| column(row, name).parse::<u64>().unwrap();
        let (total, bloom, stats) = (n("row_groups_total"), n("bloom_pruned"), n("stats_pruned"));
        assert_eq!(total, 9, "2231 rows at 256 per group: {row}");
        let is_miss = notes.contains("-miss");
        if is_miss {
            assert_eq!(stats, 0, "each miss sits inside every group's range: {row}");
        }
        if label.contains("nobloom") {
            assert_eq!((bloom, n("filter_bytes")), (0, 0), "{row}");
        } else if is_miss {
            // At least `total - 1` in general (a false positive keeps one);
            // on this fixture every group is ruled out.
            assert_eq!(bloom, total, "{row}");
        } else {
            // One feature: its rows sit in at most two adjacent groups.
            assert!(bloom >= total - 2, "{row}");
            assert_eq!(bloom, total - 1, "one group holds the value here: {row}");
        }
        let count = n("result_count");
        assert_eq!(count, if is_miss { 0 } else { 1 }, "{row}");
        let tag = notes.split(';').next().unwrap().to_string();
        assert_eq!(*counts.entry(tag).or_insert(count), count, "{row}");
    }
    assert_eq!(counts.len(), 4, "{counts:?}");
}

#[test]
fn an_attribute_probe_column_without_a_filter_fails_the_run_naming_it() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    expect_rejection(
        &[
            "--input",
            input.to_str().unwrap(),
            "--prepared-dir",
            prepared.path().to_str().unwrap(),
            "--out",
            out_csv.to_str().unwrap(),
            "--repeat",
            "1",
            "--scenarios",
            "attr-lookup",
            "--bloom-attributes",
            "status",
            "--variants",
            "cityparquet,cityparquet+nobloom",
        ],
        "bloom attribute 'status' carries no Bloom filter",
    );
}
