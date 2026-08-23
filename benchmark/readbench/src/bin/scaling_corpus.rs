//! The FlatCityBuf adapter for [`cityparquet_readbench::scaling`]: stream
//! one local `.fcb` file front to back and cut CityJSONSeq scaling slices
//! out of it (`just fetch-scaling-data` owns the download + this call).
//!
//! The header line is `fcb_core`'s own CityJSON metadata reconstruction
//! (`deserializer::to_cj_metadata`), i.e. exactly what `fcb deser` would
//! emit — including the SOURCE file's `transform` and CRS, which every
//! slice shares, and the source's whole-file `geographicalExtent`, which a
//! prefix slice over-covers. That is left as-is deliberately: downstream
//! consumers (`cityparquet convert`, the readbench param derivation)
//! compute extents from the features themselves, and fabricating a
//! tighter extent here would put hand-rolled numbers in a measurement
//! input.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use fcb_core::FcbReader;
use fcb_core::deserializer::to_cj_metadata;

use cityparquet_readbench::scaling::write_scaling_slices;

/// Cut fixed-CityObject-count CityJSONSeq prefixes out of one FlatCityBuf
/// file.
#[derive(Parser)]
struct Args {
    /// The source `.fcb` file (read locally, front to back, only as far as
    /// the largest slice needs).
    #[arg(long)]
    input: PathBuf,
    /// Directory the `<stem>_n<size>.city.jsonl` slices are written into.
    #[arg(long)]
    out_dir: PathBuf,
    /// Filename stem naming the source dataset.
    #[arg(long)]
    stem: String,
    /// Comma-separated CityObject targets, one slice each. A slice stops
    /// at the first whole feature that reaches its target, so actual
    /// counts can slightly exceed the nominal size (reported per slice).
    #[arg(long, value_delimiter = ',', required = true)]
    sizes: Vec<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let file =
        File::open(&args.input).with_context(|| format!("opening {}", args.input.display()))?;
    let mut iter = FcbReader::open(BufReader::new(file))
        .with_context(|| format!("reading FCB header of {}", args.input.display()))?
        .select_all_seq()
        .context("starting sequential FCB scan")?;

    let header = iter.header();
    let features_total = header.features_count();
    let header_line = serde_json::to_string(
        &to_cj_metadata(&header).context("reconstructing the CityJSON header line")?,
    )?;

    // `FeatureIter::next` does not return `None` at end of stream on its
    // own; the header's `features_count` bounds the walk, exactly as
    // `fcb deser` bounds its own (fcb_cli 0.7.6, `deserialize`).
    let mut seen = 0u64;
    let summaries = write_scaling_slices(
        &header_line,
        || {
            if seen >= features_total {
                return Ok(None);
            }
            match iter.next()? {
                None => Ok(None),
                Some(feature) => {
                    seen += 1;
                    let cj = feature.cur_cj_feature()?;
                    Ok(Some((serde_json::to_string(&cj)?, cj.city_objects.len())))
                }
            }
        },
        &args.sizes,
        &args.out_dir,
        &args.stem,
    )?;

    for s in &summaries {
        println!(
            "{} target={} features={} city_objects={}",
            s.path.display(),
            s.target,
            s.features,
            s.city_objects
        );
    }
    println!(
        "scaling-corpus: {} slice(s) from {} ({} of {} features read)",
        summaries.len(),
        args.input.display(),
        seen,
        features_total
    );
    Ok(())
}
