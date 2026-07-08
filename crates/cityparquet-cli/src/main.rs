use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, RowOrder, TableLayout, convert};
use cityparquet::recipe::{RecipePreset, WriterRecipe};
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
    /// Convert CityJSON/CityJSONSeq to CityParquet package
    Convert {
        /// Input CityJSON or CityJSONSeq file
        #[arg(value_name = "INPUT")]
        input: PathBuf,

        /// Output directory for CityParquet package
        #[arg(value_name = "OUTPUT")]
        output: PathBuf,

        /// Profile: core or compatibility
        #[arg(long, default_value = "core")]
        profile: String,

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

        /// Row-emission order for the main table: "source" (as the input
        /// stream yields features) or "hilbert" (buffer every feature and
        /// sort by bbox-centroid Hilbert index, improving bbox row-group
        /// pruning at the cost of holding the whole dataset in memory).
        #[arg(long, default_value = "source")]
        ordering: String,

        /// Table layout for the main CityObject data: "by-type" (default —
        /// one file per object type, e.g. building.parquet / bridge.parquet)
        /// or "single" (one cityobjects.parquet holding every type).
        #[arg(long, default_value = "by-type")]
        layout: String,

        /// Emit GeoParquet/GeoArrow self-description (the geoarrow.wkb field
        /// extension + the file-level `geo` key). Off by default: default
        /// output is plain-BLOB geometry that DuckDB `SELECT *` and the
        /// three_d extension read directly. Pass this for GeoPandas/QGIS/
        /// GDAL interop.
        #[arg(long, default_value_t = false)]
        geoarrow: bool,
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
        /// (`<preset>[+hilbert][+by-type][+rg<N>]`, e.g.
        /// `cityparquet+hilbert`, `cityparquet+rg512`); omit for the
        /// default 10-variant set
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

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Convert {
            input,
            output,
            profile,
            overwrite,
            batch_size,
            row_group_size,
            zstd_level,
            recipe,
            ordering,
            layout,
            geoarrow,
        } => {
            // Parse the profile string
            let profile = match profile.as_str() {
                "core" => cityparquet_schema::Profile::Core,
                "compatibility" => cityparquet_schema::Profile::Compatibility,
                _ => {
                    eprintln!(
                        "error: invalid profile '{}' (expected 'core' or 'compatibility')",
                        profile
                    );
                    return std::process::ExitCode::FAILURE;
                }
            };

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

            let recipe = WriterRecipe {
                row_group_size,
                zstd_level,
                statistics_for_json: false,
                preset,
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

            let layout = match layout.as_str() {
                "single" => TableLayout::Single,
                "by-type" => TableLayout::ByType,
                _ => {
                    eprintln!(
                        "error: invalid layout '{}' (expected 'single' or 'by-type')",
                        layout
                    );
                    return std::process::ExitCode::FAILURE;
                }
            };

            let opts = ConvertOptions {
                input,
                output_dir: output,
                profile,
                overwrite,
                batch_size,
                recipe,
                ordering,
                layout,
                geoarrow,
            };

            match convert(&opts) {
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

        Commands::Export {
            package_dir,
            output,
        } => {
            let opts = ExportOptions {
                package_dir,
                output,
            };

            match export(&opts) {
                Ok(report) => {
                    println!(
                        "{} {} {} {} {}",
                        report.feature_count,
                        report.object_count,
                        report.instance_geometries_dropped,
                        report.appearance_refs_dropped,
                        report.appearance_lod_misses
                    );
                    std::process::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::ExitCode::FAILURE
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
