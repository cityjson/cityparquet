use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::inputs::resolve_inputs;
use cityparquet::merge::merge_sources;
use cityparquet::package::{ConvertOptions, RowOrder, convert_source};
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::recipe::{Codec, RecipePreset, WriterRecipe};
use cityparquet::source::{Source, SourceFormat};
use cityparquet_cli::bench::{self, BenchOptions};
use cityparquet_schema::Result as CpResult;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cityparquet")]
#[command(about = "CityParquet command-line tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert CityJSON/CityJSONSeq/CityGML to a CityParquet package
    Convert {
        /// Input files, directories, or glob patterns (CityJSON, CityJSONSeq,
        /// CityGML). Multiple inputs are merged into one dataset; directories
        /// contribute their immediate .json/.jsonl/.gml children.
        #[arg(value_name = "INPUTS", required = true, num_args = 1..)]
        inputs: Vec<PathBuf>,

        /// Output directory for the CityParquet package (parent directory of
        /// per-partition packages when --partition is given)
        #[arg(short = 'o', long = "output", value_name = "OUTPUT")]
        output: PathBuf,

        /// Partition method: "count" (N equal chunks, needs --number),
        /// "features" (<= M features each, needs --feature-num), or "box"
        /// (spatial grid, needs --cell-size). Omit to write one package.
        #[arg(long, value_name = "METHOD")]
        partition: Option<String>,

        /// count: number of partitions
        #[arg(long, value_name = "N")]
        number: Option<usize>,

        /// features: maximum features per partition. Exceeded only where
        /// features reference each other's objects across the feature
        /// boundary and must share a package to stay resolvable — a warning
        /// says so when it happens
        #[arg(long, value_name = "M")]
        feature_num: Option<usize>,

        /// box: square grid cell edge length, in CRS units (metres)
        #[arg(long, value_name = "METRES")]
        cell_size: Option<f64>,

        /// Overwrite existing output directory
        #[arg(long)]
        overwrite: bool,

        /// Batch size for encoding
        #[arg(long, default_value = "4096")]
        batch_size: usize,

        /// Row group size for Parquet
        #[arg(long, default_value = "65536")]
        row_group_size: usize,

        /// Zstd compression level (ignored by --recipe snappy, which always
        /// compresses with Snappy)
        #[arg(long, default_value = "3")]
        zstd_level: i32,

        /// Named writer-property preset: cityparquet, parquet-defaults,
        /// no-dictionary, no-bss, no-delta, snappy. Selects the per-column
        /// tuning rules; --row-group-size and --zstd-level still apply on
        /// top where meaningful.
        #[arg(long, value_enum, default_value = "cityparquet")]
        recipe: RecipeArg,

        /// Compression codec, overriding whichever codec --recipe would
        /// otherwise pick: uncompressed, snappy, gzip, lz4, brotli, zstd.
        /// Unset by default, which keeps the recipe's own codec choice
        /// (zstd at --zstd-level for every preset but snappy, which always
        /// uses snappy).
        #[arg(long)]
        compression: Option<String>,

        /// Row-emission order for the main table: "source" (as the input
        /// stream yields features) or "hilbert" (buffer every feature and
        /// sort by bbox-centroid Hilbert index, improving bbox row-group
        /// pruning at the cost of holding the whole dataset in memory).
        #[arg(long, value_enum, default_value = "source")]
        ordering: OrderingArg,

        /// Emit GeoParquet/GeoArrow self-description (the geoarrow.wkb field
        /// extension + the file-level `geo` key). Off by default: default
        /// output is plain-BLOB geometry that DuckDB `SELECT *` and the
        /// three_d extension read directly. Pass this for GeoPandas/QGIS/
        /// GDAL interop.
        #[arg(long, default_value_t = false)]
        geoarrow: bool,

        /// Do NOT synthesise an LoD0 footprint into the primary `geometry`
        /// column for objects lacking a source LoD0. By default a footprint is
        /// derived from the lowest higher LoD (§9 "LoD0 synthesis") so the
        /// GeoParquet primary column is populated; pass this to keep the output
        /// strictly source-faithful.
        #[arg(long, default_value_t = false)]
        no_lod0: bool,

        /// Operator-supplied CRS (e.g. EPSG:25832, or the bare 25832) used
        /// ONLY when the source declares none — it is ignored for a source
        /// that declares its own. Without it, a source carrying CRS-bearing
        /// coordinates but no resolvable CRS still converts, but the package
        /// is written with `city.crs: null` (CRS unknown) and warns. When the
        /// override is applied, the output records
        /// `city.other.crs_source = "operator-supplied"`. A geographic
        /// (degree-valued) code is refused: nothing here reprojects.
        #[arg(long, value_name = "EPSG")]
        crs: Option<String>,

        /// Drop a material/texture index that falls outside its local
        /// definitions array instead of aborting conversion. Off by default:
        /// this implementation is the appearance-resolution oracle, so a
        /// dangling reference is fatal unless explicitly waived — pass this
        /// for input another CityParquet implementation reads regardless. A
        /// dropped reference is counted in the report's trailing field,
        /// never silent.
        #[arg(long, default_value_t = false)]
        tolerate_invalid_appearance: bool,
    },

    /// Export CityParquet package back to CityJSON/CityJSONSeq
    Export {
        /// Input CityParquet package directory
        #[arg(value_name = "PACKAGE_DIR")]
        package_dir: PathBuf,

        /// Output file (.city.jsonl for Seq, .city.json for doc)
        #[arg(value_name = "OUTPUT")]
        output: PathBuf,
    },

    /// Compare two CityJSON/CityJSONSeq datasets for semantic equality
    Compare {
        /// First dataset (CityJSON/CityJSONSeq or exported package)
        #[arg(value_name = "A")]
        a: PathBuf,

        /// Second dataset (CityJSON/CityJSONSeq or exported package)
        #[arg(value_name = "B")]
        b: PathBuf,

        /// Exclude material/texture blocks from comparison
        #[arg(long)]
        exclude_appearance: bool,

        /// Exclude GeometryInstance geometries from comparison
        #[arg(long)]
        exclude_instances: bool,
    },

    /// Run the variant-matrix benchmark harness, appending one CSV row per
    /// variant to --out
    Bench {
        /// Input CityJSON or CityJSONSeq file
        #[arg(long, value_name = "INPUT")]
        input: PathBuf,

        /// Output CSV path (created with a header if absent, appended to
        /// otherwise)
        #[arg(long, value_name = "CSV")]
        out: PathBuf,

        /// Number of repeats per timed measurement (write/full-scan/window-
        /// query); the reported value is the median across repeats
        #[arg(long, default_value = "5")]
        repeat: usize,

        /// Comma-separated variant identifiers
        /// (`<preset>[+hilbert][+rg<N>][+<codec>]`, e.g.
        /// `cityparquet+hilbert`, `cityparquet+rg512`,
        /// `cityparquet+gzip+rg512`); omit for the default 9-variant set
        #[arg(long)]
        variants: Option<String>,

        /// Fraction of the dataset bbox's x/y extent the window query
        /// covers, anchored at the bbox's lower-left corner
        #[arg(long, default_value = "0.05")]
        window_frac: f64,

        /// Skip the export+compare round-trip correctness check
        #[arg(long)]
        skip_roundtrip: bool,
    },
}

