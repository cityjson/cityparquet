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

        /// features: maximum features per partition
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
        #[arg(long, default_value = "cityparquet")]
        recipe: String,

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
        #[arg(long, default_value = "source")]
        ordering: String,

        /// Emit GeoParquet/GeoArrow self-description (the geoarrow.wkb field
        /// extension + the file-level `geo` key). Off by default: default
        /// output is plain-BLOB geometry that DuckDB `SELECT *` and the
        /// three_d extension read directly. Pass this for GeoPandas/QGIS/
        /// GDAL interop.
        #[arg(long, default_value_t = false)]
        geoarrow: bool,

        /// Physical geometry column encoding: "wkb" (default, normative) or
        /// "arrow-native" (experimental nested Arrow List/Struct columns —
        /// see docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md).
        /// Only the DECLARED schema responds to this so far (the row-encoder
        /// still only writes WKB bytes; see `Task 6`).
        #[arg(long, default_value = "wkb")]
        geometry_encoding: String,

        /// Do NOT synthesise an LoD0 footprint into the primary `geometry`
        /// column for objects lacking a source LoD0. By default a footprint is
        /// derived from the lowest higher LoD (§9 "LoD0 synthesis") so the
        /// GeoParquet primary column is populated; pass this to keep the output
        /// strictly source-faithful.
        #[arg(long, default_value_t = false)]
        no_lod0: bool,
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

/// Resolve `inputs` (files/dirs/globs) and open each as a [`Source`].
fn resolve_and_open(inputs: &[PathBuf]) -> Result<Vec<Source>, String> {
    let resolved = resolve_inputs(inputs).map_err(|e| e.to_string())?;
    let mut sources = Vec::with_capacity(resolved.len());
    for p in &resolved {
        sources.push(Source::open(p).map_err(|e| e.to_string())?);
    }
    Ok(sources)
}

/// Collapse `sources` to one [`Source`]: the lone input directly, or — for
/// several — a merged in-memory source ([`merge_sources`] enforces a shared
/// CRS and requantises onto one transform).
fn merge_to_one(sources: Vec<Source>) -> Result<Source, String> {
    if sources.len() == 1 {
        return Ok(sources.into_iter().next().expect("one source"));
    }
    let merged = merge_sources(&sources).map_err(|e| e.to_string())?;
    Ok(Source::from_parts(
        merged.header,
        merged.features,
        merged.doc_appearance,
        SourceFormat::CityJsonSeq,
    ))
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
            geometry_encoding,
            no_lod0,
        } => {
            let preset = match RecipePreset::parse(&recipe) {
                Some(preset) => preset,
                None => {
                    let valid: Vec<&str> = RecipePreset::ALL.iter().map(|p| p.name()).collect();
                    eprintln!(
                        "error: invalid recipe '{}' (expected one of: {})",
                        recipe,
                        valid.join(", ")
                    );
                    return std::process::ExitCode::FAILURE;
                }
            };

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
                preset,
                compression,
            };

            let ordering = match ordering.as_str() {
                "source" => RowOrder::Source,
                "hilbert" => RowOrder::Hilbert,
                _ => {
                    eprintln!(
                        "error: invalid ordering '{}' (expected 'source' or 'hilbert')",
                        ordering
                    );
                    return std::process::ExitCode::FAILURE;
                }
            };

            let geometry_encoding = match geometry_encoding.as_str() {
                "wkb" => cityparquet_schema::types::GeometryEncoding::Wkb,
                "arrow-native" => cityparquet_schema::types::GeometryEncoding::ArrowNative,
                other => {
                    eprintln!(
                        "error: --geometry-encoding must be \"wkb\" or \"arrow-native\", got {other:?}"
                    );
                    return std::process::ExitCode::FAILURE;
                }
            };

            let opts = ConvertOptions {
                input: inputs.first().cloned().unwrap_or_default(),
                output_dir: output,
                overwrite,
                batch_size,
                recipe,
                ordering,
                geoarrow,
                geometry_encoding,
                generate_lod0: !no_lod0,
                lod0: cityparquet::lod0::Lod0Options::default(),
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

            let sources = match resolve_and_open(&inputs) {
                Ok(sources) => sources,
                Err(e) => {
                    eprintln!("error: {}", e);
                    return std::process::ExitCode::FAILURE;
                }
            };

            match partition {
                Some(method) => {
                    let spec = match parse_partition_spec(&method, number, feature_num, cell_size) {
                        Ok(spec) => spec,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            return std::process::ExitCode::FAILURE;
                        }
                    };
                    match convert_partitioned(&sources, &spec, &opts) {
                        Ok(report) => {
                            println!(
                                "partitions={} duplicate_ids={}",
                                report.partitions.len(),
                                report.duplicate_ids
                            );
                            for (label, r) in &report.partitions {
                                println!("{} {}", label, r.object_count);
                            }
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
                None => {
                    let source = match merge_to_one(sources) {
                        Ok(source) => source,
                        Err(e) => {
                            eprintln!("error: {}", e);
                            return std::process::ExitCode::FAILURE;
                        }
                    };
                    match convert_source(&source, &opts) {
                        Ok(report) => {
                            println!(
                                "{} {} {} {} {} {} {} {} {}",
                                report.object_count,
                                report.files.len(),
                                report.skipped_same_lod_geometries,
                                report.attribute_coercion_nulls,
                                report.degenerate_rings_dropped,
                                report.degenerate_surfaces_dropped,
                                report.materials_written,
                                report.textures_written,
                                report.templates_written
                            );
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
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
                        eprintln!("error: {}", e);
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
                        eprintln!("error: {}", e);
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
                    eprintln!("error: {}", e);
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
                    eprintln!("error: {}", e);
                    std::process::ExitCode::FAILURE
                }
            }
        }
    }
}
