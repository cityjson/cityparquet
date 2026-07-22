//! The read-benchmark COORDINATOR: `cityparquet-readbench run ...`.
//!
//! Everything Tasks 8-10 built is a `--child` process that runs exactly one
//! (format, scenario) measurement and prints one line to stdout. This module
//! is the piece that actually drives a whole benchmark matrix: for every
//! requested (format, scenario) pair it derives real [`QueryParams`] from the
//! data itself (never a hardcoded id/attribute/bbox), spawns the `--child`
//! process `repeat` times (plus one discarded warmup) via
//! [`std::env::current_exe`], medians the timings (+ MAD), takes the MAX
//! peak heap/RSS across the repeats, computes `selectivity`, and appends one
//! row to the results CSV — which this module owns outright (a fresh
//! truncate-and-write per run, never an append, so a re-run is always
//! clean).
//!
//! **Where `QueryParams` come from.** ALL of them are derived once per
//! dataset from the `cityparquet`-format package (`<x>.parquet`), which is
//! therefore REQUIRED to be present regardless of which `--formats` were
//! requested:
//! - the dataset bbox, scanned from the `bbox` struct column, sized into
//!   three windows (1%/5%/25% of the x/y extent, anchored at the lower-left
//!   corner, full z) — the same construction
//!   `crates/cityparquet-cli/src/bench.rs` uses for its own single window;
//! - the attribute predicate for [`Scenario::AttrFilter`]: `object_type` Eq
//!   the MOST-FREQUENT value actually present (always a safe string-typed
//!   column under the Commit A `--attr-eq` fix, and present on every row —
//!   see `formats::cityjsonseq`/`formats::flatcitybuf`'s own module docs on
//!   why this scenario is CityObject-level and therefore directly
//!   comparable across every format);
//! - the numeric attribute column for [`Scenario::AttrStats`]/
//!   [`Scenario::Project`]: the alphabetically-first `Int64`/`Float64`
//!   column in the package's own `attribute_columns` metadata list, or —
//!   never fabricated — a logged skip if none exists (true for
//!   `lod3_railway.city.json`, which has no numeric attributes at all);
//! - the target id for [`Scenario::IdLookup`]: the first non-null `id`
//!   value in the table.
//!
//! **Self-consistency (logged, never a hard failure).** After the
//! `AttrFilter` scenario has run for every resolved format, this module
//! compares their `result_count`s: `object_type` equality is CityObject-level
//! for every format (CityParquet's own row grain; CityJSONSeq/FlatCityBuf
//! deliberately flatten to the same grain for this scenario — see their own
//! module docs), so a healthy run should see them agree exactly. A mismatch
//! is reported on stderr for the operator, not treated as a fatal error —
//! this is a diagnostic, not a correctness gate on the coordinator itself.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, DictionaryArray, Float64Array, RecordBatch, StringArray, StructArray,
};
use arrow_schema::{DataType, Schema};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet_schema::CityMetadata;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::scenario::{AttrPred, QueryParams, Scenario};

/// The `run` subcommand's own options — the parsed form of `main.rs`'s
/// `RunArgs` clap struct, kept independent of clap so this module has no
/// CLI-parsing concerns of its own.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The ORIGINAL CityJSON/CityJSONSeq input (also the `cityjsonseq`
    /// format's own artefact — never converted or copied).
    pub input: PathBuf,
    /// Directory `just readbench-prepare` wrote the per-format artefacts
    /// into (default `bench/data/readbench`).
    pub prepared_dir: PathBuf,
    /// Result CSV path; this run OWNS the file (fresh truncate + write).
    pub out: PathBuf,
    /// Warm repeats per measurement (a further, discarded warmup precedes
    /// every one). Must be >= 1.
    pub repeat: usize,
    /// Requested `--format` names; `None`/empty selects
    /// [`DEFAULT_FORMATS`].
    pub formats: Option<Vec<String>>,
    /// Requested scenario names (canonical [`Scenario::as_str`] spelling,
    /// case-insensitive); `None`/empty selects every [`Scenario::ALL`].
    pub scenarios: Option<Vec<String>>,
    /// After the warm matrix, run one additional `FullRead` per format,
    /// tagged `cold` in `notes` (see [`run`]'s own doc comment on the
    /// `sudo purge` protocol this does NOT automate).
    pub cold: bool,
}

