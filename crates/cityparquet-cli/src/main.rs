use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::recipe::WriterRecipe;
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

        /// Zstd compression level
        #[arg(long, default_value = "3")]
        zstd_level: i32,
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

            let recipe = WriterRecipe {
                row_group_size,
                zstd_level,
                statistics_for_json: false,
            };

            let opts = ConvertOptions {
                input,
                output_dir: output,
                profile,
                overwrite,
                batch_size,
                recipe,
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
    }
}