/// CLI mirror of [`RecipePreset`], so clap itself validates `--recipe` and
/// lists the accepted values. The library type stays clap-free.
///
/// Two names are pinned with `#[value(name)]` because clap's kebab-casing
/// would rename them: `CityParquet` would render `city-parquet`, and
/// `NoByteStreamSplit` would render `no-byte-stream-split` — the established
/// value is `no-bss` ([`RecipePreset::name`] is the same vocabulary, shared
/// with the benchmark variant identifiers).
#[derive(Clone, Copy, clap::ValueEnum)]
enum RecipeArg {
    #[value(name = "cityparquet")]
    CityParquet,
    ParquetDefaults,
    NoDictionary,
    #[value(name = "no-bss")]
    NoByteStreamSplit,
    NoDelta,
    Snappy,
}

impl RecipeArg {
    fn preset(self) -> RecipePreset {
        match self {
            RecipeArg::CityParquet => RecipePreset::CityParquet,
            RecipeArg::ParquetDefaults => RecipePreset::ParquetDefaults,
            RecipeArg::NoDictionary => RecipePreset::NoDictionary,
            RecipeArg::NoByteStreamSplit => RecipePreset::NoByteStreamSplit,
            RecipeArg::NoDelta => RecipePreset::NoDelta,
            RecipeArg::Snappy => RecipePreset::Snappy,
        }
    }
}