/// Formats this coordinator drives when `--formats` is omitted.
/// `duckdb-parquet` is deliberately excluded — it is a separate SQL-engine
/// baseline driven entirely by `scripts/readbench_duckdb.sh` (Task 12), never
/// a `--child` format.
const DEFAULT_FORMATS: [&str; 5] = [
    "cityparquet",
    "cityparquet-hilbert",
    "flatcitybuf",
    "cityjsonseq",
    "cityjsonseq-gz",
];

/// `(fraction of the dataset bbox's x/y extent, notes tag)` for
/// [`Scenario::BBoxQuery`]'s three selectivity targets — one CSV row per
/// entry.
const BBOX_FRACTIONS: [(f64, &str); 3] = [
    (0.01, "bbox-1pct"),
    (0.05, "bbox-5pct"),
    (0.25, "bbox-25pct"),
];

/// The exact CSV header this coordinator writes.
const CSV_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,\
peak_heap_bytes,peak_rss_bytes,repeat,notes";

/// Runs `opts`'s whole (format x scenario) matrix, writing `opts.out` fresh.
pub fn run(opts: &RunOptions) -> Result<()> {
    if opts.repeat == 0 {
        bail!("--repeat must be >= 1");
    }

    let dataset = opts
        .input
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot derive a dataset name from input path {}",
                opts.input.display()
            )
        })?;
    let base = strip_known_extension(&dataset);

    // The cityparquet package is REQUIRED regardless of `--formats`: every
    // QueryParams derivation below reads from it (see this module's own doc
    // comment).
    let cp_table = locate_cityparquet_table(&opts.prepared_dir, base)?;

    let requested_formats: Vec<String> = match &opts.formats {
        Some(v) if !v.is_empty() => v.clone(),
        _ => DEFAULT_FORMATS.iter().map(|s| s.to_string()).collect(),
    };

    let mut resolved_formats: Vec<(String, PathBuf)> = Vec::new();
    for format in &requested_formats {
        match resolve_format_artefact(format, &opts.input, &opts.prepared_dir, base) {
            ArtefactResolution::Path(path) if path.exists() => {
                resolved_formats.push((format.clone(), path));
            }
            ArtefactResolution::Path(path) => eprintln!(
                "cityparquet-readbench: skipping format '{format}': missing artefact {} \
                 (run `just readbench-prepare {}` first)",
                path.display(),
                opts.input.display()
            ),
            ArtefactResolution::NotCoordinated => eprintln!(
                "cityparquet-readbench: skipping format '{format}': driven by \
                 scripts/readbench_duckdb.sh, not this coordinator"
            ),
            ArtefactResolution::Unknown => {
                eprintln!("cityparquet-readbench: skipping unknown format '{format}'")
            }
        }
    }
    if resolved_formats.is_empty() {
        bail!(
            "no requested format has a present artefact for dataset '{dataset}'; nothing to run \
             (run `just readbench-prepare {}` first)",
            opts.input.display()
        );
    }

    let scenarios: Vec<Scenario> = match &opts.scenarios {
        Some(v) if !v.is_empty() => v
            .iter()
            .map(|s| s.parse().map_err(|e: String| anyhow::anyhow!(e)))
            .collect::<Result<Vec<_>>>()?,
        _ => Scenario::ALL.to_vec(),
    };

    // --- Derive every QueryParams once, from real data in the cityparquet
    // package — no hardcoded ids/attrs/windows anywhere in this function.
    let meta = open_metadata(&cp_table)?;
    let schema = open_arrow_schema(&cp_table)?;
    let dataset_bbox = scan_dataset_bbox(&cp_table)?;
    let windows: Vec<([f64; 6], &'static str)> = BBOX_FRACTIONS
        .iter()
        .map(|(frac, tag)| (bbox_window(dataset_bbox, *frac), *tag))
        .collect();
    let (object_type_value, object_type_count) = most_frequent_object_type(&cp_table)?;
    let numeric_attr = pick_numeric_attribute(&meta, &schema);
    let sample_id = sample_object_id(&cp_table)?;

    // The dataset-global CityObject total — the SAME denominator for every
    // format's object-level scenarios (`AttrFilter`/`AttrStats`/`Project`/
    // `IdLookup`), because those scenarios are deliberately CityObject-level
    // on every format (see this module's own doc comment). `cityparquet`'s
    // own `Count` is exactly that total (one row per CityObject), and the
    // cityparquet package is already required/located above regardless of
    // `--formats`.
    let cp_object_total = total_count_for("cityparquet", &cp_table)
        .context("deriving the dataset-global CityObject total from the cityparquet package")?;

    eprintln!(
        "cityparquet-readbench: derived params for '{dataset}': bbox={dataset_bbox:?}, \
         object_type most-frequent='{object_type_value}' (n={object_type_count}), numeric \
         attribute={numeric_attr:?}, sample id='{sample_id}', CityObject total={cp_object_total}"
    );

    if let Some(parent) = opts.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut csv = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&opts.out)
        .with_context(|| format!("creating {}", opts.out.display()))?;
    writeln!(csv, "{CSV_HEADER}").context("writing CSV header")?;

    // AttrFilter's result_count per format, collected for the
    // self-consistency check below.
    let mut attr_filter_counts: HashMap<String, u64> = HashMap::new();

    for (format, path) in &resolved_formats {
        let total = total_count_for(format, path)
            .with_context(|| format!("deriving total count for format '{format}'"))?;

        for scenario in &scenarios {
            match scenario {
                Scenario::Count | Scenario::FullRead => {
                    run_measurement(
                        &mut csv,
                        &dataset,
                        format,
                        path,
                        *scenario,
                        &QueryParams::default(),
                        opts.repeat,
                        None,
                        "",
                    )?;
                }
                Scenario::BBoxQuery => {
                    for (window, tag) in &windows {
                        let params = QueryParams {
                            bbox: Some(*window),
                            ..Default::default()
                        };
                        run_measurement(
                            &mut csv,
                            &dataset,
                            format,
                            path,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(total),
                            tag,
                        )?;
                    }
                }
                Scenario::AttrFilter => {
                    let params = QueryParams {
                        attr_column: Some("object_type".to_string()),
                        attr_pred: Some(AttrPred::Eq(serde_json::Value::String(
                            object_type_value.clone(),
                        ))),
                        ..Default::default()
                    };
                    let notes = format!("object_type={object_type_value}");
                    let count = run_measurement(
                        &mut csv,
                        &dataset,
                        format,
                        path,
                        *scenario,
                        &params,
                        opts.repeat,
                        Some(cp_object_total),
                        &notes,
                    )?;
                    attr_filter_counts.insert(format.clone(), count);
                }
                Scenario::AttrStats | Scenario::Project => match &numeric_attr {
                    Some(column) => {
                        let params = QueryParams {
                            attr_column: Some(column.clone()),
                            ..Default::default()
                        };
                        let notes = format!("attr={column}");
                        run_measurement(
                            &mut csv,
                            &dataset,
                            format,
                            path,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(cp_object_total),
                            &notes,
                        )?;
                    }
                    None => eprintln!(
                        "cityparquet-readbench: skipping scenario '{scenario}' for format \
                         '{format}': dataset '{dataset}' has no Int64/Float64 attribute column \
                         (never fabricated)"
                    ),
                },
                Scenario::IdLookup => {
                    let params = QueryParams {
                        target_id: Some(sample_id.clone()),
                        ..Default::default()
                    };
                    let notes = format!("id={sample_id}");
                    run_measurement(
                        &mut csv,
                        &dataset,
                        format,
                        path,
                        *scenario,
                        &params,
                        opts.repeat,
                        Some(cp_object_total),
                        &notes,
                    )?;
                }
            }
        }

        if opts.cold {
            eprintln!(
                "cityparquet-readbench: --cold for format '{format}': run `sudo purge` NOW to \
                 drop the OS disk/page cache before this FullRead measurement, if you have not \
                 already (this coordinator cannot invoke `sudo` itself)"
            );
            let line = spawn_child(format, Scenario::FullRead, path, &QueryParams::default())?;
            write_row(
                &mut csv,
                &dataset,
                format,
                Scenario::FullRead,
                None,
                line.result_count,
                line.time_s,
                0.0,
                line.peak_heap_bytes,
                line.ru_maxrss_bytes,
                1,
                "cold",
            )?;
        }
    }

    // Self-consistency check: log, never fail (see this module's own doc
    // comment).
    if attr_filter_counts.len() > 1 {
        let mut values = attr_filter_counts.values();
        let first = *values.next().expect("len > 1 implies at least one value");
        if values.all(|v| *v == first) {
            eprintln!(
                "cityparquet-readbench: self-consistency OK: every resolved format's \
                 AttrFilter(object_type) result_count == {first}"
            );
        } else {
            eprintln!(
                "cityparquet-readbench: WARNING: formats disagree on \
                 AttrFilter(object_type) result_count: {attr_filter_counts:?}"
            );
        }
    }

    Ok(())
}

