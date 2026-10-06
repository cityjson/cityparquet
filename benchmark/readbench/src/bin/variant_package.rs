//! `variant-package <prepared_dir> <base> <variant-id>...`: build the
//! configuration-axis packages `<base>.<id>.parquet` from the prepared
//! `<base>.city.jsonl`, by the code path a `--variants` run uses (see
//! `cityparquet_readbench::variant_package`). A current package is kept.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use cityparquet::variant::Variant;
use cityparquet_readbench::variant_package::reuse_or_build;
use clap::Parser;

#[derive(Parser)]
struct Args {
    prepared_dir: PathBuf,
    base: String,
    #[arg(required = true)]
    ids: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let seq = args.prepared_dir.join(format!("{}.city.jsonl", args.base));
    if !seq.is_file() {
        return Err(anyhow!("no prepared CityJSONSeq {}", seq.display()));
    }
    for id in &args.ids {
        let variant = Variant::parse(id).map_err(|e| anyhow!("{id}: {e}"))?;
        let (pkg, reused) = reuse_or_build(&args.base, id, &variant, &args.prepared_dir, &seq)
            .with_context(|| format!("building {id}"))?;
        println!(
            "{} {}",
            if reused { "kept" } else { "built" },
            pkg.display()
        );
    }
    Ok(())
}
