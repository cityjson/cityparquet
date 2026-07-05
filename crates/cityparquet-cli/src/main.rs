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
                        "{} {} {} {} {} {}",
                        report.object_count,
                        report.files.len(),
                        report.skipped_same_lod_geometries,
                        report.attribute_coercion_nulls,
                        report.degenerate_rings_dropped,
                        report.degenerate_surfaces_dropped
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