/// `name` with a trailing `.city.jsonl`/`.city.json`/`.jsonl`/`.json`
/// removed (the longest/most specific suffix wins) — the same stripping
/// rule `scripts/readbench_prepare.sh` and the justfile's `bench-fixtures`/
/// `convert-all` recipes use, so this coordinator locates exactly the
/// artefacts those tools produce.
fn strip_known_extension(name: &str) -> &str {
    for ext in [".city.jsonl", ".city.json", ".jsonl", ".json"] {
        if let Some(stripped) = name.strip_suffix(ext) {
            return stripped;
        }
    }
    name
}

/// One requested format's artefact path, or how it is out of this
/// coordinator's scope.
enum ArtefactResolution {
    Path(PathBuf),
    /// `duckdb-parquet`: a separate SQL-engine baseline (Task 12).
    NotCoordinated,
    /// Not one of the five formats this coordinator knows.
    Unknown,
}

/// Maps `format` onto its artefact path under `prepared_dir` (or `input`
/// itself, for `cityjsonseq`) — the exact naming convention
/// `scripts/readbench_prepare.sh` produces.
fn resolve_format_artefact(
    format: &str,
    input: &Path,
    prepared_dir: &Path,
    base: &str,
) -> ArtefactResolution {
    match format {
        "cityparquet" => ArtefactResolution::Path(prepared_dir.join(format!("{base}.parquet"))),
        "cityparquet-hilbert" => {
            ArtefactResolution::Path(prepared_dir.join(format!("{base}-hilbert.parquet")))
        }
        "flatcitybuf" => ArtefactResolution::Path(prepared_dir.join(format!("{base}.fcb"))),
        "cityjsonseq" => ArtefactResolution::Path(input.to_path_buf()),
        "cityjsonseq-gz" => ArtefactResolution::Path(prepared_dir.join(format!("{base}.jsonl.gz"))),
        "duckdb-parquet" => ArtefactResolution::NotCoordinated,
        _ => ArtefactResolution::Unknown,
    }
}

