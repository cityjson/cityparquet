//! `cityparquet bench`: the variant-matrix benchmark harness (M5 task 6).
//!
//! For every requested variant (a [`RecipePreset`] plus optional Hilbert row
//! ordering), this converts `input` into a fresh tempdir, times the
//! conversion, measures package size, times a full table scan (also
//! deriving the dataset bbox by unioning every row's own bbox — cheap,
//! since the scan already reads every row), times a bbox-pruned "window"
//! query anchored at the dataset bbox's lower-left corner, counts how many
//! row groups that window query touches vs. the table's total, and (unless
//! `--skip-roundtrip`) exports the package back to CityJSONSeq and compares
//! it against `input` for exact semantic equality. One CSV row per variant
//! is appended, in variant order, to `--out`.
//!
//! The single-vs-by-type layout comparison was retired when by-type became
//! the sole, mandatory table layout (2026-07-21); last numbers under the
//! old `+by-type` axis are in `.superpowers/sdd/bytype-family-report.md`.
//!
//! Every variant converts with [`Profile::Compatibility`] — never
//! conditionally on whether `input` actually has appearance/templates — to
//! keep the harness uniform across variants and datasets: sidecar files are
//! only written when the source data has something to put in them, so a
//! Core-only dataset (no materials/textures/templates) costs nothing extra
//! by always asking for Compatibility.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use arrow_array::{Array, Float64Array, RecordBatch, StructArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet::reader::{CityParquetReaderBuilder, row_group_intersects};
use cityparquet::recipe::{Codec, RecipePreset, WriterRecipe};
use cityparquet::schema::Profile;
use cityparquet::stac::properties::PackageTables;
use cityparquet::{CityParquetError, Result};

/// The exact CSV header `run` writes (and the smoke test asserts against).
const CSV_HEADER: &str = "dataset,variant,object_count,write_s,total_bytes,cityobjects_bytes,\
sidecar_bytes,full_scan_s,window_query_s,row_groups_total,row_groups_touched,roundtrip_equal";

/// Options controlling one `cityparquet bench` run.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub input: PathBuf,
    pub out_csv: PathBuf,
    /// Number of repeats per timed measurement (write/full-scan/window-query);
    /// the reported value is the MEDIAN across repeats. Must be >= 1.
    pub repeat: usize,
    /// Variant identifiers (`<preset>[+hilbert][+rg<N>]`); empty selects the
    /// default 9-variant set (see [`default_variant_ids`]).
    pub variants: Vec<String>,
    /// Fraction of the dataset bbox's x/y extent the window query covers,
    /// anchored at the bbox's lower-left corner (z is always the full
    /// range). Must satisfy `0 < window_frac <= 1` and be finite.
    pub window_frac: f64,
    /// Skip the export+compare round-trip check; `roundtrip_equal` is then
    /// left empty in the CSV rather than `true`/`false`.
    pub skip_roundtrip: bool,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            out_csv: PathBuf::new(),
            // M5 Codex review, Important finding 5(b): 3 repeats is too few
            // to trust the small (sub-10ms) deltas the paper draws on; 5
            // gives a sturdier median at a modest extra cost.
            repeat: 5,
            variants: Vec::new(),
            window_frac: 0.05,
            skip_roundtrip: false,
        }
    }
}

/// The default 9-variant set: every [`RecipePreset::ALL`] plain, plus
/// `cityparquet+hilbert`, and — M5 Codex review (Important finding 4) —
/// `cityparquet+rg512` / `cityparquet+hilbert+rg512`, a row-group size small
/// enough that the larger committed datasets (delft 2,231 objects, the
/// dense-urban tile 2,423) genuinely split into multiple (5) row groups, so
/// `row_groups_touched` can actually demonstrate pruning instead of every
/// dataset landing in a single group. 512 rather than the originally-ruled
/// 4096: even the LARGEST committed dataset has fewer than 4,096 rows, so
/// rg4096 still produced one group everywhere and demonstrated nothing —
/// confirmed empirically on the 2026-07-08 re-run and re-ruled to rg512.
/// (A `cityparquet+by-type` variant used to sit alongside plain
/// `cityparquet` here to compare the single-file and by-type layouts; it was
/// retired 2026-07-21 when by-type became the sole, mandatory layout, since
/// it would now be byte-for-byte identical to plain `cityparquet` — see this
/// module's own doc comment for where the old numbers live.)
fn default_variant_ids() -> Vec<String> {
    let mut ids: Vec<String> = RecipePreset::ALL
        .iter()
        .map(|preset| preset.name().to_string())
        .collect();
    ids.push("cityparquet+hilbert".to_string());
    ids.push("cityparquet+rg512".to_string());
    ids.push("cityparquet+hilbert+rg512".to_string());
    ids
}