/// CLI mirror of [`RowOrder`] (`--ordering`).
#[derive(Clone, Copy, clap::ValueEnum)]
enum OrderingArg {
    Source,
    Hilbert,
}

impl OrderingArg {
    fn row_order(self) -> RowOrder {
        match self {
            OrderingArg::Source => RowOrder::Source,
            OrderingArg::Hilbert => RowOrder::Hilbert,
        }
    }
}

/// Resolve `inputs` (files/dirs/globs) and open each as a [`Source`].
fn resolve_and_open(inputs: &[PathBuf]) -> CpResult<Vec<Source>> {
    let resolved = resolve_inputs(inputs)?;
    for p in &resolved.skipped_non_files {
        eprintln!(
            "warning: glob match {} is not a file; skipping",
            p.display()
        );
    }
    let mut sources = Vec::with_capacity(resolved.files.len());
    for p in &resolved.files {
        sources.push(Source::open(p)?);
    }
    Ok(sources)
}

/// Collapse `sources` to one [`Source`]: the lone input directly, or — for
/// several — a merged in-memory source ([`merge_sources`] enforces a shared
/// CRS and requantises onto one transform).
///
/// An operator-supplied CRS is a property of the header, so it travels onto
/// the merged source — but only when EVERY input got its CRS from `--crs`.
/// `merge_sources` enforces one shared CRS across the inputs, so a single
/// input that declared that CRS itself makes the merged CRS source-declared:
/// the operator's value was, for that input, a no-op. Asking `any` instead
/// would stamp a whole mixed batch `crs_source: "operator-supplied"` and strip
/// the genuine `referenceSystem` out of the verbatim `source_metadata`,
/// leaving a footer that denies a declaration the source did make.
fn merge_to_one(sources: Vec<Source>) -> CpResult<Source> {
    if sources.len() == 1 {
        return Ok(sources.into_iter().next().expect("one source"));
    }
    let crs_is_operator_supplied = sources.iter().all(Source::crs_is_operator_supplied);
    let merged = merge_sources(&sources)?;
    if merged.duplicate_ids > 0 {
        eprintln!(
            "warning: {} duplicate feature id(s) across inputs; all kept (a package with \
             duplicate ids cannot faithfully round-trip through export)",
            merged.duplicate_ids
        );
    }
    Ok(Source::from_parts(
        merged.header,
        merged.features,
        merged.doc_appearance,
        SourceFormat::CityJsonSeq,
    )
    .with_crs_operator_supplied(crs_is_operator_supplied))
}

/// Build a [`PartitionSpec`] from the `--partition` method and its sizing flag,
/// requiring exactly the matching flag and rejecting flags for other methods.
fn parse_partition_spec(
    method: &str,
    number: Option<usize>,
    feature_num: Option<usize>,
    cell_size: Option<f64>,
) -> Result<PartitionSpec, String> {
    let extras = |allowed: &str| -> Result<(), String> {
        let mut wrong = Vec::new();
        if allowed != "number" && number.is_some() {
            wrong.push("--number");
        }
        if allowed != "feature_num" && feature_num.is_some() {
            wrong.push("--feature-num");
        }
        if allowed != "cell_size" && cell_size.is_some() {
            wrong.push("--cell-size");
        }
        if wrong.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "--partition {method} does not accept {}",
                wrong.join(", ")
            ))
        }
    };
    match method {
        "count" => {
            extras("number")?;
            let n = number.ok_or("--partition count requires --number")?;
            if n < 1 {
                return Err("--number must be >= 1".to_string());
            }
            Ok(PartitionSpec::Count(n))
        }
        "features" => {
            extras("feature_num")?;
            let m = feature_num.ok_or("--partition features requires --feature-num")?;
            if m < 1 {
                return Err("--feature-num must be >= 1".to_string());
            }
            Ok(PartitionSpec::Features(m))
        }
        "box" => {
            extras("cell_size")?;
            let c = cell_size.ok_or("--partition box requires --cell-size")?;
            if !c.is_finite() || c <= 0.0 {
                return Err("--cell-size must be a finite value > 0".to_string());
            }
            Ok(PartitionSpec::Box { cell: c })
        }
        other => Err(format!(
            "invalid partition method '{other}' (expected count, features, or box)"
        )),
    }
}