/// The cityparquet package's main table — required for QueryParams
/// derivation regardless of `--formats` (see this module's own doc
/// comment). Errors clearly, pointing at `just readbench-prepare`, rather
/// than failing deep inside a later scan.
///
/// Reads `metadata.json` and requires exactly one listed table: every
/// by-type package from a single-family dataset (e.g. delft) lists exactly
/// one, which this uses regardless of its derived name; a multi-family
/// by-type package (several family tables, no single file holding the whole
/// dataset) is rejected outright, since this coordinator's `QueryParams`
/// derivation (bbox, attribute predicate, target id — see this module's own
/// doc comment) is single-file — out of scope here rather than silently
/// deriving params from only one family's rows.
fn locate_cityparquet_table(prepared_dir: &Path, base: &str) -> Result<PathBuf> {
    let package_dir = prepared_dir.join(format!("{base}.parquet"));
    if !package_dir.is_dir() {
        bail!(
            "cannot derive QueryParams: no CityParquet package at {} — run \
             `just readbench-prepare <input>` first",
            package_dir.display()
        );
    }
    // `PackageTables::open` is the sole reader of `metadata.json` here; it
    // already rejects an empty or duplicate-naming manifest. The
    // "exactly one object table" requirement below is this coordinator's
    // own — its `QueryParams` derivation is single-file, out of scope for a
    // multi-family by-type package (see this fn's doc comment).
    let tables =
        cityparquet::stac::properties::PackageTables::open(&package_dir).with_context(|| {
            format!(
                "cannot derive QueryParams: reading {}",
                package_dir.display()
            )
        })?;
    match tables.tables.as_slice() {
        [only] => Ok(only.clone()),
        many => {
            let names: Vec<&str> = many
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect();
            bail!(
                "cannot derive QueryParams: {} has {} tables ({names:?}); only single-table \
                 (single-family) packages are supported here, not multi-table by-type packages",
                package_dir.display(),
                many.len(),
            )
        }
    }
}

fn open_metadata(table: &Path) -> Result<CityMetadata> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet metadata from {}", table.display()))?;
    Ok(builder.cityparquet_metadata()?)
}