/// One parsed variant identifier: a [`RecipePreset`] plus the row ordering,
/// (optional) row-group-size override, and (optional) compression-codec
/// override its `+hilbert`/`+rg<N>`/`+<codec>` suffixes (if any) select.
#[derive(Debug, Clone, Copy)]
struct ParsedVariant {
    preset: RecipePreset,
    ordering: RowOrder,
    /// `Some(n)` overrides [`RecipePreset::recipe`]'s default row-group size
    /// (parsed from a `+rg<N>` suffix); `None` keeps the preset's default.
    row_group_size: Option<usize>,
    /// `Some(codec)` overrides the preset's default compression codec
    /// (parsed from a `+<codec>` suffix, e.g. `+gzip`); `None` keeps the
    /// preset's default codec.
    compression: Option<Codec>,
}

impl ParsedVariant {
    /// This variant's [`WriterRecipe`], with [`Self::row_group_size`] and
    /// [`Self::compression`] applied on top of the preset's default when
    /// present.
    fn recipe(&self) -> WriterRecipe {
        let mut recipe = self.preset.recipe();
        if let Some(row_group_size) = self.row_group_size {
            recipe.row_group_size = row_group_size;
        }
        if let Some(compression) = self.compression {
            recipe.compression = Some(compression);
        }
        recipe
    }
}

/// Parses `<preset>[+hilbert][+rg<N>][+<codec>]` (suffixes in any order,
/// each at most once) — e.g. `cityparquet`, `cityparquet+hilbert`,
/// `no-bss+hilbert`, `cityparquet+rg512`, `cityparquet+hilbert+rg512`,
/// `cityparquet+gzip+rg512`. `<N>` in `+rg<N>` must be a positive (non-zero)
/// integer; `<codec>` is one of [`Codec::ALL`]'s names
/// (`uncompressed`/`snappy`/`gzip`/`lz4`/`brotli`/`zstd`). A duplicated
/// suffix (e.g. `cityparquet+hilbert+hilbert`, two `+rg<N>`s, or two codec
/// tokens) is rejected rather than silently accepted as a distinct-looking
/// label for the same or an ambiguous configuration (M5 Codex review, Minor
/// finding).
fn parse_variant(id: &str) -> Result<ParsedVariant> {
    let mut parts = id.split('+');
    let preset_name = parts.next().unwrap_or("");
    let preset = RecipePreset::parse(preset_name).ok_or_else(|| variant_grammar_err(id))?;

    let mut ordering = RowOrder::Source;
    let mut row_group_size: Option<usize> = None;
    let mut compression: Option<Codec> = None;
    let mut seen_hilbert = false;
    for part in parts {
        if let Some(digits) = part.strip_prefix("rg") {
            if row_group_size.is_some() {
                return Err(variant_grammar_err(id));
            }
            let n: usize = digits.parse().map_err(|_| variant_grammar_err(id))?;
            if n == 0 {
                return Err(variant_grammar_err(id));
            }
            row_group_size = Some(n);
            continue;
        }
        if let Some(codec) = Codec::parse(part) {
            if compression.is_some() {
                return Err(variant_grammar_err(id));
            }
            compression = Some(codec);
            continue;
        }
        match part {
            "hilbert" if !seen_hilbert => {
                seen_hilbert = true;
                ordering = RowOrder::Hilbert;
            }
            _ => return Err(variant_grammar_err(id)),
        }
    }

    Ok(ParsedVariant {
        preset,
        ordering,
        row_group_size,
        compression,
    })
}