/// Render `e` and its whole `source()` chain as `"top: cause: cause"`.
///
/// `CityParquetError::Io`/`Parquet` carry the underlying `std::io::Error` /
/// `ParquetError` as a real `#[source]` rather than flattening it into the
/// message (review P7), so the Display string alone is only the context half
/// ("cannot open <path>"). This is the exit boundary where a human reads the
/// error, so it walks the chain and appends every cause — the errno text an
/// operator needs is on the chain, not in the top-level message.
fn render_error(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        // A source whose Display the context already quotes verbatim would
        // just stutter; `parquet_from` deliberately builds such a pair.
        let text = c.to_string();
        if !out.ends_with(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        cur = c.source();
    }
    out
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Convert {
            inputs,
            output,
            partition,
            number,
            feature_num,
            cell_size,
            overwrite,
            batch_size,
            row_group_size,
            zstd_level,
            recipe,
            compression,
            ordering,
            geoarrow,
            no_lod0,
            crs,
            tolerate_invalid_appearance,
        } => {
            // `--compression` deliberately keeps its hand-rolled parse: its
            // "error: invalid compression '<v>' (expected one of: …)" text and
            // its exit code are pinned by the CLI smoke tests, and clap's own
            // value validation would replace both. The other three flags are
            // clap `ValueEnum`s above.
            let compression = match compression {
                Some(s) => match Codec::parse(&s) {
                    Some(codec) => Some(codec),
                    None => {
                        let valid: Vec<&str> = Codec::ALL.iter().map(|c| c.name()).collect();
                        eprintln!(
                            "error: invalid compression '{}' (expected one of: {})",
                            s,
                            valid.join(", ")
                        );
                        return std::process::ExitCode::FAILURE;
                    }
                },
                None => None,
            };

            let recipe = WriterRecipe {
                row_group_size,
                zstd_level,
                statistics_for_json: false,
                preset: recipe.preset(),
                compression,
            };
            let ordering = ordering.row_order();

            let mut opts = ConvertOptions {
                input: inputs.first().cloned().unwrap_or_default(),
                output_dir: output,
                overwrite,
                batch_size,
                recipe,
                ordering,
                geoarrow,
                generate_lod0: !no_lod0,
                lod0: cityparquet::lod0::Lod0Options::default(),
                crs_override: None,
                tolerate_invalid_appearance,
            };

            // A sizing flag only makes sense with --partition.
            if partition.is_none()
                && (number.is_some() || feature_num.is_some() || cell_size.is_some())
            {
                eprintln!(
                    "error: --number/--feature-num/--cell-size require --partition <count|features|box>"
                );
                return std::process::ExitCode::FAILURE;
            }

            let mut sources = match resolve_and_open(&inputs) {
                Ok(sources) => sources,
                Err(e) => {
                    eprintln!("error: {}", render_error(&e));
                    return std::process::ExitCode::FAILURE;
                }
            };

            // Declare the operator's CRS on every source BEFORE they are
            // merged or partitioned, so the merge's shared-CRS check and the
            // scan both see an ordinary, resolvable CRS. Each source records
            // for itself whether the declaration took effect, and the footer's
            // `crs_source` stamp is read from there — a source that declares
            // its own CRS is left untouched and claims nothing.
            //
            // `crs_override` carries the value onward for validation, and is
            // set whenever the flag was given — NOT only when applying it did
            // something. A value that was ignored is still a value the
            // operator typed, and gating validation on applied-ness meant
            // `--crs banana`, `--crs EPSG:4326` and `--crs ""` all exited 0
            // with no message at all on a source that declares its own CRS.
            // No false stamp can follow: the provenance is read from the
            // `Source`, never from this option.
            if let Some(code) = &crs {
                for source in &mut sources {
                    source.set_reference_system(code);
                }
                opts.crs_override = Some(code.clone());
            }

            match partition {
                Some(method) => {
                    let spec = match parse_partition_spec(&method, number, feature_num, cell_size) {
                        Ok(spec) => spec,
                        // `parse_partition_spec` yields a plain String, not a
                        // CityParquetError — there is no chain to render.
                        Err(e) => {
                            eprintln!("error: {e}");
                            return std::process::ExitCode::FAILURE;
                        }
                    };
                    match convert_partitioned(&sources, &spec, &opts) {
                        Ok(report) => {
                            // Every partition scans the same merged source, so
                            // the CRS diagnostic is one dataset-level fact, not
                            // a per-partition one — printed once rather than
                            // repeated per output directory.
                            if let Some(message) = report
                                .partitions
                                .iter()
                                .find_map(|(_, r)| r.crs_diagnostic.as_deref())
                            {
                                eprintln!("warning: {message}");
                            }
                            // Reference locality: both counts are 0 for
                            // conformant input, so these lines only ever
                            // appear when the input really did carry a
                            // hierarchy split across features.
                            if report.co_assigned_features > 0 {
                                eprintln!(
                                    "warning: {} feature(s) reference a parent or child in \
                                     another feature; they were assigned to a shared partition \
                                     so the references still resolve within one package \
                                     (CityJSONSeq features are meant to be self-contained)",
                                    report.co_assigned_features
                                );
                            }
                            if report.unresolvable_refs > 0 {
                                eprintln!(
                                    "warning: {} parent/child reference(s) name an object that \
                                     is not in the input at all; they are written as-is and \
                                     will not resolve in any package",
                                    report.unresolvable_refs
                                );
                            }
                            // Unlike the CRS diagnostic above, this is
                            // OBJECT-level, not dataset-level — every
                            // partition's own drops are printed, not just
                            // the first partition that has one.
                            for (_, r) in &report.partitions {
                                for message in &r.dropped_colliding_member_diagnostics {
                                    eprintln!("warning: {message}");
                                }
                            }
                            // Summed across partitions and printed once,
                            // same shape as `partitions=`/`duplicate_ids=`
                            // above: a dataset-level fact, not a
                            // per-partition one. Also appended to each
                            // partition's own line, since a dropped
                            // reference is meaningful at that granularity
                            // too and the flag's own doc comment promises
                            // it is "counted ... never silent" regardless
                            // of which convert path took it.
                            let invalid_appearance_refs_dropped: usize = report
                                .partitions
                                .iter()
                                .map(|(_, r)| r.invalid_appearance_refs_dropped)
                                .sum();
                            println!(
                                "partitions={} duplicate_ids={} invalid_appearance_refs_dropped={}",
                                report.partitions.len(),
                                report.duplicate_ids,
                                invalid_appearance_refs_dropped
                            );
                            for (label, r) in &report.partitions {
                                println!(
                                    "{} {} {}",
                                    label, r.object_count, r.invalid_appearance_refs_dropped
                                );
                            }
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("error: {}", render_error(&e));
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
                None => {
                    let source = match merge_to_one(sources) {
                        Ok(source) => source,
                        Err(e) => {
                            eprintln!("error: {}", render_error(&e));
                            return std::process::ExitCode::FAILURE;
                        }
                    };
                    match convert_source(&source, &opts) {
                        Ok(report) => {
                            // A diagnostic, not a failure (spec "CRS rules":
                            // an unresolvable CRS is declared, not fatal).
                            // stderr keeps the stdout report a stable
                            // positional line — the same idiom as the
                            // `skipped_non_files` warning above.
                            if let Some(message) = &report.crs_diagnostic {
                                eprintln!("warning: {message}");
                            }
                            // One line per dropped member (§5.1 "warn and
                            // prefer attribute"): each names the object and
                            // the colliding key, the same `warning:` idiom
                            // as the CRS diagnostic above.
                            for message in &report.dropped_colliding_member_diagnostics {
                                eprintln!("warning: {message}");
                            }
                            println!(
                                "{} {} {} {} {} {} {} {} {} {}",
                                report.object_count,
                                report.files.len(),
                                report.skipped_same_lod_geometries,
                                report.attribute_coercion_nulls,
                                report.degenerate_rings_dropped,
                                report.degenerate_surfaces_dropped,
                                report.materials_written,
                                report.textures_written,
                                report.templates_written,
                                report.invalid_appearance_refs_dropped
                            );
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("error: {}", render_error(&e));
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
            }
        }

        Commands::Export {
            package_dir,
            output,
        } => {
            // A `.gml` output goes to the native CityGML 2.0 writer; every
            // other extension is CityJSON/CityJSONSeq via `export`.
            if output.extension().and_then(|e| e.to_str()) == Some("gml") {
                let opts = WriteOptions {
                    package_dir,
                    output,
                };
                match write_package(&opts) {
                    Ok(report) => {
                        println!(
                            "{} buildings written; {} non-building skipped, {} without geometry, \
                             {} composite solids written, {} multi-solids skipped, \
                             {} lod columns skipped, {} attributes written, {} attributes skipped",
                            report.buildings_written,
                            report.non_building_skipped,
                            report.buildings_without_solid_skipped,
                            report.composite_solids_written,
                            report.multi_solids_skipped,
                            report.lod_columns_skipped,
                            report.attributes_written,
                            report.attributes_skipped,
                        );
                        std::process::ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: {}", render_error(&e));
                        std::process::ExitCode::FAILURE
                    }
                }
            } else {
                let opts = ExportOptions {
                    package_dir,
                    output,
                };

                match export(&opts) {
                    Ok(report) => {
                        println!(
                            "{} {} {} {}",
                            report.feature_count,
                            report.object_count,
                            report.instance_geometries_dropped,
                            report.appearance_refs_dropped
                        );
                        std::process::ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: {}", render_error(&e));
                        std::process::ExitCode::FAILURE
                    }
                }
            }
        }

        Commands::Compare {
            a,
            b,
            exclude_appearance,
            exclude_instances,
        } => {
            let opts = CompareOptions {
                coord_tolerance: [0.0; 3],
                exclusions: Exclusions {
                    appearance: exclude_appearance,
                    geometry_instances: exclude_instances,
                },
            };

            match compare_datasets(&a, &b, &opts) {
                Ok(report) => {
                    if report.equal {
                        // Print excluded count info if any
                        let excluded_count = report.excluded.len();
                        if excluded_count > 0 {
                            println!("equal (excluded: {})", excluded_count);
                        } else {
                            println!("equal");
                        }
                        std::process::ExitCode::SUCCESS
                    } else {
                        const MAX_PRINTED_DIFFERENCES: usize = 20;
                        for diff in report.differences.iter().take(MAX_PRINTED_DIFFERENCES) {
                            println!("{diff}");
                        }
                        if report.differences.len() > MAX_PRINTED_DIFFERENCES {
                            println!(
                                "... ({} more)",
                                report.differences.len() - MAX_PRINTED_DIFFERENCES
                            );
                        }
                        std::process::ExitCode::from(2)
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", render_error(&e));
                    std::process::ExitCode::FAILURE
                }
            }
        }

        Commands::Bench {
            input,
            out,
            repeat,
            variants,
            window_frac,
            skip_roundtrip,
        } => {
            let variants = variants
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();

            let opts = BenchOptions {
                input,
                out_csv: out,
                repeat,
                variants,
                window_frac,
                skip_roundtrip,
            };

            match bench::run(&opts) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {}", render_error(&e));
                    std::process::ExitCode::FAILURE
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, render_error};
    use cityparquet_schema::CityParquetError;
    use clap::CommandFactory;

    /// clap's own debug assertions catch conflicting/invalid arg
    /// definitions (including the ValueEnum names) at test time.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// P7 moved the OS detail off the Display string and onto `source()`.
    /// The exit boundary must therefore walk the chain, or the operator
    /// loses the errno text that used to be interpolated into the message.
    #[test]
    fn rendered_error_shows_the_whole_source_chain() {
        let e = CityParquetError::io_source(
            "cannot open /tmp/nope",
            std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"),
        );
        let rendered = render_error(&e);
        assert!(rendered.contains("cannot open /tmp/nope"), "{rendered}");
        assert!(rendered.contains("No such file or directory"), "{rendered}");
    }

    #[test]
    fn rendered_error_without_a_source_is_just_its_display() {
        let e = CityParquetError::io("no input files resolved");
        assert_eq!(render_error(&e), "io error: no input files resolved");
    }
}