fn open_arrow_schema(table: &Path) -> Result<Schema> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading Parquet schema from {}", table.display()))?;
    Ok((*builder.cityparquet_arrow_schema()?).clone())
}

/// Unions every non-null `bbox` row in `batch` into `acc` (creating it on
/// the first row seen). Adapted from `crates/cityparquet-cli/src/bench.rs`'s
/// own `union_batch_bbox`.
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

/// Scans the whole `bbox` column of `table` (a single-column projection) and
/// unions it into the dataset's own extent.
fn scan_dataset_bbox(table: &Path) -> Result<[f64; 6]> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["bbox"]);
    let reader = builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning bbox column of {}", table.display()))?;

    let mut acc: Option<[f64; 6]> = None;
    for batch in reader {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        union_batch_bbox(&batch, &mut acc);
    }
    acc.ok_or_else(|| {
        anyhow::anyhow!(
            "no row in {} has a bbox — cannot derive a query window",
            table.display()
        )
    })
}

/// A query window covering `frac` of `bbox`'s x/y extent, anchored at its
/// lower-left corner (z is always the full range) — the same construction
/// `crates/cityparquet-cli/src/bench.rs::run_variant` uses for its own
/// single window.
fn bbox_window(bbox: [f64; 6], frac: f64) -> [f64; 6] {
    let span_x = bbox[3] - bbox[0];
    let span_y = bbox[4] - bbox[1];
    [
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[0] + span_x * frac,
        bbox[1] + span_y * frac,
        bbox[5],
    ]
}

/// `array`'s Utf8 values as `Option<String>` per row (`None` for a null
/// cell) — handles both a plain `Utf8` array and a `Dictionary<Int32,
/// Utf8>` array. The reserved `object_type` column is ALWAYS
/// `Dictionary<Int32, Utf8>` per `cityparquet_schema::model`'s own schema
/// (never plain `Utf8`), but this accepts either shape rather than assuming
/// one, mirroring `cityparquet::query::evaluate_attr_predicate`'s own
/// `Utf8`/`Dictionary` dispatch.
fn utf8_values(array: &dyn Array) -> Result<Vec<Option<String>>> {
    match array.data_type() {
        DataType::Utf8 => {
            let values = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow::anyhow!("expected a Utf8 array"))?;
            Ok((0..values.len())
                .map(|i| (!values.is_null(i)).then(|| values.value(i).to_string()))
                .collect())
        }
        DataType::Dictionary(key_type, value_type)
            if key_type.as_ref() == &DataType::Int32 && value_type.as_ref() == &DataType::Utf8 =>
        {
            let dict = array
                .as_any()
                .downcast_ref::<DictionaryArray<Int32Type>>()
                .ok_or_else(|| anyhow::anyhow!("expected a Dictionary<Int32, Utf8> array"))?;
            let values = dict
                .downcast_dict::<StringArray>()
                .ok_or_else(|| anyhow::anyhow!("dictionary values are not Utf8"))?;
            Ok((0..dict.len())
                .map(|i| (!dict.is_null(i)).then(|| values.value(i).to_string()))
                .collect())
        }
        other => bail!("expected a Utf8 or Dictionary<Int32, Utf8> array, got {other:?}"),
    }
}