fn variant_grammar_err(id: &str) -> CityParquetError {
    let presets: Vec<&str> = RecipePreset::ALL.iter().map(|p| p.name()).collect();
    let codecs: Vec<&str> = Codec::ALL.iter().map(|c| c.name()).collect();
    CityParquetError::Schema(format!(
        "invalid variant '{id}': expected `<preset>[+hilbert][+rg<N>][+<codec>]` \
         (each suffix at most once, <N> a positive integer, <codec> one of: {}) where preset is \
         one of: {}",
        codecs.join(", "),
        presets.join(", ")
    ))
}

fn io_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Io(msg.into())
}

/// Short file names for a package's table paths, for user-facing messages —
/// `PackageTables::tables` entries are absolute paths (e.g. under a bench
/// tempdir), and echoing the whole path leaks environment detail that isn't
/// part of the package's own naming.
fn table_display_names(tables: &[PathBuf]) -> Vec<&str> {
    tables
        .iter()
        .map(|p| p.file_name().and_then(|n| n.to_str()).unwrap_or("<table>"))
        .collect()
}

fn parquet_err(msg: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Parquet(msg.to_string())
}

/// The median of `durations`, in seconds. `durations` must be non-empty.
fn median_secs(mut durations: Vec<Duration>) -> f64 {
    durations.sort();
    let n = durations.len();
    let mid = n / 2;
    if n.is_multiple_of(2) {
        (durations[mid - 1].as_secs_f64() + durations[mid].as_secs_f64()) / 2.0
    } else {
        durations[mid].as_secs_f64()
    }
}

/// One CSV data row, in the exact column order [`CSV_HEADER`] declares.
struct BenchRow {
    dataset: String,
    variant: String,
    object_count: usize,
    write_s: f64,
    total_bytes: u64,
    cityobjects_bytes: u64,
    sidecar_bytes: u64,
    full_scan_s: f64,
    window_query_s: f64,
    row_groups_total: usize,
    row_groups_touched: usize,
    roundtrip_equal: Option<bool>,
}

impl BenchRow {
    fn to_csv_line(&self) -> String {
        let roundtrip = match self.roundtrip_equal {
            Some(true) => "true",
            Some(false) => "false",
            None => "",
        };
        // M5 Codex review, Important finding 5(a): microsecond precision
        // (6 decimals), not millisecond (3) — several observations compare
        // deltas well under a millisecond, which 3-decimal rounding erases
        // outright.
        format!(
            "{},{},{},{:.6},{},{},{},{:.6},{:.6},{},{},{}",
            self.dataset,
            self.variant,
            self.object_count,
            self.write_s,
            self.total_bytes,
            self.cityobjects_bytes,
            self.sidecar_bytes,
            self.full_scan_s,
            self.window_query_s,
            self.row_groups_total,
            self.row_groups_touched,
            roundtrip,
        )
    }
}

/// Unions every non-null `bbox` row in `batch` into `acc` (creating it on
/// the first row seen if `acc` is `None`). A batch with no `bbox` column, or
/// whose `bbox` column is not the expected struct shape, is silently
/// skipped — this mirrors [`cityparquet::reader`]'s "missing statistics never
/// silently drops rows" stance, applied here to "missing bbox never silently
/// fabricates an extent" instead.
fn union_batch_bbox(batch: &RecordBatch, acc: &mut Option<[f64; 6]>) {
    let Some(bbox_col) = batch.column_by_name("bbox") else {
        return;
    };
    let Some(bbox_col) = bbox_col.as_any().downcast_ref::<StructArray>() else {
        return;
    };
    let leaf = |name: &str| -> Option<&Float64Array> {
        bbox_col
            .column_by_name(name)?
            .as_any()
            .downcast_ref::<Float64Array>()
    };
    let (Some(xmin), Some(ymin), Some(zmin), Some(xmax), Some(ymax), Some(zmax)) = (
        leaf("xmin"),
        leaf("ymin"),
        leaf("zmin"),
        leaf("xmax"),
        leaf("ymax"),
        leaf("zmax"),
    ) else {
        return;
    };

    for row in 0..batch.num_rows() {
        if bbox_col.is_null(row) {
            continue;
        }
        let row_box = [
            xmin.value(row),
            ymin.value(row),
            zmin.value(row),
            xmax.value(row),
            ymax.value(row),
            zmax.value(row),
        ];
        *acc = Some(match *acc {
            None => row_box,
            Some(current) => [
                current[0].min(row_box[0]),
                current[1].min(row_box[1]),
                current[2].min(row_box[2]),
                current[3].max(row_box[3]),
                current[4].max(row_box[4]),
                current[5].max(row_box[5]),
            ],
        });
    }
}

