mod alloc;

use clap::Parser;

/// Cross-format read benchmark for CityParquet (FlatCityBuf, GeoParquet, etc.).
#[derive(Parser, Debug)]
#[command(name = "cityparquet-readbench", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