/// The most-frequent `object_type` value in `table` (and its count) — a
/// single-column projected scan, tallied in memory (the reserved
/// `object_type` column is always present, so this never needs the
/// attribute-column machinery). Ties are broken deterministically by the
/// `object_type` string itself (rather than `HashMap` iteration order, which
/// is SipHash-randomised per process) so the derived `AttrFilter` predicate —
/// and therefore the whole run — is reproducible run-to-run.
fn most_frequent_object_type(table: &Path) -> Result<(String, u64)> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["object_type"]);
    let reader = builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning object_type column of {}", table.display()))?;

    let mut counts: HashMap<String, u64> = HashMap::new();
    for batch in reader {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        for value in utf8_values(batch.column(0).as_ref())?.into_iter().flatten() {
            *counts.entry(value).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .ok_or_else(|| anyhow::anyhow!("{} has no object_type values", table.display()))
}

/// The alphabetically-first `Int64`/`Float64` attribute column in `meta`'s
/// own `attribute_columns` list (never a geometry/reserved column), or
/// `None` if the dataset has no numeric attribute at all — deterministic
/// across runs, never fabricated when no such column exists (e.g.
/// `lod3_railway.city.json`, whose attributes are all strings).
fn pick_numeric_attribute(meta: &CityMetadata, schema: &Schema) -> Option<String> {
    let mut candidates: Vec<String> = meta
        .attributes
        .iter()
        .filter(|name| {
            schema
                .field_with_name(name)
                .map(|f| matches!(f.data_type(), DataType::Int64 | DataType::Float64))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// The first non-null `id` value in `table` (a single-column projected
/// scan) — a real, present object id, never hardcoded.
fn sample_object_id(table: &Path) -> Result<String> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["id"]);
    let reader = builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning id column of {}", table.display()))?;

    for batch in reader {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("'id' column is not Utf8"))?;
        for i in 0..column.len() {
            if !column.is_null(i) {
                return Ok(column.value(i).to_string());
            }
        }
    }
    bail!("{} has no non-null id values", table.display())
}

/// One parsed `--child` protocol stdout line.
struct ChildLine {
    time_s: f64,
    peak_heap_bytes: u64,
    ru_maxrss_bytes: u64,
    result_count: u64,
}

/// Spawns a FRESH `--child` process (this binary's own executable, found via
/// [`std::env::current_exe`] — never `env!("CARGO_BIN_EXE_...")`, which is
/// only set for `cargo test`/`cargo bench` targets, not a normal build) for
/// one (format, scenario) measurement, and parses its one stdout line. A
/// fresh process per call is the point (see `formats::FormatRunner`'s own
/// doc comment): independent cache state and an independent `peak_alloc`
/// high-water mark for every single sample.
fn spawn_child(
    format: &str,
    scenario: Scenario,
    input: &Path,
    params: &QueryParams,
) -> Result<ChildLine> {
    let self_exe = std::env::current_exe().context("cannot determine own executable path")?;

    let mut cmd = Command::new(&self_exe);
    cmd.arg("--child")
        .arg("--format")
        .arg(format)
        .arg("--scenario")
        .arg(scenario.as_str())
        .arg("--input")
        .arg(input);

    if let Some(bbox) = params.bbox {
        let joined = bbox
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
        cmd.arg("--bbox").arg(joined);
    }
    if let Some(column) = &params.attr_column {
        cmd.arg("--attr-column").arg(column);
    }
    match &params.attr_pred {
        Some(AttrPred::Eq(value)) => {
            let raw = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            cmd.arg("--attr-eq").arg(raw);
        }
        Some(AttrPred::Ge(bound)) => {
            cmd.arg("--attr-ge").arg(bound.to_string());
        }
        Some(AttrPred::Le(bound)) => {
            cmd.arg("--attr-le").arg(bound.to_string());
        }
        Some(AttrPred::Range(lo, hi)) => {
            cmd.arg("--attr-ge").arg(lo.to_string());
            cmd.arg("--attr-le").arg(hi.to_string());
        }
        None => {}
    }
    if let Some(id) = &params.target_id {
        cmd.arg("--target-id").arg(id);
    }

    let output = cmd.output().with_context(|| {
        format!("spawning child process (format={format}, scenario={scenario})")
    })?;
    if !output.status.success() {
        bail!(
            "child process failed (format={format}, scenario={scenario}); stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8(output.stdout).context("child stdout was not valid UTF-8")?;
    let line = stdout.trim();
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() != 4 {
        bail!(
            "expected 4 whitespace-separated fields from the child protocol, got {} in '{line}' \
             (format={format}, scenario={scenario})",
            fields.len()
        );
    }
    Ok(ChildLine {
        time_s: fields[0]
            .parse()
            .with_context(|| format!("parsing time_s from '{}'", fields[0]))?,
        peak_heap_bytes: fields[1]
            .parse()
            .with_context(|| format!("parsing peak_heap_bytes from '{}'", fields[1]))?,
        ru_maxrss_bytes: fields[2]
            .parse()
            .with_context(|| format!("parsing ru_maxrss_bytes from '{}'", fields[2]))?,
        result_count: fields[3]
            .parse()
            .with_context(|| format!("parsing result_count from '{}'", fields[3]))?,
    })
}

/// A single `Count`-scenario child call (untimed — its own `time_s` is
/// discarded), used to establish `format`'s own total object/feature count.
/// Each format counts at its own natural grain (see
/// `formats::cityparquet`/`formats::cityjsonseq`/`formats::flatcitybuf`'s own
/// module docs on CityObject-vs-feature granularity), so this per-format
/// total is the correct SELECTIVITY denominator only for [`Scenario::BBoxQuery`]
/// (feature-level numerator over a feature-level denominator, for every
/// format). For the CityObject-level scenarios (`AttrFilter`/`AttrStats`/
/// `Project`/`IdLookup`), [`run`] instead uses the dataset-global CityObject
/// total — this same function called once against the `cityparquet` package
/// — as a SHARED denominator across every format, so those scenarios'
/// selectivity is directly comparable and always in `(0, 1]` (see this
/// module's own doc comment).
fn total_count_for(format: &str, path: &Path) -> Result<u64> {
    let line = spawn_child(format, Scenario::Count, path, &QueryParams::default())?;
    Ok(line.result_count)
}

/// Runs one (format, scenario, params) measurement: `repeat + 1` fresh child
/// processes (the first discarded as a warmup), then the MEDIAN `time_s` (+
/// MAD), the MAX `peak_heap_bytes`/`ru_maxrss_bytes` across the `repeat` warm
/// samples, and `result_count` from the first warm sample (every warm sample
/// measures the identical scenario against the identical unmodified input,
/// so they always agree on `result_count`; only the timing/memory varies).
/// Appends one CSV row and returns the `result_count` for callers that need
/// it (the self-consistency check in [`run`]).
#[allow(clippy::too_many_arguments)]
fn run_measurement(
    csv: &mut File,
    dataset: &str,
    format: &str,
    path: &Path,
    scenario: Scenario,
    params: &QueryParams,
    repeat: usize,
    total_for_selectivity: Option<u64>,
    notes: &str,
) -> Result<u64> {
    let mut times = Vec::with_capacity(repeat);
    let mut peak_heap_max = 0u64;
    let mut peak_rss_max = 0u64;
    let mut result_count: Option<u64> = None;

    for i in 0..=repeat {
        let line = spawn_child(format, scenario, path, params)?;
        if i == 0 {
            // Warmup: discarded entirely (never contributes to the median,
            // the MAX peak metrics, or `result_count`).
            continue;
        }
        times.push(line.time_s);
        peak_heap_max = peak_heap_max.max(line.peak_heap_bytes);
        peak_rss_max = peak_rss_max.max(line.ru_maxrss_bytes);
        if result_count.is_none() {
            result_count = Some(line.result_count);
        }
    }

    let result_count = result_count.expect("repeat >= 1 guarantees at least one warm sample");
    let time_s = median(&times);
    let time_mad_s = mad(&times, time_s);

    let selectivity = match scenario {
        Scenario::Count | Scenario::FullRead => None,
        _ => total_for_selectivity
            .and_then(|total| (total > 0).then_some(result_count as f64 / total as f64)),
    };

    write_row(
        csv,
        dataset,
        format,
        scenario,
        selectivity,
        result_count,
        time_s,
        time_mad_s,
        peak_heap_max,
        peak_rss_max,
        repeat,
        notes,
    )?;

    Ok(result_count)
}

/// The median of `values` (must be non-empty).
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("time_s is always finite"));
    let n = sorted.len();
    let mid = n / 2;
    if n.is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// The median absolute deviation of `values` from `med` (must be non-empty).
fn mad(values: &[f64], med: f64) -> f64 {
    let deviations: Vec<f64> = values.iter().map(|v| (v - med).abs()).collect();
    median(&deviations)
}

/// Appends one CSV row in [`CSV_HEADER`]'s exact column order.
#[allow(clippy::too_many_arguments)]
fn write_row(
    csv: &mut File,
    dataset: &str,
    format: &str,
    scenario: Scenario,
    selectivity: Option<f64>,
    result_count: u64,
    time_s: f64,
    time_mad_s: f64,
    peak_heap_bytes: u64,
    peak_rss_bytes: u64,
    repeat: usize,
    notes: &str,
) -> Result<()> {
    let selectivity_field = match selectivity {
        Some(value) => format!("{value:.6}"),
        None => String::new(),
    };
    writeln!(
        csv,
        "{dataset},{format},{scenario},{selectivity_field},{result_count},{time_s:.6},\
         {time_mad_s:.6},{peak_heap_bytes},{peak_rss_bytes},{repeat},{notes}"
    )
    .context("writing a CSV row")?;
    Ok(())
}