/// The on-disk byte size of `dir/name`.
fn file_size(dir: &std::path::Path, name: &str) -> Result<u64> {
    fs::metadata(dir.join(name))
        .map(|m| m.len())
        .map_err(|e| io_err(format!("cannot stat {}: {e}", dir.join(name).display())))
}

/// The on-disk byte size of an already-absolute path (`PackageTables::tables`
/// entries), as opposed to [`file_size`]'s `dir`+bare-name join.
fn path_size(path: &std::path::Path) -> Result<u64> {
    fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| io_err(format!("cannot stat {}: {e}", path.display())))
}

/// Runs one variant end to end (convert, full scan, window query, round
/// trip) and returns its CSV row.
fn run_variant(
    opts: &BenchOptions,
    dataset: &str,
    variant_id: &str,
    variant: ParsedVariant,
) -> Result<BenchRow> {
    // --- write_s: `opts.repeat` CLEAN conversions. Each repeat gets its
    // OWN fresh, empty tempdir, created — and, once the sample is captured,
    // dropped (deleting everything `convert` just wrote) — OUTSIDE the
    // timed window, and converts with `overwrite: false` (nothing to purge:
    // the directory is brand new), so every timed sample pays for exactly
    // one clean encode+write and nothing else. This matches the DuckDB
    // baseline script (`scripts/bench_duckdb.sh`'s `mktemp -d` per sample).
    // Before this fix, every repeat converted into the SAME tempdir with
    // `overwrite: true`, so repeats after the first ALSO timed the
    // purge/unlink of the PREVIOUS repeat's files inside the timed window —
    // a cost the baseline never paid, biasing variants that write more
    // files (e.g. `by-type`, or any Compatibility-profile sidecar) to look
    // slower than their actual encode cost (M5 Codex review, Important
    // finding 3).
    let mut write_times = Vec::with_capacity(opts.repeat);
    for _ in 0..opts.repeat {
        let repeat_dir = tempfile::tempdir().map_err(|e| io_err(e.to_string()))?;
        let mut convert_opts =
            ConvertOptions::new(opts.input.clone(), repeat_dir.path().to_path_buf());
        convert_opts.profile = Profile::Compatibility;
        convert_opts.recipe = variant.recipe();
        convert_opts.ordering = variant.ordering;
        convert_opts.overwrite = false;

        let start = Instant::now();
        convert(&convert_opts)?;
        write_times.push(start.elapsed());
        // `repeat_dir` drops here — deleting everything `convert` just
        // wrote — AFTER `start.elapsed()` was already captured, so cleanup
        // never lands inside a timed window.
    }
    let write_s = median_secs(write_times);

    // --- The package used for every measurement BELOW (size, full scan,
    // window query, round trip) is written once more, untimed, into its own
    // fresh `out_dir` — kept separate from the timing loop above so those
    // measurements never contend with, or get billed into, `write_s`.
    let out_dir = tempfile::tempdir().map_err(|e| io_err(e.to_string()))?;
    let mut convert_opts = ConvertOptions::new(opts.input.clone(), out_dir.path().to_path_buf());
    convert_opts.profile = Profile::Compatibility;
    convert_opts.recipe = variant.recipe();
    convert_opts.ordering = variant.ordering;
    convert_opts.overwrite = false;
    let report = convert(&convert_opts)?;

    // --- Package sizes, from the table/sidecar inventory `convert` just
    // wrote. `PackageTables::open` is the sole reader of `metadata.json`
    // here.
    let tables = PackageTables::open(out_dir.path())?;

    let cityobjects_bytes = tables
        .tables
        .iter()
        .map(|path| path_size(path))
        .collect::<Result<Vec<u64>>>()?
        .into_iter()
        .sum();
    let sidecar_bytes = tables
        .sidecar_files
        .iter()
        .map(|name| file_size(out_dir.path(), name))
        .collect::<Result<Vec<u64>>>()?
        .into_iter()
        .sum();
    let total_bytes: u64 = fs::read_dir(out_dir.path())
        .map_err(|e| io_err(e.to_string()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum();

    // --- full_scan_s: read every main table's every batch, `opts.repeat`
    // times; also unions the dataset bbox from the rows already being read
    // (cheap — the scan visits every row regardless).
    let mut full_scan_times = Vec::with_capacity(opts.repeat);
    let mut dataset_bbox: Option<[f64; 6]> = None;
    let mut total_rows = 0usize;
    for _ in 0..opts.repeat {
        let start = Instant::now();
        let mut rows_this_run = 0usize;
        let mut bbox_this_run: Option<[f64; 6]> = None;
        for path in &tables.tables {
            let file = File::open(path).map_err(|e| io_err(e.to_string()))?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
            let reader = builder.build().map_err(parquet_err)?;
            for batch in reader {
                let batch = batch?;
                rows_this_run += batch.num_rows();
                union_batch_bbox(&batch, &mut bbox_this_run);
            }
        }
        full_scan_times.push(start.elapsed());
        total_rows = rows_this_run;
        dataset_bbox = bbox_this_run;
    }
    let full_scan_s = median_secs(full_scan_times);

    if total_rows != report.object_count {
        return Err(CityParquetError::Schema(format!(
            "variant {variant_id}: full scan read {total_rows} rows across {:?}, but convert \
             reported object_count {}",
            table_display_names(&tables.tables),
            report.object_count
        )));
    }

    let dataset_bbox = dataset_bbox.ok_or_else(|| {
        CityParquetError::Schema(format!(
            "variant {variant_id}: no row in {:?} has a bbox — cannot derive a window query",
            table_display_names(&tables.tables)
        ))
    })?;

    // --- window_query_s: a bbox window anchored at the dataset bbox's
    // lower-left corner, extending `window_frac` of the x/y extent (z is the
    // full range), executed via the pruning reader path.
    let span_x = dataset_bbox[3] - dataset_bbox[0];
    let span_y = dataset_bbox[4] - dataset_bbox[1];
    let window_bbox: [f64; 6] = [
        dataset_bbox[0],
        dataset_bbox[1],
        dataset_bbox[2],
        dataset_bbox[0] + span_x * opts.window_frac,
        dataset_bbox[1] + span_y * opts.window_frac,
        dataset_bbox[5],
    ];

    let mut window_query_times = Vec::with_capacity(opts.repeat);
    for _ in 0..opts.repeat {
        let start = Instant::now();
        for path in &tables.tables {
            let file = File::open(path).map_err(|e| io_err(e.to_string()))?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
            let pruned = builder.with_bbox_row_groups(window_bbox)?;
            let reader = pruned.build().map_err(parquet_err)?;
            for batch in reader {
                let _ = batch?;
            }
        }
        window_query_times.push(start.elapsed());
    }
    let window_query_s = median_secs(window_query_times);

    // --- row_groups_total/touched, summed across every main table (untimed
    // — a from-scratch recount over the file's own row-group statistics, not
    // part of the timed query above).
    let mut row_groups_total = 0usize;
    let mut row_groups_touched = 0usize;
    for path in &tables.tables {
        let file = File::open(path).map_err(|e| io_err(e.to_string()))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
        let metadata = builder.metadata().clone();
        row_groups_total += metadata.num_row_groups();
        row_groups_touched += (0..metadata.num_row_groups())
            .filter(|&i| row_group_intersects(metadata.row_group(i), &window_bbox))
            .count();
    }

    // --- roundtrip_equal: export back to CityJSONSeq and compare against
    // `opts.input`. Any failure along the way (export error, compare error,
    // or a real semantic difference) surfaces as `Some(false)`, never a
    // panic and never a silent `true`.
    let roundtrip_equal = if opts.skip_roundtrip {
        None
    } else {
        let export_dir = tempfile::tempdir().map_err(|e| io_err(e.to_string()))?;
        let export_path = export_dir.path().join("roundtrip.city.jsonl");
        let equal = export(&ExportOptions {
            package_dir: out_dir.path().to_path_buf(),
            output: export_path.clone(),
        })
        .ok()
        .and_then(|_| compare_datasets(&opts.input, &export_path, &CompareOptions::default()).ok())
        .map(|report| report.equal)
        .unwrap_or(false);
        Some(equal)
    };

    Ok(BenchRow {
        dataset: dataset.to_string(),
        variant: variant_id.to_string(),
        object_count: report.object_count,
        write_s,
        total_bytes,
        cityobjects_bytes,
        sidecar_bytes,
        full_scan_s,
        window_query_s,
        row_groups_total,
        row_groups_touched,
        roundtrip_equal,
    })
}

/// Runs `opts`'s whole variant matrix, appending one CSV row per variant (in
/// variant order) to `opts.out_csv` — creating it with the header row if it
/// does not already exist, else appending (so e.g. a DuckDB-baseline script
/// can accumulate rows into the same file across separate invocations).
///
/// `opts.repeat` must be >= 1 (checked FIRST, before any conversion runs):
/// `run_variant`'s write/full-scan/window-query loops each execute
/// `opts.repeat` times and then unconditionally take the median of the
/// collected samples, so `repeat == 0` would otherwise panic deep inside
/// `median_secs` (empty `Vec`) instead of failing fast with a clear error.
///
/// `opts.window_frac` must satisfy `0 < window_frac <= 1` and be finite
/// (M5 Codex review, Minor finding): the window query is built by scaling
/// the dataset bbox's x/y extent by this fraction (see [`run_variant`]), so
/// `0`, negative, `NaN`/`inf`, or `> 1` values would silently produce a
/// degenerate, inverted, or larger-than-the-dataset "window" — e.g.
/// `--window-frac 5` intended as 5% instead benchmarking a 500% extent that
/// touches every row group.
pub fn run(opts: &BenchOptions) -> Result<()> {
    if opts.repeat == 0 {
        return Err(io_err("repeat must be >= 1"));
    }
    if !(opts.window_frac.is_finite() && opts.window_frac > 0.0 && opts.window_frac <= 1.0) {
        return Err(io_err(format!(
            "window_frac must be finite and satisfy 0 < window_frac <= 1, got {}",
            opts.window_frac
        )));
    }

    let variant_ids = if opts.variants.is_empty() {
        default_variant_ids()
    } else {
        opts.variants.clone()
    };
    let parsed_variants = variant_ids
        .iter()
        .map(|id| parse_variant(id).map(|parsed| (id.clone(), parsed)))
        .collect::<Result<Vec<_>>>()?;

    let dataset = opts
        .input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io_err(format!(
                "cannot derive a dataset name from input path {}",
                opts.input.display()
            ))
        })?
        .to_string();

    if let Some(parent) = opts.out_csv.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| io_err(format!("cannot create {}: {e}", parent.display())))?;
    }
    let file_existed = opts.out_csv.exists();
    if file_existed {
        // Appending to a CSV some OTHER tool (or an earlier, differently
        // shaped `run`) already created must not silently mix two column
        // schemas into one file — check the first line matches exactly
        // before appending any row.
        let existing = fs::read_to_string(&opts.out_csv)
            .map_err(|e| io_err(format!("cannot read {}: {e}", opts.out_csv.display())))?;
        let first_line = existing.lines().next().unwrap_or("");
        if first_line != CSV_HEADER {
            return Err(io_err(format!(
                "{} already exists with a different header; expected `{CSV_HEADER}`, found `{first_line}` \
                 — refusing to append rows with a mismatched schema",
                opts.out_csv.display()
            )));
        }
    }
    let mut csv_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&opts.out_csv)
        .map_err(|e| io_err(format!("cannot open {}: {e}", opts.out_csv.display())))?;
    if !file_existed {
        writeln!(csv_file, "{CSV_HEADER}").map_err(|e| io_err(e.to_string()))?;
    }

    for (variant_id, parsed) in parsed_variants {
        let row = run_variant(opts, &dataset, &variant_id, parsed)?;
        writeln!(csv_file, "{}", row.to_csv_line()).map_err(|e| io_err(e.to_string()))?;
    }

    Ok(())
}
